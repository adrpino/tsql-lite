pub mod btree_index;
mod redb_checkpoint;
mod redb_index_adapter;
mod redb_row_storage;
pub mod redb_storage;
pub use btree_index::{BTreeIndex, IndexStorage};
pub use redb_storage::RedbStorage;

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::DbError;
use crate::types::Value;
use serde::{Deserialize, Serialize};

pub type StorageRowStream<'a> = Box<dyn Iterator<Item = Result<StoredRow, DbError>> + 'a>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRow {
    pub values: Vec<Value>,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageCheckpointData {
    InMemory(HashMap<u32, Vec<StoredRow>>),
    Persistent,
}

pub trait Storage: std::fmt::Debug + Send + Sync {
    fn scan_rows<'a>(&'a self, table_id: u32) -> Result<StorageRowStream<'a>, DbError>;

    fn get_row(&self, table_id: u32, index: usize) -> Result<Option<StoredRow>, DbError>;

    fn get_rows(&self, table_id: u32) -> Result<Vec<StoredRow>, DbError> {
        self.scan_rows(table_id)?.collect()
    }

    fn insert_row(&mut self, table_id: u32, row: StoredRow) -> Result<(), DbError>;
    fn update_row(&mut self, table_id: u32, index: usize, row: StoredRow) -> Result<(), DbError>;
    fn delete_row(&mut self, table_id: u32, index: usize) -> Result<(), DbError>;
    fn replace_table(&mut self, table_id: u32, rows: Vec<StoredRow>) -> Result<(), DbError>;
    fn clear_table(&mut self, table_id: u32) -> Result<(), DbError>;
    fn remove_table(&mut self, table_id: u32) -> Result<(), DbError>;
    fn ensure_table(&mut self, table_id: u32) -> Result<(), DbError>;

    fn clone_boxed(&self) -> Box<dyn Storage>;

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

mod cow_tables_serde {
    use super::StoredRow;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::HashMap;
    use std::sync::Arc;

    pub fn serialize<S>(
        val: &Arc<HashMap<u32, Arc<Vec<StoredRow>>>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let plain: HashMap<u32, &[StoredRow]> =
            val.iter().map(|(&k, v)| (k, v.as_slice())).collect();
        plain.serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<Arc<HashMap<u32, Arc<Vec<StoredRow>>>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let plain: HashMap<u32, Vec<StoredRow>> = HashMap::deserialize(deserializer)?;
        Ok(Arc::new(
            plain.into_iter().map(|(k, v)| (k, Arc::new(v))).collect(),
        ))
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct InMemoryStorage {
    #[serde(with = "cow_tables_serde")]
    pub tables: Arc<HashMap<u32, Arc<Vec<StoredRow>>>>,
}

impl Storage for InMemoryStorage {
    fn scan_rows<'a>(&'a self, table_id: u32) -> Result<StorageRowStream<'a>, DbError> {
        let rows = self
            .tables
            .get(&table_id)
            .ok_or_else(|| DbError::Storage(format!("table {} not found in storage", table_id)))?;

        Ok(Box::new(rows.iter().cloned().map(Ok)))
    }

    fn get_row(&self, table_id: u32, index: usize) -> Result<Option<StoredRow>, DbError> {
        let rows = self
            .tables
            .get(&table_id)
            .ok_or_else(|| DbError::Storage(format!("table {} not found", table_id)))?;
        Ok(rows.get(index).cloned())
    }

    fn insert_row(&mut self, table_id: u32, row: StoredRow) -> Result<(), DbError> {
        let rows = Arc::make_mut(&mut self.tables)
            .get_mut(&table_id)
            .ok_or_else(|| DbError::Storage(format!("table {} not found in storage", table_id)))?;
        Arc::make_mut(rows).push(row);
        Ok(())
    }

    fn update_row(&mut self, table_id: u32, index: usize, row: StoredRow) -> Result<(), DbError> {
        let rows = Arc::make_mut(&mut self.tables)
            .get_mut(&table_id)
            .ok_or_else(|| DbError::Storage(format!("table {} not found", table_id)))?;
        if index >= rows.len() {
            return Err(DbError::Storage(format!(
                "index {} out of bounds for table {}",
                index, table_id
            )));
        }
        Arc::make_mut(rows)[index] = row;
        Ok(())
    }

    fn delete_row(&mut self, table_id: u32, index: usize) -> Result<(), DbError> {
        let rows = Arc::make_mut(&mut self.tables)
            .get_mut(&table_id)
            .ok_or_else(|| DbError::Storage(format!("table {} not found", table_id)))?;
        if index >= rows.len() {
            return Err(DbError::Storage(format!(
                "index {} out of bounds for table {}",
                index, table_id
            )));
        }
        Arc::make_mut(rows)[index].deleted = true;
        Ok(())
    }

    fn replace_table(&mut self, table_id: u32, rows: Vec<StoredRow>) -> Result<(), DbError> {
        if let std::collections::hash_map::Entry::Occupied(mut e) =
            Arc::make_mut(&mut self.tables).entry(table_id)
        {
            e.insert(Arc::new(rows));
            Ok(())
        } else {
            Err(DbError::Storage(format!(
                "table {} not found in storage",
                table_id
            )))
        }
    }

    fn clear_table(&mut self, table_id: u32) -> Result<(), DbError> {
        if let Some(rows) = Arc::make_mut(&mut self.tables).get_mut(&table_id) {
            if !rows.is_empty() {
                Arc::make_mut(rows).clear();
            }
            Ok(())
        } else {
            Err(DbError::Storage(format!("table {} not found", table_id)))
        }
    }

    fn ensure_table(&mut self, table_id: u32) -> Result<(), DbError> {
        if !self.tables.contains_key(&table_id) {
            Arc::make_mut(&mut self.tables)
                .entry(table_id)
                .or_insert_with(|| Arc::new(Vec::new()));
        }
        Ok(())
    }

    fn remove_table(&mut self, table_id: u32) -> Result<(), DbError> {
        if self.tables.contains_key(&table_id) {
            Arc::make_mut(&mut self.tables).remove(&table_id);
        }
        Ok(())
    }

    fn clone_boxed(&self) -> Box<dyn Storage> {
        Box::new(self.clone())
    }

    fn as_index_storage(&self) -> Option<&dyn IndexStorage> {
        None
    }
    fn as_index_storage_mut(&mut self) -> Option<&mut dyn IndexStorage> {
        None
    }

    fn merge_workspace(&mut self, workspace: Self, base: &Self) {
        if Arc::ptr_eq(&workspace.tables, &base.tables) {
            return;
        }
        let target = Arc::make_mut(&mut self.tables);
        for (&table_id, ws_rows) in workspace.tables.iter() {
            let unchanged = base
                .tables
                .get(&table_id)
                .is_some_and(|base_rows| Arc::ptr_eq(ws_rows, base_rows));
            if !unchanged {
                target.insert(table_id, Arc::clone(ws_rows));
            }
        }
        for &table_id in base.tables.keys() {
            if !workspace.tables.contains_key(&table_id) {
                target.remove(&table_id);
            }
        }
    }
}

impl CheckpointableStorage for InMemoryStorage {
    fn get_checkpoint_data(&self) -> StorageCheckpointData {
        StorageCheckpointData::InMemory(
            self.tables
                .iter()
                .map(|(&k, v)| (k, v.as_ref().clone()))
                .collect(),
        )
    }

    fn restore_from_checkpoint(&mut self, data: StorageCheckpointData) -> Result<(), DbError> {
        if let StorageCheckpointData::InMemory(tables) = data {
            self.tables = Arc::new(tables.into_iter().map(|(k, v)| (k, Arc::new(v))).collect());
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
