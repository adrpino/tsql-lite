use crate::ast::Expr;

use crate::executor::aggregates::is_aggregate_function;
use crate::executor::projection::{expand_projection_columns, resolve_projected_order_index};
use crate::executor::query::plan::RelationalQuery;
use crate::executor::window::has_window_function;

#[derive(Debug, Clone)]
pub(crate) struct PipelineState {
    pub(crate) has_aggregate: bool,
    pub(crate) has_window: bool,
    pub(crate) needs_pre_sort: bool,
}

fn expr_contains_aggregate(expr: &Expr) -> bool {
    match expr {
        Expr::FunctionCall { name, args, .. } => {
            is_aggregate_function(name) || args.iter().any(expr_contains_aggregate)
        }
        Expr::Binary { left, right, .. } => {
            expr_contains_aggregate(left) || expr_contains_aggregate(right)
        }
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::TryCast { expr, .. }
        | Expr::Convert { expr, .. }
        | Expr::TryConvert { expr, .. }
        | Expr::IsNull(expr)
        | Expr::IsNotNull(expr) => expr_contains_aggregate(expr),
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => {
            operand.as_deref().is_some_and(expr_contains_aggregate)
                || when_clauses.iter().any(|wc| {
                    expr_contains_aggregate(&wc.condition) || expr_contains_aggregate(&wc.result)
                })
                || else_result.as_deref().is_some_and(expr_contains_aggregate)
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            expr_contains_aggregate(expr)
                || expr_contains_aggregate(low)
                || expr_contains_aggregate(high)
        }
        Expr::InList { expr, list, .. } => {
            expr_contains_aggregate(expr) || list.iter().any(expr_contains_aggregate)
        }
        Expr::Like {
            expr,
            pattern,
            escape,
            ..
        } => {
            expr_contains_aggregate(expr)
                || expr_contains_aggregate(pattern)
                || escape.as_deref().is_some_and(expr_contains_aggregate)
        }
        Expr::WindowFunction { .. } | Expr::Subquery(_) | Expr::Exists { .. } => false,
        _ => false,
    }
}

impl PipelineState {
    pub(crate) fn new(query: &RelationalQuery) -> Self {
        let has_aggregate = query
            .projection
            .items
            .iter()
            .any(|item| expr_contains_aggregate(&item.expr));

        let has_window = query
            .projection
            .items
            .iter()
            .any(|item| has_window_function(&item.expr));

        let projection_columns: Vec<String> = query
            .projection
            .items
            .iter()
            .flat_map(|item| expand_projection_columns(std::slice::from_ref(item), None))
            .collect();

        let needs_pre_sort = requires_pre_sort(query, &projection_columns, has_aggregate);

        Self {
            has_aggregate,
            has_window,
            needs_pre_sort,
        }
    }
}

fn requires_pre_sort(
    query: &RelationalQuery,
    projection_columns: &[String],
    has_aggregate: bool,
) -> bool {
    !query.sort.order_by.is_empty()
        && !has_aggregate
        && query
            .sort
            .order_by
            .iter()
            .any(|ob| resolve_projected_order_index(projection_columns, ob).is_none())
}
