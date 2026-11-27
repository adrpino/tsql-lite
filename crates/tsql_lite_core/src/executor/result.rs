#[cfg(feature = "checkpoint")]
use serde::{Deserialize, Serialize};

use crate::types::Value;

use crate::types::DataType;

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub column_types: Vec<DataType>,
    pub column_nullabilities: Vec<bool>,
    pub rows: Vec<Vec<Value>>,
    pub return_status: Option<i32>,
    pub is_procedure: bool,
}

/// Per-statement execution outcome for multi-statement TDS batches, carrying
/// both the optional result set and wire-level rowcount metadata (`SET NOCOUNT`).
#[derive(Debug, Clone, Default)]
pub struct BatchResultItem {
    pub result: Option<QueryResult>,
    /// Affected row count for DML statements (`INSERT`/`UPDATE`/`DELETE`/`MERGE`)
    /// when `SET NOCOUNT` is `OFF`.
    pub rows_affected: Option<u64>,
    /// True when `SET NOCOUNT ON` was active for this statement (suppresses `DONE_COUNT`).
    pub suppress_rowcount: bool,
}

#[derive(Debug)]
#[cfg_attr(feature = "checkpoint", derive(Serialize))]
pub struct JsonQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<serde_json::Value>,
    pub row_count: usize,
}

impl QueryResult {
    pub fn to_json_result(&self) -> JsonQueryResult {
        let rows = self
            .rows
            .iter()
            .map(|r| {
                let values = r
                    .iter()
                    .map(|v| serde_json::Value::from(v.to_json()))
                    .collect::<Vec<_>>();
                serde_json::Value::Array(values)
            })
            .collect::<Vec<_>>();

        JsonQueryResult {
            columns: self.columns.clone(),
            row_count: rows.len(),
            rows,
        }
    }
}
