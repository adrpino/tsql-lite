use super::super::ScriptExecutor;
use super::result_sets::{
    abort_result_set_capture, begin_result_set_capture, finish_result_set_capture,
};
use crate::ast::ExecStmt;
use crate::error::{StmtOutcome, StmtResult};
use crate::executor::context::ExecutionContext;
use crate::executor::result::QueryResult;

pub(crate) fn execute_exec_dynamic(
    exec: &mut ScriptExecutor<'_>,
    stmt: ExecStmt,
    ctx: &mut ExecutionContext<'_>,
) -> StmtResult<Option<QueryResult>> {
    let sql_val = crate::executor::evaluator::eval_expr(
        &stmt.sql_expr,
        &[],
        ctx,
        exec.catalog,
        exec.storage,
        exec.clock,
    )?;
    let sql_str = sql_val.to_string_value();
    let batch = crate::parser::parse_batch(&sql_str)?;

    let capture = begin_result_set_capture(stmt.result_sets.as_ref(), ctx);
    ctx.enter_scope();
    let res = exec.execute_batch(&batch, ctx);
    exec.cleanup_scope_table_vars(ctx)?;
    match res {
        Ok(StmtOutcome::Ok(raw_opt)) => {
            let shaped = finish_result_set_capture(
                capture,
                stmt.result_sets.as_ref(),
                raw_opt,
                ctx,
                exec.catalog,
            )?;
            Ok(StmtOutcome::Ok(shaped))
        }
        Ok(other) => {
            abort_result_set_capture(capture, ctx);
            Ok(other)
        }
        Err(e) => {
            abort_result_set_capture(capture, ctx);
            Err(e)
        }
    }
}
