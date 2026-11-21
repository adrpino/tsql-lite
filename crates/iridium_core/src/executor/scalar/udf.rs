use crate::ast::{Expr, FunctionBody, RoutineParamType};
use crate::catalog::{Catalog, RoutineKind};
use crate::error::DbError;
use crate::storage::Storage;
use crate::types::Value;

use super::super::clock::Clock;
use super::super::context::{ExecutionContext, ModuleFrame, ModuleKind};
use super::super::evaluator::eval_expr;
use super::super::model::ContextTable;
use super::super::type_mapping;
use super::super::value_ops;

pub(crate) fn eval_user_scalar_function(
    name: &str,
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let parts: Vec<&str> = name.split('.').collect();
    let (schema, fname) = match parts.len() {
        3 => (parts[1], parts[2]),
        2 => (parts[0], parts[1]),
        _ => ("dbo", name),
    };

    // Special handling for common SSMS/MSDB system functions that might be missing
    if name.contains("fn_syspolicy_is_automation_enabled") {
        return Ok(Value::Int(1));
    }

    let Some(routine) = catalog.find_routine(schema, fname) else {
        return Err(DbError::Execution(format!("function '{}' not found", name)));
    };
    let RoutineKind::Function { body, .. } = &routine.kind else {
        return Err(DbError::Execution(format!("'{}' is not a function", name)));
    };
    if args.len() != routine.params.len() {
        return Err(DbError::Execution(format!(
            "function '{}' expected {} args, got {}",
            name,
            routine.params.len(),
            args.len()
        )));
    }

    // Evaluate all argument expressions in the caller's variable scope before switching
    // to an isolated function scope (so same-named parameters cannot shadow or clobber
    // caller variables or earlier arguments).
    let mut evaluated_params = Vec::with_capacity(routine.params.len());
    for (param, arg_expr) in routine.params.iter().zip(args.iter()) {
        let RoutineParamType::Scalar(dt) = &param.param_type else {
            return Err(DbError::Execution(format!(
                "function '{}' has unsupported non-scalar parameter '{}'",
                name, param.name
            )));
        };
        let val = eval_expr(arg_expr, row, ctx, catalog, storage, clock)?;
        let ty = type_mapping::data_type_spec_to_runtime(dt);
        let coerced =
            value_ops::coerce_value_to_type_with_dateformat(val, &ty, &ctx.options.dateformat)?;
        evaluated_params.push((param.name.clone(), ty, coerced));
    }

    ctx.push_module(ModuleFrame {
        object_id: routine.object_id,
        schema: routine.schema.clone(),
        name: routine.name.clone(),
        kind: ModuleKind::Function,
    });
    let saved_caller_vars = std::mem::take(ctx.session.variables);
    for (k, v) in &saved_caller_vars {
        if k.starts_with("@@") {
            ctx.session.variables.insert(k.clone(), v.clone());
        }
    }
    let scope_depth = ctx.frame.scope_vars.len();
    let out = (|| {
        ctx.enter_scope();
        for (param_name, ty, coerced) in evaluated_params {
            ctx.session
                .variables
                .insert(param_name.clone(), (ty, coerced));
            ctx.register_declared_var(&param_name);
        }
        let out = match body {
            FunctionBody::ScalarReturn(expr) => eval_expr(expr, row, ctx, catalog, storage, clock),
            FunctionBody::Scalar(stmts) => {
                super::super::evaluator::eval_udf_body(stmts, ctx, catalog, storage, clock)
            }
            FunctionBody::InlineTable(_)
            | FunctionBody::InlineTableWithCte(_)
            | FunctionBody::MultiStatementTable { .. } => Err(DbError::Execution(format!(
                "table-valued function '{}' cannot be used in scalar context",
                name
            ))),
        };
        ctx.leave_scope();
        out
    })();
    while ctx.frame.scope_vars.len() > scope_depth {
        ctx.leave_scope();
    }
    *ctx.session.variables = saved_caller_vars;
    ctx.pop_module();
    out
}
