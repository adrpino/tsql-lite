use crate::ast::Expr;
use crate::catalog::{Catalog, IndexDef, TableDef};
use crate::error::DbError;
use crate::storage::{Storage, StoredRow};

use super::super::super::clock::Clock;
use super::super::super::context::ExecutionContext;
use super::super::super::model::single_row_context;
use super::super::super::value_ops::{compare_values, truthy};

fn composite_unique_positions(table: &TableDef) -> Vec<(Option<&str>, Vec<usize>)> {
    table
        .unique_constraints
        .iter()
        .filter_map(|uq| {
            let mut positions = Vec::with_capacity(uq.columns.len());
            for col_name in &uq.columns {
                let pos = table
                    .columns
                    .iter()
                    .position(|c| c.name.eq_ignore_ascii_case(col_name))?;
                positions.push(pos);
            }
            if positions.len() >= 2 {
                Some((uq.name.as_deref(), positions))
            } else {
                None
            }
        })
        .collect()
}

fn composite_unique_error(name: Option<&str>) -> DbError {
    match name {
        Some(n) => DbError::Execution(format!(
            "Violation of UNIQUE KEY constraint '{}'. Cannot insert duplicate key.",
            n
        )),
        None => DbError::Execution(
            "Violation of UNIQUE KEY constraint. Cannot insert duplicate key.".to_string(),
        ),
    }
}

