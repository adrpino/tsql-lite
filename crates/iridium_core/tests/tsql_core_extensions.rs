use iridium_core::{Database, Engine, QueryResult, SessionManager, StatementExecutor};

fn exec(engine: &mut Engine, sql: &str) {
    let stmts = iridium_core::parse_batch(sql)
        .unwrap_or_else(|e| panic!("parse failed: {sql}\nerror: {e}"));
    engine
        .execute_batch(stmts)
        .unwrap_or_else(|e| panic!("SQL failed: {sql}\nerror: {e}"));
}

fn query(engine: &mut Engine, sql: &str) -> QueryResult {
    let stmts = iridium_core::parse_batch(sql)
        .unwrap_or_else(|e| panic!("parse failed: {sql}\nerror: {e}"));
    engine
        .execute_batch(stmts)
        .unwrap_or_else(|e| panic!("SQL failed: {sql}\nerror: {e}"))
        .unwrap_or_default()
}

#[test]
fn for_json_path_and_auto() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE TABLE dbo.items (id INT, label NVARCHAR(50), price DECIMAL(10, 2), active BIT, note NVARCHAR(50) NULL);
         INSERT INTO dbo.items VALUES (1, N'Alpha', 12.50, 1, NULL), (2, N'Beta', 7.00, 0, N'ok');",
    );

    // 1. FOR JSON PATH default (omits NULLs, wraps in array)
    let r1 = query(
        &mut e,
        "SELECT id, label, active, note FROM dbo.items ORDER BY id FOR JSON PATH",
    );
    let parsed1: serde_json::Value =
        serde_json::from_str(&r1.rows[0][0].to_string_value()).expect("valid JSON");
    assert_eq!(
        parsed1,
        serde_json::json!([
            {"id": 1, "label": "Alpha", "active": true},
            {"id": 2, "label": "Beta", "active": false, "note": "ok"}
        ])
    );

    // 2. FOR JSON PATH with INCLUDE_NULL_VALUES and ROOT('items')
    let r2 = query(
        &mut e,
        "SELECT id, note FROM dbo.items ORDER BY id FOR JSON PATH, ROOT('items'), INCLUDE_NULL_VALUES",
    );
    let parsed2: serde_json::Value =
        serde_json::from_str(&r2.rows[0][0].to_string_value()).expect("valid JSON");
    assert_eq!(
        parsed2,
        serde_json::json!({
            "items": [
                {"id": 1, "note": null},
                {"id": 2, "note": "ok"}
            ]
        })
    );

    // 3. FOR JSON PATH with dotted nested aliases + WITHOUT_ARRAY_WRAPPER
    let r3 = query(
        &mut e,
        "SELECT id, label AS [meta.label], active AS [meta.active]
         FROM dbo.items WHERE id = 1
         FOR JSON PATH, WITHOUT_ARRAY_WRAPPER",
    );
    let parsed3: serde_json::Value =
        serde_json::from_str(&r3.rows[0][0].to_string_value()).expect("valid JSON");
    assert_eq!(
        parsed3,
        serde_json::json!({"id": 1, "meta": {"label": "Alpha", "active": true}})
    );

    // 4. FOR JSON AUTO + scalar subquery usage
    let r4 = query(
        &mut e,
        "SELECT (SELECT id, label FROM dbo.items WHERE id = 2 FOR JSON AUTO, WITHOUT_ARRAY_WRAPPER) AS payload",
    );
    let parsed4: serde_json::Value =
        serde_json::from_str(&r4.rows[0][0].to_string_value()).expect("valid JSON");
    assert_eq!(parsed4, serde_json::json!({"id": 2, "label": "Beta"}));

    // 5. Empty result set: 0 rows without ROOT, empty array with ROOT
    let r5_empty = query(
        &mut e,
        "SELECT id FROM dbo.items WHERE id = 999 FOR JSON PATH",
    );
    assert!(r5_empty.rows.is_empty());

    let r5_root = query(
        &mut e,
        "SELECT id FROM dbo.items WHERE id = 999 FOR JSON PATH, ROOT('items')",
    );
    let parsed5: serde_json::Value =
        serde_json::from_str(&r5_root.rows[0][0].to_string_value()).expect("valid JSON");
    assert_eq!(parsed5, serde_json::json!({"items": []}));
}

#[test]
fn xml_exist_query_and_nodes_shredding() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE TABLE dbo.xml_docs (doc_id INT, payload XML);
         INSERT INTO dbo.xml_docs VALUES
           (1, N'<order><item id=\"10\"><name>Bolt</name><qty>3</qty></item><item id=\"20\"><name>Nut</name><qty>7</qty></item></order>'),
           (2, N'<order></order>'),
           (3, NULL);",
    );

    // 1. Scalar .exist() on column and variable
    let r_exist = query(
        &mut e,
        "SELECT doc_id,
                payload.exist('/order/item') AS has_item,
                payload.exist('/order/item[@id=\"20\"]') AS has_20,
                payload.exist('/order/item[@id=\"99\"]') AS has_99
         FROM dbo.xml_docs ORDER BY doc_id",
    );
    assert_eq!(r_exist.rows[0][1].to_bool(), Some(true));
    assert_eq!(r_exist.rows[0][2].to_bool(), Some(true));
    assert_eq!(r_exist.rows[0][3].to_bool(), Some(false));
    assert_eq!(r_exist.rows[1][1].to_bool(), Some(false));
    assert!(r_exist.rows[2][1].is_null());

    // 2. Scalar .query() returning XML fragment
    let r_query = query(
        &mut e,
        "SELECT payload.query('/order/item[1]/name') AS first_name_xml
         FROM dbo.xml_docs WHERE doc_id = 1",
    );
    assert_eq!(r_query.rows[0][0].to_string_value(), "<name>Bolt</name>");

    // 3. CROSS APPLY .nodes() shredding column into rows with .value() and .exist()
    let r_nodes = query(
        &mut e,
        "SELECT d.doc_id,
                x.c.value('@id', 'INT') AS item_id,
                x.c.value('(name/text())[1]', 'NVARCHAR(50)') AS item_name,
                x.c.value('(qty/text())[1]', 'INT') AS qty
         FROM dbo.xml_docs d
         CROSS APPLY d.payload.nodes('/order/item') AS x(c)
         ORDER BY d.doc_id, item_id",
    );
    assert_eq!(r_nodes.rows.len(), 2);
    assert_eq!(r_nodes.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r_nodes.rows[0][1].to_integer_i64(), Some(10));
    assert_eq!(r_nodes.rows[0][2].to_string_value(), "Bolt");
    assert_eq!(r_nodes.rows[0][3].to_integer_i64(), Some(3));
    assert_eq!(r_nodes.rows[1][1].to_integer_i64(), Some(20));
    assert_eq!(r_nodes.rows[1][2].to_string_value(), "Nut");
    assert_eq!(r_nodes.rows[1][3].to_integer_i64(), Some(7));

    // 4. OUTER APPLY .nodes() preserves unmatched rows with NULL
    let r_outer = query(
        &mut e,
        "SELECT d.doc_id, x.c.value('@id', 'INT') AS item_id
         FROM dbo.xml_docs d
         OUTER APPLY d.payload.nodes('/order/item') AS x(c)
         ORDER BY d.doc_id, item_id",
    );
    assert_eq!(r_outer.rows.len(), 4);
    assert_eq!(r_outer.rows[2][0].to_integer_i64(), Some(2));
    assert!(r_outer.rows[2][1].is_null());
    assert_eq!(r_outer.rows[3][0].to_integer_i64(), Some(3));
    assert!(r_outer.rows[3][1].is_null());

    // 5. Variable @doc.nodes(...) in FROM / CROSS APPLY
    let r_var = query(
        &mut e,
        "DECLARE @x XML = N'<root><v>100</v><v>200</v></root>';
         SELECT SUM(t.n.value('.', 'INT')) AS total
         FROM @x.nodes('/root/v') AS t(n);",
    );
    assert_eq!(r_var.rows[0][0].to_integer_i64(), Some(300));
}

