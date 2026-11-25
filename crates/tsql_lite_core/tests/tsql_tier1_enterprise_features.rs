use tsql_lite_core::executor::result::QueryResult;
use tsql_lite_core::types::Value;
use tsql_lite_core::{parse_batch, DbError, Engine};

fn run(engine: &Engine, sql: &str) -> Result<Option<QueryResult>, DbError> {
    let stmts = parse_batch(sql)?;
    engine.execute_batch(stmts)
}

fn query(engine: &Engine, sql: &str) -> Vec<Vec<Value>> {
    run(engine, sql)
        .expect("query execution failed")
        .expect("expected result set")
        .rows
}

#[test]
fn filtered_and_unique_indexes_enforce_uniqueness_and_surface_in_sys_indexes() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Accounts (Id INT PRIMARY KEY, ExtCode VARCHAR(20) NULL, IsDeleted BIT NOT NULL);
         CREATE UNIQUE INDEX IX_Accounts_ExtCode_Active
             ON dbo.Accounts (ExtCode)
             WHERE ExtCode IS NOT NULL AND IsDeleted = 0;",
    )
    .unwrap();

    // Multiple NULLs and soft-deleted duplicates are allowed by the filter predicate
    run(
        &engine,
        "INSERT INTO dbo.Accounts VALUES
            (1, NULL, 0),
            (2, NULL, 0),
            (3, 'ABC', 1),
            (4, 'ABC', 1),
            (5, 'ABC', 0);",
    )
    .unwrap();

    // Inserting a second active 'ABC' violates the filtered unique index
    let dup_insert = run(&engine, "INSERT INTO dbo.Accounts VALUES (6, 'ABC', 0);");
    assert!(
        dup_insert.is_err(),
        "expected filtered unique index violation on INSERT"
    );

    // Updating a soft-deleted 'ABC' to active violates the filtered unique index
    let dup_update = run(
        &engine,
        "UPDATE dbo.Accounts SET IsDeleted = 0 WHERE Id = 3;",
    );
    assert!(
        dup_update.is_err(),
        "expected filtered unique index violation on UPDATE"
    );

    // Soft-deleting the active row frees 'ABC' for another active row
    run(
        &engine,
        "UPDATE dbo.Accounts SET IsDeleted = 1 WHERE Id = 5;
         INSERT INTO dbo.Accounts VALUES (6, 'ABC', 0);",
    )
    .unwrap();

    // CREATE UNIQUE INDEX fails if existing rows already violate it
    let bad_idx = run(
        &engine,
        "CREATE UNIQUE INDEX IX_Accounts_ExtCode_All ON dbo.Accounts (ExtCode);",
    );
    assert!(
        bad_idx.is_err(),
        "expected CREATE UNIQUE INDEX to reject existing duplicates"
    );

    // sys.indexes surfaces has_filter = 1 and non-null filter_definition
    let idx_rows = query(
        &engine,
        "SELECT has_filter, filter_definition
         FROM sys.indexes
         WHERE name = 'IX_Accounts_ExtCode_Active';",
    );
    assert_eq!(idx_rows.len(), 1);
    assert_eq!(idx_rows[0][0], Value::Bit(true));
    assert!(!idx_rows[0][1].is_null());
}

