use tsql_lite_core::types::Value;
use tsql_lite_core::{Engine, QueryResult};

fn run_sql(engine: &Engine, sql: &str) -> QueryResult {
    let stmts = tsql_lite_core::parse_batch(sql).expect("parse batch");
    engine
        .execute_batch(stmts)
        .expect("execute batch")
        .unwrap_or_default()
}

fn run_sql_all(engine: &Engine, sql: &str) -> Vec<QueryResult> {
    let stmts = tsql_lite_core::parse_batch(sql).expect("parse batch");
    engine
        .execute_batch_all_results(stmts)
        .expect("execute batch all results")
}

fn run_sql_err(engine: &Engine, sql: &str) -> String {
    let stmts = tsql_lite_core::parse_batch(sql).expect("parse batch");
    engine
        .execute_batch(stmts)
        .expect_err("expected SQL error")
        .to_string()
}

#[test]
fn exec_with_result_sets_renames_coerces_validates_and_handles_none_and_multi() {
    let engine = Engine::new();
    run_sql(
        &engine,
        "CREATE TABLE dbo.ShapeTemplate (ItemCode BIGINT NOT NULL, ShortName NVARCHAR(4) NULL);
         CREATE TYPE dbo.ShapeTvp AS TABLE (K INT, Label VARCHAR(3));",
    );
    run_sql(
        &engine,
        "CREATE PROCEDURE dbo.usp_TwoSets AS
         BEGIN
             SELECT 42 AS raw_id, 'abcdef' AS raw_name;
             SELECT 7 AS second_id, 'xyz' AS second_tag;
         END;",
    );
    run_sql(
        &engine,
        "CREATE PROCEDURE dbo.usp_NoResults AS
         BEGIN
             DECLARE @x INT = 1;
         END;",
    );
    run_sql(
        &engine,
        "CREATE PROCEDURE dbo.usp_NullRow AS
         BEGIN
             SELECT CAST(NULL AS INT) AS id;
         END;",
    );

    // 1. Single-set rename + type coercion + truncation via dynamic EXEC and sp_executesql
    let dyn_res = run_sql(
        &engine,
        "EXEC ('SELECT 123 AS a, ''hello_world'' AS b')
         WITH RESULT SETS ((OrderId BIGINT NOT NULL, Prefix NVARCHAR(5)));",
    );
    assert_eq!(dyn_res.columns, vec!["OrderId", "Prefix"]);
    assert_eq!(
        dyn_res.rows,
        vec![vec![Value::BigInt(123), Value::NVarChar("hello".into())]]
    );

    let sp_res = run_sql(
        &engine,
        "EXEC sys.sp_executesql N'SELECT @n AS x', N'@n INT', @n = 99
         WITH RESULT SETS ((ValText VARCHAR(10)));",
    );
    assert_eq!(sp_res.columns, vec!["ValText"]);
    assert_eq!(sp_res.rows, vec![vec![Value::VarChar("99".into())]]);

    // 2. Multi-result-set procedure reshaped across both sets
    let all = run_sql_all(
        &engine,
        "EXEC dbo.usp_TwoSets WITH RESULT SETS (
             (FirstId BIGINT, ShortLabel NVARCHAR(3)),
             (SecondCode VARCHAR(10), Tag NVARCHAR(10))
         );",
    );
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].columns, vec!["FirstId", "ShortLabel"]);
    assert_eq!(
        all[0].rows,
        vec![vec![Value::BigInt(42), Value::NVarChar("abc".into())]]
    );
    assert_eq!(all[1].columns, vec!["SecondCode", "Tag"]);
    assert_eq!(
        all[1].rows,
        vec![vec![
            Value::VarChar("7".into()),
            Value::NVarChar("xyz".into())
        ]]
    );

    // 3. AS OBJECT and AS TYPE definitions
    let obj_res = run_sql(
        &engine,
        "EXEC ('SELECT 55, ''ABCDEF''') WITH RESULT SETS (AS OBJECT dbo.ShapeTemplate);",
    );
    assert_eq!(obj_res.columns, vec!["ItemCode", "ShortName"]);
    assert_eq!(
        obj_res.rows,
        vec![vec![Value::BigInt(55), Value::NVarChar("ABCD".into())]]
    );

    let type_res = run_sql(
        &engine,
        "EXEC ('SELECT ''12'', ''WXYZ''') WITH RESULT SETS (AS TYPE dbo.ShapeTvp);",
    );
    assert_eq!(type_res.columns, vec!["K", "Label"]);
    assert_eq!(
        type_res.rows,
        vec![vec![Value::Int(12), Value::VarChar("WXY".into())]]
    );

    // 4. WITH RESULT SETS UNDEFINED passes through untouched
    let undef = run_sql(
        &engine,
        "EXEC ('SELECT 1 AS orig_col') WITH RESULT SETS UNDEFINED;",
    );
    assert_eq!(undef.columns, vec!["orig_col"]);

    // 5. WITH RESULT SETS NONE succeeds when no result sets are sent, errors when one is sent
    run_sql(&engine, "EXEC dbo.usp_NoResults WITH RESULT SETS NONE;");
    let err_none = run_sql_err(&engine, "EXEC dbo.usp_TwoSets WITH RESULT SETS NONE;");
    assert!(
        err_none.contains("0 result set"),
        "unexpected error: {err_none}"
    );

    // 6. Too many result sets errors
    let err_count = run_sql_err(
        &engine,
        "EXEC dbo.usp_TwoSets WITH RESULT SETS ((A INT, B VARCHAR(20)));",
    );
    assert!(
        err_count.contains("1 result set"),
        "unexpected error: {err_count}"
    );

    // 7. Column count mismatch errors
    let err_cols = run_sql_err(
        &engine,
        "EXEC ('SELECT 1, 2') WITH RESULT SETS ((OnlyOne INT));",
    );
    assert!(
        err_cols.contains("1 column(s)"),
        "unexpected error: {err_cols}"
    );

    // 8. NOT NULL violation errors
    let err_null = run_sql_err(
        &engine,
        "EXEC dbo.usp_NullRow WITH RESULT SETS ((Id INT NOT NULL));",
    );
    assert!(
        err_null.contains("NOT NULL"),
        "unexpected error: {err_null}"
    );
}