pub(crate) fn enforce_unique_on_insert(
    table: &TableDef,
    storage: &mut dyn Storage,
    table_id: u32,
    new_row: &StoredRow,
) -> Result<(), DbError> {
    let composite_uniques = composite_unique_positions(table);
    if let Some(slice) = storage.get_table_slice(table_id) {
        for (col_idx, col) in table.columns.iter().enumerate().filter(|(_, c)| c.unique) {
            let new_val = &new_row.values[col_idx];
            if new_val.is_null() {
                continue;
            }
            for existing in slice {
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
        for (uq_name, uq_cols) in &composite_uniques {
            for existing in slice {
                if existing.deleted {
                    continue;
                }
                if key_tuples_equal(new_row, existing, uq_cols) {
                    return Err(composite_unique_error(*uq_name));
                }
            }
        }
        let pk_count = table.columns.iter().filter(|c| c.primary_key).count();
        if pk_count >= 2 {
            let pk_cols: Vec<usize> = table
                .columns
                .iter()
                .enumerate()
                .filter(|(_, c)| c.primary_key)
                .map(|(i, _)| i)
                .collect();
            for existing in slice {
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
                        "Violation of PRIMARY KEY constraint. Cannot insert duplicate key."
                            .to_string(),
                    ));
                }
            }
        }
        return Ok(());
    }

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
    for (uq_name, uq_cols) in &composite_uniques {
        for existing in storage.scan_rows(table_id)? {
            let existing = existing?;
            if existing.deleted {
                continue;
            }
            if key_tuples_equal(new_row, &existing, uq_cols) {
                return Err(composite_unique_error(*uq_name));
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
    let composite_uniques = composite_unique_positions(table);
    if let Some(slice) = storage.get_table_slice(table_id) {
        for (col_idx, col) in table.columns.iter().enumerate().filter(|(_, c)| c.unique) {
            let new_val = &updated_row.values[col_idx];
            if new_val.is_null() {
                continue;
            }
            for (i, existing) in slice.iter().enumerate() {
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
        for (uq_name, uq_cols) in &composite_uniques {
            for (i, existing) in slice.iter().enumerate() {
                if i == updated_idx || existing.deleted {
                    continue;
                }
                if key_tuples_equal(updated_row, existing, uq_cols) {
                    return Err(composite_unique_error(*uq_name));
                }
            }
        }
        let pk_count = table.columns.iter().filter(|c| c.primary_key).count();
        if pk_count >= 2 {
            let pk_cols: Vec<usize> = table
                .columns
                .iter()
                .enumerate()
                .filter(|(_, c)| c.primary_key)
                .map(|(i, _)| i)
                .collect();
            for (i, existing) in slice.iter().enumerate() {
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
                        "Violation of PRIMARY KEY constraint. Cannot insert duplicate key."
                            .to_string(),
                    ));
                }
            }
        }
        return Ok(());
    }

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
    for (uq_name, uq_cols) in &composite_uniques {
        for (i, existing) in storage.scan_rows(table_id)?.enumerate() {
            let existing = existing?;
            if i == updated_idx || existing.deleted {
                continue;
            }
            if key_tuples_equal(updated_row, &existing, uq_cols) {
                return Err(composite_unique_error(*uq_name));
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

fn row_matches_filter(
    table: &TableDef,
    row: &StoredRow,
    filter: &Option<Box<Expr>>,
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<bool, DbError> {
    let Some(pred) = filter else {
        return Ok(true);
    };
    let joined = single_row_context(table, row.clone());
    let val =
        super::super::super::evaluator::eval_expr(pred, &joined, ctx, catalog, storage, clock)?;
    Ok(!val.is_null() && truthy(&val))
}

fn index_key_positions(table: &TableDef, idx: &IndexDef) -> Option<Vec<usize>> {
    let mut positions = Vec::with_capacity(idx.column_ids.len());
    for &col_id in &idx.column_ids {
        let pos = table.columns.iter().position(|c| c.id == col_id)?;
        positions.push(pos);
    }
    Some(positions)
}

fn key_tuples_equal(a: &StoredRow, b: &StoredRow, positions: &[usize]) -> bool {
    positions.iter().all(|&pos| {
        let va = &a.values[pos];
        let vb = &b.values[pos];
        match (va.is_null(), vb.is_null()) {
            (true, true) => true,
            (true, false) | (false, true) => false,
            (false, false) => compare_values(va, vb) == std::cmp::Ordering::Equal,
        }
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn enforce_unique_indexes_on_write(
    table: &TableDef,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    table_id: u32,
    candidate_row: &StoredRow,
    skip_row_idx: Option<usize>,
    ctx: &mut ExecutionContext<'_>,
    clock: &dyn Clock,
) -> Result<(), DbError> {
    let unique_indexes: Vec<IndexDef> = catalog
        .get_indexes()
        .iter()
        .filter(|i| i.table_id == table_id && i.is_unique)
        .map(|i| i.as_ref().clone())
        .collect();
    if unique_indexes.is_empty() {
        return Ok(());
    }
    if let Some(slice) = storage.get_table_slice(table_id) {
        for idx in &unique_indexes {
            let Some(positions) = index_key_positions(table, idx) else {
                continue;
            };
            if !row_matches_filter(
                table,
                candidate_row,
                &idx.filter_predicate,
                ctx,
                catalog,
                storage,
                clock,
            )? {
                continue;
            }
            for (i, existing) in slice.iter().enumerate() {
                if existing.deleted || skip_row_idx == Some(i) {
                    continue;
                }
                if !key_tuples_equal(candidate_row, existing, &positions) {
                    continue;
                }
                if row_matches_filter(
                    table,
                    existing,
                    &idx.filter_predicate,
                    ctx,
                    catalog,
                    storage,
                    clock,
                )? {
                    return Err(DbError::Execution(format!(
                        "Cannot insert duplicate key row in object '{}.{}' with unique index '{}'.",
                        table.schema_or_dbo(),
                        table.name,
                        idx.name
                    )));
                }
            }
        }
        return Ok(());
    }
    let existing_rows: Vec<(usize, StoredRow)> = storage
        .scan_rows(table_id)?
        .enumerate()
        .filter_map(|(i, r)| match r {
            Ok(row) if !row.deleted && skip_row_idx != Some(i) => Some(Ok((i, row))),
            Ok(_) => None,
            Err(e) => Some(Err(e)),
        })
        .collect::<Result<Vec<_>, DbError>>()?;

    for idx in &unique_indexes {
        let Some(positions) = index_key_positions(table, idx) else {
            continue;
        };
        if !row_matches_filter(
            table,
            candidate_row,
            &idx.filter_predicate,
            ctx,
            catalog,
            storage,
            clock,
        )? {
            continue;
        }
        for (_, existing) in &existing_rows {
            if !key_tuples_equal(candidate_row, existing, &positions) {
                continue;
            }
            if row_matches_filter(
                table,
                existing,
                &idx.filter_predicate,
                ctx,
                catalog,
                storage,
                clock,
            )? {
                return Err(DbError::Execution(format!(
                    "Cannot insert duplicate key row in object '{}.{}' with unique index '{}'.",
                    table.schema_or_dbo(),
                    table.name,
                    idx.name
                )));
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_existing_rows_for_unique_index(
    table: &TableDef,
    index_name: &str,
    col_names: &[String],
    filter_predicate: &Option<Box<Expr>>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    ctx: &mut ExecutionContext<'_>,
    clock: &dyn Clock,
) -> Result<(), DbError> {
    let mut positions = Vec::with_capacity(col_names.len());
    for col in col_names {
        let pos = table
            .columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(col))
            .ok_or_else(|| DbError::column_not_found(col))?;
        positions.push(pos);
    }
    let rows: Vec<StoredRow> = storage
        .scan_rows(table.id)?
        .filter_map(|r| match r {
            Ok(row) if !row.deleted => Some(Ok(row)),
            Ok(_) => None,
            Err(e) => Some(Err(e)),
        })
        .collect::<Result<Vec<_>, DbError>>()?;

    let mut filtered_rows: Vec<&StoredRow> = Vec::new();
    for row in &rows {
        if row_matches_filter(table, row, filter_predicate, ctx, catalog, storage, clock)? {
            for prev in &filtered_rows {
                if key_tuples_equal(prev, row, &positions) {
                    return Err(DbError::Execution(format!(
                        "The CREATE UNIQUE INDEX statement terminated because a duplicate key was found for the object name '{}.{}' and the index name '{}'.",
                        table.schema_or_dbo(),
                        table.name,
                        index_name
                    )));
                }
            }
            filtered_rows.push(row);
        }
    }
    Ok(())
}
