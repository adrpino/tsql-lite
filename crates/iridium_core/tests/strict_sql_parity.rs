use iridium_core::error::DbError;
use iridium_core::types::Value;
use iridium_core::{parse_batch, Engine, QueryResult};

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

    // Invalid XML forms must fail with error 9400.
    for invalid in [
        "<a>",
        "<a></b>",
        "</a>",
        "<a x=1/>",
        "<a x=\"1\" x=\"2\"/>",
        "<a x=\"1<2\"/>",
        "a & b",
        "<a/>&unknown;",
        "<!-- unclosed",
        "<!-- bad -- comment -->",
        "<![CDATA[unclosed",
    ] {
        let sql = format!("DECLARE @x XML = '{}';", invalid.replace('\'', "''"));
        let err = engine
            .exec(&sql)
            .expect_err(&format!("invalid XML `{invalid}` should have failed"));
        assert_eq!(
            err.number(),
            9400,
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
