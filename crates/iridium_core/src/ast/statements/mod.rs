pub mod ddl;
pub mod dml;
pub mod procedural;
pub mod query;
pub mod visitor;

pub use visitor::*;

use crate::ast::data_types::DataTypeSpec;
use crate::ast::expressions::Expr;
use crate::ast::statements::ddl::*;
use crate::ast::statements::dml::*;
use crate::ast::statements::procedural::*;
use crate::ast::statements::query::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Statement {
    Dml(DmlStatement),
    Ddl(Box<DdlStatement>),
    Procedural(ProceduralStatement),
    Transaction(TransactionStatement),
    Cursor(CursorStatement),
    Session(SessionStatement),
    WithCte(Box<WithCteStmt>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DmlStatement {
    Select(Box<SelectStmt>),
    Insert(Box<InsertStmt>),
    Update(Box<UpdateStmt>),
    Delete(Box<DeleteStmt>),
    Merge(Box<MergeStmt>),
    SetOp(Box<SetOpStmt>),
    SelectAssign(Box<SelectAssignStmt>),
    BulkInsert(Box<BulkInsertStmt>),
    InsertBulk(Box<InsertBulkStmt>),
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DdlStatement {
    CreateTable(CreateTableStmt),
    CreateIndex(CreateIndexStmt),
    CreateType(CreateTypeStmt),
    CreateScalarType(CreateScalarTypeStmt),
    CreateSchema(CreateSchemaStmt),
    AlterSchemaTransfer(AlterSchemaTransferStmt),
    DropTable(DropTableStmt),
    DropView(DropViewStmt),
    DropProcedure(DropProcedureStmt),
    DropFunction(DropFunctionStmt),
    DropTrigger(DropTriggerStmt),
    DropIndex(DropIndexStmt),
    DropType(DropTypeStmt),
    DropSchema(DropSchemaStmt),
    TruncateTable(TruncateTableStmt),
    AlterTable(AlterTableStmt),
    CreateSynonym(CreateSynonymStmt),
    DropSynonym(DropSynonymStmt),
    CreateSequence(CreateSequenceStmt),
    AlterSequence(AlterSequenceStmt),
    DropSequence(DropSequenceStmt),
    CatalogMetadata(Box<CatalogMetadataAction>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProceduralStatement {
    Declare(Box<DeclareStmt>),
    Set(Box<SetStmt>),
    SetOption(SetOptionStmt),
    If(Box<IfStmt>),
    BeginEnd(Vec<Statement>),
    While(Box<WhileStmt>),
    Break,
    Continue,
    Goto(String),
    Label(String),
    Return(Option<Expr>),
    ExecDynamic(Box<ExecStmt>),
    ExecProcedure(Box<ExecProcedureStmt>),
    SpExecuteSql(Box<SpExecuteSqlStmt>),
    Print(Expr),
    DeclareTableVar(Box<DeclareTableVarStmt>),
    DeclareCursor(Box<DeclareCursorStmt>),
    CreateProcedure(Box<CreateProcedureStmt>),
    CreateFunction(Box<CreateFunctionStmt>),
    CreateView(Box<CreateViewStmt>),
    CreateTrigger(Box<CreateTriggerStmt>),
    Raiserror(Box<RaiserrorStmt>),
    Throw(Box<ThrowStmt>),
    TryCatch(Box<TryCatchStmt>),
    DbccCheckIdent(Box<DbccCheckIdentStmt>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransactionStatement {
    Begin(Option<String>),
    Commit(Option<String>),
    Rollback(Option<String>),
    Save(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CursorStatement {
    OpenCursor(String),
    FetchCursor(Box<FetchCursorStmt>),
    CloseCursor(String),
    DeallocateCursor(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionStatement {
    SetTransactionIsolationLevel(IsolationLevel),
    UseDatabase(String),
    SetOption(SetOptionStmt),
    SetIdentityInsert(Box<SetIdentityInsertStmt>),
    SetContextInfo(Expr),
    ExecuteAs(String),
    Revert,
    Checkpoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum IsolationLevel {
    ReadUncommitted,
    #[default]
    ReadCommitted,
    RepeatableRead,
    Serializable,
    Snapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionOption {
    AnsiNulls,
    QuotedIdentifier,
    NoCount,
    XactAbort,
    FmtOnly,
    NoExec,
    DateFirst,
    Language,
    DateFormat,
    LockTimeout,
    RowCount,
    TextSize,
    ConcatNullYieldsNull,
    ArithAbort,
    QueryGovernorCostLimit,
    DeadlockPriority,
    AnsiNullDfltOn,
    AnsiPadding,
    AnsiWarnings,
    CursorCloseOnCommit,
    ImplicitTransactions,
    StatisticsIo,
    StatisticsTime,
    ShowplanAll,
    ShowplanXml,
    StatisticsXml,
    AnsiDefaults,
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionOptionValue {
    Bool(bool),
    Int(i64),
    Text(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoutineParamType {
    Scalar(DataTypeSpec),
    TableType(crate::ast::ObjectName),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineParam {
    pub name: String,
    pub param_type: RoutineParamType,
    pub is_output: bool,
    pub is_readonly: bool,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetIdentityInsertStmt {
    pub table: crate::ast::ObjectName,
    pub on: bool,
}
