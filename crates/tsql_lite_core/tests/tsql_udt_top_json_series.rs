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
fn scalar_alias_udts_work_across_tables_vars_procs_cast_and_sys_types() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TYPE dbo.ShortCode FROM VARCHAR(5) NOT NULL;
         CREATE TYPE dbo.Amount FROM DECIMAL(10, 2) NULL;
         CREATE TABLE dbo.Invoices (
             Id INT PRIMARY KEY,
             Code dbo.ShortCode,
             OptCode dbo.ShortCode NULL,
             Total dbo.Amount
         );
         INSERT INTO dbo.Invoices (Id, Code, OptCode, Total)
         VALUES (1, 'ABC', NULL, 123.45);",
    )
    .expect("create scalar UDTs and table");

    // Inherited NOT NULL from dbo.ShortCode rejects NULL insert
    let null_err = run(
        &engine,
        "INSERT INTO dbo.Invoices (Id, Code, OptCode, Total) VALUES (2, NULL, NULL, 10.00);",
    );
    assert!(null_err.is_err(), "NOT NULL scalar UDT must reject NULL");

    // DECLARE variable + procedure parameter + CAST using scalar UDTs
    run(
        &engine,
        "CREATE PROCEDURE dbo.EchoCode @c dbo.ShortCode, @a dbo.Amount AS
         BEGIN
             DECLARE @local dbo.ShortCode = @c;
             SELECT @local AS Code, CAST(@a AS dbo.Amount) AS Amt;
         END;",
    )
    .expect("create proc with scalar UDT params");

    let res = query(&engine, "EXEC dbo.EchoCode 'ABCDEFGH', 42.5;");
    assert_eq!(res.rows.len(), 1);
    assert_eq!(res.rows[0][0], Value::VarChar("ABCDE".to_string()));
    assert_eq!(res.rows[0][1].to_string_value(), "42.50");

    // Visible in sys.types with is_user_defined=1, is_table_type=0, accurate nullability
    let meta = query(
        &engine,
        "SELECT name, is_user_defined, is_table_type, is_nullable, max_length, precision, scale
         FROM sys.types
         WHERE name IN ('ShortCode', 'Amount')
         ORDER BY name;",
    );
    assert_eq!(meta.rows.len(), 2);
    assert_eq!(meta.rows[0][0], Value::VarChar("Amount".to_string()));
    assert_eq!(meta.rows[0][1], Value::Bit(true));
    assert_eq!(meta.rows[0][2], Value::Bit(false));
    assert_eq!(meta.rows[0][3], Value::Bit(true));
    assert_eq!(meta.rows[0][5], Value::TinyInt(10));
    assert_eq!(meta.rows[0][6], Value::TinyInt(2));
    assert_eq!(meta.rows[1][0], Value::VarChar("ShortCode".to_string()));
    assert_eq!(meta.rows[1][3], Value::Bit(false));
    assert_eq!(meta.rows[1][4], Value::SmallInt(5));

    // TYPE_ID / TYPEPROPERTY
    let tp = query(
        &engine,
        "SELECT TYPE_ID('dbo.ShortCode') AS tid,
                TYPEPROPERTY('dbo.ShortCode', 'AllowsNull') AS an,
                TYPEPROPERTY('dbo.ShortCode', 'IsTableType') AS tt;",
    );
    assert!(!tp.rows[0][0].is_null());
    assert_eq!(tp.rows[0][1], Value::Int(0));
    assert_eq!(tp.rows[0][2], Value::Int(0));

    // DROP TYPE works for scalar alias types
    run(&engine, "DROP TYPE dbo.ShortCode;").expect("drop scalar type");
    let gone = query(
        &engine,
        "SELECT name FROM sys.types WHERE name = 'ShortCode';",
    );
    assert!(gone.rows.is_empty());
}