#[test]
fn with_schemabinding_blocks_drop_and_alter_of_referenced_objects_and_columns() {
    let engine = Engine::new();
    run_sql(
        &engine,
        "CREATE TABLE dbo.BaseItems (Id INT PRIMARY KEY, Price INT NOT NULL, Notes NVARCHAR(50) NULL);",
    );
    run_sql(
        &engine,
        "CREATE VIEW dbo.vBoundItems WITH SCHEMABINDING AS
         SELECT Id, Price FROM dbo.BaseItems;",
    );
    run_sql(
        &engine,
        "CREATE FUNCTION dbo.ufn_BasePrice(@x INT) RETURNS INT WITH SCHEMABINDING AS
         BEGIN
             RETURN @x * 10;
         END;",
    );
    run_sql(
        &engine,
        "CREATE VIEW dbo.vUsesFn WITH SCHEMABINDING AS
         SELECT Id, dbo.ufn_BasePrice(Price) AS ScaledPrice FROM dbo.BaseItems;",
    );

    // sys.sql_modules and sys.sql_expression_dependencies reflect is_schema_bound = 1
    let mod_res = run_sql(
        &engine,
        "SELECT is_schema_bound FROM sys.sql_modules WHERE object_id = OBJECT_ID('dbo.vBoundItems');",
    );
    assert_eq!(mod_res.rows, vec![vec![Value::Bit(true)]]);

    let dep_res = run_sql(
        &engine,
        "SELECT is_schema_bound_reference
         FROM sys.sql_expression_dependencies
         WHERE referencing_id = OBJECT_ID('dbo.vBoundItems')
           AND referenced_entity_name = 'BaseItems';",
    );
    assert_eq!(dep_res.rows, vec![vec![Value::Bit(true)]]);

    // DROP TABLE blocked by schema-bound view
    let err_drop_tbl = run_sql_err(&engine, "DROP TABLE dbo.BaseItems;");
    assert!(
        err_drop_tbl.contains("Cannot DROP TABLE") && err_drop_tbl.contains("vBoundItems"),
        "unexpected error: {err_drop_tbl}"
    );

    // ALTER TABLE DROP/ALTER COLUMN blocked on referenced column `Price`
    let err_drop_col = run_sql_err(&engine, "ALTER TABLE dbo.BaseItems DROP COLUMN Price;");
    assert!(
        err_drop_col.contains("dependent on column 'Price'"),
        "unexpected error: {err_drop_col}"
    );
    let err_alter_col = run_sql_err(
        &engine,
        "ALTER TABLE dbo.BaseItems ALTER COLUMN Price BIGINT NOT NULL;",
    );
    assert!(
        err_alter_col.contains("dependent on column 'Price'"),
        "unexpected error: {err_alter_col}"
    );

    // ALTER TABLE DROP COLUMN succeeds on unreferenced column `Notes`
    run_sql(&engine, "ALTER TABLE dbo.BaseItems DROP COLUMN Notes;");

    // DROP FUNCTION blocked while dbo.vUsesFn depends on it with SCHEMABINDING
    let err_drop_fn = run_sql_err(&engine, "DROP FUNCTION dbo.ufn_BasePrice;");
    assert!(
        err_drop_fn.contains("Cannot DROP FUNCTION") && err_drop_fn.contains("vUsesFn"),
        "unexpected error: {err_drop_fn}"
    );

    // Dropping the schema-bound views unblocks dropping the function and base table
    run_sql(
        &engine,
        "DROP VIEW dbo.vUsesFn;
         DROP FUNCTION dbo.ufn_BasePrice;
         DROP VIEW dbo.vBoundItems;
         DROP TABLE dbo.BaseItems;",
    );
}

