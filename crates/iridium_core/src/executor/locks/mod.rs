pub(crate) mod hints;
pub(crate) mod row_locks;
pub(crate) mod row_runtime;
pub(crate) mod table_locks;
mod types;
mod workspace;

pub use hints::TableLockHints;
pub use types::{AcquiredLock, LockMode, LockResource, SessionId, TableLockState};
pub use workspace::TxWorkspace;

use crate::ast::{IsolationLevel, Statement};
use crate::error::DbError;

use super::table_util::{collect_read_tables, collect_write_tables};
use row_locks::RowLockManager;
use table_locks::TableLockManager;

#[cfg(not(target_arch = "wasm32"))]
type LockStart = std::time::Instant;

#[cfg(target_arch = "wasm32")]
type LockStart = f64;

#[cfg(not(target_arch = "wasm32"))]
fn lock_start() -> LockStart {
    std::time::Instant::now()
}

#[cfg(target_arch = "wasm32")]
fn lock_start() -> LockStart {
    js_sys::Date::now()
}

#[cfg(not(target_arch = "wasm32"))]
fn elapsed_millis_since(start: &LockStart) -> u128 {
    start.elapsed().as_millis()
}

#[cfg(target_arch = "wasm32")]
fn elapsed_millis_since(start: &LockStart) -> u128 {
    (js_sys::Date::now() - *start) as u128
}

/// Unified lock manager that delegates to table-level and row-level sub-managers.
pub struct LockTable {
    tables: TableLockManager,
    rows: RowLockManager,
    explicit_tablock_x: std::collections::HashMap<String, SessionId>,
    deadlock_victims: std::collections::HashSet<SessionId>,
    pub condvar: std::sync::Arc<parking_lot::Condvar>,
    pub wait_for_graph: super::deadlock::WaitForGraph,
}

impl Default for LockTable {
    fn default() -> Self {
        Self {
            tables: TableLockManager::new(),
            rows: RowLockManager::new(),
            explicit_tablock_x: std::collections::HashMap::new(),
            deadlock_victims: std::collections::HashSet::new(),
            condvar: std::sync::Arc::new(parking_lot::Condvar::new()),
            wait_for_graph: super::deadlock::WaitForGraph::new(),
        }
    }
}

// ── Public property access ───────────────────────────────────────

impl LockTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    pub fn clear(&mut self) {
        self.tables.clear();
        self.rows.clear();
        self.explicit_tablock_x.clear();
        self.deadlock_victims.clear();
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.tables.keys()
    }

    pub fn set_escalation_threshold(&mut self, threshold: usize) {
        self.rows.escalation_threshold = threshold;
    }

    fn take_deadlock_victim(&mut self, session_id: SessionId) -> bool {
        self.deadlock_victims.remove(&session_id)
    }

    fn mark_deadlock_victim(&mut self, session_id: SessionId) {
        self.deadlock_victims.insert(session_id);
        self.condvar.notify_all();
    }

    fn choose_deadlock_victim(
        cycle: &[SessionId],
        current_session: SessionId,
        deadlock_priority: i32,
        priority_lookup: &dyn Fn(SessionId) -> i32,
    ) -> SessionId {
        let mut best = current_session;
        let mut best_priority = deadlock_priority;
        for &sid in cycle {
            let priority = priority_lookup(sid);
            if priority < best_priority || (priority == best_priority && sid == current_session) {
                best = sid;
                best_priority = priority;
            }
        }
        best
    }
}

// ── Statement-level table locking ────────────────────────────────

