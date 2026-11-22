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
fn fulltext_indexes_and_extended_properties_persist_in_sys_catalog() {
    let engine = Engine::new();

    // sys.fulltext_indexes exposes unique_index_id
    let ft = query(
        &engine,
        "SELECT object_id, unique_index_id, is_enabled FROM sys.fulltext_indexes;",
    );
    assert_eq!(
        ft.columns,
        vec!["object_id", "unique_index_id", "is_enabled"]
    );
    assert!(ft.rows.is_empty());

    // sp_addextendedproperty / update / drop persist into sys.extended_properties
    run(
        &engine,
        "CREATE TABLE dbo.Products (Id INT PRIMARY KEY, Title VARCHAR(50));
         EXEC sys.sp_addextendedproperty
             @name = N'MS_Description',
             @value = N'Product catalog table',
             @level0type = N'SCHEMA', @level0name = N'dbo',
             @level1type = N'TABLE',  @level1name = N'Products';
         EXEC sys.sp_addextendedproperty
             @name = N'MS_Description',
             @value = N'Display title',
             @level0type = N'SCHEMA', @level0name = N'dbo',
             @level1type = N'TABLE',  @level1name = N'Products',
             @level2type = N'COLUMN', @level2name = N'Title';",
    )
    .expect("add extended properties");

    let eps = query(
        &engine,
        "SELECT minor_id, name, value FROM sys.extended_properties ORDER BY minor_id;",
    );
    assert_eq!(eps.rows.len(), 2);
    assert_eq!(eps.rows[0][0], Value::Int(0));
    assert_eq!(eps.rows[0][2].to_string_value(), "Product catalog table");
    assert!(eps.rows[1][0].to_integer_i64().unwrap() > 0);
    assert_eq!(eps.rows[1][2].to_string_value(), "Display title");

    // Update & Drop extended property
    run(
        &engine,
        "EXEC sys.sp_updateextendedproperty
             @name = N'MS_Description',
             @value = N'Updated catalog table',
             @level0type = N'SCHEMA', @level0name = N'dbo',
             @level1type = N'TABLE',  @level1name = N'Products';
         EXEC sys.sp_dropextendedproperty
             @name = N'MS_Description',
             @level0type = N'SCHEMA', @level0name = N'dbo',
             @level1type = N'TABLE',  @level1name = N'Products',
             @level2type = N'COLUMN', @level2name = N'Title';",
    )
    .expect("update and drop extended properties");

    let eps_after = query(&engine, "SELECT name, value FROM sys.extended_properties;");
    assert_eq!(eps_after.rows.len(), 1);
    assert_eq!(
        eps_after.rows[0][1].to_string_value(),
        "Updated catalog table"
    );
}

#[test]
fn security_principals_roles_and_permissions_persist_in_sys_catalog() {
    let engine = Engine::new();
    run(
        &engine,
        "CREATE SCHEMA app;
         CREATE TABLE app.Orders (Id INT PRIMARY KEY);
         CREATE USER app_reader WITHOUT LOGIN WITH DEFAULT_SCHEMA = app;
         CREATE ROLE app_read_role;
         ALTER ROLE app_read_role ADD MEMBER app_reader;
         GRANT SELECT, INSERT ON OBJECT::app.Orders TO app_read_role WITH GRANT OPTION;
         DENY DELETE ON SCHEMA::app TO app_reader;",
    )
    .expect("create security objects");

    // sys.database_principals
    let principals = query(
        &engine,
        "SELECT name, type, type_desc, default_schema_name
         FROM sys.database_principals
         WHERE name IN ('app_reader', 'app_read_role')
         ORDER BY name;",
    );
    assert_eq!(principals.rows.len(), 2);
    assert_eq!(
        principals.rows[0][0],
        Value::VarChar("app_read_role".into())
    );
    assert_eq!(principals.rows[0][1], Value::Char("R".into()));
    assert_eq!(principals.rows[1][0], Value::VarChar("app_reader".into()));
    assert_eq!(principals.rows[1][1], Value::Char("S".into()));
    assert_eq!(principals.rows[1][3], Value::VarChar("app".into()));

    // sys.database_role_members
    let members = query(
        &engine,
        "SELECT r.name AS role_name, m.name AS member_name
         FROM sys.database_role_members rm
         JOIN sys.database_principals r ON r.principal_id = rm.role_principal_id
         JOIN sys.database_principals m ON m.principal_id = rm.member_principal_id;",
    );
    assert_eq!(members.rows.len(), 1);
    assert_eq!(members.rows[0][0], Value::VarChar("app_read_role".into()));
    assert_eq!(members.rows[0][1], Value::VarChar("app_reader".into()));

    // sys.database_permissions
    let perms = query(
        &engine,
        "SELECT class_desc, permission_name, state, state_desc
         FROM sys.database_permissions
         ORDER BY permission_name;",
    );
    assert_eq!(perms.rows.len(), 3);
    assert_eq!(perms.rows[0][0], Value::VarChar("SCHEMA".into()));
    assert_eq!(perms.rows[0][1], Value::VarChar("DELETE".into()));
    assert_eq!(perms.rows[0][2], Value::Char("D".into()));
    assert_eq!(perms.rows[1][1], Value::VarChar("INSERT".into()));
    assert_eq!(perms.rows[1][2], Value::Char("W".into()));
    assert_eq!(perms.rows[2][1], Value::VarChar("SELECT".into()));

    // REVOKE removes permission; ALTER ROLE DROP MEMBER removes membership
    run(
        &engine,
        "REVOKE INSERT ON OBJECT::app.Orders FROM app_read_role;
         ALTER ROLE app_read_role DROP MEMBER app_reader;",
    )
    .expect("revoke and drop member");

    assert_eq!(
        query(&engine, "SELECT COUNT(*) FROM sys.database_permissions;").rows[0][0],
        Value::BigInt(2)
    );
    assert_eq!(
        query(&engine, "SELECT COUNT(*) FROM sys.database_role_members;").rows[0][0],
        Value::BigInt(0)
    );

    // DROP USER / DROP ROLE
    run(
        &engine,
        "DROP USER IF EXISTS app_reader; DROP ROLE IF EXISTS app_read_role;",
    )
    .expect("drop principals");
    let left = query(
        &engine,
        "SELECT name FROM sys.database_principals WHERE name IN ('app_reader', 'app_read_role');",
    );
    assert!(left.rows.is_empty());
}