#[test]
fn goto_labels_forward_backward_and_nested() {
    let mut e = Engine::new();

    // 1. Forward jump in a top-level batch (skips middle assignment)
    let r1 = query(
        &mut e,
        "DECLARE @v INT = 1;
         GOTO Done;
         SET @v = 999;
         Done:
         SELECT @v AS v;",
    );
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(1));

    // 2. Backward jump loop + jumping out of nested IF/WHILE in a stored procedure
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.usp_goto_demo @limit INT
         AS
         BEGIN
             DECLARE @i INT = 0, @sum INT = 0;
             LoopStart:
             SET @i = @i + 1;
             IF @i > @limit
             BEGIN
                 GOTO ExitProc;
             END
             SET @sum = @sum + @i;
             GOTO LoopStart;
             SET @sum = -1;
             ExitProc:
             SELECT @sum AS total;
         END;",
    );
    let r2 = query(&mut e, "EXEC dbo.usp_goto_demo @limit = 5;");
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(15));

    // 3. Undefined label produces an error
    let stmts = iridium_core::parse_batch("GOTO MissingLabel;").expect("should parse");
    let err = e
        .execute_batch(stmts)
        .expect_err("undefined label must fail");
    assert!(
        err.to_string().to_lowercase().contains("missinglabel")
            || err.to_string().to_lowercase().contains("label"),
        "unexpected error: {err}"
    );
}

#[test]
fn grouping_sets_rollup_cube_and_grouping_functions() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE TABLE dbo.sales (region NVARCHAR(20), dept NVARCHAR(20), amount INT);
         INSERT INTO dbo.sales VALUES
           (N'East', N'A', 10),
           (N'East', N'B', 20),
           (N'West', N'A', 30);",
    );

    // 1. ROLLUP(region, dept) -> detail (3) + region subtotals (2) + grand total (1) = 6 rows
    let r_rollup = query(
        &mut e,
        "SELECT region, dept, SUM(amount) AS total,
                GROUPING(region) AS g_reg,
                GROUPING(dept) AS g_dept,
                GROUPING_ID(region, dept) AS gid
         FROM dbo.sales
         GROUP BY ROLLUP(region, dept)
         ORDER BY gid, region, dept",
    );
    assert_eq!(r_rollup.rows.len(), 6);
    // Detail row East/A: gid = 0
    assert_eq!(r_rollup.rows[0][0].to_string_value(), "East");
    assert_eq!(r_rollup.rows[0][1].to_string_value(), "A");
    assert_eq!(r_rollup.rows[0][2].to_integer_i64(), Some(10));
    assert_eq!(r_rollup.rows[0][5].to_integer_i64(), Some(0));
    // Region subtotal East/NULL: gid = 1 (g_reg=0, g_dept=1)
    assert_eq!(r_rollup.rows[3][0].to_string_value(), "East");
    assert!(r_rollup.rows[3][1].is_null());
    assert_eq!(r_rollup.rows[3][2].to_integer_i64(), Some(30));
    assert_eq!(r_rollup.rows[3][3].to_integer_i64(), Some(0));
    assert_eq!(r_rollup.rows[3][4].to_integer_i64(), Some(1));
    assert_eq!(r_rollup.rows[3][5].to_integer_i64(), Some(1));
    // Grand total NULL/NULL: gid = 3 (g_reg=1, g_dept=1)
    assert!(r_rollup.rows[5][0].is_null());
    assert!(r_rollup.rows[5][1].is_null());
    assert_eq!(r_rollup.rows[5][2].to_integer_i64(), Some(60));
    assert_eq!(r_rollup.rows[5][5].to_integer_i64(), Some(3));

    // 2. CUBE(region, dept) -> detail (3) + by region (2) + by dept (2) + grand total (1) = 8 rows
    let r_cube = query(
        &mut e,
        "SELECT region, dept, SUM(amount) AS total, GROUPING_ID(region, dept) AS gid
         FROM dbo.sales
         GROUP BY CUBE(region, dept)",
    );
    assert_eq!(r_cube.rows.len(), 8);

    // 3. Explicit GROUPING SETS ((region), (dept), ()) -> 2 + 2 + 1 = 5 rows
    let r_sets = query(
        &mut e,
        "SELECT region, dept, SUM(amount) AS total
         FROM dbo.sales
         GROUP BY GROUPING SETS ((region), (dept), ())",
    );
    assert_eq!(r_sets.rows.len(), 5);
}

