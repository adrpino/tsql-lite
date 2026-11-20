use iridium_core::{types::Value, Engine};

fn exec(engine: &mut Engine, sql: &str) {
    engine.exec(sql).expect(sql);
}

fn query(engine: &mut Engine, sql: &str) -> iridium_core::QueryResult {
    engine.query(sql).expect(sql)
}

#[test]
fn procedural_while_loop_sum() {
    let mut e = Engine::new();
    exec(&mut e, "DECLARE @sum INT = 0, @i INT = 1");
    exec(
        &mut e,
        "WHILE @i <= 10 BEGIN SET @sum = @sum + @i; SET @i = @i + 1 END",
    );
    let r = query(&mut e, "SELECT @sum");
    assert_eq!(r.rows[0][0], Value::Int(55));
}

#[test]
fn procedural_while_break() {
    let mut e = Engine::new();
    exec(&mut e, "DECLARE @sum INT = 0, @i INT = 1");
    exec(
        &mut e,
        "WHILE @i <= 100 BEGIN SET @sum = @sum + @i; IF @sum > 20 BREAK; SET @i = @i + 1 END",
    );
    let r = query(&mut e, "SELECT @sum");
    assert!(r.rows[0][0].to_integer_i64().unwrap() > 20);
}

#[test]
fn procedural_while_continue() {
    let mut e = Engine::new();
    exec(&mut e, "DECLARE @evens INT = 0, @i INT = 0");
    exec(
        &mut e,
        "WHILE @i < 10 BEGIN SET @i = @i + 1; IF @i % 2 <> 0 CONTINUE; SET @evens = @evens + 1 END",
    );
    let r = query(&mut e, "SELECT @evens");
    assert_eq!(r.rows[0][0], Value::Int(5));
}

#[test]
fn procedural_nested_begin_end() {
    let mut e = Engine::new();
    exec(&mut e, "DECLARE @x INT = 0");
    exec(
        &mut e,
        "BEGIN SET @x = 1; BEGIN SET @x = @x + 2; BEGIN SET @x = @x + 3 END END END",
    );
    let r = query(&mut e, "SELECT @x");
    assert_eq!(r.rows[0][0], Value::Int(6));
}

#[test]
fn procedural_raiserror_severity_handling() {
    let mut e = Engine::new();
    exec(&mut e, "RAISERROR('info msg', 10, 1)");
    assert_eq!(e.print_output()[0], "info msg");

    let err = e.exec("RAISERROR('error msg', 16, 1)");
    assert!(err.is_err(), "severity 16 should raise error");
}

#[test]
fn procedural_try_catch_with_error_message() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "BEGIN TRY RAISERROR('deliberate error', 16, 1) END TRY BEGIN CATCH PRINT ERROR_MESSAGE() END CATCH",
    );
    let output = e.print_output();
    assert!(!output.is_empty(), "CATCH should print error message");
}

#[test]
fn procedural_proc_output_param() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.swap @a INT OUTPUT, @b INT OUTPUT AS BEGIN DECLARE @tmp INT = @a; SET @a = @b; SET @b = @tmp END",
    );
    exec(&mut e, "DECLARE @x INT = 10, @y INT = 20");
    let _ = e.query("EXEC dbo.swap @a = @x OUTPUT, @b = @y OUTPUT");

    let r = query(&mut e, "SELECT @x, @y");
    assert_eq!(r.rows[0][0], Value::Int(20));
    assert_eq!(r.rows[0][1], Value::Int(10));
}

#[test]
fn procedural_if_else_branching() {
    let mut e = Engine::new();
    exec(&mut e, "DECLARE @result NVARCHAR(10)");
    exec(
        &mut e,
        "IF 1 > 2 SET @result = N'false' ELSE SET @result = N'true'",
    );
    let r = query(&mut e, "SELECT @result");
    assert_eq!(r.rows[0][0].to_string_value(), "true");
}

#[test]
fn procedural_declare_multiple_variables() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "DECLARE @a INT = 1, @b INT = 2, @c NVARCHAR(10) = N'hello'",
    );
    let r = query(&mut e, "SELECT @a + @b, @c");
    assert_eq!(r.rows[0][0], Value::BigInt(3));
    assert_eq!(r.rows[0][1].to_string_value(), "hello");
}

#[test]
fn procedural_select_into_variable() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TABLE dbo.tmp (val INT)");
    exec(&mut e, "INSERT INTO dbo.tmp VALUES (42)");

    exec(&mut e, "DECLARE @v INT");
    exec(&mut e, "SELECT @v = val FROM dbo.tmp");

    let r = query(&mut e, "SELECT @v");
    assert_eq!(r.rows[0][0], Value::Int(42));
}

#[test]
fn procedural_while_with_insert_accumulation() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TABLE dbo.accumulator (n INT)");
    exec(&mut e, "DECLARE @i INT = 1");
    exec(
        &mut e,
        "WHILE @i <= 5 BEGIN INSERT INTO dbo.accumulator VALUES (@i); SET @i = @i + 1 END",
    );

    let r = query(&mut e, "SELECT SUM(n) FROM dbo.accumulator");
    assert_eq!(r.rows[0][0], Value::BigInt(15));
}

