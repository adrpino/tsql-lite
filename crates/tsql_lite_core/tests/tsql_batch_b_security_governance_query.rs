use tsql_lite_core::executor::engine::Engine;
use tsql_lite_core::executor::result::QueryResult;
use tsql_lite_core::types::Value;

fn run(engine: &mut Engine, sql: &str) -> QueryResult {
    let stmts = tsql_lite_core::parse_batch(sql)
        .unwrap_or_else(|e| panic!("SQL parse failed:\n{sql}\nError: {e}"));
    engine
        .execute_batch(stmts)
        .unwrap_or_else(|e| panic!("SQL failed:\n{sql}\nError: {e}"))
        .unwrap_or_default()
}

fn run_exec(engine: &mut Engine, sql: &str) {
    let stmts = tsql_lite_core::parse_batch(sql)
        .unwrap_or_else(|e| panic!("SQL parse failed:\n{sql}\nError: {e}"));
    let _ = engine
        .execute_batch(stmts)
        .unwrap_or_else(|e| panic!("SQL execute failed:\n{sql}\nError: {e}"));
}

#[test]
fn option_maxrecursion_default_explicit_zero_and_error_530() {
    let mut engine = Engine::new();

    // Explicit MAXRECURSION 3 allows 3 recursive steps (values 1, 2, 3, 4).
    let ok = run(
        &mut engine,
        "WITH nums AS (
            SELECT 1 AS n
            UNION ALL
            SELECT n + 1 FROM nums WHERE n < 4
         )
         SELECT COUNT(*), MAX(n) FROM nums OPTION (MAXRECURSION 3);",
    );
    assert_eq!(ok.rows[0][0].to_integer_i64(), Some(4));
    assert_eq!(ok.rows[0][1].to_integer_i64(), Some(4));

    // Exceeding MAXRECURSION 3 raises SQL Server Error 530.
    let err_530 = run(
        &mut engine,
        "DECLARE @err INT = 0;
         BEGIN TRY
             WITH nums AS (
                 SELECT 1 AS n
                 UNION ALL
                 SELECT n + 1 FROM nums WHERE n < 5
             )
             SELECT COUNT(*) FROM nums OPTION (MAXRECURSION 3);
         END TRY
         BEGIN CATCH
             SET @err = ERROR_NUMBER();
         END CATCH
         SELECT @err;",
    );
    assert_eq!(err_530.rows, vec![vec![Value::Int(530)]]);

    // Default MAXRECURSION is 100 (101 recursive steps fails with Error 530).
    let default_err = run(
        &mut engine,
        "DECLARE @err INT = 0;
         BEGIN TRY
             WITH nums AS (
                 SELECT 1 AS n
                 UNION ALL
                 SELECT n + 1 FROM nums WHERE n <= 101
             )
             SELECT COUNT(*) FROM nums;
         END TRY
         BEGIN CATCH
             SET @err = ERROR_NUMBER();
         END CATCH
         SELECT @err;",
    );
    assert_eq!(default_err.rows, vec![vec![Value::Int(530)]]);

    // OPTION (MAXRECURSION 0) allows > 100 recursive iterations.
    let unlimited = run(
        &mut engine,
        "WITH nums AS (
            SELECT 1 AS n
            UNION ALL
            SELECT n + 1 FROM nums WHERE n < 150
         )
         SELECT COUNT(*), MAX(n) FROM nums OPTION (MAXRECURSION 0);",
    );
    assert_eq!(unlimited.rows[0][0].to_integer_i64(), Some(150));
    assert_eq!(unlimited.rows[0][1].to_integer_i64(), Some(150));
}

