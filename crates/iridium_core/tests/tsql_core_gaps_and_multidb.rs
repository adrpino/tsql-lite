use iridium_core::types::Value;
use iridium_core::Engine;

fn exec(engine: &Engine, sid: u64, sql: &str) -> iridium_core::QueryResult {
    engine
        .execute_session_batch_sql(sid, sql)
        .unwrap_or_else(|e| panic!("SQL failed:\n{sql}\nerror: {e}"))
        .unwrap_or_default()
}

#[test]
fn inline_column_constraints_parse_enforce_and_appear_in_sys_catalog() {
    let engine = Engine::new();
    let sid = engine.create_session();

    exec(
        &engine,
        sid,
        "CREATE TABLE dbo.RootNodes (
            Id INT NOT NULL CONSTRAINT PK_RootNodes PRIMARY KEY,
            Code NVARCHAR(20) NOT NULL CONSTRAINT UQ_RootNodes_Code UNIQUE
         );
         CREATE TABLE dbo.LeafNodes (
            Id INT NOT NULL CONSTRAINT PK_LeafNodes PRIMARY KEY,
            RootId INT NOT NULL CONSTRAINT FK_LeafNodes_Root REFERENCES dbo.RootNodes(Id) ON DELETE CASCADE,
            Qty INT NOT NULL CONSTRAINT CK_LeafNodes_Qty CHECK (Qty >= 0),
            Flag BIT NOT NULL CONSTRAINT DF_LeafNodes_Flag DEFAULT (1)
         );",
    );

    let ck = exec(
        &engine,
        sid,
        "SELECT name, is_system_named FROM sys.check_constraints WHERE parent_object_id = OBJECT_ID('dbo.LeafNodes');",
    );
    assert_eq!(
        ck.rows,
        vec![vec![
            Value::VarChar("CK_LeafNodes_Qty".into()),
            Value::Bit(false)
        ]]
    );

    let fk = exec(
        &engine,
        sid,
        "SELECT name FROM sys.foreign_keys WHERE parent_object_id = OBJECT_ID('dbo.LeafNodes');",
    );
    assert_eq!(
        fk.rows,
        vec![vec![Value::VarChar("FK_LeafNodes_Root".into())]]
    );

    exec(&engine, sid, "INSERT INTO dbo.RootNodes VALUES (1, N'A1');");
    exec(
        &engine,
        sid,
        "INSERT INTO dbo.LeafNodes (Id, RootId, Qty) VALUES (10, 1, 5);",
    );

    // Inline CHECK violation must fail
    let bad_chk = engine.execute_session_batch_sql(
        sid,
        "INSERT INTO dbo.LeafNodes (Id, RootId, Qty) VALUES (11, 1, -2);",
    );
    assert!(
        bad_chk.is_err(),
        "inline CHECK constraint should reject Qty = -2"
    );

    // Inline FK violation must fail
    let bad_fk = engine.execute_session_batch_sql(
        sid,
        "INSERT INTO dbo.LeafNodes (Id, RootId, Qty) VALUES (12, 999, 3);",
    );
    assert!(
        bad_fk.is_err(),
        "inline FK constraint should reject missing parent"
    );

    // ON DELETE CASCADE from inline FK must remove child row
    exec(&engine, sid, "DELETE FROM dbo.RootNodes WHERE Id = 1;");
    let remaining = exec(&engine, sid, "SELECT COUNT(*) FROM dbo.LeafNodes;");
    assert_eq!(remaining.rows, vec![vec![Value::BigInt(0)]]);
}