#[test]
fn partition_functions_schemes_filegroups_and_tableoption_persist_in_sys_catalog() {
    let engine = Engine::new();
    run(
        &engine,
        "ALTER DATABASE TestDb ADD FILEGROUP FG_2025;
         ALTER DATABASE TestDb ADD FILEGROUP FG_2026;
         CREATE PARTITION FUNCTION pf_Year (INT) AS RANGE RIGHT FOR VALUES (2025, 2026);
         CREATE PARTITION SCHEME ps_Year AS PARTITION pf_Year TO ([PRIMARY], FG_2025, FG_2026);
         CREATE TABLE dbo.BigDocs (Id INT PRIMARY KEY, Body VARCHAR(MAX));
         EXEC sys.sp_tableoption N'dbo.BigDocs', N'large value types out of row', 1;",
    )
    .expect("create storage and partition objects");

    // sys.filegroups & sys.data_spaces
    let fgs = query(
        &engine,
        "SELECT name FROM sys.filegroups ORDER BY data_space_id;",
    );
    assert_eq!(
        fgs.rows,
        vec![
            vec![Value::VarChar("PRIMARY".into())],
            vec![Value::VarChar("FG_2025".into())],
            vec![Value::VarChar("FG_2026".into())],
        ]
    );

    // sys.partition_functions, partition_parameters, partition_range_values
    let pfs = query(
        &engine,
        "SELECT name, fanout, boundary_value_on_right FROM sys.partition_functions;",
    );
    assert_eq!(pfs.rows.len(), 1);
    assert_eq!(pfs.rows[0][0], Value::VarChar("pf_Year".into()));
    assert_eq!(pfs.rows[0][1], Value::Int(3));
    assert_eq!(pfs.rows[0][2], Value::Bit(true));

    let pparams = query(
        &engine,
        "SELECT parameter_id, system_type_id FROM sys.partition_parameters;",
    );
    assert_eq!(pparams.rows.len(), 1);
    assert_eq!(pparams.rows[0][1], Value::Int(56)); // INT

    let pranges = query(
        &engine,
        "SELECT boundary_id, value FROM sys.partition_range_values ORDER BY boundary_id;",
    );
    assert_eq!(pranges.rows.len(), 2);
    assert_eq!(pranges.rows[0][1].to_string_value(), "2025");
    assert_eq!(pranges.rows[1][1].to_string_value(), "2026");

    // sys.partition_schemes & sys.destination_data_spaces
    let pschemes = query(&engine, "SELECT name, type FROM sys.partition_schemes;");
    assert_eq!(pschemes.rows.len(), 1);
    assert_eq!(pschemes.rows[0][0], Value::VarChar("ps_Year".into()));

    let dds = query(
        &engine,
        "SELECT d.destination_id, fg.name
         FROM sys.destination_data_spaces d
         JOIN sys.filegroups fg ON fg.data_space_id = d.data_space_id
         ORDER BY d.destination_id;",
    );
    assert_eq!(dds.rows.len(), 3);
    assert_eq!(dds.rows[0][1], Value::VarChar("PRIMARY".into()));
    assert_eq!(dds.rows[1][1], Value::VarChar("FG_2025".into()));
    assert_eq!(dds.rows[2][1], Value::VarChar("FG_2026".into()));

    // sys.tables.large_value_types_out_of_row persisted by sp_tableoption
    let tbl_opt = query(
        &engine,
        "SELECT large_value_types_out_of_row FROM sys.tables WHERE name = 'BigDocs';",
    );
    assert_eq!(tbl_opt.rows[0][0], Value::Bit(true));

    // Drop partition scheme & function
    run(
        &engine,
        "DROP PARTITION SCHEME ps_Year; DROP PARTITION FUNCTION pf_Year;",
    )
    .expect("drop partition scheme and function");
    assert!(query(&engine, "SELECT name FROM sys.partition_schemes;")
        .rows
        .is_empty());
    assert!(query(&engine, "SELECT name FROM sys.partition_functions;")
        .rows
        .is_empty());
}
