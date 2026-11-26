use chrono::{NaiveDate, NaiveDateTime};

use crate::ast::{SystemTimeClause, TableRef};
use crate::catalog::{Catalog, TableDef, TemporalTableConfig};
use crate::error::DbError;
use crate::executor::clock::Clock;
use crate::executor::context::ExecutionContext;
use crate::executor::evaluator::eval_expr;
use crate::executor::model::BoundTable;
use crate::executor::value_ops::compare_values;
use crate::storage::{Storage, StoredRow};
use crate::types::{DataType, Value};

pub(crate) fn max_temporal_datetime() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(9999, 12, 31)
        .unwrap()
        .and_hms_nano_opt(23, 59, 59, 999_999_900)
        .unwrap()
}

pub(crate) fn ensure_history_table(
    catalog: &mut dyn Catalog,
    storage: &mut dyn Storage,
    base_table: &TableDef,
    history_schema: &str,
    history_name: &str,
) -> Result<u32, DbError> {
    if let Some(existing) = catalog.find_table(history_schema, history_name) {
        return Ok(existing.id);
    }
    let schema_id = catalog
        .get_schema_id(history_schema)
        .ok_or_else(|| DbError::schema_not_found(history_schema))?;
    let history_id = catalog.alloc_table_id();
    let mut cols = Vec::with_capacity(base_table.columns.len());
    for c in &base_table.columns {
        let mut hc = c.clone();
        hc.id = catalog.alloc_column_id();
        hc.primary_key = false;
        hc.unique = false;
        hc.identity = None;
        hc.default = None;
        hc.default_constraint_name = None;
        hc.check = None;
        hc.check_constraint_name = None;
        hc.is_clustered = false;
        cols.push(hc);
    }
    let history_def = TableDef {
        id: history_id,
        schema_id,
        schema_name: history_schema.to_string(),
        name: history_name.to_string(),
        columns: cols,
        check_constraints: vec![],
        foreign_keys: vec![],
        unique_constraints: vec![],
    };
    catalog.register_table(history_def);
    storage.ensure_table(history_id)?;
    Ok(history_id)
}

fn find_col_idx(table: &TableDef, name: &str) -> Option<usize> {
    table
        .columns
        .iter()
        .position(|c| c.name.eq_ignore_ascii_case(name))
}

pub(crate) fn fill_temporal_insert_values(
    table: &TableDef,
    values: &mut [Value],
    catalog: &dyn Catalog,
    clock: &dyn Clock,
) {
    let Some(cfg) = catalog.get_temporal_config(table.id) else {
        return;
    };
    if !cfg.system_versioning {
        return;
    }
    let now = clock.now_datetime_literal();
    if let Some(si) = find_col_idx(table, &cfg.start_col) {
        if si < values.len() && values[si].is_null() {
            values[si] = Value::DateTime2(now);
        }
    }
    if let Some(ei) = find_col_idx(table, &cfg.end_col) {
        if ei < values.len() && values[ei].is_null() {
            values[ei] = Value::DateTime2(max_temporal_datetime());
        }
    }
}

pub(crate) fn archive_old_row_on_update(
    table: &TableDef,
    old_row: &StoredRow,
    new_row: &mut StoredRow,
    catalog: &dyn Catalog,
    storage: &mut dyn Storage,
    clock: &dyn Clock,
) -> Result<(), DbError> {
    let Some(cfg) = catalog.get_temporal_config(table.id).cloned() else {
        return Ok(());
    };
    if !cfg.system_versioning {
        return Ok(());
    }
    let now = clock.now_datetime_literal();
    archive_row_to_history(table, &cfg, old_row, now, catalog, storage)?;

    if let Some(si) = find_col_idx(table, &cfg.start_col) {
        if si < new_row.values.len() {
            new_row.values[si] = Value::DateTime2(now);
        }
    }
    if let Some(ei) = find_col_idx(table, &cfg.end_col) {
        if ei < new_row.values.len() {
            new_row.values[ei] = Value::DateTime2(max_temporal_datetime());
        }
    }
    Ok(())
}

pub(crate) fn archive_old_row_on_delete(
    table: &TableDef,
    deleted_row: &StoredRow,
    catalog: &dyn Catalog,
    storage: &mut dyn Storage,
    clock: &dyn Clock,
) -> Result<(), DbError> {
    let Some(cfg) = catalog.get_temporal_config(table.id).cloned() else {
        return Ok(());
    };
    if !cfg.system_versioning {
        return Ok(());
    }
    let now = clock.now_datetime_literal();
    archive_row_to_history(table, &cfg, deleted_row, now, catalog, storage)
}

fn archive_row_to_history(
    table: &TableDef,
    cfg: &TemporalTableConfig,
    old_row: &StoredRow,
    now: NaiveDateTime,
    catalog: &dyn Catalog,
    storage: &mut dyn Storage,
) -> Result<(), DbError> {
    let Some(hist_table) = catalog.find_table(&cfg.history_schema, &cfg.history_table) else {
        return Ok(());
    };
    let mut archived = old_row.clone();
    archived.deleted = false;
    if let Some(ei) = find_col_idx(table, &cfg.end_col) {
        if ei < archived.values.len() {
            archived.values[ei] = Value::DateTime2(now);
        }
    }
    storage.insert_row(hist_table.id, archived)?;
    Ok(())
}

