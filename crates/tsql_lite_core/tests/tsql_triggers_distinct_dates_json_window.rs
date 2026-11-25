use tsql_lite_core::executor::result::QueryResult;
use tsql_lite_core::types::Value;
use tsql_lite_core::{parse_batch, DbError, Engine};

fn run(engine: &Engine, sql: &str) -> Result<Option<QueryResult>, DbError> {
    let stmts = parse_batch(sql)?;
    engine.execute_batch(stmts)
}

fn query(engine: &Engine, sql: &str) -> QueryResult {
    run(engine, sql)
        .expect("query execution failed")
        .expect("expected result set")
}

#[test]
fn trigger_update_col_columns_updated_and_trigger_nestlevel() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Orders (Id INT PRIMARY KEY, Status VARCHAR(20), Amount INT);
         CREATE TABLE dbo.ChildLog (Note VARCHAR(50), Nest INT, Mask VARBINARY(4));
         CREATE TABLE dbo.ChildEcho (Nest INT);

         CREATE TRIGGER dbo.trChildEcho ON dbo.ChildLog AFTER INSERT AS
         BEGIN
             INSERT INTO dbo.ChildEcho (Nest) VALUES (TRIGGER_NESTLEVEL());
         END;

         CREATE TRIGGER dbo.trOrdersAudit ON dbo.Orders AFTER INSERT, UPDATE AS
         BEGIN
             DECLARE @note VARCHAR(50) = '';
             IF UPDATE(Status) SET @note = @note + 'S';
             IF UPDATE(Amount) SET @note = @note + 'A';
             INSERT INTO dbo.ChildLog (Note, Nest, Mask)
             VALUES (@note, TRIGGER_NESTLEVEL(), COLUMNS_UPDATED());
         END;",
    )
    .expect("create tables and triggers");

    // Outside triggers: TRIGGER_NESTLEVEL() = 0
    assert_eq!(
        query(&engine, "SELECT TRIGGER_NESTLEVEL();").rows[0][0],
        Value::Int(0)
    );

    // INSERT marks all columns updated (bits 0,1,2 -> 0x07) and nests trigger level 1 -> 2
    run(
        &engine,
        "INSERT INTO dbo.Orders (Id, Status, Amount) VALUES (1, 'NEW', 100);",
    )
    .expect("insert order");

    // UPDATE Status only -> UPDATE(Status)=true, UPDATE(Amount)=false, bit 1 (0x02)
    run(
        &engine,
        "UPDATE dbo.Orders SET Status = 'PAID' WHERE Id = 1;",
    )
    .expect("update status only");

    // UPDATE Amount only -> UPDATE(Status)=false, UPDATE(Amount)=true, bit 2 (0x04)
    run(&engine, "UPDATE dbo.Orders SET Amount = 250 WHERE Id = 1;").expect("update amount only");

    let logs = query(&engine, "SELECT Note, Nest, Mask FROM dbo.ChildLog;");
    assert_eq!(logs.rows.len(), 3);
    assert_eq!(logs.rows[0][0], Value::VarChar("SA".into()));
    assert_eq!(logs.rows[0][1], Value::Int(1));
    assert_eq!(logs.rows[0][2], Value::VarBinary(vec![0x07]));

    assert_eq!(logs.rows[1][0], Value::VarChar("S".into()));
    assert_eq!(logs.rows[1][1], Value::Int(1));
    assert_eq!(logs.rows[1][2], Value::VarBinary(vec![0x02]));

    assert_eq!(logs.rows[2][0], Value::VarChar("A".into()));
    assert_eq!(logs.rows[2][1], Value::Int(1));
    assert_eq!(logs.rows[2][2], Value::VarBinary(vec![0x04]));

    // Nested child trigger observed TRIGGER_NESTLEVEL() = 2
    let echo = query(&engine, "SELECT Nest FROM dbo.ChildEcho;");
    assert_eq!(echo.rows.len(), 3);
    assert_eq!(echo.rows[0][0], Value::Int(2));
}

#[test]
fn is_distinct_from_and_is_not_distinct_from_handle_nulls_safely() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Pairs (Id INT, A INT, B INT);
         INSERT INTO dbo.Pairs VALUES
             (1, NULL, NULL),
             (2, NULL, 10),
             (3, 10, NULL),
             (4, 10, 10),
             (5, 10, 20);",
    )
    .expect("seed pairs");

    let distinct = query(
        &engine,
        "SELECT Id FROM dbo.Pairs WHERE A IS DISTINCT FROM B ORDER BY Id;",
    );
    assert_eq!(
        distinct.rows,
        vec![
            vec![Value::Int(2)],
            vec![Value::Int(3)],
            vec![Value::Int(5)]
        ]
    );

    let not_distinct = query(
        &engine,
        "SELECT Id FROM dbo.Pairs WHERE A IS NOT DISTINCT FROM B ORDER BY Id;",
    );
    assert_eq!(
        not_distinct.rows,
        vec![vec![Value::Int(1)], vec![Value::Int(4)]]
    );
}

