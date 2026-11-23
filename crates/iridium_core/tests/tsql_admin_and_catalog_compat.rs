use iridium_core::types::Value;
use iridium_core::Engine;

fn exec(engine: &Engine, sid: u64, sql: &str) -> iridium_core::QueryResult {
    engine
        .execute_session_batch_sql(sid, sql)
        .unwrap_or_else(|e| panic!("SQL failed:\n{sql}\nerror: {e}"))
        .unwrap_or_default()
}

#[test]
fn checkpoint_statement_runs_in_batches_and_stored_procedures() {
    let engine = Engine::new();
    let sid = engine.create_session();

    exec(
        &engine,
        sid,
        "CREATE TABLE dbo.WorkItems (Id INT PRIMARY KEY, Val INT);",
    );
    exec(
        &engine,
        sid,
        "INSERT INTO dbo.WorkItems VALUES (1, 10), (2, 20);",
    );

    exec(&engine, sid, "CHECKPOINT;");
    exec(&engine, sid, "CHECKPOINT 30;");
    exec(&engine, sid, "DECLARE @dur INT = 15; CHECKPOINT @dur;");

    exec(
        &engine,
        sid,
        "CREATE PROCEDURE dbo.usp_PurgeWorkItems @CutId INT AS
         BEGIN
             SET NOCOUNT ON;
             DELETE FROM dbo.WorkItems WHERE Id <= @CutId;
             CHECKPOINT;
             SELECT COUNT(*) AS Remaining FROM dbo.WorkItems;
         END;",
    );

    let res = exec(&engine, sid, "EXEC dbo.usp_PurgeWorkItems @CutId = 1;");
    assert_eq!(res.rows, vec![vec![Value::BigInt(1)]]);
}

#[test]
fn xp_msver_returns_rows() {
    let engine = Engine::new();
    let sid = engine.create_session();

    let all = exec(&engine, sid, "EXEC xp_msver;");
    assert_eq!(all.columns.len(), 4);
    assert!(all.rows.len() >= 4);

    let filtered = exec(
        &engine,
        sid,
        "EXEC master.dbo.xp_msver N'Platform', N'ProductVersion';",
    );
    assert!(!filtered.rows.is_empty());
}

#[test]
fn context_info_defaults_to_null_and_clears_on_empty_binary() {
    let engine = Engine::new();
    let sid = engine.create_session();

    let initial = exec(
        &engine,
        sid,
        "SELECT CONVERT(NVARCHAR(36), CONTEXT_INFO()) AS Ctx;",
    );
    assert_eq!(initial.rows, vec![vec![Value::Null]]);

    exec(&engine, sid, "SET CONTEXT_INFO 0xCAFE;");
    let populated = exec(&engine, sid, "SELECT CONTEXT_INFO();");
    match &populated.rows[0][0] {
        Value::VarBinary(b) => {
            assert_eq!(b.len(), 128);
            assert_eq!(&b[0..2], &[0xCA, 0xFE]);
        }
        other => panic!("expected VarBinary, got {other:?}"),
    }

    exec(&engine, sid, "SET CONTEXT_INFO 0x;");
    let cleared = exec(&engine, sid, "SELECT CONTEXT_INFO();");
    assert_eq!(cleared.rows, vec![vec![Value::Null]]);
}

#[test]
fn national_string_literal_column_aliases_parse_and_project() {
    let engine = Engine::new();
    let sid = engine.create_session();

    let res = exec(
        &engine,
        sid,
        "SELECT CAST(1 AS BIT) AS N'HasAccess', CAST(0 AS BIT) N'IsBlocked';",
    );
    assert_eq!(res.columns, vec!["HasAccess", "IsBlocked"]);
    assert_eq!(res.rows, vec![vec![Value::Bit(true), Value::Bit(false)]]);
}

