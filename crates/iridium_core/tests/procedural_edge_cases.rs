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
    exec(
        &mut e,
        "INSERT INTO dbo.items VALUES (1, 'alpha'), (2, 'beta')",
    );

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
    exec(
        &mut e,
        "ALTER TABLE dbo.t_inline WITH CHECK CHECK CONSTRAINT ALL",
    );

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
    exec(
        &mut e,
        "CREATE TABLE dbo.order_lines (order_id INT, prod_id INT, seq INT)",
    );
    exec(
        &mut e,
        "CREATE TABLE dbo.products (id INT, name VARCHAR(30))",
    );
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
    let r2 = query(&mut e, "SELECT order_id, name FROM dbo.fn_cte_lines(2)");
    assert_eq!(r2.rows.len(), 1);
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r2.rows[0][1], Value::VarChar("Beta".to_string()));
}

#[test]
fn query_for_xml_path_stuff_concatenation_and_type_value() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE TABLE dbo.groups (id INT, title VARCHAR(20))",
    );
    exec(
        &mut e,
        "CREATE TABLE dbo.tags (group_id INT, tag VARCHAR(20))",
    );
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
    let r1 = query(
        &mut e,
        "SELECT COUNT(*) AS cnt, MAX(d) AS last_d FROM #dates",
    );
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(5));
    assert_eq!(r1.rows[0][1].to_string_value(), "2026-01-05");

    // 2. MERGE with derived subquery source and VALUES source
    exec(
        &mut e,
        "CREATE TABLE dbo.m_dst (id INT PRIMARY KEY, v VARCHAR(20))",
    );
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

#[test]
fn nested_aggregates_in_select_and_empty_wildcard_union_all() {
    let mut e = Engine::new();

    // 1. Nested aggregate inside CAST(DATEADD(..., CAST(MIN(col) AS DATE)) AS DATETIME2(0))
    exec(&mut e, "CREATE TABLE #events (ts DATETIME)");
    exec(
        &mut e,
        "INSERT INTO #events VALUES ('2026-03-10 08:00:00'), ('2026-03-15 12:00:00')",
    );
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_nested_agg
        AS
        BEGIN
            DECLARE @min_d DATETIME2(0), @max_d DATETIME2(0);
            SELECT @min_d = CAST(DATEADD(DAY, -1, CAST(MIN(ts) AS DATE)) AS DATETIME2(0)),
                   @max_d = CAST(DATEADD(DAY,  1, CAST(MAX(ts) AS DATE)) AS DATETIME2(0))
            FROM #events;
            SELECT CAST(@min_d AS DATE) AS d1, CAST(@max_d AS DATE) AS d2;
        END",
    );
    let r1 = query(&mut e, "EXEC dbo.p_nested_agg");
    assert_eq!(r1.rows[0][0].to_string_value(), "2026-03-09");
    assert_eq!(r1.rows[0][1].to_string_value(), "2026-03-16");

    // 2. UNION ALL of SELECT * where one side is empty (0 rows) and the other has rows
    exec(&mut e, "CREATE TABLE #part_a (id INT, label VARCHAR(20))");
    exec(&mut e, "CREATE TABLE #part_b (id INT, label VARCHAR(20))");
    exec(&mut e, "INSERT INTO #part_b VALUES (1, 'ok')");
    let r2 = query(
        &mut e,
        "SELECT * FROM (SELECT * FROM #part_a UNION ALL SELECT * FROM #part_b) u",
    );
    assert_eq!(r2.rows.len(), 1);
    assert_eq!(r2.columns, vec!["id", "label"]);
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r2.rows[0][1].to_string_value(), "ok");
}

#[test]
fn delete_alias_from_table_and_sp_executesql_same_name_output_var() {
    let mut e = Engine::new();

    // 1. DELETE alias FROM tbl alias INNER JOIN ...
    exec(&mut e, "CREATE TABLE dbo.items (id INT, tenant INT)");
    exec(&mut e, "CREATE TABLE dbo.to_del (id INT)");
    exec(
        &mut e,
        "INSERT INTO dbo.items VALUES (1, 10), (2, 10), (3, 20)",
    );
    exec(&mut e, "INSERT INTO dbo.to_del VALUES (2), (3)");
    exec(
        &mut e,
        "DELETE x
         FROM dbo.items x
         INNER JOIN dbo.to_del d ON x.id = d.id
         WHERE x.tenant = 10",
    );
    let r1 = query(&mut e, "SELECT id FROM dbo.items ORDER BY id");
    assert_eq!(r1.rows.len(), 2);
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r1.rows[1][0].to_integer_i64(), Some(3));

    // 2. sp_executesql with same-name OUTPUT parameter must preserve caller's variable
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_dyn_count
        AS
        BEGIN
            DECLARE @RowCount BIGINT = 0;
            DECLARE @sql NVARCHAR(MAX) = N'SELECT @RowCount = COUNT(*) FROM dbo.items';
            EXEC sp_executesql @sql, N'@RowCount BIGINT OUTPUT', @RowCount = @RowCount OUTPUT;
            SET @RowCount = @RowCount + 10;
            SELECT @RowCount AS final_cnt;
        END",
    );
    let r2 = query(&mut e, "EXEC dbo.p_dyn_count");
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(12));
}