#[test]
fn at_time_zone_dst_switchoffset_and_todatetimeoffset() {
    let mut e = Engine::new();

    // 1. Winter vs Summer DST for Eastern Standard Time (UTC-5 winter, UTC-4 summer)
    let r_winter = query(
        &mut e,
        "SELECT CAST('2025-01-15 17:00:00' AS DATETIME2) AT TIME ZONE 'UTC' AT TIME ZONE 'Eastern Standard Time'",
    );
    assert_eq!(
        r_winter.rows[0][0].to_string_value(),
        "2025-01-15 12:00:00 -05:00"
    );

    let r_summer = query(
        &mut e,
        "SELECT CAST('2025-07-15 16:00:00' AS DATETIME2) AT TIME ZONE 'UTC' AT TIME ZONE 'Eastern Standard Time'",
    );
    assert_eq!(
        r_summer.rows[0][0].to_string_value(),
        "2025-07-15 12:00:00 -04:00"
    );

    // 2. EU / UK DST (GMT Standard Time -> +01:00 in July, +00:00 in Jan) & Asia/Tokyo (+09:00 no DST)
    let r_london_summer = query(
        &mut e,
        "SELECT CAST('2025-07-15 12:00:00' AS DATETIME2) AT TIME ZONE 'GMT Standard Time'",
    );
    assert_eq!(
        r_london_summer.rows[0][0].to_string_value(),
        "2025-07-15 12:00:00 +01:00"
    );

    let r_tokyo = query(
        &mut e,
        "SELECT CAST('2025-01-15 00:00:00' AS DATETIME2) AT TIME ZONE 'UTC' AT TIME ZONE 'Tokyo Standard Time'",
    );
    assert_eq!(
        r_tokyo.rows[0][0].to_string_value(),
        "2025-01-15 09:00:00 +09:00"
    );

    // 3. Unknown timezone name raises an error
    let bad = iridium_core::parse_batch(
        "SELECT CAST('2025-01-15 12:00:00' AS DATETIME2) AT TIME ZONE 'Mars/Olympus'",
    )
    .expect("should parse");
    assert!(e.execute_batch(bad).is_err());

    // 4. TODATETIMEOFFSET and SWITCHOFFSET
    let r_to = query(
        &mut e,
        "SELECT TODATETIMEOFFSET(CAST('2025-03-01 10:30:00' AS DATETIME2), '-05:00'),
                TODATETIMEOFFSET(CAST('2025-03-01 10:30:00' AS DATETIME2), 120)",
    );
    assert_eq!(
        r_to.rows[0][0].to_string_value(),
        "2025-03-01 10:30:00 -05:00"
    );
    assert_eq!(
        r_to.rows[0][1].to_string_value(),
        "2025-03-01 10:30:00 +02:00"
    );

    let r_sw = query(
        &mut e,
        "SELECT SWITCHOFFSET(TODATETIMEOFFSET(CAST('2025-03-01 10:30:00' AS DATETIME2), '-05:00'), '+02:00')",
    );
    assert_eq!(
        r_sw.rows[0][0].to_string_value(),
        "2025-03-01 17:30:00 +02:00"
    );
}

#[test]
fn collate_case_sensitive_like_in_order_by_and_group_by() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE TABLE dbo.codes (code NVARCHAR(20));
         INSERT INTO dbo.codes VALUES (N'abc'), (N'ABC'), (N'Abc'), (N'abc');",
    );

    // 1. Case-sensitive LIKE vs default case-insensitive LIKE
    let r_like = query(
        &mut e,
        "SELECT
            CASE WHEN N'abc' COLLATE Latin1_General_CS_AS LIKE N'A%' THEN 1 ELSE 0 END AS cs_miss,
            CASE WHEN N'Abc' COLLATE Latin1_General_CS_AS LIKE N'A%' THEN 1 ELSE 0 END AS cs_hit,
            CASE WHEN N'abc' COLLATE Latin1_General_CI_AS LIKE N'A%' THEN 1 ELSE 0 END AS ci_hit",
    );
    assert_eq!(r_like.rows[0][0].to_integer_i64(), Some(0));
    assert_eq!(r_like.rows[0][1].to_integer_i64(), Some(1));
    assert_eq!(r_like.rows[0][2].to_integer_i64(), Some(1));

    // 2. Case-sensitive IN (...)
    let r_in = query(
        &mut e,
        "SELECT
            CASE WHEN N'abc' COLLATE Latin1_General_CS_AS IN (N'ABC', N'Abc') THEN 1 ELSE 0 END AS cs_miss,
            CASE WHEN N'abc' COLLATE Latin1_General_CS_AS IN (N'ABC', N'abc') THEN 1 ELSE 0 END AS cs_hit",
    );
    assert_eq!(r_in.rows[0][0].to_integer_i64(), Some(0));
    assert_eq!(r_in.rows[0][1].to_integer_i64(), Some(1));

    // 3. Case-sensitive ORDER BY (... COLLATE Latin1_General_BIN: ASCII 'A' (65) < 'a' (97))
    let r_ord = query(
        &mut e,
        "SELECT code FROM dbo.codes ORDER BY code COLLATE Latin1_General_BIN ASC",
    );
    let ordered: Vec<String> = r_ord.rows.iter().map(|r| r[0].to_string_value()).collect();
    assert_eq!(ordered, vec!["ABC", "Abc", "abc", "abc"]);

    // 4. Case-sensitive GROUP BY (... COLLATE Latin1_General_CS_AS separates 'abc', 'ABC', 'Abc')
    let r_grp = query(
        &mut e,
        "SELECT code COLLATE Latin1_General_CS_AS AS c, COUNT(*) AS cnt
         FROM dbo.codes
         GROUP BY code COLLATE Latin1_General_CS_AS
         ORDER BY c COLLATE Latin1_General_BIN ASC",
    );
    assert_eq!(r_grp.rows.len(), 3);
    assert_eq!(r_grp.rows[0][0].to_string_value(), "ABC");
    assert_eq!(r_grp.rows[0][1].to_integer_i64(), Some(1));
    assert_eq!(r_grp.rows[1][0].to_string_value(), "Abc");
    assert_eq!(r_grp.rows[1][1].to_integer_i64(), Some(1));
    assert_eq!(r_grp.rows[2][0].to_string_value(), "abc");
    assert_eq!(r_grp.rows[2][1].to_integer_i64(), Some(2));
}

