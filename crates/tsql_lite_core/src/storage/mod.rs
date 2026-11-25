pub mod btree_index;
#[cfg(feature = "persistence")]
mod redb_checkpoint;
#[cfg(feature = "persistence")]
mod redb_index_adapter;
#[cfg(feature = "persistence")]
mod redb_row_storage;
#[cfg(feature = "persistence")]
pub mod redb_storage;
pub use btree_index::{BTreeIndex, IndexStorage};
#[cfg(feature = "persistence")]
pub use redb_storage::RedbStorage;

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::DbError;
use crate::types::Value;
use serde::{Deserialize, Serialize};

pub type StorageRowStream<'a> = Box<dyn Iterator<Item = Result<StoredRow, DbError>> + 'a>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredRow {
    pub values: Vec<Value>,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageCheckpointData {
    InMemory(HashMap<u32, Vec<StoredRow>>),
    Persistent,
}

#[derive(Debug, Clone)]
pub enum StorageSnapshot {
    /// O(1) copy-on-write snapshot for `InMemoryStorage` (`Arc::clone` of `base` + small `delta`).
    InMemory(InMemoryStorage),
    /// Fallback per-table row snapshot for non-in-memory storage backends.
    Tables(HashMap<u32, Vec<StoredRow>>),
}

pub trait Storage: std::fmt::Debug + Send + Sync {
    fn scan_rows<'a>(&'a self, table_id: u32) -> Result<StorageRowStream<'a>, DbError>;

    fn get_row(&self, table_id: u32, index: usize) -> Result<Option<StoredRow>, DbError>;

    fn get_rows(&self, table_id: u32) -> Result<Vec<StoredRow>, DbError> {
        self.scan_rows(table_id)?.collect()
    }

    /// Zero-copy slice access to all stored rows of `table_id` when backed by in-memory storage.
    fn get_table_slice(&self, _table_id: u32) -> Option<&[Arc<StoredRow>]> {
        None
    }

    fn insert_row(&mut self, table_id: u32, row: StoredRow) -> Result<(), DbError>;
    fn update_row(&mut self, table_id: u32, index: usize, row: StoredRow) -> Result<(), DbError>;
    fn delete_row(&mut self, table_id: u32, index: usize) -> Result<(), DbError>;
    fn replace_table(&mut self, table_id: u32, rows: Vec<StoredRow>) -> Result<(), DbError>;
    fn clear_table(&mut self, table_id: u32) -> Result<(), DbError>;
    fn remove_table(&mut self, table_id: u32) -> Result<(), DbError>;
    fn ensure_table(&mut self, table_id: u32) -> Result<(), DbError>;

    fn clone_boxed(&self) -> Box<dyn Storage>;

    /// Captures a fast snapshot of storage state (O(1) `Arc` clone for `InMemoryStorage`).
    /// Returns `None` when the backend requires a catalog-guided per-table copy.
    fn take_fast_snapshot(&self) -> Option<StorageSnapshot> {
        None
    }

    /// Restores storage state from a snapshot captured by `take_fast_snapshot`, preserving
    /// any table IDs in `preserve_table_ids` (e.g. table variables that survive `ROLLBACK`).
    fn restore_fast_snapshot(
        &mut self,
        _snapshot: &StorageSnapshot,
        _preserve_table_ids: &std::collections::HashSet<u32>,
    ) -> Result<bool, DbError> {
        Ok(false)
    }

    fn as_index_storage(&self) -> Option<&dyn IndexStorage> {
        None
    }
    fn as_index_storage_mut(&mut self) -> Option<&mut dyn IndexStorage> {
        None
    }

    /// Merges changes made in `workspace` relative to `base` into `self` during
    /// a concurrent transaction commit.
    fn merge_workspace(&mut self, workspace: Self, _base: &Self)
    where
        Self: Sized,
    {
        *self = workspace;
    }
}

pub trait CheckpointableStorage: Storage {
    fn get_checkpoint_data(&self) -> StorageCheckpointData;
    fn restore_from_checkpoint(&mut self, data: StorageCheckpointData) -> Result<(), DbError>;
    fn clone_checkpointable(&self) -> Box<dyn CheckpointableStorage>;
}

/// Copy-on-write in-memory table storage backed by a shared immutable `base` map
/// plus a small per-fork `delta` overlay (`None` = deleted table, `Some(rows)` = added/modified).
///
/// - Before any clone (`Arc::strong_count(&self.base) == 1 && self.delta.is_empty()`),
///   mutations update `base` directly in place with zero overlay overhead.
/// - After `clone()` / `fork_isolated()`, mutations record only touched table IDs in
///   `delta` in O(1), avoiding cloning the full `base` HashMap on every fork or transaction.
type SharedTableRows = Arc<Vec<Arc<StoredRow>>>;

