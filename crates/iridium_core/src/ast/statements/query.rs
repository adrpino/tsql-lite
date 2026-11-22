use crate::ast::common::TableRef;
use crate::ast::expressions::Expr;
use crate::ast::ObjectName;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SelectStmt {
    pub from_clause: Option<FromNode>,
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
    pub set_op: Option<Box<SetOpClause>>,
    #[serde(default)]
    pub extra: Option<Box<SelectExtra>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SelectExtra {
    #[serde(default)]
    pub for_xml: Option<ForXmlClause>,
    #[serde(default)]
    pub for_json: Option<ForJsonClause>,
    #[serde(default)]
    pub grouping_sets: Vec<Vec<Expr>>,
}

impl SelectStmt {
    pub fn for_xml(&self) -> Option<&ForXmlClause> {
        self.extra.as_deref().and_then(|e| e.for_xml.as_ref())
    }

    pub fn for_json(&self) -> Option<&ForJsonClause> {
        self.extra.as_deref().and_then(|e| e.for_json.as_ref())
    }

    pub fn grouping_sets(&self) -> &[Vec<Expr>] {
        self.extra
            .as_deref()
            .map(|e| e.grouping_sets.as_slice())
            .unwrap_or(&[])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ForXmlClause {
    pub mode: String,
    pub path: Option<String>,
    pub root: Option<String>,
    pub type_directive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ForJsonClause {
    pub mode: String,
    pub root: Option<String>,
    pub include_null_values: bool,
    pub without_array_wrapper: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FromNode {
    Table(TableRef),
    Aliased {
        source: Box<FromNode>,
        alias: String,
    },
    Join {
        left: Box<FromNode>,
        join_type: JoinType,
        right: Box<FromNode>,
        on: Option<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JoinClause {
    pub join_type: JoinType,
    pub table: TableRef,
    pub on: Option<Expr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApplyClause {
    pub apply_type: ApplyType,
    pub subquery: Box<SelectStmt>,
    pub alias: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApplyType {
    Cross,
    Outer,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SelectItem {
    pub expr: Expr,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TopSpec {
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrderByExpr {
    pub expr: Expr,
    pub asc: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SetOpClause {
    pub kind: SetOpKind,
    pub right: SelectStmt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SetOpKind {
    Union,
    UnionAll,
    Intersect,
    Except,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetOpStmt {
    pub left: Box<crate::ast::Statement>,
    pub op: SetOpKind,
    pub right: Box<crate::ast::Statement>,
}