#[test]
fn alter_table_nocheck_and_with_check_constraint_enforcement() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Parents (Id INT PRIMARY KEY);
         INSERT INTO dbo.Parents VALUES (1);
         CREATE TABLE dbo.Children (
             Id INT PRIMARY KEY,
             ParentId INT,
             Amount INT,
             CONSTRAINT CK_Amount_Pos CHECK (Amount > 0),
             CONSTRAINT FK_Child_Parent FOREIGN KEY (ParentId) REFERENCES dbo.Parents(Id)
         );",
    )
    .unwrap();

    // Initially enforced
    assert!(run(&engine, "INSERT INTO dbo.Children VALUES (1, 1, -5);").is_err());
    assert!(run(&engine, "INSERT INTO dbo.Children VALUES (1, 999, 10);").is_err());

    // Disable all constraints -> bad rows succeed, sys.* shows is_disabled = 1, is_not_trusted = 1
    run(
        &engine,
        "ALTER TABLE dbo.Children NOCHECK CONSTRAINT ALL;
         INSERT INTO dbo.Children VALUES (1, 999, -5);",
    )
    .unwrap();

    let meta_disabled = query(
        &engine,
        "SELECT is_disabled, is_not_trusted FROM sys.check_constraints WHERE name = 'CK_Amount_Pos';",
    );
    assert_eq!(meta_disabled[0], vec![Value::Bit(true), Value::Bit(true)]);

    let fk_disabled = query(
        &engine,
        "SELECT is_disabled FROM sys.foreign_keys WHERE name = 'FK_Child_Parent';",
    );
    assert_eq!(fk_disabled[0], vec![Value::Bit(true)]);

    // Re-enabling WITH CHECK fails because existing bad row violates both constraints
    assert!(run(
        &engine,
        "ALTER TABLE dbo.Children WITH CHECK CHECK CONSTRAINT CK_Amount_Pos;"
    )
    .is_err());
    assert!(run(
        &engine,
        "ALTER TABLE dbo.Children WITH CHECK CHECK CONSTRAINT FK_Child_Parent;"
    )
    .is_err());

    // Re-enabling WITHOUT WITH CHECK succeeds, enforces future writes, leaves is_not_trusted = 1
    run(&engine, "ALTER TABLE dbo.Children CHECK CONSTRAINT ALL;").unwrap();
    let meta_untrusted = query(
        &engine,
        "SELECT is_disabled, is_not_trusted FROM sys.check_constraints WHERE name = 'CK_Amount_Pos';",
    );
    assert_eq!(meta_untrusted[0], vec![Value::Bit(false), Value::Bit(true)]);
    assert!(run(&engine, "INSERT INTO dbo.Children VALUES (2, 1, -1);").is_err());

    // Clean bad row -> WITH CHECK CHECK CONSTRAINT succeeds and marks trusted (is_not_trusted = 0)
    run(
        &engine,
        "UPDATE dbo.Children SET ParentId = 1, Amount = 10 WHERE Id = 1;
         ALTER TABLE dbo.Children WITH CHECK CHECK CONSTRAINT ALL;",
    )
    .unwrap();
    let meta_trusted = query(
        &engine,
        "SELECT is_disabled, is_not_trusted FROM sys.check_constraints WHERE name = 'CK_Amount_Pos';",
    );
    assert_eq!(meta_trusted[0], vec![Value::Bit(false), Value::Bit(false)]);
}

#[test]
fn sys_sql_expression_dependencies_tracks_views_procs_functions_and_triggers() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Products (Id INT PRIMARY KEY, Price INT);
         CREATE TABLE dbo.AuditLog (ProductId INT);",
    )
    .unwrap();
    run(
        &engine,
        "CREATE VIEW dbo.vExpensiveProducts AS
             SELECT Id, Price FROM dbo.Products WHERE Price > 100;",
    )
    .unwrap();
    run(
        &engine,
        "CREATE PROCEDURE dbo.usp_LogProducts AS
         BEGIN
             INSERT INTO dbo.AuditLog (ProductId)
             SELECT Id FROM dbo.vExpensiveProducts;
         END;",
    )
    .unwrap();
    run(
        &engine,
        "CREATE TRIGGER dbo.trProductsAudit ON dbo.Products AFTER INSERT AS
         BEGIN
             INSERT INTO dbo.AuditLog (ProductId) SELECT Id FROM inserted;
         END;",
    )
    .unwrap();

    let view_deps = query(
        &engine,
        "SELECT referenced_schema, referenced_name, referenced_id
         FROM sys.sql_expression_dependencies
         WHERE referencing_name = 'vExpensiveProducts';",
    );
    assert_eq!(view_deps.len(), 1);
    assert_eq!(view_deps[0][0], Value::VarChar("dbo".into()));
    assert_eq!(view_deps[0][1], Value::VarChar("Products".into()));
    assert!(!view_deps[0][2].is_null());

    let proc_deps = query(
        &engine,
        "SELECT referenced_name
         FROM sys.sql_expression_dependencies
         WHERE referencing_name = 'usp_LogProducts'
         ORDER BY referenced_name;",
    );
    assert_eq!(
        proc_deps,
        vec![
            vec![Value::VarChar("AuditLog".into())],
            vec![Value::VarChar("vExpensiveProducts".into())],
        ]
    );

    let trig_deps = query(
        &engine,
        "SELECT referenced_name
         FROM sys.sql_expression_dependencies
         WHERE referencing_name = 'trProductsAudit'
         ORDER BY referenced_name;",
    );
    assert_eq!(
        trig_deps,
        vec![
            vec![Value::VarChar("AuditLog".into())],
            vec![Value::VarChar("Products".into())],
        ]
    );
}