#[test]
fn applock_acquire_and_release_return_status() {
    let mut e = Engine::new();
    let r = query(
        &mut e,
        "DECLARE @rc1 INT, @rc2 INT;
         EXEC @rc1 = sys.sp_getapplock @Resource = N'deploy_mutex', @LockMode = 'Exclusive', @LockOwner = 'Session', @LockTimeout = 5000;
         EXEC @rc2 = sys.sp_releaseapplock @Resource = N'deploy_mutex', @LockOwner = 'Session';
         SELECT @rc1 AS acq, @rc2 AS rel;",
    );
    assert_eq!(r.rows[0][0].to_integer_i64(), Some(0));
    assert_eq!(r.rows[0][1].to_integer_i64(), Some(0));
}

#[test]
fn create_table_storage_compression_options_and_temporal() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE TABLE [dbo].[Orders] (
            [Id] INT IDENTITY(1,1) NOT FOR REPLICATION NOT NULL,
            [RowGuid] UNIQUEIDENTIFIER ROWGUIDCOL NOT NULL,
            [Code] NVARCHAR(50) SPARSE NULL,
            [Notes] NVARCHAR(MAX) NULL,
            CONSTRAINT [PK_Orders] PRIMARY KEY CLUSTERED ([Id] ASC)
                WITH (PAD_INDEX = OFF, FILLFACTOR = 80, DATA_COMPRESSION = PAGE) ON [PRIMARY],
            CONSTRAINT [UQ_Orders_Code] UNIQUE NONCLUSTERED ([Code] ASC)
                WITH (FILLFACTOR = 90) ON [PRIMARY]
        ) ON [PRIMARY] TEXTIMAGE_ON [PRIMARY] WITH (DATA_COMPRESSION = PAGE);",
    );
    exec(
        &mut e,
        "CREATE TABLE [dbo].[AuditLog] (
            [Id] INT NOT NULL PRIMARY KEY CLUSTERED WITH (DATA_COMPRESSION = ROW) ON [PRIMARY],
            [Payload] NVARCHAR(100) NULL,
            [ValidFrom] DATETIME2 GENERATED ALWAYS AS ROW START HIDDEN NOT NULL,
            [ValidTo] DATETIME2 GENERATED ALWAYS AS ROW END HIDDEN NOT NULL,
            PERIOD FOR SYSTEM_TIME ([ValidFrom], [ValidTo])
        ) WITH (SYSTEM_VERSIONING = ON (HISTORY_TABLE = [dbo].[AuditLogHistory]));",
    );
    exec(
        &mut e,
        "CREATE NONCLUSTERED INDEX [IX_Orders_Code] ON [dbo].[Orders] ([Code] ASC)
            WITH (FILLFACTOR = 85, DATA_COMPRESSION = PAGE) ON [ps_monthly]([Code]);",
    );
    exec(
        &mut e,
        "INSERT INTO [dbo].[Orders] ([RowGuid], [Code], [Notes]) VALUES ('00000000-0000-0000-0000-000000000001', N'ORD-1', N'ok');",
    );
    let r = query(&mut e, "SELECT [Code], [Notes] FROM [dbo].[Orders]");
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0].to_string_value(), "ORD-1");
}

#[test]
fn extended_properties_and_tableoption_system_procedures() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE TABLE [dbo].[Items] ([Id] INT NOT NULL PRIMARY KEY);",
    );
    exec(
        &mut e,
        "EXEC sys.sp_addextendedproperty
            @name = N'MS_Description',
            @value = N'Item catalog',
            @level0type = N'SCHEMA', @level0name = N'dbo',
            @level1type = N'TABLE', @level1name = N'Items';
         EXEC sys.sp_updateextendedproperty
            @name = N'MS_Description',
            @value = N'Updated item catalog',
            @level0type = N'SCHEMA', @level0name = N'dbo',
            @level1type = N'TABLE', @level1name = N'Items';
         EXEC sys.sp_dropextendedproperty
            @name = N'MS_Description',
            @level0type = N'SCHEMA', @level0name = N'dbo',
            @level1type = N'TABLE', @level1name = N'Items';
         EXEC sys.sp_tableoption N'[dbo].[Items]', N'large value types out of row', N'1';",
    );
}

#[test]
fn admin_storage_security_ddl_statements() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE SCHEMA [reporting] AUTHORIZATION [dbo];
         CREATE TYPE [dbo].[CodeAlias] FROM VARCHAR(32) NOT NULL;
         CREATE TABLE [reporting].[Sales] ([Id] INT NOT NULL, [Code] VARCHAR(32) NULL);
         CREATE PARTITION FUNCTION [pf_year] (INT) AS RANGE RIGHT FOR VALUES (2024, 2025, 2026);
         CREATE PARTITION SCHEME [ps_year] AS PARTITION [pf_year] ALL TO ([PRIMARY]);
         CREATE LOGIN [app_login] WITH PASSWORD = N'StrongP@ss1!';
         CREATE USER [app_user] FOR LOGIN [app_login] WITH DEFAULT_SCHEMA = [dbo];
         CREATE ROLE [app_reader] AUTHORIZATION [dbo];
         ALTER ROLE [app_reader] ADD MEMBER [app_user];
         ALTER ROLE [app_reader] DROP MEMBER [app_user];
         CREATE FULLTEXT CATALOG [ft_main] AS DEFAULT;
         CREATE STATISTICS [st_sales_code] ON [reporting].[Sales]([Code]) WITH FULLSCAN;
         CREATE MASTER KEY ENCRYPTION BY PASSWORD = N'MasterKey!123';
         CREATE CERTIFICATE [cert_app] WITH SUBJECT = N'App Cert';
         CREATE SYMMETRIC KEY [sym_app] WITH ALGORITHM = AES_256 ENCRYPTION BY CERTIFICATE [cert_app];
         ALTER DATABASE [master] ADD FILEGROUP [FG_DATA];
         ALTER DATABASE [master] ADD FILE (NAME = N'fg_data_1', FILENAME = N'/tmp/fg_data_1.ndf') TO FILEGROUP [FG_DATA];",
    );
}

