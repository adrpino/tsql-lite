use super::super::ScriptExecutor;
use crate::ast::{Expr, JoinClause, SelectAssignStmt, SelectItem, TableRef};
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::executor::aggregates::is_aggregate_function;
use crate::executor::context::ExecutionContext;
use crate::executor::evaluator::eval_expr;
use crate::executor::query::plan::{
    FilterPlan, PaginationPlan, ProjectionPlan, RelationalQuery, SortPlan,
};
use crate::executor::query::QueryExecutor;
use crate::executor::result::QueryResult;
use crate::executor::value_ops::coerce_value_to_type_with_dateformat;
use crate::executor::window::has_window_function;
use crate::storage::Storage;

fn expr_contains_aggregate(expr: &Expr) -> bool {
    if has_window_function(expr) {
        return true;
    }
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
        | Expr::TryConvert { expr, .. } => expr_contains_aggregate(expr),
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
        Expr::IsNull(inner) => expr_contains_aggregate(inner),
        _ => false,
    }
}

fn build_from_clause(from: Option<TableRef>, joins: &[JoinClause]) -> Option<crate::ast::FromNode> {
    let mut node = from.map(crate::ast::FromNode::Table)?;
    for join in joins {
        node = crate::ast::FromNode::Join {
            left: Box::new(node),
            join_type: join.join_type,
            right: Box::new(crate::ast::FromNode::Table(join.table.clone())),
            on: join.on.clone(),
        };
    }
    Some(node)
}

impl<'a> ScriptExecutor<'a> {
    pub(crate) fn execute_select_assign(
        &mut self,
        stmt: SelectAssignStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        if stmt.targets.is_empty() {
            return Ok(None);
        }
        if stmt.from.is_none() {
            for t in stmt.targets {
                let val = eval_expr(&t.expr, &[], ctx, self.catalog, self.storage, self.clock)?;
                if let Some((ty, var)) = ctx.session.get_var_mut(&t.variable) {
                    *var = coerce_value_to_type_with_dateformat(val, ty, &ctx.options.dateformat)?;
                } else {
                    return Err(DbError::invalid_identifier(&t.variable));
                }
            }
            return Ok(None);
        }

        let q = RelationalQuery {
            from_clause: build_from_clause(stmt.from, &stmt.joins),
            applies: vec![],
            projection: ProjectionPlan {
                items: stmt
                    .targets
                    .iter()
                    .map(|t| SelectItem {
                        expr: t.expr.clone(),
                        alias: None,
                    })
                    .collect(),
                distinct: false,
            },
            filter: FilterPlan {
                selection: stmt.selection,
                group_by: vec![],
                having: None,
            },
            sort: SortPlan { order_by: vec![] },
            pagination: PaginationPlan {
                top: None,
                offset: None,
                fetch: None,
            },
            into_table: None,
            set_op: None,
            for_xml: None,
        };
        let has_agg = stmt
            .targets
            .iter()
            .any(|t| expr_contains_aggregate(&t.expr));
        let qe = QueryExecutor {
            catalog: self.catalog as &dyn Catalog,
            storage: self.storage as &dyn Storage,
            clock: self.clock,
        };
        if has_agg {
            let result = qe.execute_select(q, ctx)?;
            ctx.set_last_rowcount(result.rows.len());
            if let Some(last) = result.rows.last() {
                for (idx, t) in stmt.targets.iter().enumerate() {
                    if let Some((ty, var)) = ctx.session.get_var_mut(&t.variable) {
                        *var = coerce_value_to_type_with_dateformat(
                            last[idx].clone(),
                            ty,
                            &ctx.options.dateformat,
                        )?;
                    } else {
                        return Err(DbError::invalid_identifier(&t.variable));
                    }
                }
            }
        } else {
            let joined_rows = qe.execute_to_joined_rows(q, ctx)?;
            let limit = if ctx.options.rowcount > 0 {
                (ctx.options.rowcount as usize).min(joined_rows.len())
            } else {
                joined_rows.len()
            };
            ctx.set_last_rowcount(limit);
            for row in &joined_rows[..limit] {
                for t in &stmt.targets {
                    let val = eval_expr(
                        &t.expr,
                        row,
                        ctx,
                        self.catalog as &dyn Catalog,
                        self.storage as &dyn Storage,
                        self.clock,
                    )?;
                    if let Some((ty, var)) = ctx.session.get_var_mut(&t.variable) {
                        *var =
                            coerce_value_to_type_with_dateformat(val, ty, &ctx.options.dateformat)?;
                    } else {
                        return Err(DbError::invalid_identifier(&t.variable));
                    }
                }
            }
        }
        Ok(None)
    }
}