#[test]
fn select_assign_joins_sp_executesql_named_stmt_convert_trunc_and_dateadd_null() {
    let mut e = Engine::new();

    // 1. SELECT @var = ... FROM a LEFT JOIN b ON ...
    exec(&mut e, "CREATE TABLE dbo.a (id INT)");
    exec(&mut e, "CREATE TABLE dbo.b (a_id INT, label VARCHAR(20))");
    exec(&mut e, "INSERT INTO dbo.a VALUES (1)");
    exec(&mut e, "INSERT INTO dbo.b VALUES (1, 'found')");
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_assign_join
        AS
        BEGIN
            DECLARE @out VARCHAR(20) = 'none';
            SELECT @out = b.label
            FROM dbo.a a
            LEFT JOIN dbo.b b ON b.a_id = a.id
            WHERE b.label = 'found';
            SELECT @out AS res;
        END",
    );
    let r1 = query(&mut e, "EXEC dbo.p_assign_join");
    assert_eq!(r1.rows[0][0].to_string_value(), "found");

    // 2. EXEC sp_executesql @stmt = @sql (named @stmt argument)
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_named_sp_exec
        AS
        BEGIN
            DECLARE @q NVARCHAR(MAX) = N'SELECT 99 AS val';
            EXEC sp_executesql @stmt = @q;
        END",
    );
    let r2 = query(&mut e, "EXEC dbo.p_named_sp_exec");
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(99));

    // 3. CONVERT(VARCHAR(10), dt, 120) truncates to 10 chars
    let r3 = query(
        &mut e,
        "SELECT CONVERT(VARCHAR(10), CAST('2026-03-01 14:30:00' AS DATETIME), 120) AS d10",
    );
    assert_eq!(r3.rows[0][0].to_string_value(), "2026-03-01");

    // 4. DATEADD / DATEDIFF / DATEPART NULL propagation + numeric truncation
    let r4 = query(
        &mut e,
        "SELECT DATEADD(DAY, -1, NULL) AS n1,
                DATEADD(DAY, NULL, '2026-01-01') AS n2,
                CAST(DATEADD(DAY, 2.9, '2026-01-01') AS DATE) AS d_trunc,
                DATEDIFF(DAY, NULL, '2026-01-02') AS df_null",
    );
    assert_eq!(r4.rows[0][0], Value::Null);
    assert_eq!(r4.rows[0][1], Value::Null);
    assert_eq!(r4.rows[0][2].to_string_value(), "2026-01-03");
    assert_eq!(r4.rows[0][3], Value::Null);
}

#[test]
fn procedural_temp_table_auto_drops_on_exit_and_isolates_nested_scope() {
    let mut e = Engine::new();

    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_make_temp @val INT
        AS
        BEGIN
            CREATE TABLE #work (n INT);
            INSERT INTO #work VALUES (@val);
            SELECT n * 10 AS scaled INTO #work2 FROM #work;
            SELECT scaled FROM #work2;
        END",
    );

    // Caller also has its own #work table — procedure's #work must neither collide nor clobber it
    exec(&mut e, "CREATE TABLE #work (n INT)");
    exec(&mut e, "INSERT INTO #work VALUES (777)");

    // Calling the procedure twice must not fail with duplicate table error
    let r1 = query(&mut e, "EXEC dbo.p_make_temp @val = 3");
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(30));
    let r2 = query(&mut e, "EXEC dbo.p_make_temp @val = 4");
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(40));

    // Caller's #work table is still intact
    let r3 = query(&mut e, "SELECT n FROM #work");
    assert_eq!(r3.rows[0][0].to_integer_i64(), Some(777));
}

#[test]
fn updatable_cte_update_and_delete_with_window_functions() {
    let mut e = Engine::new();

    exec(
        &mut e,
        "CREATE TABLE #staging (id INT, grp VARCHAR(10), status VARCHAR(20))",
    );
    exec(
        &mut e,
        "INSERT INTO #staging VALUES (1, 'A', 'New'), (2, 'A', 'New'), (3, 'B', 'New'), (4, 'A', 'New')",
    );

    exec(
        &mut e,
        "WITH ranked AS (
            SELECT id, grp, status,
                   ROW_NUMBER() OVER (PARTITION BY grp ORDER BY id) AS rn
            FROM #staging
        )
        UPDATE ranked SET status = 'Duplicate' WHERE rn > 1;",
    );

    let r1 = query(&mut e, "SELECT id, status FROM #staging ORDER BY id");
    assert_eq!(r1.rows.len(), 4);
    assert_eq!(r1.rows[0][1].to_string_value(), "New");
    assert_eq!(r1.rows[1][1].to_string_value(), "Duplicate");
    assert_eq!(r1.rows[2][1].to_string_value(), "New");
    assert_eq!(r1.rows[3][1].to_string_value(), "Duplicate");

    exec(
        &mut e,
        "WITH dups AS (
            SELECT id, ROW_NUMBER() OVER (PARTITION BY grp ORDER BY id) AS rn
            FROM #staging
        )
        DELETE FROM dups WHERE rn > 1;",
    );

    let r2 = query(&mut e, "SELECT id FROM #staging ORDER BY id");
    assert_eq!(r2.rows.len(), 2);
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r2.rows[1][0].to_integer_i64(), Some(3));
}

#[test]
fn optional_omitted_tvp_parameter_and_bit_severity_raiserror() {
    let mut e = Engine::new();

    exec(&mut e, "CREATE TYPE dbo.IntList AS TABLE (Val INT)");
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_opt_tvp
            @tag VARCHAR(20),
            @items dbo.IntList READONLY,
            @debug BIT = 0
        AS
        BEGIN
            RAISERROR('%s', @debug, 1, @tag) WITH NOWAIT;
            SELECT COUNT(*) AS cnt FROM @items;
        END",
    );

    // Omitting @items defaults to an empty table; @debug = 0 and 1 are informational severities
    let r1 = query(&mut e, "EXEC dbo.p_opt_tvp @tag = 'ok', @debug = 0");
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(0));

    let r2 = query(&mut e, "EXEC dbo.p_opt_tvp @tag = 'debug', @debug = 1");
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(0));
}

