use std::collections::HashSet;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

use tsql_lite_core::types::Value;
use tsql_lite_core::{Engine, SessionId};

fn exec_ok(engine: &Engine, sid: SessionId, sql: &str) {
    engine
        .execute_session_batch_sql(sid, sql)
        .unwrap_or_else(|e| panic!("SQL failed ({sql}): {e}"));
}

fn query_rows(engine: &Engine, sid: SessionId, sql: &str) -> Vec<Vec<Value>> {
    engine
        .execute_session_batch_sql(sid, sql)
        .unwrap_or_else(|e| panic!("query failed ({sql}): {e}"))
        .map(|qr| qr.rows)
        .unwrap_or_default()
}

fn as_i64(v: &Value) -> i64 {
    match v {
        Value::Int(n) => *n as i64,
        Value::BigInt(n) => *n,
        Value::SmallInt(n) => *n as i64,
        Value::TinyInt(n) => *n as i64,
        other => panic!("expected integer, got {other:?}"),
    }
}

#[test]
fn concurrent_worker_pool_updlock_rowlock_readpast_zero_duplicates() {
    let engine = Arc::new(Engine::new());
    let setup = engine.create_session();
    exec_ok(
        &engine,
        setup,
        "CREATE TABLE dbo.OutboxJobs (
            JobId INT PRIMARY KEY,
            Status VARCHAR(20) NOT NULL,
            WorkerId INT NULL
        );",
    );
    for id in 1..=16 {
        exec_ok(
            &engine,
            setup,
            &format!(
                "INSERT INTO dbo.OutboxJobs (JobId, Status, WorkerId) VALUES ({id}, 'Pending', NULL);"
            ),
        );
    }
    engine.close_session(setup).unwrap();

    const WORKERS: usize = 8;
    let barrier = Arc::new(Barrier::new(WORKERS));
    let claimed = Arc::new(Mutex::new(Vec::<(usize, i64)>::new()));

    let mut handles = Vec::with_capacity(WORKERS);
    for worker_id in 0..WORKERS {
        let eng = Arc::clone(&engine);
        let bar = Arc::clone(&barrier);
        let claimed_ref = Arc::clone(&claimed);
        handles.push(thread::spawn(move || {
            let sid = eng.create_session();
            bar.wait();
            loop {
                let rows = query_rows(
                    &eng,
                    sid,
                    "BEGIN TRAN;
                     SELECT TOP (1) JobId
                     FROM dbo.OutboxJobs WITH (UPDLOCK, ROWLOCK, READPAST)
                     WHERE Status = 'Pending'
                     ORDER BY JobId;",
                );
                if rows.is_empty() {
                    exec_ok(&eng, sid, "COMMIT TRAN;");
                    break;
                }
                let job_id = as_i64(&rows[0][0]);
                claimed_ref.lock().unwrap().push((worker_id, job_id));
                exec_ok(
                    &eng,
                    sid,
                    &format!(
                        "UPDATE dbo.OutboxJobs SET Status = 'Done', WorkerId = {worker_id} WHERE JobId = {job_id};
                         COMMIT TRAN;"
                    ),
                );
            }
            eng.close_session(sid).unwrap();
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let claimed_list = claimed.lock().unwrap();
    assert_eq!(
        claimed_list.len(),
        16,
        "every job must be claimed exactly once, got {:?}",
        *claimed_list
    );
    let distinct: HashSet<i64> = claimed_list.iter().map(|(_, id)| *id).collect();
    assert_eq!(distinct.len(), 16, "duplicate job claim detected!");

    let verify = engine.create_session();
    let pending = query_rows(
        &engine,
        verify,
        "SELECT COUNT(*) FROM dbo.OutboxJobs WHERE Status = 'Done' AND WorkerId IS NOT NULL;",
    );
    assert_eq!(as_i64(&pending[0][0]), 16);
    engine.close_session(verify).unwrap();
}

#[test]
fn single_user_with_rollback_immediate_evicts_sibling_sessions_and_drops_db() {
    let engine = Engine::new();
    let s1 = engine.create_session();
    let s2 = engine.create_session();

    exec_ok(&engine, s1, "CREATE DATABASE [PoolTeardownDb];");
    exec_ok(
        &engine,
        s1,
        "USE [PoolTeardownDb]; CREATE TABLE dbo.T (Id INT PRIMARY KEY); INSERT INTO dbo.T VALUES (1);",
    );
    // Simulate a pooled connection (s2) parked in PoolTeardownDb with an open uncommitted tx
    exec_ok(
        &engine,
        s2,
        "USE [PoolTeardownDb]; BEGIN TRAN; INSERT INTO dbo.T VALUES (2);",
    );

    // Switching s1 to master and attempting raw DROP DATABASE fails with SQL Server error 3702
    exec_ok(&engine, s1, "USE master;");
    let drop_err = engine
        .execute_session_batch_sql(s1, "DROP DATABASE [PoolTeardownDb];")
        .expect_err("DROP DATABASE must fail while s2 is still attached");
    assert_eq!(drop_err.number(), 3702, "expected 3702, got: {drop_err}");

    // Standard E2E teardown pattern: SINGLE_USER WITH ROLLBACK IMMEDIATE evicts s2 and rolls back its tx
    exec_ok(
        &engine,
        s1,
        "ALTER DATABASE [PoolTeardownDb] SET SINGLE_USER WITH ROLLBACK IMMEDIATE;",
    );
    let ua = query_rows(
        &engine,
        s1,
        "SELECT user_access, user_access_desc FROM sys.databases WHERE name = 'PoolTeardownDb';",
    );
    assert_eq!(
        ua,
        vec![vec![
            Value::TinyInt(1),
            Value::VarChar("SINGLE_USER".into())
        ]]
    );

    // s2 was evicted back to master and its uncommitted tx was rolled back
    let s2_state = query_rows(&engine, s2, "SELECT DB_NAME(), @@TRANCOUNT;");
    assert_eq!(
        s2_state,
        vec![vec![Value::NVarChar("master".into()), Value::Int(0)]]
    );

    // Now DROP DATABASE succeeds cleanly
    exec_ok(&engine, s1, "DROP DATABASE [PoolTeardownDb];");
    let gone = query_rows(&engine, s1, "SELECT DB_ID('PoolTeardownDb');");
    assert_eq!(gone, vec![vec![Value::Null]]);

    engine.close_session(s1).unwrap();
    engine.close_session(s2).unwrap();
}

#[test]
fn create_database_as_clone_of_and_as_snapshot_of_work_and_isolate() {
    let engine = Engine::new();
    let s = engine.create_session();

    exec_ok(&engine, s, "CREATE DATABASE [TemplateDb];");
    exec_ok(
        &engine,
        s,
        "ALTER DATABASE [TemplateDb] SET READ_COMMITTED_SNAPSHOT ON;
         ALTER DATABASE [TemplateDb] SET ALLOW_SNAPSHOT_ISOLATION ON;",
    );
    exec_ok(
        &engine,
        s,
        "USE [TemplateDb];
         CREATE TABLE dbo.Items (Id INT PRIMARY KEY, Name NVARCHAR(50) NOT NULL);
         CREATE UNIQUE INDEX UX_Items_Name ON dbo.Items(Name);
         INSERT INTO dbo.Items VALUES (1, N'Alpha'), (2, N'Beta');
         CREATE VIEW dbo.vItems AS SELECT Id, Name FROM dbo.Items;
         CREATE SEQUENCE dbo.SeqItem START WITH 10 INCREMENT BY 1;",
    );

    // Writable clones via both Rust API and SQL DDL
    engine.clone_database("TemplateDb", "TestClone1").unwrap();
    exec_ok(
        &engine,
        s,
        "CREATE DATABASE [TestClone2] AS CLONE OF [TemplateDb];",
    );

    // Mutate TestClone1; verify TemplateDb and TestClone2 stay completely untouched
    exec_ok(
        &engine,
        s,
        "USE [TestClone1];
         INSERT INTO dbo.Items VALUES (3, N'Gamma');
         UPDATE dbo.Items SET Name = N'AlphaEdited' WHERE Id = 1;",
    );

    let clone1 = query_rows(
        &engine,
        s,
        "USE [TestClone1]; SELECT COUNT(*) FROM dbo.vItems;",
    );
    let clone2 = query_rows(
        &engine,
        s,
        "USE [TestClone2]; SELECT COUNT(*) FROM dbo.vItems;",
    );
    let tmpl = query_rows(
        &engine,
        s,
        "USE [TemplateDb]; SELECT Name FROM dbo.Items WHERE Id = 1;",
    );
    assert_eq!(as_i64(&clone1[0][0]), 3);
    assert_eq!(as_i64(&clone2[0][0]), 2);
    assert_eq!(tmpl, vec![vec![Value::NVarChar("Alpha".into())]]);

    // Unique index & RCSI options were cloned too
    let dup_err = engine
        .execute_session_batch_sql(
            s,
            "USE [TestClone2]; INSERT INTO dbo.Items VALUES (9, N'Alpha');",
        )
        .expect_err("cloned unique index should reject duplicate");
    assert_eq!(dup_err.number(), 2601);

    let clone_opts = query_rows(
        &engine,
        s,
        "SELECT is_read_committed_snapshot_on, snapshot_isolation_state, is_read_only
         FROM sys.databases WHERE name = 'TestClone2';",
    );
    assert_eq!(
        clone_opts,
        vec![vec![Value::Bit(true), Value::TinyInt(1), Value::Bit(false)]]
    );

    // Read-only snapshot via AS SNAPSHOT OF
    engine
        .create_database_snapshot("TemplateDb", "TemplateSnap")
        .unwrap();
    let snap_meta = query_rows(
        &engine,
        s,
        "SELECT is_read_only, CASE WHEN source_database_id = DB_ID('TemplateDb') THEN 1 ELSE 0 END
         FROM sys.databases WHERE name = 'TemplateSnap';",
    );
    assert_eq!(snap_meta, vec![vec![Value::Bit(true), Value::Int(1)]]);

    let snap_rows = query_rows(
        &engine,
        s,
        "USE [TemplateSnap]; SELECT COUNT(*) FROM dbo.Items;",
    );
    assert_eq!(as_i64(&snap_rows[0][0]), 2);

    let ro_err = engine
        .execute_session_batch_sql(
            s,
            "USE [TemplateSnap]; INSERT INTO dbo.Items VALUES (99, N'Forbidden');",
        )
        .expect_err("snapshot database must reject writes");
    assert_eq!(ro_err.number(), 3906, "expected 3906, got: {ro_err}");

    engine.close_session(s).unwrap();
}

#[test]
fn snapshot_isolation_allows_disjoint_row_commits_via_3way_merge_and_rejects_overlap_3960() {
    let engine = Engine::new();
    let s1 = engine.create_session();
    let s2 = engine.create_session();

    exec_ok(&engine, s1, "CREATE DATABASE [SnapMergeDb];");
    exec_ok(
        &engine,
        s1,
        "ALTER DATABASE [SnapMergeDb] SET ALLOW_SNAPSHOT_ISOLATION ON;",
    );
    exec_ok(
        &engine,
        s1,
        "USE [SnapMergeDb];
         CREATE TABLE dbo.Ledger (Id INT PRIMARY KEY, Amount INT NOT NULL);
         INSERT INTO dbo.Ledger VALUES (1, 100), (2, 200);",
    );
    exec_ok(&engine, s2, "USE [SnapMergeDb];");

    // Both enter SNAPSHOT isolation and update DISJOINT rows -> both commit cleanly!
    exec_ok(
        &engine,
        s1,
        "SET TRANSACTION ISOLATION LEVEL SNAPSHOT; BEGIN TRAN; SELECT Amount FROM dbo.Ledger WHERE Id = 1;",
    );
    exec_ok(
        &engine,
        s2,
        "SET TRANSACTION ISOLATION LEVEL SNAPSHOT; BEGIN TRAN; UPDATE dbo.Ledger SET Amount = 150 WHERE Id = 1; COMMIT TRAN;",
    );
    // s1 still sees 100 for Id=1, updates Id=2 (disjoint), and commits cleanly
    let s1_read = query_rows(&engine, s1, "SELECT Amount FROM dbo.Ledger WHERE Id = 1;");
    assert_eq!(as_i64(&s1_read[0][0]), 100);
    exec_ok(
        &engine,
        s1,
        "UPDATE dbo.Ledger SET Amount = 250 WHERE Id = 2; COMMIT TRAN;",
    );

    let final_rows = query_rows(
        &engine,
        s1,
        "SELECT Id, Amount FROM dbo.Ledger ORDER BY Id;",
    );
    assert_eq!(
        final_rows,
        vec![
            vec![Value::Int(1), Value::Int(150)],
            vec![Value::Int(2), Value::Int(250)],
        ]
    );

    // Now test overlapping row update -> immediate 3960 abort
    exec_ok(
        &engine,
        s1,
        "BEGIN TRAN; SELECT Amount FROM dbo.Ledger WHERE Id = 1;",
    );
    exec_ok(
        &engine,
        s2,
        "BEGIN TRAN; UPDATE dbo.Ledger SET Amount = 175 WHERE Id = 1; COMMIT TRAN;",
    );
    let conflict = engine
        .execute_session_batch_sql(s1, "UPDATE dbo.Ledger SET Amount = 999 WHERE Id = 1;")
        .expect_err("overlapping snapshot update must fail with 3960");
    assert_eq!(conflict.number(), 3960);

    engine.close_session(s1).unwrap();
    engine.close_session(s2).unwrap();
}
