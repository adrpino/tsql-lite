use iridium_core::executor::result::QueryResult;
use iridium_core::types::Value;
use iridium_core::{parse_batch, DbError, Engine};

fn run(engine: &Engine, sql: &str) -> Result<Option<QueryResult>, DbError> {
    let stmts = parse_batch(sql)?;
    engine.execute_batch(stmts)
}

fn query(engine: &Engine, sql: &str) -> Vec<Vec<Value>> {
    run(engine, sql)
        .expect("query execution failed")
        .expect("expected result set")
        .rows
}

fn val_as_f64(v: &Value) -> f64 {
    match v {
        Value::Float(bits) => f64::from_bits(*bits),
        other => panic!("expected Float, got {:?}", other),
    }
}

#[test]
fn aggregates_count_big_stats_checksum_agg_and_distinct() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Metrics (Grp INT, Val INT);
         INSERT INTO dbo.Metrics VALUES
            (1, 2), (1, 4), (1, 4), (1, 4), (1, 5), (1, 5), (1, 7), (1, 9), (1, NULL),
            (2, 10);",
    )
    .unwrap();

    let rows = query(
        &engine,
        "SELECT
            Grp,
            COUNT_BIG(*) AS c_all,
            COUNT_BIG(Val) AS c_val,
            COUNT_BIG(DISTINCT Val) AS c_dist,
            SUM(DISTINCT Val) AS sum_dist,
            AVG(DISTINCT Val) AS avg_dist,
            STDEV(Val) AS sd,
            STDEVP(Val) AS sdp,
            VAR(Val) AS v,
            VARP(Val) AS vp,
            CHECKSUM_AGG(Val) AS chk_all,
            CHECKSUM_AGG(DISTINCT Val) AS chk_dist
         FROM dbo.Metrics
         GROUP BY Grp
         ORDER BY Grp;",
    );
    assert_eq!(rows.len(), 2);

    // Group 1: values = [2, 4, 4, 4, 5, 5, 7, 9], distinct = [2, 4, 5, 7, 9]
    assert_eq!(rows[0][0], Value::Int(1));
    assert_eq!(rows[0][1], Value::BigInt(9));
    assert_eq!(rows[0][2], Value::BigInt(8));
    assert_eq!(rows[0][3], Value::BigInt(5));
    assert_eq!(rows[0][4], Value::BigInt(27)); // 2+4+5+7+9
    assert_eq!(rows[0][5], Value::Int(5)); // 27 / 5 = 5 (integer avg)
    assert!((val_as_f64(&rows[0][6]) - (32.0_f64 / 7.0).sqrt()).abs() < 1e-9);
    assert!((val_as_f64(&rows[0][7]) - 2.0).abs() < 1e-9);
    assert!((val_as_f64(&rows[0][8]) - (32.0_f64 / 7.0)).abs() < 1e-9);
    assert!((val_as_f64(&rows[0][9]) - 4.0).abs() < 1e-9);
    assert_eq!(rows[0][10], Value::Int(2 ^ 4 ^ 4 ^ 4 ^ 5 ^ 5 ^ 7 ^ 9));
    assert_eq!(rows[0][11], Value::Int(2 ^ 4 ^ 5 ^ 7 ^ 9));

    // Group 2: single value -> sample STDEV/VAR are NULL; population STDEVP/VARP are 0.0
    assert_eq!(rows[1][0], Value::Int(2));
    assert_eq!(rows[1][6], Value::Null);
    assert!((val_as_f64(&rows[1][7]) - 0.0).abs() < 1e-9);
    assert_eq!(rows[1][8], Value::Null);
    assert!((val_as_f64(&rows[1][9]) - 0.0).abs() < 1e-9);
    assert_eq!(rows[1][10], Value::Int(10));

    // Windowed COUNT_BIG and STDEVP OVER ()
    let win_rows = query(
        &engine,
        "SELECT Grp, COUNT_BIG(Val) OVER (PARTITION BY Grp), STDEVP(Val) OVER (PARTITION BY Grp)
         FROM dbo.Metrics
         WHERE Grp = 2;",
    );
    assert_eq!(win_rows[0][1], Value::BigInt(1));
    assert!((val_as_f64(&win_rows[0][2]) - 0.0).abs() < 1e-9);
}

