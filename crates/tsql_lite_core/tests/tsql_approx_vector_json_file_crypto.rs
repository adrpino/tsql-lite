use tsql_lite_core::types::Value;
use tsql_lite_core::{Engine, QueryResult};

fn run_sql(engine: &Engine, sql: &str) -> QueryResult {
    let stmts = tsql_lite_core::parse_batch(sql).expect("parse batch");
    engine
        .execute_batch(stmts)
        .expect("execute batch")
        .unwrap_or_default()
}

fn run_sql_err(engine: &Engine, sql: &str) -> String {
    let stmts = tsql_lite_core::parse_batch(sql).expect("parse batch");
    engine
        .execute_batch(stmts)
        .expect_err("expected SQL error")
        .to_string()
}

#[test]
fn approx_aggregates_count_distinct_and_percentiles() {
    let engine = Engine::new();
    run_sql(
        &engine,
        "CREATE TABLE dbo.Metrics (Grp VARCHAR(10), Score INT NULL);
         INSERT INTO dbo.Metrics VALUES
           ('A', 10), ('A', 20), ('A', 30), ('A', 40), ('A', 20), ('A', NULL),
           ('B', 100), ('B', 200), ('B', 300);",
    );

    let res = run_sql(
        &engine,
        "SELECT
             Grp,
             APPROX_COUNT_DISTINCT(Score) AS DistCnt,
             APPROX_PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY Score) AS MedCont,
             APPROX_PERCENTILE_DISC(0.5) WITHIN GROUP (ORDER BY Score) AS MedDisc,
             APPROX_PERCENTILE_DISC(0.0) WITHIN GROUP (ORDER BY Score) AS MinDisc,
             APPROX_PERCENTILE_DISC(1.0) WITHIN GROUP (ORDER BY Score) AS MaxDisc
         FROM dbo.Metrics
         GROUP BY Grp
         ORDER BY Grp;",
    );
    assert_eq!(res.rows.len(), 2);
    // Group A non-null sorted: [10, 20, 20, 30, 40] -> distinct=4, cont(0.5)=20.0, disc(0.5)=20, min=10, max=40
    assert_eq!(res.rows[0][0], Value::VarChar("A".into()));
    assert_eq!(res.rows[0][1], Value::BigInt(4));
    assert_eq!(res.rows[0][2], Value::Float(20.0f64.to_bits()));
    assert_eq!(res.rows[0][3], Value::Int(20));
    assert_eq!(res.rows[0][4], Value::Int(10));
    assert_eq!(res.rows[0][5], Value::Int(40));

    // Group B non-null sorted: [100, 200, 300] -> distinct=3, cont(0.5)=200.0, disc(0.5)=200
    assert_eq!(res.rows[1][0], Value::VarChar("B".into()));
    assert_eq!(res.rows[1][1], Value::BigInt(3));
    assert_eq!(res.rows[1][2], Value::Float(200.0f64.to_bits()));
    assert_eq!(res.rows[1][3], Value::Int(200));

    // Continuous interpolation on even-sized group: [10, 20, 30, 40] -> 0.5 => 25.0
    run_sql(&engine, "CREATE TABLE dbo.EvenScores (V INT); INSERT INTO dbo.EvenScores VALUES (10), (20), (30), (40);");
    let even = run_sql(
        &engine,
        "SELECT APPROX_PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY V),
                APPROX_PERCENTILE_DISC(0.5) WITHIN GROUP (ORDER BY V)
         FROM dbo.EvenScores;",
    );
    assert_eq!(even.rows[0][0], Value::Float(25.0f64.to_bits()));
    assert_eq!(even.rows[0][1], Value::Int(20));

    // Out-of-range percentile errors
    let err = run_sql_err(
        &engine,
        "SELECT APPROX_PERCENTILE_CONT(1.5) WITHIN GROUP (ORDER BY V) FROM dbo.EvenScores;",
    );
    assert!(
        err.contains("between 0.0 and 1.0"),
        "unexpected error: {err}"
    );
}

