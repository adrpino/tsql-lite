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
                if let Some((ty, _)) = ctx.session.get_var(&t.variable) {
                    let target_ty = ty.clone();
                    crate::executor::value_ops::coercion::xml::check_implicit_xml_var_conversion(
                        &t.expr, &target_ty, ctx,
                    )?;
                }
                let val = eval_expr(&t.expr, &[], ctx, self.catalog, self.storage, self.clock)?;
                if let Some((ty, var)) = ctx.session.get_var_mut(&t.variable) {
                    *var = coerce_value_to_type_with_dateformat(val, ty, &ctx.options.dateformat)?;
                } else {
                    return Err(DbError::invalid_identifier(&t.variable));
                }
            }
            return Ok(None);
        }

        let order_by = stmt.order_by;
        let top = stmt.top;
        let q = RelationalQuery {
            from_clause: build_from_clause(stmt.from, &stmt.joins),
            applies: stmt.applies,
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
                grouping_sets: vec![],
                having: None,
            },
            sort: SortPlan {
                order_by: order_by.clone(),
            },
            pagination: PaginationPlan {
                top: top.clone(),
                offset: None,
                fetch: None,
            },
            into_table: None,
            set_op: None,
            for_xml: None,
            for_json: None,
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
            let mut joined_rows = qe.execute_to_joined_rows(q, ctx)?;
            if !order_by.is_empty() && joined_rows.len() > 1 {
                let mut keyed = Vec::with_capacity(joined_rows.len());
                for r in joined_rows {
                    let mut keys = Vec::with_capacity(order_by.len());
                    for ob in &order_by {
                        keys.push(eval_expr(
                            &ob.expr,
                            &r,
                            ctx,
                            self.catalog as &dyn Catalog,
                            self.storage as &dyn Storage,
                            self.clock,
                        )?);
                    }
                    keyed.push((keys, r));
                }
                keyed.sort_by(|(ka, _), (kb, _)| {
                    for (idx, ob) in order_by.iter().enumerate() {
                        let ord = crate::executor::value_ops::compare_values(&ka[idx], &kb[idx]);
                        let ord = if ob.asc { ord } else { ord.reverse() };
                        if ord != std::cmp::Ordering::Equal {
                            return ord;
                        }
                    }
                    std::cmp::Ordering::Equal
                });
                joined_rows = keyed.into_iter().map(|(_, r)| r).collect();
            }
            let top_limit = if let Some(ref t) = top {
                let tv = eval_expr(
                    &t.value,
                    &[],
                    ctx,
                    self.catalog as &dyn Catalog,
                    self.storage as &dyn Storage,
                    self.clock,
                )?;
                Some(tv.to_integer_i64().unwrap_or(0).max(0) as usize)
            } else {
                None
            };
            let mut limit = joined_rows.len();
            if let Some(tl) = top_limit {
                limit = limit.min(tl);
            }
            if ctx.options.rowcount > 0 {
                limit = limit.min(ctx.options.rowcount as usize);
            }
            ctx.set_last_rowcount(limit);
            let masked_slice = crate::executor::masking::apply_dynamic_masking_to_rows(
                self.catalog as &dyn Catalog,
                ctx,
                &joined_rows[..limit],
            );
            for row in masked_slice.iter() {
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