#[test]
fn catalog_views_principals_security_policies_and_nested_for_json() {
    let mut e = Engine::new();
    // 1. sys.database_principals columns
    let _ = query(
        &mut e,
        "SELECT dp.principal_id, dp.name, dp.type_desc, dp.sid, dp.authentication_type,
                dp.owning_principal_id, dp.is_fixed_role, dp.default_schema_name
         FROM sys.database_principals dp
         WHERE dp.is_fixed_role = 0",
    );
    // 2. sys.server_principals columns (sid, default_language_name)
    let _ = query(
        &mut e,
        "SELECT sp.principal_id, sp.name, sp.sid, sp.default_database_name, sp.default_language_name
         FROM sys.server_principals sp",
    );
    // 3. sys.database_permissions.class_desc
    let _ = query(
        &mut e,
        "SELECT p.class, p.class_desc, p.major_id, p.permission_name, p.state_desc
         FROM sys.database_permissions p",
    );
    // 4. sys.security_policies & sys.security_predicates
    let _ = query(
        &mut e,
        "SELECT s.name AS schema_name, sp.name AS policy_name, sp.is_enabled,
                (SELECT pr.predicate_type_desc, pr.operation_desc, pr.predicate_definition
                 FROM sys.security_predicates pr
                 WHERE pr.object_id = sp.object_id
                 FOR JSON PATH) AS predicates
         FROM sys.security_policies sp
         JOIN sys.schemas s ON sp.schema_id = s.schema_id
         FOR JSON PATH",
    );
    // 5. sys.extended_properties (class_desc, value), sys.tables.large_value_types_out_of_row,
    //    sys.masked_columns, and sys.partition_range_values
    let _ = query(
        &mut e,
        "SELECT ep.class, ep.class_desc, ep.major_id, ep.minor_id, ep.name, ep.value
         FROM sys.extended_properties ep",
    );
    let _ = query(
        &mut e,
        "SELECT t.name, t.large_value_types_out_of_row FROM sys.tables t",
    );
    let _ = query(
        &mut e,
        "SELECT mc.object_id, mc.column_id, mc.is_masked, mc.masking_function
         FROM sys.masked_columns mc",
    );
    let _ = query(
        &mut e,
        "SELECT prv.function_id, prv.boundary_id, prv.parameter_id, prv.value
         FROM sys.partition_range_values prv",
    );
    // 6. Nested FOR JSON PATH embeds raw JSON array, not double-escaped string
    exec(
        &mut e,
        "CREATE TABLE dbo.depts (id INT, name NVARCHAR(50));
         CREATE TABLE dbo.emps (dept_id INT, emp_name NVARCHAR(50));
         INSERT INTO dbo.depts VALUES (1, N'Eng');
         INSERT INTO dbo.emps VALUES (1, N'Alice'), (1, N'Bob');",
    );
    let r = query(
        &mut e,
        "SELECT d.name,
                (SELECT e.emp_name FROM dbo.emps e WHERE e.dept_id = d.id ORDER BY e.emp_name FOR JSON PATH) AS members
         FROM dbo.depts d
         ORDER BY d.id
         FOR JSON PATH",
    );
    let json_text = r.rows[0][0].to_string_value();
    let parsed: serde_json::Value = serde_json::from_str(&json_text).expect("valid JSON");
    assert!(
        parsed[0]["members"].is_array(),
        "nested FOR JSON PATH must embed raw JSON array, got: {}",
        json_text
    );
    assert_eq!(parsed[0]["members"][0]["emp_name"], "Alice");
    assert_eq!(parsed[0]["members"][1]["emp_name"], "Bob");

    // 7. Catalog emptiness probe with Bit = 0 and NOT EXISTS against sys.extended_properties
    let probe = query(
        &mut e,
        "SELECT
            (SELECT COUNT(1) FROM sys.objects o
             WHERE o.is_ms_shipped = 0
               AND NOT EXISTS (
                   SELECT 1 FROM sys.extended_properties ep
                   WHERE ep.major_id = o.object_id
                     AND ep.minor_id = 0
                     AND ep.name = N'microsoft_database_tools_support'
               )) AS user_objects,
            (SELECT COUNT(1) FROM sys.table_types WHERE is_user_defined = 1) AS user_table_types,
            (SELECT COUNT(1) FROM sys.schemas WHERE schema_id > 4 AND schema_id < 16384) AS custom_schemas
         FOR JSON PATH, WITHOUT_ARRAY_WRAPPER",
    );
    let probe_json: serde_json::Value =
        serde_json::from_str(&probe.rows[0][0].to_string_value()).expect("valid probe JSON");
    assert!(
        probe_json["user_objects"].as_u64().unwrap_or(0) >= 2,
        "expected >= 2 user objects (depts, emps), got: {}",
        probe_json
    );
}

#[test]
fn not_operator_precedence_and_alter_table_multi_add_and_set() {
    let mut e = Engine::new();
    // SQL precedence: NOT binds tighter than AND / OR.
    // NOT (1 = 2) AND (1 = 2) => TRUE AND FALSE => FALSE (0)
    let r = query(
        &mut e,
        "SELECT CASE WHEN NOT 1 = 2 AND 1 = 2 THEN 1 ELSE 0 END AS val",
    );
    assert_eq!(r.rows[0][0].to_integer_i64(), Some(0));

    // Chained NOT ... LIKE ... AND NOT ... LIKE ... must stay flat (constant depth).
    exec(
        &mut e,
        "CREATE TABLE [dbo].[Users] (
            [Id] INT NOT NULL PRIMARY KEY,
            [Secret] NVARCHAR(50) COLLATE Latin1_General_BIN2
                ENCRYPTED WITH (COLUMN_ENCRYPTION_KEY = [CEK_1], ALGORITHM = N'AEAD_AES_256_CBC_HMAC_SHA_256', ENCRYPTION_TYPE = DETERMINISTIC) NOT NULL,
            [Code] NVARCHAR(50) NOT NULL
        );
        CREATE TABLE [dbo].[UsersAudit] ([Id] INT NOT NULL);
        ALTER TABLE [dbo].[Users] ADD UNIQUE NONCLUSTERED ([Code]) ON [PRIMARY];
        ALTER TABLE [dbo].[Users] ADD
            CONSTRAINT [CHK_Users_1] CHECK ([Id] > 0),
            CONSTRAINT [CHK_Users_2] CHECK (
                NOT [Code] LIKE '%a%' AND NOT [Code] LIKE '%b%' AND NOT [Code] LIKE '%c%'
                AND NOT [Code] LIKE '%d%' AND NOT [Code] LIKE '%e%' AND NOT [Code] LIKE '%f%'
                AND NOT [Code] LIKE '%g%' AND NOT [Code] LIKE '%h%' AND NOT [Code] LIKE '%i%'
                AND NOT [Code] LIKE '%j%' AND NOT [Code] LIKE '%k%' AND NOT [Code] LIKE '%l%'
                AND NOT [Code] LIKE '%m%' AND NOT [Code] LIKE '%n%' AND NOT [Code] LIKE '%o%'
                AND NOT [Code] LIKE '%p%' AND NOT [Code] LIKE '%q%' AND NOT [Code] LIKE '%r%'
            );
        ALTER TABLE [dbo].[Users] SET (SYSTEM_VERSIONING = ON (HISTORY_TABLE = [dbo].[UsersAudit]));",
    );
}