#[test]
fn collate_cs_ci_isnumeric_formatmessage_dotnet_format_and_conversions() {
    let mut e = Engine::new();

    // 1. COLLATE case-sensitive vs case-insensitive
    let r1 = query(
        &mut e,
        "SELECT
            CASE WHEN 'Secret' = 'secret' COLLATE Latin1_General_CS_AS THEN 1 ELSE 0 END,
            CASE WHEN 'Secret' = 'Secret' COLLATE Latin1_General_CS_AS THEN 1 ELSE 0 END,
            CASE WHEN 'Secret' = 'secret' COLLATE Latin1_General_CI_AS THEN 1 ELSE 0 END",
    );
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(0));
    assert_eq!(r1.rows[0][1].to_integer_i64(), Some(1));
    assert_eq!(r1.rows[0][2].to_integer_i64(), Some(1));

    // 2. ISNUMERIC, FORMATMESSAGE, and .NET datetime FORMAT
    let r2 = query(
        &mut e,
        "SELECT
            ISNUMERIC('123.45'),
            ISNUMERIC('-$25.50'),
            ISNUMERIC('abc'),
            ISNUMERIC(NULL),
            FORMATMESSAGE('Item %d: %s', 7, 'ok'),
            FORMAT(CAST('2026-06-15 14:30:05' AS DATETIME), 'yyyy-MM-dd 00:00:00')",
    );
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r2.rows[0][1].to_integer_i64(), Some(1));
    assert_eq!(r2.rows[0][2].to_integer_i64(), Some(0));
    assert_eq!(r2.rows[0][3].to_integer_i64(), Some(0));
    assert_eq!(r2.rows[0][4].to_string_value(), "Item 7: ok");
    assert_eq!(r2.rows[0][5].to_string_value(), "2026-06-15 00:00:00");

    // 3. Leading-dot DECIMAL, signed currency MONEY, time-only DATETIME, empty-string INT
    let r3 = query(
        &mut e,
        "SELECT
            CAST('.01' AS DECIMAL(10, 2)),
            CAST('-.10' AS DECIMAL(10, 2)),
            CAST('-$25.5000' AS MONEY),
            CAST('08:00:00' AS DATETIME),
            CAST('' AS INT)",
    );
    assert_eq!(r3.rows[0][0], Value::Decimal(1, 2));
    assert_eq!(r3.rows[0][1], Value::Decimal(-10, 2));
    assert_eq!(r3.rows[0][2], Value::Money(-255000));
    assert_eq!(r3.rows[0][3].to_string_value(), "1900-01-01 08:00:00");
    assert_eq!(r3.rows[0][4].to_integer_i64(), Some(0));
}

#[test]
fn merge_unaliased_and_cte_target_update_apply_index_include_waitfor_and_stats() {
    let mut e = Engine::new();

    // 1. Unaliased MERGE target (USING must not be consumed as alias)
    exec(
        &mut e,
        "CREATE TABLE dbo.m_items (id INT, owner_id INT, val INT)",
    );
    exec(
        &mut e,
        "INSERT INTO dbo.m_items VALUES (1, 10, 100), (2, 10, 200), (3, 20, 300)",
    );
    exec(
        &mut e,
        "MERGE dbo.m_items
         USING (SELECT 1 AS id, 10 AS owner_id, 150 AS val) AS src
            ON dbo.m_items.id = src.id
         WHEN MATCHED THEN UPDATE SET val = src.val;",
    );
    let r0 = query(&mut e, "SELECT val FROM dbo.m_items WHERE id = 1");
    assert_eq!(r0.rows[0][0].to_integer_i64(), Some(150));

    // 2. Partitioned updatable CTE MERGE (only owner_id = 10 rows touched; owner_id = 20 preserved)
    exec(
        &mut e,
        "WITH TARGET AS (SELECT * FROM dbo.m_items WHERE owner_id = 10)
         MERGE TARGET
         USING (VALUES (1, 111), (4, 444)) AS SOURCE (id, val)
            ON TARGET.id = SOURCE.id
         WHEN MATCHED THEN UPDATE SET val = SOURCE.val
         WHEN NOT MATCHED BY TARGET THEN INSERT (id, owner_id, val) VALUES (SOURCE.id, 10, SOURCE.val)
         WHEN NOT MATCHED BY SOURCE THEN DELETE;",
    );
    let r1 = query(
        &mut e,
        "SELECT id, owner_id, val FROM dbo.m_items ORDER BY id",
    );
    assert_eq!(r1.rows.len(), 3);
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r1.rows[0][2].to_integer_i64(), Some(111));
    assert_eq!(r1.rows[1][0].to_integer_i64(), Some(3));
    assert_eq!(r1.rows[1][1].to_integer_i64(), Some(20));
    assert_eq!(r1.rows[2][0].to_integer_i64(), Some(4));
    assert_eq!(r1.rows[2][2].to_integer_i64(), Some(444));

    // 3. UPDATE and DELETE with CROSS APPLY + JOIN in FROM
    exec(&mut e, "CREATE TABLE #acc (id INT, total INT)");
    exec(&mut e, "CREATE TABLE #delta (acc_id INT, amt INT)");
    exec(&mut e, "INSERT INTO #acc VALUES (1, 10), (2, 20)");
    exec(&mut e, "INSERT INTO #delta VALUES (1, 5), (1, 7)");
    exec(
        &mut e,
        "UPDATE a
         SET a.total = a.total + ca.s
         FROM #acc a
         CROSS APPLY (SELECT ISNULL(SUM(d.amt), 0) AS s FROM #delta d WHERE d.acc_id = a.id) ca;",
    );
    let r2 = query(&mut e, "SELECT id, total FROM #acc ORDER BY id");
    assert_eq!(r2.rows[0][1].to_integer_i64(), Some(22));
    assert_eq!(r2.rows[1][1].to_integer_i64(), Some(20));

    // 4. CREATE INDEX with INCLUDE / WHERE / WITH options, WAITFOR DELAY, UPDATE STATISTICS
    exec(
        &mut e,
        "CREATE NONCLUSTERED INDEX IX_acc_id ON #acc (id ASC) INCLUDE (total) WHERE id > 0 WITH (FILLFACTOR = 80, PAD_INDEX = ON);",
    );
    exec(&mut e, "UPDATE STATISTICS #acc WITH FULLSCAN;");
    exec(&mut e, "WAITFOR DELAY '00:00:00';");
}

