use super::super::ScriptExecutor;
use super::routine::{execute_xp_msver, procedure_return_value};
use super::shared::{resolve_table_identifier, validate_table_matches_type};
use super::system_procedures::{execute_system_procedure, is_system_procedure};
use crate::ast::{ExecProcedureStmt, RoutineParamType};
use crate::catalog::RoutineKind;
use crate::error::DbError;
use crate::executor::context::{ExecutionContext, ModuleFrame, ModuleKind};
use crate::executor::evaluator::eval_expr;
use crate::executor::result::QueryResult;
use crate::executor::value_ops::coerce_value_to_type_with_dateformat;
use crate::types::Value;

fn hoist_procedure_declares(stmts: &[crate::ast::Statement], ctx: &mut ExecutionContext<'_>) {
    use crate::ast::{ProceduralStatement, Statement};
    for stmt in stmts {
        match stmt {
            Statement::Procedural(ProceduralStatement::Declare(decl)) => {
                if decl.custom_type.is_none() && ctx.session.get_var(&decl.name).is_none() {
                    let ty =
                        crate::executor::type_mapping::data_type_spec_to_runtime(&decl.data_type);
                    ctx.session
                        .variables
                        .insert(decl.name.clone(), (ty, Value::Null));
                    ctx.register_declared_var(&decl.name);
                }
            }
            Statement::Procedural(ProceduralStatement::BeginEnd(inner)) => {
                hoist_procedure_declares(inner, ctx);
            }
            Statement::Procedural(ProceduralStatement::If(if_stmt)) => {
                hoist_procedure_declares(&if_stmt.then_body, ctx);
                if let Some(ref else_b) = if_stmt.else_body {
                    hoist_procedure_declares(else_b, ctx);
                }
            }
            Statement::Procedural(ProceduralStatement::While(w)) => {
                hoist_procedure_declares(&w.body, ctx);
            }
            Statement::Procedural(ProceduralStatement::TryCatch(tc)) => {
                hoist_procedure_declares(&tc.try_body, ctx);
                hoist_procedure_declares(&tc.catch_body, ctx);
            }
            _ => {}
        }
    }
}

