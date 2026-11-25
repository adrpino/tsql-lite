use crate::ast::{DmlStatement, SessionStatement, Statement};
use crate::catalog::Catalog;
use crate::error::{DbError, StmtOutcome, StmtResult};

use super::super::clock::SystemClock;
use super::super::context::ExecutionContext;
use super::super::journal::{Journal, JournalEvent};
use super::super::locks::{SessionId, TxWorkspace};
use super::super::result::QueryResult;
use super::super::session::{SessionSnapshot, SharedState};
use super::super::string_norm::normalize_identifier;
use super::super::table_util::collect_read_tables;
use super::super::tooling::apply_set_option;
use super::super::tooling::SessionOptions;
use super::super::transaction::TransactionManager;
use super::{EngineCatalog, EngineStorage};
use crate::storage::Storage;
use crate::types::Value;

pub(crate) fn should_start_implicit_transaction(stmt: &Statement) -> bool {
    matches!(
        stmt,
        Statement::Dml(DmlStatement::Insert(_))
            | Statement::Dml(DmlStatement::Update(_))
            | Statement::Dml(DmlStatement::Delete(_))
            | Statement::Dml(DmlStatement::Merge(_))
            | Statement::Dml(DmlStatement::SelectAssign(_))
            | Statement::Ddl(_)
    )
}

pub(crate) fn lookup_session_deadlock_priority<C, S>(
    state: &SharedState<C, S>,
    session_id: SessionId,
) -> i32
where
    C: EngineCatalog,
    S: EngineStorage,
{
    state
        .deadlock_priorities
        .read()
        .get(&session_id)
        .copied()
        .unwrap_or(0)
}

pub(crate) fn handle_session_statement<C, S>(
    state: &SharedState<C, S>,
    session_id: SessionId,
    stmt: &Statement,
    session_options: &mut SessionOptions,
    ctx: &mut ExecutionContext,
    journal: &mut dyn Journal,
) -> Option<StmtResult<Option<QueryResult>>>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    if let Statement::Session(SessionStatement::UseDatabase(database)) = stmt {
        let storage_guard = state.storage.read();
        let (catalog, _) = storage_guard.get_refs();
        match super::super::database_catalog::database_id_for_name_in(catalog, database) {
            Some(database_id) => {
                let canonical_name =
                    super::super::database_catalog::database_name_for_id_in(catalog, database_id)
                        .unwrap_or_else(|| database.clone());
                let db_opts = catalog.get_database_options(&canonical_name);
                drop(storage_guard);
                if db_opts.user_access == 1
                    && has_other_session_in_database(state, session_id, &canonical_name)
                {
                    return Some(Err(DbError::Execution(format!(
                        "Database '{}' is already open and can only have one user at a time.",
                        canonical_name
                    ))));
                }
                ctx.metadata.database = Some(canonical_name);
                Some(Ok(StmtOutcome::Ok(None)))
            }
            None => Some(Err(DbError::Execution(format!(
                "Cannot open database '{}' requested by the login. The login failed.",
                database
            )))),
        }
    } else if let Statement::Session(SessionStatement::SetOption(opt)) = stmt {
        match apply_set_option(opt, session_options) {
            Ok(apply) => {
                ctx.options = session_options.clone();
                ctx.metadata.ansi_nulls = session_options.ansi_nulls;
                ctx.metadata.datefirst = session_options.datefirst;
                state
                    .deadlock_priorities
                    .write()
                    .insert(session_id, session_options.deadlock_priority);
                for warn in apply.warnings {
                    journal.record(JournalEvent::Info { message: warn });
                }
                Some(Ok(StmtOutcome::Ok(None)))
            }
            Err(e) => Some(Err(e)),
        }
    } else if let Statement::Procedural(crate::ast::ProceduralStatement::BeginEnd(stmts)) = stmt {
        if !stmts.is_empty()
            && stmts
                .iter()
                .all(|s| matches!(s, Statement::Session(SessionStatement::SetOption(_))))
        {
            for s in stmts {
                if let Statement::Session(SessionStatement::SetOption(opt)) = s {
                    match apply_set_option(opt, session_options) {
                        Ok(apply) => {
                            for warn in apply.warnings {
                                journal.record(JournalEvent::Info { message: warn });
                            }
                        }
                        Err(e) => return Some(Err(e)),
                    }
                }
            }
            ctx.options = session_options.clone();
            ctx.metadata.ansi_nulls = session_options.ansi_nulls;
            ctx.metadata.datefirst = session_options.datefirst;
            state
                .deadlock_priorities
                .write()
                .insert(session_id, session_options.deadlock_priority);
            Some(Ok(StmtOutcome::Ok(None)))
        } else {
            None
        }
    } else if let Statement::Session(SessionStatement::SetIdentityInsert(ref id_stmt)) = stmt {
        let storage_guard = state.storage.read();
        let (catalog, _) = storage_guard.get_refs();
        let schema = id_stmt.table.schema_or_dbo();
        if catalog.find_table(schema, &id_stmt.table.name).is_none() {
            return Some(Err(DbError::table_not_found(schema, &id_stmt.table.name)));
        }

        let table_name = normalize_identifier(&id_stmt.table.name);
        if id_stmt.on {
            session_options.identity_insert.clear();
            session_options.identity_insert.insert(table_name);
        } else {
            session_options.identity_insert.remove(&table_name);
        }
        ctx.options.identity_insert = session_options.identity_insert.clone();
        Some(Ok(StmtOutcome::Ok(None)))
    } else if let Statement::Session(SessionStatement::SetContextInfo(ref expr)) = stmt {
        let storage_guard = state.storage.read();
        let (catalog, storage) = storage_guard.get_refs();
        match crate::executor::evaluator::eval_expr(
            expr,
            &[],
            ctx,
            catalog as &dyn Catalog,
            storage as &dyn Storage,
            &SystemClock,
        ) {
            Ok(val) => {
                let bytes = match val {
                    Value::Null => Vec::new(),
                    Value::Binary(mut b) | Value::VarBinary(mut b) => {
                        if !b.is_empty() {
                            if b.len() > 128 {
                                b.truncate(128);
                            } else {
                                b.resize(128, 0);
                            }
                        }
                        b
                    }
                    _ => {
                        let mut b = val.to_string_value().into_bytes();
                        if b.len() > 128 {
                            b.truncate(128);
                        } else {
                            b.resize(128, 0);
                        }
                        b
                    }
                };
                *ctx.session.context_info = bytes;
                Some(Ok(StmtOutcome::Ok(None)))
            }
            Err(e) => Some(Err(e)),
        }
    } else {
        None
    }
}