#[test]
fn datetrunc_date_bucket_binary_checksum_and_compress_decompress() {
    let engine = Engine::new();

    // DATETRUNC across year, quarter, month, iso_week, day, hour, minute, second
    let dt = query(
        &engine,
        "DECLARE @ts DATETIME2 = '2026-05-14 15:42:37';
         SELECT
             DATETRUNC(year, @ts) AS y,
             DATETRUNC(quarter, @ts) AS q,
             DATETRUNC(month, @ts) AS m,
             DATETRUNC(iso_week, @ts) AS iw,
             DATETRUNC(hour, @ts) AS h,
             DATETRUNC(minute, @ts) AS mi;",
    );
    assert_eq!(dt.rows[0][0].to_string_value(), "2026-01-01 00:00:00");
    assert_eq!(dt.rows[0][1].to_string_value(), "2026-04-01 00:00:00");
    assert_eq!(dt.rows[0][2].to_string_value(), "2026-05-01 00:00:00");
    assert_eq!(dt.rows[0][3].to_string_value(), "2026-05-11 00:00:00"); // Monday of that week
    assert_eq!(dt.rows[0][4].to_string_value(), "2026-05-14 15:00:00");
    assert_eq!(dt.rows[0][5].to_string_value(), "2026-05-14 15:42:00");

    // DATE_BUCKET with default origin (1900-01-01) and explicit origin
    let db = query(
        &engine,
        "DECLARE @ts DATETIME2 = '2026-05-14 15:42:37';
         DECLARE @orig DATETIME2 = '2026-05-01 00:00:00';
         SELECT
             DATE_BUCKET(day, 7, @ts, @orig) AS b_day7,
             DATE_BUCKET(hour, 4, @ts, @orig) AS b_hr4,
             DATE_BUCKET(month, 3, @ts, '2026-01-01 00:00:00') AS b_mo3;",
    );
    assert_eq!(db.rows[0][0].to_string_value(), "2026-05-08 00:00:00");
    assert_eq!(db.rows[0][1].to_string_value(), "2026-05-14 12:00:00");
    assert_eq!(db.rows[0][2].to_string_value(), "2026-04-01 00:00:00");

    // Non-positive DATE_BUCKET width errors
    assert!(run(&engine, "SELECT DATE_BUCKET(day, 0, '2026-05-14');").is_err());

    // BINARY_CHECKSUM is case-sensitive and supports *
    run(
        &engine,
        "CREATE TABLE dbo.Words (Id INT, W VARCHAR(20));
         INSERT INTO dbo.Words VALUES (1, 'Hello'), (2, 'hello'), (3, 'Hello');",
    )
    .expect("seed words");
    let cs = query(
        &engine,
        "SELECT BINARY_CHECKSUM('Hello'), BINARY_CHECKSUM('hello'), BINARY_CHECKSUM('Hello');",
    );
    assert_ne!(cs.rows[0][0], cs.rows[0][1]);
    assert_eq!(cs.rows[0][0], cs.rows[0][2]);
    let cs_star = query(
        &engine,
        "SELECT BINARY_CHECKSUM(*) FROM dbo.Words ORDER BY Id;",
    );
    assert_eq!(cs_star.rows.len(), 3);
    assert_ne!(cs_star.rows[0][0], cs_star.rows[1][0]);

    // COMPRESS / DECOMPRESS round-trip with real GZIP header (0x1f 0x8b)
    let gz = query(
        &engine,
        "DECLARE @c VARBINARY(MAX) = COMPRESS('hello gzip world');
         SELECT @c AS compressed, CAST(DECOMPRESS(@c) AS VARCHAR(MAX)) AS restored;",
    );
    match &gz.rows[0][0] {
        Value::VarBinary(bytes) => {
            assert!(bytes.len() > 10);
            assert_eq!(&bytes[0..2], &[0x1f, 0x8b], "must be real RFC 1952 GZIP");
        }
        other => panic!("expected VarBinary, got {other:?}"),
    }
    assert_eq!(gz.rows[0][1], Value::VarChar("hello gzip world".into()));
}