#[derive(Debug, Default, Clone)]
pub struct InMemoryStorage {
    base: Arc<HashMap<u32, SharedTableRows>>,
    delta: HashMap<u32, Option<SharedTableRows>>,
}

impl InMemoryStorage {
    #[inline]
    fn is_unshared(&self) -> bool {
        self.delta.is_empty() && Arc::strong_count(&self.base) == 1
    }

    #[inline]
    fn get_table(&self, table_id: u32) -> Option<&Arc<Vec<Arc<StoredRow>>>> {
        if !self.delta.is_empty() {
            if let Some(entry) = self.delta.get(&table_id) {
                return entry.as_ref();
            }
        }
        self.base.get(&table_id)
    }

    fn get_table_mut_arc(&mut self, table_id: u32) -> Option<&mut Arc<Vec<Arc<StoredRow>>>> {
        if self.is_unshared() {
            return Arc::make_mut(&mut self.base).get_mut(&table_id);
        }
        use std::collections::hash_map::Entry;
        match self.delta.entry(table_id) {
            Entry::Occupied(entry) => entry.into_mut().as_mut(),
            Entry::Vacant(entry) => {
                let base_arc = self.base.get(&table_id)?;
                entry.insert(Some(Arc::clone(base_arc))).as_mut()
            }
        }
    }

    fn materialized_tables(&self) -> HashMap<u32, &[Arc<StoredRow>]> {
        let mut out: HashMap<u32, &[Arc<StoredRow>]> =
            HashMap::with_capacity(self.base.len() + self.delta.len());
        for (&k, v) in self.base.iter() {
            out.insert(k, v.as_slice());
        }
        for (&k, v) in &self.delta {
            match v {
                Some(rows) => {
                    out.insert(k, rows.as_slice());
                }
                None => {
                    out.remove(&k);
                }
            }
        }
        out
    }

    /// Returns `true` if `self` and `other` share the exact same underlying `base` map pointer.
    #[doc(hidden)]
    pub fn shares_base_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.base, &other.base)
    }
}

impl Serialize for InMemoryStorage {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wire<'a> {
            tables: HashMap<u32, Vec<&'a StoredRow>>,
        }
        let tables = self
            .materialized_tables()
            .into_iter()
            .map(|(k, slice)| (k, slice.iter().map(|r| r.as_ref()).collect()))
            .collect();
        Wire { tables }.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for InMemoryStorage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            tables: HashMap<u32, Vec<StoredRow>>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            base: Arc::new(
                wire.tables
                    .into_iter()
                    .map(|(k, v)| (k, Arc::new(v.into_iter().map(Arc::new).collect())))
                    .collect(),
            ),
            delta: HashMap::new(),
        })
    }
}