#[test]
fn wave11a_derived_col_list_delete_output_before_from_persisted_try_parse_compound_update_at_tz() {
    let mut e = Engine::new();

    // 1. Derived table column alias list in MERGE USING (...) AS S (c1, c2)
    exec(&mut e, "CREATE TABLE #bal (k INT, amt INT) ON [PRIMARY]");
    exec(&mut e, "INSERT INTO #bal VALUES (1, 10), (2, 20)");
    exec(
        &mut e,
        "MERGE #bal AS T
         USING (SELECT 1, 55 UNION ALL SELECT 2, 77) AS S (k, new_amt)
            ON T.k = S.k
         WHEN MATCHED THEN UPDATE SET amt = S.new_amt;",
    );
    let r1 = query(&mut e, "SELECT k, amt FROM #bal ORDER BY k");
    assert_eq!(r1.rows[0][1].to_integer_i64(), Some(55));
    assert_eq!(r1.rows[1][1].to_integer_i64(), Some(77));

    // 2. DELETE alias OUTPUT ... INTO ... FROM tbl alias
    exec(&mut e, "CREATE TABLE #del_log (k INT)");
    exec(
        &mut e,
        "DELETE b
         OUTPUT deleted.k INTO #del_log
         FROM #bal b
         WHERE b.k = 1;",
    );
    let r2 = query(&mut e, "SELECT k FROM #del_log");
    assert_eq!(r2.rows.len(), 1);
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(1));

    // 3. Computed column PERSISTED + TRY_PARSE
    exec(
        &mut e,
        "CREATE TABLE #comp (a INT, doubled AS (a * 2) PERSISTED, INDEX IX_a CLUSTERED (a));",
    );
    exec(&mut e, "INSERT INTO #comp (a) VALUES (7)");
    let r3 = query(
        &mut e,
        "SELECT doubled, TRY_PARSE('42' AS INT), TRY_PARSE('nope' AS INT) FROM #comp",
    );
    assert_eq!(r3.rows[0][0].to_integer_i64(), Some(14));
    assert_eq!(r3.rows[0][1].to_integer_i64(), Some(42));
    assert_eq!(r3.rows[0][2], Value::Null);

    // 4. UPDATE compound += and chained @var = col += 1
    exec(
        &mut e,
        "CREATE TABLE dbo.seq_counter (cid INT, last_val INT)",
    );
    exec(&mut e, "INSERT INTO dbo.seq_counter VALUES (1, 10)");
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_next_num @cid INT, @next INT OUTPUT AS
         BEGIN
             UPDATE dbo.seq_counter
             SET @next = last_val += 1
             WHERE cid = @cid;
         END",
    );
    let r4 = e
        .execute_batch(
            iridium_core::parse_batch(
                "DECLARE @n INT; EXEC dbo.p_next_num @cid = 1, @next = @n OUTPUT; SELECT @n, last_val FROM dbo.seq_counter WHERE cid = 1;",
            )
            .unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(r4.rows[0][0].to_integer_i64(), Some(11));
    assert_eq!(r4.rows[0][1].to_integer_i64(), Some(11));

    // 5. AT TIME ZONE conversion
    let r5 = query(
        &mut e,
        "SELECT CAST(CAST('2026-01-15 12:00:00' AS DATETIME) AT TIME ZONE 'UTC' AT TIME ZONE 'Eastern Standard Time' AS DATETIME)",
    );
    assert_eq!(r5.rows[0][0].to_string_value(), "2026-01-15 07:00:00");
}

#[test]
fn wave11b_rowcount_insert_skips_identity_select_into_union_distinct_and_datetime_float() {
    let mut e = Engine::new();

    // 1. @@ROWCOUNT across INSERT, UPDATE, DELETE, SELECT
    exec(
        &mut e,
        "CREATE TABLE #rc (id INT IDENTITY(1,1), tag VARCHAR(20))",
    );
    // INSERT without column list skips IDENTITY column automatically
    let r1 = e
        .execute_batch(
            iridium_core::parse_batch(
                "INSERT INTO #rc SELECT 'alpha' UNION ALL SELECT 'beta';
                 DECLARE @ins INT = @@ROWCOUNT;
                 UPDATE #rc SET tag = 'ALPHA' WHERE tag = 'alpha';
                 DECLARE @upd INT = @@ROWCOUNT;
                 DELETE FROM #rc WHERE tag = 'beta';
                 DECLARE @del INT = @@ROWCOUNT;
                 SELECT @ins, @upd, @del;",
            )
            .unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(2));
    assert_eq!(r1.rows[0][1].to_integer_i64(), Some(1));
    assert_eq!(r1.rows[0][2].to_integer_i64(), Some(1));

    // 2. SELECT INTO with UNION ALL
    exec(
        &mut e,
        "SELECT 1 AS n INTO #union_dst UNION ALL SELECT 2 AS n;",
    );
    let r2 = query(&mut e, "SELECT COUNT(*), SUM(n) FROM #union_dst");
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(2));
    assert_eq!(r2.rows[0][1].to_integer_i64(), Some(3));

    // 3. SELECT DISTINCT normalizes case-insensitive / trimmed strings
    exec(&mut e, "CREATE TABLE #dups (s VARCHAR(10))");
    exec(
        &mut e,
        "INSERT INTO #dups VALUES ('Abc'), ('abc  '), ('ABC')",
    );
    let r3 = query(&mut e, "SELECT DISTINCT s FROM #dups");
    assert_eq!(r3.rows.len(), 1);

    // 4. CAST(DATETIME AS FLOAT) returns SQL Server epoch days since 1900-01-01
    let r4 = query(
        &mut e,
        "SELECT CAST(CAST('1900-01-02 12:00:00' AS DATETIME) AS FLOAT)",
    );
    assert_eq!(r4.rows[0][0].to_f64(), Some(1.5));
}

