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

#[test]
fn procedural_merge_uses_actual_source_columns() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TABLE dbo.dest (id INT, status INT)");
    exec(&mut e, "INSERT INTO dbo.dest VALUES (1, 0)");
    exec(
        &mut e,
        "BEGIN
            CREATE TABLE #src (id INT, is_ready BIT);
            INSERT INTO #src VALUES (1, 1), (2, 0);
            MERGE dbo.dest AS target
            USING #src AS source
            ON target.id = source.id
            WHEN MATCHED THEN
                UPDATE SET target.status = CASE WHEN source.is_ready = 1 THEN 2 ELSE 1 END
            WHEN NOT MATCHED THEN
                INSERT (id, status) VALUES (source.id, CASE WHEN source.is_ready = 1 THEN 2 ELSE 1 END);
        END",
    );
    let r = query(&mut e, "SELECT id, status FROM dbo.dest ORDER BY id");
    assert_eq!(r.rows.len(), 2);
    assert_eq!(r.rows[0], vec![Value::Int(1), Value::Int(2)]);
    assert_eq!(r.rows[1], vec![Value::Int(2), Value::Int(1)]);
}

#[test]
fn procedural_execute_batch_all_results_captures_multiple_result_sets() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_multi_rs
        AS
        BEGIN
            SELECT 1 AS a, 'first' AS label;
            SELECT 10 AS x, 20 AS y, 30 AS z;
            RETURN 0;
        END",
    );
    let results = e
        .execute_batch_all_results(iridium_core::parse_batch("EXEC dbo.p_multi_rs").unwrap())
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].columns, vec!["a", "label"]);
    assert_eq!(
        results[0].rows[0],
        vec![Value::Int(1), Value::VarChar("first".to_string())]
    );
    assert_eq!(results[1].columns, vec!["x", "y", "z"]);
    assert_eq!(
        results[1].rows[0],
        vec![Value::Int(10), Value::Int(20), Value::Int(30)]
    );
}

#[test]
fn procedural_temp_unaliased_join_return_after_select_and_declare_hoisting() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TABLE dbo.items (id INT, name VARCHAR(20))");
    exec(&mut e, "INSERT INTO dbo.items VALUES (1, 'alpha'), (2, 'beta')");

    // 1. Unaliased #temp table referenced as #ids.id in JOIN ON
    exec(
        &mut e,
        "BEGIN
            SELECT id INTO #ids FROM dbo.items WHERE id = 1;
        END",
    );
    let r_join = query(
        &mut e,
        "SELECT i.name FROM dbo.items i JOIN #ids ON i.id = #ids.id",
    );
    assert_eq!(r_join.rows.len(), 1);
    assert_eq!(r_join.rows[0][0], Value::VarChar("alpha".to_string()));

    // 2. SELECT followed by early RETURN inside IF, captured via INSERT ... EXEC
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_early_return @mode INT
        AS
        BEGIN
            IF @mode = 1
            BEGIN
                SELECT 99 AS code, 'EARLY' AS status;
                RETURN;
            END
            IF @mode = 0
            BEGIN
                RETURN;
            END
            SELECT 1 AS code, 'NORMAL' AS status;
        END",
    );
    exec(&mut e, "CREATE TABLE #out (code INT, status VARCHAR(20))");
    exec(&mut e, "INSERT INTO #out EXEC dbo.p_early_return @mode = 1");
    exec(&mut e, "INSERT INTO #out EXEC dbo.p_early_return @mode = 0");
    let r_out = query(&mut e, "SELECT code, status FROM #out");
    assert_eq!(r_out.rows.len(), 1);
    assert_eq!(
        r_out.rows[0],
        vec![Value::Int(99), Value::VarChar("EARLY".to_string())]
    );

    // 3. DECLARE inside an untaken IF branch is still in scope across the procedure
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_hoisted @flag INT
        AS
        BEGIN
            IF @flag = 1
            BEGIN
                DECLARE @msg VARCHAR(50) = 'branch1';
            END
            ELSE
            BEGIN
                SET @msg = 'branch2';
            END
            SELECT @msg AS res;
        END",
    );
    let r_hoist = query(&mut e, "EXEC dbo.p_hoisted @flag = 2");
    assert_eq!(r_hoist.rows[0][0], Value::VarChar("branch2".to_string()));
}