#[test]
fn system_versioned_temporal_tables_archive_and_query_for_system_time() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Employees (
             EmpId INT PRIMARY KEY,
             Salary INT NOT NULL,
             ValidFrom DATETIME2 GENERATED ALWAYS AS ROW START NOT NULL,
             ValidTo DATETIME2 GENERATED ALWAYS AS ROW END NOT NULL,
             PERIOD FOR SYSTEM_TIME (ValidFrom, ValidTo)
         ) WITH (SYSTEM_VERSIONING = ON (HISTORY_TABLE = dbo.EmployeesHistory));",
    )
    .unwrap();

    // Insert row with controlled historical timestamps to test exact AS OF / BETWEEN / CONTAINED IN / ALL
    run(
        &engine,
        "INSERT INTO dbo.Employees (EmpId, Salary, ValidFrom, ValidTo)
         VALUES (1, 100, '2024-01-01 00:00:00', '9999-12-31 23:59:59');",
    )
    .unwrap();

    // Update archives the old version into dbo.EmployeesHistory
    run(
        &engine,
        "UPDATE dbo.Employees SET Salary = 150 WHERE EmpId = 1;",
    )
    .unwrap();

    let hist_count = query(&engine, "SELECT COUNT(*) FROM dbo.EmployeesHistory;");
    assert_eq!(hist_count[0][0], Value::BigInt(1));

    // Delete archives the current version as well
    run(&engine, "DELETE FROM dbo.Employees WHERE EmpId = 1;").unwrap();
    let cur_count = query(&engine, "SELECT COUNT(*) FROM dbo.Employees;");
    assert_eq!(cur_count[0][0], Value::BigInt(0));

    // FOR SYSTEM_TIME ALL returns both archived versions
    let all_rows = query(
        &engine,
        "SELECT Salary FROM dbo.Employees FOR SYSTEM_TIME ALL ORDER BY Salary;",
    );
    assert_eq!(all_rows, vec![vec![Value::Int(100)], vec![Value::Int(150)]]);

    // FOR SYSTEM_TIME AS OF '2024-06-01' sees the original 100 salary version
    let as_of_rows = query(
        &engine,
        "SELECT Salary FROM dbo.Employees FOR SYSTEM_TIME AS OF '2024-06-01 00:00:00';",
    );
    assert_eq!(as_of_rows, vec![vec![Value::Int(100)]]);

    // sys.periods exposes the SYSTEM_TIME period
    let period_rows = query(
        &engine,
        "SELECT name, start_column_id, end_column_id FROM sys.periods;",
    );
    assert_eq!(period_rows.len(), 1);
    assert_eq!(period_rows[0][0], Value::VarChar("SYSTEM_TIME".into()));
    assert_eq!(period_rows[0][1], Value::Int(3));
    assert_eq!(period_rows[0][2], Value::Int(4));

    // ALTER TABLE SET (SYSTEM_VERSIONING = OFF) stops archiving further mutations
    run(
        &engine,
        "ALTER TABLE dbo.Employees SET (SYSTEM_VERSIONING = OFF);
         INSERT INTO dbo.Employees (EmpId, Salary, ValidFrom, ValidTo)
         VALUES (2, 200, '2025-01-01 00:00:00', '9999-12-31 23:59:59');
         DELETE FROM dbo.Employees WHERE EmpId = 2;",
    )
    .unwrap();
    let hist_after_off = query(&engine, "SELECT COUNT(*) FROM dbo.EmployeesHistory;");
    assert_eq!(hist_after_off[0][0], Value::BigInt(2));
}

