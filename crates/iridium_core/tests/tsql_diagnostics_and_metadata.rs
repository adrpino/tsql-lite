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
fn raiserror_and_throw_preserve_number_severity_and_state() {
    let e = Engine::new();

    let err_re = e
        .execute_batch(parse_batch("RAISERROR('custom failure', 14, 7)").unwrap())
        .expect_err("RAISERROR severity 14 must fail");
    assert_eq!(err_re.number(), 50000);
    assert_eq!(err_re.class_severity(), 14);
    assert_eq!(err_re.state(), 7);

    let err_th = e
        .execute_batch(parse_batch("THROW 51234, 'thrown failure', 9;").unwrap())
        .expect_err("THROW must fail");
    assert_eq!(err_th.number(), 51234);
    assert_eq!(err_th.class_severity(), 16);
    assert_eq!(err_th.state(), 9);

    let caught = query(
        &e,
        "BEGIN TRY
             RAISERROR('caught failure', 13, 42);
         END TRY
         BEGIN CATCH
             SELECT ERROR_NUMBER(), ERROR_SEVERITY(), ERROR_STATE();
         END CATCH;",
    );
    assert_eq!(caught.rows[0][0].to_integer_i64(), Some(50000));
    assert_eq!(caught.rows[0][1].to_integer_i64(), Some(13));
    assert_eq!(caught.rows[0][2].to_integer_i64(), Some(42));
}

#[test]
fn projection_column_nullabilities_reflect_columns_and_expressions() {
    let e = Engine::new();
    exec(
        &e,
        "CREATE TABLE dbo.NullCheck (
             Id INT NOT NULL PRIMARY KEY,
             Label NVARCHAR(50) NULL
         );
         INSERT INTO dbo.NullCheck (Id, Label) VALUES (1, N'ok');",
    );

    let r = query(
        &e,
        "SELECT Id, Label, 123 AS Lit, NULL AS N, ISNULL(Label, N'fallback') AS NonNull FROM dbo.NullCheck",
    );
    assert_eq!(r.column_nullabilities, vec![false, true, false, true, false]);

    let r_star = query(&e, "SELECT * FROM dbo.NullCheck");
    assert_eq!(r_star.column_nullabilities, vec![false, true]);
}

#[test]
fn take_print_output_drains_printed_and_informational_messages() {
    let e = Engine::new();
    e.clear_print_output();
    exec(&e, "PRINT 'alpha'; RAISERROR('info %d', 0, 1, 7); PRINT 'beta';");
    assert_eq!(
        e.take_print_output(),
        vec!["alpha".to_string(), "info 7".to_string(), "beta".to_string()]
    );
    assert!(e.print_output().is_empty());
}

#[test]
fn trigger_is_disabled_and_extended_properties_persist_in_sys_views() {
    let e = Engine::new();
    exec(
        &e,
        "CREATE TABLE dbo.MetaHost (
             Id INT NOT NULL PRIMARY KEY,
             Label NVARCHAR(50) NULL
         );
         CREATE TRIGGER dbo.TR_MetaHost_After ON dbo.MetaHost AFTER INSERT AS BEGIN RETURN; END;
         DISABLE TRIGGER dbo.TR_MetaHost_After ON dbo.MetaHost;",
    );

    let tr_dis = query(
        &e,
        "SELECT is_disabled FROM sys.triggers WHERE name = 'TR_MetaHost_After'",
    );
    assert_eq!(tr_dis.rows[0][0].to_integer_i64(), Some(1));

    exec(&e, "ENABLE TRIGGER dbo.TR_MetaHost_After ON dbo.MetaHost;");
    let tr_en = query(
        &e,
        "SELECT is_disabled FROM sys.triggers WHERE name = 'TR_MetaHost_After'",
    );
    assert_eq!(tr_en.rows[0][0].to_integer_i64(), Some(0));

    exec(
        &e,
        "EXEC sys.sp_addextendedproperty
             @name = N'MS_Description',
             @value = N'Initial doc',
             @level0type = N'SCHEMA', @level0name = N'dbo',
             @level1type = N'TABLE', @level1name = N'MetaHost',
             @level2type = N'COLUMN', @level2name = N'Label';",
    );
    let ep1 = query(
        &e,
        "SELECT name, value, class_desc FROM sys.extended_properties WHERE name = 'MS_Description'",
    );
    assert_eq!(ep1.rows.len(), 1);
    assert_eq!(ep1.rows[0][1].to_string_value(), "Initial doc");
    assert_eq!(ep1.rows[0][2].to_string_value(), "OBJECT_OR_COLUMN");

    exec(
        &e,
        "EXEC sys.sp_updateextendedproperty
             @name = N'MS_Description',
             @value = N'Updated doc',
             @level0type = N'SCHEMA', @level0name = N'dbo',
             @level1type = N'TABLE', @level1name = N'MetaHost',
             @level2type = N'COLUMN', @level2name = N'Label';",
    );
    let ep2 = query(
        &e,
        "SELECT value FROM sys.extended_properties WHERE name = 'MS_Description'",
    );
    assert_eq!(ep2.rows[0][0].to_string_value(), "Updated doc");

    exec(
        &e,
        "EXEC sys.sp_dropextendedproperty
             @name = N'MS_Description',
             @level0type = N'SCHEMA', @level0name = N'dbo',
             @level1type = N'TABLE', @level1name = N'MetaHost',
             @level2type = N'COLUMN', @level2name = N'Label';",
    );
    let ep3 = query(
        &e,
        "SELECT COUNT(*) FROM sys.extended_properties WHERE name = 'MS_Description'",
    );
    assert_eq!(ep3.rows[0][0].to_integer_i64(), Some(0));
}