#[test]
fn procedural_top_level_tvp_reusable_across_exec_and_tvf_tvp_param() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TYPE dbo.IntList AS TABLE (n INT)");
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_count_tvp @items dbo.IntList READONLY
        AS
        BEGIN
            SELECT COUNT(*) AS cnt FROM @items;
        END",
    );
    exec(
        &mut e,
        "CREATE FUNCTION dbo.fn_double_tvp (@items dbo.IntList READONLY)
        RETURNS TABLE
        AS
        RETURN (SELECT n * 2 AS doubled FROM @items)",
    );

    exec(&mut e, "DECLARE @vals dbo.IntList");
    exec(&mut e, "INSERT INTO @vals VALUES (3), (7)");

    // Call procedure twice with the same top-level TVP
    let r1 = query(&mut e, "EXEC dbo.p_count_tvp @items = @vals");
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(2));
    let r2 = query(&mut e, "EXEC dbo.p_count_tvp @items = @vals");
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(2));

    // Pass TVP to a table-valued function
    let r3 = query(
        &mut e,
        "SELECT doubled FROM dbo.fn_double_tvp(@vals) ORDER BY doubled",
    );
    assert_eq!(r3.rows.len(), 2);
    assert_eq!(r3.rows[0][0].to_integer_i64(), Some(6));
    assert_eq!(r3.rows[1][0].to_integer_i64(), Some(14));
}

#[test]
fn lexer_signed_arithmetic_unary_plus_and_zero_width_whitespace() {
    let mut e = Engine::new();

    // Tight binary minus/plus before integer literals (must not lex as signed literal)
    let r1 = query(
        &mut e,
        "SELECT CHARINDEX('.', 'ab.cd')+1 AS idx, (10)-3 AS sub, -1 + 2 AS prec1, -1 * 3 AS prec2, -(1 + 2) AS prec3",
    );
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(4));
    assert_eq!(r1.rows[0][1].to_integer_i64(), Some(7));
    assert_eq!(r1.rows[0][2].to_integer_i64(), Some(1));
    assert_eq!(r1.rows[0][3].to_integer_i64(), Some(-3));
    assert_eq!(r1.rows[0][4].to_integer_i64(), Some(-3));

    // Tight subtraction on function return value (GETDATE()-35)
    let r2 = query(
        &mut e,
        "SELECT DATEDIFF(day, GETDATE()-35, GETDATE()) AS days_diff",
    );
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(35));

    // Unary plus on numbers and string concatenation ('a' + + 'b')
    let r3 = query(&mut e, "SELECT +5 AS pos, 'a' + + 'b' AS cat");
    assert_eq!(r3.rows[0][0].to_integer_i64(), Some(5));
    assert_eq!(r3.rows[0][1], Value::VarChar("ab".to_string()));

    // Zero-width space / BOM treated as whitespace
    let r4 = query(&mut e, "\u{FEFF}\u{200B}SELECT 42 AS v");
    assert_eq!(r4.rows[0][0].to_integer_i64(), Some(42));
}

