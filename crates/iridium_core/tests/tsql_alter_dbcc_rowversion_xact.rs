use iridium_core::executor::result::QueryResult;
use iridium_core::types::Value;
use iridium_core::{parse_batch, DbError, Engine};

fn run(engine: &Engine, sql: &str) -> Result<Option<QueryResult>, DbError> {
    let stmts = parse_batch(sql)?;
    engine.execute_batch(stmts)
}

fn query(engine: &Engine, sql: &str) -> QueryResult {
    run(engine, sql)
        .expect("query execution failed")
        .expect("expected result set")
}

#[test]
fn alter_and_create_or_alter_view_function_trigger_and_sequence() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Items (Id INT PRIMARY KEY, Val INT);
         CREATE TABLE dbo.Audit (Tag VARCHAR(20));
         INSERT INTO dbo.Items VALUES (1, 10), (2, 20);",
    )
    .expect("seed");

    // CREATE OR ALTER VIEW + ALTER VIEW
    run(
        &engine,
        "CREATE OR ALTER VIEW dbo.vItems AS SELECT Id, Val FROM dbo.Items WHERE Val >= 15;",
    )
    .expect("create or alter view");
    assert_eq!(
        query(&engine, "SELECT COUNT(*) FROM dbo.vItems;").rows[0][0],
        Value::BigInt(1)
    );

    run(
        &engine,
        "ALTER VIEW dbo.vItems AS SELECT Id, Val FROM dbo.Items;",
    )
    .expect("alter view");
    assert_eq!(
        query(&engine, "SELECT COUNT(*) FROM dbo.vItems;").rows[0][0],
        Value::BigInt(2)
    );

    // CREATE OR ALTER FUNCTION + ALTER FUNCTION
    run(
        &engine,
        "CREATE OR ALTER FUNCTION dbo.fnScale(@x INT) RETURNS INT AS BEGIN RETURN @x * 2; END;",
    )
    .expect("create or alter function");
    assert_eq!(
        query(&engine, "SELECT dbo.fnScale(5);").rows[0][0].to_integer_i64(),
        Some(10)
    );

    run(
        &engine,
        "ALTER FUNCTION dbo.fnScale(@x INT) RETURNS INT AS BEGIN RETURN @x * 3; END;",
    )
    .expect("alter function");
    assert_eq!(
        query(&engine, "SELECT dbo.fnScale(5);").rows[0][0].to_integer_i64(),
        Some(15)
    );

    // CREATE OR ALTER TRIGGER + ALTER TRIGGER
    run(
        &engine,
        "CREATE OR ALTER TRIGGER dbo.trItems ON dbo.Items AFTER INSERT AS
         BEGIN INSERT INTO dbo.Audit VALUES ('v1'); END;",
    )
    .expect("create or alter trigger");
    run(&engine, "INSERT INTO dbo.Items VALUES (3, 30);").expect("insert 3");
    assert_eq!(
        query(&engine, "SELECT Tag FROM dbo.Audit;").rows,
        vec![vec![Value::VarChar("v1".to_string())]]
    );

    run(
        &engine,
        "ALTER TRIGGER dbo.trItems ON dbo.Items AFTER INSERT AS
         BEGIN INSERT INTO dbo.Audit VALUES ('v2'); END;",
    )
    .expect("alter trigger");
    run(&engine, "INSERT INTO dbo.Items VALUES (4, 40);").expect("insert 4");
    assert_eq!(
        query(&engine, "SELECT Tag FROM dbo.Audit ORDER BY Tag;").rows,
        vec![
            vec![Value::VarChar("v1".to_string())],
            vec![Value::VarChar("v2".to_string())],
        ]
    );

    // ALTER SEQUENCE (RESTART WITH, INCREMENT BY, RESTART)
    run(
        &engine,
        "CREATE SEQUENCE dbo.SeqNum AS INT START WITH 10 INCREMENT BY 5;",
    )
    .expect("create sequence");
    assert_eq!(
        query(&engine, "SELECT NEXT VALUE FOR dbo.SeqNum;").rows[0][0],
        Value::BigInt(10)
    );
    assert_eq!(
        query(&engine, "SELECT NEXT VALUE FOR dbo.SeqNum;").rows[0][0],
        Value::BigInt(15)
    );
    run(
        &engine,
        "ALTER SEQUENCE dbo.SeqNum RESTART WITH 100 INCREMENT BY 10;",
    )
    .expect("alter sequence restart with");
    assert_eq!(
        query(&engine, "SELECT NEXT VALUE FOR dbo.SeqNum;").rows[0][0],
        Value::BigInt(100)
    );
    assert_eq!(
        query(&engine, "SELECT NEXT VALUE FOR dbo.SeqNum;").rows[0][0],
        Value::BigInt(110)
    );
    run(&engine, "ALTER SEQUENCE dbo.SeqNum RESTART;").expect("alter sequence bare restart");
    assert_eq!(
        query(&engine, "SELECT NEXT VALUE FOR dbo.SeqNum;").rows[0][0],
        Value::BigInt(10)
    );
}

