use std::time::Instant;

use iridium_core::types::Value;
use iridium_core::{Engine, QueryResult};

fn exec(engine: &Engine, sql: &str) -> QueryResult {
    let stmts =
        iridium_core::parse_batch(sql).unwrap_or_else(|e| panic!("parse failed:\n{sql}\n{e}"));
    engine
        .execute_batch(stmts)
        .unwrap_or_else(|e| panic!("exec failed:\n{sql}\n{e}"))
        .unwrap_or_default()
}

fn seed_perf_tables(engine: &Engine, n_rows: usize) {
    exec(
        engine,
        "CREATE TABLE dbo.PerfMain (
            Id INT PRIMARY KEY,
            GrpId INT NOT NULL,
            Flag BIT NOT NULL,
            Score INT NOT NULL,
            Tag VARCHAR(32) NOT NULL
         );
         CREATE INDEX IX_PerfMain_GrpId ON dbo.PerfMain (GrpId);
         CREATE TABLE dbo.PerfDetail (
            DetailId INT PRIMARY KEY,
            ParentId INT NOT NULL,
            IsActive BIT NOT NULL,
            MinScore INT NOT NULL,
            Label VARCHAR(32) NOT NULL
         );",
    );
    // Bulk insert in chunks of 500 rows for fast setup.
    let mut i = 1usize;
    while i <= n_rows {
        let end = (i + 499).min(n_rows);
        let mut main_vals = Vec::with_capacity(end - i + 1);
        let mut det_vals = Vec::with_capacity(end - i + 1);
        for k in i..=end {
            let grp = (k % 200) + 1;
            let flag = if k % 2 == 0 { 1 } else { 0 };
            let score = (k * 7) % 1000;
            main_vals.push(format!("({k}, {grp}, {flag}, {score}, 'tag_{grp}')"));
            let active = if k % 3 == 0 { 1 } else { 0 };
            let min_score = (k * 3) % 500;
            det_vals.push(format!("({k}, {k}, {active}, {min_score}, 'lbl_{grp}')"));
        }
        exec(
            engine,
            &format!(
                "INSERT INTO dbo.PerfMain (Id, GrpId, Flag, Score, Tag) VALUES {};",
                main_vals.join(",")
            ),
        );
        exec(
            engine,
            &format!(
                "INSERT INTO dbo.PerfDetail (DetailId, ParentId, IsActive, MinScore, Label) VALUES {};",
                det_vals.join(",")
            ),
        );
        i = end + 1;
    }
}

#[test]
fn residual_hash_join_exact_semantics_inner_left_right_full() {
    let engine = Engine::new();
    exec(
        &engine,
        "CREATE TABLE dbo.L (Id INT, Val INT);
         CREATE TABLE dbo.R (Id INT, IsActive BIT, MinVal INT, Name VARCHAR(20));
         INSERT INTO dbo.L VALUES (1, 100), (2, 10), (3, 50), (NULL, 99);
         INSERT INTO dbo.R VALUES
            (1, 1, 50, 'r1_ok'),
            (1, 0, 50, 'r1_inactive'),
            (2, 1, 50, 'r2_too_low'),
            (4, 1, 10, 'r4_only'),
            (NULL, 1, 0, 'r_null');",
    );

    // INNER JOIN with mixed equi + residual predicates (both single-side IsActive = 1 and cross-side l.Val >= r.MinVal)
    let inner = exec(
        &engine,
        "SELECT l.Id, r.Name
         FROM dbo.L l
         INNER JOIN dbo.R r ON l.Id = r.Id AND r.IsActive = 1 AND l.Val >= r.MinVal
         ORDER BY l.Id, r.Name;",
    );
    assert_eq!(
        inner.rows,
        vec![vec![Value::Int(1), Value::VarChar("r1_ok".into())]]
    );

    // LEFT JOIN: rows whose hash key matched R but failed residual predicate MUST still emit NULL-padded right row!
    let left = exec(
        &engine,
        "SELECT l.Id, r.Name
         FROM dbo.L l
         LEFT JOIN dbo.R r ON l.Id = r.Id AND r.IsActive = 1 AND l.Val >= r.MinVal
         ORDER BY CASE WHEN l.Id IS NULL THEN 999 ELSE l.Id END;",
    );
    assert_eq!(
        left.rows,
        vec![
            vec![Value::Int(1), Value::VarChar("r1_ok".into())],
            vec![Value::Int(2), Value::Null],
            vec![Value::Int(3), Value::Null],
            vec![Value::Null, Value::Null],
        ]
    );

    // RIGHT JOIN: R rows whose hash key matched L (like r1_inactive, r2_too_low) but failed residual predicate
    // MUST remain unmatched and emit NULL-padded left row!
    let right = exec(
        &engine,
        "SELECT l.Id, r.Name
         FROM dbo.L l
         RIGHT JOIN dbo.R r ON l.Id = r.Id AND r.IsActive = 1 AND l.Val >= r.MinVal
         ORDER BY r.Name;",
    );
    assert_eq!(
        right.rows,
        vec![
            vec![Value::Null, Value::VarChar("r1_inactive".into())],
            vec![Value::Int(1), Value::VarChar("r1_ok".into())],
            vec![Value::Null, Value::VarChar("r2_too_low".into())],
            vec![Value::Null, Value::VarChar("r4_only".into())],
            vec![Value::Null, Value::VarChar("r_null".into())],
        ]
    );

    // FULL OUTER JOIN: both unmatched left rows AND unmatched right rows appear once.
    let full = exec(
        &engine,
        "SELECT l.Id, r.Name
         FROM dbo.L l
         FULL OUTER JOIN dbo.R r ON l.Id = r.Id AND r.IsActive = 1 AND l.Val >= r.MinVal
         ORDER BY COALESCE(l.Id, 999), COALESCE(r.Name, '');",
    );
    assert_eq!(full.rows.len(), 8);
    assert_eq!(
        full.rows[0],
        vec![Value::Int(1), Value::VarChar("r1_ok".into())]
    );
    assert_eq!(full.rows[1], vec![Value::Int(2), Value::Null]);
    assert_eq!(full.rows[2], vec![Value::Int(3), Value::Null]);
}