#[test]
fn procedural_raiserror_options_if_semicolon_else_and_cursor_options() {
    let mut e = Engine::new();

    // 1. RAISERROR with format args and WITH NOWAIT, plus IF <stmt>; ELSE <stmt>
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_raiserror_and_if @flag INT
        AS
        BEGIN
            DECLARE @cnt INT = 7;
            RAISERROR ('Processed %d items for %s.', 0, 1, @cnt, 'alpha') WITH NOWAIT;
            IF @flag = 1
                SELECT 'one' AS res;
            ELSE IF @flag = 2
                RAISERROR ('Bad flag %d', 16, 1, @flag) WITH NOWAIT, SETERROR;
            ELSE
                SELECT 'other' AS res;
        END",
    );
    let r1 = query(&mut e, "EXEC dbo.p_raiserror_and_if @flag = 1");
    assert_eq!(r1.rows[0][0], Value::VarChar("one".to_string()));
    let r3 = query(&mut e, "EXEC dbo.p_raiserror_and_if @flag = 3");
    assert_eq!(r3.rows[0][0], Value::VarChar("other".to_string()));
    let err = e
        .execute_batch(iridium_core::parse_batch("EXEC dbo.p_raiserror_and_if @flag = 2").unwrap())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("Bad flag 2"),
        "expected formatted RAISERROR message, got: {err}"
    );

    // 2. DECLARE CURSOR with LOCAL FAST_FORWARD READ_ONLY options
    exec(&mut e, "CREATE TABLE dbo.nums (n INT)");
    exec(&mut e, "INSERT INTO dbo.nums VALUES (10), (20)");
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_cursor_opts
        AS
        BEGIN
            DECLARE @n INT, @sum INT = 0;
            DECLARE cur CURSOR LOCAL FAST_FORWARD READ_ONLY FOR
                SELECT n FROM dbo.nums ORDER BY n;
            OPEN cur;
            FETCH NEXT FROM cur INTO @n;
            WHILE @@FETCH_STATUS = 0
            BEGIN
                SET @sum = @sum + @n;
                FETCH NEXT FROM cur INTO @n;
            END
            CLOSE cur;
            DEALLOCATE cur;
            SELECT @sum AS total;
        END",
    );
    let rc = query(&mut e, "EXEC dbo.p_cursor_opts");
    assert_eq!(rc.rows[0][0].to_integer_i64(), Some(30));
}

#[test]
fn ddl_dml_inline_index_trailing_comma_table_hints_option_and_nocheck() {
    let mut e = Engine::new();

    // 1. CREATE TABLE with trailing comma, inline INDEX, and PK without preceding comma
    exec(
        &mut e,
        "CREATE TABLE dbo.t_inline (
            id INT NOT NULL,
            code VARCHAR(20) NOT NULL,
            INDEX IX_code UNIQUE NONCLUSTERED (code),
        )",
    );
    exec(
        &mut e,
        "CREATE TABLE dbo.t_nocomma_pk (
            id INT NOT NULL
            PRIMARY KEY (id)
        )",
    );

    // 2. ALTER TABLE ... NOCHECK / CHECK CONSTRAINT
    exec(&mut e, "ALTER TABLE dbo.t_inline NOCHECK CONSTRAINT ALL");
    exec(&mut e, "ALTER TABLE dbo.t_inline WITH CHECK CHECK CONSTRAINT ALL");

    // 3. INSERT ... WITH (TABLOCK) and trailing OPTION (RECOMPILE)
    exec(
        &mut e,
        "INSERT INTO dbo.t_inline WITH (TABLOCK) (id, code)
         SELECT 1, 'A'
         OPTION (RECOMPILE)",
    );
    exec(
        &mut e,
        "INSERT dbo.t_nocomma_pk WITH (TABLOCK) (id)
         SELECT id FROM dbo.t_inline
         OPTION (MAXRECURSION 0)",
    );
    let r = query(
        &mut e,
        "SELECT id, code FROM dbo.t_inline ORDER BY id OPTION (RECOMPILE)",
    );
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r.rows[0][1], Value::VarChar("A".to_string()));
}

