use std::sync::Arc;

use crate::ast::Statement;
#[cfg(feature = "persistence")]
use crate::catalog::CatalogImpl;
use crate::error::DbError;
use crate::executor::locks::SessionId;
use crate::executor::result::QueryResult;
use crate::executor::session::SessionManager as SessionManagerTrait;
use crate::executor::tooling::{ExecutionTrace, ExplainPlan, SessionOptions};
use crate::executor::wal::{Lsn, WalRecord};

#[cfg(feature = "persistence")]
use super::super::durability::RecoveryReader;
use super::super::durability::{DurabilitySink, RecoveryCheckpoint};
use super::super::session::{SharedState, SharedStorage};
#[cfg(feature = "checkpoint")]
use super::CheckpointManager as CheckpointManagerTrait;
use super::{
    EngineCatalog, EngineStorage, RandomSeed as RandomSeedTrait, SqlAnalyzer as SqlAnalyzerTrait,
    StatementExecutor as StatementExecutorTrait,
};

#[cfg(feature = "checkpoint")]
mod checkpoint;
mod session;

/// Unified facade over focused service structs (`StatementExecutorService`,
/// `CheckpointManagerService`, `SqlAnalyzerService`, `SessionManagerService`).
///
/// Trait implementations (`StatementExecutor`, `CheckpointManager`, `SqlAnalyzer`,
/// `SessionManager`) delegate to the corresponding service obtained via `executor()`,
/// `checkpoint_manager()`, `analyzer()`, and `session_manager()`. This keeps each service's
/// internals isolated while exposing a single API surface to callers.
#[derive(Clone)]
pub struct DatabaseInner<C, S>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    pub(crate) inner: Arc<SharedState<C, S>>,
}

#[cfg(feature = "persistence")]
impl DatabaseInner<CatalogImpl, crate::storage::RedbStorage> {
    pub fn new_persistent(path: &std::path::Path) -> Result<Self, DbError> {
        std::fs::create_dir_all(path)
            .map_err(|e| DbError::Storage(format!("failed to create database directory: {}", e)))?;
        let storage = crate::storage::RedbStorage::new(path.join("data.redb"))?;
        let durability = super::super::durability::FileDurability::new(path.join("catalog.json"))?;

        let wal_path = path.join("wal.log");
        let wal = crate::executor::wal::Wal::open(&wal_path)?;

        // P1: Replay WAL records after checkpoint to recover uncommitted transactions
        let has_checkpoint = durability.latest_checkpoint().is_some();
        if has_checkpoint {
            if let Ok(frames) = wal.read_all_records() {
                let recovery = crate::executor::wal::replay_wal_records(&frames);
                if !recovery.rolled_back_tx_ids.is_empty() {
                    eprintln!(
                        "WAL recovery: found {} rolled back txs",
                        recovery.rolled_back_tx_ids.len()
                    );
                }
            }
        }

        let state = if has_checkpoint {
            let state = SharedState::from_checkpoint(
                durability.latest_checkpoint().unwrap(),
                Box::new(durability),
                storage,
            );
            *state.wal.lock() = Some(wal);
            state
        } else {
            let mut state = SharedState::with_initial(CatalogImpl::new(), storage);
            *state.durability.get_mut() = Box::new(durability);
            *state.wal.lock() = Some(wal);
            state
        };
        Ok(Self {
            inner: std::sync::Arc::new(state),
        })
    }
}

impl<C, S> DatabaseInner<C, S>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    pub fn new() -> Self {
        let mut catalog = C::default();
        let _ = catalog.create_schema("dbo");
        let state = SharedState::with_initial(catalog, S::default());
        Self {
            inner: Arc::new(state),
        }
    }
}