#[test]
fn procedural_nested_while() {
    let mut e = Engine::new();
    exec(&mut e, "DECLARE @total INT = 0, @i INT = 1");
    exec(
        &mut e,
        "WHILE @i <= 3 BEGIN DECLARE @j INT = 1; WHILE @j <= 3 BEGIN SET @total = @total + 1; SET @j = @j + 1 END; SET @i = @i + 1 END",
    );
    let r = query(&mut e, "SELECT @total");
    assert_eq!(r.rows[0][0], Value::Int(9));
}

#[test]
fn procedural_proc_parenthesized_params_with_defaults_and_output() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.add_scaled (@a INT, @out INT OUTPUT, @b INT = 10) AS BEGIN SET @out = @a + @b END",
    );
    exec(&mut e, "DECLARE @res INT");
    e.execute_batch(
        iridium_core::parse_batch("EXEC dbo.add_scaled @a = 5, @out = @res OUTPUT").unwrap(),
    )
    .unwrap();
    let r = query(&mut e, "SELECT @res");
    assert_eq!(r.rows[0][0], Value::Int(15));

    exec(
        &mut e,
        "CREATE PROC dbo.double_val (@x INT) AS SELECT @x * 2 AS doubled",
    );
    let r2 = query(&mut e, "EXEC dbo.double_val @x = 7");
    assert_eq!(r2.rows[0][0], Value::BigInt(14));
}

#[test]
fn procedural_proc_parenthesized_tvp_readonly_param() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TYPE dbo.IntList AS TABLE (id INT)");
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.count_ids (@items dbo.IntList READONLY) AS BEGIN SELECT COUNT(*) AS cnt FROM @items END",
    );
    exec(&mut e, "DECLARE @ids TABLE (id INT)");
    exec(&mut e, "INSERT INTO @ids VALUES (1)");
    exec(&mut e, "INSERT INTO @ids VALUES (2)");
    exec(&mut e, "INSERT INTO @ids VALUES (3)");
    let r = query(&mut e, "EXEC dbo.count_ids @items = @ids");
    assert_eq!(r.rows[0][0], Value::BigInt(3));
}

#[test]
fn procedural_proc_parenthesized_params_rejects_empty_and_unclosed() {
    assert!(iridium_core::parse_sql("CREATE PROCEDURE dbo.empty_parens () AS SELECT 1").is_err());
    assert!(iridium_core::parse_sql("CREATE PROCEDURE dbo.unclosed (@a INT AS SELECT @a").is_err());
    assert!(
        iridium_core::parse_sql("CREATE PROCEDURE dbo.extra_rparen @a INT) AS SELECT @a").is_err()
    );
}

#[test]
fn procedural_exec_named_arguments_out_of_order_and_omitted_middle_default() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.compute (@a INT, @b INT = 10, @c INT OUTPUT) AS BEGIN SET @c = @a * 100 + @b END",
    );

    exec(&mut e, "DECLARE @res INT");
    e.execute_batch(
        iridium_core::parse_batch("EXEC dbo.compute @c = @res OUTPUT, @a = 3").unwrap(),
    )
    .unwrap();
    let r = query(&mut e, "SELECT @res");
    assert_eq!(r.rows[0][0], Value::Int(310));

    e.execute_batch(
        iridium_core::parse_batch("EXEC dbo.compute @b = 7, @c = @res OUTPUT, @a = 2").unwrap(),
    )
    .unwrap();
    let r2 = query(&mut e, "SELECT @res");
    assert_eq!(r2.rows[0][0], Value::Int(207));

    assert!(e
        .execute_batch(iridium_core::parse_batch("EXEC dbo.compute @unknown = 1").unwrap())
        .is_err());
}

