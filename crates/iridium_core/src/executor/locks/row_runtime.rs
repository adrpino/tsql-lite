use std::collections::HashMap;
use std::sync::Arc;

use crate::ast::IsolationLevel;
use crate::error::DbError;
use crate::executor::model::JoinedRow;
use crate::storage::StoredRow;

use super::hints::{canonical_lock_key_from_parts, TableLockHints};
use super::types::{AcquiredLock, LockMode, LockResource, SessionId};
use super::LockTable;

pub(crate) type SnapshotConflictChecker = Arc<dyn Fn(u32, usize) -> bool + Send + Sync>;
pub(crate) type CommittedRowFetcher = Arc<dyn Fn(u32, usize) -> Option<StoredRow> + Send + Sync>;

/// Per-statement runtime handle attached to `SessionExtras` during query/DML execution
/// so row-level scans and mutations can apply SQL Server locking hints (`UPDLOCK`,
/// `ROWLOCK`, `READPAST`, `HOLDLOCK`, `NOLOCK`, `READCOMMITTEDLOCK`, `NOWAIT`),
/// RCSI non-blocking reads, and `SNAPSHOT` row-level update conflict detection (`3960`).
#[derive(Clone)]
pub(crate) struct RowLockRuntime {
    pub(crate) table_locks: Arc<parking_lot::Mutex<LockTable>>,
    pub(crate) session_id: SessionId,
    pub(crate) active_db: String,
    pub(crate) isolation_level: IsolationLevel,
    pub(crate) rcsi_enabled: bool,
    pub(crate) allow_snapshot_isolation: bool,
    pub(crate) in_transaction: bool,
    pub(crate) savepoint_depth: usize,
    pub(crate) timeout_ms: i64,
    pub(crate) deadlock_priority: i32,
    pub(crate) priority_lookup: Arc<dyn Fn(SessionId) -> i32 + Send + Sync>,
    pub(crate) hints_by_table: HashMap<String, TableLockHints>,
    pub(crate) acquired_locks: Arc<parking_lot::Mutex<Vec<AcquiredLock>>>,
    /// Returns `true` if `(table_id, row_idx)` was modified/deleted by a committed transaction
    /// since this `SNAPSHOT` transaction started.
    pub(crate) snapshot_conflict_checker: Option<SnapshotConflictChecker>,
    /// Fetches the latest committed row for `(table_id, row_idx)` after a row lock is acquired,
    /// enabling SQL Server's lock-then-recheck semantics under concurrent workers.
    pub(crate) committed_row_fetcher: Option<CommittedRowFetcher>,
}

impl std::fmt::Debug for RowLockRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RowLockRuntime")
            .field("session_id", &self.session_id)
            .field("active_db", &self.active_db)
            .field("isolation_level", &self.isolation_level)
            .field("rcsi_enabled", &self.rcsi_enabled)
            .field("allow_snapshot_isolation", &self.allow_snapshot_isolation)
            .field("in_transaction", &self.in_transaction)
            .field("timeout_ms", &self.timeout_ms)
            .field("hints_by_table", &self.hints_by_table)
            .finish()
    }
}

impl RowLockRuntime {
    pub(crate) fn hints_for_table(&self, schema_raw: &str, table_name: &str) -> TableLockHints {
        let key = canonical_lock_key_from_parts(schema_raw, table_name, &self.active_db);
        self.hints_by_table.get(&key).cloned().unwrap_or_default()
    }

    pub(crate) fn canonical_key(&self, schema_raw: &str, table_name: &str) -> String {
        canonical_lock_key_from_parts(schema_raw, table_name, &self.active_db)
    }

    /// Acquires a row-level `Write` lock on `(schema_raw, table_name, row_idx)` for an
    /// `UPDATE` / `DELETE` / `MERGE` statement, and enforces `SNAPSHOT` update conflict (`3960`).
    pub(crate) fn acquire_write_row_lock_and_check_snapshot(
        &self,
        table_id: u32,
        schema_raw: &str,
        table_name: &str,
        row_idx: usize,
    ) -> Result<(), DbError> {
        let hints = self.hints_for_table(schema_raw, table_name);
        let eff_timeout = if hints.nowait { 0 } else { self.timeout_ms };
        let key = self.canonical_key(schema_raw, table_name);

        LockTable::acquire_row_lock_for_hint(
            &self.table_locks,
            self.session_id,
            &key,
            row_idx,
            LockMode::Write,
            eff_timeout,
            self.deadlock_priority,
            self.priority_lookup.as_ref(),
        )?;

        self.acquired_locks.lock().push(AcquiredLock {
            resource: LockResource::Row(key.clone(), row_idx),
            mode: LockMode::Write,
            savepoint_depth: self.savepoint_depth,
        });

        // After acquiring the row lock (waiting for any in-flight writer to commit/rollback),
        // check whether this row changed since our SNAPSHOT transaction began.
        if self.isolation_level == IsolationLevel::Snapshot && self.allow_snapshot_isolation {
            if let Some(ref checker) = self.snapshot_conflict_checker {
                if checker(table_id, row_idx) {
                    return Err(DbError::Execution(format!(
                        "Snapshot isolation transaction aborted due to update conflict. You cannot use snapshot isolation to access table '{}' directly or indirectly in database '{}' to update, delete, or modify the row that has been modified or deleted by another transaction. Retry the transaction.",
                        table_name, self.active_db
                    )));
                }
            }
        }
        Ok(())
    }

