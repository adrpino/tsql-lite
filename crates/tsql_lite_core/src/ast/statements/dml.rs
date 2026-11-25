use crate::ast::common::TableRef;
use crate::ast::expressions::Expr;
use crate::ast::statements::query::{ApplyClause, JoinClause, SelectStmt, TopSpec};
use crate::ast::ObjectName;
use crate::ast::Statement;
#[cfg(feature = "checkpoint")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct InsertStmt {
    pub table: ObjectName,
    pub columns: Option<Vec<String>>,
    pub source: InsertSource,
    pub output: Option<Vec<OutputColumn>>,
    pub output_into: Option<ObjectName>,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub output_into_columns: Vec<String>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub enum InsertSource {
    Values(Vec<Vec<Expr>>),
    Select(Box<SelectStmt>),
    Exec(Box<Statement>),
    DefaultValues,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct UpdateStmt {
    pub table: ObjectName,
    pub assignments: Vec<Assignment>,
    pub top: Option<TopSpec>,
    pub selection: Option<Expr>,
    pub from: Option<FromClause>,
    pub output: Option<Vec<OutputColumn>>,
    pub output_into: Option<ObjectName>,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub output_into_columns: Vec<String>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct DeleteStmt {
    pub table: ObjectName,
    pub top: Option<TopSpec>,
    pub selection: Option<Expr>,
    pub from: Option<FromClause>,
    pub output: Option<Vec<OutputColumn>>,
    pub output_into: Option<ObjectName>,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub output_into_columns: Vec<String>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct MergeStmt {
    pub target: TableRef,
    pub source: MergeSource,
    pub on_condition: Expr,
    pub when_clauses: Vec<MergeWhenClause>,
    pub output: Option<Vec<OutputColumn>>,
    pub output_into: Option<ObjectName>,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub output_into_columns: Vec<String>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub enum MergeSource {
    Table(TableRef),
    Subquery(Box<SelectStmt>, Option<String>),
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub enum MergeWhen {
    Matched,
    NotMatched,
    NotMatchedBySource,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct MergeWhenClause {
    pub when: MergeWhen,
    pub condition: Option<Expr>,
    pub action: MergeAction,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub enum MergeAction {
    Update {
        assignments: Vec<Assignment>,
    },
    Insert {
        columns: Vec<String>,
        values: Vec<Expr>,
    },
    Delete,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct FromClause {
    pub tables: Vec<TableRef>,
    pub joins: Vec<JoinClause>,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub applies: Vec<ApplyClause>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct Assignment {
    pub column: String,
    pub expr: Expr,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct OutputColumn {
    pub source: OutputSource,
    pub column: String,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub expr: Option<Expr>,
    pub alias: Option<String>,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub is_wildcard: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub enum OutputSource {
    Inserted,
    Deleted,
    Expr,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct BulkInsertStmt {
    pub table: ObjectName,
    pub from: String,
    pub options: Vec<BulkInsertOption>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
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

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct InsertBulkStmt {
    pub table: ObjectName,
    pub columns: Vec<crate::ast::statements::ddl::ColumnSpec>,
}
