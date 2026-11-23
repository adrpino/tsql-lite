use std::cmp::Ordering;

use crate::error::DbError;
use crate::types::Value;

use crate::executor::context::ExecutionContext;
use crate::executor::model::JoinedRow;
use crate::executor::projection::compare_projected_rows_with_projection;
use crate::executor::query::plan::RelationalQuery;
use crate::executor::value_ops;

use super::super::QueryExecutor;
use super::order_validation::validate_projected_order_by;

pub(crate) fn apply_source_ordering(
    executor: &QueryExecutor<'_>,
    query: &RelationalQuery,
    source_rows: &mut [JoinedRow],
    ctx: &mut ExecutionContext,
) -> Result<(), DbError> {
    let order_by = &query.sort.order_by;
    source_rows.sort_by(|a, b| {
        for item in order_by {
            let va = crate::executor::evaluator::eval_expr(
                &item.expr,
                a,
                ctx,
                executor.catalog,
                executor.storage,
                executor.clock,
            )
            .unwrap_or(Value::Null);
            let vb = crate::executor::evaluator::eval_expr(
                &item.expr,
                b,
                ctx,
                executor.catalog,
                executor.storage,
                executor.clock,
            )
            .unwrap_or(Value::Null);
            let ord = if let Some(flags) = crate::executor::collation::expr_collation(&item.expr) {
                crate::executor::collation::compare_values_with_collation(&va, &vb, flags)
            } else {
                value_ops::compare_values(&va, &vb)
            };
            if ord != Ordering::Equal {
                return if item.asc { ord } else { ord.reverse() };
            }
        }
        Ordering::Equal
    });
    Ok(())
}

pub(crate) fn extract_source_order_keys(
    executor: &QueryExecutor<'_>,
    query: &RelationalQuery,
    source_rows: &[JoinedRow],
    ctx: &mut ExecutionContext,
) -> Vec<Vec<Value>> {
    let order_by = &query.sort.order_by;
    source_rows
        .iter()
        .map(|row| {
            order_by
                .iter()
                .map(|item| {
                    crate::executor::evaluator::eval_expr(
                        &item.expr,
                        row,
                        ctx,
                        executor.catalog,
                        executor.storage,
                        executor.clock,
                    )
                    .unwrap_or(Value::Null)
                })
                .collect()
        })
        .collect()
}

pub(crate) fn order_keys_equal(
    order_by: &[crate::ast::OrderByExpr],
    a: &[Value],
    b: &[Value],
) -> bool {
    for (idx, item) in order_by.iter().enumerate() {
        let va = a.get(idx).unwrap_or(&Value::Null);
        let vb = b.get(idx).unwrap_or(&Value::Null);
        let ord = if let Some(flags) = crate::executor::collation::expr_collation(&item.expr) {
            crate::executor::collation::compare_values_with_collation(va, vb, flags)
        } else {
            value_ops::compare_values(va, vb)
        };
        if ord != Ordering::Equal {
            return false;
        }
    }
    true
}

pub(crate) fn apply_result_ordering(
    _executor: &QueryExecutor<'_>,
    query: &RelationalQuery,
    columns: &[String],
    mut final_rows: Vec<Vec<Value>>,
    _ctx: &mut ExecutionContext,
) -> Result<Vec<Vec<Value>>, DbError> {
    let order_by_refs = &query.sort.order_by;
    validate_projected_order_by(columns, &query.projection.items, order_by_refs)?;
    final_rows.sort_by(|a, b| {
        compare_projected_rows_with_projection(
            a,
            b,
            columns,
            &query.projection.items,
            order_by_refs,
        )
        .unwrap_or(Ordering::Equal)
    });
    Ok(final_rows)
}