#[allow(clippy::too_many_arguments)]
impl LockTable {
    pub fn acquire_statement_locks<C, S>(
        state_lock: &parking_lot::Mutex<LockTable>,
        session_id: SessionId,
        tx_manager: &super::transaction::TransactionManager<C, S, super::session::SessionSnapshot>,
        workspace_slot: &mut Option<TxWorkspace<C, S>>,
        stmt: &Statement,
        timeout_ms: i64,
        deadlock_priority: i32,
        priority_lookup: &dyn Fn(SessionId) -> i32,
    ) -> Result<(), DbError> {
        let empty_hints = std::collections::HashMap::new();
        Self::acquire_statement_locks_with_hints(
            state_lock,
            session_id,
            tx_manager,
            workspace_slot,
            stmt,
            "master",
            false,
            &empty_hints,
            timeout_ms,
            deadlock_priority,
            priority_lookup,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn acquire_statement_locks_with_hints<C, S>(
        state_lock: &parking_lot::Mutex<LockTable>,
        session_id: SessionId,
        tx_manager: &super::transaction::TransactionManager<C, S, super::session::SessionSnapshot>,
        workspace_slot: &mut Option<TxWorkspace<C, S>>,
        stmt: &Statement,
        active_db: &str,
        allow_snapshot_isolation: bool,
        hints_by_table: &std::collections::HashMap<String, TableLockHints>,
        timeout_ms: i64,
        deadlock_priority: i32,
        priority_lookup: &dyn Fn(SessionId) -> i32,
    ) -> Result<(), DbError> {
        let read_tables = collect_read_tables(stmt);
        let write_tables = collect_write_tables(stmt);
        let depth = tx_manager
            .active
            .as_ref()
            .map(|tx| tx.savepoints.len())
            .unwrap_or(0);
        let isolation_level = tx_manager
            .active
            .as_ref()
            .map(|tx| tx.isolation_level)
            .unwrap_or(tx_manager.session_isolation_level);

        let effective_timeout_ms = if hints_by_table.values().any(|h| h.nowait) {
            0
        } else {
            timeout_ms
        };
        let db_prefix = active_db.to_ascii_lowercase();
        let qualify_key = |raw: &str| -> String {
            if raw.starts_with("__") || raw.starts_with('(') {
                raw.to_string()
            } else {
                format!("{}.{}", db_prefix, raw)
            }
        };

        let is_row_lockable_dml = matches!(
            stmt,
            Statement::Dml(
                crate::ast::DmlStatement::Update(_)
                    | crate::ast::DmlStatement::Delete(_)
                    | crate::ast::DmlStatement::Merge(_)
            )
        );

        let default_read_lock_required = matches!(
            isolation_level,
            IsolationLevel::RepeatableRead | IsolationLevel::Serializable
        ) || (isolation_level == IsolationLevel::Snapshot
            && !allow_snapshot_isolation);

        let mut tables_to_lock: Vec<(String, LockMode, bool)> = Vec::new();
        if !read_tables.is_empty() && write_tables.is_empty() {
            for table in read_tables {
                let key = qualify_key(&table);
                let hints = hints_by_table.get(&key);
                if let Some(h) = hints {
                    if h.nolock {
                        continue;
                    }
                    if h.tablock {
                        let mode = if h.xlock {
                            LockMode::Write
                        } else {
                            LockMode::Read
                        };
                        tables_to_lock.push((key, mode, h.xlock));
                        continue;
                    }
                    if h.readpast || h.rowlock || h.updlock || h.xlock || h.holdlock {
                        // Handled at row granularity by RowLockRuntime.
                        continue;
                    }
                }
                if default_read_lock_required {
                    tables_to_lock.push((key, LockMode::Read, false));
                }
            }
        }
        {
            let pre_guard = state_lock.lock();
            for table in write_tables {
                let key = qualify_key(&table);
                let hints = hints_by_table.get(&key);
                let explicit_tablock_x = hints.is_some_and(|h| h.tablock && h.xlock);
                let use_row_only = !explicit_tablock_x
                    && is_row_lockable_dml
                    && (hints.is_some_and(|h| h.rowlock || h.readpast)
                        || (isolation_level == IsolationLevel::Snapshot
                            && allow_snapshot_isolation)
                        || pre_guard.rows.session_holds_lock_on_table(session_id, &key));
                if !use_row_only {
                    tables_to_lock.push((key, LockMode::Write, explicit_tablock_x));
                }
            }
        }

        if tables_to_lock.is_empty() {
            return Ok(());
        }

        let start = lock_start();
        let mut guard = state_lock.lock();

        loop {
            if guard.take_deadlock_victim(session_id) {
                return Err(DbError::Deadlock(
                    "Transaction was deadlocked on lock resources with another process and has been chosen as the deadlock victim.".into(),
                ));
            }

            let mut all_acquired = true;
            let mut conflict_info = None;
            let mut holders = std::collections::HashSet::new();

            for (table, mode, _) in &tables_to_lock {
                if !guard.tables.can_acquire(session_id, table, *mode) {
                    all_acquired = false;
                    conflict_info = Some((table.clone(), *mode));
                    for h in guard.tables.get_blocking_sessions(session_id, table, *mode) {
                        holders.insert(h);
                    }
                    break;
                }
            }

            if all_acquired {
                guard.wait_for_graph.remove_waiter(session_id);
                for (table, mode, is_explicit_tablock_x) in tables_to_lock {
                    guard
                        .tables
                        .acquire(session_id, workspace_slot, &table, mode, depth);
                    if is_explicit_tablock_x {
                        guard
                            .explicit_tablock_x
                            .insert(super::string_norm::normalize_identifier(&table), session_id);
                    }
                }
                return Ok(());
            }

            if effective_timeout_ms == 0 {
                let (table, mode) = conflict_info.ok_or_else(|| {
                    DbError::Execution("lock conflict without conflict info".into())
                })?;
                return Err(DbError::Execution(format!(
                    "lock conflict (no-wait): {:?} lock on '{}' is blocked",
                    mode, table
                )));
            }

            guard.wait_for_graph.remove_waiter(session_id);
            for &holder in &holders {
                guard.wait_for_graph.add_edge(session_id, holder);
            }

            if let Some(cycle) = guard.wait_for_graph.detect_cycle(session_id) {
                let victim = Self::choose_deadlock_victim(
                    &cycle,
                    session_id,
                    deadlock_priority,
                    priority_lookup,
                );
                if victim == session_id {
                    guard.wait_for_graph.remove_waiter(session_id);
                    return Err(DbError::Deadlock(format!(
                        "Transaction was deadlocked on lock resources with another process and has been chosen as the deadlock victim. Cycle: {:?}",
                        cycle
                    )));
                }
                guard.mark_deadlock_victim(victim);
            }

            let elapsed_ms = elapsed_millis_since(&start);
            if effective_timeout_ms > 0 && elapsed_ms >= effective_timeout_ms as u128 {
                guard.wait_for_graph.remove_waiter(session_id);
                let (table, mode) = conflict_info.ok_or_else(|| {
                    DbError::Execution("lock timeout without conflict info".into())
                })?;
                return Err(DbError::Execution(format!(
                    "lock timeout ({}ms): {:?} lock on '{}' is blocked",
                    effective_timeout_ms, mode, table
                )));
            }

            let condvar = guard.condvar.clone();
            if effective_timeout_ms < 0 {
                condvar.wait(&mut guard);
            } else {
                let remaining = std::time::Duration::from_millis(effective_timeout_ms as u64)
                    .saturating_sub(std::time::Duration::from_millis(elapsed_ms as u64));
                if condvar.wait_for(&mut guard, remaining).timed_out() {
                    guard.wait_for_graph.remove_waiter(session_id);
                    let (table, mode) = conflict_info.ok_or_else(|| {
                        DbError::Execution("lock timeout without conflict info".into())
                    })?;
                    return Err(DbError::Execution(format!(
                        "lock timeout ({}ms): {:?} lock on '{}' is blocked",
                        effective_timeout_ms, mode, table
                    )));
                }
            }
        }
    }

    pub fn get_blocking_sessions(
        &self,
        session_id: SessionId,
        table: &str,
        mode: LockMode,
    ) -> Vec<SessionId> {
        self.tables.get_blocking_sessions(session_id, table, mode)
    }

    pub fn perform_acquire_lock<C, S>(
        &mut self,
        session_id: SessionId,
        workspace_slot: &mut Option<TxWorkspace<C, S>>,
        table: &str,
        mode: LockMode,
        savepoint_depth: usize,
    ) {
        self.tables
            .acquire(session_id, workspace_slot, table, mode, savepoint_depth);
    }
}

// ── Row-level locking ────────────────────────────────────────────

impl LockTable {
    /// Check if a row lock can be acquired (considering both table- and row-level state).
    fn can_acquire_row(
        &self,
        session_id: SessionId,
        table: &str,
        row_id: usize,
        mode: LockMode,
    ) -> bool {
        // A conflicting table-level lock blocks any row lock
        if !self.tables.can_acquire(session_id, table, mode) {
            return false;
        }
        self.rows.can_acquire(session_id, table, row_id, mode)
    }

    /// Acquire a row lock and handle escalation if triggered.
    fn do_acquire_row(
        &mut self,
        session_id: SessionId,
        table: &str,
        row_id: usize,
        mode: LockMode,
    ) {
        if let Some(req) = self.rows.acquire(session_id, table, row_id, mode) {
            // Escalation triggered: remove row locks, promote to table lock
            self.rows.escalate_remove_rows(session_id, &req.table);
            self.tables.acquire_raw(session_id, &req.table, req.mode);
        }
    }

    /// Collect all sessions blocking a row lock (table-level + row-level).
    fn row_blocking_sessions(
        &self,
        session_id: SessionId,
        table: &str,
        row_id: usize,
        mode: LockMode,
    ) -> Vec<SessionId> {
        let mut blockers = self.tables.get_blocking_sessions(session_id, table, mode);
        for b in self
            .rows
            .get_blocking_sessions(session_id, table, row_id, mode)
        {
            if !blockers.contains(&b) {
                blockers.push(b);
            }
        }
        blockers
    }

    #[allow(clippy::too_many_arguments)]
    pub fn acquire_row_lock(
        state_lock: &parking_lot::Mutex<LockTable>,
        session_id: SessionId,
        table: &str,
        row_id: usize,
        mode: LockMode,
        timeout_ms: i64,
        deadlock_priority: i32,
        priority_lookup: &dyn Fn(SessionId) -> i32,
    ) -> Result<(), DbError> {
        let start = lock_start();
        let mut guard = state_lock.lock();

        loop {
            if guard.take_deadlock_victim(session_id) {
                return Err(DbError::Deadlock(
                    "Transaction was deadlocked on lock resources with another process and has been chosen as the deadlock victim.".into(),
                ));
            }

            if guard.can_acquire_row(session_id, table, row_id, mode) {
                guard.wait_for_graph.remove_waiter(session_id);
                guard.do_acquire_row(session_id, table, row_id, mode);
                return Ok(());
            }

            if timeout_ms == 0 {
                return Err(DbError::Execution(format!(
                    "lock conflict (no-wait): {:?} row lock on '{}' row {} is blocked",
                    mode, table, row_id
                )));
            }

            let holders = guard.row_blocking_sessions(session_id, table, row_id, mode);
            guard.wait_for_graph.remove_waiter(session_id);
            for &holder in &holders {
                guard.wait_for_graph.add_edge(session_id, holder);
            }

            if let Some(cycle) = guard.wait_for_graph.detect_cycle(session_id) {
                let victim = Self::choose_deadlock_victim(
                    &cycle,
                    session_id,
                    deadlock_priority,
                    priority_lookup,
                );
                if victim == session_id {
                    guard.wait_for_graph.remove_waiter(session_id);
                    return Err(DbError::Deadlock(format!(
                        "Transaction was deadlocked on lock resources with another process and has been chosen as the deadlock victim. Cycle: {:?}",
                        cycle
                    )));
                }
                guard.mark_deadlock_victim(victim);
            }

            let elapsed_ms = elapsed_millis_since(&start);
            if timeout_ms > 0 && elapsed_ms >= timeout_ms as u128 {
                guard.wait_for_graph.remove_waiter(session_id);
                return Err(DbError::Execution(format!(
                    "lock timeout ({}ms): {:?} row lock on '{}' row {} is blocked",
                    timeout_ms, mode, table, row_id
                )));
            }

            let condvar = guard.condvar.clone();
            if timeout_ms < 0 {
                condvar.wait(&mut guard);
            } else {
                let remaining = std::time::Duration::from_millis(timeout_ms as u64)
                    .saturating_sub(std::time::Duration::from_millis(elapsed_ms as u64));
                if condvar.wait_for(&mut guard, remaining).timed_out() {
                    guard.wait_for_graph.remove_waiter(session_id);
                    return Err(DbError::Execution(format!(
                        "lock timeout ({}ms): {:?} row lock on '{}' row {} is blocked",
                        timeout_ms, mode, table, row_id
                    )));
                }
            }
        }
    }

    pub fn release_row_lock(
        &mut self,
        session_id: SessionId,
        table: &str,
        row_id: usize,
        mode: LockMode,
    ) {
        self.rows.release_one(session_id, table, row_id, mode);
        self.condvar.notify_all();
    }

    pub fn get_row_blocking_sessions(
        &self,
        session_id: SessionId,
        table: &str,
        row_id: usize,
        mode: LockMode,
    ) -> Vec<SessionId> {
        self.row_blocking_sessions(session_id, table, row_id, mode)
    }

    pub fn has_any_row_locks_for_other_session(&self, session_id: SessionId) -> bool {
        self.rows.has_any_locks_for_other_session(session_id)
            || self.explicit_tablock_x.values().any(|&sid| sid != session_id)
    }

    fn explicit_tablock_blocker(&self, session_id: SessionId, table: &str) -> Option<SessionId> {
        let normalized = super::string_norm::normalize_identifier(table);
        self.explicit_tablock_x
            .get(&normalized)
            .copied()
            .filter(|&sid| sid != session_id)
    }

    fn can_acquire_row_for_hint(
        &self,
        session_id: SessionId,
        table: &str,
        row_id: usize,
        mode: LockMode,
    ) -> bool {
        if self.explicit_tablock_blocker(session_id, table).is_some() {
            return false;
        }
        // Respect conflicting table-level Read locks when requesting a Write row lock
        // (e.g., REPEATABLE READ / SERIALIZABLE table readers block row writers),
        // while allowing row-level locks on disjoint rows even if another session holds
        // an implicit table-level Write lock from updating a different row.
        if mode == LockMode::Write {
            let table_blockers = self.tables.get_blocking_sessions(session_id, table, LockMode::Write);
            for blocker in table_blockers {
                // Only block at table level if the blocker does NOT hold row locks (i.e., pure table reader/writer like SERIALIZABLE SELECT or INSERT)
                if !self.rows.session_holds_lock_on_table(blocker, table) {
                    return false;
                }
            }
        }
        self.rows.can_acquire(session_id, table, row_id, mode)
    }

    fn hint_row_blocking_sessions(
        &self,
        session_id: SessionId,
        table: &str,
        row_id: usize,
        mode: LockMode,
    ) -> Vec<SessionId> {
        let mut blockers = Vec::new();
        if let Some(b) = self.explicit_tablock_blocker(session_id, table) {
            blockers.push(b);
        }
        if mode == LockMode::Write {
            for b in self.tables.get_blocking_sessions(session_id, table, LockMode::Write) {
                if !self.rows.session_holds_lock_on_table(b, table) && !blockers.contains(&b) {
                    blockers.push(b);
                }
            }
        }
        for b in self.rows.get_blocking_sessions(session_id, table, row_id, mode) {
            if !blockers.contains(&b) {
                blockers.push(b);
            }
        }
        blockers
    }

    pub fn is_row_blocked_readpast(
        &self,
        session_id: SessionId,
        table: &str,
        row_id: usize,
        mode: LockMode,
    ) -> bool {
        !self.can_acquire_row_for_hint(session_id, table, row_id, mode)
    }

    pub fn try_acquire_row_lock_readpast(
        &mut self,
        session_id: SessionId,
        table: &str,
        row_id: usize,
        mode: LockMode,
    ) -> bool {
        if !self.can_acquire_row_for_hint(session_id, table, row_id, mode) {
            return false;
        }
        self.do_acquire_row(session_id, table, row_id, mode);
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub fn acquire_row_lock_for_hint(
        state_lock: &parking_lot::Mutex<LockTable>,
        session_id: SessionId,
        table: &str,
        row_id: usize,
        mode: LockMode,
        timeout_ms: i64,
        deadlock_priority: i32,
        priority_lookup: &dyn Fn(SessionId) -> i32,
    ) -> Result<(), DbError> {
        Self::wait_or_acquire_row_inner(
            state_lock,
            session_id,
            table,
            row_id,
            mode,
            true,
            timeout_ms,
            deadlock_priority,
            priority_lookup,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn wait_for_row_unlocked(
        state_lock: &parking_lot::Mutex<LockTable>,
        session_id: SessionId,
        table: &str,
        row_id: usize,
        mode: LockMode,
        timeout_ms: i64,
        deadlock_priority: i32,
        priority_lookup: &dyn Fn(SessionId) -> i32,
    ) -> Result<(), DbError> {
        Self::wait_or_acquire_row_inner(
            state_lock,
            session_id,
            table,
            row_id,
            mode,
            false,
            timeout_ms,
            deadlock_priority,
            priority_lookup,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn wait_or_acquire_row_inner(
        state_lock: &parking_lot::Mutex<LockTable>,
        session_id: SessionId,
        table: &str,
        row_id: usize,
        mode: LockMode,
        acquire: bool,
        timeout_ms: i64,
        deadlock_priority: i32,
        priority_lookup: &dyn Fn(SessionId) -> i32,
    ) -> Result<(), DbError> {
        let start = lock_start();
        let mut guard = state_lock.lock();

        loop {
            if guard.take_deadlock_victim(session_id) {
                return Err(DbError::Deadlock(
                    "Transaction was deadlocked on lock resources with another process and has been chosen as the deadlock victim.".into(),
                ));
            }

            if guard.can_acquire_row_for_hint(session_id, table, row_id, mode) {
                guard.wait_for_graph.remove_waiter(session_id);
                if acquire {
                    guard.do_acquire_row(session_id, table, row_id, mode);
                }
                return Ok(());
            }

            if timeout_ms == 0 {
                return Err(DbError::Execution(format!(
                    "lock conflict (no-wait): {:?} row lock on '{}' row {} is blocked",
                    mode, table, row_id
                )));
            }

            let holders = guard.hint_row_blocking_sessions(session_id, table, row_id, mode);
            guard.wait_for_graph.remove_waiter(session_id);
            for &holder in &holders {
                guard.wait_for_graph.add_edge(session_id, holder);
            }

            if let Some(cycle) = guard.wait_for_graph.detect_cycle(session_id) {
                let victim = Self::choose_deadlock_victim(
                    &cycle,
                    session_id,
                    deadlock_priority,
                    priority_lookup,
                );
                if victim == session_id {
                    guard.wait_for_graph.remove_waiter(session_id);
                    return Err(DbError::Deadlock(format!(
                        "Transaction was deadlocked on lock resources with another process and has been chosen as the deadlock victim. Cycle: {:?}",
                        cycle
                    )));
                }
                guard.mark_deadlock_victim(victim);
            }

            let elapsed_ms = elapsed_millis_since(&start);
            if timeout_ms > 0 && elapsed_ms >= timeout_ms as u128 {
                guard.wait_for_graph.remove_waiter(session_id);
                return Err(DbError::Execution(format!(
                    "lock timeout ({}ms): {:?} row lock on '{}' row {} is blocked",
                    timeout_ms, mode, table, row_id
                )));
            }

            let condvar = guard.condvar.clone();
            if timeout_ms < 0 {
                condvar.wait(&mut guard);
            } else {
                let remaining = std::time::Duration::from_millis(timeout_ms as u64)
                    .saturating_sub(std::time::Duration::from_millis(elapsed_ms as u64));
                if condvar.wait_for(&mut guard, remaining).timed_out() {
                    guard.wait_for_graph.remove_waiter(session_id);
                    return Err(DbError::Execution(format!(
                        "lock timeout ({}ms): {:?} row lock on '{}' row {} is blocked",
                        timeout_ms, mode, table, row_id
                    )));
                }
            }
        }
    }
}

// ── Workspace / session release ──────────────────────────────────

impl LockTable {
    pub fn release_workspace_locks<C, S>(
        &mut self,
        session_id: SessionId,
        workspace_slot: &mut Option<TxWorkspace<C, S>>,
        keep_depth_inclusive: usize,
    ) {
        let Some(workspace) = workspace_slot.as_mut() else {
            return;
        };

        let mut retained = Vec::with_capacity(workspace.acquired_locks.len());
        for lock in workspace.acquired_locks.drain(..) {
            if lock.savepoint_depth < keep_depth_inclusive {
                retained.push(lock);
                continue;
            }
            match &lock.resource {
                LockResource::Table(t) => {
                    self.tables.release_one(session_id, t, lock.mode);
                    if self.explicit_tablock_x.get(t).copied() == Some(session_id) {
                        self.explicit_tablock_x.remove(t);
                    }
                }
                LockResource::Row(t, row_id) => {
                    self.rows.release_one(session_id, t, *row_id, lock.mode);
                }
            }
        }
        workspace.acquired_locks = retained;
        self.condvar.notify_all();
    }

    pub fn release_all_for_session(&mut self, session_id: SessionId) {
        self.tables.release_all_for_session(session_id);
        self.rows.release_all_for_session(session_id);
        self.explicit_tablock_x.retain(|_, &mut sid| sid != session_id);
        self.wait_for_graph.remove_waiter(session_id);
        self.deadlock_victims.remove(&session_id);
        self.condvar.notify_all();
    }
}