#[test]
fn xp_msver_columns_and_optname_filtering_and_event_session_views() {
    let engine = Engine::new();
    let sid = engine.create_session();

    let all = exec(&engine, sid, "EXEC master.dbo.xp_msver;");
    assert_eq!(
        all.columns,
        vec!["Index", "Name", "Internal_Value", "Character_Value"]
    );
    assert!(all.rows.len() >= 15);

    let filtered = exec(
        &engine,
        sid,
        "EXEC xp_msver N'Platform', N'ProductVersion';",
    );
    assert_eq!(filtered.rows.len(), 2);
    assert_eq!(
        filtered.rows[0][1],
        Value::NVarChar("ProductVersion".into())
    );
    assert_eq!(filtered.rows[1][1], Value::NVarChar("Platform".into()));

    let xe = exec(
        &engine,
        sid,
        "SELECT ses.name, sese.name, sesa.name, sest.name,
                SQL_VARIANT_PROPERTY(sesf.value, N'BaseType')
         FROM sys.server_event_sessions ses
         LEFT JOIN sys.server_event_session_events sese ON sese.event_session_id = ses.event_session_id
         LEFT JOIN sys.server_event_session_actions sesa ON sesa.event_session_id = ses.event_session_id
         LEFT JOIN sys.server_event_session_targets sest ON sest.event_session_id = ses.event_session_id
         LEFT JOIN sys.server_event_session_fields sesf ON sesf.event_session_id = ses.event_session_id;",
    );
    assert!(xe.rows.is_empty());
}

#[test]
fn always_encrypted_cmk_cek_ddl_lifecycle_and_catalog_views() {
    let engine = Engine::new();
    let sid = engine.create_session();

    exec(
        &engine,
        sid,
        "CREATE COLUMN MASTER KEY [CMK_SamplePrimary]
         WITH (
             KEY_STORE_PROVIDER_NAME = N'MSSQL_CERTIFICATE_STORE',
             KEY_PATH = N'CurrentUser/my/AABBCCDDEE00112233445566778899FF',
             ENCLAVE_COMPUTATIONS (SIGNATURE = 0xDEADBEEF)
         );
         CREATE COLUMN MASTER KEY [CMK_SampleBackup]
         WITH (
             KEY_STORE_PROVIDER_NAME = N'AZURE_KEY_VAULT',
             KEY_PATH = N'https://vault.example.test/keys/backup/1'
         );
         CREATE COLUMN ENCRYPTION KEY [CEK_SampleMain]
         WITH VALUES (
             COLUMN_MASTER_KEY = [CMK_SamplePrimary],
             ALGORITHM = 'RSA_OAEP',
             ENCRYPTED_VALUE = 0x016E00112233
         );",
    );

    let cmks = exec(
        &engine,
        sid,
        "SELECT name, key_store_provider_name, allow_enclave_computations
         FROM sys.column_master_keys
         ORDER BY column_master_key_id;",
    );
    assert_eq!(cmks.rows.len(), 2);
    assert_eq!(cmks.rows[0][0], Value::VarChar("CMK_SamplePrimary".into()));
    assert_eq!(cmks.rows[0][2], Value::Bit(true));
    assert_eq!(cmks.rows[1][0], Value::VarChar("CMK_SampleBackup".into()));
    assert_eq!(cmks.rows[1][2], Value::Bit(false));

    // Add second encrypted value via ALTER COLUMN ENCRYPTION KEY
    exec(
        &engine,
        sid,
        "ALTER COLUMN ENCRYPTION KEY [CEK_SampleMain]
         ADD VALUE (
             COLUMN_MASTER_KEY = [CMK_SampleBackup],
             ALGORITHM = 'RSA_OAEP',
             ENCRYPTED_VALUE = 0x016E44556677
         );",
    );
    let cek_vals = exec(
        &engine,
        sid,
        "SELECT COUNT(*) FROM sys.column_encryption_key_values;",
    );
    assert_eq!(cek_vals.rows, vec![vec![Value::BigInt(2)]]);

    // Referenced CMK cannot be dropped
    assert!(engine
        .execute_session_batch_sql(sid, "DROP COLUMN MASTER KEY [CMK_SamplePrimary];")
        .is_err());

    // Drop value + CEK + CMKs
    exec(
        &engine,
        sid,
        "ALTER COLUMN ENCRYPTION KEY [CEK_SampleMain]
         DROP VALUE (COLUMN_MASTER_KEY = [CMK_SampleBackup]);
         DROP COLUMN MASTER KEY [CMK_SampleBackup];
         DROP COLUMN ENCRYPTION KEY [CEK_SampleMain];
         DROP COLUMN MASTER KEY [CMK_SamplePrimary];",
    );
    let left = exec(&engine, sid, "SELECT COUNT(*) FROM sys.column_master_keys;");
    assert_eq!(left.rows, vec![vec![Value::BigInt(0)]]);
}

