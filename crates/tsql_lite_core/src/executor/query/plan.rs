use std::borrow::Cow;

use crate::ast::{
    ApplyClause, Expr, ForJsonClause, ForXmlClause, FromNode, ObjectName, OrderByExpr, SelectItem,
    SelectStmt, SetOpClause, TopSpec,
};

#[derive(Debug, Clone)]
pub(crate) struct ProjectionPlan<'a> {
    pub(crate) items: Cow<'a, [SelectItem]>,
    pub(crate) distinct: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FilterPlan<'a> {
    pub(crate) selection: Option<Cow<'a, Expr>>,
    pub(crate) group_by: Cow<'a, [Expr]>,
    pub(crate) grouping_sets: Cow<'a, [Vec<Expr>]>,
    pub(crate) having: Option<Cow<'a, Expr>>,
}

#[derive(Debug, Clone)]
pub(crate) struct SortPlan<'a> {
    pub(crate) order_by: Cow<'a, [OrderByExpr]>,
}

#[derive(Debug, Clone)]
pub(crate) struct PaginationPlan<'a> {
    pub(crate) top: Option<Cow<'a, TopSpec>>,
    pub(crate) offset: Option<Cow<'a, Expr>>,
    pub(crate) fetch: Option<Cow<'a, Expr>>,
}

#[derive(Debug, Clone)]
pub(crate) struct RelationalQuery<'a> {
    pub(crate) from_clause: Option<Cow<'a, FromNode>>,
    pub(crate) applies: Cow<'a, [ApplyClause]>,
    pub(crate) projection: ProjectionPlan<'a>,
    pub(crate) filter: FilterPlan<'a>,
    pub(crate) sort: SortPlan<'a>,
    pub(crate) pagination: PaginationPlan<'a>,
    pub(crate) into_table: Option<Cow<'a, ObjectName>>,
    pub(crate) set_op: Option<Cow<'a, SetOpClause>>,
    pub(crate) for_xml: Option<Cow<'a, ForXmlClause>>,
    pub(crate) for_json: Option<Cow<'a, ForJsonClause>>,
}

impl<'a> RelationalQuery<'a> {
    /// Zero-clone borrowed view over a parsed [`SelectStmt`].
    pub(crate) fn from_ref(stmt: &'a SelectStmt) -> Self {
        let (for_xml, for_json, grouping_sets) = match stmt.extra.as_deref() {
            Some(ex) => (
                ex.for_xml.as_ref().map(Cow::Borrowed),
                ex.for_json.as_ref().map(Cow::Borrowed),
                Cow::Borrowed(ex.grouping_sets.as_slice()),
            ),
            None => (None, None, Cow::Borrowed(&[][..])),
        };
        Self {
            from_clause: stmt.from_clause.as_ref().map(Cow::Borrowed),
            applies: Cow::Borrowed(stmt.applies.as_slice()),
            projection: ProjectionPlan {
                items: Cow::Borrowed(stmt.projection.as_slice()),
                distinct: stmt.distinct,
            },
            filter: FilterPlan {
                selection: stmt.selection.as_ref().map(Cow::Borrowed),
                group_by: Cow::Borrowed(stmt.group_by.as_slice()),
                grouping_sets,
                having: stmt.having.as_ref().map(Cow::Borrowed),
            },
            sort: SortPlan {
                order_by: Cow::Borrowed(stmt.order_by.as_slice()),
            },
            pagination: PaginationPlan {
                top: stmt.top.as_ref().map(Cow::Borrowed),
                offset: stmt.offset.as_ref().map(Cow::Borrowed),
                fetch: stmt.fetch.as_ref().map(Cow::Borrowed),
            },
            into_table: stmt.into_table.as_ref().map(Cow::Borrowed),
            set_op: stmt.set_op.as_deref().map(Cow::Borrowed),
            for_xml,
            for_json,
        }
    }
}

impl From<SelectStmt> for RelationalQuery<'static> {
    fn from(stmt: SelectStmt) -> Self {
        let (for_xml, for_json, grouping_sets) = match stmt.extra {
            Some(ex) => (
                ex.for_xml.map(Cow::Owned),
                ex.for_json.map(Cow::Owned),
                Cow::Owned(ex.grouping_sets),
            ),
            None => (None, None, Cow::Borrowed(&[][..])),
        };
        Self {
            from_clause: stmt.from_clause.map(Cow::Owned),
            applies: Cow::Owned(stmt.applies),
            projection: ProjectionPlan {
                items: Cow::Owned(stmt.projection),
                distinct: stmt.distinct,
            },
            filter: FilterPlan {
                selection: stmt.selection.map(Cow::Owned),
                group_by: Cow::Owned(stmt.group_by),
                grouping_sets,
                having: stmt.having.map(Cow::Owned),
            },
            sort: SortPlan {
                order_by: Cow::Owned(stmt.order_by),
            },
            pagination: PaginationPlan {
                top: stmt.top.map(Cow::Owned),
                offset: stmt.offset.map(Cow::Owned),
                fetch: stmt.fetch.map(Cow::Owned),
            },
            into_table: stmt.into_table.map(Cow::Owned),
            set_op: stmt.set_op.map(|b| Cow::Owned(*b)),
            for_xml,
            for_json,
        }
    }
}
