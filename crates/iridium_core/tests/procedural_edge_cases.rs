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

#[test]
fn engine_checkpoint_deep_ast_and_fork_isolated() {
    let e = Engine::new();
    e.exec("CREATE TABLE dbo.items (id INT, val INT)").unwrap();
    e.exec("INSERT INTO dbo.items VALUES (1, 10)").unwrap();

    // Build a deeply nested binary expression (>128 JSON nesting levels) to exceed serde_json's default limit.
    let mut expr = String::from("1");
    for _ in 0..45 {
        expr.push_str(" + 1");
    }
    let proc_sql = format!("CREATE PROCEDURE dbo.p_deep AS SELECT {expr} AS total");
    e.exec(&proc_sql).unwrap();
    e.exec("CREATE PROCEDURE dbo.p_ok AS SELECT 42 AS answer")
        .unwrap();

    let payload = e.export_checkpoint().unwrap();
    let restored = Engine::from_checkpoint(&payload).unwrap();
    let r = restored.query("EXEC dbo.p_ok").unwrap();
    assert_eq!(r.rows[0][0].to_integer_i64(), Some(42));

    let fork_a = e.fork_isolated();
    let fork_b = e.fork_isolated();
    fork_a.exec("INSERT INTO dbo.items VALUES (2, 20)").unwrap();
    let ra = fork_a.query("SELECT COUNT(*) FROM dbo.items").unwrap();
    let rb = fork_b.query("SELECT COUNT(*) FROM dbo.items").unwrap();
    let rp = e.query("SELECT COUNT(*) FROM dbo.items").unwrap();
    assert_eq!(ra.rows[0][0].to_integer_i64(), Some(2));
    assert_eq!(rb.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(rp.rows[0][0].to_integer_i64(), Some(1));
}

#[test]
fn procedural_select_assign_accumulates_row_by_row() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TABLE dbo.ids (id INT)");
    exec(&mut e, "INSERT INTO dbo.ids VALUES (101), (102), (103)");
    exec(&mut e, "DECLARE @csv VARCHAR(200) = ''");
    exec(
        &mut e,
        "SELECT @csv = @csv + CAST(id AS VARCHAR(20)) + ',' FROM dbo.ids",
    );
    let r = query(&mut e, "SELECT @csv");
    assert_eq!(r.rows[0][0], Value::VarChar("101,102,103,".to_string()));

    // Aggregate assignment still works cleanly
    exec(&mut e, "DECLARE @max_id INT = 0");
    exec(&mut e, "SELECT @max_id = MAX(id) FROM dbo.ids");
    let r2 = query(&mut e, "SELECT @max_id");
    assert_eq!(r2.rows[0][0], Value::Int(103));
}

#[test]
fn procedural_nested_exec_preserves_caller_variables_and_tvp() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TYPE dbo.IntList AS TABLE (n INT)");
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_inner @id INT, @items dbo.IntList READONLY, @sum INT OUTPUT
        AS
        BEGIN
            SELECT @sum = SUM(n) FROM @items;
            SET @sum = @id + ISNULL(@sum, 0);
        END",
    );
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_outer @id INT, @items dbo.IntList READONLY
        AS
        BEGIN
            DECLARE @inner_sum INT = 0;
            EXEC dbo.p_inner @id = @id, @items = @items, @sum = @inner_sum OUTPUT;
            DECLARE @item_cnt INT = 0;
            SELECT @item_cnt = COUNT(*) FROM @items;
            SELECT @id AS outer_id, @inner_sum AS inner_sum, @item_cnt AS item_cnt;
        END",
    );
    exec(
        &mut e,
        "BEGIN
            DECLARE @vals dbo.IntList;
            INSERT INTO @vals VALUES (10), (20);
            CREATE TABLE #out (outer_id INT, inner_sum INT, item_cnt INT);
            INSERT INTO #out EXEC dbo.p_outer @id = 5, @items = @vals;
        END",
    );
    let r = query(&mut e, "SELECT outer_id, inner_sum, item_cnt FROM #out");
    assert_eq!(r.rows[0][0], Value::Int(5));
    assert_eq!(r.rows[0][1], Value::Int(35));
    assert_eq!(r.rows[0][2], Value::Int(2));
}

#[test]
fn procedural_create_and_drop_index_on_temp_table() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TABLE dbo.src (id INT, label VARCHAR(20))");
    exec(&mut e, "INSERT INTO dbo.src VALUES (1, 'a'), (2, 'b')");
    exec(
        &mut e,
        "BEGIN
            SELECT id, label INTO #stage FROM dbo.src;
            CREATE CLUSTERED INDEX CIX_stage ON #stage (id);
            DROP INDEX CIX_stage ON #stage;
        END",
    );
    let r = query(&mut e, "SELECT COUNT(*) FROM #stage");
    assert_eq!(r.rows[0][0].to_integer_i64(), Some(2));
}

#[test]
fn procedural_multi_statement_tvf_with_identity_and_schemabinding() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE FUNCTION dbo.split_ints (@list NVARCHAR(4000))
        RETURNS @tbl TABLE (pos INT IDENTITY(1, 1) NOT NULL, num INT NOT NULL)
        WITH SCHEMABINDING
        AS
        BEGIN
            DECLARE @pos INT;
            DECLARE @rest NVARCHAR(4000) = LTRIM(RTRIM(@list));
            SET @pos = CHARINDEX(',', @rest);
            WHILE @pos > 0
            BEGIN
                INSERT @tbl (num) VALUES (CONVERT(INT, SUBSTRING(@rest, 1, @pos - 1)));
                SET @rest = LTRIM(SUBSTRING(@rest, @pos + 1, LEN(@rest)));
                SET @pos = CHARINDEX(',', @rest);
            END
            IF @rest <> ''
                INSERT @tbl (num) VALUES (CONVERT(INT, @rest));
            RETURN;
        END",
    );
    let r = query(
        &mut e,
        "SELECT pos, num FROM dbo.split_ints('10, 20, 30') ORDER BY pos",
    );
    assert_eq!(r.rows.len(), 3);
    assert_eq!(r.rows[0], vec![Value::Int(1), Value::Int(10)]);
    assert_eq!(r.rows[1], vec![Value::Int(2), Value::Int(20)]);
    assert_eq!(r.rows[2], vec![Value::Int(3), Value::Int(30)]);
}

#[test]
fn procedural_bare_return_before_end_and_deep_expression() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE FUNCTION dbo.fn_bare_return (@n INT)
        RETURNS @out TABLE (val INT)
        AS
        BEGIN
            INSERT @out VALUES (@n * 2)
            RETURN
        END",
    );
    let r = query(&mut e, "SELECT val FROM dbo.fn_bare_return(21)");
    assert_eq!(r.rows[0][0].to_integer_i64(), Some(42));

    let mut expr = String::from("1");
    for _ in 0..45 {
        expr.push_str(" + 1");
    }
    let r2 = query(&mut e, &format!("SELECT {expr}"));
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(46));
}
