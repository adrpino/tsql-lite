use crate::catalog::TableDef;
use crate::error::DbError;
use crate::storage::{Storage, StoredRow};

use super::super::super::value_ops::compare_values;

pub(crate) fn enforce_unique_on_insert(
    table: &TableDef,
    storage: &mut dyn Storage,
    table_id: u32,
    new_row: &StoredRow,
) -> Result<(), DbError> {
    for (col_idx, col) in table.columns.iter().enumerate().filter(|(_, c)| c.unique) {
        let new_val = &new_row.values[col_idx];
        if new_val.is_null() {
            continue;
        }
        for existing in storage.scan_rows(table_id)? {
            let existing = existing?;
            if existing.deleted {
                continue;
            }
            let existing_val = &existing.values[col_idx];
            if !existing_val.is_null()
                && compare_values(new_val, existing_val) == std::cmp::Ordering::Equal
            {
                return Err(DbError::Execution(format!(
                    "Violation of UNIQUE KEY constraint on column '{}'. Cannot insert duplicate key.",
                    col.name
                )));
            }
        }
    }
    let pk_cols: Vec<usize> = table
        .columns
        .iter()
        .enumerate()
        .filter(|(_, c)| c.primary_key)
        .map(|(i, _)| i)
        .collect();
    if pk_cols.len() >= 2 {
        for existing in storage.scan_rows(table_id)? {
            let existing = existing?;
            if existing.deleted {
                continue;
            }
            let all_match = pk_cols.iter().all(|&ci| {
                let nv = &new_row.values[ci];
                let ev = &existing.values[ci];
                !nv.is_null()
                    && !ev.is_null()
                    && compare_values(nv, ev) == std::cmp::Ordering::Equal
            });
            if all_match {
                return Err(DbError::Execution(
                    "Violation of PRIMARY KEY constraint. Cannot insert duplicate key.".to_string(),
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn enforce_unique_on_update(
    table: &TableDef,
    storage: &mut dyn Storage,
    table_id: u32,
    updated_row: &StoredRow,
    updated_idx: usize,
) -> Result<(), DbError> {
    for (col_idx, col) in table.columns.iter().enumerate().filter(|(_, c)| c.unique) {
        let new_val = &updated_row.values[col_idx];
        if new_val.is_null() {
            continue;
        }
        for (i, existing) in storage.scan_rows(table_id)?.enumerate() {
            let existing = existing?;
            if i == updated_idx || existing.deleted {
                continue;
            }
            let existing_val = &existing.values[col_idx];
            if !existing_val.is_null()
                && compare_values(new_val, existing_val) == std::cmp::Ordering::Equal
            {
                return Err(DbError::Execution(format!(
                    "Violation of UNIQUE KEY constraint on column '{}'. Cannot insert duplicate key.",
                    col.name
                )));
            }
        }
    }
    let pk_cols: Vec<usize> = table
        .columns
        .iter()
        .enumerate()
        .filter(|(_, c)| c.primary_key)
        .map(|(i, _)| i)
        .collect();
    if pk_cols.len() >= 2 {
        for (i, existing) in storage.scan_rows(table_id)?.enumerate() {
            let existing = existing?;
            if i == updated_idx || existing.deleted {
                continue;
            }
            let all_match = pk_cols.iter().all(|&ci| {
                let nv = &updated_row.values[ci];
                let ev = &existing.values[ci];
                !nv.is_null()
                    && !ev.is_null()
                    && compare_values(nv, ev) == std::cmp::Ordering::Equal
            });
            if all_match {
                return Err(DbError::Execution(
                    "Violation of PRIMARY KEY constraint. Cannot insert duplicate key.".to_string(),
                ));
            }
        }
    }
    Ok(())
}
