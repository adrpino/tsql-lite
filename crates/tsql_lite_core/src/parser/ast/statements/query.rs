use super::super::common::{ObjectName, TableRef};
use super::super::expressions::Expr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectStmt {
    pub from: Option<TableRef>,
    pub joins: Vec<JoinClause>,
    pub applies: Vec<ApplyClause>,
    pub projection: Vec<SelectItem>,
    pub into_table: Option<ObjectName>,
    pub distinct: bool,
    pub top: Option<TopSpec>,
    pub selection: Option<Expr>,
    pub group_by: Vec<Expr>,
    pub having: Option<Expr>,
    pub order_by: Vec<OrderByExpr>,
    pub offset: Option<Expr>,
    pub fetch: Option<Expr>,
    pub set_op: Option<Box<SetOp>>,
    pub extra: Option<Box<SelectExtra>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct SelectExtra {
    pub for_xml: Option<ForXmlClause>,
    pub for_json: Option<ForJsonClause>,
    pub grouping_sets: Vec<Vec<Expr>>,
    pub windows: Vec<crate::parser::ast::NamedWindowDef>,
    pub max_recursion: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ForXmlClause {
    pub mode: String,
    pub path: Option<String>,
    pub root: Option<String>,
    pub type_directive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ForJsonClause {
    pub mode: String,
    pub root: Option<String>,
    pub include_null_values: bool,
    pub without_array_wrapper: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JoinClause {
    pub join_type: JoinType,
    pub table: TableRef,
    pub on: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ApplyClause {
    pub apply_type: ApplyType,
    pub table: TableRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApplyType {
    Cross,
    Outer,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SetOp {
    pub kind: SetOpKind,
    pub right: SelectStmt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SetOpKind {
    Union,
    UnionAll,
    Intersect,
    Except,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SelectItem {
    pub expr: Expr,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TopSpec {
    pub value: Expr,
    pub percent: bool,
    pub with_ties: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OrderByExpr {
    pub expr: Expr,
    pub asc: bool,
}
