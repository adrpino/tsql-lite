use tsql_lite_core::Engine;

#[test]
fn drop_table_if_exists_succeeds_for_missing_and_existing_tables() {
    let e = Engine::new();
    e.exec("DROP TABLE IF EXISTS dbo.missing_tbl").unwrap();
    e.exec("CREATE TABLE dbo.temp_tbl (id INT)").unwrap();
    e.exec("DROP TABLE IF EXISTS dbo.temp_tbl").unwrap();
    assert!(e.exec("DROP TABLE dbo.temp_tbl").is_err());
    e.exec("DROP TABLE IF EXISTS dbo.temp_tbl").unwrap();
}

#[test]
fn drop_temp_table_if_exists_succeeds() {
    let e = Engine::new();
    e.exec("DROP TABLE IF EXISTS #scratch").unwrap();
    e.exec("CREATE TABLE #scratch (id INT)").unwrap();
    e.exec("DROP TABLE IF EXISTS #scratch").unwrap();
    e.exec("DROP TABLE IF EXISTS #scratch").unwrap();
}

#[test]
fn drop_view_procedure_function_trigger_type_schema_if_exists() {
    let e = Engine::new();

    e.exec("DROP VIEW IF EXISTS dbo.missing_view").unwrap();
    e.exec("CREATE VIEW dbo.sample_view AS SELECT 1 AS v")
        .unwrap();
    e.exec("DROP VIEW IF EXISTS dbo.sample_view").unwrap();
    e.exec("DROP VIEW IF EXISTS dbo.sample_view").unwrap();

    e.exec("DROP PROCEDURE IF EXISTS dbo.missing_proc").unwrap();
    e.exec("DROP PROC IF EXISTS dbo.missing_proc").unwrap();
    e.exec("CREATE PROCEDURE dbo.sample_proc AS SELECT 1")
        .unwrap();
    e.exec("DROP PROCEDURE IF EXISTS dbo.sample_proc").unwrap();
    e.exec("DROP PROCEDURE IF EXISTS dbo.sample_proc").unwrap();

    e.exec("DROP FUNCTION IF EXISTS dbo.missing_fn").unwrap();
    e.exec("CREATE FUNCTION dbo.sample_fn() RETURNS INT AS BEGIN RETURN 1 END")
        .unwrap();
    e.exec("DROP FUNCTION IF EXISTS dbo.sample_fn").unwrap();
    e.exec("DROP FUNCTION IF EXISTS dbo.sample_fn").unwrap();

    e.exec("DROP TYPE IF EXISTS dbo.missing_type").unwrap();
    e.exec("CREATE TYPE dbo.sample_type AS TABLE (id INT)")
        .unwrap();
    e.exec("DROP TYPE IF EXISTS dbo.sample_type").unwrap();
    e.exec("DROP TYPE IF EXISTS dbo.sample_type").unwrap();

    e.exec("DROP TRIGGER IF EXISTS dbo.missing_trg").unwrap();
    e.exec("DROP SCHEMA IF EXISTS missing_schema").unwrap();
    e.exec("CREATE SCHEMA temp_schema").unwrap();
    e.exec("DROP SCHEMA IF EXISTS temp_schema").unwrap();
    e.exec("DROP SCHEMA IF EXISTS temp_schema").unwrap();
}