#[test]
fn ddl_triggers_log_eventdata_and_rollback_blocked_ddl() {
    let engine = Engine::new();

    // Outside a DDL trigger, EVENTDATA() returns NULL
    let outside = run_sql(&engine, "SELECT EVENTDATA() AS ev;");
    assert_eq!(outside.rows, vec![vec![Value::Null]]);

    run_sql(
        &engine,
        "CREATE TABLE dbo.DdlAudit (
             EventType NVARCHAR(100),
             SchemaName NVARCHAR(128),
             ObjectName NVARCHAR(128),
             ObjectType NVARCHAR(100)
         );",
    );
    run_sql(
        &engine,
        "CREATE TRIGGER tr_AuditTableDdl ON DATABASE
         FOR DDL_TABLE_EVENTS
         AS
         BEGIN
             SET NOCOUNT ON;
             INSERT INTO dbo.DdlAudit (EventType, SchemaName, ObjectName, ObjectType)
             VALUES (
                 EVENTDATA().value('(/EVENT_INSTANCE/EventType)[1]', 'NVARCHAR(100)'),
                 EVENTDATA().value('(/EVENT_INSTANCE/SchemaName)[1]', 'NVARCHAR(128)'),
                 EVENTDATA().value('(/EVENT_INSTANCE/ObjectName)[1]', 'NVARCHAR(128)'),
                 EVENTDATA().value('(/EVENT_INSTANCE/ObjectType)[1]', 'NVARCHAR(100)')
             );
         END;",
    );

    // sys.triggers shows parent_class = 0, DATABASE
    let trig_meta = run_sql(
        &engine,
        "SELECT name, parent_class, parent_class_desc, parent_id, is_disabled
         FROM sys.triggers WHERE name = 'tr_AuditTableDdl';",
    );
    assert_eq!(
        trig_meta.rows,
        vec![vec![
            Value::NVarChar("tr_AuditTableDdl".into()),
            Value::TinyInt(0),
            Value::NVarChar("DATABASE".into()),
            Value::Int(0),
            Value::Bit(false),
        ]]
    );

    // CREATE / ALTER / DROP table log via EVENTDATA()
    run_sql(
        &engine,
        "CREATE TABLE dbo.TrackedTbl (Id INT);
         ALTER TABLE dbo.TrackedTbl ADD Tag NVARCHAR(20);
         DROP TABLE dbo.TrackedTbl;",
    );
    let audit = run_sql(
        &engine,
        "SELECT EventType, SchemaName, ObjectName, ObjectType FROM dbo.DdlAudit;",
    );
    assert_eq!(
        audit.rows,
        vec![
            vec![
                Value::NVarChar("CREATE_TABLE".into()),
                Value::NVarChar("dbo".into()),
                Value::NVarChar("TrackedTbl".into()),
                Value::NVarChar("TABLE".into()),
            ],
            vec![
                Value::NVarChar("ALTER_TABLE".into()),
                Value::NVarChar("dbo".into()),
                Value::NVarChar("TrackedTbl".into()),
                Value::NVarChar("TABLE".into()),
            ],
            vec![
                Value::NVarChar("DROP_TABLE".into()),
                Value::NVarChar("dbo".into()),
                Value::NVarChar("TrackedTbl".into()),
                Value::NVarChar("TABLE".into()),
            ],
        ]
    );

    // Drop-protection trigger using ROLLBACK preserves table AND its data
    run_sql(
        &engine,
        "CREATE TABLE dbo.ProtectedData (Id INT PRIMARY KEY, Name NVARCHAR(30));
         INSERT INTO dbo.ProtectedData VALUES (1, N'safe');",
    );
    run_sql(
        &engine,
        "CREATE TRIGGER tr_PreventDrop ON DATABASE
         FOR DROP_TABLE
         AS
         BEGIN
             ROLLBACK;
         END;",
    );
    let drop_err = run_sql_err(&engine, "DROP TABLE dbo.ProtectedData;");
    assert!(
        drop_err.contains("transaction ended in the trigger"),
        "unexpected error: {drop_err}"
    );
    // Table and row still intact!
    let still_there = run_sql(&engine, "SELECT Id, Name FROM dbo.ProtectedData;");
    assert_eq!(
        still_there.rows,
        vec![vec![Value::Int(1), Value::NVarChar("safe".into())]]
    );

    // DISABLE TRIGGER ON DATABASE allows DROP; DROP TRIGGER ON DATABASE cleans up
    run_sql(
        &engine,
        "DISABLE TRIGGER tr_PreventDrop ON DATABASE;
         DROP TABLE dbo.ProtectedData;
         DROP TRIGGER tr_PreventDrop ON DATABASE;
         DROP TRIGGER tr_AuditTableDdl ON DATABASE;",
    );
}

