use super::super::ScriptExecutor;
use crate::ast::statements::procedural::ThrowStmt;
use crate::error::DbError;
use crate::executor::context::ExecutionContext;
use crate::executor::evaluator::eval_expr;

impl<'a> ScriptExecutor<'a> {
    pub(crate) fn execute_throw(
        &mut self,
        stmt: ThrowStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<crate::executor::result::QueryResult>, DbError> {
        if stmt.error_number.is_none() {
            if let Some(ref err) = ctx.frame.last_error {
                return Err(err.clone());
            }
            return Err(DbError::Execution(
                "THROW without parameters must be inside a CATCH block with an active error".into(),
            ));
        }
        let error_number = eval_expr(
            stmt.error_number.as_ref().unwrap(),
            &[],
            ctx,
            self.catalog,
            self.storage,
            self.clock,
        )?;
        let message = eval_expr(
            stmt.message.as_ref().unwrap(),
            &[],
            ctx,
            self.catalog,
            self.storage,
            self.clock,
        )?;
        let state_val = eval_expr(
            stmt.state.as_ref().unwrap(),
            &[],
            ctx,
            self.catalog,
            self.storage,
            self.clock,
        )?;
        let state = state_val.to_integer_i64().unwrap_or(1).clamp(0, 255) as u8;
        let msg = message.to_string_value().to_string();
        Err(DbError::Raised {
            number: error_number.to_integer_i64().unwrap_or(50000) as i32,
            severity: 16,
            state,
            message: msg,
        })
    }
}