#[test]
fn sql2025_json_contains_and_vector_functions() {
    let engine = Engine::new();

    // JSON_CONTAINS: equality, wildcards, LIKE search_mode=1, missing path -> NULL
    let jc = run_sql(
        &engine,
        r#"DECLARE @doc NVARCHAR(MAX) = N'{"user":"Alice","score":42,"active":true,"skills":["SQL","Python","Rust"],"items":[{"id":10},{"id":20}]}';
           SELECT
               JSON_CONTAINS(@doc, 'Python', '$.skills[*]'),
               JSON_CONTAINS(@doc, 'Java', '$.skills[*]'),
               JSON_CONTAINS(@doc, 'Py%', '$.skills[*]', 1),
               JSON_CONTAINS(@doc, 20, '$.items[*].id'),
               JSON_CONTAINS(@doc, 99, '$.items[*].id'),
               JSON_CONTAINS(@doc, 42, '$.score'),
               JSON_CONTAINS(@doc, 'Alice'),
               JSON_CONTAINS(@doc, 'x', '$.missing');"#,
    );
    assert_eq!(
        jc.rows,
        vec![vec![
            Value::Int(1),
            Value::Int(0),
            Value::Int(1),
            Value::Int(1),
            Value::Int(0),
            Value::Int(1),
            Value::Int(1),
            Value::Null,
        ]]
    );

    // VECTOR_NORM, VECTOR_NORMALIZE, VECTORPROPERTY
    let vr = run_sql(
        &engine,
        "DECLARE @v VECTOR(2) = '[3.0, -4.0]';
         DECLARE @n VECTOR(2) = VECTOR_NORMALIZE(@v, 'norm2');
         SELECT
             VECTOR_NORM(@v, 'norm1'),
             VECTOR_NORM(@v, 'norm2'),
             VECTOR_NORM(@v, 'norminf'),
             VECTORPROPERTY(@v, 'Dimensions'),
             VECTORPROPERTY(@v, 'BaseType'),
             VECTOR_NORM(@n, 'norm2');",
    );
    let norm1 = f64::from_bits(match vr.rows[0][0] {
        Value::Float(b) => b,
        ref other => panic!("expected Float, got {other:?}"),
    });
    let norm2 = f64::from_bits(match vr.rows[0][1] {
        Value::Float(b) => b,
        ref other => panic!("expected Float, got {other:?}"),
    });
    let norminf = f64::from_bits(match vr.rows[0][2] {
        Value::Float(b) => b,
        ref other => panic!("expected Float, got {other:?}"),
    });
    let normalized_norm2 = f64::from_bits(match vr.rows[0][5] {
        Value::Float(b) => b,
        ref other => panic!("expected Float, got {other:?}"),
    });
    assert!((norm1 - 7.0).abs() < 1e-6);
    assert!((norm2 - 5.0).abs() < 1e-6);
    assert!((norminf - 4.0).abs() < 1e-6);
    assert_eq!(vr.rows[0][3], Value::Int(2));
    assert_eq!(vr.rows[0][4], Value::NVarChar("float".into()));
    assert!((normalized_norm2 - 1.0).abs() < 1e-6);

    // Zero vector normalizes cleanly without NaN
    let z = run_sql(
        &engine,
        "DECLARE @z VECTOR(2) = '[0.0, 0.0]';
         SELECT VECTOR_NORM(VECTOR_NORMALIZE(@z, 'norm2'), 'norm2');",
    );
    assert_eq!(z.rows[0][0], Value::Float(0.0f64.to_bits()));
}