#[test]
fn applock_mode_test_upgrades_conflicts_and_tx_release() {
    let engine = Engine::new();

    // Before acquiring: NoLock and grantable
    let init = run_sql(
        &engine,
        "SELECT APPLOCK_MODE('public', 'res_alpha', 'Session'),
                APPLOCK_TEST('public', 'res_alpha', 'Exclusive', 'Session');",
    );
    assert_eq!(
        init.rows,
        vec![vec![Value::NVarChar("NoLock".into()), Value::SmallInt(1)]]
    );

    // APPLOCK_TEST with Transaction owner outside a transaction raises SQL Server error 3918
    let err_3918 = run_sql_err(
        &engine,
        "SELECT APPLOCK_TEST('public', 'res_alpha', 'Exclusive', 'Transaction');",
    );
    assert!(
        err_3918.contains("user transaction"),
        "unexpected error: {err_3918}"
    );

    // Session lock upgrades Shared -> SharedIntentExclusive and blocks Transaction owner inside a transaction
    let res = run_sql(
        &engine,
        "DECLARE @rc1 INT, @rc2 INT, @rc3 INT, @rc4 INT;
         EXEC @rc1 = sp_getapplock @Resource = 'res_alpha', @LockMode = 'Shared', @LockOwner = 'Session';
         DECLARE @m1 NVARCHAR(32) = APPLOCK_MODE('public', 'res_alpha', 'Session');
         DECLARE @t_same SMALLINT = APPLOCK_TEST('public', 'res_alpha', 'Exclusive', 'Session');
         DECLARE @t_other_s SMALLINT, @t_other_x SMALLINT;
         BEGIN TRANSACTION;
             SET @t_other_s = APPLOCK_TEST('public', 'res_alpha', 'Shared', 'Transaction');
             SET @t_other_x = APPLOCK_TEST('public', 'res_alpha', 'Exclusive', 'Transaction');
         COMMIT TRANSACTION;
         EXEC @rc2 = sp_getapplock @Resource = 'res_alpha', @LockMode = 'IntentExclusive', @LockOwner = 'Session';
         DECLARE @m2 NVARCHAR(32) = APPLOCK_MODE('public', 'res_alpha', 'Session');
         EXEC @rc3 = sp_releaseapplock @Resource = 'res_alpha', @LockOwner = 'Session';
         DECLARE @m3 NVARCHAR(32) = APPLOCK_MODE('public', 'res_alpha', 'Session');
         EXEC @rc4 = sp_releaseapplock @Resource = 'res_alpha', @LockOwner = 'Session';
         DECLARE @m4 NVARCHAR(32) = APPLOCK_MODE('public', 'res_alpha', 'Session');
         SELECT @rc1, @rc2, @rc3, @rc4, @m1, @t_same, @t_other_s, @t_other_x, @m2, @m3, @m4;",
    );
    assert_eq!(
        res.rows,
        vec![vec![
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::NVarChar("Shared".into()),
            Value::SmallInt(1),
            Value::SmallInt(1),
            Value::SmallInt(1),
            Value::NVarChar("SharedIntentExclusive".into()),
            Value::NVarChar("Shared".into()),
            Value::NVarChar("NoLock".into()),
        ]]
    );

    // Transaction-owned lock outside tx returns -999; inside tx auto-releases on COMMIT
    let tx_res = run_sql(
        &engine,
        "DECLARE @no_tx INT, @in_tx INT;
         EXEC @no_tx = sp_getapplock @Resource = 'res_tx', @LockMode = 'Exclusive', @LockOwner = 'Transaction';
         BEGIN TRANSACTION;
             EXEC @in_tx = sp_getapplock @Resource = 'res_tx', @LockMode = 'Exclusive', @LockOwner = 'Transaction';
             DECLARE @during NVARCHAR(32) = APPLOCK_MODE('public', 'res_tx', 'Transaction');
             DECLARE @same_sess SMALLINT = APPLOCK_TEST('public', 'res_tx', 'Shared', 'Session');
         COMMIT TRANSACTION;
         DECLARE @after NVARCHAR(32) = APPLOCK_MODE('public', 'res_tx', 'Transaction');
         SELECT @no_tx, @in_tx, @during, @same_sess, @after;",
    );
    assert_eq!(
        tx_res.rows,
        vec![vec![
            Value::Int(-999),
            Value::Int(0),
            Value::NVarChar("Exclusive".into()),
            Value::SmallInt(1),
            Value::NVarChar("NoLock".into()),
        ]]
    );

    // Cross-session conflict detection & release across two distinct sessions
    let s1 = engine.create_session();
    let s2 = engine.create_session();
    let run_on = |sid: u64, sql: &str| {
        let stmts = tsql_lite_core::parse_batch(sql).expect("parse batch");
        engine
            .execute_on_session(sid, stmts)
            .expect("exec on session")
            .unwrap_or_default()
    };
    run_on(
        s1,
        "EXEC sp_getapplock @Resource = 'shared_mutex', @LockMode = 'Shared', @LockOwner = 'Session';",
    );
    let s2_check = run_on(
        s2,
        "DECLARE @rc_x INT;
         EXEC @rc_x = sp_getapplock @Resource = 'shared_mutex', @LockMode = 'Exclusive', @LockOwner = 'Session';
         SELECT APPLOCK_TEST('public', 'shared_mutex', 'Shared', 'Session'),
                APPLOCK_TEST('public', 'shared_mutex', 'Exclusive', 'Session'),
                @rc_x;",
    );
    assert_eq!(
        s2_check.rows,
        vec![vec![Value::SmallInt(1), Value::SmallInt(0), Value::Int(-1),]]
    );
    run_on(
        s1,
        "EXEC sp_releaseapplock @Resource = 'shared_mutex', @LockOwner = 'Session';",
    );
    let s2_after = run_on(
        s2,
        "DECLARE @rc_ok INT;
         EXEC @rc_ok = sp_getapplock @Resource = 'shared_mutex', @LockMode = 'Exclusive', @LockOwner = 'Session';
         SELECT APPLOCK_MODE('public', 'shared_mutex', 'Session'), @rc_ok;",
    );
    assert_eq!(
        s2_after.rows,
        vec![vec![Value::NVarChar("Exclusive".into()), Value::Int(0)]]
    );
}

