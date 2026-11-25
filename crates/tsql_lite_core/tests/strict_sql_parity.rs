use tsql_lite_core::error::DbError;
use tsql_lite_core::types::Value;
use tsql_lite_core::{parse_batch, Engine, QueryResult};

fn run_batch(engine: &Engine, sql: &str) -> Result<Option<QueryResult>, DbError> {
    let stmts = parse_batch(sql)?;
    engine.execute_batch(stmts)
}

#[test]
fn xml_rejects_numeric_and_temporal_conversions() {
    let engine = Engine::new();

    // INT -> XML via DECLARE, SET, and CAST must fail with error 529.
    let err =
        run_batch(&engine, "DECLARE @x XML = 123;").expect_err("DECLARE @x XML = 123 must fail");
    assert_eq!(err.number(), 529, "unexpected err: {err}");

    let err = run_batch(&engine, "SELECT CAST(42 AS XML);").expect_err("CAST(42 AS XML) must fail");
    assert_eq!(err.number(), 529, "unexpected err: {err}");

    let err = run_batch(&engine, "DECLARE @x XML; SET @x = CAST(1 AS BIT);")
        .expect_err("BIT -> XML must fail");
    assert_eq!(err.number(), 529, "unexpected err: {err}");

    let err = run_batch(
        &engine,
        "DECLARE @x XML; SET @x = CAST('2025-01-01' AS DATE);",
    )
    .expect_err("DATE -> XML must fail");
    assert_eq!(err.number(), 529, "unexpected err: {err}");

    // TRY_CAST of invalid XML string returns NULL instead of throwing.
    let qr = run_batch(&engine, "SELECT TRY_CAST('<unclosed' AS XML) AS bad;")
        .expect("TRY_CAST should return NULL")
        .unwrap();
    assert!(qr.rows[0][0].is_null());
}

#[test]
fn xml_rejects_implicit_conversion_from_xml_var_but_allows_explicit_cast() {
    let engine = Engine::new();

    let err = run_batch(
        &engine,
        "DECLARE @x XML = '<a/>'; DECLARE @s VARCHAR(50) = @x;",
    )
    .expect_err("implicit XML -> VARCHAR via DECLARE must fail");
    assert_eq!(err.number(), 257, "unexpected err: {err}");

    let err = run_batch(
        &engine,
        "DECLARE @x XML = '<a/>'; DECLARE @s NVARCHAR(50); SET @s = @x;",
    )
    .expect_err("implicit XML -> NVARCHAR via SET must fail");
    assert_eq!(err.number(), 257, "unexpected err: {err}");

    let err = run_batch(
        &engine,
        "DECLARE @x XML = '<a/>'; DECLARE @s VARCHAR(50); SELECT @s = @x;",
    )
    .expect_err("implicit XML -> VARCHAR via SELECT assign must fail");
    assert_eq!(err.number(), 257, "unexpected err: {err}");

    // Explicit CAST / CONVERT must succeed.
    let qr = run_batch(
        &engine,
        "DECLARE @x XML = '<a/>'; DECLARE @s VARCHAR(50) = CAST(@x AS VARCHAR(50)); SELECT @s;",
    )
    .expect("explicit CAST(@x AS VARCHAR) must succeed")
    .unwrap();
    assert_eq!(qr.rows[0][0], Value::VarChar("<a/>".to_string()));

    // XML -> XML assignment must succeed.
    run_batch(
        &engine,
        "DECLARE @x XML = '<a/>'; DECLARE @y XML = @x; SET @y = @x;",
    )
    .expect("XML -> XML must succeed");
}