#[test]
fn filegroup_and_file_metadata_functions() {
    let engine = Engine::new();
    run_sql(&engine, "ALTER DATABASE master ADD FILEGROUP FG_ARCHIVE;");

    let res = run_sql(
        &engine,
        "SELECT
             FILEGROUP_ID('PRIMARY'),
             FILEGROUP_NAME(1),
             FILEGROUPPROPERTY('PRIMARY', 'IsDefault'),
             FILEGROUPPROPERTY('PRIMARY', 'IsUserDefinedFG'),
             FILEGROUPPROPERTY('PRIMARY', 'IsReadOnly'),
             FILEGROUP_NAME(FILEGROUP_ID('FG_ARCHIVE')),
             FILEGROUPPROPERTY('FG_ARCHIVE', 'IsUserDefinedFG'),
             FILEGROUP_ID('NO_SUCH_FG'),
             FILE_ID(FILE_NAME(1)),
             FILE_ID(FILE_NAME(2)),
             FILEPROPERTY(FILE_NAME(1), 'IsPrimaryFile'),
             FILEPROPERTY(FILE_NAME(1), 'IsLogFile'),
             FILEPROPERTY(FILE_NAME(2), 'IsLogFile'),
             FILE_NAME(999);",
    );
    assert_eq!(
        res.rows,
        vec![vec![
            Value::Int(1),
            Value::NVarChar("PRIMARY".into()),
            Value::Int(1),
            Value::Int(0),
            Value::Int(0),
            Value::NVarChar("FG_ARCHIVE".into()),
            Value::Int(1),
            Value::Null,
            Value::SmallInt(1),
            Value::SmallInt(2),
            Value::Int(1),
            Value::Int(0),
            Value::Int(1),
            Value::Null,
        ]]
    );
}

#[test]
fn crypt_gen_random_and_passphrase_encryption_roundtrip() {
    let engine = Engine::new();

    // CRYPT_GEN_RANDOM length, uniqueness, optional seed, and bounds
    let rnd = run_sql(
        &engine,
        "DECLARE @r1 VARBINARY(32) = CRYPT_GEN_RANDOM(32);
         DECLARE @r2 VARBINARY(32) = CRYPT_GEN_RANDOM(32, 0x01020304);
         SELECT
             DATALENGTH(@r1),
             DATALENGTH(@r2),
             CASE WHEN @r1 <> @r2 THEN 1 ELSE 0 END,
             CRYPT_GEN_RANDOM(0),
             CRYPT_GEN_RANDOM(8001);",
    );
    assert_eq!(
        rnd.rows,
        vec![vec![
            Value::Int(32),
            Value::Int(32),
            Value::Int(1),
            Value::Null,
            Value::Null,
        ]]
    );

    // ENCRYPTBYPASSPHRASE / DECRYPTBYPASSPHRASE with and without authenticator + wrong passphrase -> NULL
    let enc = run_sql(
        &engine,
        "DECLARE @c1 VARBINARY(8000) = ENCRYPTBYPASSPHRASE('s3cr3t', 'hello world');
         DECLARE @c2 VARBINARY(8000) = ENCRYPTBYPASSPHRASE('s3cr3t', 'auth payload', 1, 'row-42');
         SELECT
             CAST(DECRYPTBYPASSPHRASE('s3cr3t', @c1) AS VARCHAR(100)),
             DECRYPTBYPASSPHRASE('wrong-pw', @c1),
             CAST(DECRYPTBYPASSPHRASE('s3cr3t', @c2, 1, 'row-42') AS VARCHAR(100)),
             DECRYPTBYPASSPHRASE('s3cr3t', @c2, 1, 'row-99'),
             DECRYPTBYPASSPHRASE('s3cr3t', @c2),
             DECRYPTBYPASSPHRASE('s3cr3t', 0xDEADBEEF);",
    );
    assert_eq!(
        enc.rows,
        vec![vec![
            Value::VarChar("hello world".into()),
            Value::Null,
            Value::VarChar("auth payload".into()),
            Value::Null,
            Value::Null,
            Value::Null,
        ]]
    );
}