#[test]
fn dbcc_checkident_reseeds_and_validates_identity_columns() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Logs (Id INT IDENTITY(1, 1) PRIMARY KEY, Msg VARCHAR(20));
         INSERT INTO dbo.Logs (Msg) VALUES ('a'), ('b');",
    )
    .expect("seed logs");

    // NORESEED leaves counter intact
    run(&engine, "DBCC CHECKIDENT ('dbo.Logs', NORESEED);").expect("noreseed");
    run(&engine, "INSERT INTO dbo.Logs (Msg) VALUES ('c');").expect("insert c");
    assert_eq!(
        query(&engine, "SELECT MAX(Id) FROM dbo.Logs;").rows[0][0],
        Value::Int(3)
    );

    // RESEED to 100 -> IDENT_CURRENT=100, next insert gets 101
    run(
        &engine,
        "DBCC CHECKIDENT ('dbo.Logs', RESEED, 100) WITH NO_INFOMSGS;",
    )
    .expect("reseed 100");
    assert_eq!(
        query(&engine, "SELECT IDENT_CURRENT('dbo.Logs');").rows[0][0],
        Value::BigInt(100)
    );
    run(&engine, "INSERT INTO dbo.Logs (Msg) VALUES ('d');").expect("insert d");
    assert_eq!(
        query(&engine, "SELECT MAX(Id) FROM dbo.Logs;").rows[0][0],
        Value::Int(101)
    );

    // Table without identity column fails DBCC CHECKIDENT
    run(&engine, "CREATE TABLE dbo.NoIdent (Id INT PRIMARY KEY);").expect("no ident");
    assert!(run(&engine, "DBCC CHECKIDENT ('dbo.NoIdent', NORESEED);").is_err());
}

#[test]
fn rowversion_and_timestamp_auto_populate_and_bump_on_update() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Docs (
             Id INT PRIMARY KEY,
             Title VARCHAR(30),
             Ver ROWVERSION
         );
         INSERT INTO dbo.Docs (Id, Title) VALUES (1, 'first'), (2, 'second');",
    )
    .expect("create rowversion table and insert");

    // Explicit insert into rowversion column must fail
    assert!(run(
        &engine,
        "INSERT INTO dbo.Docs (Id, Title, Ver) VALUES (3, 'bad', 0x0000000000000001);",
    )
    .is_err());

    // Explicit update to rowversion column must fail
    assert!(run(
        &engine,
        "UPDATE dbo.Docs SET Ver = 0x0000000000000001 WHERE Id = 1;",
    )
    .is_err());

    let before = query(&engine, "SELECT Id, Ver FROM dbo.Docs ORDER BY Id;");
    let v1 = match &before.rows[0][1] {
        Value::Binary(b) => {
            assert_eq!(b.len(), 8);
            u64::from_be_bytes(b.as_slice().try_into().unwrap())
        }
        other => panic!("expected 8-byte binary rowversion, got {other:?}"),
    };
    let v2 = match &before.rows[1][1] {
        Value::Binary(b) => u64::from_be_bytes(b.as_slice().try_into().unwrap()),
        other => panic!("expected 8-byte binary rowversion, got {other:?}"),
    };
    assert!(
        v2 > v1,
        "rowversion must increase across rows ({v2} > {v1})"
    );

    // UPDATE automatically bumps rowversion on modified row only
    run(
        &engine,
        "UPDATE dbo.Docs SET Title = 'updated' WHERE Id = 1;",
    )
    .expect("update doc 1");
    let after = query(&engine, "SELECT Id, Ver FROM dbo.Docs ORDER BY Id;");
    let v1_after = match &after.rows[0][1] {
        Value::Binary(b) => u64::from_be_bytes(b.as_slice().try_into().unwrap()),
        other => panic!("expected binary, got {other:?}"),
    };
    let v2_after = match &after.rows[1][1] {
        Value::Binary(b) => u64::from_be_bytes(b.as_slice().try_into().unwrap()),
        other => panic!("expected binary, got {other:?}"),
    };
    assert!(v1_after > v2, "updated rowversion must be > previous max");
    assert_eq!(v2_after, v2, "untouched row keeps its rowversion");

    // sys.columns reports system_type_id = 189 (timestamp)
    let col_meta = query(
        &engine,
        "SELECT system_type_id, is_nullable FROM sys.columns WHERE name = 'Ver';",
    );
    assert_eq!(col_meta.rows[0][0], Value::TinyInt(189));
    assert_eq!(col_meta.rows[0][1], Value::Bit(false));
}

