pub(crate) mod ddl;
pub(crate) mod dml;
pub(crate) mod procedural;

use super::clock::Clock;
use super::context::ExecutionContext;
use super::result::QueryResult;
use super::schema::SchemaExecutor;
use crate::ast::statements::StatementVisitor;
use crate::ast::{
    CursorStatement, DdlStatement, DmlStatement, DropTableStmt, ObjectName, ProceduralStatement,
    SessionStatement, Statement, TransactionStatement, WithCteStmt,
};
use crate::catalog::Catalog;
use crate::error::{DbError, StmtOutcome, StmtResult};
use crate::storage::Storage;

pub struct ScriptExecutor<'a> {
    pub catalog: &'a mut dyn Catalog,
    pub storage: &'a mut dyn Storage,
    pub clock: &'a dyn Clock,
}

impl<'a> ScriptExecutor<'a> {
    pub fn execute(
        &mut self,
        stmt: Statement,
        ctx: &mut ExecutionContext<'_>,
    ) -> StmtResult<Option<QueryResult>> {
        self.visit_statement(stmt, ctx)
    }

    pub fn execute_batch(
        &mut self,
        stmts: &[Statement],
        ctx: &mut ExecutionContext<'_>,
    ) -> StmtResult<Option<QueryResult>> {
        const MAX_GOTO_JUMPS: usize = 100_000;
        let mut last_result = StmtOutcome::Ok(None);
        let mut idx = 0usize;
        let mut jumps = 0usize;
        while idx < stmts.len() {
            match self.execute(stmts[idx].clone(), ctx) {
                Ok(StmtOutcome::Goto(target)) => {
                    if let Some(pos) = stmts.iter().position(|s| {
                        matches!(
                            s,
                            Statement::Procedural(crate::ast::ProceduralStatement::Label(lbl))
                                if lbl.eq_ignore_ascii_case(&target)
                        )
                    }) {
                        jumps += 1;
                        if jumps > MAX_GOTO_JUMPS {
                            return Err(DbError::Execution(format!(
                                "GOTO loop exceeded maximum jump count ({MAX_GOTO_JUMPS}) targeting label '{target}'"
                            )));
                        }
                        idx = pos + 1;
                        continue;
                    }
                    return Ok(StmtOutcome::Goto(target));
                }
                Ok(r) => {
                    if r.is_control_flow() {
                        return Ok(r);
                    }
                    last_result = r;
                    idx += 1;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(last_result)
    }

    fn cleanup_scope_table_vars(&mut self, ctx: &mut ExecutionContext<'_>) -> Result<(), DbError> {
        let dropped_physical = ctx.leave_scope_collect_table_vars();
        for physical in dropped_physical {
            let Some(table) = self
                .catalog
                .get_tables()
                .iter()
                .find(|table| table.name.eq_ignore_ascii_case(&physical))
                .cloned()
            else {
                continue;
            };

            self.schema(ctx).drop_table(DropTableStmt {
                name: ObjectName {
                    database: None,
                    schema: Some(table.schema_name.clone()),
                    name: physical,
                },
                if_exists: false,
            })?;
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn push_dirty_insert(
        &self,
        ctx: &mut ExecutionContext<'_>,
        table_name: &str,
        row: &crate::storage::StoredRow,
    ) {
        super::dirty_buffer::push_dirty_op(
            &ctx.session.dirty_buffer,
            ctx.session_id(),
            table_name.to_string(),
            super::dirty_buffer::DirtyOp::Insert { row: row.clone() },
        );
    }

    fn snapshot_transactional_tables(
        &self,
        ctx: &ExecutionContext<'_>,
    ) -> Result<std::collections::HashMap<u32, Vec<crate::storage::StoredRow>>, DbError> {
        let mut table_var_physicals: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for scope in &ctx.frame.table_vars {
            for phys in scope.values() {
                table_var_physicals.insert(phys.to_ascii_lowercase());
            }
        }
        for phys in ctx.session.var_map.values() {
            table_var_physicals.insert(phys.to_ascii_lowercase());
        }
        let mut snapshot = std::collections::HashMap::new();
        for table in self.catalog.get_tables() {
            if table_var_physicals.contains(&table.name.to_ascii_lowercase()) {
                continue;
            }
            let rows = self.storage.get_rows(table.id)?;
            snapshot.insert(table.id, rows);
        }
        Ok(snapshot)
    }

    fn restore_transactional_tables(
        &mut self,
        snapshot: &std::collections::HashMap<u32, Vec<crate::storage::StoredRow>>,
    ) -> Result<(), DbError> {
        for (&table_id, rows) in snapshot {
            self.storage.replace_table(table_id, rows.clone())?;
        }
        Ok(())
    }

    fn schema<'b>(&'b mut self, ctx: &'b ExecutionContext<'_>) -> SchemaExecutor<'b> {
        SchemaExecutor {
            catalog: self.catalog,
            storage: self.storage,
            session_options: &ctx.options,
        }
    }
}

impl<'a> StatementVisitor<ExecutionContext<'_>> for ScriptExecutor<'a> {
    fn visit_transaction(
        &mut self,
        stmt: TransactionStatement,
        ctx: &mut ExecutionContext<'_>,
    ) -> StmtResult<Option<QueryResult>> {
        match stmt {
            TransactionStatement::Begin(_) => {
                if ctx.frame.script_tx.root_snapshot.is_none() {
                    ctx.frame.script_tx.base_depth = ctx.frame.trancount;
                    ctx.frame.script_tx.root_snapshot =
                        Some(self.snapshot_transactional_tables(ctx)?);
                    ctx.frame.script_tx.savepoints.clear();
                }
                ctx.frame.trancount += 1;
                ctx.frame.xact_state = 1;
                Ok(StmtOutcome::Ok(None))
            }
            TransactionStatement::Save(name) => {
                if ctx.frame.trancount == 0 {
                    return Err(DbError::Execution(
                        "Cannot roll back or save transaction without an active transaction".into(),
                    ));
                }
                let snap = self.snapshot_transactional_tables(ctx)?;
                ctx.frame
                    .script_tx
                    .savepoints
                    .push((name.to_ascii_lowercase(), snap));
                Ok(StmtOutcome::Ok(None))
            }
            TransactionStatement::Commit(_) => {
                if ctx.frame.trancount == 0 {
                    return Err(DbError::Execution(
                        "COMMIT without active transaction".into(),
                    ));
                }
                ctx.frame.trancount -= 1;
                if ctx.frame.trancount <= ctx.frame.script_tx.base_depth {
                    ctx.frame.script_tx.root_snapshot = None;
                    ctx.frame.script_tx.savepoints.clear();
                    if ctx.frame.trancount == 0 {
                        ctx.frame.xact_state = 0;
                    }
                }
                Ok(StmtOutcome::Ok(None))
            }
            TransactionStatement::Rollback(savepoint) => {
                if ctx.frame.trancount == 0 {
                    return Err(DbError::Execution(
                        "ROLLBACK without active transaction".into(),
                    ));
                }
                if let Some(ref sp_name) = savepoint {
                    let target = sp_name.to_ascii_lowercase();
                    if let Some(pos) = ctx
                        .frame
                        .script_tx
                        .savepoints
                        .iter()
                        .rposition(|(n, _)| *n == target)
                    {
                        let snap = ctx.frame.script_tx.savepoints[pos].1.clone();
                        self.restore_transactional_tables(&snap)?;
                        ctx.frame.script_tx.savepoints.truncate(pos + 1);
                        return Ok(StmtOutcome::Ok(None));
                    }
                }
                if let Some(root_snap) = ctx.frame.script_tx.root_snapshot.take() {
                    self.restore_transactional_tables(&root_snap)?;
                    ctx.frame.script_tx.savepoints.clear();
                    ctx.frame.trancount = ctx.frame.script_tx.base_depth;
                } else {
                    ctx.frame.script_tx.savepoints.clear();
                    ctx.frame.script_tx.rolled_back = true;
                    ctx.frame.trancount = 0;
                }
                if ctx.frame.trancount == 0 {
                    ctx.frame.xact_state = 0;
                }
                Ok(StmtOutcome::Ok(None))
            }
        }
    }

    fn visit_cursor(
        &mut self,
        stmt: CursorStatement,
        ctx: &mut ExecutionContext<'_>,
    ) -> StmtResult<Option<QueryResult>> {
        match stmt {
            CursorStatement::OpenCursor(name) => self.execute_open_cursor(name, ctx),
            CursorStatement::FetchCursor(stmt) => self.execute_fetch_cursor(stmt, ctx),
            CursorStatement::CloseCursor(name) => self.execute_close_cursor(name, ctx),
            CursorStatement::DeallocateCursor(name) => self.execute_deallocate_cursor(name, ctx),
        }
        .map(StmtOutcome::Ok)
    }

    /// Rejects session statements (`SET IDENTITY_INSERT`, `SET TRANSACTION ISOLATION LEVEL`,
    /// `SET` options) at runtime by design.
    ///
    /// These statements require access to shared state owned by the engine/database layer and are
    /// dispatched there (see `dispatch.rs`). `ScriptExecutor` does not hold that state, so
    /// execution here is intentionally unsupported. The visitor trait still requires this method
    /// for compile-time exhaustiveness over `SessionStatement`.
    fn visit_session(
        &mut self,
        stmt: SessionStatement,
        ctx: &mut ExecutionContext<'_>,
    ) -> StmtResult<Option<QueryResult>> {
        match stmt {
            SessionStatement::SetOption(opt) => {
                crate::executor::tooling::apply_set_option(&opt, &mut ctx.options)?;
                ctx.metadata.ansi_nulls = ctx.options.ansi_nulls;
                ctx.metadata.datefirst = ctx.options.datefirst;
                Ok(StmtOutcome::Ok(None))
            }
            SessionStatement::SetIdentityInsert(stmt) => {
                let table_upper = stmt.table.name.to_uppercase();
                if stmt.on {
                    ctx.options.identity_insert.clear();
                    ctx.options.identity_insert.insert(table_upper);
                } else {
                    ctx.options.identity_insert.remove(&table_upper);
                }
                ctx.session.identity_insert = ctx.options.identity_insert.clone();
                Ok(StmtOutcome::Ok(None))
            }
            SessionStatement::UseDatabase(_) => {
                Err(DbError::Execution("USE is handled at engine level".into()))
            }
            SessionStatement::SetTransactionIsolationLevel(_) => Ok(StmtOutcome::Ok(None)),
            SessionStatement::SetContextInfo(_) => Err(DbError::Execution(
                "session option statements are handled at engine level".into(),
            )),
        }
    }

    fn visit_dml(
        &mut self,
        stmt: DmlStatement,
        ctx: &mut ExecutionContext<'_>,
    ) -> StmtResult<Option<QueryResult>> {
        self.execute_dml(stmt, ctx)
    }

    fn visit_ddl(
        &mut self,
        stmt: DdlStatement,
        ctx: &mut ExecutionContext<'_>,
    ) -> StmtResult<Option<QueryResult>> {
        self.execute_ddl(stmt, ctx).map(StmtOutcome::Ok)
    }

    fn visit_procedural(
        &mut self,
        stmt: ProceduralStatement,
        ctx: &mut ExecutionContext<'_>,
    ) -> StmtResult<Option<QueryResult>> {
        self.execute_procedural(stmt, ctx)
    }

    fn visit_with_cte(
        &mut self,
        stmt: WithCteStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> StmtResult<Option<QueryResult>> {
        self.execute_with_cte(stmt, ctx).map(StmtOutcome::Ok)
    }
}