#[test]
fn partition_function_evaluation_and_switch_partition() {
    let engine = Engine::new();
    let sid = engine.create_session();

    exec(
        &engine,
        sid,
        "CREATE PARTITION FUNCTION pf_Left (INT) AS RANGE LEFT FOR VALUES (10, 20, 30);
         CREATE PARTITION FUNCTION pf_Right (INT) AS RANGE RIGHT FOR VALUES (10, 20, 30);",
    );

    let res = exec(
        &engine,
        sid,
        "SELECT
            $PARTITION.pf_Left(NULL),
            $PARTITION.pf_Left(10),
            $PARTITION.pf_Left(11),
            $PARTITION.pf_Left(30),
            $PARTITION.pf_Left(31),
            $PARTITION.pf_Right(NULL),
            $PARTITION.pf_Right(9),
            $PARTITION.pf_Right(10),
            $PARTITION.pf_Right(20),
            $PARTITION.pf_Right(30);",
    );
    assert_eq!(
        res.rows,
        vec![vec![
            Value::Int(1),
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4),
            Value::Int(1),
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4),
        ]]
    );

    exec(
        &engine,
        sid,
        "CREATE TABLE dbo.StagePart (Id INT, Label VARCHAR(20));
         CREATE TABLE dbo.ArchivePart (Id INT, Label VARCHAR(20));
         INSERT INTO dbo.StagePart VALUES (1, 'p1'), (2, 'p2');
         ALTER TABLE dbo.StagePart SWITCH PARTITION 1 TO dbo.ArchivePart PARTITION 1;",
    );
    let counts = exec(
        &engine,
        sid,
        "SELECT (SELECT COUNT(*) FROM dbo.StagePart), (SELECT COUNT(*) FROM dbo.ArchivePart);",
    );
    assert_eq!(counts.rows, vec![vec![Value::BigInt(0), Value::BigInt(2)]]);

    // Switching into a non-empty target must fail
    exec(&engine, sid, "INSERT INTO dbo.StagePart VALUES (3, 'p3');");
    assert!(engine
        .execute_session_batch_sql(
            sid,
            "ALTER TABLE dbo.StagePart SWITCH PARTITION 1 TO dbo.ArchivePart PARTITION 1;"
        )
        .is_err());
}