impl<C, S> Default for DatabaseInner<C, S>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<C, S> DatabaseInner<C, S>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    pub fn new_with_durability(durability: Box<dyn DurabilitySink<C>>) -> Self {
        let state = if let Some(checkpoint) = durability.latest_checkpoint() {
            SharedState::from_checkpoint(checkpoint, durability, S::default())
        } else {
            let mut catalog = C::default();
            let _ = catalog.create_schema("dbo");
            let mut state = SharedState::with_initial(catalog, S::default());
            *state.durability.get_mut() = durability;
            state
        };
        Self {
            inner: Arc::new(state),
        }
    }

    #[cfg(feature = "checkpoint")]
    pub fn from_checkpoint(payload: &str) -> Result<Self, DbError> {
        let checkpoint = RecoveryCheckpoint::<C>::from_json(payload)?;
        let state = SharedState::from_checkpoint_internal(checkpoint);
        Ok(Self {
            inner: Arc::new(state),
        })
    }

    pub fn fork_isolated(&self) -> Self {
        let guard = self.inner.storage.read();
        let state = SharedState::from_parts_internal(
            guard.catalog.clone(),
            guard.storage.clone(),
            guard.commit_ts,
            guard.table_versions.clone(),
        );
        drop(guard);
        Self {
            inner: Arc::new(state),
        }
    }

    pub fn reset(&self) {
        let mut storage = self.inner.storage.write();
        let mut catalog = C::default();
        let _ = catalog.create_schema("dbo");
        storage.catalog = catalog;
        storage.storage = S::default();
        storage.commit_ts = 0;
        storage.table_versions.clear();
        self.inner.table_locks.lock().clear();
        let sessions: Vec<(SessionId, Arc<parking_lot::Mutex<_>>)> = self
            .inner
            .sessions
            .read()
            .iter()
            .map(|(&sid, s)| (sid, Arc::clone(s)))
            .collect();
        for (session_id, session_arc) in sessions {
            session_arc.lock().reset();
            self.inner.deadlock_priorities.write().insert(session_id, 0);
        }
    }

    pub fn set_durability_sink(&self, durability: Box<dyn DurabilitySink<C>>) {
        let mut guard = self.inner.durability.lock();
        *guard = durability;
    }

    pub fn executor(&self) -> super::StatementExecutorService<C, S> {
        super::StatementExecutorService {
            state: self.inner.clone(),
        }
    }

    #[cfg(feature = "checkpoint")]
    pub fn checkpoint_manager(&self) -> super::CheckpointManagerService<C, S> {
        super::CheckpointManagerService {
            state: self.inner.clone(),
        }
    }

    pub fn analyzer(&self) -> super::SqlAnalyzerService<C, S> {
        super::SqlAnalyzerService {
            state: self.inner.clone(),
        }
    }

    pub fn session_manager(&self) -> super::SessionManagerService<C, S> {
        super::SessionManagerService {
            state: self.inner.clone(),
        }
    }

    pub fn print_output(&self, session_id: SessionId) -> Vec<String> {
        self.inner
            .get_session(session_id)
            .map(|s| s.lock().diagnostics.print_output.clone())
            .unwrap_or_default()
    }

    pub fn take_print_output(&self, session_id: SessionId) -> Vec<String> {
        self.inner
            .get_session(session_id)
            .map(|s| std::mem::take(&mut s.lock().diagnostics.print_output))
            .unwrap_or_default()
    }

    pub fn clear_print_output(&self, session_id: SessionId) {
        if let Some(s) = self.inner.get_session(session_id) {
            s.lock().diagnostics.print_output.clear();
        }
    }

    pub fn create_session(&self) -> SessionId {
        self.session_manager().create_session()
    }

    pub fn close_session(&self, session_id: SessionId) -> Result<(), DbError> {
        self.session_manager().close_session(session_id)
    }

    pub fn execute_session(
        &self,
        session_id: SessionId,
        stmt: Statement,
    ) -> Result<Option<QueryResult>, DbError> {
        self.executor().execute_session(session_id, stmt)
    }

    pub fn execute_session_batch(
        &self,
        session_id: SessionId,
        stmts: Vec<Statement>,
    ) -> Result<Option<QueryResult>, DbError> {
        self.executor().execute_session_batch(session_id, stmts)
    }

    pub fn execute_session_batch_all_results(
        &self,
        session_id: SessionId,
        stmts: Vec<Statement>,
    ) -> Result<Vec<QueryResult>, DbError> {
        let sink = Arc::new(parking_lot::Mutex::new(Vec::new()));
        if let Some(entry) = self.inner.get_session(session_id) {
            entry.lock().diagnostics.result_set_sink = Some(sink.clone());
        }
        let res = self.executor().execute_session_batch(session_id, stmts);
        if let Some(entry) = self.inner.get_session(session_id) {
            entry.lock().diagnostics.result_set_sink = None;
        }
        res?;
        let collected = std::mem::take(&mut *sink.lock());
        Ok(collected)
    }

    pub fn execute_session_batch_sql(
        &self,
        session_id: SessionId,
        sql: &str,
    ) -> Result<Option<QueryResult>, DbError> {
        self.executor().execute_session_batch_sql(session_id, sql)
    }

    pub fn execute_session_batch_sql_multi(
        &self,
        session_id: SessionId,
        sql: &str,
    ) -> Result<Vec<Option<QueryResult>>, DbError> {
        self.executor()
            .execute_session_batch_sql_multi(session_id, sql)
    }

    pub fn set_session_seed(&self, session_id: SessionId, seed: u64) -> Result<(), DbError> {
        self.analyzer().set_session_seed(session_id, seed)
    }

    pub fn explain_sql(&self, sql: &str) -> Result<ExplainPlan, DbError> {
        self.analyzer().explain_sql(sql)
    }

    pub fn trace_execute_session_sql(
        &self,
        session_id: SessionId,
        sql: &str,
    ) -> Result<ExecutionTrace, DbError> {
        self.analyzer().trace_execute_session_sql(session_id, sql)
    }

    pub fn session_options(&self, session_id: SessionId) -> Result<SessionOptions, DbError> {
        self.analyzer().session_options(session_id)
    }

    #[cfg(feature = "checkpoint")]
    pub fn export_checkpoint(&self) -> Result<String, DbError> {
        self.checkpoint_manager().export_checkpoint()
    }

    #[cfg(feature = "checkpoint")]
    pub fn import_checkpoint(&self, payload: &str) -> Result<(), DbError> {
        self.checkpoint_manager().import_checkpoint(payload)
    }
}

