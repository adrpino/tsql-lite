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
        .into_iter()
        .last()
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
    let r5_empty = query(&mut e, "SELECT id FROM dbo.items WHERE id = 999 FOR JSON PATH");
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
    let err = e.execute_batch(stmts).expect_err("undefined label must fail");
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
    assert_eq!(r_to.rows[0][0].to_string_value(), "2025-03-01 10:30:00 -05:00");
    assert_eq!(r_to.rows[0][1].to_string_value(), "2025-03-01 10:30:00 +02:00");

    let r_sw = query(
        &mut e,
        "SELECT SWITCHOFFSET(TODATETIMEOFFSET(CAST('2025-03-01 10:30:00' AS DATETIME2), '-05:00'), '+02:00')",
    );
    assert_eq!(r_sw.rows[0][0].to_string_value(), "2025-03-01 17:30:00 +02:00");
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
        .execute_session_batch_sql(s2, "SELECT COUNT(*) AS cnt, @@TRANCOUNT AS tc FROM dbo.TxPersist")
        .expect("session 2 count after rollback")
        .expect("result");
    assert_eq!(res2.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(res2.rows[0][1].to_integer_i64(), Some(0));
}
