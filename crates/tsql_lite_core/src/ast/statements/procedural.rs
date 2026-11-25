use crate::ast::common::TableRef;
use crate::ast::data_types::DataTypeSpec;
use crate::ast::expressions::Expr;
use crate::ast::statements::ddl::{ColumnSpec, TableConstraintSpec};
use crate::ast::statements::query::SelectStmt;
use crate::ast::ObjectName;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IfStmt {
    pub condition: Expr,
    pub then_body: Vec<crate::ast::Statement>,
    pub else_body: Option<Vec<crate::ast::Statement>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhileStmt {
    pub condition: Expr,
    pub body: Vec<crate::ast::Statement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultSetColumnSpec {
    pub name: String,
    pub data_type: DataTypeSpec,
    pub nullable: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResultSetDefinition {
    Columns(Vec<ResultSetColumnSpec>),
    AsObject(ObjectName),
    AsType(ObjectName),
    AsForXml,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecResultSets {
    Undefined,
    None,
    Definitions(Vec<ResultSetDefinition>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecStmt {
    pub sql_expr: Expr,
    #[serde(default)]
    pub result_sets: Option<ExecResultSets>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecArgument {
    pub name: Option<String>,
    pub expr: Expr,
    pub is_output: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecProcedureStmt {
    pub return_variable: Option<String>,
    pub name: ObjectName,
    pub args: Vec<ExecArgument>,
    #[serde(default)]
    pub result_sets: Option<ExecResultSets>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpExecuteSqlStmt {
    pub sql_expr: Expr,
    pub params_def: Option<Expr>,
    pub args: Vec<ExecArgument>,
    #[serde(default)]
    pub result_sets: Option<ExecResultSets>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclareStmt {
    pub name: String,
    pub data_type: DataTypeSpec,
    pub default: Option<Expr>,
    #[serde(default)]
    pub custom_type: Option<ObjectName>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetStmt {
    pub name: String,
    pub expr: Expr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetOptionStmt {
    pub option: crate::ast::SessionOption,
    pub value: crate::ast::SessionOptionValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclareTableVarStmt {
    pub name: String,
    pub columns: Vec<ColumnSpec>,
    pub table_constraints: Vec<TableConstraintSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProcedureStmt {
    pub name: ObjectName,
    pub params: Vec<crate::ast::RoutineParam>,
    pub body: Vec<crate::ast::Statement>,
    #[serde(default)]
    pub or_alter: bool,
    #[serde(default)]
    pub definition_sql: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropProcedureStmt {
    pub name: ObjectName,
    #[serde(default)]
    pub if_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FunctionBody {
    ScalarReturn(Expr),
    Scalar(Vec<crate::ast::Statement>),
    InlineTable(Box<SelectStmt>),
    InlineTableWithCte(Box<WithCteStmt>),
    MultiStatementTable {
        table_var: String,
        columns: Vec<crate::ast::ColumnSpec>,
        constraints: Vec<crate::ast::TableConstraintSpec>,
        body: Vec<crate::ast::Statement>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateFunctionStmt {
    pub name: ObjectName,
    pub params: Vec<crate::ast::RoutineParam>,
    pub returns: Option<DataTypeSpec>,
    pub body: FunctionBody,
    #[serde(default)]
    pub or_alter: bool,
    #[serde(default)]
    pub is_schema_bound: bool,
    #[serde(default)]
    pub definition_sql: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropFunctionStmt {
    pub name: ObjectName,
    #[serde(default)]
    pub if_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithCteStmt {
    pub recursive: bool,
    pub ctes: Vec<CteDef>,
    pub body: Box<crate::ast::Statement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CteDef {
    pub name: String,
    #[serde(default)]
    pub columns: Vec<String>,
    pub query: crate::ast::Statement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectAssignStmt {
    pub targets: Vec<SelectAssignTarget>,
    pub from: Option<TableRef>,
    pub joins: Vec<crate::ast::statements::query::JoinClause>,
    #[serde(default)]
    pub applies: Vec<crate::ast::statements::query::ApplyClause>,
    pub selection: Option<Expr>,
    #[serde(default)]
    pub order_by: Vec<crate::ast::statements::query::OrderByExpr>,
    #[serde(default)]
    pub top: Option<crate::ast::statements::query::TopSpec>,
    #[serde(default)]
    pub max_recursion: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectAssignTarget {
    pub variable: String,
    pub expr: Expr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaiserrorStmt {
    pub message: Expr,
    pub severity: Expr,
    pub state: Expr,
    #[serde(default)]
    pub args: Vec<Expr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThrowStmt {
    pub error_number: Option<Expr>,
    pub message: Option<Expr>,
    pub state: Option<Expr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TryCatchStmt {
    pub try_body: Vec<crate::ast::Statement>,
    pub catch_body: Vec<crate::ast::Statement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbccCheckIdentStmt {
    pub table: Expr,
    pub reseed: bool,
    pub new_reseed_value: Option<Expr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclareCursorStmt {
    pub name: String,
    pub query: SelectStmt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchCursorStmt {
    pub name: String,
    pub direction: FetchDirection,
    pub into: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FetchDirection {
    Next,
    Prior,
    First,
    Last,
    Absolute(Expr),
    Relative(Expr),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTriggerStmt {
    pub name: ObjectName,
    pub table: ObjectName,
    pub events: Vec<TriggerEvent>,
    pub is_instead_of: bool,
    pub body: Vec<crate::ast::Statement>,
    #[serde(default)]
    pub or_alter: bool,
    #[serde(default)]
    pub is_database_scope: bool,
    #[serde(default)]
    pub ddl_events: Vec<String>,
    #[serde(default)]
    pub definition_sql: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropTriggerStmt {
    pub name: ObjectName,
    #[serde(default)]
    pub if_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitForStmt {
    pub is_time: bool,
    pub expr: Expr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TriggerEvent {
    Insert,
    Update,
    Delete,
}