#[test]
fn fulltext_contains_freetext_spatial_and_hierarchyid_work() {
    let engine = Engine::new();
    let sid = engine.create_session();

    exec(
        &engine,
        sid,
        "CREATE TABLE dbo.Docs (Id INT PRIMARY KEY, Body NVARCHAR(200));
         INSERT INTO dbo.Docs VALUES
            (1, N'Fast Rust query engine for structured data'),
            (2, N'Legacy slow parser without optimization'),
            (3, N'Rusty gears and fast mechanical clocks');",
    );

    let c1 = exec(
        &engine,
        sid,
        "SELECT Id FROM dbo.Docs WHERE CONTAINS(Body, 'rust AND \"query engine\" AND NOT slow') ORDER BY Id;",
    );
    assert_eq!(c1.rows, vec![vec![Value::Int(1)]]);

    let c2 = exec(
        &engine,
        sid,
        "SELECT Id FROM dbo.Docs WHERE CONTAINS(Body, '\"rust*\"') ORDER BY Id;",
    );
    assert_eq!(c2.rows, vec![vec![Value::Int(1)], vec![Value::Int(3)]]);

    let ft = exec(
        &engine,
        sid,
        "SELECT Id FROM dbo.Docs WHERE FREETEXT(Body, 'optimization clocks') ORDER BY Id;",
    );
    assert_eq!(ft.rows, vec![vec![Value::Int(2)], vec![Value::Int(3)]]);

    // Spatial: Geometry exact 3-4-5 triangle + Geography NYC->London distance
    let geom = exec(
        &engine,
        sid,
        "DECLARE @a GEOMETRY = geometry::Point(0, 0, 0);
         DECLARE @b GEOMETRY = geometry::STGeomFromText('POINT(3 4)', 0);
         SELECT @a.STDistance(@b), @b.STX, @b.STY, @b.STAsText();",
    );
    assert_eq!(geom.rows[0][0], Value::Float(5.0f64.to_bits()));
    assert_eq!(geom.rows[0][1], Value::Float(3.0f64.to_bits()));
    assert_eq!(geom.rows[0][2], Value::Float(4.0f64.to_bits()));
    assert_eq!(geom.rows[0][3], Value::NVarChar("POINT (3 4)".into()));

    let geog = exec(
        &engine,
        sid,
        "DECLARE @nyc GEOGRAPHY = geography::Point(40.7128, -74.0060, 4326);
         DECLARE @lon GEOGRAPHY = geography::Point(51.5074, -0.1278, 4326);
         SELECT @nyc.Lat, @nyc.Long, @nyc.STSrid, @nyc.STDistance(@lon);",
    );
    assert_eq!(geog.rows[0][2], Value::Int(4326));
    if let Value::Float(bits) = geog.rows[0][3] {
        let meters = f64::from_bits(bits);
        assert!(
            meters > 5_550_000.0 && meters < 5_600_000.0,
            "expected NYC->London ~5,570 km, got {meters} m"
        );
    } else {
        panic!("expected Float distance, got {:?}", geog.rows[0][3]);
    }

    // HIERARCHYID methods
    let hid = exec(
        &engine,
        sid,
        "DECLARE @root HIERARCHYID = hierarchyid::GetRoot();
         DECLARE @node HIERARCHYID = hierarchyid::Parse('/1/2/3/');
         DECLARE @parent HIERARCHYID = hierarchyid::Parse('/1/2/');
         SELECT
            @root.ToString(),
            @node.GetLevel(),
            @node.IsDescendantOf(@parent),
            @parent.IsDescendantOf(@node),
            @node.GetAncestor(1).ToString(),
            @node.GetReparentedValue(@parent, hierarchyid::Parse('/9/')).ToString();",
    );
    assert_eq!(
        hid.rows,
        vec![vec![
            Value::NVarChar("/".into()),
            Value::SmallInt(3),
            Value::Bit(true),
            Value::Bit(false),
            Value::NVarChar("/1/2/".into()),
            Value::NVarChar("/9/3/".into()),
        ]]
    );
}

