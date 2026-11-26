use crate::ast::InsertSource;
use crate::catalog::TableDef;
use crate::error::{DbError, StmtOutcome};
use crate::executor::query::plan::RelationalQuery;
use crate::executor::result::QueryResult;
use crate::storage::StoredRow;

use super::super::context::ExecutionContext;
use super::super::query::QueryExecutor;
use super::super::script::ScriptExecutor;
use super::MutationExecutor;

impl<'a> MutationExecutor<'a> {
    pub(crate) fn collect_insert_rows(
        &mut self,
        table: &TableDef,
        insert_columns: &[String],
        source: &InsertSource,
        ctx: &mut ExecutionContext<'_>,
        rowcount_limit: Option<usize>,
    ) -> Result<Vec<StoredRow>, DbError> {
        match source {
            InsertSource::DefaultValues => {
                Ok(vec![self.build_insert_row(table, &[], vec![], ctx)?])
            }
            InsertSource::Values(values) => self.collect_insert_rows_from_values(
                table,
                insert_columns,
                values,
                ctx,
                rowcount_limit,
            ),
            InsertSource::Select(select_stmt) => {
                let query_result = QueryExecutor {
                    catalog: self.catalog as &dyn crate::catalog::Catalog,
                    storage: self.storage,
                    clock: self.clock,
                }
                .execute_select(RelationalQuery::from_ref(select_stmt), ctx)?;
                self.collect_insert_rows_from_query_result(
                    table,
                    insert_columns,
                    query_result,
                    ctx,
                    rowcount_limit,
                )
            }
            InsertSource::Exec(exec_stmt) => {
                let saved_sink = ctx.frame.result_set_sink.take();
                let outcome = ScriptExecutor {
                    catalog: self.catalog,
                    storage: self.storage,
                    clock: self.clock,
                }
                .execute(*exec_stmt.clone(), ctx);
                ctx.frame.result_set_sink = saved_sink;
                let outcome = outcome?;
                let query_result = match outcome {
                    StmtOutcome::Ok(Some(r)) => r,
                    StmtOutcome::Ok(None) => {
                        return Err(DbError::Execution(
                            "INSERT EXEC source returned no result".into(),
                        ))
                    }
                    other => {
                        other.into_result()?;
                        unreachable!()
                    }
                };
                self.collect_insert_rows_from_query_result(
                    table,
                    insert_columns,
                    query_result,
                    ctx,
                    rowcount_limit,
                )
            }
        }
    }

    pub(crate) fn collect_insert_rows_from_values(
        &mut self,
        table: &TableDef,
        insert_columns: &[String],
        values: &[Vec<crate::ast::Expr>],
        ctx: &mut ExecutionContext<'_>,
        rowcount_limit: Option<usize>,
    ) -> Result<Vec<StoredRow>, DbError> {
        let mut rows = Vec::new();
        for value_row in values.iter().cloned() {
            if let Some(limit) = rowcount_limit {
                if rows.len() >= limit {
                    break;
                }
            }
            rows.push(self.build_insert_row(table, insert_columns, value_row, ctx)?);
        }
        Ok(rows)
    }