#[test]
fn alter_table_add_column_default_not_null_and_with_values() {
    let mut engine = Engine::new();
    run_exec(
        &mut engine,
        "CREATE TABLE dbo.AddColTest (Id INT PRIMARY KEY, Name NVARCHAR(50) NOT NULL);
         INSERT INTO dbo.AddColTest (Id, Name) VALUES (1, N'Alpha'), (2, N'Beta');",
    );

    // 1. Nullable column with DEFAULT and NO WITH VALUES -> existing rows stay NULL.
    run_exec(
        &mut engine,
        "ALTER TABLE dbo.AddColTest ADD NullNoBackfill INT NULL DEFAULT 10;",
    );
    // 2. Nullable column with DEFAULT ... WITH VALUES -> existing rows backfilled.
    run_exec(
        &mut engine,
        "ALTER TABLE dbo.AddColTest ADD NullWithValues INT NULL DEFAULT 20 WITH VALUES;",
    );
    // 3. NOT NULL column with DEFAULT -> existing rows backfilled even without WITH VALUES.
    run_exec(
        &mut engine,
        "ALTER TABLE dbo.AddColTest ADD NotNullDefault NVARCHAR(20) NOT NULL CONSTRAINT DF_AddCol_NN DEFAULT N'ready';",
    );
    // 4. NOT NULL column without DEFAULT on non-empty table -> Error 4901.
    let err_4901 = run(
        &mut engine,
        "DECLARE @err INT = 0;
         BEGIN TRY
             ALTER TABLE dbo.AddColTest ADD BadCol INT NOT NULL;
         END TRY
         BEGIN CATCH
             SET @err = ERROR_NUMBER();
         END CATCH
         SELECT @err;",
    );
    assert_eq!(err_4901.rows, vec![vec![Value::Int(4901)]]);

    // New row gets all defaults.
    run_exec(
        &mut engine,
        "INSERT INTO dbo.AddColTest (Id, Name) VALUES (3, N'Gamma');",
    );

    let res = run(
        &mut engine,
        "SELECT Id, NullNoBackfill, NullWithValues, NotNullDefault
         FROM dbo.AddColTest ORDER BY Id;",
    );
    assert_eq!(
        res.rows,
        vec![
            vec![
                Value::Int(1),
                Value::Null,
                Value::Int(20),
                Value::NVarChar("ready".into())
            ],
            vec![
                Value::Int(2),
                Value::Null,
                Value::Int(20),
                Value::NVarChar("ready".into())
            ],
            vec![
                Value::Int(3),
                Value::Int(10),
                Value::Int(20),
                Value::NVarChar("ready".into())
            ],
        ]
    );
}

#[test]
fn nested_for_xml_type_composition_and_value_unescaping() {
    let mut engine = Engine::new();
    run_exec(
        &mut engine,
        "CREATE TABLE dbo.XmlDepts (DeptId INT PRIMARY KEY, DeptName NVARCHAR(50));
         CREATE TABLE dbo.XmlStaff (StaffId INT PRIMARY KEY, DeptId INT, FullName NVARCHAR(50));
         INSERT INTO dbo.XmlDepts VALUES (1, N'R&D'), (2, N'Ops');
         INSERT INTO dbo.XmlStaff VALUES (10, 1, N'A & B'), (11, 1, N'C <D>'), (20, 2, N'Eve');",
    );

    // Nested FOR XML ..., TYPE embeds raw child XML elements without double-escaping.
    let with_type = run(
        &mut engine,
        "SELECT
            d.DeptId AS '@id',
            d.DeptName AS 'name',
            (
                SELECT s.FullName AS 'member'
                FROM dbo.XmlStaff s
                WHERE s.DeptId = d.DeptId
                ORDER BY s.StaffId
                FOR XML PATH(''), TYPE
            ) AS 'members'
         FROM dbo.XmlDepts d
         ORDER BY d.DeptId
         FOR XML PATH('dept'), ROOT('depts');",
    );
    assert_eq!(
        with_type.rows[0][0].to_string_value(),
        "<depts><dept id=\"1\"><name>R&amp;D</name><members><member>A &amp; B</member><member>C &lt;D&gt;</member></members></dept><dept id=\"2\"><name>Ops</name><members><member>Eve</member></members></dept></depts>"
    );

    // Without TYPE, inner FOR XML returns text which the outer FOR XML escapes.
    let without_type = run(
        &mut engine,
        "SELECT
            (SELECT s.FullName AS 'm' FROM dbo.XmlStaff s WHERE s.DeptId = 2 FOR XML PATH('')) AS raw_str
         FOR XML PATH('wrap');",
    );
    assert_eq!(
        without_type.rows[0][0].to_string_value(),
        "<wrap><raw_str>&lt;m&gt;Eve&lt;/m&gt;</raw_str></wrap>"
    );

    // Classic STUFF((SELECT ... FOR XML PATH(''), TYPE).value('.', 'NVARCHAR(MAX)'), 1, 2, '')
    // preserves special XML characters unescaped.
    let csv = run(
        &mut engine,
        "SELECT STUFF((
            SELECT N', ' + FullName
            FROM dbo.XmlStaff
            ORDER BY StaffId
            FOR XML PATH(''), TYPE
         ).value('.', 'NVARCHAR(MAX)'), 1, 2, N'');",
    );
    assert_eq!(csv.rows[0][0].to_string_value(), "A & B, C <D>, Eve");
}