impl<C, S> SessionManagerTrait for DatabaseInner<C, S>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    fn create_session(&self) -> SessionId {
        self.session_manager().create_session()
    }

    fn reset_session(&self, session_id: SessionId) -> Result<(), DbError> {
        self.session_manager().reset_session(session_id)
    }

    fn close_session(&self, session_id: SessionId) -> Result<(), DbError> {
        self.session_manager().close_session(session_id)
    }

    fn set_session_journal(
        &self,
        session_id: SessionId,
        journal: Box<dyn crate::executor::journal::Journal>,
    ) -> Result<(), DbError> {
        self.session_manager()
            .set_session_journal(session_id, journal)
    }

    fn session_cancel_flag(
        &self,
        session_id: SessionId,
    ) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        self.session_manager().session_cancel_flag(session_id)
    }

    fn take_print_output(&self, session_id: SessionId) -> Vec<String> {
        self.session_manager().take_print_output(session_id)
    }
}

impl<C, S> StatementExecutorTrait for DatabaseInner<C, S>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    fn execute_session(
        &self,
        session_id: SessionId,
        stmt: Statement,
    ) -> Result<Option<QueryResult>, DbError> {
        self.executor().execute_session(session_id, stmt)
    }

    fn execute_session_batch(
        &self,
        session_id: SessionId,
        stmts: Vec<Statement>,
    ) -> Result<Option<QueryResult>, DbError> {
        self.executor().execute_session_batch(session_id, stmts)
    }

    fn execute_session_batch_sql(
        &self,
        session_id: SessionId,
        sql: &str,
    ) -> Result<Option<QueryResult>, DbError> {
        self.executor().execute_session_batch_sql(session_id, sql)
    }

    fn execute_session_batch_sql_multi(
        &self,
        session_id: SessionId,
        sql: &str,
    ) -> Result<Vec<Option<QueryResult>>, DbError> {
        self.executor()
            .execute_session_batch_sql_multi(session_id, sql)
    }

    fn execute_session_batch_sql_multi_detailed(
        &self,
        session_id: SessionId,
        sql: &str,
    ) -> Result<Vec<crate::BatchResultItem>, DbError> {
        self.executor()
            .execute_session_batch_sql_multi_detailed(session_id, sql)
    }

    fn set_session_metadata(
        &self,
        session_id: SessionId,
        user: Option<String>,
        app_name: Option<String>,
        host_name: Option<String>,
        database: Option<String>,
    ) -> Result<(), DbError> {
        self.executor()
            .set_session_metadata(session_id, user, app_name, host_name, database)
    }

    fn set_session_database(&self, session_id: SessionId, database: String) -> Result<(), DbError> {
        self.executor().set_session_database(session_id, database)
    }

    fn cursor_rpc_open(
        &self,
        session_id: SessionId,
        sql: &str,
        scroll_opt: i32,
    ) -> Result<(i32, QueryResult), DbError> {
        self.executor().cursor_rpc_open(session_id, sql, scroll_opt)
    }

    fn cursor_rpc_fetch(
        &self,
        session_id: SessionId,
        handle: i32,
        fetch_type: i32,
        row_num: i32,
        n_rows: i32,
    ) -> Result<super::CursorFetchResult, DbError> {
        self.executor()
            .cursor_rpc_fetch(session_id, handle, fetch_type, row_num, n_rows)
    }

    fn cursor_rpc_close(&self, session_id: SessionId, handle: i32) -> Result<(), DbError> {
        self.executor().cursor_rpc_close(session_id, handle)
    }

    fn cursor_rpc_deallocate(&self, session_id: SessionId, handle: i32) -> Result<(), DbError> {
        self.executor().cursor_rpc_deallocate(session_id, handle)
    }

    fn set_bulk_load_active(
        &self,
        session_id: SessionId,
        active: bool,
        table: crate::ast::ObjectName,
        columns: Vec<crate::ast::statements::ddl::ColumnSpec>,
        received_metadata: bool,
    ) -> Result<(), DbError> {
        self.executor()
            .set_bulk_load_active(session_id, active, table, columns, received_metadata)
    }

    fn get_bulk_load_state(
        &self,
        session_id: SessionId,
    ) -> (
        bool,
        Option<crate::ast::ObjectName>,
        Option<Vec<crate::ast::statements::ddl::ColumnSpec>>,
        bool,
    ) {
        self.executor().get_bulk_load_state(session_id)
    }
}

