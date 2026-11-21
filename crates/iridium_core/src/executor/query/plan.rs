use crate::ast::{
    ApplyClause, Expr, ForJsonClause, ForXmlClause, FromNode, ObjectName, OrderByExpr, SelectItem,
    SelectStmt, SetOpClause, TopSpec,
};

#[derive(Debug, Clone)]
pub(crate) struct ProjectionPlan {
    pub(crate) items: Vec<SelectItem>,
    pub(crate) distinct: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FilterPlan {
    pub(crate) selection: Option<Expr>,
    pub(crate) group_by: Vec<Expr>,
    pub(crate) grouping_sets: Vec<Vec<Expr>>,
    pub(crate) having: Option<Expr>,
}

#[derive(Debug, Clone)]
pub(crate) struct SortPlan {
    pub(crate) order_by: Vec<OrderByExpr>,
}

#[derive(Debug, Clone)]
pub(crate) struct PaginationPlan {
    pub(crate) top: Option<TopSpec>,
    pub(crate) offset: Option<Expr>,
    pub(crate) fetch: Option<Expr>,
}

#[derive(Debug, Clone)]
pub(crate) struct RelationalQuery {
    pub(crate) from_clause: Option<FromNode>,
    pub(crate) applies: Vec<ApplyClause>,
    pub(crate) projection: ProjectionPlan,
    pub(crate) filter: FilterPlan,
    pub(crate) sort: SortPlan,
    pub(crate) pagination: PaginationPlan,
    pub(crate) into_table: Option<ObjectName>,
    pub(crate) set_op: Option<Box<SetOpClause>>,
    pub(crate) for_xml: Option<Box<ForXmlClause>>,
    pub(crate) for_json: Option<Box<ForJsonClause>>,
}

impl From<SelectStmt> for RelationalQuery {
    fn from(stmt: SelectStmt) -> Self {
        let (for_xml, for_json, grouping_sets) = match stmt.extra {
            Some(ex) => (
                ex.for_xml.map(Box::new),
                ex.for_json.map(Box::new),
                ex.grouping_sets,
            ),
            None => (None, None, Vec::new()),
        };
        Self {
            from_clause: stmt.from_clause,
            applies: stmt.applies,
            projection: ProjectionPlan {
                items: stmt.projection,
                distinct: stmt.distinct,
            },
            filter: FilterPlan {
                selection: stmt.selection,
                group_by: stmt.group_by,
                grouping_sets,
                having: stmt.having,
            },
            sort: SortPlan {
                order_by: stmt.order_by,
            },
            pagination: PaginationPlan {
                top: stmt.top,
                offset: stmt.offset,
                fetch: stmt.fetch,
            },
            into_table: stmt.into_table,
            set_op: stmt.set_op,
            for_xml,
            for_json,
        }
    }
}