#[test]
fn top_short_circuit_preserves_order_distinct_percent_and_ties() {
    let engine = Engine::new();
    exec(
        &engine,
        "CREATE TABLE dbo.TopT (Id INT PRIMARY KEY, Grp INT NOT NULL);
         INSERT INTO dbo.TopT VALUES (1, 10), (2, 10), (3, 20), (4, 20), (5, 30), (6, 30);",
    );

    // Unsorted TOP (2) returns 2 rows
    let t2 = exec(&engine, "SELECT TOP (2) Id FROM dbo.TopT;");
    assert_eq!(t2.rows.len(), 2);

    // TOP (0) returns 0 rows
    let t0 = exec(&engine, "SELECT TOP (0) Id FROM dbo.TopT;");
    assert_eq!(t0.rows.len(), 0);

    // TOP (2) WITH ORDER BY DESC must NOT short-circuit before sorting!
    let ordered = exec(&engine, "SELECT TOP (2) Id FROM dbo.TopT ORDER BY Id DESC;");
    assert_eq!(ordered.rows, vec![vec![Value::Int(6)], vec![Value::Int(5)]]);

    // TOP (2) DISTINCT must NOT short-circuit before deduplicating!
    let dist = exec(
        &engine,
        "SELECT DISTINCT TOP (2) Grp FROM dbo.TopT ORDER BY Grp;",
    );
    assert_eq!(dist.rows, vec![vec![Value::Int(10)], vec![Value::Int(20)]]);

    // TOP (2) WITH TIES must NOT short-circuit before checking ties!
    let ties = exec(
        &engine,
        "SELECT TOP (1) WITH TIES Id, Grp FROM dbo.TopT ORDER BY Grp;",
    );
    assert_eq!(ties.rows.len(), 2);

    // TOP (50) PERCENT must NOT short-circuit before counting total rows!
    let pct = exec(
        &engine,
        "SELECT TOP (50) PERCENT Id FROM dbo.TopT ORDER BY Id;",
    );
    assert_eq!(pct.rows.len(), 3);
}