#[test]
fn conditional_commit_and_rollback_inside_if_persist_across_sessions() {
    let db = Database::new();

    // Session 1: BEGIN TRAN -> DDL + DML -> IF @@TRANCOUNT > 0 COMMIT TRANSACTION;
    let s1 = db.create_session();
    db.execute_session_batch_sql_multi(
        s1,
        "SET XACT_ABORT ON;
         BEGIN TRANSACTION;
         CREATE TABLE dbo.TxPersist (Id INT NOT NULL PRIMARY KEY);
         INSERT INTO dbo.TxPersist VALUES (123);
         IF @@TRANCOUNT > 0 COMMIT TRANSACTION;",
    )
    .expect("session 1 commit batch must succeed");

    // Verify @@TRANCOUNT returned to 0 in session 1 before disconnect
    let tc = db
        .execute_session_batch_sql(s1, "SELECT @@TRANCOUNT AS tc")
        .expect("trancount query")
        .expect("result");
    assert_eq!(tc.rows[0][0].to_integer_i64(), Some(0));

    // Reset & close session 1 (simulates TDS client disconnect / pool checkin)
    db.reset_session(s1).expect("reset session 1");
    db.close_session(s1).expect("close session 1");

    // Session 2 MUST observe committed table & row
    let s2 = db.create_session();
    let res = db
        .execute_session_batch_sql(s2, "SELECT Id FROM dbo.TxPersist")
        .expect("session 2 select must see committed table")
        .expect("result");
    assert_eq!(res.rows.len(), 1);
    assert_eq!(res.rows[0][0].to_integer_i64(), Some(123));

    // Session 2: BEGIN TRAN -> INSERT -> IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION;
    db.execute_session_batch_sql_multi(
        s2,
        "BEGIN TRANSACTION;
         INSERT INTO dbo.TxPersist VALUES (999);
         IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION;",
    )
    .expect("session 2 rollback batch must succeed");

    let res2 = db
        .execute_session_batch_sql(
            s2,
            "SELECT COUNT(*) AS cnt, @@TRANCOUNT AS tc FROM dbo.TxPersist",
        )
        .expect("session 2 count after rollback")
        .expect("result");
    assert_eq!(res2.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(res2.rows[0][1].to_integer_i64(), Some(0));
}

#[test]
fn concurrent_disjoint_ddl_and_dml_transactions_merge_without_clobbering() {
    let db = Database::new();
    let s1 = db.create_session();
    let s2 = db.create_session();

    // Both sessions open transactions against the same initial snapshot.
    db.execute_session_batch_sql_multi(s1, "BEGIN TRANSACTION;")
        .expect("s1 begin");
    db.execute_session_batch_sql_multi(s2, "BEGIN TRANSACTION;")
        .expect("s2 begin");

    // Each session creates disjoint tables, indexes, procedures, views, and rows.
    db.execute_session_batch_sql_multi(
        s1,
        "CREATE TABLE dbo.Alpha (Id INT NOT NULL PRIMARY KEY, Val NVARCHAR(20));
         CREATE NONCLUSTERED INDEX IX_Alpha_Val ON dbo.Alpha (Val);
         INSERT INTO dbo.Alpha VALUES (1, N'one');
         CREATE PROCEDURE dbo.usp_alpha AS BEGIN SELECT 101 AS v; END;",
    )
    .expect("s1 ddl+dml");

    db.execute_session_batch_sql_multi(
        s2,
        "CREATE TABLE dbo.Beta (Id INT NOT NULL PRIMARY KEY, Val NVARCHAR(20));
         CREATE NONCLUSTERED INDEX IX_Beta_Val ON dbo.Beta (Val);
         INSERT INTO dbo.Beta VALUES (2, N'two');
         CREATE PROCEDURE dbo.usp_beta AS BEGIN SELECT 202 AS v; END;",
    )
    .expect("s2 ddl+dml");

    // Commit s1 first, then s2 — s2 must merge its delta without clobbering s1.
    db.execute_session_batch_sql_multi(s1, "COMMIT TRANSACTION;")
        .expect("s1 commit");
    db.execute_session_batch_sql_multi(s2, "COMMIT TRANSACTION;")
        .expect("s2 commit");

    let s3 = db.create_session();
    let a = db
        .execute_session_batch_sql(s3, "SELECT Id, Val FROM dbo.Alpha")
        .expect("s3 select Alpha")
        .expect("result");
    assert_eq!(a.rows.len(), 1);
    assert_eq!(a.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(a.rows[0][1].to_string_value(), "one");

    let b = db
        .execute_session_batch_sql(s3, "SELECT Id, Val FROM dbo.Beta")
        .expect("s3 select Beta")
        .expect("result");
    assert_eq!(b.rows.len(), 1);
    assert_eq!(b.rows[0][0].to_integer_i64(), Some(2));
    assert_eq!(b.rows[0][1].to_string_value(), "two");

    // Distinct table IDs & both indexes/procedures preserved.
    let ids = db
        .execute_session_batch_sql(
            s3,
            "SELECT COUNT(DISTINCT object_id) FROM sys.tables WHERE name IN ('Alpha', 'Beta')",
        )
        .expect("s3 sys.tables")
        .expect("result");
    assert_eq!(ids.rows[0][0].to_integer_i64(), Some(2));

    let idxs = db
        .execute_session_batch_sql(
            s3,
            "SELECT COUNT(*) FROM sys.indexes WHERE name IN ('IX_Alpha_Val', 'IX_Beta_Val')",
        )
        .expect("s3 sys.indexes")
        .expect("result");
    assert_eq!(idxs.rows[0][0].to_integer_i64(), Some(2));

    let pa = db
        .execute_session_batch_sql(s3, "EXEC dbo.usp_alpha")
        .expect("s3 exec usp_alpha")
        .expect("result");
    assert_eq!(pa.rows[0][0].to_integer_i64(), Some(101));

    let pb = db
        .execute_session_batch_sql(s3, "EXEC dbo.usp_beta")
        .expect("s3 exec usp_beta")
        .expect("result");
    assert_eq!(pb.rows[0][0].to_integer_i64(), Some(202));

    // Same table name in two distinct schemas must acquire disjoint schema-qualified locks.
    db.execute_session_batch_sql(s3, "CREATE SCHEMA analytics;")
        .expect("create schema analytics");
    let s4 = db.create_session();
    let s5 = db.create_session();
    db.execute_session_batch_sql_multi(
        s4,
        "BEGIN TRANSACTION; CREATE TABLE dbo.SharedName (Id INT);",
    )
    .expect("s4 dbo.SharedName");
    db.execute_session_batch_sql_multi(
        s5,
        "BEGIN TRANSACTION; CREATE TABLE analytics.SharedName (Id INT);",
    )
    .expect("s5 analytics.SharedName must not conflict with dbo.SharedName");
    db.execute_session_batch_sql_multi(s4, "COMMIT TRANSACTION;")
        .expect("s4 commit");
    db.execute_session_batch_sql_multi(s5, "COMMIT TRANSACTION;")
        .expect("s5 commit");
}

#[test]
fn multithreaded_concurrent_table_batches_never_lose_committed_tables() {
    use std::sync::Arc;
    let db = Arc::new(Database::new());
    let mut handles = Vec::new();
    for worker in 0..4usize {
        let db = Arc::clone(&db);
        handles.push(std::thread::spawn(move || {
            let sid = db.create_session();
            for batch in 0..20usize {
                let mut sql = String::from("SET XACT_ABORT ON;\nBEGIN TRANSACTION;\n");
                for i in 0..10usize {
                    let tbl = format!("Tbl_W{}_B{}_I{}", worker, batch, i);
                    sql.push_str(&format!(
                        "CREATE TABLE [dbo].[{tbl}] ([Id] INT NOT NULL PRIMARY KEY, [Val] INT);\n"
                    ));
                }
                sql.push_str("IF @@TRANCOUNT > 0 COMMIT TRANSACTION;");
                db.execute_session_batch_sql_multi(sid, &sql)
                    .unwrap_or_else(|e| panic!("worker {worker} batch {batch} failed: {e}"));
            }
            db.close_session(sid).unwrap();
        }));
    }
    for h in handles {
        h.join().expect("worker panicked");
    }
    let check_sid = db.create_session();
    let res = db
        .execute_session_batch_sql(check_sid, "SELECT COUNT(*) FROM sys.tables")
        .expect("count sys.tables")
        .expect("result");
    assert_eq!(
        res.rows[0][0].to_integer_i64(),
        Some(800),
        "all 4 * 20 * 10 = 800 concurrently created tables must survive COMMIT merging"
    );
}

#[test]
fn numbered_stored_procedures_and_linked_server_exec_at() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.usp_item_group;1 @id INT AS SELECT @id AS v;",
    );
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.usp_item_group;2 @id INT AS SELECT @id + 1 AS v;",
    );
    // In T-SQL, CREATE PROCEDURE consumes the entire batch until GO/EOF; statements
    // after an initial BEGIN...END block remain part of the procedure body and must
    // NEVER execute eagerly at CREATE PROCEDURE time.
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.usp_multi_block_after_begin_end AS
         BEGIN
             DECLARE @x INT = 1;
         END
         INSERT INTO dbo.NotYetCreatedTable VALUES (1);",
    );
    let tc = query(&mut e, "SELECT @@TRANCOUNT AS tc");
    assert_eq!(tc.rows[0][0].to_integer_i64(), Some(0));
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.usp_remote_query AS
         BEGIN
             DECLARE @p INT = 1;
             CREATE TABLE #Out (V INT);
             EXEC ('SELECT 1') AT [RemoteServer];
             EXEC ('SELECT ?', @p) AT [RemoteServer];
             INSERT INTO #Out EXEC ('SELECT 1') AT [RemoteServer];
             CREATE INDEX IX_Out_V ON #Out (V);
             DROP INDEX #Out.IX_Out_V;
         END;",
    );
}