#[test]
fn trigger_exposes_empty_inserted_on_delete_year_only_date_and_paren_insert_select() {
    let mut e = Engine::new();

    // 1. Trigger on DELETE still exposes empty INSERTED pseudo-table + 4-digit year string '9999'
    exec(&mut e, "CREATE TABLE dbo.tr_src (id INT, name VARCHAR(20))");
    exec(
        &mut e,
        "CREATE TABLE dbo.tr_audit (id INT, act CHAR(1), end_dt DATETIME)",
    );
    exec(
        &mut e,
        "CREATE TRIGGER dbo.trg_tr_src_audit ON dbo.tr_src AFTER UPDATE, DELETE AS
         BEGIN
             IF NOT EXISTS (SELECT 1 FROM INSERTED)
                 INSERT INTO dbo.tr_audit (id, act, end_dt)
                 SELECT id, 'D', CAST('9999' AS DATETIME) FROM DELETED;
         END",
    );
    exec(&mut e, "INSERT INTO dbo.tr_src VALUES (1, 'one')");
    exec(&mut e, "DELETE FROM dbo.tr_src WHERE id = 1");
    let r1 = query(&mut e, "SELECT id, act, end_dt FROM dbo.tr_audit");
    assert_eq!(r1.rows.len(), 1);
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r1.rows[0][1].to_string_value().trim(), "D");
    assert_eq!(r1.rows[0][2].to_string_value(), "9999-01-01 00:00:00");

    // 2. Parenthesized INSERT INTO tbl (SELECT ...)
    exec(&mut e, "CREATE TABLE #p_ins (v INT)");
    exec(&mut e, "INSERT INTO #p_ins (SELECT 42)");
    let r2 = query(&mut e, "SELECT v FROM #p_ins");
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(42));
}

#[test]
fn bare_throw_nullable_param_insert_exec_rc_alter_proc_toggle_trigger_default_pk_and_xml_modify() {
    let mut e = Engine::new();

    // 1. Bare THROW followed by RETURN without semicolon + nullable procedure parameter + EXEC before END
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_rethrow (@x INT NULL) AS
         BEGIN
             BEGIN TRY
                 IF @x IS NULL RAISERROR('null_x', 16, 1)
             END TRY
             BEGIN CATCH
                 THROW
                 RETURN 55555
             END CATCH
             EXEC dbo.p_noop
         END",
    );
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_noop AS BEGIN SELECT 1 AS ok END",
    );
    assert!(e.exec("EXEC dbo.p_rethrow NULL").is_err());

    // 2. INSERT INTO tbl EXEC @rc = proc + ALTER PROCEDURE + bare first-statement proc call
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_val (@n INT) AS BEGIN SELECT @n AS v; RETURN 7; END",
    );
    exec(&mut e, "CREATE TABLE #res (v INT)");
    exec(&mut e, "DECLARE @rc INT = 0");
    exec(&mut e, "INSERT INTO #res EXEC @rc = dbo.p_val 99");
    let r = query(&mut e, "SELECT @rc, (SELECT v FROM #res)");
    assert_eq!(r.rows[0][0].to_integer_i64(), Some(7));
    assert_eq!(r.rows[0][1].to_integer_i64(), Some(99));

    exec(
        &mut e,
        "ALTER PROCEDURE dbo.p_val (@n INT) AS BEGIN SELECT @n + 1 AS v; RETURN 8; END",
    );
    let r2 = query(&mut e, "EXEC ('dbo.p_val 10')");
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(11));

    // 3. ALTER TABLE ADD CONSTRAINT DEFAULT + composite PRIMARY KEY enforcement
    exec(
        &mut e,
        "CREATE TABLE dbo.t_c (a INT NOT NULL, b INT NOT NULL, c INT)",
    );
    exec(
        &mut e,
        "ALTER TABLE dbo.t_c ADD CONSTRAINT DF_t_c_c DEFAULT (42) FOR c",
    );
    exec(
        &mut e,
        "ALTER TABLE dbo.t_c ADD CONSTRAINT PK_t_c PRIMARY KEY CLUSTERED (a, b)",
    );
    exec(&mut e, "INSERT INTO dbo.t_c (a, b) VALUES (1, 2)");
    let r3 = query(&mut e, "SELECT c FROM dbo.t_c WHERE a = 1 AND b = 2");
    assert_eq!(r3.rows[0][0].to_integer_i64(), Some(42));
    assert!(e.exec("INSERT INTO dbo.t_c (a, b) VALUES (1, 2)").is_err());

    // 4. DISABLE TRIGGER / ENABLE TRIGGER
    exec(&mut e, "CREATE TABLE dbo.t_tr (id INT)");
    exec(&mut e, "CREATE TABLE dbo.t_tr_log (id INT)");
    exec(
        &mut e,
        "CREATE TRIGGER dbo.trg_t_tr ON dbo.t_tr AFTER INSERT AS
         BEGIN INSERT INTO dbo.t_tr_log SELECT id FROM INSERTED; END",
    );
    exec(&mut e, "INSERT INTO dbo.t_tr VALUES (1)");
    exec(&mut e, "DISABLE TRIGGER dbo.trg_t_tr ON dbo.t_tr");
    exec(&mut e, "INSERT INTO dbo.t_tr VALUES (2)");
    exec(&mut e, "ENABLE TRIGGER dbo.trg_t_tr ON dbo.t_tr");
    exec(&mut e, "INSERT INTO dbo.t_tr VALUES (3)");
    let r4 = query(&mut e, "SELECT COUNT(*) FROM dbo.t_tr_log");
    assert_eq!(r4.rows[0][0].to_integer_i64(), Some(2));

    // 5. XML .modify('delete (...)') and .modify('insert sql:column(...) into (...)')
    exec(&mut e, "CREATE TABLE #x1 (id INT, doc XML)");
    exec(&mut e, "CREATE TABLE #x2 (id INT, frag XML)");
    exec(
        &mut e,
        "INSERT INTO #x1 VALUES (1, '<Root><Old>x</Old><Keep>y</Keep></Root>')",
    );
    exec(&mut e, "INSERT INTO #x2 VALUES (1, '<New>z</New>')");
    exec(&mut e, "UPDATE #x1 SET doc.modify('delete (/Root/Old)')");
    exec(
        &mut e,
        "UPDATE a SET doc.modify('insert sql:column(\"b.frag\") into (/Root)[1]') FROM #x1 a JOIN #x2 b ON a.id = b.id",
    );
    let r5 = query(&mut e, "SELECT doc FROM #x1 WHERE id = 1");
    assert_eq!(
        r5.rows[0][0].to_string_value(),
        "<Root><Keep>y</Keep><New>z</New></Root>"
    );
}

