use iridium_core::types::Value;
use iridium_core::{parse_batch, Engine, QueryResult};

fn exec(engine: &Engine, sql: &str) {
    let stmts = parse_batch(sql).unwrap_or_else(|e| panic!("parse failed for `{sql}`: {e}"));
    engine
        .execute_batch(stmts)
        .unwrap_or_else(|e| panic!("exec failed for `{sql}`: {e}"));
}

fn query(engine: &Engine, sql: &str) -> QueryResult {
    let stmts = parse_batch(sql).unwrap_or_else(|e| panic!("parse failed for `{sql}`: {e}"));
    engine
        .execute_batch(stmts)
        .unwrap_or_else(|e| panic!("query failed for `{sql}`: {e}"))
        .expect("expected result set")
}

#[test]
fn sys_columns_and_info_schema_expose_per_column_collation_and_null_for_non_char() {
    let e = Engine::new();
    exec(
        &e,
        "CREATE TABLE dbo.CollationSample (
             Id INT IDENTITY(1,1) NOT NULL,
             DefaultStr NVARCHAR(40) NULL,
             BinStr VARCHAR(40) COLLATE Latin1_General_BIN NOT NULL,
             CsStr NVARCHAR(40) COLLATE Latin1_General_CS_AS NULL,
             Flag BIT NOT NULL
         );",
    );

    let res = query(
        &e,
        "SELECT name, collation_name, is_identity
         FROM sys.columns
         WHERE object_id = OBJECT_ID('dbo.CollationSample')
         ORDER BY column_id;",
    );
    assert_eq!(res.rows.len(), 5);
    assert_eq!(res.rows[0][0].to_string_value(), "Id");
    assert_eq!(res.rows[0][1], Value::Null);
    assert_eq!(res.rows[0][2], Value::Bit(true));

    assert_eq!(res.rows[1][0].to_string_value(), "DefaultStr");
    assert_eq!(
        res.rows[1][1].to_string_value(),
        "SQL_Latin1_General_CP1_CI_AS"
    );
    assert_eq!(res.rows[1][2], Value::Bit(false));

    assert_eq!(res.rows[2][0].to_string_value(), "BinStr");
    assert_eq!(res.rows[2][1].to_string_value(), "Latin1_General_BIN");

    assert_eq!(res.rows[3][0].to_string_value(), "CsStr");
    assert_eq!(res.rows[3][1].to_string_value(), "Latin1_General_CS_AS");

    assert_eq!(res.rows[4][0].to_string_value(), "Flag");
    assert_eq!(res.rows[4][1], Value::Null);

    let info = query(
        &e,
        "SELECT COLUMN_NAME, COLLATION_NAME
         FROM INFORMATION_SCHEMA.COLUMNS
         WHERE TABLE_SCHEMA = 'dbo' AND TABLE_NAME = 'CollationSample'
         ORDER BY ORDINAL_POSITION;",
    );
    assert_eq!(info.rows.len(), 5);
    assert_eq!(info.rows[0][1], Value::Null);
    assert_eq!(
        info.rows[1][1].to_string_value(),
        "SQL_Latin1_General_CP1_CI_AS"
    );
    assert_eq!(info.rows[2][1].to_string_value(), "Latin1_General_BIN");
    assert_eq!(info.rows[3][1].to_string_value(), "Latin1_General_CS_AS");
    assert_eq!(info.rows[4][1], Value::Null);

    // Temp tables also expose collation_name and is_identity via sys.columns.
    exec(
        &e,
        "CREATE TABLE #TempCol (
             KeyId INT IDENTITY(10,1) NOT NULL,
             Code VARCHAR(20) COLLATE Latin1_General_BIN NOT NULL
         );",
    );
    let tmp = query(
        &e,
        "SELECT name, collation_name, is_identity
         FROM sys.columns
         WHERE object_id = OBJECT_ID('tempdb..#TempCol')
         ORDER BY column_id;",
    );
    assert_eq!(tmp.rows.len(), 2);
    assert_eq!(tmp.rows[0][1], Value::Null);
    assert_eq!(tmp.rows[0][2], Value::Bit(true));
    assert_eq!(tmp.rows[1][1].to_string_value(), "Latin1_General_BIN");
}

