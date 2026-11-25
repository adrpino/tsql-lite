pub(crate) mod assignment;
pub(crate) mod control_flow;
pub(crate) mod cursor;
pub(crate) mod definitions;
pub(crate) mod exec_dynamic;
pub(crate) mod print;
pub(crate) mod procedure;
pub(crate) mod raiserror;
pub(crate) mod result_sets;
pub(crate) mod routine;
pub(crate) mod shared;
pub(crate) mod sp_executesql;
pub(crate) mod system_procedures;
pub(crate) mod throw;
pub(crate) mod try_catch;
pub(crate) mod variable;

use self::exec_dynamic::execute_exec_dynamic;
use self::sp_executesql::execute_sp_executesql;
use super::ScriptExecutor;
use crate::ast::ProceduralStatement;
use crate::error::DbError;
use crate::executor::context::ExecutionContext;

impl<'a> ScriptExecutor<'a> {
    pub(crate) fn execute_procedural(
        &mut self,
        proc: ProceduralStatement,
        ctx: &mut ExecutionContext<'_>,
    ) -> crate::error::StmtResult<Option<crate::executor::result::QueryResult>> {
        use crate::error::StmtOutcome;

        match proc {
            ProceduralStatement::Declare(stmt) => {
                self.execute_declare(*stmt, ctx).map(StmtOutcome::Ok)
            }
            ProceduralStatement::DeclareTableVar(stmt) => self
                .execute_declare_table_var(*stmt, ctx)
                .map(StmtOutcome::Ok),
            ProceduralStatement::Set(stmt) => self.execute_set(*stmt, ctx).map(StmtOutcome::Ok),
            ProceduralStatement::SetOption(_) => Err(DbError::Execution(
                "SET option statements are handled at engine level".into(),
            )),
            ProceduralStatement::If(stmt) => self.execute_if(*stmt, ctx),
            ProceduralStatement::BeginEnd(stmts) => self.execute_batch(&stmts, ctx),
            ProceduralStatement::While(stmt) => self.execute_while(*stmt, ctx),
            ProceduralStatement::Break => self.execute_break(ctx),
            ProceduralStatement::Continue => self.execute_continue(ctx),
            ProceduralStatement::Goto(label) => Ok(StmtOutcome::Goto(label)),
            ProceduralStatement::Label(_) => Ok(StmtOutcome::Ok(None)),
            ProceduralStatement::WaitFor(stmt) => {
                self.execute_waitfor(*stmt, ctx).map(StmtOutcome::Ok)
            }
            ProceduralStatement::Return(expr) => self.execute_return(expr, ctx),
            ProceduralStatement::ExecDynamic(stmt) => execute_exec_dynamic(self, *stmt, ctx),
            ProceduralStatement::ExecProcedure(stmt) => {
                self.execute_procedure(*stmt, ctx).map(StmtOutcome::Ok)
            }
            ProceduralStatement::SpExecuteSql(stmt) => {
                execute_sp_executesql(self, *stmt, ctx).map(StmtOutcome::Ok)
            }
            ProceduralStatement::Print(expr) => self.execute_print(expr, ctx).map(StmtOutcome::Ok),
            ProceduralStatement::Raiserror(stmt) => {
                self.execute_raiserror(*stmt, ctx).map(StmtOutcome::Ok)
            }
            ProceduralStatement::Throw(stmt) => self.execute_throw(*stmt, ctx).map(StmtOutcome::Ok),
            ProceduralStatement::TryCatch(stmt) => self.execute_try_catch(*stmt, ctx),
            ProceduralStatement::DbccCheckIdent(stmt) => self
                .execute_dbcc_checkident(*stmt, ctx)
                .map(StmtOutcome::Ok),
            ProceduralStatement::DeclareCursor(stmt) => self.execute_declare_cursor(*stmt, ctx),
            ProceduralStatement::CreateProcedure(stmt) => {
                let sch = stmt.name.schema_or_dbo().to_string();
                let ev = if stmt.or_alter
                    && self.catalog.find_routine(&sch, &stmt.name.name).is_some()
                {
                    "ALTER_PROCEDURE"
                } else {
                    "CREATE_PROCEDURE"
                };
                let meta = Some(crate::executor::script::ddl::triggers::DdlEventMeta {
                    event_type: ev,
                    schema_name: Some(sch.clone()),
                    object_name: stmt.name.name.clone(),
                    object_type: "PROCEDURE",
                    command_text: format!("{} {}.{}", ev.replace('_', " "), sch, stmt.name.name),
                });
                self.run_with_ddl_triggers(meta, ctx, |this, _ctx| {
                    this.execute_create_procedure(*stmt)
                })?;
                Ok(StmtOutcome::Ok(None))
            }
            ProceduralStatement::CreateFunction(stmt) => {
                let sch = stmt.name.schema_or_dbo().to_string();
                let ev = if stmt.or_alter
                    && self.catalog.find_routine(&sch, &stmt.name.name).is_some()
                {
                    "ALTER_FUNCTION"
                } else {
                    "CREATE_FUNCTION"
                };
                let meta = Some(crate::executor::script::ddl::triggers::DdlEventMeta {
                    event_type: ev,
                    schema_name: Some(sch.clone()),
                    object_name: stmt.name.name.clone(),
                    object_type: "FUNCTION",
                    command_text: format!("{} {}.{}", ev.replace('_', " "), sch, stmt.name.name),
                });
                self.run_with_ddl_triggers(meta, ctx, |this, _ctx| {
                    this.execute_create_function(*stmt)
                })?;
                Ok(StmtOutcome::Ok(None))
            }
            ProceduralStatement::CreateView(stmt) => {
                let sch = stmt.name.schema_or_dbo().to_string();
                let ev = if stmt.or_alter && self.catalog.find_view(&sch, &stmt.name.name).is_some()
                {
                    "ALTER_VIEW"
                } else {
                    "CREATE_VIEW"
                };
                let meta = Some(crate::executor::script::ddl::triggers::DdlEventMeta {
                    event_type: ev,
                    schema_name: Some(sch.clone()),
                    object_name: stmt.name.name.clone(),
                    object_type: "VIEW",
                    command_text: format!("{} {}.{}", ev.replace('_', " "), sch, stmt.name.name),
                });
                self.run_with_ddl_triggers(meta, ctx, |this, ctx| {
                    this.execute_create_view(*stmt, ctx)
                })?;
                Ok(StmtOutcome::Ok(None))
            }
            ProceduralStatement::CreateTrigger(stmt) => {
                let sch = stmt.name.schema_or_dbo().to_string();
                let is_db_trig = stmt.is_database_scope;
                let ev = if stmt.or_alter {
                    "ALTER_TRIGGER"
                } else {
                    "CREATE_TRIGGER"
                };
                let meta = if is_db_trig {
                    None
                } else {
                    Some(crate::executor::script::ddl::triggers::DdlEventMeta {
                        event_type: ev,
                        schema_name: Some(sch.clone()),
                        object_name: stmt.name.name.clone(),
                        object_type: "TRIGGER",
                        command_text: format!(
                            "{} {}.{}",
                            ev.replace('_', " "),
                            sch,
                            stmt.name.name
                        ),
                    })
                };
                self.run_with_ddl_triggers(meta, ctx, |this, _ctx| {
                    this.execute_create_trigger(*stmt)
                })?;
                Ok(StmtOutcome::Ok(None))
            }
        }
    }