#[test]
fn nullif_sql_eq_stuff_bounds_money_varchar_custom_format_nvarchar_chars_param_trunc_and_like_classes() {
    let mut e = Engine::new();

    // 1. NULLIF uses SQL equality (NVARCHAR vs VARCHAR, trailing spaces)
    let r1 = query(
        &mut e,
        "SELECT COALESCE(NULLIF(N'', ''), 'fallback'), COALESCE(NULLIF(N'   ', ''), 'fallback2')",
    );
    assert_eq!(r1.rows[0][0].to_string_value(), "fallback");
    assert_eq!(r1.rows[0][1].to_string_value(), "fallback2");

    // 2. STUFF returns NULL when start <= 0 or start > len(str)
    let r2 = query(&mut e, "SELECT STUFF('', 1, 2, ''), STUFF('ab', 5, 1, 'x'), STUFF('abcd', 1, 2, '')");
    assert!(r2.rows[0][0].is_null());
    assert!(r2.rows[0][1].is_null());
    assert_eq!(r2.rows[0][2].to_string_value(), "cd");

    // 3. MONEY -> VARCHAR formats 2dp without '$', and CONCAT formats MONEY the same way
    let r3 = query(
        &mut e,
        "SELECT CAST(CAST(-25.50 AS MONEY) AS VARCHAR(20)), CONCAT('amt=', CAST(-25.50 AS MONEY))",
    );
    assert_eq!(r3.rows[0][0].to_string_value(), "-25.50");
    assert_eq!(r3.rows[0][1].to_string_value(), "amt=-25.50");

    // 4. FORMAT with .NET custom numeric patterns (0.0000, #,0.00####, 0.00)
    let r4 = query(
        &mut e,
        "SELECT FORMAT(CAST(1000 AS DECIMAL(18,4)), '0.0000', 'en-US'),
                FORMAT(CAST(2.5 AS DECIMAL(18,6)), '0.0000'),
                FORMAT(CAST(1234.5 AS DECIMAL(18,4)), '#,0.00####'),
                FORMAT(980, '0.00')",
    );
    assert_eq!(r4.rows[0][0].to_string_value(), "1000.0000");
    assert_eq!(r4.rows[0][1].to_string_value(), "2.5000");
    assert_eq!(r4.rows[0][2].to_string_value(), "1,234.50");
    assert_eq!(r4.rows[0][3].to_string_value(), "980.00");

    // 5. NVARCHAR(N) enforces Unicode character count, not UTF-8 byte length
    exec(&mut e, "CREATE TABLE #u (s NVARCHAR(5))");
    exec(&mut e, "INSERT INTO #u VALUES (REPLICATE(N'É', 5))");
    let r5 = query(&mut e, "SELECT LEN(s) FROM #u");
    assert_eq!(r5.rows[0][0].to_integer_i64(), Some(5));

    // 6. Procedure VARCHAR(N) parameter truncates longer inputs
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.p_len (@code VARCHAR(5)) AS BEGIN SELECT @code AS c; END",
    );
    let r6 = query(&mut e, "EXEC dbo.p_len 'ABCDEFGH'");
    assert_eq!(r6.rows[0][0].to_string_value(), "ABCDE");

    // 7. T-SQL LIKE bracket character classes ([%], [_], [[], [a-z], [^0-9])
    let r7 = query(
        &mut e,
        "SELECT CASE WHEN 'INV_50% [X]' LIKE '%[_]50[%] [[]X]%' THEN 1 ELSE 0 END,
                CASE WHEN 'INVX50A [X]' LIKE '%[_]50[%] [[]X]%' THEN 1 ELSE 0 END,
                CASE WHEN 'B' LIKE '[A-C]' THEN 1 ELSE 0 END,
                CASE WHEN 'D' LIKE '[^A-C]' THEN 1 ELSE 0 END",
    );
    assert_eq!(r7.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r7.rows[0][1].to_integer_i64(), Some(0));
    assert_eq!(r7.rows[0][2].to_integer_i64(), Some(1));
    assert_eq!(r7.rows[0][3].to_integer_i64(), Some(1));
}

