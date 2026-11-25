use crate::error::DbError;
use crate::storage::btree_index::IndexKey;
use crate::storage::{BTreeIndex, IndexStorage};
use crate::types::Value;

use super::redb_storage::RedbStorage;

impl IndexStorage for RedbStorage {
    fn register_index(
        &mut self,
        table_id: u32,
        index_id: u32,
        column_ids: Vec<u32>,
        is_unique: bool,
        is_clustered: bool,
    ) {
        self.indexes.insert(
            (table_id, index_id),
            BTreeIndex::new(column_ids, is_unique, is_clustered),
        );
    }

    fn get_index(&self, table_id: u32, index_id: u32) -> Option<&BTreeIndex> {
        self.indexes.get(&(table_id, index_id))
    }

    fn get_index_mut(&mut self, table_id: u32, index_id: u32) -> Option<&mut BTreeIndex> {
        self.indexes.get_mut(&(table_id, index_id))
    }

    fn seek_index(&self, table_id: u32, index_id: u32, key: &Value) -> Result<Vec<usize>, DbError> {
        let index = self
            .indexes
            .get(&(table_id, index_id))
            .ok_or_else(|| DbError::Storage(format!("index {} not found", index_id)))?;

        let result = index.seek(key).cloned().unwrap_or_default();
        Ok(result)
    }

    fn seek_index_range(
        &self,
        table_id: u32,
        index_id: u32,
        lower: Option<&Value>,
        upper: Option<&Value>,
    ) -> Result<Vec<(Value, Vec<usize>)>, DbError> {
        let index = self
            .indexes
            .get(&(table_id, index_id))
            .ok_or_else(|| DbError::Storage(format!("index {} not found", index_id)))?;

        let entries = index.seek_range(lower, upper);
        let mut result = Vec::new();
        for (key, indices) in entries {
            if let Some(first_val) = key.as_values().first() {
                result.push((first_val.clone(), indices));
            }
        }
        Ok(result)
    }

    fn insert_index_entry(
        &mut self,
        table_id: u32,
        index_id: u32,
        key: Value,
        row_index: usize,
    ) -> Result<(), DbError> {
        let index = self
            .indexes
            .get_mut(&(table_id, index_id))
            .ok_or_else(|| DbError::Storage(format!("index {} not found", index_id)))?;

        index.insert_key(row_index, IndexKey::new(vec![key]))
    }

    fn delete_index_entry(
        &mut self,
        table_id: u32,
        index_id: u32,
        key: &Value,
        row_index: usize,
    ) -> Result<(), DbError> {
        let index = self
            .indexes
            .get_mut(&(table_id, index_id))
            .ok_or_else(|| DbError::Storage(format!("index {} not found", index_id)))?;

        index.delete_key(row_index, &IndexKey::new(vec![key.clone()]))
    }

    fn rebuild_index(
        &mut self,
        table_id: u32,
        index_id: u32,
        entries: Vec<(Value, usize)>,
    ) -> Result<(), DbError> {
        let index = self
            .indexes
            .get_mut(&(table_id, index_id))
            .ok_or_else(|| DbError::Storage(format!("index {} not found", index_id)))?;

        index.clear();
        for (key, row_index) in entries {
            index.insert_key(row_index, IndexKey::new(vec![key]))?;
        }
        Ok(())
    }
}