#[test]
fn datetime_constructors_and_timezone_helpers() {
    let engine = Engine::new();
    let rows = query(
        &engine,
        "SELECT
            TIMEFROMPARTS(14, 30, 15, 500, 3),
            SMALLDATETIMEFROMPARTS(2025, 6, 15, 9, 45),
            DATETIMEOFFSETFROMPARTS(2025, 6, 15, 14, 30, 15, 5, 2, 30, 1),
            DATETIMEOFFSETFROMPARTS(2025, 6, 15, 14, 30, 15, 0, -5, 0, 0),
            CURRENT_TIMEZONE(),
            CURRENT_TIMEZONE_ID();",
    );
    assert_eq!(rows[0][0].to_string_value(), "14:30:15.500");
    assert_eq!(rows[0][1].to_string_value(), "2025-06-15 09:45:00");
    assert_eq!(
        rows[0][2],
        Value::DateTimeOffset("2025-06-15 14:30:15.5 +02:30".into())
    );
    assert_eq!(
        rows[0][3],
        Value::DateTimeOffset("2025-06-15 14:30:15 -05:00".into())
    );
    assert!(rows[0][4].to_string_value().contains("UTC"));
    assert_eq!(rows[0][5], Value::NVarChar("UTC".into()));
}

#[test]
fn bitwise_sql2022_functions() {
    let engine = Engine::new();
    let rows = query(
        &engine,
        "SELECT
            LEFT_SHIFT(1, 4),
            RIGHT_SHIFT(32, 3),
            BIT_COUNT(CAST(21 AS INT)),
            GET_BIT(CAST(21 AS INT), 0),
            GET_BIT(CAST(21 AS INT), 1),
            GET_BIT(CAST(21 AS INT), 2),
            SET_BIT(CAST(0 AS INT), 3),
            SET_BIT(CAST(15 AS INT), 1, 0),
            BIT_COUNT(0xFF0F);",
    );
    assert_eq!(rows[0][0], Value::Int(16));
    assert_eq!(rows[0][1], Value::Int(4));
    assert_eq!(rows[0][2], Value::BigInt(3)); // 21 = 0b10101 -> 3 ones
    assert_eq!(rows[0][3], Value::Bit(true));
    assert_eq!(rows[0][4], Value::Bit(false));
    assert_eq!(rows[0][5], Value::Bit(true));
    assert_eq!(rows[0][6], Value::Int(8));
    assert_eq!(rows[0][7], Value::Int(13));
    assert_eq!(rows[0][8], Value::BigInt(12)); // 8 + 4 bits
}

#[test]
fn system_functions_isjson_two_arg_and_version_parity() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Items (Id INT, Rv ROWVERSION);
         INSERT INTO dbo.Items (Id) VALUES (1), (2);",
    )
    .unwrap();

    let rows = query(
        &engine,
        "SELECT Id FROM dbo.Items;
         SELECT
            ROWCOUNT_BIG(),
            NEWSEQUENTIALID(),
            NEWSEQUENTIALID(),
            MIN_ACTIVE_ROWVERSION(),
            CURRENT_TRANSACTION_ID(),
            GETANSINULL(),
            @@VERSION,
            @@SERVERNAME,
            ISJSON('{\"a\":1}', OBJECT),
            ISJSON('{\"a\":1}', ARRAY),
            ISJSON('[1,2]', ARRAY),
            ISJSON('42', SCALAR),
            ISJSON('\"hi\"', VALUE),
            ISJSON('not json', VALUE);",
    );
    assert_eq!(rows[0][0], Value::BigInt(2));
    match (&rows[0][1], &rows[0][2]) {
        (Value::UniqueIdentifier(u1), Value::UniqueIdentifier(u2)) => {
            assert_ne!(u1, u2);
            assert!(u1.as_bytes() < u2.as_bytes());
        }
        other => panic!("expected two UniqueIdentifiers, got {:?}", other),
    }
    match &rows[0][3] {
        Value::Binary(b) => assert_eq!(b.len(), 8),
        other => panic!("expected 8-byte Binary rowversion, got {:?}", other),
    }
    match &rows[0][4] {
        Value::BigInt(id) => assert!(*id > 0),
        other => panic!("expected positive BigInt tx id, got {:?}", other),
    }
    assert_eq!(rows[0][5], Value::Int(1));
    assert!(
        rows[0][6].to_string_value().contains("SQL Server"),
        "@@VERSION should report SQL Server, got: {}",
        rows[0][6].to_string_value()
    );
    assert_eq!(rows[0][7], Value::NVarChar("localhost".into()));
    assert_eq!(rows[0][8], Value::Bit(true));
    assert_eq!(rows[0][9], Value::Bit(false));
    assert_eq!(rows[0][10], Value::Bit(true));
    assert_eq!(rows[0][11], Value::Bit(true));
    assert_eq!(rows[0][12], Value::Bit(true));
    assert_eq!(rows[0][13], Value::Bit(false));
}