#[test]
fn fk_check_nocheck_recursive_triggers_off_select_assign_apply_cte_cols_merge_var_8672_group_window_and_empty_agg_col() {
    let mut e = Engine::new();

    // 1. FK NOCHECK/CHECK CONSTRAINT + ON DELETE SET NULL
    exec(&mut e, "CREATE TABLE dbo.p_parent (id INT PRIMARY KEY)");
    exec(
        &mut e,
        "CREATE TABLE dbo.p_child (id INT PRIMARY KEY, pid INT NULL, CONSTRAINT FK_p_child_parent FOREIGN KEY (pid) REFERENCES dbo.p_parent(id) ON DELETE SET NULL)",
    );
    exec(&mut e, "ALTER TABLE dbo.p_child NOCHECK CONSTRAINT ALL");
    exec(&mut e, "INSERT INTO dbo.p_child VALUES (1, 999)");
    exec(&mut e, "DELETE FROM dbo.p_child");
    exec(&mut e, "ALTER TABLE dbo.p_child CHECK CONSTRAINT FK_p_child_parent");
    exec(&mut e, "INSERT INTO dbo.p_parent VALUES (10)");
    exec(&mut e, "INSERT INTO dbo.p_child VALUES (1, 10)");
    exec(&mut e, "DELETE FROM dbo.p_parent WHERE id = 10");
    let r1 = query(&mut e, "SELECT pid FROM dbo.p_child WHERE id = 1");
    assert!(r1.rows[0][0].is_null());

    // 2. RECURSIVE_TRIGGERS OFF prevents infinite direct recursion
    e.set_recursive_triggers(false);
    exec(&mut e, "CREATE TABLE dbo.t_self (id INT, status INT, is_active BIT)");
    exec(
        &mut e,
        "CREATE TRIGGER dbo.trg_t_self ON dbo.t_self AFTER UPDATE AS
         BEGIN
             UPDATE t SET is_active = CASE WHEN i.status = 0 THEN 0 ELSE 1 END
             FROM dbo.t_self t JOIN INSERTED i ON t.id = i.id;
         END",
    );
    exec(&mut e, "INSERT INTO dbo.t_self VALUES (1, 1, 1)");
    exec(&mut e, "UPDATE dbo.t_self SET status = 0 WHERE id = 1");
    let r2 = query(&mut e, "SELECT is_active FROM dbo.t_self WHERE id = 1");
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(0));

    // 3. SELECT TOP 1 @var = ... FROM T CROSS/OUTER APPLY (...) Alias ORDER BY ...
    exec(&mut e, "CREATE TABLE dbo.t_base (id INT, grp INT)");
    exec(&mut e, "INSERT INTO dbo.t_base VALUES (1, 10), (2, 20)");
    exec(&mut e, "DECLARE @picked INT = 0");
    exec(
        &mut e,
        "SELECT TOP 1 @picked = A.score
         FROM dbo.t_base B
         CROSS APPLY (SELECT B.grp * 2 AS score) A
         ORDER BY A.score DESC",
    );
    let r3 = query(&mut e, "SELECT @picked");
    assert_eq!(r3.rows[0][0].to_integer_i64(), Some(40));

    // 4. WITH Cte(col1, col2) column alias list used in UPDATE ... FROM
    exec(&mut e, "CREATE TABLE #upd_t (id INT, flag INT)");
    exec(&mut e, "INSERT INTO #upd_t VALUES (1, 0), (2, 0)");
    exec(
        &mut e,
        ";WITH FlagCte(id, computed_flag) AS (SELECT id, CASE WHEN id = 1 THEN 1 ELSE 0 END FROM #upd_t)
         UPDATE #upd_t SET flag = computed_flag FROM FlagCte S JOIN #upd_t E ON S.id = E.id",
    );
    let r4 = query(&mut e, "SELECT flag FROM #upd_t ORDER BY id");
    assert_eq!(r4.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r4.rows[1][0].to_integer_i64(), Some(0));

    // 5. MERGE WHEN MATCHED THEN UPDATE SET @var = 1 + Error 8672 on duplicate source match
    exec(&mut e, "CREATE TABLE dbo.m_tgt (id INT, val INT)");
    exec(&mut e, "INSERT INTO dbo.m_tgt VALUES (1, 100)");
    exec(&mut e, "DECLARE @was_updated BIT = 0");
    exec(
        &mut e,
        "MERGE dbo.m_tgt AS T
         USING (SELECT 1 AS id, 200 AS val) AS S ON T.id = S.id
         WHEN MATCHED THEN UPDATE SET @was_updated = 1;",
    );
    let r5 = query(&mut e, "SELECT @was_updated");
    assert_eq!(r5.rows[0][0].to_integer_i64(), Some(1));
    assert!(e
        .exec(
            "MERGE dbo.m_tgt AS T
             USING (SELECT 1 AS id, 10 AS val UNION ALL SELECT 1, 20) AS S ON T.id = S.id
             WHEN MATCHED THEN UPDATE SET val = S.val;"
        )
        .is_err());

    // 6. GROUP BY + ROW_NUMBER() OVER (ORDER BY ...) in SELECT INTO
    exec(&mut e, "CREATE TABLE #batches (code VARCHAR(10))");
    exec(&mut e, "INSERT INTO #batches VALUES ('B'), ('A'), ('B')");
    exec(
        &mut e,
        "SELECT ROW_NUMBER() OVER (ORDER BY code) AS seq, code INTO #b_out FROM #batches GROUP BY code",
    );
    let r6 = query(&mut e, "SELECT seq, code FROM #b_out ORDER BY seq");
    assert_eq!(r6.rows.len(), 2);
    assert_eq!(r6.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r6.rows[0][1].to_string_value(), "A");
    assert_eq!(r6.rows[1][0].to_integer_i64(), Some(2));
    assert_eq!(r6.rows[1][1].to_string_value(), "B");

    // 7. Empty-table aggregate query validates nonexistent column name
    exec(&mut e, "CREATE TABLE dbo.t_empty (id INT)");
    assert!(e.exec("SELECT COUNT(*), MIN(no_such_col) FROM dbo.t_empty").is_err());
}

