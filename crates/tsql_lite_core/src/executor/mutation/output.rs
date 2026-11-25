use crate::ast::{OutputColumn, OutputSource};
use crate::catalog::{Catalog, ColumnDef, TableDef};
use crate::error::DbError;
use crate::executor::clock::Clock;
use crate::executor::context::ExecutionContext;
use crate::executor::evaluator::eval_expr;
use crate::executor::model::{ContextTable, JoinedRow};
use crate::storage::{Storage, StoredRow};
use crate::types::{DataType, Value};

use super::super::result::QueryResult;

fn build_output_columns(output: &[OutputColumn]) -> Vec<String> {
    output
        .iter()
        .map(|col| {
            col.alias.clone().unwrap_or_else(|| match col.source {
                OutputSource::Inserted => format!("INSERTED.{}", col.column),
                OutputSource::Deleted => format!("DELETED.{}", col.column),
                OutputSource::Expr => col.column.clone(),
            })
        })
        .collect()
}

fn expand_wildcards(output: &[OutputColumn], table: &TableDef) -> Vec<OutputColumn> {
    let mut expanded = Vec::new();
    for col in output {
        if col.is_wildcard {
            for tcol in &table.columns {
                if tcol.computed_expr.is_some() {
                    continue;
                }
                expanded.push(OutputColumn {
                    source: col.source,
                    column: tcol.name.clone(),
                    expr: None,
                    alias: None,
                    is_wildcard: false,
                });
            }
        } else {
            expanded.push(col.clone());
        }
    }
    expanded
}

fn build_output_eval_env(
    table: &TableDef,
    inserted: Option<&[Value]>,
    deleted: Option<&[Value]>,
    extra: &[ContextTable],
    action: Option<&'static str>,
) -> JoinedRow {
    let mut env: JoinedRow = extra.to_vec();
    if let Some(act) = action {
        let action_table = TableDef {
            id: 0,
            schema_id: 0,
            schema_name: String::new(),
            name: "__output_meta".to_string(),
            columns: vec![ColumnDef {
                id: 1,
                name: "$action".to_string(),
                data_type: DataType::VarChar { max_len: 10 },
                nullable: false,
                default: None,
                default_constraint_name: None,
                identity: None,
                primary_key: false,
                unique: false,
                check: None,
                check_constraint_name: None,
                computed_expr: None,
                collation: None,
                ansi_padding_on: true,
                is_rowversion: false,
                is_clustered: false,
            }],
            check_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        };
        env.push(ContextTable {
            table: std::sync::Arc::new(action_table),
            alias: std::sync::Arc::from("__output_meta"),
            row: Some(std::sync::Arc::new(StoredRow {
                values: vec![Value::VarChar(act.to_string())],
                deleted: false,
            })),
            storage_index: None,
            source_aliases: Vec::new(),
        });
    }
    let table_arc = std::sync::Arc::new(table.clone());
    env.push(ContextTable {
        table: std::sync::Arc::clone(&table_arc),
        alias: std::sync::Arc::from("INSERTED"),
        row: inserted.map(|vals| {
            std::sync::Arc::new(StoredRow {
                values: vals.to_vec(),
                deleted: false,
            })
        }),
        storage_index: None,
        source_aliases: Vec::new(),
    });
    env.push(ContextTable {
        table: table_arc,
        alias: std::sync::Arc::from("DELETED"),
        row: deleted.map(|vals| {
            std::sync::Arc::new(StoredRow {
                values: vals.to_vec(),
                deleted: false,
            })
        }),
        storage_index: None,
        source_aliases: Vec::new(),
    });
    env
}

fn extract_output_value(
    output_col: &OutputColumn,
    table: &TableDef,
    row: &StoredRow,
) -> Result<Value, DbError> {
    let col_idx = table
        .columns
        .iter()
        .position(|c| c.name.eq_ignore_ascii_case(&output_col.column))
        .ok_or_else(|| DbError::column_not_found(&output_col.column))?;
    Ok(row.values[col_idx].clone())
}

fn resolve_col_idx(col_name: &str, table: &TableDef) -> Result<usize, DbError> {
    table
        .columns
        .iter()
        .position(|c| c.name.eq_ignore_ascii_case(col_name))
        .ok_or_else(|| DbError::column_not_found(col_name))
}