#[test]
fn create_view_with_cte_executes_and_returns_rows() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE TABLE dbo.SalesLog (Id INT NOT NULL PRIMARY KEY, Category NVARCHAR(20), Amount INT);
         INSERT INTO dbo.SalesLog VALUES (1, N'A', 10), (2, N'A', 25), (3, N'B', 50);",
    );
    exec(
        &mut e,
        "CREATE VIEW dbo.vw_TopCategorySales AS
         WITH Ranked AS (
             SELECT Category, Amount,
                    ROW_NUMBER() OVER (PARTITION BY Category ORDER BY Amount DESC) AS rn
             FROM dbo.SalesLog
         )
         SELECT Category, Amount FROM Ranked WHERE rn = 1;",
    );
    let r = query(
        &mut e,
        "SELECT Category, Amount FROM dbo.vw_TopCategorySales ORDER BY Category",
    );
    assert_eq!(r.rows.len(), 2);
    assert_eq!(r.rows[0][0].to_string_value(), "A");
    assert_eq!(r.rows[0][1].to_integer_i64(), Some(25));
    assert_eq!(r.rows[1][0].to_string_value(), "B");
    assert_eq!(r.rows[1][1].to_integer_i64(), Some(50));
}

#[test]
fn create_function_allows_optional_as_before_begin() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE FUNCTION dbo.ufn_empty_rows()
         RETURNS @res TABLE (Id INT, Label NVARCHAR(20))
         BEGIN
             INSERT INTO @res VALUES (1, N'ok');
             RETURN;
         END;",
    );
    exec(
        &mut e,
        "CREATE FUNCTION dbo.ufn_double(@x INT)
         RETURNS INT
         BEGIN
             RETURN @x * 2;
         END;",
    );
    let r1 = query(&mut e, "SELECT Id, Label FROM dbo.ufn_empty_rows()");
    assert_eq!(r1.rows.len(), 1);
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r1.rows[0][1].to_string_value(), "ok");

    let r2 = query(&mut e, "SELECT dbo.ufn_double(21) AS val");
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(42));
}