    pub(crate) fn collect_insert_rows_from_query_result(
        &mut self,
        table: &TableDef,
        insert_columns: &[String],
        query_result: QueryResult,
        ctx: &mut ExecutionContext<'_>,
        rowcount_limit: Option<usize>,
    ) -> Result<Vec<StoredRow>, DbError> {
        if query_result.columns.is_empty() && query_result.rows.is_empty() {
            return Ok(Vec::new());
        }
        if insert_columns.len() != query_result.columns.len() {
            return Err(DbError::Execution(format!(
                "insert column count ({}) does not match source column count ({})",
                insert_columns.len(),
                query_result.columns.len()
            )));
        }

        let col_indices: Vec<usize> = insert_columns
            .iter()
            .map(|input_col| {
                table
                    .columns
                    .iter()
                    .position(|c| c.name.eq_ignore_ascii_case(input_col))
                    .ok_or_else(|| DbError::column_not_found(input_col))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let cap = match rowcount_limit {
            Some(limit) => query_result.rows.len().min(limit),
            None => query_result.rows.len(),
        };
        let mut rows = Vec::with_capacity(cap);
        for row_values in query_result.rows {
            if let Some(limit) = rowcount_limit {
                if rows.len() >= limit {
                    break;
                }
            }
            rows.push(self.build_row_from_values_indexed(table, &col_indices, row_values, ctx)?);
        }
        Ok(rows)
    }

    pub(crate) fn commit_insert_rows(
        &mut self,
        table: &TableDef,
        table_id: u32,
        rows: Vec<StoredRow>,
        ctx: &mut ExecutionContext<'_>,
        collect_rows: bool,
    ) -> Result<Vec<StoredRow>, DbError> {
        let mut inserted_rows_for_output = if collect_rows {
            Vec::with_capacity(rows.len())
        } else {
            Vec::new()
        };

        crate::executor::security::check_object_permission(
            self.catalog,
            ctx,
            "INSERT",
            table.schema_or_dbo(),
            &table.name,
            table_id as i32,
        )?;
        let has_unique_index = self
            .catalog
            .get_indexes()
            .iter()
            .any(|i| i.table_id == table_id && i.is_unique);
        let has_unique = table.columns.iter().any(|c| c.unique)
            || table.columns.iter().filter(|c| c.primary_key).count() >= 2
            || has_unique_index;
        let has_fks = table.foreign_keys.iter().any(|fk| fk.enabled);
        let has_checks =
            !table.check_constraints.is_empty() || table.columns.iter().any(|c| c.check.is_some());
        let has_dirty = ctx.session.dirty_buffer.is_some();
        let table_indexes: Vec<u32> = self
            .catalog
            .get_indexes()
            .iter()
            .filter(|i| i.table_id == table_id)
            .map(|i| i.id)
            .collect();
        let mut next_row_index =
            if !table_indexes.is_empty() && self.storage.as_index_storage_mut().is_some() {
                self.storage
                    .get_rows(table_id)
                    .map(|r| r.len())
                    .unwrap_or(0)
            } else {
                0
            };

        for row in rows {
            if has_unique {
                crate::executor::mutation::validation::enforce_unique_on_insert(
                    table,
                    self.storage,
                    table_id,
                    &row,
                )?;
                if has_unique_index {
                    crate::executor::mutation::validation::enforce_unique_indexes_on_write(
                        table,
                        self.catalog,
                        self.storage,
                        table_id,
                        &row,
                        None,
                        ctx,
                        self.clock,
                    )?;
                }
            }
            if has_fks {
                crate::executor::mutation::validation::enforce_foreign_keys_on_insert(
                    table,
                    self.catalog,
                    self.storage,
                    &row,
                )?;
            }
            if has_checks {
                crate::executor::mutation::validation::enforce_checks_on_row(
                    table,
                    &row,
                    ctx,
                    self.catalog,
                    self.storage,
                    self.clock,
                )?;
            }

            if !collect_rows && table_indexes.is_empty() && !has_dirty {
                self.storage.insert_row(table_id, row)?;
                continue;
            }

            self.storage.insert_row(table_id, row.clone())?;
            let row_index = next_row_index;
            next_row_index += 1;

            if !table_indexes.is_empty() {
                if let Some(index_storage) = self.storage.as_index_storage_mut() {
                    for &idx_id in &table_indexes {
                        if let Some(bi) = index_storage.get_index_mut(table_id, idx_id) {
                            let _ = bi.insert(row_index, &row.values);
                        }
                    }
                }
            }

            if has_dirty {
                let dirty_key = format!("{}\x02{}", table.schema_name, table.name);
            self.push_dirty_insert(ctx, &dirty_key, &row);
            }
            if collect_rows {
                inserted_rows_for_output.push(row);
            }
        }

        Ok(inserted_rows_for_output)
    }
}