impl<C, S> SqlAnalyzerTrait for DatabaseInner<C, S>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    fn explain_sql(&self, sql: &str) -> Result<ExplainPlan, DbError> {
        self.analyzer().explain_sql(sql)
    }

    fn trace_execute_session_sql(
        &self,
        session_id: SessionId,
        sql: &str,
    ) -> Result<ExecutionTrace, DbError> {
        self.analyzer().trace_execute_session_sql(session_id, sql)
    }

    fn session_isolation_level(
        &self,
        session_id: SessionId,
    ) -> Result<crate::ast::IsolationLevel, DbError> {
        self.analyzer().session_isolation_level(session_id)
    }

    fn transaction_is_active(&self, session_id: SessionId) -> Result<bool, DbError> {
        self.analyzer().transaction_is_active(session_id)
    }

    fn session_options(&self, session_id: SessionId) -> Result<SessionOptions, DbError> {
        self.analyzer().session_options(session_id)
    }
}

impl<C, S> RandomSeedTrait for DatabaseInner<C, S>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    fn set_session_seed(&self, session_id: SessionId, seed: u64) -> Result<(), DbError> {
        self.analyzer().set_session_seed(session_id, seed)
    }
}

#[cfg(feature = "checkpoint")]
impl<C, S> super::CheckpointManager for DatabaseInner<C, S>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    fn export_checkpoint(&self) -> Result<String, DbError> {
        self.checkpoint_manager().export_checkpoint()
    }

    fn import_checkpoint(&self, payload: &str) -> Result<(), DbError> {
        self.checkpoint_manager().import_checkpoint(payload)
    }
}

