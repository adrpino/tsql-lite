use tsql_lite_core::{types::Value, Engine};

fn exec(engine: &mut Engine, sql: &str) {
    engine.exec(sql).expect(sql);
}

fn query(engine: &mut Engine, sql: &str) -> tsql_lite_core::QueryResult {
    engine.query(sql).expect(sql)
}

#[test]
fn tvp_procedure_reads_rows() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TYPE dbo.IntList AS TABLE (id INT)");
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.sum_ids @items dbo.IntList READONLY AS BEGIN SELECT SUM(id) AS s FROM @items END",
    );
    exec(&mut e, "DECLARE @src TABLE (id INT)");
    exec(&mut e, "INSERT INTO @src VALUES (1)");
    exec(&mut e, "INSERT INTO @src VALUES (2)");
    exec(&mut e, "INSERT INTO @src VALUES (3)");

    let r = query(&mut e, "EXEC dbo.sum_ids @items = @src");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], Value::BigInt(6));
}

#[test]
fn declare_table_variable_from_user_defined_table_type() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TYPE dbo.IntList AS TABLE (id INT)");
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.sum_ids (@items dbo.IntList READONLY) AS BEGIN SELECT SUM(id) AS s FROM @items END",
    );

    exec(&mut e, "DECLARE @ids dbo.IntList");
    exec(&mut e, "INSERT INTO @ids VALUES (10), (20), (30)");
    let r = query(&mut e, "EXEC dbo.sum_ids @items = @ids");
    assert_eq!(r.rows[0][0], Value::BigInt(60));

    exec(&mut e, "DECLARE @ids_as AS dbo.IntList");
    exec(&mut e, "INSERT INTO @ids_as VALUES (5)");
    let r2 = query(&mut e, "SELECT COUNT(*) FROM @ids_as");
    assert_eq!(r2.rows[0][0], Value::BigInt(1));
}

#[test]
fn tvp_is_readonly_in_procedure() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TYPE dbo.IntList AS TABLE (id INT)");
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.bad_tvp @items dbo.IntList READONLY AS BEGIN INSERT INTO @items VALUES (99) END",
    );
    exec(&mut e, "DECLARE @src TABLE (id INT)");
    let err = e
        .exec("EXEC dbo.bad_tvp @items = @src")
        .expect_err("expected READONLY failure");
    assert!(err.to_string().to_uppercase().contains("READONLY"));
}

#[test]
fn sp_executesql_tvp_binds_table_param() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TYPE dbo.IntList AS TABLE (id INT)");
    exec(&mut e, "DECLARE @src TABLE (id INT)");
    exec(&mut e, "INSERT INTO @src VALUES (10)");
    exec(&mut e, "INSERT INTO @src VALUES (20)");

    let r = query(
        &mut e,
        "EXEC sp_executesql N'SELECT COUNT(*) AS c FROM @items', N'@items dbo.IntList READONLY', @items = @src",
    );
    assert_eq!(r.rows[0][0], Value::BigInt(2));
}

#[test]
fn xact_state_transitions() {
    let mut e = Engine::new();
    let r0 = query(&mut e, "SELECT XACT_STATE()");
    assert_eq!(r0.rows[0][0], Value::Int(0));

    exec(&mut e, "BEGIN TRANSACTION");
    let r1 = query(&mut e, "SELECT XACT_STATE()");
    assert_eq!(r1.rows[0][0], Value::Int(1));

    let err = e.exec("INSERT INTO no_such_table VALUES (1)");
    assert!(err.is_err());
    let r2 = query(&mut e, "SELECT XACT_STATE()");
    assert_eq!(r2.rows[0][0], Value::Int(-1));

    let commit_err = e.exec("COMMIT");
    assert!(commit_err.is_err());

    exec(&mut e, "ROLLBACK");
    let r3 = query(&mut e, "SELECT XACT_STATE()");
    assert_eq!(r3.rows[0][0], Value::Int(0));
}

#[test]
fn xact_abort_forces_state_zero() {
    let mut e = Engine::new();
    exec(&mut e, "SET XACT_ABORT ON");
    exec(&mut e, "BEGIN TRANSACTION");
    let err = e.exec("INSERT INTO no_such_table VALUES (1)");
    assert!(err.is_err());
    let r = query(&mut e, "SELECT XACT_STATE()");
    assert_eq!(r.rows[0][0], Value::Int(0));
}