#[test]
fn xml_validates_well_formedness_for_documents_and_fragments() {
    let engine = Engine::new();

    // Valid SQL Server XML forms: empty, text fragment, multi-root fragment, PI, comment, CDATA, entities.
    for valid in [
        "",
        "plain text",
        "<a/>",
        "<a/><b/>",
        "<?xml version=\"1.0\"?><root attr=\"1 &amp; 2\"><!-- ok --><![CDATA[<raw & text>]]></root>",
        "<ns:item xmlns:ns='urn:test' id='&#65;&#x42;'>ok</ns:item>",
    ] {
        let sql = format!("DECLARE @x XML = '{}';", valid.replace('\'', "''"));
        engine
            .exec(&sql)
            .unwrap_or_else(|e| panic!("valid XML `{valid}` failed: {e}"));
    }

    // Invalid XML forms must fail with XML parsing errors (9436 for mismatched end tag, 9400 otherwise).
    for (invalid, expected_code) in [
        ("<a>", 9400),
        ("<a></b>", 9436),
        ("</a>", 9400),
        ("<a x=1/>", 9400),
        ("<a x=\"1\" x=\"2\"/>", 9400),
        ("<a x=\"1<2\"/>", 9400),
        ("a & b", 9400),
        ("<a/>&unknown;", 9400),
        ("<!-- unclosed", 9400),
        ("<!-- bad -- comment -->", 9400),
        ("<![CDATA[unclosed", 9400),
    ] {
        let sql = format!("DECLARE @x XML = '{}';", invalid.replace('\'', "''"));
        let err = engine
            .exec(&sql)
            .expect_err(&format!("invalid XML `{invalid}` should have failed"));
        assert_eq!(
            err.number(),
            expected_code,
            "invalid XML `{invalid}` gave wrong error: {err}"
        );
    }
}

#[test]
fn identifier_length_limits_enforced_with_error_103() {
    let engine = Engine::new();

    let ok_128 = "a".repeat(128);
    let too_long_129 = "a".repeat(129);
    let ok_temp_116 = format!("#{}", "t".repeat(115));
    let too_long_temp_117 = format!("#{}", "t".repeat(116));
    let ok_var_128 = format!("@{}", "v".repeat(127));
    let too_long_var_129 = format!("@{}", "v".repeat(128));

    // Boundary 128 chars succeeds.
    engine
        .exec(&format!("CREATE TABLE dbo.[{ok_128}] (Id INT);"))
        .expect("128-char table name must succeed");
    engine
        .exec(&format!("CREATE TABLE {ok_temp_116} (Id INT);"))
        .expect("116-char local temp table must succeed");
    engine
        .exec(&format!("DECLARE {ok_var_128} INT = 1;"))
        .expect("128-char variable name must succeed");

    // 129-char regular / bracketed / variable and 117-char #temp fail with error 103.
    let err = engine
        .exec(&format!("CREATE TABLE dbo.{too_long_129} (Id INT);"))
        .expect_err("129-char bare identifier must fail");
    assert_eq!(err.number(), 103, "unexpected err: {err}");

    let err = engine
        .exec(&format!("CREATE TABLE dbo.[{too_long_129}] (Id INT);"))
        .expect_err("129-char bracketed identifier must fail");
    assert_eq!(err.number(), 103, "unexpected err: {err}");

    let err = engine
        .exec(&format!("CREATE TABLE {too_long_temp_117} (Id INT);"))
        .expect_err("117-char #temp table must fail");
    assert_eq!(err.number(), 103, "unexpected err: {err}");

    let err = engine
        .exec(&format!("DECLARE {too_long_var_129} INT = 1;"))
        .expect_err("129-char @variable must fail");
    assert_eq!(err.number(), 103, "unexpected err: {err}");

    // String literals > 128 chars remain completely unaffected.
    let long_str = "x".repeat(500);
    run_batch(&engine, &format!("SELECT '{long_str}';")).expect("long string literal must succeed");
}