#[test]
fn updatable_views_and_with_check_option_error_550() {
    let mut engine = Engine::new();
    run_exec(
        &mut engine,
        "CREATE TABLE dbo.ViewBase (
            Id INT PRIMARY KEY,
            Region NVARCHAR(20) NOT NULL,
            Amount INT NOT NULL
         );
         INSERT INTO dbo.ViewBase VALUES (1, N'West', 100), (2, N'East', 200);
         CREATE VIEW dbo.vWestOnly AS
             SELECT Id, Region AS ZoneName, Amount FROM dbo.ViewBase WHERE Region = N'West'
             WITH CHECK OPTION;",
    );

    // sys.views exposes with_check_option = 1
    let sys_v = run(
        &mut engine,
        "SELECT with_check_option FROM sys.views WHERE name = 'vWestOnly';",
    );
    assert_eq!(sys_v.rows, vec![vec![Value::Bit(true)]]);

    // Valid INSERT through view (with aliased column ZoneName -> Region) succeeds.
    run_exec(
        &mut engine,
        "INSERT INTO dbo.vWestOnly (Id, ZoneName, Amount) VALUES (3, N'West', 300);",
    );

    // Violating INSERT fails with Error 550 and leaves base table untouched.
    let ins_err = run(
        &mut engine,
        "DECLARE @err INT = 0;
         BEGIN TRY
             INSERT INTO dbo.vWestOnly (Id, ZoneName, Amount) VALUES (4, N'East', 400);
         END TRY
         BEGIN CATCH
             SET @err = ERROR_NUMBER();
         END CATCH
         SELECT @err, (SELECT COUNT(*) FROM dbo.ViewBase);",
    );
    assert_eq!(ins_err.rows[0][0].to_integer_i64(), Some(550));
    assert_eq!(ins_err.rows[0][1].to_integer_i64(), Some(3));

    // Valid UPDATE through view only touches rows visible through view.
    run_exec(
        &mut engine,
        "UPDATE dbo.vWestOnly SET Amount = Amount + 50;",
    );

    // Violating UPDATE (moving a West row out of the view's predicate) fails atomically with Error 550.
    let upd_err = run(
        &mut engine,
        "DECLARE @err INT = 0;
         BEGIN TRY
             UPDATE dbo.vWestOnly SET ZoneName = N'South' WHERE Id = 1;
         END TRY
         BEGIN CATCH
             SET @err = ERROR_NUMBER();
         END CATCH
         SELECT @err, (SELECT Region FROM dbo.ViewBase WHERE Id = 1);",
    );
    assert_eq!(
        upd_err.rows,
        vec![vec![Value::Int(550), Value::NVarChar("West".into())]]
    );

    // DELETE through view only deletes matching rows visible through the view.
    run_exec(&mut engine, "DELETE FROM dbo.vWestOnly WHERE Id IN (2, 3);");

    let final_rows = run(
        &mut engine,
        "SELECT Id, Region, Amount FROM dbo.ViewBase ORDER BY Id;",
    );
    assert_eq!(
        final_rows.rows,
        vec![
            vec![
                Value::Int(1),
                Value::NVarChar("West".into()),
                Value::Int(150)
            ],
            vec![
                Value::Int(2),
                Value::NVarChar("East".into()),
                Value::Int(200)
            ],
        ]
    );
}

