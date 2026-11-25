use crate::error::DbError;
use crate::executor::result::QueryResult;

use super::super::finalize;
use super::super::from_tree;
use super::super::plan::RelationalQuery;
use super::super::source;
use super::super::QueryExecutor;
use crate::executor::context::ExecutionContext;

fn compute_early_materialize_limit(
    executor: &QueryExecutor<'_>,
    query: &RelationalQuery,
    ctx: &mut ExecutionContext,
) -> Option<usize> {
    if ctx
        .session
        .extras
        .row_locks
        .as_ref()
        .is_some_and(|rt| rt.hints_by_table.values().any(|h| h.readpast))
    {
        return None;
    }
    if !query.sort.order_by.is_empty()
        || !query.filter.group_by.is_empty()
        || !query.filter.grouping_sets.is_empty()
        || query.filter.having.is_some()
        || query.projection.distinct
        || query.pagination.offset.is_some()
        || query.pagination.fetch.is_some()
    {
        return None;
    }
    let state = super::super::pipeline::analysis::PipelineState::new(query);
    if state.has_aggregate || state.has_window {
        return None;
    }
    let mut cap: Option<usize> = None;
    if let Some(ref top) = query.pagination.top {
        if !top.percent && !top.with_ties {
            if let Ok(val) = crate::executor::evaluator::eval_expr(
                &top.value,
                &[],
                ctx,
                executor.catalog,
                executor.storage,
                executor.clock,
            ) {
                if let Some(n) = val.to_integer_i64() {
                    if n >= 0 {
                        cap = Some(n as usize);
                    }
                }
            }
        }
    }
    if ctx.options.rowcount > 0 {
        let rc = ctx.options.rowcount as usize;
        cap = Some(cap.map_or(rc, |c| c.min(rc)));
    }
    cap
}

pub(crate) fn execute_select_internal(
    executor: &QueryExecutor<'_>,
    query: &RelationalQuery,
    ctx: &mut ExecutionContext,
) -> Result<QueryResult, DbError> {
    from_tree::enforce_query_governor_cost_limit(query, ctx)?;
    let early_limit = compute_early_materialize_limit(executor, query, ctx);
    let mut source_eval = source::execute_source(executor, query, ctx)?;
    let source_shape = source_eval.shape.clone();
    let rows = source_eval.materialize_limited(
        early_limit,
        ctx,
        executor.catalog,
        executor.storage,
        executor.clock,
    )?;
    let shape_opt = if source_shape.is_empty() {
        None
    } else {
        Some(&source_shape)
    };
    finalize::finalize_rows(executor, query, rows, shape_opt, ctx)
}