#[test]
fn collate_accent_sensitivity_ai_vs_as_across_ops_like_in_and_group_by() {
    let engine = Engine::new();
    run_sql(
        &engine,
        "CREATE TABLE dbo.Words (Id INT PRIMARY KEY, Word NVARCHAR(40) NOT NULL);
         INSERT INTO dbo.Words VALUES (1, N'cafe'), (2, N'café'), (3, N'CAFÉ'), (4, N'résumé'), (5, N'resume');",
    );

    // = comparisons across CI_AI, CI_AS, CS_AI, CS_AS
    let cmp = run_sql(
        &engine,
        "SELECT
            CASE WHEN N'cafe' COLLATE Latin1_General_CI_AI = N'café' THEN 1 ELSE 0 END,
            CASE WHEN N'cafe' COLLATE Latin1_General_CI_AS = N'café' THEN 1 ELSE 0 END,
            CASE WHEN N'Cafe' COLLATE Latin1_General_CS_AI = N'Café' THEN 1 ELSE 0 END,
            CASE WHEN N'cafe' COLLATE Latin1_General_CS_AI = N'Café' THEN 1 ELSE 0 END,
            CASE WHEN N'Café' COLLATE Latin1_General_CI_AS = N'CAFÉ' THEN 1 ELSE 0 END,
            CASE WHEN N'Café' COLLATE Latin1_General_CS_AS = N'CAFÉ' THEN 1 ELSE 0 END;",
    );
    assert_eq!(
        cmp.rows,
        vec![vec![
            Value::Int(1),
            Value::Int(0),
            Value::Int(1),
            Value::Int(0),
            Value::Int(1),
            Value::Int(0),
        ]]
    );

    // LIKE with CI_AI vs CI_AS
    let like_ai = run_sql(
        &engine,
        "SELECT COUNT(*) FROM dbo.Words WHERE Word COLLATE Latin1_General_CI_AI LIKE N'resume';",
    );
    assert_eq!(like_ai.rows, vec![vec![Value::BigInt(2)]]);

    let like_as = run_sql(
        &engine,
        "SELECT COUNT(*) FROM dbo.Words WHERE Word COLLATE Latin1_General_CI_AS LIKE N'resume';",
    );
    assert_eq!(like_as.rows, vec![vec![Value::BigInt(1)]]);

    // IN with CI_AI vs CI_AS
    let in_ai = run_sql(
        &engine,
        "SELECT COUNT(*) FROM dbo.Words WHERE Word COLLATE Latin1_General_CI_AI IN (N'cafe');",
    );
    assert_eq!(in_ai.rows, vec![vec![Value::BigInt(3)]]);

    let in_as = run_sql(
        &engine,
        "SELECT COUNT(*) FROM dbo.Words WHERE Word COLLATE Latin1_General_CI_AS IN (N'cafe');",
    );
    assert_eq!(in_as.rows, vec![vec![Value::BigInt(1)]]);

    // GROUP BY with CI_AI merges cafe/café/CAFÉ and résumé/resume into 2 groups;
    // GROUP BY with CI_AS produces 4 groups (cafe, café/CAFÉ, résumé, resume).
    let grp_ai = run_sql(
        &engine,
        "SELECT COUNT(*) FROM (
             SELECT Word COLLATE Latin1_General_CI_AI AS W, COUNT(*) AS C
             FROM dbo.Words
             GROUP BY Word COLLATE Latin1_General_CI_AI
         ) g;",
    );
    assert_eq!(grp_ai.rows, vec![vec![Value::BigInt(2)]]);

    let grp_as = run_sql(
        &engine,
        "SELECT COUNT(*) FROM (
             SELECT Word COLLATE Latin1_General_CI_AS AS W, COUNT(*) AS C
             FROM dbo.Words
             GROUP BY Word COLLATE Latin1_General_CI_AS
         ) g;",
    );
    assert_eq!(grp_as.rows, vec![vec![Value::BigInt(4)]]);
}