#[test]
fn database_lifecycle_ddl_and_catalog_metadata_views_work() {
    let engine = Engine::new();
    let sid = engine.create_session();

    exec(
        &engine,
        sid,
        "CREATE DATABASE [SandboxAppDb] COLLATE SQL_Latin1_General_CP1_CI_AS;",
    );
    exec(
        &engine,
        sid,
        "ALTER DATABASE [SandboxAppDb]
            SET ANSI_NULLS ON,
                QUOTED_IDENTIFIER ON,
                RECOVERY FULL
            WITH ROLLBACK IMMEDIATE;",
    );
    exec(
        &engine,
        sid,
        "ALTER DATABASE SCOPED CONFIGURATION SET MAXDOP = 0;",
    );
    exec(&engine, sid, "DROP DATABASE IF EXISTS [SandboxAppDb];");

    let cfg = exec(
        &engine,
        sid,
        "SELECT value FROM [master].[dbo].[sysconfigures] WHERE config = 1126;",
    );
    assert_eq!(cfg.rows, vec![vec![Value::Int(1033)]]);

    let dbs = exec(
        &engine,
        sid,
        "SELECT name FROM [master].[dbo].[sysdatabases] WHERE name = N'master';",
    );
    assert_eq!(dbs.rows.len(), 1);

    let scoped = exec(
        &engine,
        sid,
        "SELECT value FROM sys.database_scoped_configurations WHERE name = 'MAXDOP';",
    );
    assert_eq!(scoped.rows, vec![vec![Value::NVarChar("0".to_string())]]);

    let syscols = exec(&engine, sid, "SELECT COUNT(*) FROM sys.system_columns;");
    assert_eq!(syscols.rows.len(), 1);

    let srv_perms = exec(
        &engine,
        sid,
        "SELECT COUNT(*) FROM sys.server_permissions WHERE state = N'D';",
    );
    assert_eq!(srv_perms.rows, vec![vec![Value::BigInt(0)]]);

    let builtin_perms = exec(
        &engine,
        sid,
        "SELECT permission_name FROM sys.fn_builtin_permissions('');",
    );
    assert!(!builtin_perms.rows.is_empty());

    exec(
        &engine,
        sid,
        "CREATE TABLE dbo.ParentItems (Id INT PRIMARY KEY);
         CREATE TABLE dbo.ChildItems (
             Id INT PRIMARY KEY,
             ParentId INT NOT NULL,
             Qty INT NOT NULL,
             CONSTRAINT CK_Child_Qty CHECK (Qty >= 0),
             CONSTRAINT FK_Child_Parent FOREIGN KEY (ParentId) REFERENCES dbo.ParentItems(Id)
         );",
    );

    let check_rows = exec(
        &engine,
        sid,
        "SELECT schema_id, name, is_disabled, is_not_trusted, is_system_named, is_ms_shipped, create_date
         FROM sys.check_constraints ck
         LEFT JOIN sys.filetable_system_defined_objects ft
           ON ck.parent_object_id = ft.parent_object_id AND ck.object_id = ft.object_id
         WHERE ft.object_id IS NULL;",
    );
    assert_eq!(check_rows.rows.len(), 1);

    let fk_rows = exec(
        &engine,
        sid,
        "SELECT schema_id, name, is_disabled, is_not_trusted, is_system_named, is_ms_shipped, create_date
         FROM sys.foreign_keys fk
         LEFT JOIN sys.filetable_system_defined_objects ft
           ON fk.parent_object_id = ft.parent_object_id AND fk.object_id = ft.object_id
         WHERE ft.object_id IS NULL;",
    );
    assert_eq!(fk_rows.rows.len(), 1);

    let edge = exec(
        &engine,
        sid,
        "SELECT COUNT(*) FROM sys.edge_constraint_clauses;",
    );
    assert_eq!(edge.rows, vec![vec![Value::BigInt(0)]]);

    let msgs = exec(
        &engine,
        sid,
        "SELECT m.message_id, l.alias
         FROM sys.messages m
         LEFT JOIN sys.syslanguages l ON l.msglangid = m.language_id;",
    );
    assert!(msgs.rows.is_empty());
}