#[test]
fn spaced_comparison_ops_comma_after_join_switch_partition_and_double_colon() {
    let mut e = Engine::new();
    // 1. Spaced two-char comparison operators: > =, < =, < >
    let r_ops = query(
        &mut e,
        "SELECT CASE WHEN 5 > = 5 AND 4 < = 4 AND 3 < > 4 THEN 1 ELSE 0 END AS ok",
    );
    assert_eq!(r_ops.rows[0][0].to_integer_i64(), Some(1));

    // 2. Comma-join after INNER JOIN (including TVF or table)
    exec(
        &mut e,
        "CREATE TABLE dbo.LeftItems (Id INT);
         CREATE TABLE dbo.MidItems (Id INT);
         CREATE TABLE dbo.RightItems (Id INT);
         INSERT INTO dbo.LeftItems VALUES (7);
         INSERT INTO dbo.MidItems VALUES (7);
         INSERT INTO dbo.RightItems VALUES (7);",
    );
    let r_join = query(
        &mut e,
        "SELECT l.Id, r.Id
         FROM dbo.LeftItems l
         INNER JOIN dbo.MidItems m ON l.Id = m.Id,
         dbo.RightItems r
         WHERE r.Id = l.Id",
    );
    assert_eq!(r_join.rows.len(), 1);
    assert_eq!(r_join.rows[0][0].to_integer_i64(), Some(7));
    assert_eq!(r_join.rows[0][1].to_integer_i64(), Some(7));

    // 3. ALTER TABLE ... SWITCH PARTITION (SQL Server requires target table to be empty)
    exec(
        &mut e,
        "TRUNCATE TABLE dbo.RightItems;
         ALTER TABLE dbo.LeftItems SWITCH PARTITION 1 TO dbo.RightItems;
         TRUNCATE TABLE dbo.RightItems;
         ALTER TABLE dbo.LeftItems SWITCH PARTITION 2 TO dbo.RightItems PARTITION 2;",
    );

    // 4. Static type double-colon method syntax, DEC(p,s) synonym,
    //    parameterized table hints WITH (INDEX(...)), and PERCENTILE_CONT WITHIN GROUP OVER
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.usp_extended_tsql_syntax AS
         BEGIN
             DECLARE @g NVARCHAR(100) = geometry::STGeomFromText('POINT(0 0)', 4326);
             CREATE TABLE #Metrics (Id INT, Category INT, Val DEC(18, 2));
             CREATE INDEX IX_Metrics_Id ON #Metrics (Id);
             SELECT DISTINCT
                 m.Category,
                 PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY m.Val) OVER (PARTITION BY m.Category) AS MedianVal
             FROM #Metrics m WITH (INDEX(IX_Metrics_Id), NOLOCK);
             SELECT m.Id FROM #Metrics m (NOLOCK);
             INSERT INTO #Metrics (m.Id, m.Category, m.Val) VALUES (1, 10, 2.50);
             ALTER TABLE #Metrics DROP COLUMN Category, Val;
             IF 1 = 0 GOTO ErrLabel;
             RETURN
             ErrLabel:
             BEGIN
                 RETURN;
             END
         END;",
    );
}

#[test]
fn catalog_metadata_columns_and_variant_property_functions() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE TABLE dbo.CatalogHost (
             Id INT NOT NULL PRIMARY KEY,
             Code INT NOT NULL DEFAULT (0),
             CONSTRAINT CK_CatalogHost_Code CHECK (Code >= 0)
         );
         CREATE TABLE dbo.CatalogChild (
             Id INT NOT NULL PRIMARY KEY,
             HostId INT NOT NULL REFERENCES dbo.CatalogHost(Id)
         );",
    );

    let kc = query(
        &mut e,
        "SELECT name, unique_index_id, is_system_named FROM sys.key_constraints",
    );
    assert!(kc.rows.len() >= 2);
    let dc = query(
        &mut e,
        "SELECT name, is_system_named FROM sys.default_constraints",
    );
    assert_eq!(dc.rows.len(), 1);
    let cc = query(
        &mut e,
        "SELECT name, is_system_named FROM sys.check_constraints",
    );
    assert_eq!(cc.rows.len(), 1);
    let fk = query(
        &mut e,
        "SELECT name, is_not_for_replication FROM sys.foreign_keys",
    );
    assert_eq!(fk.rows.len(), 1);
    let ty = query(
        &mut e,
        "SELECT name, is_nullable, is_table_type FROM sys.types WHERE is_table_type = 0",
    );
    assert!(!ty.rows.is_empty());
    let _ = query(&mut e, "SELECT secondary_type_desc FROM sys.xml_indexes");
    let _ = query(
        &mut e,
        "SELECT is_accent_sensitivity_on FROM sys.fulltext_catalogs",
    );
    let _ = query(&mut e, "SELECT user_type_id FROM sys.partition_parameters");

    let sv = query(
        &mut e,
        "SELECT SQL_VARIANT_PROPERTY(CAST(42 AS INT), 'BaseType'),
                SQL_VARIANT_PROPERTY(CAST(12.34 AS DECIMAL(10, 2)), 'Scale'),
                XML_SCHEMA_NAMESPACE(N'dbo', N'MissingSchema')",
    );
    assert_eq!(sv.rows[0][0].to_string_value(), "int");
    assert_eq!(sv.rows[0][1].to_integer_i64(), Some(2));
    assert!(sv.rows[0][2].is_null());
}
