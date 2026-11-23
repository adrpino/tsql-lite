use super::super::common::DataType;
use super::super::common::TableRef;
use super::super::expressions::Expr;
use super::query::{JoinClause, SelectStmt};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Statement {
    Dml(DmlStatement),
    Ddl(Box<DdlStatement>),
    Procedural(ProceduralStatement),
    Transaction(TransactionStatement),
    Cursor(CursorStatement),
    Session(SessionStatement),
    WithCte {
        ctes: Vec<CteDef>,
        body: Box<Statement>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DmlStatement {
    Select(Box<SelectStmt>),
    Insert(Box<InsertStmt>),
    Update(Box<UpdateStmt>),
    Delete(Box<DeleteStmt>),
    Merge(Box<MergeStmt>),
    BulkInsert(Box<BulkInsertStmt>),
    InsertBulk(Box<InsertBulkStmt>),
    SelectAssign(Box<SelectAssignStmt>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SelectAssignStmt {
    pub assignments: Vec<SelectAssignTarget>,
    pub from: Option<TableRef>,
    pub joins: Vec<JoinClause>,
    pub applies: Vec<super::query::ApplyClause>,
    pub selection: Option<Expr>,
    pub order_by: Vec<super::query::OrderByExpr>,
    pub top: Option<super::query::TopSpec>,
    pub max_recursion: Option<u16>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DdlStatement {
    Create(Box<CreateStmt>),
    AlterTable {
        table: Vec<String>,
        action: AlterTableAction,
    },
    TruncateTable(Vec<String>),
    DropTable {
        name: Vec<String>,
        if_exists: bool,
    },
    DropView {
        name: Vec<String>,
        if_exists: bool,
    },
    DropProcedure {
        name: Vec<String>,
        if_exists: bool,
    },
    DropFunction {
        name: Vec<String>,
        if_exists: bool,
    },
    DropTrigger {
        name: Vec<String>,
        if_exists: bool,
    },
    DropIndex {
        name: Vec<String>,
        table: Vec<String>,
        if_exists: bool,
    },
    DropType {
        name: Vec<String>,
        if_exists: bool,
    },
    DropSchema {
        name: String,
        if_exists: bool,
    },
    CreateSynonym {
        name: Vec<String>,
        base_object: Vec<String>,
    },
    DropSynonym(Vec<String>),
    CreateSequence {
        name: Vec<String>,
        data_type: Option<DataType>,
        start_with: Option<i64>,
        increment_by: Option<i64>,
        min_value: Option<i64>,
        max_value: Option<i64>,
        cycle: bool,
    },
    DropSequence(Vec<String>),
    AlterSequence {
        name: Vec<String>,
        restart: Option<Option<i64>>,
        increment_by: Option<i64>,
        min_value: Option<Option<i64>>,
        max_value: Option<Option<i64>>,
        cycle: Option<bool>,
    },
    CreateIndex(Box<CreateIndexStmt>),
    CreateType {
        name: Vec<String>,
        columns: Vec<ColumnDef>,
    },
    CreateScalarType {
        name: Vec<String>,
        base_type: DataType,
        nullable: bool,
    },
    CreateSchema(String),
    AlterSchemaTransfer {
        target_schema: String,
        entity_type: Option<String>,
        source: Vec<String>,
    },
    CatalogMetadata(Box<crate::ast::CatalogMetadataAction>),
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProceduralStatement {
    Declare(Vec<DeclareVar>),
    DeclareTableVar {
        name: String,
        columns: Vec<ColumnDef>,
        constraints: Vec<TableConstraint>,
    },
    Synonym {
        name: Vec<String>,
        base_object: Vec<String>,
    },
    Sequence {
        name: Vec<String>,
        data_type: Option<DataType>,
        start_with: Option<i64>,
        increment_by: Option<i64>,
        min_value: Option<i64>,
        max_value: Option<i64>,
        cycle: bool,
    },
    DeclareCursor {
        name: String,
        query: Box<SelectStmt>,
    },
    Set {
        variable: String,
        expr: Expr,
    },
    If {
        condition: Expr,
        then_stmt: Box<Statement>,
        else_stmt: Option<Box<Statement>>,
    },
    BeginEnd(Vec<Statement>),
    While {
        condition: Expr,
        stmt: Box<Statement>,
    },
    Break,
    Continue,
    Goto(String),
    Label(String),
    Return(Option<Expr>),
    Print(Expr),
    Raiserror {
        message: Expr,
        severity: Expr,
        state: Expr,
        args: Vec<Expr>,
    },
    Throw {
        error_number: Option<Expr>,
        message: Option<Expr>,
        state: Option<Expr>,
    },
    DbccCheckIdent {
        table: Expr,
        reseed: bool,
        new_reseed_value: Option<Expr>,
    },
    TryCatch {
        try_body: Vec<Statement>,
        catch_body: Vec<Statement>,
    },
    ExecDynamic {
        sql_expr: Expr,
        result_sets: Option<ResultSetsClause>,
    },
    ExecProcedure {
        return_variable: Option<String>,
        name: Vec<String>,
        args: Vec<ExecArg>,
        result_sets: Option<ResultSetsClause>,
    },
    SpExecuteSql {
        sql_expr: Expr,
        params_def: Option<Expr>,
        args: Vec<ExecArg>,
        result_sets: Option<ResultSetsClause>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResultSetColumnDef {
    pub name: String,
    pub data_type: crate::parser::ast::DataType,
    pub nullable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResultSetDef {
    Columns(Vec<ResultSetColumnDef>),
    AsObject(Vec<String>),
    AsType(Vec<String>),
    AsForXml,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResultSetsClause {
    Undefined,
    None,
    Definitions(Vec<ResultSetDef>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransactionStatement {
    Begin(Option<String>),
    Commit(Option<String>),
    Rollback(Option<String>),
    Save(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CursorStatement {
    Open(String),
    Fetch {
        name: String,
        direction: FetchDirection,
        into_vars: Option<Vec<String>>,
    },
    Close(String),
    Deallocate(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionStatement {
    SetTransactionIsolationLevel(IsolationLevel),
    UseDatabase(String),
    SetOption {
        option: SessionOption,
        value: SessionOptionValue,
    },
    SetIdentityInsert {
        table: Vec<String>,
        on: bool,
    },
    SetContextInfo(Expr),
    ExecuteAs(String),
    Revert,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MergeStmt {
    pub target: TableRef,
    pub source: TableRef,
    pub on_condition: Expr,
    pub when_clauses: Vec<MergeWhenClause>,
    pub output: Option<Vec<OutputColumn>>,
    pub output_into: Option<Vec<String>>,
    pub output_into_columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MergeWhenClause {
    pub when: MergeWhen,
    pub condition: Option<Expr>,
    pub action: MergeAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MergeWhen {
    Matched,
    NotMatched,
    NotMatchedBySource,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MergeAction {
    Update {
        assignments: Vec<UpdateAssignment>,
    },
    Insert {
        columns: Vec<String>,
        values: Vec<Expr>,
    },
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CteDef {
    pub name: String,
    pub columns: Vec<String>,
    pub query: SelectStmt,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InsertStmt {
    pub table: Vec<String>,
    pub columns: Vec<String>,
    pub source: InsertSource,
    pub output: Option<Vec<OutputColumn>>,
    pub output_into: Option<Vec<String>>,
    pub output_into_columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InsertSource {
    Values(Vec<Vec<Expr>>),
    Select(Box<SelectStmt>),
    Exec {
        return_variable: Option<String>,
        procedure: Vec<String>,
        args: Vec<ExecArg>,
    },
    ExecDynamic(Expr),
    DefaultValues,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BulkInsertStmt {
    pub table: Vec<String>,
    pub from: String,
    pub options: Vec<BulkInsertOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BulkInsertOption {
    CheckConstraints,
    FireTriggers,
    KeepIdentity,
    KeepNulls,
    TabLock,
    Format(String),
    DataFiletype(String),
    FieldTerminator(String),
    RowTerminator(String),
    FirstRow(i64),
    LastRow(i64),
    ErrorFile(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InsertBulkStmt {
    pub table: Vec<String>,
    pub columns: Vec<ColumnDef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UpdateStmt {
    pub table: TableRef,
    pub assignments: Vec<UpdateAssignment>,
    pub top: Option<Expr>,
    pub from: Option<Vec<TableRef>>,
    pub joins: Vec<JoinClause>,
    pub applies: Vec<crate::parser::ast::ApplyClause>,
    pub selection: Option<Expr>,
    pub output: Option<Vec<OutputColumn>>,
    pub output_into: Option<Vec<String>>,
    pub output_into_columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UpdateAssignment {
    pub column: String,
    pub expr: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeleteStmt {
    pub table: Vec<String>,
    pub top: Option<Expr>,
    pub from: Vec<TableRef>,
    pub joins: Vec<JoinClause>,
    pub applies: Vec<crate::parser::ast::ApplyClause>,
    pub selection: Option<Expr>,
    pub output: Option<Vec<OutputColumn>>,
    pub output_into: Option<Vec<String>>,
    pub output_into_columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeclareVar {
    pub name: String,
    pub data_type: DataType,
    pub initial_value: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CreateStmt {
    Table {
        name: Vec<String>,
        columns: Vec<ColumnDef>,
        constraints: Vec<TableConstraint>,
    },
    View {
        name: Vec<String>,
        columns: Vec<String>,
        ctes: Vec<CteDef>,
        query: SelectStmt,
        or_alter: bool,
        is_schema_bound: bool,
        with_check_option: bool,
    },
    Procedure {
        name: Vec<String>,
        params: Vec<RoutineParam>,
        body: Vec<Statement>,
        or_alter: bool,
    },
    Function {
        name: Vec<String>,
        params: Vec<RoutineParam>,
        returns: Option<DataType>,
        body: FunctionBody,
        or_alter: bool,
        is_schema_bound: bool,
    },
    Trigger {
        name: Vec<String>,
        table: Vec<String>,
        events: Vec<TriggerEvent>,
        is_instead_of: bool,
        body: Vec<Statement>,
        or_alter: bool,
        is_database_scope: bool,
        ddl_events: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoutineParam {
    pub name: String,
    pub data_type: DataType,
    pub is_output: bool,
    pub is_readonly: bool,
    pub default: Option<Expr>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FunctionBody {
    ScalarReturn(Expr),
    Block(Vec<Statement>),
    Table(SelectStmt),
    TableWithCte {
        ctes: Vec<CteDef>,
        query: SelectStmt,
    },
    MultiStatementTable {
        table_var: String,
        columns: Vec<ColumnDef>,
        constraints: Vec<TableConstraint>,
        body: Vec<Statement>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ColumnDef {
    pub name: String,
    pub data_type: DataType,
    pub is_nullable: Option<bool>,
    pub is_identity: bool,
    pub identity_spec: Option<(i64, i64)>,
    pub is_primary_key: bool,
    pub is_unique: bool,
    pub default_expr: Option<Expr>,
    pub default_constraint_name: Option<String>,
    pub check_expr: Option<Expr>,
    pub check_constraint_name: Option<String>,
    pub computed_expr: Option<Expr>,
    pub foreign_key: Option<ForeignKeyRef>,
    pub collation: Option<String>,
    pub is_clustered: bool,
    #[serde(default)]
    pub with_values: bool,
    #[serde(default)]
    pub masking_function: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ForeignKeyRef {
    pub ref_table: Vec<String>,
    pub ref_columns: Vec<String>,
    pub on_delete: Option<ReferentialAction>,
    pub on_update: Option<ReferentialAction>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AlterTableAction {
    AddColumn(ColumnDef),
    DropColumn(String),
    AlterColumn {
        name: String,
        data_type: DataType,
        nullable: Option<bool>,
    },
    AddConstraint {
        constraint: TableConstraint,
        with_check: Option<bool>,
    },
    DropConstraint(String),
    CheckConstraint {
        check: bool,
        with_check: Option<bool>,
        name: Option<String>,
    },
    ToggleTrigger {
        trigger_name: Option<Vec<String>>,
        enable: bool,
    },
    SetSystemVersioning {
        enabled: bool,
        history_table: Option<Vec<String>>,
    },
    SetColumnMask {
        column: String,
        masking_function: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TableConstraint {
    PrimaryKey {
        name: Option<String>,
        columns: Vec<IndexColumn>,
        is_clustered: bool,
    },
    Unique {
        name: Option<String>,
        columns: Vec<IndexColumn>,
        is_clustered: bool,
    },
    ForeignKey {
        name: Option<String>,
        columns: Vec<String>,
        ref_table: Vec<String>,
        ref_columns: Vec<String>,
        on_delete: Option<ReferentialAction>,
        on_update: Option<ReferentialAction>,
    },
    Check {
        name: Option<String>,
        expr: Expr,
    },
    Default {
        name: Option<String>,
        column: String,
        expr: Expr,
    },
    SystemTimePeriod {
        start_col: String,
        end_col: String,
    },
    SystemVersioning {
        enabled: bool,
        history_table: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReferentialAction {
    NoAction,
    Cascade,
    SetNull,
    SetDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FetchDirection {
    Next,
    Prior,
    First,
    Last,
    Absolute(Expr),
    Relative(Expr),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IndexColumn {
    pub name: String,
    pub is_desc: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IndexOption {
    FillFactor(u8),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CreateIndexStmt {
    pub name: Vec<String>,
    pub table: Vec<String>,
    pub is_unique: bool,
    pub is_clustered: bool,
    pub columns: Vec<IndexColumn>,
    pub options: Vec<IndexOption>,
    pub filter_predicate: Option<Box<Expr>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SelectAssignTarget {
    pub variable: String,
    pub expr: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExecArg {
    pub name: Option<String>,
    pub expr: Expr,
    pub is_output: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OutputColumn {
    pub source: OutputSource,
    pub column: String,
    pub expr: Option<Expr>,
    pub alias: Option<String>,
    pub is_wildcard: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OutputSource {
    Inserted,
    Deleted,
    Expr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IsolationLevel {
    ReadUncommitted,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RoutineParamType {
    Scalar(super::super::data_types::DataTypeSpec),
    TableType(super::super::common::ObjectName),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TriggerEvent {
    Insert,
    Update,
    Delete,
}