#[test]
fn merge_not_matched_by_target_and_general_output_expressions() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE TABLE dbo.dst (id INT PRIMARY KEY, val VARCHAR(20))",
    );
    exec(&mut e, "INSERT INTO dbo.dst VALUES (1, 'old')");
    exec(
        &mut e,
        "CREATE TABLE dbo.src (id INT, val VARCHAR(20), tag VARCHAR(20))",
    );
    exec(
        &mut e,
        "INSERT INTO dbo.src VALUES (1, 'updated', 't1'), (2, 'added', 't2')",
    );
    exec(
        &mut e,
        "CREATE TABLE dbo.audit_log (
            seq INT IDENTITY(1,1) PRIMARY KEY,
            act VARCHAR(10),
            item_id INT,
            src_tag VARCHAR(20),
            label VARCHAR(20),
            ts DATETIME
        )",
    );

    exec(
        &mut e,
        "MERGE dbo.dst WITH (HOLDLOCK) AS TARGET
        USING dbo.src AS SOURCE
            ON TARGET.id = SOURCE.id
        WHEN MATCHED THEN
            UPDATE SET val = SOURCE.val
        WHEN NOT MATCHED BY TARGET THEN
            INSERT (id, val) VALUES (SOURCE.id, SOURCE.val)
        OUTPUT $action, inserted.id, SOURCE.tag, 'MERGE', GETUTCDATE()
        INTO dbo.audit_log (act, item_id, src_tag, label, ts);",
    );

    let r = query(
        &mut e,
        "SELECT act, item_id, src_tag, label FROM dbo.audit_log ORDER BY item_id",
    );
    assert_eq!(r.rows.len(), 2);
    assert_eq!(r.rows[0][0], Value::VarChar("UPDATE".to_string()));
    assert_eq!(r.rows[0][1].to_integer_i64(), Some(1));
    assert_eq!(r.rows[0][2], Value::VarChar("t1".to_string()));
    assert_eq!(r.rows[0][3], Value::VarChar("MERGE".to_string()));
    assert_eq!(r.rows[1][0], Value::VarChar("INSERT".to_string()));
    assert_eq!(r.rows[1][1].to_integer_i64(), Some(2));
    assert_eq!(r.rows[1][2], Value::VarChar("t2".to_string()));

    // DELETE ... OUTPUT DELETED.id, GETUTCDATE(), 'DEL'
    let rd = query(
        &mut e,
        "DELETE FROM dbo.dst OUTPUT DELETED.id, 'DEL' AS kind WHERE id = 2",
    );
    assert_eq!(rd.rows.len(), 1);
    assert_eq!(rd.rows[0][0].to_integer_i64(), Some(2));
    assert_eq!(rd.rows[0][1], Value::VarChar("DEL".to_string()));
}

#[test]
fn query_join_after_apply_inline_tvf_cte_and_keyword_aliases() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TABLE dbo.orders (id INT, cust_id INT)");
    exec(&mut e, "CREATE TABLE dbo.order_lines (order_id INT, prod_id INT, seq INT)");
    exec(&mut e, "CREATE TABLE dbo.products (id INT, name VARCHAR(30))");
    exec(&mut e, "INSERT INTO dbo.orders VALUES (1, 10), (2, 20)");
    exec(
        &mut e,
        "INSERT INTO dbo.order_lines VALUES (1, 100, 1), (1, 200, 2)",
    );
    exec(
        &mut e,
        "INSERT INTO dbo.products VALUES (100, 'Alpha'), (200, 'Beta')",
    );

    // 1. LEFT JOIN after OUTER APPLY referencing the APPLY alias, plus AS date / AS row / [Group] aliases
    let r1 = query(
        &mut e,
        "SELECT o.id AS row,
                p.name [Group],
                '2026-01-01' AS date
         FROM dbo.orders o
         OUTER APPLY (
             SELECT TOP (1) l.prod_id
             FROM dbo.order_lines l
             WHERE l.order_id = o.id
             ORDER BY l.seq DESC
         ) AS latest
         LEFT JOIN dbo.products p
             ON p.id = latest.prod_id
         ORDER BY o.id",
    );
    assert_eq!(r1.rows.len(), 2);
    let cols_lower: Vec<String> = r1.columns.iter().map(|c| c.to_ascii_lowercase()).collect();
    assert_eq!(cols_lower, vec!["row", "group", "date"]);
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r1.rows[0][1], Value::VarChar("Beta".to_string()));
    assert_eq!(r1.rows[1][0].to_integer_i64(), Some(2));
    assert_eq!(r1.rows[1][1], Value::Null);

    // 2. Inline TVF with CTE: RETURNS TABLE AS RETURN (WITH cte AS (...) SELECT ...)
    exec(
        &mut e,
        "CREATE FUNCTION dbo.fn_cte_lines (@min_seq INT)
        RETURNS TABLE
        AS
        RETURN (
            WITH filtered AS (
                SELECT order_id, prod_id
                FROM dbo.order_lines
                WHERE seq >= @min_seq
            )
            SELECT f.order_id, p.name
            FROM filtered f
            JOIN dbo.products p ON p.id = f.prod_id
        )",
    );
    let r2 = query(
        &mut e,
        "SELECT order_id, name FROM dbo.fn_cte_lines(2)",
    );
    assert_eq!(r2.rows.len(), 1);
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r2.rows[0][1], Value::VarChar("Beta".to_string()));
}