fn rebuild_all_indexes_from_catalog<C, S>(catalog: &C, storage: &mut S)
where
    C: EngineCatalog,
    S: EngineStorage,
{
    let tables = catalog.get_tables();
    let indexes = catalog.get_indexes();
    for index_def in indexes {
        let rows = match storage.get_rows(index_def.table_id) {
            Ok(rows) => rows,
            Err(_) => continue,
        };
        let col_positions: Vec<u32> = tables
            .iter()
            .find(|t| t.id == index_def.table_id)
            .map(|tbl| {
                index_def
                    .column_ids
                    .iter()
                    .filter_map(|&cid| {
                        tbl.columns
                            .iter()
                            .position(|c| c.id == cid)
                            .map(|p| p as u32)
                    })
                    .collect()
            })
            .unwrap_or_else(|| index_def.column_ids.clone());
        if let Some(index_storage) = storage.as_index_storage_mut() {
            index_storage.register_index(
                index_def.table_id,
                index_def.id,
                col_positions,
                index_def.is_unique,
                index_def.is_clustered,
            );
            if let Some(idx) = index_storage.get_index_mut(index_def.table_id, index_def.id) {
                let row_refs: Vec<(usize, &[crate::types::Value])> = rows
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| !r.deleted)
                    .map(|(i, r)| (i, r.values.as_slice()))
                    .collect();
                let _ = idx.rebuild_from_rows(&row_refs);
            }
        }
    }
}