impl<'a> ScriptExecutor<'a> {
    pub(crate) fn execute_procedure(
        &mut self,
        stmt: ExecProcedureStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<crate::executor::result::QueryResult>, DbError> {
        if stmt.name.name.eq_ignore_ascii_case("xp_msver") {
            self.assign_exec_return_value(&stmt.return_variable, Value::Int(0), ctx)?;
            let mut res = execute_xp_msver();
            res.return_status = Some(0);
            res.is_procedure = true;
            return Ok(Some(res));
        }
        if stmt.name.name.eq_ignore_ascii_case("xp_qv") {
            let (mut result, return_code) = self.execute_xp_qv(&stmt.args, ctx)?;
            self.assign_exec_return_value(&stmt.return_variable, Value::Int(return_code), ctx)?;
            if let Some(res) = &mut result {
                res.return_status = Some(return_code);
                res.is_procedure = true;
            } else {
                result = Some(QueryResult {
                    return_status: Some(return_code),
                    is_procedure: true,
                    ..Default::default()
                });
            }
            return Ok(result);
        }
        if stmt
            .name
            .name
            .eq_ignore_ascii_case("sp_MSIsContainedAGSession")
        {
            self.assign_exec_return_value(&stmt.return_variable, Value::Int(0), ctx)?;
            return Ok(None);
        }

        if stmt.name.name.eq_ignore_ascii_case("sp_executesql")
            && stmt
                .name
                .schema
                .as_deref()
                .is_none_or(|s| s.eq_ignore_ascii_case("sys") || s.eq_ignore_ascii_case("dbo"))
        {
            let mut sql_expr: Option<crate::ast::Expr> = None;
            let mut params_def: Option<crate::ast::Expr> = None;
            let mut remaining = Vec::new();
            for (idx, arg) in stmt.args.into_iter().enumerate() {
                if let Some(ref name) = arg.name {
                    let norm = name.trim_start_matches('@');
                    if norm.eq_ignore_ascii_case("stmt") || norm.eq_ignore_ascii_case("statement") {
                        sql_expr = Some(arg.expr);
                        continue;
                    }
                    if norm.eq_ignore_ascii_case("params") {
                        params_def = Some(arg.expr);
                        continue;
                    }
                    remaining.push(arg);
                } else if idx == 0 && sql_expr.is_none() {
                    sql_expr = Some(arg.expr);
                } else if idx == 1 && params_def.is_none() {
                    params_def = Some(arg.expr);
                } else {
                    remaining.push(arg);
                }
            }
            let Some(sql_expr) = sql_expr else {
                return Err(DbError::Execution(
                    "sp_executesql requires a SQL statement argument".into(),
                ));
            };
            let sp_stmt = crate::ast::SpExecuteSqlStmt {
                sql_expr,
                params_def,
                args: remaining,
            };
            let res = super::sp_executesql::execute_sp_executesql(self, sp_stmt, ctx)?;
            self.assign_exec_return_value(&stmt.return_variable, Value::Int(0), ctx)?;
            return Ok(res);
        }

        if is_system_procedure(&stmt.name.name) {
            self.assign_exec_return_value(&stmt.return_variable, Value::Int(0), ctx)?;
            return execute_system_procedure(self, &stmt, ctx);
        }

        let schema = stmt.name.schema_or_dbo().to_string();
        let Some(routine) = self.catalog.find_routine(&schema, &stmt.name.name).cloned() else {
            return Err(DbError::object_not_found(format!(
                "procedure '{}.{}'",
                schema, stmt.name.name
            )));
        };
        let crate::catalog::RoutineDef {
            object_id,
            schema: routine_schema,
            name: routine_name,
            params,
            kind,
            ..
        } = routine;
        let RoutineKind::Procedure { body } = kind else {
            return Err(DbError::object_not_found(format!(
                "'{}.{}' is not a procedure",
                schema, stmt.name.name
            )));
        };
        ctx.push_module(ModuleFrame {
            object_id,
            schema: routine_schema.clone(),
            name: routine_name.clone(),
            kind: ModuleKind::Procedure,
        });
        let scope_depth = ctx.frame.scope_vars.len();
        let result = (|| {
            let mut bound_args: Vec<Option<&crate::ast::ExecArgument>> = vec![None; params.len()];
            let mut seen_named = false;
            for (pos, arg) in stmt.args.iter().enumerate() {
                if let Some(ref arg_name) = arg.name {
                    seen_named = true;
                    let norm_arg = arg_name.trim_start_matches('@');
                    let Some(param_idx) = params.iter().position(|p| {
                        p.name
                            .trim_start_matches('@')
                            .eq_ignore_ascii_case(norm_arg)
                    }) else {
                        return Err(DbError::Execution(format!(
                            "unknown parameter '{}' for procedure '{}.{}'",
                            arg_name, routine_schema, routine_name
                        )));
                    };
                    if bound_args[param_idx].is_some() {
                        return Err(DbError::Execution(format!(
                            "duplicate argument for parameter '{}'",
                            params[param_idx].name
                        )));
                    }
                    bound_args[param_idx] = Some(arg);
                } else {
                    if seen_named {
                        return Err(DbError::Execution(
                            "positional argument cannot follow a named argument".into(),
                        ));
                    }
                    if pos >= params.len() {
                        return Err(DbError::Execution(format!(
                            "too many arguments specified for procedure '{}.{}'",
                            routine_schema, routine_name
                        )));
                    }
                    bound_args[pos] = Some(arg);
                }
            }

            enum PreparedArg {
                Scalar {
                    name: String,
                    ty: crate::types::DataType,
                    val: Value,
                    output_caller: Option<String>,
                },
                Tvp {
                    name: String,
                    physical: String,
                    is_readonly: bool,
                },
            }

            let mut prepared: Vec<PreparedArg> = Vec::with_capacity(params.len());
            let mut default_tvp_physicals: Vec<String> = Vec::new();
            for (idx, param) in params.iter().enumerate() {
                let arg = bound_args[idx];
                let Some(arg) = arg else {
                    if let RoutineParamType::TableType(type_name) = &param.param_type {
                        // In T-SQL, omitted table-valued parameters default to an empty table
                        // matching the parameter's declared table type.
                        let tdef = self
                            .catalog
                            .find_table_type(type_name.schema_or_dbo(), &type_name.name)
                            .cloned()
                            .ok_or_else(|| {
                                DbError::Execution(format!(
                                    "table type '{}.{}' not found",
                                    type_name.schema_or_dbo(),
                                    type_name.name
                                ))
                            })?;
                        let unique = ctx.next_table_var_id();
                        let physical = format!(
                            "__tvp_default_{}_{}_{}",
                            ctx.frame.depth,
                            param.name.trim_start_matches('@'),
                            unique
                        );
                        let create = crate::ast::CreateTableStmt {
                            name: crate::ast::ObjectName {
                                database: None,
                                schema: Some("dbo".to_string()),
                                name: physical.clone(),
                            },
                            columns: tdef.columns,
                            table_constraints: tdef.table_constraints,
                        };
                        self.schema(ctx).create_table(create)?;
                        default_tvp_physicals.push(physical.clone());
                        prepared.push(PreparedArg::Tvp {
                            name: param.name.clone(),
                            physical,
                            is_readonly: param.is_readonly,
                        });
                        continue;
                    }
                    if let Some(def) = &param.default {
                        let RoutineParamType::Scalar(dt) = &param.param_type else {
                            return Err(DbError::Execution(format!(
                                "missing argument for table-valued parameter '{}'",
                                param.name
                            )));
                        };
                        let val = eval_expr(def, &[], ctx, self.catalog, self.storage, self.clock)?;
                        let ty = crate::executor::type_mapping::data_type_spec_to_runtime(dt);
                        let coerced = coerce_value_to_type_with_dateformat(
                            val,
                            &ty,
                            &ctx.options.dateformat,
                        )?;
                        prepared.push(PreparedArg::Scalar {
                            name: param.name.clone(),
                            ty,
                            val: coerced,
                            output_caller: None,
                        });
                        continue;
                    }
                    return Err(DbError::Execution(format!(
                        "missing argument for parameter '{}'",
                        param.name
                    )));
                };
                match &param.param_type {
                    RoutineParamType::Scalar(dt) => {
                        let val =
                            eval_expr(&arg.expr, &[], ctx, self.catalog, self.storage, self.clock)?;
                        let ty = crate::executor::type_mapping::data_type_spec_to_runtime(dt);
                        let coerced = truncate_param_string(
                            coerce_value_to_type_with_dateformat(
                                val,
                                &ty,
                                &ctx.options.dateformat,
                            )?,
                            &ty,
                        );
                        let output_caller = if param.is_output && arg.is_output {
                            if let crate::ast::Expr::Identifier(ref caller) = arg.expr {
                                Some(caller.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        prepared.push(PreparedArg::Scalar {
                            name: param.name.clone(),
                            ty,
                            val: coerced,
                            output_caller,
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
                        let table = self.catalog.find_table("dbo", &physical).ok_or_else(|| {
                            DbError::Execution(format!("table '{}' not found", physical))
                        })?;
                        let tdef = self
                            .catalog
                            .find_table_type(type_name.schema_or_dbo(), &type_name.name)
                            .ok_or_else(|| {
                                DbError::Execution(format!(
                                    "table type '{}.{}' not found",
                                    type_name.schema_or_dbo(),
                                    type_name.name
                                ))
                            })?;
                        validate_table_matches_type(self.catalog, table, tdef)?;
                        prepared.push(PreparedArg::Tvp {
                            name: param.name.clone(),
                            physical,
                            is_readonly: param.is_readonly,
                        });
                    }
                }
            }

            // Isolate callee variable scope from caller while preserving @@ system keys.
            let mut saved_caller_vars = std::mem::take(ctx.session.variables);
            for (k, v) in &saved_caller_vars {
                if k.starts_with("@@") {
                    ctx.session.variables.insert(k.clone(), v.clone());
                }
            }

            ctx.enter_scope();
            let mut output_bindings: Vec<(String, String)> = vec![];
            for item in prepared {
                match item {
                    PreparedArg::Scalar {
                        name,
                        ty,
                        val,
                        output_caller,
                    } => {
                        ctx.session.variables.insert(name.clone(), (ty, val));
                        ctx.register_declared_var(&name);
                        if let Some(caller) = output_caller {
                            output_bindings.push((name, caller));
                        }
                    }
                    PreparedArg::Tvp {
                        name,
                        physical,
                        is_readonly,
                    } => {
                        ctx.register_table_var(&name, &physical);
                        if is_readonly {
                            ctx.mark_table_var_readonly(&name);
                        }
                    }
                }
            }

            hoist_procedure_declares(&body, ctx);
            let saved_last_proc_result = ctx.frame.last_proc_result.take();
            let proc_result = self.execute_batch(&body, ctx);
            let captured_proc_result = ctx.frame.last_proc_result.take();
            ctx.frame.last_proc_result = saved_last_proc_result;
            let mut out_values: Vec<(String, Value)> = vec![];
            for (inner_name, caller_var) in &output_bindings {
                if let Some((_, v)) = ctx.session.get_var(inner_name) {
                    out_values.push((caller_var.clone(), v.clone()));
                }
            }
            let updated_sys_vars: Vec<(String, (crate::types::DataType, Value))> = ctx
                .session
                .variables
                .iter()
                .filter(|(k, _)| k.starts_with("@@"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let cleanup_res = self.leave_scope_and_cleanup(ctx);
            for physical in default_tvp_physicals {
                let _ = self.schema(ctx).drop_table(crate::ast::DropTableStmt {
                    name: crate::ast::ObjectName {
                        database: None,
                        schema: Some("dbo".to_string()),
                        name: physical,
                    },
                    if_exists: true,
                });
            }
            for (k, v) in updated_sys_vars {
                saved_caller_vars.insert(k, v);
            }
            *ctx.session.variables = saved_caller_vars;
            cleanup_res?;
            for (caller_var, val) in out_values {
                if let Some((ty, out_var)) = ctx.session.get_var_mut(&caller_var) {
                    *out_var =
                        coerce_value_to_type_with_dateformat(val, ty, &ctx.options.dateformat)?;
                }
            }

            let (result, return_value) = match proc_result {
                Ok(crate::error::StmtOutcome::Return(v)) => {
                    (captured_proc_result, procedure_return_value(v))
                }
                Ok(crate::error::StmtOutcome::Break) | Ok(crate::error::StmtOutcome::Continue) => {
                    (captured_proc_result, Value::Int(0))
                }
                Ok(crate::error::StmtOutcome::Ok(r)) => {
                    (r.or(captured_proc_result), Value::Int(0))
                }
                Err(e) => return Err(e),
            };
            self.assign_exec_return_value(&stmt.return_variable, return_value.clone(), ctx)?;
            let mut final_result = result;
            if let Some(res) = &mut final_result {
                res.return_status = Some(return_value.to_integer_i64().unwrap_or(0) as i32);
                res.is_procedure = true;
            } else {
                final_result = Some(QueryResult {
                    return_status: Some(return_value.to_integer_i64().unwrap_or(0) as i32),
                    is_procedure: true,
                    ..Default::default()
                });
            }
            Ok(final_result)
        })();
        while ctx.frame.scope_vars.len() > scope_depth {
            ctx.leave_scope();
        }
        for physical in ctx.pop_module_collect_temp_tables() {
            if let Ok(tid) = self.catalog.drop_table("dbo", &physical) {
                let _ = self.storage.remove_table(tid);
            }
        }
        result
    }

    fn assign_exec_return_value(
        &mut self,
        return_variable: &Option<String>,
        value: Value,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<(), DbError> {
        let Some(return_variable) = return_variable else {
            return Ok(());
        };
        if let Some((ty, out_var)) = ctx.session.get_var_mut(return_variable) {
            *out_var = coerce_value_to_type_with_dateformat(value, ty, &ctx.options.dateformat)?;
            return Ok(());
        }
        Err(DbError::invalid_identifier(return_variable))
    }

    fn execute_xp_qv(
        &mut self,
        args: &[crate::ast::ExecArgument],
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<(Option<QueryResult>, i32), DbError> {
        let query_id = match args.first() {
            Some(arg) => eval_expr(&arg.expr, &[], ctx, self.catalog, self.storage, self.clock)?
                .to_string_value(),
            None => {
                return Err(DbError::Execution(
                    "xp_qv requires at least one query identifier argument".into(),
                ))
            }
        };

        match query_id.as_str() {
            // SSMS Object Explorer AlwaysOn probe:
            // DECLARE @alwayson INT; EXEC @alwayson = master.dbo.xp_qv N'3641190370', @@SERVICENAME;
            // SELECT ISNULL(@alwayson, -1) AS [AlwaysOn]
            "3641190370" => Ok((None, -1)),
            _ => Ok((None, -1)),
        }
    }
}

fn truncate_param_string(val: Value, ty: &crate::types::DataType) -> Value {
    let max_len = match ty {
        crate::types::DataType::VarChar { max_len }
        | crate::types::DataType::NVarChar { max_len }
            if *max_len < u16::MAX =>
        {
            Some(*max_len as usize)
        }
        crate::types::DataType::Char { len } | crate::types::DataType::NChar { len } => {
            Some(*len as usize)
        }
        _ => None,
    };
    let Some(limit) = max_len else {
        return val;
    };
    match val {
        Value::VarChar(s) => Value::VarChar(s.chars().take(limit).collect()),
        Value::NVarChar(s) => Value::NVarChar(s.chars().take(limit).collect()),
        Value::Char(s) => Value::Char(s.chars().take(limit).collect()),
        Value::NChar(s) => Value::NChar(s.chars().take(limit).collect()),
        other => other,
    }
}