#[test]
fn grouped_window_with_sum_select_into_varchar_widen_composite_fk_null_datefromparts_cast_trunc_currency_commas_charindex_ci_error_number_coalesce_and_bare_exec_arg() {
    let mut e = Engine::new();

    // 1. GROUP BY + SUM() + ROW_NUMBER() OVER (...) in the same SELECT
    exec(&mut e, "CREATE TABLE #sales (grp VARCHAR(10), amt INT)");
    exec(&mut e, "INSERT INTO #sales VALUES ('B', 10), ('B', 5), ('A', 20)");
    let r1 = query(
        &mut e,
        "SELECT grp, SUM(amt) AS total, ROW_NUMBER() OVER (ORDER BY grp) AS rn FROM #sales GROUP BY grp ORDER BY grp",
    );
    assert_eq!(r1.rows.len(), 2);
    assert_eq!(r1.rows[0][0].to_string_value(), "A");
    assert_eq!(r1.rows[0][1].to_integer_i64(), Some(20));
    assert_eq!(r1.rows[0][2].to_integer_i64(), Some(1));
    assert_eq!(r1.rows[1][0].to_string_value(), "B");
    assert_eq!(r1.rows[1][1].to_integer_i64(), Some(15));
    assert_eq!(r1.rows[1][2].to_integer_i64(), Some(2));

    // 2. SELECT INTO widens inferred VARCHAR column so later UPDATE with longer string succeeds
    exec(&mut e, "SELECT CAST(123 AS VARCHAR(50)) AS code INTO #tmp_w");
    exec(&mut e, "UPDATE #tmp_w SET code = '0000001234'");
    let r2 = query(&mut e, "SELECT code FROM #tmp_w");
    assert_eq!(r2.rows[0][0].to_string_value(), "0000001234");

    // 3. Composite FK skips validation when any FK column is NULL
    exec(&mut e, "CREATE TABLE dbo.cfk_parent (a INT NOT NULL, b INT NOT NULL, CONSTRAINT PK_cfk_p PRIMARY KEY (a, b))");
    exec(&mut e, "CREATE TABLE dbo.cfk_child (id INT, a INT NOT NULL, b INT NULL, CONSTRAINT FK_cfk FOREIGN KEY (a, b) REFERENCES dbo.cfk_parent (a, b))");
    exec(&mut e, "INSERT INTO dbo.cfk_child VALUES (1, 99, NULL)");

    // 4. DATEFROMPARTS, CAST(TIME AS VARCHAR(2)) truncation, FORMAT C4 commas, CHARINDEX CI
    let r4 = query(
        &mut e,
        "SELECT DATEFROMPARTS(2026, 6, 15),
                CAST(CAST('10:00:00' AS TIME) AS VARCHAR(2)),
                FORMAT(CAST(1225 AS DECIMAL(18,4)), 'C4', 'en-US'),
                CHARINDEX('<html', '<HTML>hi</HTML>')",
    );
    assert_eq!(r4.rows[0][0].to_string_value(), "2026-06-15");
    assert_eq!(r4.rows[0][1].to_string_value(), "10");
    assert_eq!(r4.rows[0][2].to_string_value(), "$1,225.0000");
    assert_eq!(r4.rows[0][3].to_integer_i64(), Some(1));

    // 5. ERROR_NUMBER() returns 547 on CHECK constraint violation
    exec(&mut e, "CREATE TABLE dbo.t_chk (x INT, CONSTRAINT CK_t_chk CHECK (x <> 42))");
    exec(&mut e, "DECLARE @err INT = 0");
    exec(
        &mut e,
        "BEGIN TRY INSERT INTO dbo.t_chk VALUES (42); END TRY BEGIN CATCH SET @err = ERROR_NUMBER(); END CATCH",
    );
    let r5 = query(&mut e, "SELECT @err");
    assert_eq!(r5.rows[0][0].to_integer_i64(), Some(547));

    // 6. COALESCE(INT NULL, '') coerces to INT (0)
    exec(&mut e, "CREATE TABLE #c_int (n INT NULL)");
    exec(&mut e, "INSERT INTO #c_int VALUES (NULL)");
    let r6 = query(&mut e, "SELECT COALESCE(n, '') FROM #c_int");
    assert_eq!(r6.rows[0][0].to_integer_i64(), Some(0));

    // 7. Bare unquoted word argument in EXEC proc
    exec(&mut e, "CREATE PROCEDURE dbo.p_kind (@k VARCHAR(10)) AS BEGIN SELECT @k AS k; END");
    let r7 = query(&mut e, "EXEC ('dbo.p_kind FRT')");
    assert_eq!(r7.rows[0][0].to_string_value(), "FRT");
}
