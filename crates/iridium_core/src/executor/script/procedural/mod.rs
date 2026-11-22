pub(crate) mod assignment;
pub(crate) mod control_flow;
pub(crate) mod cursor;
pub(crate) mod definitions;
pub(crate) mod exec_dynamic;
pub(crate) mod print;
pub(crate) mod procedure;
pub(crate) mod raiserror;
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
            ProceduralStatement::Return(expr) => self.execute_return(expr, ctx),
            ProceduralStatement::ExecDynamic(stmt) => execute_exec_dynamic(self, stmt, ctx),
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
                self.execute_create_procedure(*stmt)?;
                Ok(StmtOutcome::Ok(None))
            }
            ProceduralStatement::CreateFunction(stmt) => {
                self.execute_create_function(*stmt)?;
                Ok(StmtOutcome::Ok(None))
            }
            ProceduralStatement::CreateView(stmt) => {
                self.execute_create_view(*stmt, ctx)?;
                Ok(StmtOutcome::Ok(None))
            }
            ProceduralStatement::CreateTrigger(stmt) => {
                self.execute_create_trigger(*stmt)?;
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
}