impl Storage for InMemoryStorage {
    fn scan_rows<'a>(&'a self, table_id: u32) -> Result<StorageRowStream<'a>, DbError> {
        let rows = self
            .get_table(table_id)
            .ok_or_else(|| DbError::Storage(format!("table {} not found in storage", table_id)))?;

        Ok(Box::new(rows.iter().map(|r| Ok(r.as_ref().clone()))))
    }

    fn get_row(&self, table_id: u32, index: usize) -> Result<Option<StoredRow>, DbError> {
        let rows = self
            .get_table(table_id)
            .ok_or_else(|| DbError::Storage(format!("table {} not found", table_id)))?;
        Ok(rows.get(index).map(|r| r.as_ref().clone()))
    }

    fn insert_row(&mut self, table_id: u32, row: StoredRow) -> Result<(), DbError> {
        let rows = self
            .get_table_mut_arc(table_id)
            .ok_or_else(|| DbError::Storage(format!("table {} not found in storage", table_id)))?;
        Arc::make_mut(rows).push(Arc::new(row));
        Ok(())
    }

    fn update_row(&mut self, table_id: u32, index: usize, row: StoredRow) -> Result<(), DbError> {
        let rows = self
            .get_table_mut_arc(table_id)
            .ok_or_else(|| DbError::Storage(format!("table {} not found", table_id)))?;
        if index >= rows.len() {
            return Err(DbError::Storage(format!(
                "index {} out of bounds for table {}",
                index, table_id
            )));
        }
        Arc::make_mut(rows)[index] = Arc::new(row);
        Ok(())
    }

    fn delete_row(&mut self, table_id: u32, index: usize) -> Result<(), DbError> {
        let rows = self
            .get_table_mut_arc(table_id)
            .ok_or_else(|| DbError::Storage(format!("table {} not found", table_id)))?;
        if index >= rows.len() {
            return Err(DbError::Storage(format!(
                "index {} out of bounds for table {}",
                index, table_id
            )));
        }
        Arc::make_mut(&mut Arc::make_mut(rows)[index]).deleted = true;
        Ok(())
    }

    fn replace_table(&mut self, table_id: u32, rows: Vec<StoredRow>) -> Result<(), DbError> {
        if self.get_table(table_id).is_none() {
            return Err(DbError::Storage(format!(
                "table {} not found in storage",
                table_id
            )));
        }
        let arc_rows: Vec<Arc<StoredRow>> = rows.into_iter().map(Arc::new).collect();
        if self.is_unshared() {
            Arc::make_mut(&mut self.base).insert(table_id, Arc::new(arc_rows));
        } else {
            self.delta.insert(table_id, Some(Arc::new(arc_rows)));
        }
        Ok(())
    }

    fn clear_table(&mut self, table_id: u32) -> Result<(), DbError> {
        let Some(existing) = self.get_table(table_id) else {
            return Err(DbError::Storage(format!("table {} not found", table_id)));
        };
        if existing.is_empty() {
            return Ok(());
        }
        if self.is_unshared() {
            if let Some(rows) = Arc::make_mut(&mut self.base).get_mut(&table_id) {
                Arc::make_mut(rows).clear();
            }
        } else {
            self.delta.insert(table_id, Some(Arc::new(Vec::new())));
        }
        Ok(())
    }

    fn ensure_table(&mut self, table_id: u32) -> Result<(), DbError> {
        if self.get_table(table_id).is_some() {
            return Ok(());
        }
        if self.is_unshared() {
            Arc::make_mut(&mut self.base).insert(table_id, Arc::new(Vec::new()));
        } else {
            self.delta.insert(table_id, Some(Arc::new(Vec::new())));
        }
        Ok(())
    }

    fn remove_table(&mut self, table_id: u32) -> Result<(), DbError> {
        if self.get_table(table_id).is_none() {
            return Ok(());
        }
        if self.is_unshared() {
            Arc::make_mut(&mut self.base).remove(&table_id);
        } else if self.base.contains_key(&table_id) {
            self.delta.insert(table_id, None);
        } else {
            self.delta.remove(&table_id);
        }
        Ok(())
    }

    fn clone_boxed(&self) -> Box<dyn Storage> {
        Box::new(self.clone())
    }

    fn take_fast_snapshot(&self) -> Option<StorageSnapshot> {
        Some(StorageSnapshot::InMemory(self.clone()))
    }

    fn restore_fast_snapshot(
        &mut self,
        snapshot: &StorageSnapshot,
        preserve_table_ids: &std::collections::HashSet<u32>,
    ) -> Result<bool, DbError> {
        let StorageSnapshot::InMemory(snap) = snapshot else {
            return Ok(false);
        };
        let mut overrides: Vec<(u32, Arc<Vec<Arc<StoredRow>>>)> = Vec::new();
        for &tid in preserve_table_ids {
            if let Some(cur_rows) = self.get_table(tid) {
                let unchanged = snap
                    .get_table(tid)
                    .is_some_and(|s_rows| Arc::ptr_eq(cur_rows, s_rows));
                if !unchanged {
                    overrides.push((tid, Arc::clone(cur_rows)));
                }
            }
        }
        for (&tid, entry) in &self.delta {
            if entry.is_some()
                && snap.get_table(tid).is_none()
                && !preserve_table_ids.contains(&tid)
            {
                overrides.push((tid, Arc::new(Vec::new())));
            }
        }
        if !Arc::ptr_eq(&self.base, &snap.base) {
            for &tid in self.base.keys() {
                if self.get_table(tid).is_some()
                    && snap.get_table(tid).is_none()
                    && !preserve_table_ids.contains(&tid)
                {
                    overrides.push((tid, Arc::new(Vec::new())));
                }
            }
        }
        self.base = Arc::clone(&snap.base);
        self.delta = snap.delta.clone();
        for (tid, rows) in overrides {
            self.delta.insert(tid, Some(rows));
        }
        Ok(true)
    }

    fn as_index_storage(&self) -> Option<&dyn IndexStorage> {
        None
    }
    fn as_index_storage_mut(&mut self) -> Option<&mut dyn IndexStorage> {
        None
    }

    fn get_table_slice(&self, table_id: u32) -> Option<&[Arc<StoredRow>]> {
        self.get_table(table_id).map(|arc| arc.as_slice())
    }

    fn merge_workspace(&mut self, workspace: Self, base: &Self) {
        if Arc::ptr_eq(&workspace.base, &base.base)
            && workspace.delta.is_empty()
            && base.delta.is_empty()
        {
            return;
        }
        let mut changed: Vec<(u32, Option<SharedTableRows>)> = Vec::new();
        if Arc::ptr_eq(&workspace.base, &base.base) {
            for (&tid, ws_opt) in &workspace.delta {
                let base_opt = base.get_table(tid);
                let unchanged = match (ws_opt.as_ref(), base_opt) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                };
                if !unchanged {
                    changed.push((tid, ws_opt.as_ref().map(Arc::clone)));
                }
            }
            for &tid in base.delta.keys() {
                if !workspace.delta.contains_key(&tid) {
                    let ws_opt = workspace.get_table(tid);
                    let base_opt = base.get_table(tid);
                    let unchanged = match (ws_opt, base_opt) {
                        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                        (None, None) => true,
                        _ => false,
                    };
                    if !unchanged {
                        changed.push((tid, ws_opt.map(Arc::clone)));
                    }
                }
            }
        } else {
            let mut all_ids: std::collections::HashSet<u32> = std::collections::HashSet::new();
            all_ids.extend(workspace.base.keys().copied());
            all_ids.extend(workspace.delta.keys().copied());
            all_ids.extend(base.base.keys().copied());
            all_ids.extend(base.delta.keys().copied());
            for tid in all_ids {
                let ws_opt = workspace.get_table(tid);
                let base_opt = base.get_table(tid);
                let unchanged = match (ws_opt, base_opt) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (None, None) => true,
                    _ => false,
                };
                if !unchanged {
                    changed.push((tid, ws_opt.map(Arc::clone)));
                }
            }
        }
        if changed.is_empty() {
            return;
        }
        // Perform 3-way row-level merge when both `self` (latest committed state) and `workspace`
        // diverged from `base` on the same table (e.g., concurrent row-locked workers or snapshot
        // transactions modifying disjoint rows of the same table).
        let merged_changes: Vec<(u32, Option<SharedTableRows>)> = changed
            .into_iter()
            .map(|(tid, opt_ws)| {
                let merged = match (opt_ws, self.get_table(tid), base.get_table(tid)) {
                    (Some(ws_rows), Some(cur_rows), Some(base_rows))
                        if !Arc::ptr_eq(cur_rows, base_rows)
                            && ws_rows.len() >= base_rows.len() =>
                    {
                        let mut out = cur_rows.as_ref().clone();
                        let common = base_rows.len().min(out.len()).min(ws_rows.len());
                        for i in 0..common {
                            if !Arc::ptr_eq(&ws_rows[i], &base_rows[i])
                                && ws_rows[i] != base_rows[i]
                            {
                                out[i] = Arc::clone(&ws_rows[i]);
                            }
                        }
                        if ws_rows.len() > base_rows.len() {
                            out.extend(ws_rows[base_rows.len()..].iter().cloned());
                        }
                        Some(Arc::new(out))
                    }
                    (opt, _, _) => opt,
                };
                (tid, merged)
            })
            .collect();
        if self.is_unshared() {
            let map = Arc::make_mut(&mut self.base);
            for (tid, opt_rows) in merged_changes {
                match opt_rows {
                    Some(rows) => {
                        map.insert(tid, rows);
                    }
                    None => {
                        map.remove(&tid);
                    }
                }
            }
        } else {
            for (tid, opt_rows) in merged_changes {
                match opt_rows {
                    Some(rows) => {
                        self.delta.insert(tid, Some(rows));
                    }
                    None => {
                        if self.base.contains_key(&tid) {
                            self.delta.insert(tid, None);
                        } else {
                            self.delta.remove(&tid);
                        }
                    }
                }
            }
        }
    }
}

impl CheckpointableStorage for InMemoryStorage {
    fn get_checkpoint_data(&self) -> StorageCheckpointData {
        StorageCheckpointData::InMemory(
            self.materialized_tables()
                .into_iter()
                .map(|(k, v)| (k, v.iter().map(|r| r.as_ref().clone()).collect()))
                .collect(),
        )
    }

    fn restore_from_checkpoint(&mut self, data: StorageCheckpointData) -> Result<(), DbError> {
        if let StorageCheckpointData::InMemory(tables) = data {
            self.base = Arc::new(
                tables
                    .into_iter()
                    .map(|(k, v)| (k, Arc::new(v.into_iter().map(Arc::new).collect())))
                    .collect(),
            );
            self.delta.clear();
            Ok(())
        } else {
            Err(DbError::Storage(
                "invalid checkpoint data for InMemoryStorage".into(),
            ))
        }
    }

    fn clone_checkpointable(&self) -> Box<dyn CheckpointableStorage> {
        Box::new(self.clone())
    }
}