    /// Applies row-level locking hints and `READ COMMITTED` / `READPAST` / `UPDLOCK` /
    /// `XLOCK` / `HOLDLOCK` semantics to ordered candidate rows before projection/TOP.
    ///
    /// When `top_limit` is `Some(n)`, stops locking once `n` unlocked/locked rows are collected
    /// so `SELECT TOP (1) ... WITH (UPDLOCK, ROWLOCK, READPAST)` locks ONLY the claimed row!
    pub(crate) fn apply_select_row_locking<F>(
        &self,
        rows: Vec<JoinedRow>,
        top_limit: Option<usize>,
        mut recheck_predicate: F,
    ) -> Result<Vec<JoinedRow>, DbError>
    where
        F: FnMut(&JoinedRow) -> Result<bool, DbError>,
    {
        // Quick exit if nothing in this query needs per-row lock inspection.
        let needs_any_check = self.needs_row_check_for_rows(&rows);
        if !needs_any_check {
            return Ok(rows);
        }

        let mut out = Vec::with_capacity(top_limit.map_or(rows.len(), |n| n.min(rows.len())));
        for mut row in rows {
            if let Some(limit) = top_limit {
                if out.len() >= limit {
                    break;
                }
            }

            let mut skip_row = false;
            let mut row_refreshed = false;
            let mut row_acquired: Vec<(String, usize, LockMode)> = Vec::new();

            for ct in &mut row {
                let Some(row_idx) = ct.storage_index else {
                    continue;
                };
                if ct.table.id == 0 {
                    continue;
                }
                let hints = self.hints_for_table(&ct.table.schema_name, &ct.table.name);
                if hints.nolock || self.isolation_level == IsolationLevel::ReadUncommitted {
                    continue;
                }
                let key = self.canonical_key(&ct.table.schema_name, &ct.table.name);
                let eff_timeout = if hints.nowait { 0 } else { self.timeout_ms };

                if hints.readpast {
                    if hints.requires_any_row_lock_held() {
                        let mode = if hints.requires_write_row_lock() {
                            LockMode::Write
                        } else {
                            LockMode::Read
                        };
                        let acquired = self.table_locks.lock().try_acquire_row_lock_readpast(
                            self.session_id,
                            &key,
                            row_idx,
                            mode,
                        );
                        if !acquired {
                            skip_row = true;
                            break;
                        }
                        row_acquired.push((key, row_idx, mode));
                    } else {
                        let blocked = self.table_locks.lock().is_row_blocked_readpast(
                            self.session_id,
                            &key,
                            row_idx,
                            LockMode::Read,
                        );
                        if blocked {
                            skip_row = true;
                            break;
                        }
                    }
                } else if hints.requires_any_row_lock_held() {
                    let mode = if hints.requires_write_row_lock() {
                        LockMode::Write
                    } else {
                        LockMode::Read
                    };
                    LockTable::acquire_row_lock_for_hint(
                        &self.table_locks,
                        self.session_id,
                        &key,
                        row_idx,
                        mode,
                        eff_timeout,
                        self.deadlock_priority,
                        self.priority_lookup.as_ref(),
                    )?;
                    row_acquired.push((key, row_idx, mode));
                } else if self.isolation_level == IsolationLevel::ReadCommitted
                    && (!self.rcsi_enabled || hints.readcommittedlock)
                {
                    // Standard SQL Server READ COMMITTED (RCSI OFF) or explicit READCOMMITTEDLOCK:
                    // wait for any conflicting uncommitted row write lock held by another session.
                    LockTable::wait_for_row_unlocked(
                        &self.table_locks,
                        self.session_id,
                        &key,
                        row_idx,
                        LockMode::Read,
                        eff_timeout,
                        self.deadlock_priority,
                        self.priority_lookup.as_ref(),
                    )?;
                }

                // Lock-then-recheck: after acquiring/waiting on a row lock under READ COMMITTED,
                // re-fetch the latest committed row to ensure a concurrent committer did not
                // already modify or delete it before we locked it.
                if self.isolation_level == IsolationLevel::ReadCommitted
                    && (hints.requires_any_row_lock_held()
                        || hints.readpast
                        || !self.rcsi_enabled
                        || hints.readcommittedlock)
                {
                    if let Some(ref fetcher) = self.committed_row_fetcher {
                        if let Some(fresh) = fetcher(ct.table.id, row_idx) {
                            if fresh.deleted {
                                skip_row = true;
                                break;
                            }
                            if ct.row.as_ref() != Some(&fresh) {
                                ct.row = Some(fresh);
                                row_refreshed = true;
                            }
                        }
                    }
                }
            }

            if !skip_row && row_refreshed && !recheck_predicate(&row)? {
                skip_row = true;
            }

            if skip_row {
                // Release any locks acquired for earlier tables in this skipped joined row.
                if !row_acquired.is_empty() {
                    let mut guard = self.table_locks.lock();
                    for (k, ridx, m) in row_acquired {
                        guard.release_row_lock(self.session_id, &k, ridx, m);
                    }
                }
                continue;
            }

            if !row_acquired.is_empty() {
                let mut acq = self.acquired_locks.lock();
                for (k, ridx, m) in row_acquired {
                    acq.push(AcquiredLock {
                        resource: LockResource::Row(k, ridx),
                        mode: m,
                        savepoint_depth: self.savepoint_depth,
                    });
                }
            }
            out.push(row);
        }

        Ok(out)
    }

    fn needs_row_check_for_rows(&self, rows: &[JoinedRow]) -> bool {
        if rows.is_empty() {
            return false;
        }
        if !self.hints_by_table.is_empty() {
            return true;
        }
        if self.isolation_level == IsolationLevel::ReadCommitted && !self.rcsi_enabled {
            // Fast path: only inspect rows if there are active row locks anywhere in the engine.
            let guard = self.table_locks.lock();
            return guard.has_any_row_locks_for_other_session(self.session_id);
        }
        false
    }
}
