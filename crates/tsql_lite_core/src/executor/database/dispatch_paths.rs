use std::collections::HashMap;
use std::sync::Arc;

use crate::ast::{DmlStatement, IsolationLevel, Statement};
use crate::error::{DbError, StmtOutcome, StmtResult};
use crate::types::{DataType, Value};

use super::super::clock::{Clock, SystemClock};
use super::super::context::ExecutionContext;
use super::super::dirty_buffer;
use super::super::journal::Journal;
use super::super::locks::hints::{collect_and_validate_statement_hints, TableLockHints};
use super::super::locks::row_runtime::RowLockRuntime;
use super::super::locks::{AcquiredLock, LockResource, LockTable, SessionId, TxWorkspace};
use super::super::result::QueryResult;
use super::super::script::ScriptExecutor;
use super::super::session::{SessionSnapshot, SharedState};
use super::super::table_util::{collect_read_tables, collect_write_tables, is_transaction_statement};
use super::super::tooling::SessionOptions;
use super::super::transaction::TransactionManager;
use super::super::transaction_exec;
use super::dispatch_helpers;
use super::{EngineCatalog, EngineStorage};
use crate::catalog::Catalog;
use crate::storage::Storage;

fn create_script_executor<'a>(
    catalog: &'a mut dyn Catalog,
    storage: &'a mut dyn Storage,
    clock: &'a dyn Clock,
) -> ScriptExecutor<'a> {
    ScriptExecutor {
        catalog,
        storage,
        clock,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_row_lock_runtime<C, S>(
    state: &SharedState<C, S>,
    session_id: SessionId,
    active_db: &str,
    isolation_level: IsolationLevel,
    rcsi_enabled: bool,
    allow_snapshot_isolation: bool,
    in_transaction: bool,
    savepoint_depth: usize,
    session_options: &SessionOptions,
    hints_by_table: HashMap<String, TableLockHints>,
    workspace_slot: &Option<TxWorkspace<C, S>>,
) -> RowLockRuntime
where
    C: EngineCatalog + 'static,
    S: EngineStorage + 'static,
{
    let priorities = Arc::clone(&state.deadlock_priorities);
    let priority_lookup: Arc<dyn Fn(SessionId) -> i32 + Send + Sync> =
        Arc::new(move |sid| priorities.get(&sid).map(|p| *p).unwrap_or(0));

    let snapshot_conflict_checker: Option<Arc<dyn Fn(u32, usize) -> bool + Send + Sync>> =
        if isolation_level == IsolationLevel::Snapshot && allow_snapshot_isolation {
            workspace_slot.as_ref().map(|ws| {
                let base_storage = ws.base_storage.clone();
                let shared_storage = Arc::clone(&state.storage);
                let f: Arc<dyn Fn(u32, usize) -> bool + Send + Sync> =
                    Arc::new(move |table_id: u32, row_idx: usize| {
                        let base_row = base_storage
                            .scan_rows(table_id)
                            .ok()
                            .and_then(|mut it| it.nth(row_idx))
                            .and_then(|r| r.ok());
                        let guard = shared_storage.read();
                        let cur_row = guard
                            .storage
                            .scan_rows(table_id)
                            .ok()
                            .and_then(|mut it| it.nth(row_idx))
                            .and_then(|r| r.ok());
                        match (base_row, cur_row) {
                            (Some(b), Some(c)) => b != c,
                            (Some(_), None) => true,
                            _ => false,
                        }
                    });
                f
            })
        } else {
            None
        };

    let shared_for_fetch = Arc::clone(&state.storage);
    let committed_row_fetcher: Option<super::super::locks::row_runtime::CommittedRowFetcher> =
        Some(Arc::new(move |table_id: u32, row_idx: usize| {
            let guard = shared_for_fetch.read();
            guard
                .storage
                .scan_rows(table_id)
                .ok()
                .and_then(|mut it| it.nth(row_idx))
                .and_then(|r| r.ok())
        }));

    RowLockRuntime {
        table_locks: Arc::clone(&state.table_locks),
        session_id,
        active_db: active_db.to_string(),
        isolation_level,
        rcsi_enabled,
        allow_snapshot_isolation,
        in_transaction,
        savepoint_depth,
        timeout_ms: session_options.lock_timeout_ms,
        deadlock_priority: session_options.deadlock_priority,
        priority_lookup,
        hints_by_table,
        acquired_locks: Arc::new(parking_lot::Mutex::new(Vec::new())),
        snapshot_conflict_checker,
        committed_row_fetcher,
    }
}

fn release_statement_row_locks(
    table_locks: &parking_lot::Mutex<LockTable>,
    session_id: SessionId,
    locks: Vec<AcquiredLock>,
) {
    if locks.is_empty() {
        return;
    }
    let mut guard = table_locks.lock();
    for lock in locks {
        if let LockResource::Row(t, ridx) = lock.resource {
            guard.release_row_lock(session_id, &t, ridx, lock.mode);
        }
    }
}

fn all_read_tables_have_nolock(
    stmt: &Statement,
    active_db: &str,
    hints_by_table: &HashMap<String, TableLockHints>,
) -> bool {
    if hints_by_table.is_empty() {
        return false;
    }
    let read_tables = collect_read_tables(stmt);
    if read_tables.is_empty() {
        return false;
    }
    let db_prefix = active_db.to_ascii_lowercase();
    read_tables.iter().all(|t| {
        let key = format!("{}.{}", db_prefix, t);
        hints_by_table.get(&key).is_some_and(|h| h.nolock)
    })
}

fn active_db_isolation_flags<C, S>(state: &SharedState<C, S>, active_db: &str) -> (bool, bool, bool)
where
    C: EngineCatalog,
    S: EngineStorage,
{
    let guard = state.storage.read();
    let opts = guard.catalog.get_database_options(active_db);
    let is_user = super::super::database_catalog::is_user_database(&guard.catalog, active_db);
    (
        opts.read_committed_snapshot,
        opts.allow_snapshot_isolation,
        is_user,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_write_without_transaction<C, S>(
    state: &SharedState<C, S>,
    session_id: SessionId,
    tx_manager: &mut TransactionManager<C, S, SessionSnapshot>,
    workspace_slot: &mut Option<TxWorkspace<C, S>>,
    session_options: &SessionOptions,
    stmt: Statement,
    ctx: &mut ExecutionContext,
    clock: &dyn Clock,
    active_db: &str,
    rcsi_enabled: bool,
    allow_snapshot_isolation: bool,
    hints_by_table: HashMap<String, TableLockHints>,
) -> StmtResult<Option<QueryResult>>
where
    C: EngineCatalog + 'static,
    S: EngineStorage + 'static,
{
    LockTable::acquire_statement_locks_with_hints(
        &state.table_locks,
        session_id,
        tx_manager,
        workspace_slot,
        &stmt,
        active_db,
        allow_snapshot_isolation,
        &hints_by_table,
        session_options.lock_timeout_ms,
        session_options.deadlock_priority,
        &|sid| dispatch_helpers::lookup_session_deadlock_priority(state, sid),
    )?;

    let rt = build_row_lock_runtime(
        state,
        session_id,
        active_db,
        tx_manager.session_isolation_level,
        rcsi_enabled,
        allow_snapshot_isolation,
        false,
        0,
        session_options,
        hints_by_table,
        workspace_slot,
    );
    ctx.session.extras.row_locks = Some(rt);

    let mut storage_guard = state.storage.write();
    let durability_active = !state.durability.lock().is_noop();
    let rollback_snapshot = if durability_active {
        Some((
            storage_guard.catalog.clone(),
            storage_guard.storage.clone(),
            storage_guard.table_versions.clone(),
            storage_guard.commit_ts,
        ))
    } else {
        None
    };
    let (cat, stor) = storage_guard.get_mut_refs();
    let mut script = create_script_executor(cat, stor, clock);
    let out = script.execute(stmt.clone(), ctx);
    ctx.session.extras.row_locks = None;
    let is_control_flow = out.as_ref().is_ok_and(|o| o.is_control_flow());
    if out.is_ok() && !is_control_flow {
        storage_guard.commit_ts += 1;
        let written_tables = collect_write_tables(&stmt);
        for table in &written_tables {
            let ts = storage_guard.commit_ts;
            storage_guard.table_versions.insert(table.clone(), ts);
        }
        if durability_active {
            let checkpoint = state.to_checkpoint_internal(&storage_guard);
            if let Err(e) = state.durability.lock().persist_checkpoint(&checkpoint) {
                if let Some((before_catalog, before_storage, before_versions, before_commit_ts)) =
                    rollback_snapshot
                {
                    storage_guard.catalog = before_catalog;
                    storage_guard.storage = before_storage;
                    storage_guard.table_versions = before_versions;
                    storage_guard.commit_ts = before_commit_ts;
                }
                state.table_locks.lock().release_all_for_session(session_id);
                return Err(e);
            }
        }
        let tx_id = state.allocate_tx_id();
        state.wal_auto_commit(tx_id);
    }
    state.table_locks.lock().release_all_for_session(session_id);
    out
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_read_without_transaction<C, S>(
    state: &SharedState<C, S>,
    session_id: SessionId,
    tx_manager: &mut TransactionManager<C, S, SessionSnapshot>,
    workspace_slot: &mut Option<TxWorkspace<C, S>>,
    session_options: &SessionOptions,
    stmt: Statement,
    ctx: &mut ExecutionContext,
    clock: &dyn Clock,
    active_db: &str,
    rcsi_enabled: bool,
    allow_snapshot_isolation: bool,
    hints_by_table: HashMap<String, TableLockHints>,
) -> StmtResult<Option<QueryResult>>
where
    C: EngineCatalog + 'static,
    S: EngineStorage + 'static,
{
    LockTable::acquire_statement_locks_with_hints(
        &state.table_locks,
        session_id,
        tx_manager,
        workspace_slot,
        &stmt,
        active_db,
        allow_snapshot_isolation,
        &hints_by_table,
        session_options.lock_timeout_ms,
        session_options.deadlock_priority,
        &|sid| dispatch_helpers::lookup_session_deadlock_priority(state, sid),
    )?;

    let rt = build_row_lock_runtime(
        state,
        session_id,
        active_db,
        tx_manager.session_isolation_level,
        rcsi_enabled,
        allow_snapshot_isolation,
        false,
        0,
        session_options,
        hints_by_table,
        workspace_slot,
    );
    ctx.session.extras.row_locks = Some(rt);

    if let Statement::Dml(DmlStatement::Select(ref select_stmt)) = stmt {
        if select_stmt.into_table.is_none() {
            let Statement::Dml(DmlStatement::Select(mut select_stmt)) = stmt else {
                unreachable!()
            };
            let storage_guard = state.storage.read();
            let (cat, stor) = storage_guard.get_refs();
            if !cat.get_user_databases().is_empty() {
                super::super::script::db_scope::qualify_select_in_ctx(&mut select_stmt, ctx, cat);
            }
            let qe = super::super::query::QueryExecutor {
                catalog: cat,
                storage: stor,
                clock,
            };
            let res = qe.execute_select(
                super::super::query::plan::RelationalQuery::from(*select_stmt),
                ctx,
            );
            ctx.session.extras.row_locks = None;
            state.table_locks.lock().release_all_for_session(session_id);
            let result = res?;
            if !result.columns.is_empty() {
                if let Some(ref sink) = ctx.frame.result_set_sink {
                    sink.lock().push(result.clone());
                }
            }
            return Ok(StmtOutcome::Ok(Some(result)));
        }
    }

    let mut storage_guard = state.storage.write();
    let (cat, stor) = storage_guard.get_mut_refs();
    let mut script = create_script_executor(cat, stor, clock);
    let out = script.execute(stmt, ctx);
    ctx.session.extras.row_locks = None;
    state.table_locks.lock().release_all_for_session(session_id);
    out
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_dirty_read_without_transaction<C, S>(
    state: &SharedState<C, S>,
    session_id: SessionId,
    workspace_slot: &mut Option<TxWorkspace<C, S>>,
    stmt: Statement,
    ctx: &mut ExecutionContext,
    clock: &dyn Clock,
) -> StmtResult<Option<QueryResult>>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    let (mut dirty_catalog, mut dirty_storage) =
        dirty_buffer::build_dirty_read_storage(state, session_id, workspace_slot)?;
    let mut script = create_script_executor(&mut dirty_catalog, &mut dirty_storage, clock);
    script.execute(stmt, ctx)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_non_transaction_statement<C, S>(
    state: &SharedState<C, S>,
    session_id: SessionId,
    tx_manager: &mut TransactionManager<C, S, SessionSnapshot>,
    journal: &mut dyn Journal,
    workspace_slot: &mut Option<TxWorkspace<C, S>>,
    clock: &dyn Clock,
    session_options: &mut SessionOptions,
    stmt: Statement,
    ctx: &mut ExecutionContext,
) -> StmtResult<Option<QueryResult>>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    if let Some(result) = dispatch_helpers::handle_session_statement(
        state,
        session_id,
        &stmt,
        session_options,
        ctx,
        journal,
    ) {
        return result;
    }

    let has_writes = !collect_write_tables(&stmt).is_empty();
    dispatch_helpers::check_and_prepare_database_lifecycle(
        state, session_id, &stmt, ctx, has_writes,
    )?;

    if session_options.fmtonly {
        if let Statement::Dml(DmlStatement::Select(select_stmt)) = &stmt {
            return execute_fmt_only_select(state, select_stmt, ctx);
        }
    }

    if session_options.noexec {
        if let Statement::Dml(DmlStatement::Select(select_stmt)) = &stmt {
            return execute_fmt_only_select(state, select_stmt, ctx);
        }
    }

    if session_options.showplan_xml {
        if let Statement::Dml(DmlStatement::Select(_)) = &stmt {
            return execute_showplan_xml(&stmt);
        }
    }

    if session_options.statistics_xml {
        if let Statement::Dml(DmlStatement::Select(_)) = &stmt {
            return execute_statistics_xml(&stmt, ctx);
        }
    }

    if tx_manager.active.is_none()
        && session_options.implicit_transactions
        && dispatch_helpers::should_start_implicit_transaction(&stmt)
        && !is_transaction_statement(&stmt)
    {
        transaction_exec::execute_transaction_statement(
            state,
            session_id,
            tx_manager,
            journal,
            workspace_slot,
            ctx,
            session_options,
            Statement::Transaction(crate::ast::TransactionStatement::Begin(None)),
        )?;
        ctx.frame.trancount = tx_manager.depth;
        ctx.frame.xact_state = tx_manager.xact_state;
    }

    let active_db = ctx
        .metadata
        .database
        .as_deref()
        .unwrap_or("master")
        .to_string();
    let hints_by_table = collect_and_validate_statement_hints(&stmt, &active_db)?;
    let (rcsi_enabled, allow_snapshot_isolation, is_user_db) =
        active_db_isolation_flags(state, &active_db);

    let isolation_level = tx_manager
        .active
        .as_ref()
        .map(|tx| tx.isolation_level)
        .unwrap_or(tx_manager.session_isolation_level);

    // SQL Server error 3952: accessing user tables under SNAPSHOT isolation when
    // ALLOW_SNAPSHOT_ISOLATION is OFF on a user database.
    if isolation_level == IsolationLevel::Snapshot
        && is_user_db
        && !allow_snapshot_isolation
        && (!collect_read_tables(&stmt).is_empty() || has_writes)
        && !matches!(&stmt, Statement::Ddl(_))
    {
        return Err(DbError::Execution(format!(
            "Snapshot isolation transaction failed accessing database '{}' because snapshot isolation is not allowed in this database. Use ALTER DATABASE to allow snapshot isolation.",
            active_db
        )));
    }

    let is_select = matches!(
        &stmt,
        Statement::Dml(DmlStatement::Select(s)) if s.into_table.is_none()
    );
    let nolock_dirty =
        is_select && all_read_tables_have_nolock(&stmt, &active_db, &hints_by_table);
    let read_uncommitted_dirty =
        (isolation_level == IsolationLevel::ReadUncommitted && is_select) || nolock_dirty;

    if tx_manager.active.is_some() {
        execute_in_transaction(
            state,
            session_id,
            tx_manager,
            journal,
            workspace_slot,
            clock,
            session_options,
            stmt,
            ctx,
            &active_db,
            rcsi_enabled,
            allow_snapshot_isolation,
            hints_by_table,
            nolock_dirty,
        )
    } else if read_uncommitted_dirty {
        execute_dirty_read_without_transaction(state, session_id, workspace_slot, stmt, ctx, clock)
    } else {
        let written_tables = collect_write_tables(&stmt);
        if written_tables.is_empty() {
            execute_read_without_transaction(
                state,
                session_id,
                tx_manager,
                workspace_slot,
                session_options,
                stmt,
                ctx,
                clock,
                &active_db,
                rcsi_enabled,
                allow_snapshot_isolation,
                hints_by_table,
            )
        } else {
            execute_write_without_transaction(
                state,
                session_id,
                tx_manager,
                workspace_slot,
                session_options,
                stmt,
                ctx,
                clock,
                &active_db,
                rcsi_enabled,
                allow_snapshot_isolation,
                hints_by_table,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_in_transaction<C, S>(
    state: &SharedState<C, S>,
    session_id: SessionId,
    tx_manager: &mut TransactionManager<C, S, SessionSnapshot>,
    journal: &mut dyn Journal,
    workspace_slot: &mut Option<TxWorkspace<C, S>>,
    clock: &dyn Clock,
    session_options: &mut SessionOptions,
    stmt: Statement,
    ctx: &mut ExecutionContext,
    active_db: &str,
    rcsi_enabled: bool,
    allow_snapshot_isolation: bool,
    hints_by_table: HashMap<String, TableLockHints>,
    nolock_dirty: bool,
) -> StmtResult<Option<QueryResult>>
where
    C: EngineCatalog + 'static,
    S: EngineStorage + 'static,
{
    LockTable::acquire_statement_locks_with_hints(
        &state.table_locks,
        session_id,
        tx_manager,
        workspace_slot,
        &stmt,
        active_db,
        allow_snapshot_isolation,
        &hints_by_table,
        session_options.lock_timeout_ms,
        session_options.deadlock_priority,
        &|sid| dispatch_helpers::lookup_session_deadlock_priority(state, sid),
    )?;

    ctx.options = session_options.clone();
    ctx.metadata.ansi_nulls = session_options.ansi_nulls;
    ctx.metadata.datefirst = session_options.datefirst;

    let isolation_level = tx_manager
        .active
        .as_ref()
        .map(|tx| tx.isolation_level)
        .unwrap_or(tx_manager.session_isolation_level);
    let is_select = matches!(
        &stmt,
        Statement::Dml(DmlStatement::Select(s)) if s.into_table.is_none()
    );
    let read_uncommitted_dirty =
        (isolation_level == IsolationLevel::ReadUncommitted && is_select) || nolock_dirty;
    let read_committed_select = isolation_level == IsolationLevel::ReadCommitted && is_select;
    let savepoint_depth = tx_manager
        .active
        .as_ref()
        .map(|tx| tx.savepoints.len())
        .unwrap_or(0);

    if tx_manager.active.is_none()
        && session_options.implicit_transactions
        && dispatch_helpers::should_start_implicit_transaction(&stmt)
        && !is_transaction_statement(&stmt)
    {
        transaction_exec::execute_transaction_statement(
            state,
            session_id,
            tx_manager,
            journal,
            workspace_slot,
            ctx,
            session_options,
            Statement::Transaction(crate::ast::TransactionStatement::Begin(None)),
        )?;
        ctx.frame.trancount = tx_manager.depth;
        ctx.frame.xact_state = tx_manager.xact_state;
    }

    let rt = build_row_lock_runtime(
        state,
        session_id,
        active_db,
        isolation_level,
        rcsi_enabled,
        allow_snapshot_isolation,
        true,
        savepoint_depth,
        session_options,
        hints_by_table,
        workspace_slot,
    );
    let acquired_handle = Arc::clone(&rt.acquired_locks);
    ctx.session.extras.row_locks = Some(rt);

    let out = if read_uncommitted_dirty {
        let (mut dirty_catalog, mut dirty_storage) =
            dirty_buffer::build_dirty_read_storage(state, session_id, workspace_slot)?;
        let mut script = create_script_executor(&mut dirty_catalog, &mut dirty_storage, clock);
        script.execute(stmt.clone(), ctx)
    } else if read_committed_select {
        let workspace = workspace_slot.as_mut().ok_or_else(|| {
            DbError::Execution("internal error: missing transaction workspace".into())
        })?;
        dispatch_helpers::refresh_workspace_for_read_committed(state, workspace, &stmt)?;
        let mut script =
            create_script_executor(&mut workspace.catalog, &mut workspace.storage, clock);
        script.execute(stmt.clone(), ctx)
    } else {
        let workspace = workspace_slot.as_mut().ok_or_else(|| {
            DbError::Execution("internal error: missing transaction workspace".into())
        })?;
        let mut script =
            create_script_executor(&mut workspace.catalog, &mut workspace.storage, clock);
        script.execute(stmt.clone(), ctx)
    };
    ctx.session.extras.row_locks = None;

    let stmt_row_locks = std::mem::take(&mut *acquired_handle.lock());
    if out.is_ok() {
        if let Some(workspace) = workspace_slot.as_mut() {
            workspace.acquired_locks.extend(stmt_row_locks);
        }
    } else {
        release_statement_row_locks(&state.table_locks, session_id, stmt_row_locks);
    }

    // Error 3960 (SNAPSHOT update conflict) always aborts and rolls back the snapshot transaction.
    if out.as_ref().is_err_and(|e| e.number() == 3960) {
        transaction_exec::force_xact_abort(
            state,
            session_id,
            tx_manager,
            journal,
            workspace_slot,
            ctx,
            session_options,
        );
        return out;
    }

    dispatch_helpers::update_transaction_state(
        &out,
        tx_manager,
        state,
        session_id,
        journal,
        workspace_slot,
        ctx,
        session_options,
        &stmt,
    );

    // Reconcile transaction state changes executed inside procedural blocks
    // (e.g. `IF @@TRANCOUNT > 0 COMMIT TRANSACTION;` or `ROLLBACK TRANSACTION;`).
    if ctx.frame.script_tx.rolled_back {
        ctx.frame.script_tx.rolled_back = false;
        if tx_manager.depth > 0 {
            transaction_exec::execute_transaction_statement(
                state,
                session_id,
                tx_manager,
                journal,
                workspace_slot,
                ctx,
                session_options,
                Statement::Transaction(crate::ast::TransactionStatement::Rollback(None)),
            )?;
        }
    } else if out.is_ok() && tx_manager.active.is_some() {
        if ctx.frame.trancount == 0 && tx_manager.depth > 0 {
            tx_manager.depth = 1;
            transaction_exec::execute_transaction_statement(
                state,
                session_id,
                tx_manager,
                journal,
                workspace_slot,
                ctx,
                session_options,
                Statement::Transaction(crate::ast::TransactionStatement::Commit(None)),
            )?;
        } else if ctx.frame.trancount != tx_manager.depth {
            tx_manager.depth = ctx.frame.trancount;
        }
    }
    out
}

fn execute_fmt_only_select<C, S>(
    state: &SharedState<C, S>,
    select_stmt: &crate::ast::SelectStmt,
    ctx: &mut ExecutionContext,
) -> StmtResult<Option<QueryResult>>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    let storage_guard = state.storage.read();
    let (catalog, storage) = storage_guard.get_refs();

    let qe = super::super::query::QueryExecutor {
        catalog,
        storage,
        clock: &SystemClock,
    };

    let mut select_owned = select_stmt.clone();
    if !catalog.get_user_databases().is_empty() {
        super::super::script::db_scope::qualify_select_in_ctx(&mut select_owned, ctx, catalog);
    }

    // Fast path for `SET FMTONLY ON; SELECT * FROM [schema].[table]` (used by SqlBulkCopy
    // and ODBC metadata discovery): return exact catalog column names, types, and nullabilities
    // with 0 rows without scanning table data.
    if select_owned.projection.len() == 1
        && matches!(select_owned.projection[0].expr, crate::ast::Expr::Wildcard)
    {
        if let Some(crate::ast::FromNode::Table(tref)) = &select_owned.from_clause {
            if let Some(obj) = tref.name_as_object() {
                let schema = obj.schema_or_dbo();
                let resolved = ctx
                    .resolve_table_name(obj.name.as_str())
                    .unwrap_or_else(|| obj.name.clone());
                if let Some(table) = catalog.find_table(schema, &resolved) {
                    let columns = table.columns.iter().map(|c| c.name.clone()).collect();
                    let column_types = table.columns.iter().map(|c| c.data_type.clone()).collect();
                    let column_nullabilities = table.columns.iter().map(|c| c.nullable).collect();
                    return Ok(StmtOutcome::Ok(Some(QueryResult {
                        columns,
                        column_types,
                        column_nullabilities,
                        rows: vec![],
                        return_status: None,
                        is_procedure: false,
                    })));
                }
            }
        }
    }

    let query_plan = super::super::query::plan::RelationalQuery::from(select_owned);
    let result = match qe.execute_select(query_plan, ctx) {
        Ok(mut r) => {
            r.rows = vec![];
            r
        }
        Err(_e) => {
            return Ok(StmtOutcome::Ok(Some(QueryResult {
                columns: vec![],
                column_types: vec![],
                column_nullabilities: vec![],
                rows: vec![],
                return_status: None,
                is_procedure: false,
            })));
        }
    };

    Ok(StmtOutcome::Ok(Some(result)))
}

fn showplan_xml_for_stmt(_stmt: &Statement, include_stats: bool) -> String {
    let stmt_id = 1u32;
    let estimated_rows = 1u64;
    let estimated_io = 0.0;
    let estimated_cpu = 0.001;
    let estimated_subtree_cost = 0.001;
    let _ = include_stats;
    let elapsed = 0.0;

    format!(
        r#"<ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan" Version="1.2" Build="16.0.0.0"><BatchSequence><Batch><CompiledPlan><QueryPlan CachedPlanSize="8" CompileTime="0" CompileCPU="0" CompileMemory="0"><RelOp NodeId="0" PhysicalOp="TableScan" LogicalOp="Table Scan" EstimateRows="{estimated_rows}" EstimateIO="{estimated_io}" EstimateCPU="{estimated_cpu}" AvgRowSize="0" EstimatedTotalSubtreeCost="{estimated_subtree_cost}" EstimatedExecutionMode="Row" Parallel="0" EstimateRebinds="0"><OutputList><ColumnReference Database="" Schema="" Table="" Column="*" /></OutputList><RunTimeInformation><RunTimeCountersPerThread Thread="0" ActualRows="0" Batches="0" ActualExecutionTimems="{elapsed}" /></RunTimeInformation></RelOp></QueryPlan></CompiledPlan><Statements><StmtSimple StatementText="" StatementId="{stmt_id}" StatementType="SELECT" StatementSubTreeCost="{estimated_subtree_cost}" StatementEstRows="{estimated_rows}" /></Statements></Batch></BatchSequence></ShowPlanXML>"#
    )
}

fn execute_showplan_xml(stmt: &Statement) -> StmtResult<Option<QueryResult>> {
    let xml = showplan_xml_for_stmt(stmt, false);
    Ok(StmtOutcome::Ok(Some(QueryResult {
        columns: vec!["Microsoft SQL Server 2005 XML Showplan".to_string()],
        column_types: vec![DataType::NVarChar { max_len: u16::MAX }],
        column_nullabilities: vec![false],
        rows: vec![vec![Value::NVarChar(xml)]],
        return_status: None,
        is_procedure: false,
    })))
}

fn execute_statistics_xml(
    stmt: &Statement,
    _ctx: &ExecutionContext,
) -> StmtResult<Option<QueryResult>> {
    let xml = showplan_xml_for_stmt(stmt, true);
    Ok(StmtOutcome::Ok(Some(QueryResult {
        columns: vec!["Microsoft SQL Server 2005 XML Statistics".to_string()],
        column_types: vec![DataType::NVarChar { max_len: u16::MAX }],
        column_nullabilities: vec![false],
        rows: vec![vec![Value::NVarChar(xml)]],
        return_status: None,
        is_procedure: false,
    })))
}
