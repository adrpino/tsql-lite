use crate::catalog::TableDef;
use crate::storage::StoredRow;
use crate::types::Value;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Group {
    pub key: Vec<Value>,
    pub rows: Vec<JoinedRow>,
}

#[derive(Debug, Clone)]
pub(crate) struct BoundTable {
    pub(crate) table: Arc<TableDef>,
    pub(crate) alias: Arc<str>,
    pub(crate) virtual_rows: Option<Vec<StoredRow>>,
}

#[derive(Debug, Clone)]
pub struct ContextTable {
    pub table: Arc<TableDef>,
    pub alias: Arc<str>,
    pub row: Option<Arc<StoredRow>>,
    pub storage_index: Option<usize>,
    pub source_aliases: Vec<String>,
}

impl ContextTable {
    pub fn null_row(&self) -> Self {
        Self {
            table: Arc::clone(&self.table),
            alias: Arc::clone(&self.alias),
            row: None,
            storage_index: None,
            source_aliases: self.source_aliases.clone(),
        }
    }
}

pub type JoinedRow = Vec<ContextTable>;

pub(crate) fn single_row_context_arc(
    table: Arc<TableDef>,
    alias: Arc<str>,
    row: StoredRow,
) -> JoinedRow {
    vec![ContextTable {
        table,
        alias,
        row: Some(Arc::new(row)),
        storage_index: None,
        source_aliases: Vec::new(),
    }]
}

pub(crate) fn single_row_context(table: &TableDef, row: StoredRow) -> JoinedRow {
    let alias: Arc<str> = Arc::from(table.name.as_str());
    single_row_context_arc(Arc::new(table.clone()), alias, row)
}

#[derive(Debug, Clone)]
pub struct Cursor {
    pub query: Option<crate::ast::SelectStmt>,
    pub query_result: super::result::QueryResult,
    pub current_row: i64,
}