#[test]
fn query_for_xml_path_stuff_concatenation_and_type_value() {
    let mut e = Engine::new();
    exec(&mut e, "CREATE TABLE dbo.groups (id INT, title VARCHAR(20))");
    exec(&mut e, "CREATE TABLE dbo.tags (group_id INT, tag VARCHAR(20))");
    exec(&mut e, "INSERT INTO dbo.groups VALUES (1, 'G1'), (2, 'G2')");
    exec(
        &mut e,
        "INSERT INTO dbo.tags VALUES (1, 'b'), (1, 'a'), (1, 'c')",
    );

    // 1. Classic STUFF + FOR XML PATH('') string aggregation
    let r1 = query(
        &mut e,
        "SELECT g.id,
                STUFF((
                    SELECT ',' + t.tag
                    FROM dbo.tags t
                    WHERE t.group_id = g.id
                    ORDER BY t.tag
                    FOR XML PATH('')
                ), 1, 1, '') AS csv
         FROM dbo.groups g
         ORDER BY g.id",
    );
    assert_eq!(r1.rows.len(), 2);
    assert_eq!(r1.rows[0][1].to_string_value(), "a,b,c");
    assert_eq!(r1.rows[1][1], Value::Null);

    // 2. FOR XML PATH(''), TYPE).value('.', 'NVARCHAR(MAX)')
    let r2 = query(
        &mut e,
        "SELECT STUFF((
             SELECT ';' + t.tag
             FROM dbo.tags t
             WHERE t.group_id = 1
             ORDER BY t.tag
             FOR XML PATH(''), TYPE
         ).value('.', 'NVARCHAR(MAX)'), 1, 1, '') AS semi",
    );
    assert_eq!(r2.rows[0][0].to_string_value(), "a;b;c");

    // 3. Named row element + ROOT
    let r3 = query(
        &mut e,
        "SELECT t.tag FROM dbo.tags t ORDER BY t.tag FOR XML PATH('item'), ROOT('items')",
    );
    assert_eq!(
        r3.rows[0][0].to_string_value(),
        "<items><item><tag>a</tag></item><item><tag>b</tag></item><item><tag>c</tag></item></items>"
    );
}