#[test]
fn json_objectagg_json_arrayagg_cume_dist_and_ignore_nulls() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Metrics (Grp VARCHAR(10), K VARCHAR(10), V INT, Seq INT);
         INSERT INTO dbo.Metrics VALUES
             ('g1', 'a', 10, 1),
             ('g1', 'b', NULL, 2),
             ('g1', 'c', 30, 3),
             ('g2', 'x', 5, 1),
             ('g2', 'y', 5, 2);",
    )
    .expect("seed metrics");

    // JSON_OBJECTAGG (default NULL ON NULL vs ABSENT ON NULL) & JSON_ARRAYAGG (ORDER BY + NULL handling)
    let agg = query(
        &engine,
        "SELECT
             Grp,
             JSON_OBJECTAGG(K: V) AS obj_default,
             JSON_OBJECTAGG(K: V ABSENT ON NULL) AS obj_absent,
             JSON_ARRAYAGG(V ORDER BY Seq DESC) AS arr_absent_desc,
             JSON_ARRAYAGG(V ORDER BY Seq ASC NULL ON NULL) AS arr_null_asc
         FROM dbo.Metrics
         GROUP BY Grp
         ORDER BY Grp;",
    );
    assert_eq!(agg.rows.len(), 2);
    assert_eq!(
        agg.rows[0][1],
        Value::NVarChar(r#"{"a":10,"b":null,"c":30}"#.into())
    );
    assert_eq!(agg.rows[0][2], Value::NVarChar(r#"{"a":10,"c":30}"#.into()));
    assert_eq!(agg.rows[0][3], Value::NVarChar("[30,10]".into()));
    assert_eq!(agg.rows[0][4], Value::NVarChar("[10,null,30]".into()));

    // CUME_DIST() with ties
    run(
        &engine,
        "CREATE TABLE dbo.Vals (Id INT, Score INT);
         INSERT INTO dbo.Vals VALUES (1, 10), (2, 20), (3, 20), (4, 30);",
    )
    .expect("seed vals");
    let cd = query(
        &engine,
        "SELECT Id, CUME_DIST() OVER (ORDER BY Score) AS cd FROM dbo.Vals ORDER BY Id;",
    );
    let f = |v: &Value| match v {
        Value::Float(bits) => f64::from_bits(*bits),
        _ => panic!("expected Float, got {v:?}"),
    };
    assert!((f(&cd.rows[0][1]) - 0.25).abs() < 1e-9);
    assert!((f(&cd.rows[1][1]) - 0.75).abs() < 1e-9);
    assert!((f(&cd.rows[2][1]) - 0.75).abs() < 1e-9);
    assert!((f(&cd.rows[3][1]) - 1.00).abs() < 1e-9);

    // IGNORE NULLS on LAG, LEAD, FIRST_VALUE, LAST_VALUE
    run(
        &engine,
        "CREATE TABLE dbo.Series (Seq INT, Val INT);
         INSERT INTO dbo.Series VALUES (1, NULL), (2, 10), (3, NULL), (4, NULL), (5, 20);",
    )
    .expect("seed series");
    let ign = query(
        &engine,
        "SELECT
             Seq,
             LAG(Val) IGNORE NULLS OVER (ORDER BY Seq) AS lag_ign,
             LEAD(Val) IGNORE NULLS OVER (ORDER BY Seq) AS lead_ign,
             FIRST_VALUE(Val) IGNORE NULLS OVER (ORDER BY Seq ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS fv_ign,
             LAST_VALUE(Val) IGNORE NULLS OVER (ORDER BY Seq ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS lv_ign
         FROM dbo.Series
         ORDER BY Seq;",
    );
    // Seq 1 (NULL): lag=NULL, lead=10, fv=NULL, lv=NULL
    assert_eq!(ign.rows[0][1], Value::Null);
    assert_eq!(ign.rows[0][2], Value::Int(10));
    assert_eq!(ign.rows[0][3], Value::Null);
    assert_eq!(ign.rows[0][4], Value::Null);
    // Seq 3 (NULL): lag=10, lead=20, fv=10, lv=10
    assert_eq!(ign.rows[2][1], Value::Int(10));
    assert_eq!(ign.rows[2][2], Value::Int(20));
    assert_eq!(ign.rows[2][3], Value::Int(10));
    assert_eq!(ign.rows[2][4], Value::Int(10));
    // Seq 5 (20): lag=10, lead=NULL, fv=10, lv=20
    assert_eq!(ign.rows[4][1], Value::Int(10));
    assert_eq!(ign.rows[4][2], Value::Null);
    assert_eq!(ign.rows[4][3], Value::Int(10));
    assert_eq!(ign.rows[4][4], Value::Int(20));
}