fn to_datetime_value(
    expr: &crate::ast::Expr,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let raw = eval_expr(expr, &[], ctx, catalog, storage, clock)?;
    if raw.is_null() {
        return Ok(Value::Null);
    }
    crate::executor::value_ops::coerce_value_to_type_with_dateformat(
        raw,
        &DataType::DateTime2,
        &ctx.options.dateformat,
    )
}

pub(crate) fn bind_temporal_table(
    tref: &TableRef,
    clause: &SystemTimeClause,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
    ctx: &mut ExecutionContext,
) -> Result<BoundTable, DbError> {
    let obj = tref
        .name_as_object()
        .ok_or_else(|| DbError::Execution("FOR SYSTEM_TIME requires a named table".into()))?;
    let schema = obj.schema_or_dbo();
    let table = catalog
        .find_table_arc(schema, &obj.name)
        .ok_or_else(|| DbError::table_not_found(schema, &obj.name))?;
    crate::executor::security::check_object_permission(
        catalog,
        ctx,
        "SELECT",
        table.schema_or_dbo(),
        &table.name,
        table.id as i32,
    )?;

    let cfg = catalog.get_temporal_config(table.id).ok_or_else(|| {
        DbError::Execution(format!(
            "Temporal FOR SYSTEM_TIME query requires table '{}.{}' to be a system-versioned temporal table.",
            table.schema_or_dbo(),
            table.name
        ))
    })?;
    let start_idx = find_col_idx(&table, &cfg.start_col)
        .ok_or_else(|| DbError::column_not_found(&cfg.start_col))?;
    let end_idx = find_col_idx(&table, &cfg.end_col)
        .ok_or_else(|| DbError::column_not_found(&cfg.end_col))?;

    let mut combined: Vec<StoredRow> = Vec::new();
    for r in storage.scan_rows(table.id)? {
        let r = r?;
        if !r.deleted {
            combined.push(r);
        }
    }
    if let Some(hist) = catalog.find_table(&cfg.history_schema, &cfg.history_table) {
        for r in storage.scan_rows(hist.id)? {
            let r = r?;
            if !r.deleted {
                combined.push(r);
            }
        }
    }

    let filtered = match clause {
        SystemTimeClause::All => combined,
        SystemTimeClause::AsOf(expr) => {
            let point = to_datetime_value(expr, ctx, catalog, storage, clock)?;
            if point.is_null() {
                Vec::new()
            } else {
                combined
                    .into_iter()
                    .filter(|r| {
                        let s = &r.values[start_idx];
                        let e = &r.values[end_idx];
                        !s.is_null()
                            && !e.is_null()
                            && compare_values(s, &point) != std::cmp::Ordering::Greater
                            && compare_values(e, &point) == std::cmp::Ordering::Greater
                    })
                    .collect()
            }
        }
        SystemTimeClause::FromTo(s_expr, e_expr) => {
            let start_bound = to_datetime_value(s_expr, ctx, catalog, storage, clock)?;
            let end_bound = to_datetime_value(e_expr, ctx, catalog, storage, clock)?;
            if start_bound.is_null() || end_bound.is_null() {
                Vec::new()
            } else {
                combined
                    .into_iter()
                    .filter(|r| {
                        let s = &r.values[start_idx];
                        let e = &r.values[end_idx];
                        !s.is_null()
                            && !e.is_null()
                            && compare_values(s, &end_bound) == std::cmp::Ordering::Less
                            && compare_values(e, &start_bound) == std::cmp::Ordering::Greater
                    })
                    .collect()
            }
        }
        SystemTimeClause::Between(s_expr, e_expr) => {
            let start_bound = to_datetime_value(s_expr, ctx, catalog, storage, clock)?;
            let end_bound = to_datetime_value(e_expr, ctx, catalog, storage, clock)?;
            if start_bound.is_null() || end_bound.is_null() {
                Vec::new()
            } else {
                combined
                    .into_iter()
                    .filter(|r| {
                        let s = &r.values[start_idx];
                        let e = &r.values[end_idx];
                        !s.is_null()
                            && !e.is_null()
                            && compare_values(s, &end_bound) != std::cmp::Ordering::Greater
                            && compare_values(e, &start_bound) == std::cmp::Ordering::Greater
                    })
                    .collect()
            }
        }
        SystemTimeClause::ContainedIn(s_expr, e_expr) => {
            let start_bound = to_datetime_value(s_expr, ctx, catalog, storage, clock)?;
            let end_bound = to_datetime_value(e_expr, ctx, catalog, storage, clock)?;
            if start_bound.is_null() || end_bound.is_null() {
                Vec::new()
            } else {
                combined
                    .into_iter()
                    .filter(|r| {
                        let s = &r.values[start_idx];
                        let e = &r.values[end_idx];
                        !s.is_null()
                            && !e.is_null()
                            && compare_values(s, &start_bound) != std::cmp::Ordering::Less
                            && compare_values(e, &end_bound) != std::cmp::Ordering::Greater
                    })
                    .collect()
            }
        }
    };

    Ok(BoundTable {
        alias: std::sync::Arc::from(tref.alias.as_deref().unwrap_or(table.name.as_str())),
        table,
        virtual_rows: Some(filtered),
    })
}