#[test]
fn alter_schema_transfer_moves_tables_views_procedures_and_functions() {
    let e = Engine::new();
    exec(
        &e,
        "CREATE SCHEMA SrcSchema;
         CREATE SCHEMA DstSchema;
         CREATE TABLE SrcSchema.Items (Id INT NOT NULL, Name NVARCHAR(50) NOT NULL);
         INSERT INTO SrcSchema.Items (Id, Name) VALUES (1, N'Alpha'), (2, N'Beta');",
    );
    exec(
        &e,
        "CREATE VIEW SrcSchema.ItemView AS SELECT Id, Name FROM DstSchema.Items;",
    );
    exec(
        &e,
        "CREATE PROCEDURE SrcSchema.GetItemCount AS BEGIN SELECT COUNT(*) AS Cnt FROM DstSchema.Items; END;",
    );
    exec(
        &e,
        "CREATE FUNCTION SrcSchema.DoubleVal(@x INT) RETURNS INT AS BEGIN RETURN @x * 2; END;",
    );

    exec(
        &e,
        "ALTER SCHEMA DstSchema TRANSFER SrcSchema.Items;
         ALTER SCHEMA DstSchema TRANSFER OBJECT::SrcSchema.ItemView;
         ALTER SCHEMA DstSchema TRANSFER SrcSchema.GetItemCount;
         ALTER SCHEMA DstSchema TRANSFER SrcSchema.DoubleVal;",
    );

    // Old schema no longer holds the objects.
    let old_check = query(
        &e,
        "SELECT OBJECT_ID('SrcSchema.Items'), OBJECT_ID('SrcSchema.ItemView'),
                OBJECT_ID('SrcSchema.GetItemCount'), OBJECT_ID('SrcSchema.DoubleVal');",
    );
    assert_eq!(old_check.rows[0][0], Value::Null);
    assert_eq!(old_check.rows[0][1], Value::Null);
    assert_eq!(old_check.rows[0][2], Value::Null);
    assert_eq!(old_check.rows[0][3], Value::Null);

    // Table data survived transfer intact.
    let rows = query(&e, "SELECT Id, Name FROM DstSchema.Items ORDER BY Id;");
    assert_eq!(rows.rows.len(), 2);
    assert_eq!(rows.rows[0][1].to_string_value(), "Alpha");
    assert_eq!(rows.rows[1][1].to_string_value(), "Beta");

    // Transferred view, procedure, and function execute under the new schema.
    let view_rows = query(&e, "SELECT COUNT(*) FROM DstSchema.ItemView;");
    assert_eq!(view_rows.rows[0][0].to_integer_i64(), Some(2));

    let proc_rows = query(&e, "EXEC DstSchema.GetItemCount;");
    assert_eq!(proc_rows.rows[0][0].to_integer_i64(), Some(2));

    let fn_rows = query(&e, "SELECT DstSchema.DoubleVal(21);");
    assert_eq!(fn_rows.rows[0][0].to_integer_i64(), Some(42));

    // Error cases: missing target schema and duplicate destination name.
    let missing_schema_err = e
        .execute_batch(parse_batch("ALTER SCHEMA NoSuchSchema TRANSFER DstSchema.Items;").unwrap());
    assert!(
        missing_schema_err.is_err(),
        "transfer to nonexistent schema must fail"
    );

    exec(&e, "CREATE TABLE SrcSchema.Items (Id INT NOT NULL);");
    let dup_err =
        e.execute_batch(parse_batch("ALTER SCHEMA DstSchema TRANSFER SrcSchema.Items;").unwrap());
    assert!(
        dup_err.is_err(),
        "transfer onto existing object name in target schema must fail"
    );
}