#[test]
fn xact_abort_on_dooms_transaction_in_try_catch_and_rolls_back_on_uncaught_error() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Ledger (Id INT PRIMARY KEY, Amt INT);
         INSERT INTO dbo.Ledger VALUES (1, 100);",
    )
    .expect("seed ledger");

    // Procedure with SET XACT_ABORT ON + TRY/CATCH: error dooms transaction (XACT_STATE() = -1),
    // COMMIT fails, savepoint rollback fails, full ROLLBACK restores pre-tx state.
    run(
        &engine,
        "CREATE PROCEDURE dbo.DoomedTxTest AS
         BEGIN
             SET XACT_ABORT ON;
             DECLARE @state_in_catch INT = 0;
             DECLARE @commit_failed BIT = 0;
             DECLARE @state_after_rollback INT = 99;
             BEGIN TRY
                 BEGIN TRANSACTION;
                 INSERT INTO dbo.Ledger VALUES (2, 200);
                 SAVE TRANSACTION sp1;
                 -- Duplicate PK triggers runtime error under XACT_ABORT ON
                 INSERT INTO dbo.Ledger VALUES (1, 999);
                 COMMIT TRANSACTION;
             END TRY
             BEGIN CATCH
                 SET @state_in_catch = XACT_STATE();
                 BEGIN TRY
                     COMMIT TRANSACTION;
                 END TRY
                 BEGIN CATCH
                     SET @commit_failed = 1;
                 END CATCH
                 IF XACT_STATE() = -1
                     ROLLBACK TRANSACTION;
                 SET @state_after_rollback = XACT_STATE();
             END CATCH
             SELECT @state_in_catch AS st_catch,
                    @commit_failed AS commit_err,
                    @state_after_rollback AS st_after;
         END;",
    )
    .expect("create DoomedTxTest");

    let r = query(&engine, "EXEC dbo.DoomedTxTest;");
    assert_eq!(r.rows[0][0], Value::Int(-1));
    assert_eq!(r.rows[0][1], Value::Bit(true));
    assert_eq!(r.rows[0][2], Value::Int(0));

    // Row 2 was rolled back!
    assert_eq!(
        query(&engine, "SELECT COUNT(*) FROM dbo.Ledger;").rows[0][0],
        Value::BigInt(1)
    );

    // Uncaught error inside procedure with SET XACT_ABORT ON auto-rolls back the transaction
    run(
        &engine,
        "CREATE PROCEDURE dbo.UncaughtAbortTest AS
         BEGIN
             SET XACT_ABORT ON;
             BEGIN TRANSACTION;
             INSERT INTO dbo.Ledger VALUES (3, 300);
             INSERT INTO dbo.Ledger VALUES (1, 999);
             COMMIT TRANSACTION;
         END;",
    )
    .expect("create UncaughtAbortTest");
    assert!(run(&engine, "EXEC dbo.UncaughtAbortTest;").is_err());
    assert_eq!(
        query(&engine, "SELECT COUNT(*) FROM dbo.Ledger;").rows[0][0],
        Value::BigInt(1)
    );
}