impl<C, S> SharedState<C, S>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    pub fn from_checkpoint(
        checkpoint: RecoveryCheckpoint<C>,
        durability: Box<dyn DurabilitySink<C>>,
        mut storage: S,
    ) -> Self {
        let _ = storage.restore_from_checkpoint(checkpoint.storage_data);
        rebuild_all_indexes_from_catalog(&checkpoint.catalog, &mut storage);
        Self {
            storage: std::sync::Arc::new(parking_lot::RwLock::new(SharedStorage {
                catalog: checkpoint.catalog,
                storage,
                commit_ts: checkpoint.commit_ts,
                table_versions: checkpoint.table_versions,
            })),
            table_locks: std::sync::Arc::new(parking_lot::Mutex::new(
                super::super::locks::LockTable::new(),
            )),
            durability: parking_lot::Mutex::new(durability),
            wal: parking_lot::Mutex::new(None),
            next_tx_id: std::sync::atomic::AtomicU64::new(1),
            sessions: parking_lot::RwLock::new(std::collections::HashMap::new()),
            deadlock_priorities: std::sync::Arc::new(parking_lot::RwLock::new(
                std::collections::HashMap::new(),
            )),
            next_session_id: std::sync::atomic::AtomicU64::new(1),
            dirty_buffer: std::sync::Arc::new(parking_lot::Mutex::new(
                super::super::dirty_buffer::DirtyBuffer::new(),
            )),
            app_locks: super::super::applock::SharedAppLocks::new(),
        }
    }

    pub fn from_parts_internal(
        catalog: C,
        storage: S,
        commit_ts: u64,
        table_versions: std::collections::HashMap<String, u64>,
    ) -> Self {
        Self {
            storage: std::sync::Arc::new(parking_lot::RwLock::new(SharedStorage {
                catalog,
                storage,
                commit_ts,
                table_versions,
            })),
            table_locks: std::sync::Arc::new(parking_lot::Mutex::new(
                super::super::locks::LockTable::new(),
            )),
            durability: parking_lot::Mutex::new(Box::new(
                super::super::durability::NoopDurability::default(),
            )),
            wal: parking_lot::Mutex::new(None),
            next_tx_id: std::sync::atomic::AtomicU64::new(1),
            sessions: parking_lot::RwLock::new(std::collections::HashMap::new()),
            deadlock_priorities: std::sync::Arc::new(parking_lot::RwLock::new(
                std::collections::HashMap::new(),
            )),
            next_session_id: std::sync::atomic::AtomicU64::new(1),
            dirty_buffer: std::sync::Arc::new(parking_lot::Mutex::new(
                super::super::dirty_buffer::DirtyBuffer::new(),
            )),
            app_locks: super::super::applock::SharedAppLocks::new(),
        }
    }

    pub fn from_checkpoint_internal(checkpoint: RecoveryCheckpoint<C>) -> Self {
        let mut storage = S::default();
        let _ = storage.restore_from_checkpoint(checkpoint.storage_data);
        rebuild_all_indexes_from_catalog(&checkpoint.catalog, &mut storage);
        Self::from_parts_internal(
            checkpoint.catalog,
            storage,
            checkpoint.commit_ts,
            checkpoint.table_versions,
        )
    }

    pub fn apply_checkpoint(&self, checkpoint: RecoveryCheckpoint<C>) {
        let mut storage = self.storage.write();
        storage.catalog = checkpoint.catalog;
        let _ = storage
            .storage
            .restore_from_checkpoint(checkpoint.storage_data);
        let shared = &mut *storage;
        rebuild_all_indexes_from_catalog(&shared.catalog, &mut shared.storage);
        shared.commit_ts = checkpoint.commit_ts;
        shared.table_versions = checkpoint.table_versions;
        self.table_locks.lock().clear();
        let sessions: Vec<Arc<parking_lot::Mutex<_>>> =
            self.sessions.read().values().cloned().collect();
        for session_arc in sessions {
            session_arc.lock().reset();
        }
    }

    pub fn to_checkpoint(&self) -> RecoveryCheckpoint<C> {
        let storage = self.storage.read();
        self.to_checkpoint_internal(&storage)
    }

    pub fn to_checkpoint_internal(&self, storage: &SharedStorage<C, S>) -> RecoveryCheckpoint<C> {
        RecoveryCheckpoint {
            catalog: storage.catalog.clone(),
            storage_data: storage.storage.get_checkpoint_data(),
            commit_ts: storage.commit_ts,
            table_versions: storage.table_versions.clone(),
        }
    }

    pub fn wal_begin(&self, tx_id: u64, isolation_level: &str) {
        let mut wal_guard = self.wal.lock();
        if let Some(ref mut wal) = *wal_guard {
            let _ = wal.append(WalRecord::Begin {
                tx_id,
                isolation_level: isolation_level.to_string(),
            });
            let _ = wal.flush();
        }
    }

    pub fn wal_commit(&self, tx_id: u64) {
        let mut wal_guard = self.wal.lock();
        if let Some(ref mut wal) = *wal_guard {
            let lsn = wal.append(WalRecord::Commit { tx_id }).unwrap_or(Lsn(0));
            let _ = wal.append(WalRecord::Checkpoint { lsn });
            let _ = wal.flush();
            let _ = wal.truncate();
        }
    }

    pub fn wal_rollback(&self, tx_id: u64) {
        let mut wal_guard = self.wal.lock();
        if let Some(ref mut wal) = *wal_guard {
            let _ = wal.append(WalRecord::Rollback { tx_id });
            let _ = wal.flush();
        }
    }

    pub fn wal_savepoint(&self, tx_id: u64, name: &str) {
        let mut wal_guard = self.wal.lock();
        if let Some(ref mut wal) = *wal_guard {
            let _ = wal.append(WalRecord::Savepoint {
                tx_id,
                name: name.to_string(),
            });
            let _ = wal.flush();
        }
    }

    pub fn wal_auto_commit(&self, tx_id: u64) {
        let mut wal_guard = self.wal.lock();
        if let Some(ref mut wal) = *wal_guard {
            let lsn = wal
                .append(WalRecord::Begin {
                    tx_id,
                    isolation_level: "AutoCommit".to_string(),
                })
                .unwrap_or(Lsn(0));
            let _ = wal.append(WalRecord::Commit { tx_id });
            let _ = wal.append(WalRecord::Checkpoint { lsn });
            let _ = wal.flush();
            let _ = wal.truncate();
        }
    }

    pub fn allocate_tx_id(&self) -> u64 {
        self.next_tx_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }
}
