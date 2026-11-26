use crate::error::DbError;

use crate::executor::context::ExecutionContext;
use crate::executor::grouping::GroupExecutor;
use crate::executor::model::JoinedRow;
use crate::executor::query::plan::RelationalQuery;
use crate::executor::result;
use crate::executor::window::WindowExecutor;

use super::super::QueryExecutor;
use super::analysis::PipelineState;

pub(crate) fn execute_projection_stage(
    executor: &QueryExecutor<'_>,
    query: &RelationalQuery,
    source_rows: Vec<JoinedRow>,
    source_shape: Option<&JoinedRow>,
    ctx: &mut ExecutionContext,
    state: &PipelineState,
) -> Result<result::QueryResult, DbError> {
    if !query.filter.group_by.is_empty()
        || !query.filter.grouping_sets.is_empty()
        || state.has_aggregate
    {
        let group_executor = GroupExecutor {
            catalog: executor.catalog,
            storage: executor.storage,
            clock: executor.clock,
        };
        if !query.filter.grouping_sets.is_empty() {
            return group_executor.execute_grouping_sets_select(
                query.projection.items.to_vec(),
                source_rows,
                source_shape,
                &query.filter.group_by,
                &query.filter.grouping_sets,
                query.filter.having.as_deref(),
                ctx,
            );
        }
        if state.has_window {
            let surviving_groups = group_executor.collect_surviving_groups(
                source_rows,
                &query.filter.group_by,
                query.filter.having.as_deref(),
                ctx,
            )?;
            let window_executor =
                WindowExecutor::new(executor.catalog, executor.storage, executor.clock);
            return window_executor.execute_grouped(&query.projection.items, surviving_groups, ctx);
        }
        group_executor.execute_grouped_select(
            query.projection.items.to_vec(),
            source_rows,
            source_shape,
            query.filter.group_by.to_vec(),
            query.filter.having.as_deref().cloned(),
            ctx,
        )
    } else if state.has_window {
        let window_executor =
            WindowExecutor::new(executor.catalog, executor.storage, executor.clock);
        window_executor.execute(&query.projection.items, source_rows, ctx)
    } else {
        crate::executor::query::projection::execute_flat_select(
            executor.catalog,
            executor.storage,
            executor.clock,
            &query.projection.items,
            source_rows,
            source_shape,
            ctx,
        )
    }
}