#[test]
fn named_window_clause_supports_over_name_and_inheritance() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Sales (Dept VARCHAR(10), DayNo INT, Amt INT);
         INSERT INTO dbo.Sales VALUES
             ('A', 1, 10), ('A', 2, 20), ('A', 3, 30),
             ('B', 1, 5),  ('B', 2, 15);",
    )
    .unwrap();

    let rows = query(
        &engine,
        "SELECT
             Dept,
             DayNo,
             SUM(Amt) OVER w_dept AS DeptTotal,
             SUM(Amt) OVER w_running AS RunningTotal,
             ROW_NUMBER() OVER (w_dept ORDER BY DayNo DESC) AS RevRank
         FROM dbo.Sales
         WINDOW
             w_dept AS (PARTITION BY Dept),
             w_running AS (w_dept ORDER BY DayNo ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)
         ORDER BY Dept, DayNo;",
    );
    assert_eq!(rows.len(), 5);
    assert_eq!(
        rows[0],
        vec![
            Value::VarChar("A".into()),
            Value::Int(1),
            Value::BigInt(60),
            Value::BigInt(10),
            Value::Int(3)
        ]
    );
    assert_eq!(
        rows[1],
        vec![
            Value::VarChar("A".into()),
            Value::Int(2),
            Value::BigInt(60),
            Value::BigInt(30),
            Value::Int(2)
        ]
    );
    assert_eq!(
        rows[2],
        vec![
            Value::VarChar("A".into()),
            Value::Int(3),
            Value::BigInt(60),
            Value::BigInt(60),
            Value::Int(1)
        ]
    );
    assert_eq!(
        rows[3],
        vec![
            Value::VarChar("B".into()),
            Value::Int(1),
            Value::BigInt(20),
            Value::BigInt(5),
            Value::Int(2)
        ]
    );
    assert_eq!(
        rows[4],
        vec![
            Value::VarChar("B".into()),
            Value::Int(2),
            Value::BigInt(20),
            Value::BigInt(20),
            Value::Int(1)
        ]
    );
}

#[test]
fn runtime_security_execute_as_revert_grant_deny_revoke_and_roles() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE TABLE dbo.Secrets (Id INT PRIMARY KEY, Val VARCHAR(20));
         INSERT INTO dbo.Secrets VALUES (1, 'top');",
    )
    .unwrap();
    run(
        &engine,
        "CREATE PROCEDURE dbo.usp_ReadSecret AS
         BEGIN
             SELECT Val FROM dbo.Secrets;
         END;",
    )
    .unwrap();
    run(
        &engine,
        "CREATE USER reader WITHOUT LOGIN;
         CREATE USER blocked WITHOUT LOGIN;
         CREATE ROLE app_readers;
         ALTER ROLE app_readers ADD MEMBER reader;
         ALTER ROLE app_readers ADD MEMBER blocked;
         GRANT SELECT ON dbo.Secrets TO app_readers;
         GRANT EXECUTE ON dbo.usp_ReadSecret TO app_readers;
         DENY SELECT ON dbo.Secrets TO blocked;",
    )
    .unwrap();

    // Impersonate reader: SELECT & EXEC allowed, INSERT/UPDATE/DELETE denied
    run(&engine, "EXECUTE AS USER = 'reader';").unwrap();
    let who = query(&engine, "SELECT CURRENT_USER, USER_NAME();");
    assert_eq!(
        who[0],
        vec![
            Value::NVarChar("reader".into()),
            Value::NVarChar("reader".into())
        ]
    );
    let sel = query(&engine, "SELECT Val FROM dbo.Secrets;");
    assert_eq!(sel[0][0], Value::VarChar("top".into()));
    let exec_res = query(&engine, "EXEC dbo.usp_ReadSecret;");
    assert_eq!(exec_res[0][0], Value::VarChar("top".into()));
    assert!(run(&engine, "INSERT INTO dbo.Secrets VALUES (2, 'bad');").is_err());
    assert!(run(&engine, "UPDATE dbo.Secrets SET Val = 'x' WHERE Id = 1;").is_err());
    assert!(run(&engine, "DELETE FROM dbo.Secrets WHERE Id = 1;").is_err());

    // REVERT restores dbo privileges
    run(&engine, "REVERT;").unwrap();
    let restored = query(&engine, "SELECT CURRENT_USER;");
    assert_eq!(restored[0][0], Value::NVarChar("dbo".into()));

    // Impersonate blocked: explicit DENY overrides role GRANT
    run(&engine, "EXECUTE AS USER = 'blocked';").unwrap();
    assert!(run(&engine, "SELECT * FROM dbo.Secrets;").is_err());
    run(&engine, "REVERT;").unwrap();

    // REVOKE removes permission from role
    run(
        &engine,
        "REVOKE SELECT ON dbo.Secrets FROM app_readers;
         EXECUTE AS USER = 'reader';",
    )
    .unwrap();
    assert!(run(&engine, "SELECT * FROM dbo.Secrets;").is_err());
    run(&engine, "REVERT;").unwrap();
}

