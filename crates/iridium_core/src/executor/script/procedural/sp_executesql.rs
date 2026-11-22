use super::super::ScriptExecutor;
use super::shared::{find_param_def, resolve_table_identifier, validate_table_matches_type};
use crate::ast::{RoutineParamType, SpExecuteSqlStmt};
use crate::error::{DbError, StmtOutcome};
use crate::executor::context::ExecutionContext;
use crate::executor::evaluator::eval_expr;
use crate::executor::result::QueryResult;
use crate::executor::value_ops::coerce_value_to_type_with_dateformat;

pub(crate) fn execute_sp_executesql(
    exec: &mut ScriptExecutor<'_>,
    stmt: SpExecuteSqlStmt,
    ctx: &mut ExecutionContext<'_>,
) -> Result<Option<QueryResult>, DbError> {
    let sql_val = eval_expr(
        &stmt.sql_expr,
        &[],
        ctx,
        exec.catalog,
        exec.storage,
        exec.clock,
    )?;
    let sql_text = sql_val.to_string_value();
    let declared_params = if let Some(def_expr) = &stmt.params_def {
        let def_text = eval_expr(def_expr, &[], ctx, exec.catalog, exec.storage, exec.clock)?
            .to_string_value();
        crate::parser::statements::procedural::parse_routine_params(&def_text)?
    } else {
        vec![]
    };

    // Evaluate all argument expressions in the caller's variable scope before isolating the
    // sp_executesql parameter scope so same-named caller variables (e.g. @cnt = @cnt OUTPUT)
    // are never clobbered or dropped on scope exit.
    enum BoundSpArg {
        Scalar {
            key: String,
            ty: crate::types::DataType,
            val: crate::types::Value,
            output_caller_var: Option<String>,
        },
        Tvp {
            key: String,
            physical: String,
        },
    }

    let mut bound_args = Vec::with_capacity(stmt.args.len());
    for (idx, arg) in stmt.args.into_iter().enumerate() {
        let pname = arg
            .name
            .clone()
            .or_else(|| declared_params.get(idx).map(|p| p.name.clone()))
            .unwrap_or_else(|| "".to_string());
        if pname.trim().is_empty() {
            continue;
        }
        let key = pname.trim().to_string();
        match find_param_def(&declared_params, &key) {
            Some(def) => match &def.param_type {
                RoutineParamType::Scalar(dt) => {
                    let val =
                        eval_expr(&arg.expr, &[], ctx, exec.catalog, exec.storage, exec.clock)?;
                    let ty =
                        crate::executor::type_mapping::resolve_data_type_spec(dt, exec.catalog);
                    let coerced =
                        coerce_value_to_type_with_dateformat(val, &ty, &ctx.options.dateformat)?;
                    let output_caller_var = if arg.is_output {
                        match &arg.expr {
                            crate::ast::Expr::Identifier(caller_var) => Some(caller_var.clone()),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    bound_args.push(BoundSpArg::Scalar {
                        key,
                        ty,
                        val: coerced,
                        output_caller_var,
                    });
                }
                RoutineParamType::TableType(type_name) => {
                    let logical = resolve_table_identifier(&arg.expr)?.to_string();
                    let Some(physical) = ctx.resolve_table_name(&logical) else {
                        return Err(DbError::Execution(format!(
                            "TVP argument '{}' is not a table variable",
                            logical
                        )));
                    };
                    let table = exec.catalog.find_table("dbo", &physical).ok_or_else(|| {
                        DbError::Execution(format!("table '{}' not found", physical))
                    })?;
                    let tdef = exec
                        .catalog
                        .find_table_type(type_name.schema_or_dbo(), &type_name.name)
                        .ok_or_else(|| {
                            DbError::Execution(format!(
                                "table type '{}.{}' not found",
                                type_name.schema_or_dbo(),
                                type_name.name
                            ))
                        })?;
                    validate_table_matches_type(exec.catalog, table, tdef)?;
                    bound_args.push(BoundSpArg::Tvp { key, physical });
                }
            },
            None => {
                let val = eval_expr(&arg.expr, &[], ctx, exec.catalog, exec.storage, exec.clock)?;
                let ty = val.data_type().unwrap_or(crate::types::DataType::Int);
                let output_caller_var = if arg.is_output {
                    match &arg.expr {
                        crate::ast::Expr::Identifier(caller_var) => Some(caller_var.clone()),
                        _ => None,
                    }
                } else {
                    None
                };
                bound_args.push(BoundSpArg::Scalar {
                    key,
                    ty,
                    val,
                    output_caller_var,
                });
            }
        }
    }

    let saved_caller_vars = std::mem::take(ctx.session.variables);
    for (k, v) in &saved_caller_vars {
        if k.starts_with("@@") {
            ctx.session.variables.insert(k.clone(), v.clone());
        }
    }

    let scope_depth = ctx.frame.scope_vars.len();
    let mut outs: Vec<(String, crate::types::Value)> = Vec::new();
    let result = (|| {
        ctx.enter_scope();
        let mut output_vars = vec![];
        for bound in bound_args {
            match bound {
                BoundSpArg::Scalar {
                    key,
                    ty,
                    val,
                    output_caller_var,
                } => {
                    ctx.session.variables.insert(key.clone(), (ty, val));
                    ctx.register_declared_var(&key);
                    if let Some(caller_var) = output_caller_var {
                        output_vars.push((key, caller_var));
                    }
                }
                BoundSpArg::Tvp { key, physical } => {
                    ctx.register_table_var(&key, &physical);
                    ctx.mark_table_var_readonly(&key);
                }
            }
        }
        let batch = crate::parser::parse_batch(&sql_text)?;
        let exec_result = exec.execute_batch(&batch, ctx);

        for (inner, outer) in output_vars {
            if let Some((_, v)) = ctx.session.get_var(&inner) {
                outs.push((outer, v.clone()));
            }
        }
        exec.leave_scope_and_cleanup(ctx)?;
        // Swallow control flow signals at procedure boundary
        match exec_result {
            Ok(StmtOutcome::Return(_)) | Ok(StmtOutcome::Break) | Ok(StmtOutcome::Continue) => {
                Ok(None)
            }
            Ok(StmtOutcome::Goto(label)) => Err(DbError::Execution(format!(
                "Label '{}' has not been declared.",
                label
            ))),
            Ok(StmtOutcome::Ok(r)) => Ok(r),
            Err(e) => Err(e),
        }
    })();
    while ctx.frame.scope_vars.len() > scope_depth {
        ctx.leave_scope();
    }
    // Preserve any updated @@ system variables (e.g. @@ROWCOUNT) while restoring caller's scope.
    let updated_sys_vars: Vec<_> = ctx
        .session
        .variables
        .iter()
        .filter(|(k, _)| k.starts_with("@@"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    *ctx.session.variables = saved_caller_vars;
    for (k, v) in updated_sys_vars {
        ctx.session.variables.insert(k, v);
    }
    if result.is_ok() {
        for (outer, val) in outs {
            if let Some((ty, out)) = ctx.session.get_var_mut(&outer) {
                *out = coerce_value_to_type_with_dateformat(val, ty, &ctx.options.dateformat)?;
            }
        }
    }
    result
}