#[test]
fn top_percent_and_top_with_ties_match_sql_server() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Scores (Id INT, Name VARCHAR(20), Score INT);
         INSERT INTO dbo.Scores VALUES
             (1, 'A', 100),
             (2, 'B', 90),
             (3, 'C', 90),
             (4, 'D', 90),
             (5, 'E', 80);",
    )
    .expect("seed scores");

    // TOP 25 PERCENT of 5 rows -> ceil(1.25) = 2 rows
    let pct = query(
        &engine,
        "SELECT TOP (25) PERCENT Name FROM dbo.Scores ORDER BY Score DESC, Id ASC;",
    );
    assert_eq!(pct.rows.len(), 2);
    assert_eq!(pct.rows[0][0], Value::VarChar("A".to_string()));
    assert_eq!(pct.rows[1][0], Value::VarChar("B".to_string()));

    // TOP 2 WITH TIES with non-projected ORDER BY column includes all 90-score ties (4 rows)
    let ties = query(
        &engine,
        "SELECT TOP 2 WITH TIES Name FROM dbo.Scores ORDER BY Score DESC;",
    );
    assert_eq!(ties.rows.len(), 4);

    // TOP 40 PERCENT WITH TIES -> ceil(2.0)=2 -> extends across all 90s -> 4 rows
    let pct_ties = query(
        &engine,
        "SELECT TOP 40 PERCENT WITH TIES Name FROM dbo.Scores ORDER BY Score DESC;",
    );
    assert_eq!(pct_ties.rows.len(), 4);

    // TOP WITH TIES without ORDER BY must error
    assert!(run(&engine, "SELECT TOP 2 WITH TIES Name FROM dbo.Scores;").is_err());
    // Out-of-range PERCENT must error
    assert!(run(&engine, "SELECT TOP (150) PERCENT Name FROM dbo.Scores;").is_err());
}

#[test]
fn json_object_json_array_and_generate_series_match_sql_server() {
    let engine = Engine::new();

    // JSON_OBJECT default (NULL ON NULL) vs ABSENT ON NULL + nested JSON_ARRAY
    let j1 = query(
        &engine,
        "SELECT
            JSON_OBJECT('a': 1, 'b': NULL, 'c': JSON_ARRAY(10, NULL, 20)) AS obj_default,
            JSON_OBJECT('a': 1, 'b': NULL ABSENT ON NULL) AS obj_absent,
            JSON_ARRAY(1, NULL, 'hi' NULL ON NULL) AS arr_null,
            JSON_ARRAY(1, NULL, 'hi') AS arr_absent;",
    );
    assert_eq!(
        j1.rows[0][0],
        Value::NVarChar(r#"{"a":1,"b":null,"c":[10,20]}"#.to_string())
    );
    assert_eq!(j1.rows[0][1], Value::NVarChar(r#"{"a":1}"#.to_string()));
    assert_eq!(
        j1.rows[0][2],
        Value::NVarChar(r#"[1,null,"hi"]"#.to_string())
    );
    assert_eq!(j1.rows[0][3], Value::NVarChar(r#"[1,"hi"]"#.to_string()));

    // GENERATE_SERIES ascending, descending, stepped, decimal, and empty direction
    let gs1 = query(
        &engine,
        "SELECT value FROM GENERATE_SERIES(1, 5, 2) ORDER BY value;",
    );
    assert_eq!(
        gs1.rows,
        vec![
            vec![Value::Int(1)],
            vec![Value::Int(3)],
            vec![Value::Int(5)]
        ]
    );

    let gs_desc = query(&engine, "SELECT value FROM GENERATE_SERIES(5, 3);");
    assert_eq!(
        gs_desc.rows,
        vec![
            vec![Value::Int(5)],
            vec![Value::Int(4)],
            vec![Value::Int(3)]
        ]
    );

    let gs_dec = query(&engine, "SELECT value FROM GENERATE_SERIES(0.0, 1.0, 0.5);");
    assert_eq!(gs_dec.rows.len(), 3);
    assert_eq!(gs_dec.rows[0][0].to_string_value(), "0.0");
    assert_eq!(gs_dec.rows[1][0].to_string_value(), "0.5");
    assert_eq!(gs_dec.rows[2][0].to_string_value(), "1.0");

    let gs_empty = query(&engine, "SELECT value FROM GENERATE_SERIES(10, 1, 1);");
    assert!(gs_empty.rows.is_empty());

    // Zero step errors
    assert!(run(&engine, "SELECT value FROM GENERATE_SERIES(1, 5, 0);").is_err());
}