#[test]
fn dynamic_data_masking_enforcement_unmask_and_catalog() {
    let mut engine = Engine::new();
    run_exec(
        &mut engine,
        "CREATE TABLE dbo.MaskedPeople (
            Id INT PRIMARY KEY,
            Email VARCHAR(100) MASKED WITH (FUNCTION = 'email()') NULL,
            Phone VARCHAR(20) MASKED WITH (FUNCTION = 'partial(2, \"XXX\", 2)') NULL,
            Salary INT MASKED WITH (FUNCTION = 'default()') NOT NULL,
            Score INT MASKED WITH (FUNCTION = 'random(100, 200)') NOT NULL,
            BirthDate DATE MASKED WITH (FUNCTION = 'datetime(\"Y\")') NULL
         );
         INSERT INTO dbo.MaskedPeople VALUES
            (1, 'alice@example.org', '555-123456', 95000, 77, '1992-07-14'),
            (2, NULL, 'ab', 50000, 88, NULL);
         CREATE USER mask_reader WITHOUT LOGIN;
         GRANT SELECT ON dbo.MaskedPeople TO mask_reader;",
    );

    // sys.masked_columns reflects all 5 masked columns.
    let cat = run(
        &mut engine,
        "SELECT name, is_masked, masking_function
         FROM sys.masked_columns
         WHERE object_id = OBJECT_ID('dbo.MaskedPeople')
         ORDER BY column_id;",
    );
    assert_eq!(cat.rows.len(), 5);
    assert_eq!(cat.rows[0][0].to_string_value(), "Email");
    assert_eq!(cat.rows[0][1], Value::Bit(true));
    assert_eq!(cat.rows[0][2].to_string_value(), "email()");

    // dbo sees real unmasked data.
    let dbo_view = run(
        &mut engine,
        "SELECT Email, Phone, Salary, BirthDate FROM dbo.MaskedPeople WHERE Id = 1;",
    );
    assert_eq!(dbo_view.rows[0][0].to_string_value(), "alice@example.org");
    assert_eq!(dbo_view.rows[0][1].to_string_value(), "555-123456");
    assert_eq!(dbo_view.rows[0][2], Value::Int(95000));
    assert_eq!(dbo_view.rows[0][3].to_string_value(), "1992-07-14");

    // Non-privileged user sees masked data, WHERE still matches unmasked value, NULL stays NULL.
    run_exec(&mut engine, "EXECUTE AS USER = 'mask_reader';");
    let masked_view = run(
        &mut engine,
        "SELECT Id, Email, Phone, Salary, Score, BirthDate
         FROM dbo.MaskedPeople
         WHERE Email = 'alice@example.org' OR Id = 2
         ORDER BY Id;",
    );
    run_exec(&mut engine, "REVERT;");
    assert_eq!(masked_view.rows.len(), 2);
    assert_eq!(masked_view.rows[0][1].to_string_value(), "aXXX@XXXX.com");
    assert_eq!(masked_view.rows[0][2].to_string_value(), "55XXX56");
    assert_eq!(masked_view.rows[0][3], Value::Int(0));
    if let Value::Int(score) = masked_view.rows[0][4] {
        assert!(
            (100..=200).contains(&score),
            "random mask out of range: {score}"
        );
    } else {
        panic!("expected int for masked Score");
    }
    assert_eq!(masked_view.rows[0][5].to_string_value(), "2000-07-14");
    // Row 2: NULL stays NULL; short string <= prefix+suffix becomes padding "XXX".
    assert_eq!(masked_view.rows[1][1], Value::Null);
    assert_eq!(masked_view.rows[1][2].to_string_value(), "XXX");
    assert_eq!(masked_view.rows[1][5], Value::Null);

    // GRANT UNMASK reveals data; REVOKE UNMASK re-masks it.
    run_exec(&mut engine, "GRANT UNMASK TO mask_reader;");
    run_exec(&mut engine, "EXECUTE AS USER = 'mask_reader';");
    let unmasked = run(
        &mut engine,
        "SELECT Email, Salary FROM dbo.MaskedPeople WHERE Id = 1;",
    );
    run_exec(&mut engine, "REVERT;");
    assert_eq!(unmasked.rows[0][0].to_string_value(), "alice@example.org");
    assert_eq!(unmasked.rows[0][1], Value::Int(95000));

    run_exec(&mut engine, "REVOKE UNMASK FROM mask_reader;");
    // DROP MASKED removes masking from Email specifically.
    run_exec(
        &mut engine,
        "ALTER TABLE dbo.MaskedPeople ALTER COLUMN Email DROP MASKED;",
    );
    run_exec(&mut engine, "EXECUTE AS USER = 'mask_reader';");
    let after_drop = run(
        &mut engine,
        "SELECT Email, Salary FROM dbo.MaskedPeople WHERE Id = 1;",
    );
    run_exec(&mut engine, "REVERT;");
    assert_eq!(after_drop.rows[0][0].to_string_value(), "alice@example.org");
    assert_eq!(after_drop.rows[0][1], Value::Int(0));
}

