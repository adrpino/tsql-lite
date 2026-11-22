use std::cmp::Ordering;

use crate::error::DbError;

use super::order::order_keys_equal;
use super::pagination_value::value_to_usize;
use crate::executor::context::ExecutionContext;
use crate::executor::projection::{compare_projected_rows, eval_top_count};
use crate::executor::query::plan::RelationalQuery;

use super::super::QueryExecutor;

pub(crate) fn apply_pagination(
    executor: &QueryExecutor<'_>,
    query: &RelationalQuery,
    columns: &[String],
    mut final_rows: Vec<Vec<crate::types::Value>>,
    source_order_keys: Option<&[Vec<crate::types::Value>]>,
    ctx: &mut ExecutionContext,
) -> Result<Vec<Vec<crate::types::Value>>, DbError> {
    if let Some(top) = query.pagination.top.clone() {
        if top.with_ties && query.sort.order_by.is_empty() {
            return Err(DbError::Execution(
                "The TOP N WITH TIES clause is not allowed without a corresponding ORDER BY clause."
                    .into(),
            ));
        }
        let mut n = eval_top_count(
            &top,
            final_rows.len(),
            ctx,
            executor.catalog,
            executor.storage,
            executor.clock,
        )?;
        if top.with_ties && n > 0 && n < final_rows.len() {
            let order_by = &query.sort.order_by;
            if let Some(keys) = source_order_keys.filter(|k| k.len() == final_rows.len()) {
                let boundary = &keys[n - 1];
                while n < final_rows.len() && order_keys_equal(order_by, boundary, &keys[n]) {
                    n += 1;
                }
            } else {
                let boundary = &final_rows[n - 1];
                while n < final_rows.len()
                    && compare_projected_rows(boundary, &final_rows[n], columns, order_by)
                        .unwrap_or(Ordering::Less)
                        == Ordering::Equal
                {
                    n += 1;
                }
            }
        }
        if final_rows.len() > n {
            final_rows.truncate(n);
        }
    }

    if let Some(offset_expr) = query.pagination.offset.clone() {
        let offset_val = crate::executor::evaluator::eval_expr(
            &offset_expr,
            &[],
            ctx,
            executor.catalog,
            executor.storage,
            executor.clock,
        )?;
        let offset_n = value_to_usize(offset_val);
        if offset_n < final_rows.len() {
            final_rows = final_rows[offset_n..].to_vec();
        } else {
            final_rows = vec![];
        }
        if let Some(fetch_expr) = query.pagination.fetch.clone() {
            let fetch_val = crate::executor::evaluator::eval_expr(
                &fetch_expr,
                &[],
                ctx,
                executor.catalog,
                executor.storage,
                executor.clock,
            )?;
            let fetch_n = value_to_usize(fetch_val);
            if final_rows.len() > fetch_n {
                final_rows.truncate(fetch_n);
            }
        }
    }

    Ok(final_rows)
}