#[test]
fn datetime_empty_epoch_dateadd_clamping_week_part_and_error_procedure() {
    let mut e = Engine::new();

    // 1. Empty string as DATETIME -> 1900-01-01, while ISDATE('') stays 0
    let r1 = query(
        &mut e,
        "SELECT CAST('' AS DATETIME) AS dt, DATEADD(day, 1, '') AS next_day, ISDATE('') AS is_d",
    );
    assert_eq!(r1.rows[0][0].to_string_value(), "1900-01-01 00:00:00");
    assert_eq!(r1.rows[0][1].to_string_value(), "1900-01-02 00:00:00");
    assert_eq!(r1.rows[0][2].to_integer_i64(), Some(0));

    // 2. DATEADD month/year end-of-month clamping + week/quarter dateparts
    let r2 = query(
        &mut e,
        "SELECT CAST(DATEADD(MONTH, 2, '2025-12-31') AS DATE) AS feb_end,
                CAST(DATEADD(YEAR, 1, '2024-02-29') AS DATE) AS leap_clamp,
                CAST(DATEADD(WEEK, 2, '2026-01-01') AS DATE) AS plus_2w,
                DATEDIFF(WEEK, '2026-01-01', '2026-01-15') AS diff_w,
                DATEPART(WEEK, '2026-01-08') AS part_w",
    );
    assert_eq!(r2.rows[0][0].to_string_value(), "2026-02-28");
    assert_eq!(r2.rows[0][1].to_string_value(), "2025-02-28");
    assert_eq!(r2.rows[0][2].to_string_value(), "2026-01-15");
    assert_eq!(r2.rows[0][3].to_integer_i64(), Some(2));
    assert_eq!(r2.rows[0][4].to_integer_i64(), Some(2));

    // 3. ERROR_PROCEDURE() and ERROR_LINE() inside CATCH
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_err_diag
        AS
        BEGIN
            BEGIN TRY
                RAISERROR('boom', 16, 1);
            END TRY
            BEGIN CATCH
                SELECT ERROR_PROCEDURE() AS proc_name, ERROR_LINE() AS err_line;
            END CATCH
        END",
    );
    let r3 = query(&mut e, "EXEC dbo.p_err_diag");
    assert_eq!(r3.rows[0][0].to_string_value(), "p_err_diag");
    assert!(r3.rows[0][1].to_integer_i64().is_some());
}

#[test]
fn tsql_recursive_cte_without_keyword_and_merge_subquery_source() {
    let mut e = Engine::new();

    // 1. Standard T-SQL recursive CTE (no RECURSIVE keyword) feeding INSERT INTO #temp
    exec(&mut e, "CREATE TABLE #dates (d DATE)");
    exec(
        &mut e,
        "WITH DateRange AS (
            SELECT CAST('2026-01-01' AS DATE) AS DateValue
            UNION ALL
            SELECT DATEADD(DAY, 1, DateValue)
            FROM DateRange
            WHERE DateValue < CAST('2026-01-05' AS DATE)
        )
        INSERT INTO #dates
        SELECT DateValue FROM DateRange;",
    );
    let r1 = query(&mut e, "SELECT COUNT(*) AS cnt, MAX(d) AS last_d FROM #dates");
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(5));
    assert_eq!(r1.rows[0][1].to_string_value(), "2026-01-05");

    // 2. MERGE with derived subquery source and VALUES source
    exec(&mut e, "CREATE TABLE dbo.m_dst (id INT PRIMARY KEY, v VARCHAR(20))");
    exec(&mut e, "INSERT INTO dbo.m_dst VALUES (1, 'old')");
    exec(
        &mut e,
        "MERGE dbo.m_dst AS T
         USING (SELECT 1 AS id, 'new1' AS v UNION ALL SELECT 2 AS id, 'new2' AS v) AS S
             ON T.id = S.id
         WHEN MATCHED THEN UPDATE SET v = S.v
         WHEN NOT MATCHED THEN INSERT (id, v) VALUES (S.id, S.v);",
    );
    exec(
        &mut e,
        "MERGE dbo.m_dst AS T
         USING (VALUES (3, 'from_vals')) AS S (id, v)
             ON T.id = S.id
         WHEN NOT MATCHED THEN INSERT (id, v) VALUES (S.id, S.v);",
    );
    let r2 = query(&mut e, "SELECT id, v FROM dbo.m_dst ORDER BY id");
    assert_eq!(r2.rows.len(), 3);
    assert_eq!(r2.rows[0][1].to_string_value(), "new1");
    assert_eq!(r2.rows[1][1].to_string_value(), "new2");
    assert_eq!(r2.rows[2][1].to_string_value(), "from_vals");
}