#[test]
fn legacy_openxml_with_sp_xml_preparedocument_and_removedocument() {
    let engine = Engine::new();
    run(
        &engine,
        "DECLARE @hdoc INT;
         DECLARE @xml NVARCHAR(MAX) = N'
         <ROOT>
             <Customer CustomerID=\"C1\" ContactName=\"Alice\">
                 <Order OrderID=\"101\" Amount=\"250\" />
                 <Order OrderID=\"102\" Amount=\"400\" />
             </Customer>
             <Customer CustomerID=\"C2\" ContactName=\"Bob\">
                 <Order OrderID=\"201\" Amount=\"99\" />
             </Customer>
         </ROOT>';
         EXEC sp_xml_preparedocument @hdoc OUTPUT, @xml;",
    )
    .unwrap();
    let rows = query(
        &engine,
        "SELECT CustomerID, ContactName, OrderID, Amount
         FROM OPENXML(1, '/ROOT/Customer/Order', 1)
         WITH (
             CustomerID VARCHAR(10) '../@CustomerID',
             ContactName VARCHAR(20) '../@ContactName',
             OrderID INT '@OrderID',
             Amount INT '@Amount'
         )
         ORDER BY OrderID;",
    );
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0],
        vec![
            Value::VarChar("C1".into()),
            Value::VarChar("Alice".into()),
            Value::Int(101),
            Value::Int(250),
        ]
    );
    assert_eq!(
        rows[2],
        vec![
            Value::VarChar("C2".into()),
            Value::VarChar("Bob".into()),
            Value::Int(201),
            Value::Int(99),
        ]
    );

    // Removing the handle frees it so subsequent OPENXML calls fail
    run(&engine, "EXEC sp_xml_removedocument 1;").unwrap();
    assert!(
        run(
            &engine,
            "SELECT * FROM OPENXML(1, '/ROOT/Customer', 1) WITH (CustomerID VARCHAR(10));"
        )
        .is_err(),
        "expected OPENXML on removed document handle to fail"
    );

    // Element-centric flags = 2 with @h variable in same batch
    let elem_rows = query(
        &engine,
        "DECLARE @h INT;
         EXEC sp_xml_preparedocument @h OUTPUT, N'<Items><Item><Code>X1</Code><Qty>7</Qty></Item><Item><Code>X2</Code><Qty>3</Qty></Item></Items>';
         SELECT Code, Qty FROM OPENXML(@h, '/Items/Item', 2) WITH (Code VARCHAR(10), Qty INT) ORDER BY Code;",
    );
    assert_eq!(
        elem_rows,
        vec![
            vec![Value::VarChar("X1".into()), Value::Int(7)],
            vec![Value::VarChar("X2".into()), Value::Int(3)],
        ]
    );
}