pub(crate) fn refresh_workspace_for_read_committed<C, S>(
    state: &SharedState<C, S>,
    workspace: &mut TxWorkspace<C, S>,
    stmt: &Statement,
) -> Result<(), DbError>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    let storage_guard = state.storage.read();
    let read_tables = collect_read_tables(stmt);
    for table_def in storage_guard.catalog.get_tables() {
        let display_schema =
            super::super::database_catalog::display_schema_name(table_def.schema_or_dbo());
        let tname = format!(
            "{}.{}",
            normalize_identifier(display_schema),
            normalize_identifier(&table_def.name)
        );
        if workspace.write_tables.contains(&tname) {
            continue;
        }
        if !read_tables.is_empty() && !read_tables.contains(&tname) {
            continue;
        }
        let tid = table_def.id;
        if let Ok(rows) = storage_guard.storage.scan_rows(tid) {
            if let Ok(committed_rows) = rows.collect::<Result<Vec<_>, DbError>>() {
                workspace.storage.replace_table(tid, committed_rows)?;
            }
        }
    }
    for table_def in storage_guard.catalog.get_tables() {
        let display_schema =
            super::super::database_catalog::display_schema_name(table_def.schema_or_dbo());
        let tname = format!(
            "{}.{}",
            normalize_identifier(display_schema),
            normalize_identifier(&table_def.name)
        );
        if workspace.write_tables.contains(&tname) {
            continue;
        }
        if !read_tables.is_empty() && !read_tables.contains(&tname) {
            continue;
        }
        if workspace
            .catalog
            .find_table(table_def.schema_or_dbo(), &table_def.name)
            .is_none()
        {
            workspace.catalog.register_table(table_def.as_ref().clone());
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_transaction_state<C, S>(
    out: &crate::error::StmtResult<Option<QueryResult>>,
    tx_manager: &mut TransactionManager<C, S, SessionSnapshot>,
    state: &SharedState<C, S>,
    session_id: SessionId,
    journal: &mut dyn Journal,
    workspace_slot: &mut Option<TxWorkspace<C, S>>,
    ctx: &mut ExecutionContext,
    session_options: &mut SessionOptions,
    stmt: &Statement,
) where
    C: EngineCatalog,
    S: EngineStorage,
{
    let is_control_flow = out.as_ref().is_ok_and(|o| o.is_control_flow());
    if out.is_ok() && !is_control_flow {
        super::super::transaction_exec::register_read_tables(workspace_slot, stmt);
        super::super::transaction_exec::register_workspace_write_tables(workspace_slot, stmt);
        super::super::transaction_exec::register_write_intent::<C, S>(tx_manager, journal, stmt);
        if tx_manager.xact_state != -1 {
            tx_manager.xact_state = 1;
        }
    } else if out.is_err() && session_options.xact_abort {
        super::super::transaction_exec::force_xact_abort(
            state,
            session_id,
            tx_manager,
            journal,
            workspace_slot,
            ctx,
            session_options,
        );
    } else if out.is_err() {
        // SQL Server error 1222 (lock timeout / NOWAIT) does NOT doom an active transaction
        // when XACT_ABORT is OFF.
        if !out.as_ref().is_err_and(|e| e.number() == 1222) {
            tx_manager.xact_state = -1;
        }
    }
}

pub(crate) fn has_other_session_in_database<C, S>(
    state: &SharedState<C, S>,
    current_session_id: SessionId,
    db_name: &str,
) -> bool
where
    C: EngineCatalog,
    S: EngineStorage,
{
    let mut other_sids: Vec<SessionId> = state
        .sessions
        .read()
        .keys()
        .copied()
        .filter(|&sid| sid != current_session_id)
        .collect();
    other_sids.sort_unstable();
    for sid in other_sids {
        if let Some(entry) = state.get_session(sid) {
            if let Some(sess) = entry.try_lock() {
                if sess.current_database.eq_ignore_ascii_case(db_name) {
                    return true;
                }
            }
        }
    }
    false
}

/// Evicts sibling sessions attached to `db_name` (rolling back any open transactions,
/// releasing their locks/dirty buffers, and switching their database context to `master`).
/// Invoked by `ALTER DATABASE [db] SET SINGLE_USER` and `WITH ROLLBACK IMMEDIATE`.
pub(crate) fn evict_sessions_for_database<C, S>(
    state: &SharedState<C, S>,
    current_session_id: SessionId,
    db_name: &str,
) where
    C: EngineCatalog,
    S: EngineStorage,
{
    let mut other_sids: Vec<SessionId> = state
        .sessions
        .read()
        .keys()
        .copied()
        .filter(|&sid| sid != current_session_id)
        .collect();
    other_sids.sort_unstable();
    for sid in other_sids {
        if let Some(entry) = state.get_session(sid) {
            let mut sess = entry.lock();
            if sess.current_database.eq_ignore_ascii_case(db_name) {
                if sess.tx_manager.depth > 0 || sess.workspace.is_some() {
                    state.table_locks.lock().release_all_for_session(sid);
                    state.dirty_buffer.lock().clear_session(sid);
                    state.app_locks.release_transaction_locks(sid);
                    sess.workspace = None;
                    sess.tx_manager.active = None;
                    sess.tx_manager.depth = 0;
                    sess.tx_manager.xact_state = 0;
                }
                sess.current_database = "master".to_string();
            }
        }
    }
}

/// Checks database-level lifecycle guards before executing DDL/DML:
/// - Evicts sibling sessions on `ALTER DATABASE ... SET SINGLE_USER` / `WITH ROLLBACK IMMEDIATE`.
/// - Rejects `DROP DATABASE` with error 3702 if another session is still connected to that DB.
/// - Rejects writes on a read-only database (e.g. `AS SNAPSHOT OF` or `SET READ_ONLY`) with error 3906.
pub(crate) fn check_and_prepare_database_lifecycle<C, S>(
    state: &SharedState<C, S>,
    session_id: SessionId,
    stmt: &Statement,
    ctx: &ExecutionContext,
    has_writes: bool,
) -> Result<(), DbError>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    if let Statement::Ddl(ddl) = stmt {
        if let crate::ast::DdlStatement::CatalogMetadata(meta) = ddl.as_ref() {
            match meta.as_ref() {
                crate::ast::CatalogMetadataAction::AlterDatabase {
                    name,
                    options,
                    rollback_immediate,
                } => {
                    let target_db = if name.eq_ignore_ascii_case("CURRENT") {
                        ctx.metadata.database.as_deref().unwrap_or("master")
                    } else {
                        name.as_str()
                    };
                    let setting_single_user = options
                        .iter()
                        .any(|o| matches!(o, crate::ast::DatabaseOptionChange::UserAccess(1)));
                    if *rollback_immediate || setting_single_user {
                        evict_sessions_for_database(state, session_id, target_db);
                    }
                    return Ok(());
                }
                crate::ast::CatalogMetadataAction::DropDatabase { name, .. } => {
                    if has_other_session_in_database(state, session_id, name) {
                        return Err(DbError::Execution(format!(
                            "Cannot drop database '{}' because it is currently in use.",
                            name
                        )));
                    }
                    return Ok(());
                }
                crate::ast::CatalogMetadataAction::CreateDatabase { .. } => {
                    return Ok(());
                }
                _ => {}
            }
        }
    }

    if has_writes {
        let active_db = ctx.metadata.database.as_deref().unwrap_or("master");
        let is_read_only = {
            let guard = state.storage.read();
            guard.catalog.get_database_options(active_db).is_read_only
        };
        if is_read_only {
            return Err(DbError::Execution(format!(
                "Failed to update database '{}' because the database is read-only.",
                active_db
            )));
        }
    }
    Ok(())
}