#[test]
fn execute_on_session_runs_independently_of_default_session_transaction() {
    let e = Engine::new();
    exec(
        &e,
        "CREATE TABLE dbo.TxWork (Id INT NOT NULL);
         CREATE TABLE dbo.SessionLog (Id INT NOT NULL, Tag NVARCHAR(40) NOT NULL);",
    );

    exec(
        &e,
        "BEGIN TRANSACTION;
         INSERT INTO dbo.TxWork (Id) VALUES (1);",
    );

    let other_session = e.create_session();
    let other_stmts =
        parse_batch("INSERT INTO dbo.SessionLog (Id, Tag) VALUES (2, N'other_session');").unwrap();
    e.execute_on_session(other_session, other_stmts)
        .expect("second session insert on unlocked table should succeed");
    e.close_session(other_session)
        .expect("closing second session should succeed");

    exec(&e, "ROLLBACK TRANSACTION;");

    let rolled_back = query(&e, "SELECT COUNT(*) FROM dbo.TxWork;");
    assert_eq!(rolled_back.rows[0][0].to_integer_i64(), Some(0));

    let survived = query(&e, "SELECT Id, Tag FROM dbo.SessionLog ORDER BY Id;");
    assert_eq!(survived.rows.len(), 1);
    assert_eq!(survived.rows[0][0].to_integer_i64(), Some(2));
    assert_eq!(survived.rows[0][1].to_string_value(), "other_session");
}

#[test]
fn lazy_definition_sql_serves_sys_sql_modules_sp_helptext_and_object_definition() {
    let e = Engine::new();
    exec(&e, "CREATE TABLE dbo.DefItems (Id INT NOT NULL);");
    exec(
        &e,
        "CREATE VIEW dbo.vDefItems AS SELECT Id FROM dbo.DefItems;",
    );
    exec(
        &e,
        "CREATE PROCEDURE dbo.usp_DefEcho AS BEGIN SELECT 1; END;",
    );
    exec(
        &e,
        "CREATE FUNCTION dbo.ufn_DefInc(@x INT) RETURNS INT AS BEGIN RETURN @x + 1; END;",
    );
    exec(
        &e,
        "CREATE TRIGGER dbo.trg_DefAudit ON dbo.DefItems AFTER INSERT AS BEGIN SELECT 1; END;",
    );

    let mods = query(
        &e,
        "SELECT o.name, m.definition
         FROM sys.sql_modules m
         JOIN sys.objects o ON o.object_id = m.object_id
         ORDER BY o.name;",
    );
    assert_eq!(mods.rows.len(), 4);
    for row in &mods.rows {
        let def = row[1].to_string_value();
        assert!(
            !def.is_empty(),
            "sys.sql_modules.definition must be populated lazily for {}",
            row[0].to_string_value()
        );
    }

    let obj_def = query(
        &e,
        "SELECT OBJECT_DEFINITION(OBJECT_ID('dbo.usp_DefEcho')),
                OBJECT_DEFINITION(OBJECT_ID('dbo.vDefItems')),
                OBJECT_DEFINITION(OBJECT_ID('dbo.ufn_DefInc')),
                OBJECT_DEFINITION(OBJECT_ID('dbo.trg_DefAudit'));",
    );
    assert!(obj_def.rows[0][0]
        .to_string_value()
        .contains("CREATE PROCEDURE"));
    assert!(obj_def.rows[0][1].to_string_value().contains("CREATE VIEW"));
    assert!(obj_def.rows[0][2]
        .to_string_value()
        .contains("CREATE FUNCTION"));
    assert!(obj_def.rows[0][3]
        .to_string_value()
        .contains("CREATE TRIGGER"));

    let ht_view = query(&e, "EXEC sp_helptext 'vDefItems';");
    assert!(ht_view.rows[0][0].to_string_value().contains("CREATE VIEW"));
    let ht_proc = query(&e, "EXEC sp_helptext 'usp_DefEcho';");
    assert!(ht_proc.rows[0][0]
        .to_string_value()
        .contains("CREATE PROCEDURE"));
}