    pub(crate) fn leave_scope_and_cleanup(
        &mut self,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<(), DbError> {
        self.cleanup_scope_table_vars(ctx)
    }

    pub(crate) fn execute_dbcc_checkident(
        &mut self,
        stmt: crate::ast::DbccCheckIdentStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<crate::executor::result::QueryResult>, DbError> {
        let raw_table = match &stmt.table {
            crate::ast::Expr::Identifier(id) => id.clone(),
            crate::ast::Expr::QualifiedIdentifier(parts) => parts.join("."),
            other => crate::executor::evaluator::eval_expr(
                other,
                &[],
                ctx,
                self.catalog,
                self.storage,
                self.clock,
            )?
            .to_string_value(),
        };
        let trimmed = raw_table.trim();
        let (schema, name) = if trimmed.starts_with('#') {
            let phys = ctx
                .resolve_table_name(trimmed)
                .unwrap_or_else(|| trimmed.to_string());
            ("dbo".to_string(), phys)
        } else if let Some((s, n)) = trimmed.rsplit_once('.') {
            (
                s.trim().trim_matches(&['[', ']'][..]).to_string(),
                n.trim().trim_matches(&['[', ']'][..]).to_string(),
            )
        } else {
            (
                "dbo".to_string(),
                trimmed.trim_matches(&['[', ']'][..]).to_string(),
            )
        };

        let (table_id, ident_col_idx) = {
            let table = self
                .catalog
                .find_table(&schema, &name)
                .ok_or_else(|| DbError::table_not_found(&schema, &name))?;
            let col_idx = table
                .columns
                .iter()
                .position(|c| c.identity.is_some())
                .ok_or_else(|| {
                    DbError::Execution(format!(
                        "Table '{}.{}' does not contain an identity column.",
                        schema, name
                    ))
                })?;
            (table.id, col_idx)
        };

        if stmt.reseed {
            let explicit_val = if let Some(ref expr) = stmt.new_reseed_value {
                let v = crate::executor::evaluator::eval_expr(
                    expr,
                    &[],
                    ctx,
                    self.catalog,
                    self.storage,
                    self.clock,
                )?;
                let n = v.to_integer_i64().ok_or_else(|| {
                    DbError::Execution("DBCC CHECKIDENT new_reseed_value must be an integer".into())
                })?;
                Some(n)
            } else {
                None
            };

            let max_in_table = if explicit_val.is_none() {
                let mut max_v: Option<i64> = None;
                for row in self.storage.get_rows(table_id)? {
                    if !row.deleted {
                        if let Some(val) = row
                            .values
                            .get(ident_col_idx)
                            .and_then(|v| v.to_integer_i64())
                        {
                            max_v = Some(max_v.map_or(val, |m| m.max(val)));
                        }
                    }
                }
                max_v
            } else {
                None
            };

            let table_mut = self
                .catalog
                .find_table_mut(&schema, &name)
                .ok_or_else(|| DbError::table_not_found(&schema, &name))?;
            if let Some(ref mut ident) = table_mut.columns[ident_col_idx].identity {
                if let Some(reseed_to) = explicit_val {
                    ident.current = reseed_to + ident.increment;
                } else if let Some(max_v) = max_in_table {
                    let needed_next = max_v + ident.increment;
                    if (ident.increment > 0 && needed_next > ident.current)
                        || (ident.increment < 0 && needed_next < ident.current)
                    {
                        ident.current = needed_next;
                    }
                } else {
                    ident.current = ident.seed;
                }
            }
        }
        Ok(None)
    }

    fn execute_waitfor(
        &mut self,
        stmt: crate::ast::WaitForStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<crate::executor::result::QueryResult>, DbError> {
        let val = crate::executor::evaluator::eval_expr(
            &stmt.expr,
            &[],
            ctx,
            self.catalog,
            self.storage,
            self.clock,
        )?;
        if val.is_null() || stmt.is_time {
            return Ok(None);
        }
        let duration_ms = parse_waitfor_delay_ms(&val.to_string_value());
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut remaining = duration_ms;
            while remaining > 0 {
                if ctx.is_cancelled() {
                    return Err(DbError::Execution("Operation cancelled by user.".into()));
                }
                let step = remaining.min(20);
                std::thread::sleep(std::time::Duration::from_millis(step));
                remaining = remaining.saturating_sub(step);
            }
            if ctx.is_cancelled() {
                return Err(DbError::Execution("Operation cancelled by user.".into()));
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = duration_ms;
        }
        Ok(None)
    }
}

fn parse_waitfor_delay_ms(raw: &str) -> u64 {
    let trimmed = raw.trim();
    // Accept 'HH:MM:SS[.fff]' or 'MM:SS' or 'SS'
    let parts: Vec<&str> = trimmed.split(':').collect();
    let (h, m, sec_part) = match parts.len() {
        3 => (
            parts[0].trim().parse::<u64>().unwrap_or(0),
            parts[1].trim().parse::<u64>().unwrap_or(0),
            parts[2].trim(),
        ),
        2 => (
            0,
            parts[0].trim().parse::<u64>().unwrap_or(0),
            parts[1].trim(),
        ),
        1 => (0, 0, parts[0].trim()),
        _ => return 0,
    };
    let (s, ms) = if let Some((s_str, f_str)) = sec_part.split_once('.') {
        let s = s_str.parse::<u64>().unwrap_or(0);
        let digits: String = f_str
            .chars()
            .filter(|c| c.is_ascii_digit())
            .take(3)
            .collect();
        let ms = match digits.len() {
            1 => digits.parse::<u64>().unwrap_or(0) * 100,
            2 => digits.parse::<u64>().unwrap_or(0) * 10,
            3 => digits.parse::<u64>().unwrap_or(0),
            _ => 0,
        };
        (s, ms)
    } else {
        (sec_part.parse::<u64>().unwrap_or(0), 0)
    };
    ((h * 3600) + (m * 60) + s) * 1000 + ms
}
