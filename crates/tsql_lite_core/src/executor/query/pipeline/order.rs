use std::cmp::Ordering;

use crate::error::DbError;
use crate::types::Value;

use crate::executor::context::ExecutionContext;
use crate::executor::model::JoinedRow;
use crate::executor::projection::resolve_projected_order_index_with_projection;
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
    if source_rows.len() <= 1 {
        return Ok(());
    }
    let order_by = &query.sort.order_by;
    // Schwartzian transform: evaluate each row's sort keys once (N evals) rather than inside
    // the comparator (2 * N log N evals).
    let collations: Vec<Option<crate::executor::collation::CollationFlags>> = order_by
        .iter()
        .map(|item| crate::executor::collation::expr_collation(&item.expr))
        .collect();
    let keys: Vec<Vec<Value>> = extract_source_order_keys(executor, query, source_rows, ctx);
    let mut indices: Vec<usize> = (0..source_rows.len()).collect();
    indices.sort_by(|&ia, &ib| {
        let ka = &keys[ia];
        let kb = &keys[ib];
        for (idx, item) in order_by.iter().enumerate() {
            let va = &ka[idx];
            let vb = &kb[idx];
            let ord = if let Some(flags) = collations[idx] {
                crate::executor::collation::compare_values_with_collation(va, vb, flags)
            } else {
                value_ops::compare_values(va, vb)
            };
            if ord != Ordering::Equal {
                return if item.asc { ord } else { ord.reverse() };
            }
        }
        Ordering::Equal
    });
    let mut reordered: Vec<JoinedRow> = Vec::with_capacity(source_rows.len());
    for &i in &indices {
        reordered.push(std::mem::take(&mut source_rows[i]));
    }
    source_rows.swap_with_slice(&mut reordered);
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
    if final_rows.len() <= 1 || order_by_refs.is_empty() {
        return Ok(final_rows);
    }
    // Pre-resolve column indices and collation flags once before sorting so the O(N log N)
    // comparator performs zero identifier allocations or linear column scans.
    let mut specs: Vec<(usize, bool, Option<crate::executor::collation::CollationFlags>)> =
        Vec::with_capacity(order_by_refs.len());
    for item in order_by_refs {
        let Some(idx) =
            resolve_projected_order_index_with_projection(columns, &query.projection.items, item)
        else {
            return Ok(final_rows);
        };
        let flags = crate::executor::collation::expr_collation(&item.expr);
        specs.push((idx, item.asc, flags));
    }
    if specs.len() == 1 {
        let (idx, asc, flags) = specs[0];
        final_rows.sort_by(|a, b| {
            let va = a.get(idx).unwrap_or(&Value::Null);
            let vb = b.get(idx).unwrap_or(&Value::Null);
            let ord = if let Some(f) = flags {
                crate::executor::collation::compare_values_with_collation(va, vb, f)
            } else {
                value_ops::compare_values(va, vb)
            };
            if asc {
                ord
            } else {
                ord.reverse()
            }
        });
    } else {
        final_rows.sort_by(|a, b| {
            for &(idx, asc, flags) in &specs {
                let va = a.get(idx).unwrap_or(&Value::Null);
                let vb = b.get(idx).unwrap_or(&Value::Null);
                let ord = if let Some(f) = flags {
                    crate::executor::collation::compare_values_with_collation(va, vb, f)
                } else {
                    value_ops::compare_values(va, vb)
                };
                if ord != Ordering::Equal {
                    return if asc { ord } else { ord.reverse() };
                }
            }
            Ordering::Equal
        });
    }
    Ok(final_rows)
}