#[test]
fn tier1_perf_benchmark_workloads() {
    let engine = Engine::new();
    let n = 2_000usize;
    seed_perf_tables(&engine, n);

    // 1. IndexSeek point lookups (50 queries)
    let t0 = Instant::now();
    for k in 1..=50 {
        let target = (k * 37) % n + 1;
        let res = exec(
            &engine,
            &format!("SELECT Id, Tag FROM dbo.PerfMain WHERE Id = {target};"),
        );
        assert_eq!(res.rows.len(), 1);
    }
    let d_seek = t0.elapsed();

    // 2. Mixed ON-clause LEFT JOIN (equi + residual predicate) over 2000 x 2000 rows
    let t1 = Instant::now();
    let res_join = exec(
        &engine,
        "SELECT COUNT(*), COUNT(d.DetailId)
         FROM dbo.PerfMain m
         LEFT JOIN dbo.PerfDetail d
           ON d.ParentId = m.Id AND d.IsActive = 1 AND m.Score >= d.MinScore;",
    );
    let d_join = t1.elapsed();
    assert_eq!(res_join.rows[0][0].to_integer_i64(), Some(n as i64));

    // 3. Hot column resolution + built-in scalar functions over 2000 rows (5 iterations)
    let t2 = Instant::now();
    for _ in 0..5 {
        let res_expr = exec(
            &engine,
            "SELECT SUM(LEN(UPPER(ISNULL(Tag, ''))) + ABS(Score - 500) + COALESCE(GrpId, 0))
             FROM dbo.PerfMain
             WHERE Flag = 1 AND Score > 100;",
        );
        assert_eq!(res_expr.rows.len(), 1);
    }
    let d_expr = t2.elapsed();

    // 4. Unsorted TOP (5) on 2000-row table (50 iterations)
    let t3 = Instant::now();
    for _ in 0..50 {
        let res_top = exec(
            &engine,
            "SELECT TOP (5) Id, Tag FROM dbo.PerfMain WHERE Flag = 1;",
        );
        assert_eq!(res_top.rows.len(), 5);
    }
    let d_top = t3.elapsed();

    // 5. Filtered INNER JOIN with WHERE predicate pushdown on both sides (10 iterations)
    let t4 = Instant::now();
    for _ in 0..10 {
        let res_fjoin = exec(
            &engine,
            "SELECT m.Id, d.Label
             FROM dbo.PerfMain m
             INNER JOIN dbo.PerfDetail d ON d.ParentId = m.Id
             WHERE m.GrpId = 42 AND d.IsActive = 1;",
        );
        assert!(!res_fjoin.rows.is_empty());
    }
    let d_fjoin = t4.elapsed();

    eprintln!(
        "[TIER1+2_BENCH n={n}] index_seek_50x={:.2}ms | mixed_left_join_2kx2k={:.2}ms | expr_scalar_5x={:.2}ms | top5_50x={:.2}ms | filtered_inner_join_10x={:.2}ms",
        d_seek.as_secs_f64() * 1000.0,
        d_join.as_secs_f64() * 1000.0,
        d_expr.as_secs_f64() * 1000.0,
        d_top.as_secs_f64() * 1000.0,
        d_fjoin.as_secs_f64() * 1000.0,
    );
}

#[test]
fn set_op_trailing_order_by_offset_fetch_and_where_pushdown_anti_join() {
    let engine = Engine::new();
    exec(
        &engine,
        "CREATE TABLE dbo.SetA (Id INT, Label VARCHAR(20));
         CREATE TABLE dbo.SetB (Id INT, Tag VARCHAR(20));
         INSERT INTO dbo.SetA VALUES (3, 'c'), (1, 'a'), (5, 'e');
         INSERT INTO dbo.SetB VALUES (4, 'd'), (2, 'b'), (1, 'a');",
    );

    // Trailing ORDER BY uses first-branch alias and sorts the entire combined UNION ALL result,
    // followed by OFFSET / FETCH pagination.
    let res = exec(
        &engine,
        "SELECT Id AS ItemId, Label AS Name FROM dbo.SetA
         UNION ALL
         SELECT Id, Tag FROM dbo.SetB
         ORDER BY ItemId ASC, Name ASC
         OFFSET 1 ROWS FETCH NEXT 3 ROWS ONLY;",
    );
    assert_eq!(
        res.rows,
        vec![
            vec![Value::Int(1), Value::VarChar("a".into())],
            vec![Value::Int(2), Value::VarChar("b".into())],
            vec![Value::Int(3), Value::VarChar("c".into())],
        ]
    );

    // Left-associative 3-way set-op chain with trailing ORDER BY DESC.
    let chain = exec(
        &engine,
        "SELECT Id FROM dbo.SetA
         UNION
         SELECT Id FROM dbo.SetB
         EXCEPT
         SELECT 3 AS Id
         ORDER BY Id DESC;",
    );
    assert_eq!(
        chain.rows,
        vec![
            vec![Value::Int(5)],
            vec![Value::Int(4)],
            vec![Value::Int(2)],
            vec![Value::Int(1)],
        ]
    );

    // LEFT JOIN anti-join: WHERE b.Id IS NULL MUST NOT be pushed below LEFT JOIN!
    let anti = exec(
        &engine,
        "SELECT a.Id
         FROM dbo.SetA a
         LEFT JOIN dbo.SetB b ON a.Id = b.Id
         WHERE b.Id IS NULL AND a.Id >= 3
         ORDER BY a.Id;",
    );
    assert_eq!(anti.rows, vec![vec![Value::Int(3)], vec![Value::Int(5)]]);
}