#[test]
fn multi_database_isolation_and_cross_database_three_part_queries() {
    let engine = Engine::new();
    let sid = engine.create_session();

    exec(&engine, sid, "CREATE DATABASE [AlphaDb];");
    exec(&engine, sid, "CREATE DATABASE [BetaDb];");

    let db_check = exec(
        &engine,
        sid,
        "SELECT DB_ID('AlphaDb'), DB_NAME(DB_ID('AlphaDb')),
                (SELECT COUNT(*) FROM sys.databases WHERE name IN ('AlphaDb', 'BetaDb'));",
    );
    assert_eq!(db_check.rows[0][1], Value::NVarChar("AlphaDb".into()));
    assert_eq!(db_check.rows[0][2], Value::BigInt(2));

    // Populate AlphaDb
    exec(&engine, sid, "USE [AlphaDb];");
    exec(
        &engine,
        sid,
        "CREATE SCHEMA [Billing];
         CREATE TABLE dbo.Items (Id INT PRIMARY KEY, Tag VARCHAR(20));
         CREATE TABLE Billing.Invoices (InvId INT PRIMARY KEY, ItemId INT);
         INSERT INTO dbo.Items VALUES (1, 'alpha-1'), (2, 'alpha-2');
         INSERT INTO Billing.Invoices VALUES (100, 1);
         CREATE PROCEDURE dbo.usp_GetAlphaCount AS
         BEGIN
             SELECT COUNT(*) AS Cnt FROM dbo.Items;
         END;",
    );

    // Populate BetaDb with a same-named dbo.Items table — completely isolated!
    exec(&engine, sid, "USE [BetaDb];");
    exec(
        &engine,
        sid,
        "CREATE TABLE dbo.Items (Id INT PRIMARY KEY, Tag VARCHAR(20));
         INSERT INTO dbo.Items VALUES (1, 'beta-1'), (3, 'beta-3');",
    );

    // BetaDb sees ONLY its own tables and schemas
    let beta_tables = exec(
        &engine,
        sid,
        "SELECT name, SCHEMA_NAME(schema_id) FROM sys.tables ORDER BY name;",
    );
    assert_eq!(
        beta_tables.rows,
        vec![vec![
            Value::VarChar("Items".into()),
            Value::NVarChar("dbo".into())
        ]]
    );
    let beta_rows = exec(&engine, sid, "SELECT Id, Tag FROM dbo.Items ORDER BY Id;");
    assert_eq!(
        beta_rows.rows,
        vec![
            vec![Value::Int(1), Value::VarChar("beta-1".into())],
            vec![Value::Int(3), Value::VarChar("beta-3".into())],
        ]
    );

    // Cross-DB 3-part JOIN from BetaDb -> AlphaDb.dbo.Items + AlphaDb.Billing.Invoices
    let cross = exec(
        &engine,
        sid,
        "SELECT b.Id, a.Tag AS AlphaTag, b.Tag AS BetaTag, inv.InvId
         FROM dbo.Items b
         INNER JOIN AlphaDb.dbo.Items a ON a.Id = b.Id
         INNER JOIN AlphaDb.Billing.Invoices inv ON inv.ItemId = a.Id;",
    );
    assert_eq!(
        cross.rows,
        vec![vec![
            Value::Int(1),
            Value::VarChar("alpha-1".into()),
            Value::VarChar("beta-1".into()),
            Value::Int(100),
        ]]
    );

    // Cross-DB EXEC from BetaDb runs procedure in AlphaDb's namespace
    let cross_proc = exec(&engine, sid, "EXEC AlphaDb.dbo.usp_GetAlphaCount;");
    assert_eq!(cross_proc.rows, vec![vec![Value::BigInt(2)]]);

    // OBJECT_ID works for both local and 3-part cross-DB names
    let obj_ids = exec(
        &engine,
        sid,
        "SELECT CASE WHEN OBJECT_ID('dbo.Items') <> OBJECT_ID('AlphaDb.dbo.Items')
                      AND OBJECT_ID('AlphaDb.dbo.Items') IS NOT NULL
                 THEN 1 ELSE 0 END;",
    );
    assert_eq!(obj_ids.rows, vec![vec![Value::Int(1)]]);

    // DROP DATABASE removes AlphaDb and its tables while leaving BetaDb intact
    exec(&engine, sid, "DROP DATABASE [AlphaDb];");
    let after_drop = exec(
        &engine,
        sid,
        "SELECT DB_ID('AlphaDb'), (SELECT COUNT(*) FROM dbo.Items);",
    );
    assert_eq!(after_drop.rows, vec![vec![Value::Null, Value::BigInt(2)]]);
}
