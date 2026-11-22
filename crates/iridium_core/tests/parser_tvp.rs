use iridium_core::{
    ast::{DdlStatement, ProceduralStatement, Statement},
    parse_sql,
};

#[test]
fn parse_create_and_drop_type() {
    let stmt = parse_sql("CREATE TYPE dbo.IntList AS TABLE (id INT)").unwrap();
    assert!(matches!(
        stmt,
        Statement::Ddl(ref d) if matches!(d.as_ref(), DdlStatement::CreateType(_))
    ));

    let stmt = parse_sql("DROP TYPE dbo.IntList").unwrap();
    assert!(matches!(
        stmt,
        Statement::Ddl(ref d) if matches!(d.as_ref(), DdlStatement::DropType(_))
    ));
}

#[test]
fn parse_tvp_param_requires_readonly() {
    // Syntax-level: non-READONLY custom types are valid for scalar alias UDTs,
    // while catalog-level DDL enforcement rejects table types without READONLY.
    let engine = iridium_core::Engine::new();
    engine
        .exec("CREATE TYPE dbo.IntList AS TABLE (id INT)")
        .expect("create table type");
    let err = engine
        .exec("CREATE PROCEDURE dbo.p @items dbo.IntList AS BEGIN SELECT 1 END")
        .unwrap_err();
    assert!(err.to_string().to_uppercase().contains("READONLY"));
}

#[test]
fn parse_sp_executesql_tvp_decl() {
    let stmt =
        parse_sql("EXEC sp_executesql N'SELECT 1', N'@items dbo.IntList READONLY', @items = @tvp")
            .unwrap();
    assert!(matches!(
        stmt,
        Statement::Procedural(ProceduralStatement::SpExecuteSql(_))
    ));
}