#[test]
fn schemabinding_creation_checks_reject_star_one_part_and_self_ref() {
    let engine = Engine::new();
    engine
        .exec("CREATE TABLE dbo.Items (Id INT NOT NULL, Name NVARCHAR(50));")
        .unwrap();

    // SELECT * rejected with 1054.
    let err = engine
        .exec("CREATE VIEW dbo.vStar WITH SCHEMABINDING AS SELECT * FROM dbo.Items;")
        .expect_err("SELECT * in SCHEMABINDING view must fail");
    assert_eq!(err.number(), 1054, "unexpected err: {err}");

    // Qualified t.* rejected with 1054.
    let err = engine
        .exec("CREATE VIEW dbo.vQStar WITH SCHEMABINDING AS SELECT i.* FROM dbo.Items i;")
        .expect_err("SELECT i.* in SCHEMABINDING view must fail");
    assert_eq!(err.number(), 1054, "unexpected err: {err}");

    // Inline TVF SELECT * rejected with 1054.
    let err = engine
        .exec(
            "CREATE FUNCTION dbo.fnStar() RETURNS TABLE WITH SCHEMABINDING AS RETURN (SELECT * FROM dbo.Items);",
        )
        .expect_err("SELECT * in SCHEMABINDING TVF must fail");
    assert_eq!(err.number(), 1054, "unexpected err: {err}");

    // 1-part table name rejected with 4512.
    let err = engine
        .exec("CREATE VIEW dbo.vOnePart WITH SCHEMABINDING AS SELECT Id FROM Items;")
        .expect_err("1-part table name in SCHEMABINDING view must fail");
    assert_eq!(err.number(), 4512, "unexpected err: {err}");

    // 3-part table name rejected with 4512.
    let err = engine
        .exec("CREATE VIEW dbo.vThreePart WITH SCHEMABINDING AS SELECT Id FROM OtherDb.dbo.Items;")
        .expect_err("3-part table name in SCHEMABINDING view must fail");
    assert_eq!(err.number(), 4512, "unexpected err: {err}");

    // Self-reference rejected with 4512.
    let err = engine
        .exec("CREATE VIEW dbo.vSelf WITH SCHEMABINDING AS SELECT Id FROM dbo.vSelf;")
        .expect_err("self-referencing SCHEMABINDING view must fail");
    assert_eq!(err.number(), 4512, "unexpected err: {err}");

    // Valid SCHEMABINDING: explicit columns, 2-part base table, COUNT(*), and 1-part CTE reference!
    engine
        .exec(
            "CREATE VIEW dbo.vValid WITH SCHEMABINDING AS
             WITH C AS (SELECT Id FROM dbo.Items)
             SELECT COUNT(*) AS Cnt FROM C;",
        )
        .expect("valid SCHEMABINDING view with CTE and COUNT(*) must succeed");

    // Non-SCHEMABINDING view still allows SELECT * and 1-part names.
    engine
        .exec("CREATE VIEW dbo.vLoose AS SELECT * FROM Items;")
        .expect("non-SCHEMABINDING view must allow SELECT * and 1-part names");
}

#[test]
fn verbatim_definition_sql_preserved_for_view_proc_func_and_trigger() {
    let engine = Engine::new();
    engine
        .exec("CREATE TABLE dbo.AuditTarget (Id INT NOT NULL PRIMARY KEY, Note NVARCHAR(50));")
        .unwrap();

    let view_sql = "-- leading view comment\nCREATE VIEW dbo.v_Verbatim\nAS\n    SELECT   Id, /* inline */ Note\n    FROM dbo.AuditTarget; -- trailing";
    let proc_sql = "/* proc header */\nCREATE PROCEDURE dbo.usp_Verbatim\n    @Val INT = 1\nAS\nBEGIN\n    -- body comment\n    SELECT @Val AS ResultVal;\nEND;";
    let fn_sql = "CREATE FUNCTION dbo.ufn_Verbatim(@x INT)\nRETURNS INT\nAS\nBEGIN\n    -- double it\n    RETURN @x * 2;\nEND;";
    let trig_sql = "CREATE TRIGGER dbo.trg_Verbatim\nON dbo.AuditTarget\nAFTER INSERT\nAS\nBEGIN\n    -- noop trigger\n    SET NOCOUNT ON;\nEND;";

    engine.exec(view_sql).unwrap();
    engine.exec(proc_sql).unwrap();
    engine.exec(fn_sql).unwrap();
    engine.exec(trig_sql).unwrap();

    for (obj, expected) in [
        ("dbo.v_Verbatim", view_sql),
        ("dbo.usp_Verbatim", proc_sql),
        ("dbo.ufn_Verbatim", fn_sql),
        ("dbo.trg_Verbatim", trig_sql),
    ] {
        let q1 = engine
            .query(&format!("SELECT OBJECT_DEFINITION(OBJECT_ID('{obj}'));"))
            .unwrap();
        assert_eq!(q1.rows[0][0].to_string_value(), expected);

        let q2 = engine
            .query(&format!(
                "SELECT definition FROM sys.sql_modules WHERE object_id = OBJECT_ID('{obj}');"
            ))
            .unwrap();
        assert_eq!(q2.rows[0][0].to_string_value(), expected);

        let q3 = engine.query(&format!("EXEC sp_helptext '{obj}';")).unwrap();
        assert_eq!(q3.rows[0][0].to_string_value(), expected);
    }

    // ALTER updates verbatim definition and normalizes leading ALTER -> CREATE like SQL Server.
    let altered_view =
        "-- v2 header\nALTER VIEW dbo.v_Verbatim AS SELECT Id FROM dbo.AuditTarget; -- v2";
    engine.exec(altered_view).unwrap();
    let q_alt = engine
        .query("SELECT OBJECT_DEFINITION(OBJECT_ID('dbo.v_Verbatim'));")
        .unwrap();
    assert_eq!(
        q_alt.rows[0][0].to_string_value(),
        "-- v2 header\nCREATE VIEW dbo.v_Verbatim AS SELECT Id FROM dbo.AuditTarget; -- v2"
    );
}