#[test]
fn row_level_security_filter_block_alter_drop_and_catalog() {
    let mut engine = Engine::new();
    run_exec(
        &mut engine,
        "CREATE TABLE dbo.TenantDocs (
            DocId INT PRIMARY KEY,
            TenantId INT NOT NULL,
            Title NVARCHAR(50) NOT NULL
         );
         INSERT INTO dbo.TenantDocs VALUES
            (1, 10, N'T10-A'),
            (2, 10, N'T10-B'),
            (3, 20, N'T20-A');",
    );
    run_exec(
        &mut engine,
        "CREATE FUNCTION dbo.fn_tenant_rls(@TenantId INT)
         RETURNS TABLE
         WITH SCHEMABINDING
         AS
         RETURN SELECT 1 AS ok
                WHERE @TenantId = CAST(SESSION_CONTEXT(N'TenantId') AS INT)
                   OR CAST(SESSION_CONTEXT(N'IsAdmin') AS INT) = 1;",
    );
    run_exec(
        &mut engine,
        "CREATE SECURITY POLICY dbo.TenantPolicy
            ADD FILTER PREDICATE dbo.fn_tenant_rls(TenantId) ON dbo.TenantDocs,
            ADD BLOCK PREDICATE dbo.fn_tenant_rls(TenantId) ON dbo.TenantDocs AFTER INSERT,
            ADD BLOCK PREDICATE dbo.fn_tenant_rls(TenantId) ON dbo.TenantDocs BEFORE UPDATE,
            ADD BLOCK PREDICATE dbo.fn_tenant_rls(TenantId) ON dbo.TenantDocs AFTER UPDATE,
            ADD BLOCK PREDICATE dbo.fn_tenant_rls(TenantId) ON dbo.TenantDocs BEFORE DELETE
         WITH (STATE = ON);",
    );

    // Catalog views sys.security_policies and sys.security_predicates are populated.
    let pol = run(
        &mut engine,
        "SELECT name, is_enabled, is_schema_bound FROM sys.security_policies WHERE name = 'TenantPolicy';",
    );
    assert_eq!(
        pol.rows,
        vec![vec![
            Value::VarChar("TenantPolicy".into()),
            Value::Bit(true),
            Value::Bit(true)
        ]]
    );
    let preds = run(&mut engine, "SELECT COUNT(*) FROM sys.security_predicates;");
    assert_eq!(preds.rows[0][0].to_integer_i64(), Some(5));

    // Tenant 10 sees only its 2 rows.
    let t10 = run(
        &mut engine,
        "EXEC sp_set_session_context @key = N'TenantId', @value = 10;
         EXEC sp_set_session_context @key = N'IsAdmin', @value = 0;
         SELECT DocId FROM dbo.TenantDocs ORDER BY DocId;",
    );
    assert_eq!(t10.rows, vec![vec![Value::Int(1)], vec![Value::Int(2)]]);

    // AFTER INSERT block predicate allows Tenant 10 row, blocks Tenant 20 row with Error 33504.
    run_exec(
        &mut engine,
        "INSERT INTO dbo.TenantDocs VALUES (4, 10, N'T10-C');",
    );
    let blk_ins = run(
        &mut engine,
        "DECLARE @err INT = 0;
         BEGIN TRY
             INSERT INTO dbo.TenantDocs VALUES (5, 20, N'Cross-Tenant');
         END TRY
         BEGIN CATCH
             SET @err = ERROR_NUMBER();
         END CATCH
         SELECT @err;",
    );
    assert_eq!(blk_ins.rows, vec![vec![Value::Int(33504)]]);

    // AFTER UPDATE block predicate blocks reassigning a row to another tenant (Error 33504).
    let blk_upd = run(
        &mut engine,
        "DECLARE @err INT = 0;
         BEGIN TRY
             UPDATE dbo.TenantDocs SET TenantId = 20 WHERE DocId = 1;
         END TRY
         BEGIN CATCH
             SET @err = ERROR_NUMBER();
         END CATCH
         SELECT @err, (SELECT TenantId FROM dbo.TenantDocs WHERE DocId = 1);",
    );
    assert_eq!(blk_upd.rows, vec![vec![Value::Int(33504), Value::Int(10)]]);

    // FILTER PREDICATE scopes DELETE to visible rows only (Tenant 20 row stays intact).
    run_exec(
        &mut engine,
        "DELETE FROM dbo.TenantDocs WHERE DocId IN (2, 3);",
    );

    // Disable policy via ALTER SECURITY POLICY WITH (STATE = OFF) -> all rows visible.
    run_exec(
        &mut engine,
        "ALTER SECURITY POLICY dbo.TenantPolicy WITH (STATE = OFF);",
    );
    let all = run(
        &mut engine,
        "SELECT DocId, TenantId FROM dbo.TenantDocs ORDER BY DocId;",
    );
    assert_eq!(
        all.rows,
        vec![
            vec![Value::Int(1), Value::Int(10)],
            vec![Value::Int(3), Value::Int(20)],
            vec![Value::Int(4), Value::Int(10)],
        ]
    );

    // DROP SECURITY POLICY cleans up catalog.
    run_exec(&mut engine, "DROP SECURITY POLICY dbo.TenantPolicy;");
    let after_drop = run(&mut engine, "SELECT COUNT(*) FROM sys.security_policies;");
    assert_eq!(after_drop.rows[0][0].to_integer_i64(), Some(0));
}