#[test]
fn procedural_schema_qualified_sp_executesql_and_refresh_procs() {
    let mut e = Engine::new();
    let r = query(
        &mut e,
        "EXEC sys.sp_executesql N'SELECT @v * 3 AS tripled', N'@v INT', @v = 7",
    );
    assert_eq!(r.rows[0][0], Value::BigInt(21));

    exec(&mut e, "CREATE PROCEDURE dbo.p_ref AS SELECT 1");
    e.execute_batch(
        iridium_core::parse_batch(
            "EXEC sys.sp_refresh_parameter_encryption N'dbo.p_ref'; EXEC sys.sp_refreshsqlmodule N'dbo.p_ref';",
        )
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn procedural_variables_are_case_insensitive() {
    let mut e = Engine::new();
    exec(&mut e, "DECLARE @TotalAmount INT = 10");
    exec(&mut e, "SET @totalamount = @TOTALAMOUNT + 5");
    let r = query(&mut e, "SELECT @TotalAmount, @totalamount");
    assert_eq!(r.rows[0][0], Value::Int(15));
    assert_eq!(r.rows[0][1], Value::Int(15));

    exec(&mut e, "CREATE TABLE dbo.nums (n INT)");
    exec(&mut e, "INSERT INTO dbo.nums VALUES (99)");
    exec(&mut e, "SELECT @TOTALAMOUNT = n FROM dbo.nums");
    let r2 = query(&mut e, "SELECT @totalamount");
    assert_eq!(r2.rows[0][0], Value::Int(99));
}

#[test]
fn procedural_create_proc_with_options_multi_stmt_body_and_param_syntax() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_with_opts
            @a AS INT = 5,
            @out INT = NULL OUTPUT
        WITH RECOMPILE, EXECUTE AS 'dbo'
        AS
        SET NOCOUNT ON;
        SET @out = @a * 4;",
    );
    exec(&mut e, "DECLARE @res INT");
    e.execute_batch(
        iridium_core::parse_batch("EXEC dbo.p_with_opts @a = 6, @out = @res OUTPUT").unwrap(),
    )
    .unwrap();
    let r = query(&mut e, "SELECT @res");
    assert_eq!(r.rows[0][0], Value::Int(24));

    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_encrypted WITH ENCRYPTION AS SELECT 42 AS answer",
    );
    let r2 = query(&mut e, "EXEC dbo.p_encrypted");
    assert_eq!(r2.rows[0][0], Value::Int(42));
}

#[test]
fn procedural_insert_exec_sp_executesql_named_args_and_initial_rowcount() {
    let mut e = Engine::new();
    let rc0 = query(&mut e, "SELECT @@ROWCOUNT");
    assert_eq!(rc0.rows[0][0], Value::Int(0));

    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_add_two @a INT, @b INT AS SELECT @a + @b AS total",
    );
    exec(&mut e, "CREATE TABLE #res (total INT)");
    exec(
        &mut e,
        "INSERT INTO #res (total) EXEC dbo.p_add_two @b = 20, @a = 5",
    );
    exec(
        &mut e,
        "INSERT INTO #res (total) EXEC sys.sp_executesql N'SELECT @v * 2', N'@v INT', @v = 7",
    );
    let r = query(&mut e, "SELECT total FROM #res ORDER BY total");
    assert_eq!(r.rows.len(), 2);
    assert_eq!(r.rows[0][0].to_integer_i64(), Some(14));
    assert_eq!(r.rows[1][0].to_integer_i64(), Some(25));

    exec(&mut e, "DECLARE @rc INT = -1");
    let r_exec = query(&mut e, "EXEC @rc = sys.sp_executesql N'SELECT 1'");
    assert_eq!(r_exec.rows[0][0].to_integer_i64(), Some(1));
    let r_rc = query(&mut e, "SELECT @rc");
    assert_eq!(r_rc.rows[0][0], Value::Int(0));
}

#[test]
fn procedural_transactions_commit_rollback_savepoint_and_isolation_level() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TABLE dbo.ledger (id INT, note VARCHAR(50))");

    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_tx_commit
        AS
        BEGIN
            SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED;
            BEGIN TRANSACTION;
            INSERT INTO dbo.ledger VALUES (1, 'committed');
            COMMIT TRANSACTION;
        END",
    );
    e.execute_batch(iridium_core::parse_batch("EXEC dbo.p_tx_commit").unwrap())
        .unwrap();

    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_tx_rollback
        AS
        BEGIN
            DECLARE @log TABLE (step INT);
            BEGIN TRY
                BEGIN TRANSACTION;
                INSERT INTO @log VALUES (100);
                INSERT INTO dbo.ledger VALUES (2, 'rolled_back');
                RAISERROR('fail inside tx', 16, 1);
                COMMIT TRANSACTION;
            END TRY
            BEGIN CATCH
                IF @@TRANCOUNT > 0
                    ROLLBACK TRANSACTION;
            END CATCH
            SELECT COUNT(*) FROM @log;
        END",
    );
    let log_cnt = query(&mut e, "EXEC dbo.p_tx_rollback");
    assert_eq!(log_cnt.rows[0][0].to_integer_i64(), Some(1));

    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_tx_savepoint
        AS
        BEGIN
            BEGIN TRANSACTION;
            INSERT INTO dbo.ledger VALUES (3, 'kept');
            SAVE TRANSACTION sp1;
            INSERT INTO dbo.ledger VALUES (4, 'reverted');
            ROLLBACK TRANSACTION sp1;
            COMMIT TRANSACTION;
        END",
    );
    e.execute_batch(iridium_core::parse_batch("EXEC dbo.p_tx_savepoint").unwrap())
        .unwrap();

    let r = query(&mut e, "SELECT id, note FROM dbo.ledger ORDER BY id");
    assert_eq!(r.rows.len(), 2);
    assert_eq!(r.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r.rows[1][0].to_integer_i64(), Some(3));

    let tc = query(&mut e, "SELECT @@TRANCOUNT");
    assert_eq!(tc.rows[0][0], Value::Int(0));
}