#[test]
fn hash_in_subquery_three_valued_nulls_and_large_join_ordering_performance() {
    let engine = Engine::new();
    run_batch(
        &engine,
        "CREATE TABLE dbo.L (Id INT, Val INT);
         CREATE TABLE dbo.R (Id INT);
         CREATE TABLE dbo.REmpty (Id INT);
         CREATE TABLE dbo.RWithNull (Id INT);
         INSERT INTO dbo.L VALUES (1, 10), (2, 20), (3, 30), (NULL, 40);
         INSERT INTO dbo.R VALUES (1), (3);
         INSERT INTO dbo.RWithNull VALUES (1), (NULL);",
    )
    .unwrap();

    // Standard IN
    let qr = engine
        .query("SELECT Id FROM dbo.L WHERE Id IN (SELECT Id FROM dbo.R) ORDER BY Id;")
        .unwrap();
    assert_eq!(qr.rows, vec![vec![Value::Int(1)], vec![Value::Int(3)]]);

    // Standard NOT IN (excludes NULL lhs)
    let qr = engine
        .query("SELECT Id FROM dbo.L WHERE Id NOT IN (SELECT Id FROM dbo.R) ORDER BY Id;")
        .unwrap();
    assert_eq!(qr.rows, vec![vec![Value::Int(2)]]);

    // NOT IN with NULL in subquery returns 0 rows
    let qr = engine
        .query("SELECT Id FROM dbo.L WHERE Id NOT IN (SELECT Id FROM dbo.RWithNull);")
        .unwrap();
    assert!(qr.rows.is_empty());

    // IN with NULL in subquery still matches non-null hits
    let qr = engine
        .query("SELECT Id FROM dbo.L WHERE Id IN (SELECT Id FROM dbo.RWithNull);")
        .unwrap();
    assert_eq!(qr.rows, vec![vec![Value::Int(1)]]);

    // NOT IN empty set returns ALL rows, including NULL lhs!
    let qr = engine
        .query("SELECT COUNT(*) FROM dbo.L WHERE Id NOT IN (SELECT Id FROM dbo.REmpty);")
        .unwrap();
    assert!(matches!(qr.rows[0][0], Value::Int(4) | Value::BigInt(4)));

    // 3-way comma join + IN subquery + ORDER BY on 2,000 rows (must stay O(N+M), sub-second)
    run_batch(
        &engine,
        "CREATE TABLE dbo.BigA (Id INT, Score INT);
         CREATE TABLE dbo.BigB (Id INT, Tag INT);
         CREATE TABLE dbo.BigC (Id INT, Extra INT);
         INSERT INTO dbo.BigA SELECT value, 2000 - value FROM GENERATE_SERIES(1, 2000);
         INSERT INTO dbo.BigB SELECT value, value % 10 FROM GENERATE_SERIES(1, 2000);
         INSERT INTO dbo.BigC SELECT value, value * 2 FROM GENERATE_SERIES(500, 1500);",
    )
    .unwrap();

    let qr = engine
        .query(
            "SELECT TOP (5) a.Id, c.Extra
             FROM dbo.BigA a, dbo.BigB b, dbo.BigC c
             WHERE a.Id = b.Id
               AND b.Id = c.Id
               AND a.Id IN (SELECT Id FROM dbo.BigC WHERE Id <= 600)
             ORDER BY a.Score DESC;",
        )
        .unwrap();
    assert_eq!(qr.rows.len(), 5);
    assert_eq!(qr.rows[0][0].to_integer_i64(), Some(500));
    assert_eq!(qr.rows[0][1].to_integer_i64(), Some(1000));
    assert_eq!(qr.rows[4][0].to_integer_i64(), Some(504));
    assert_eq!(qr.rows[4][1].to_integer_i64(), Some(1008));
}