#[allow(clippy::too_many_arguments)]
pub fn build_output_result(
    output: &[OutputColumn],
    table: &TableDef,
    inserted_rows: &[&StoredRow],
    deleted_rows: &[&StoredRow],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Option<QueryResult>, DbError> {
    if output.is_empty() {
        return Ok(None);
    }

    let expanded = expand_wildcards(output, table);
    let columns = build_output_columns(&expanded);
    let has_expr = expanded.iter().any(|c| c.source == OutputSource::Expr);
    let mut rows = Vec::new();

    if inserted_rows.is_empty() && deleted_rows.is_empty() {
        let n = columns.len();
        let column_nullabilities = vec![true; columns.len()];
        return Ok(Some(QueryResult {
            columns,
            column_types: vec![crate::types::DataType::VarChar { max_len: 4000 }; n],
            column_nullabilities,
            rows,
            ..Default::default()
        }));
    }

    if !inserted_rows.is_empty() && !deleted_rows.is_empty() {
        for (inserted, deleted) in inserted_rows.iter().zip(deleted_rows.iter()) {
            let env = if has_expr {
                build_output_eval_env(
                    table,
                    Some(&inserted.values),
                    Some(&deleted.values),
                    &[],
                    Some("UPDATE"),
                )
            } else {
                Vec::new()
            };
            let mut row = Vec::new();
            for col in &expanded {
                let val = match col.source {
                    OutputSource::Inserted => extract_output_value(col, table, inserted)?,
                    OutputSource::Deleted => extract_output_value(col, table, deleted)?,
                    OutputSource::Expr => match &col.expr {
                        Some(expr) => eval_expr(expr, &env, ctx, catalog, storage, clock)?,
                        None => Value::Null,
                    },
                };
                row.push(val);
            }
            rows.push(row);
        }
        let column_types = derive_column_types(&rows, columns.len());
        let column_nullabilities = vec![true; columns.len()];
        return Ok(Some(QueryResult {
            columns,
            column_types,
            column_nullabilities,
            rows,
            ..Default::default()
        }));
    }

    if !inserted_rows.is_empty() {
        for inserted in inserted_rows {
            let env = if has_expr {
                build_output_eval_env(table, Some(&inserted.values), None, &[], Some("INSERT"))
            } else {
                Vec::new()
            };
            let mut row = Vec::new();
            for col in &expanded {
                let val = match col.source {
                    OutputSource::Inserted => extract_output_value(col, table, inserted)?,
                    OutputSource::Deleted => Value::Null,
                    OutputSource::Expr => match &col.expr {
                        Some(expr) => eval_expr(expr, &env, ctx, catalog, storage, clock)?,
                        None => Value::Null,
                    },
                };
                row.push(val);
            }
            rows.push(row);
        }
        let column_types = derive_column_types(&rows, columns.len());
        let column_nullabilities = vec![true; columns.len()];
        return Ok(Some(QueryResult {
            columns,
            column_types,
            column_nullabilities,
            rows,
            ..Default::default()
        }));
    }

    for deleted in deleted_rows {
        let env = if has_expr {
            build_output_eval_env(table, None, Some(&deleted.values), &[], Some("DELETE"))
        } else {
            Vec::new()
        };
        let mut row = Vec::new();
        for col in &expanded {
            let val = match col.source {
                OutputSource::Inserted => Value::Null,
                OutputSource::Deleted => extract_output_value(col, table, deleted)?,
                OutputSource::Expr => match &col.expr {
                    Some(expr) => eval_expr(expr, &env, ctx, catalog, storage, clock)?,
                    None => Value::Null,
                },
            };
            row.push(val);
        }
        rows.push(row);
    }

    let column_types = derive_column_types(&rows, columns.len());
    let column_nullabilities = vec![true; columns.len()];
    Ok(Some(QueryResult {
        columns,
        column_types,
        column_nullabilities,
        rows,
        ..Default::default()
    }))
}

pub struct MergeOutputRow {
    pub inserted_values: Option<Vec<Value>>,
    pub deleted_values: Option<Vec<Value>>,
    pub source_row: JoinedRow,
    pub action: &'static str,
}

#[allow(clippy::too_many_arguments)]
pub fn build_output_result_merge(
    output: &[OutputColumn],
    table: &TableDef,
    merge_rows: &[MergeOutputRow],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Option<QueryResult>, DbError> {
    if output.is_empty() {
        return Ok(None);
    }

    let expanded = expand_wildcards(output, table);
    let columns = build_output_columns(&expanded);
    let has_expr = expanded.iter().any(|c| c.source == OutputSource::Expr);
    let mut rows = Vec::new();

    for merge_row in merge_rows {
        let env = if has_expr {
            build_output_eval_env(
                table,
                merge_row.inserted_values.as_deref(),
                merge_row.deleted_values.as_deref(),
                &merge_row.source_row,
                Some(merge_row.action),
            )
        } else {
            Vec::new()
        };
        let mut row = Vec::new();
        for col in &expanded {
            let val = match col.source {
                OutputSource::Inserted => match &merge_row.inserted_values {
                    Some(inserted) => inserted[resolve_col_idx(&col.column, table)?].clone(),
                    None => Value::Null,
                },
                OutputSource::Deleted => match &merge_row.deleted_values {
                    Some(deleted) => deleted[resolve_col_idx(&col.column, table)?].clone(),
                    None => Value::Null,
                },
                OutputSource::Expr => match &col.expr {
                    Some(expr) => eval_expr(expr, &env, ctx, catalog, storage, clock)?,
                    None => Value::Null,
                },
            };
            row.push(val);
        }
        rows.push(row);
    }

    let column_types = derive_column_types(&rows, columns.len());
    let column_nullabilities = vec![true; columns.len()];
    Ok(Some(QueryResult {
        columns,
        column_types,
        column_nullabilities,
        rows,
        ..Default::default()
    }))
}

fn derive_column_types(rows: &[Vec<Value>], num_cols: usize) -> Vec<crate::types::DataType> {
    let mut column_types = Vec::with_capacity(num_cols);
    if !rows.is_empty() {
        for val in &rows[0] {
            column_types.push(
                val.data_type()
                    .unwrap_or(crate::types::DataType::VarChar { max_len: 4000 }),
            );
        }
    } else {
        column_types = vec![crate::types::DataType::VarChar { max_len: 4000 }; num_cols];
    }
    column_types
}
