use crate::error::DbError;
use crate::types::Value;

use super::super::super::context::ExecutionContext;

pub(crate) fn eval_error_message(ctx: &ExecutionContext) -> Result<Value, DbError> {
    Ok(match &ctx.frame.last_error {
        Some(e) => Value::VarChar(e.to_string()),
        None => Value::Null,
    })
}

pub(crate) fn eval_error_number(ctx: &ExecutionContext) -> Result<Value, DbError> {
    Ok(match &ctx.frame.last_error {
        Some(err) => Value::Int(err.number()),
        None => Value::Null,
    })
}

pub(crate) fn eval_error_severity(ctx: &ExecutionContext) -> Result<Value, DbError> {
    Ok(match &ctx.frame.last_error {
        Some(err) => Value::Int(err.class_severity() as i32),
        None => Value::Null,
    })
}

pub(crate) fn eval_error_state(ctx: &ExecutionContext) -> Result<Value, DbError> {
    Ok(match &ctx.frame.last_error {
        Some(_) => Value::Int(1), // Default state
        None => Value::Null,
    })
}

pub(crate) fn eval_error_procedure(ctx: &ExecutionContext) -> Result<Value, DbError> {
    if ctx.frame.last_error.is_none() {
        return Ok(Value::Null);
    }
    Ok(match ctx.frame.module_stack.last() {
        Some(frame) => Value::VarChar(frame.name.clone()),
        None => Value::Null,
    })
}

pub(crate) fn eval_error_line(ctx: &ExecutionContext) -> Result<Value, DbError> {
    Ok(match &ctx.frame.last_error {
        Some(_) => Value::Int(1),
        None => Value::Null,
    })
}
