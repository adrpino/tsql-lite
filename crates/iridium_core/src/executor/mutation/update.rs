use std::collections::HashSet;

use crate::ast::UpdateStmt;
use crate::error::DbError;

use super::super::context::ExecutionContext;
use super::super::query::QueryExecutor;
use super::super::result::QueryResult;
use super::query_source::{
    attach_base_rows_from_cte, build_mutation_query, resolve_cte_for_mutation,
    resolve_table_for_mutation,
};

use super::output::build_output_result;
use super::shared::{rowcount_limit, visit_target_rows};
use super::validation::{
    apply_assignments, enforce_checks_on_row, enforce_foreign_keys_on_delete,
    enforce_foreign_keys_on_insert, enforce_foreign_keys_on_update, enforce_unique_on_update,
    validate_row_against_table,
};
use super::MutationExecutor;

impl<'a> MutationExecutor<'a> {
    pub(crate) fn execute_update_with_context(
        &mut self,
        mut stmt: UpdateStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        if let Some(mapped) = ctx.resolve_table_name(&stmt.table.name) {
            stmt.table.name = mapped;
            if stmt.table.schema.is_none() {
                stmt.table.schema = Some("dbo".to_string());
            }
        }
        let target_alias = stmt.table.name.clone();
        let resolved_direct =
            resolve_table_for_mutation(stmt.from.as_ref(), &stmt.table, |schema, name| {
                self.catalog.find_table(schema, name).cloned().or_else(|| {
                    ctx.resolve_table_name(name)
                        .and_then(|mapped| self.catalog.find_table("dbo", &mapped).cloned())
                })
            });

        let (table, joined_rows) = match resolved_direct {
            Ok((table, resolved_name)) => {
                let query = build_mutation_query(
                    stmt.from.as_ref(),
                    &stmt.table,
                    &table,
                    &resolved_name,
                    stmt.selection.clone(),
                    stmt.top.clone(),
                );
                let query_executor = QueryExecutor {
                    catalog: self.catalog,
                    storage: self.storage,
                    clock: self.clock,
                };
                let joined_rows = query_executor.execute_to_joined_rows(query, ctx)?;
                (table, joined_rows)
            }
            Err(orig_err) => {
                let Some((cte, cte_name)) =
                    resolve_cte_for_mutation(stmt.from.as_ref(), &stmt.table, &ctx.row.ctes)
                else {
                    return Err(orig_err);
                };
                let base_target = cte
                    .base_targets
                    .iter()
                    .find(|bt| {
                        stmt.assignments.iter().all(|a| {
                            a.column.starts_with('@')
                                || bt
                                    .table_def
                                    .columns
                                    .iter()
                                    .any(|c| c.name.eq_ignore_ascii_case(&a.column))
                        })
                    })
                    .or_else(|| cte.base_targets.first())
                    .cloned()
                    .ok_or_else(|| {
                        DbError::Execution(format!(
                            "CTE '{}' is not updatable: no underlying base table found",
                            cte_name
                        ))
                    })?;

                let query = build_mutation_query(
                    stmt.from.as_ref(),
                    &stmt.table,
                    &cte.table_def,
                    &cte_name,
                    stmt.selection.clone(),
                    stmt.top.clone(),
                );
                let query_executor = QueryExecutor {
                    catalog: self.catalog,
                    storage: self.storage,
                    clock: self.clock,
                };
                let cte_rows = query_executor.execute_to_joined_rows(query, ctx)?;
                let mapped = attach_base_rows_from_cte(
                    cte_rows,
                    &cte_name,
                    &target_alias,
                    &base_target,
                    self.storage,
                )?;
                (base_target.table_def, mapped)
            }
        };

        let table_id = table.id;

        // Check for INSTEAD OF UPDATE trigger
        let instead_of_triggers = if ctx.frame.skip_instead_of {
            vec![]
        } else {
            self.find_triggers(&table, crate::ast::TriggerEvent::Update)
                .into_iter()
                .filter(|t| t.is_instead_of)
                .collect::<Vec<_>>()
        };

        if !instead_of_triggers.is_empty() {
            let mut inserted_rows = Vec::new();
            let mut deleted_rows = Vec::new();
            let rowcount_limit = rowcount_limit(ctx);
            let mut updated_indices = HashSet::new();
            let updated_count = visit_target_rows(
                joined_rows,
                table_id,
                &target_alias,
                rowcount_limit,
                &mut updated_indices,
                |stored_row, _idx, joined_row| {
                    let mut new_row = stored_row.clone();
                    apply_assignments(
                        &table,
                        &mut new_row,
                        &stmt.assignments,
                        joined_row,
                        ctx,
                        self.catalog,
                        self.storage,
                        self.clock,
                    )?;
                    inserted_rows.push(new_row);
                    deleted_rows.push(stored_row.clone());
                    Ok(())
                },
            )?;
            ctx.set_last_rowcount(updated_count);

            self.execute_triggers(
                &table,
                crate::ast::TriggerEvent::Update,
                true,
                &inserted_rows,
                &deleted_rows,
                ctx,
            )?;

            if let Some(output) = stmt.output {
                let inserted: Vec<&crate::storage::StoredRow> = inserted_rows.iter().collect();
                let deleted: Vec<&crate::storage::StoredRow> = deleted_rows.iter().collect();
                let result = build_output_result(
                    &output,
                    &table,
                    &inserted,
                    &deleted,
                    ctx,
                    self.catalog,
                    self.storage,
                    self.clock,
                )?;
                if let Some(target) = stmt.output_into {
                    if let Some(result) = result.as_ref() {
                        self.insert_output_into(&target, &stmt.output_into_columns, result, ctx)?;
                    } else {
                        return Err(DbError::Execution("OUTPUT INTO produced no result".into()));
                    }
                    return Ok(None);
                }
                return Ok(result);
            }
            return Ok(None);
        }

        let has_after_triggers = !self
            .find_triggers(&table, crate::ast::TriggerEvent::Update)
            .into_iter()
            .filter(|t| !t.is_instead_of)
            .collect::<Vec<_>>()
            .is_empty();

        let collect_rows = stmt.output.is_some() || has_after_triggers;
        let has_incoming_fks = self.catalog.get_tables().iter().any(|other| {
            other.foreign_keys.iter().any(|fk| {
                fk.enabled
                    && fk
                        .referenced_table
                        .schema_or_dbo()
                        .eq_ignore_ascii_case(table.schema_or_dbo())
                    && fk.referenced_table.name.eq_ignore_ascii_case(&table.name)
            })
        });
        let has_outgoing_fks = table.foreign_keys.iter().any(|fk| fk.enabled);
        let has_checks = !table.check_constraints.is_empty()
            || table.columns.iter().any(|c| c.check.is_some());
        let has_unique = table.columns.iter().any(|c| c.unique)
            || table.columns.iter().filter(|c| c.primary_key).count() >= 2;
        let table_indexes: Vec<u32> = self
            .catalog
            .get_indexes()
            .iter()
            .filter(|i| i.table_id == table_id)
            .map(|i| i.id)
            .collect();
        let mut inserted_rows_for_output = Vec::new();
        let mut deleted_rows_for_output = Vec::new();
        let rowcount_limit = rowcount_limit(ctx);
        let mut updated_indices = HashSet::new();
        let updated_count = visit_target_rows(
            joined_rows,
            table_id,
            &target_alias,
            rowcount_limit,
            &mut updated_indices,
            |stored_row, idx, joined_row| {
                let mut new_row = stored_row.clone();
                if has_incoming_fks {
                    enforce_foreign_keys_on_delete(&table, self.catalog, self.storage, stored_row)?;
                }
                apply_assignments(
                    &table,
                    &mut new_row,
                    &stmt.assignments,
                    joined_row,
                    ctx,
                    self.catalog,
                    self.storage,
                    self.clock,
                )?;
                if has_incoming_fks {
                    enforce_foreign_keys_on_update(
                        &table,
                        self.catalog,
                        self.storage,
                        stored_row,
                        &new_row,
                    )?;
                }
                validate_row_against_table(&table, &new_row.values)?;
                if has_outgoing_fks {
                    enforce_foreign_keys_on_insert(&table, self.catalog, self.storage, &new_row)?;
                }
                if has_checks {
                    enforce_checks_on_row(
                        &table,
                        &new_row,
                        ctx,
                        self.catalog,
                        self.storage,
                        self.clock,
                    )?;
                }
                if has_unique {
                    enforce_unique_on_update(&table, self.storage, table_id, &new_row, idx)?;
                }

                self.storage.update_row(table_id, idx, new_row.clone())?;

                if !table_indexes.is_empty() {
                    if let Some(index_storage) = self.storage.as_index_storage_mut() {
                        for &idx_id in &table_indexes {
                            if let Some(bi) = index_storage.get_index_mut(idx_id) {
                                let _ = bi.update(idx, &stored_row.values, idx, &new_row.values);
                            }
                        }
                    }
                }

                self.push_dirty_update(ctx, &table.name, idx, &new_row);

                if collect_rows {
                    inserted_rows_for_output.push(new_row);
                    deleted_rows_for_output.push(stored_row.clone());
                }
                Ok(())
            },
        )?;
        ctx.set_last_rowcount(updated_count);

        self.execute_triggers(
            &table,
            crate::ast::TriggerEvent::Update,
            false,
            &inserted_rows_for_output,
            &deleted_rows_for_output,
            ctx,
        )?;

        if let Some(output) = stmt.output {
            let inserted: Vec<&crate::storage::StoredRow> =
                inserted_rows_for_output.iter().collect();
            let deleted: Vec<&crate::storage::StoredRow> = deleted_rows_for_output.iter().collect();
            let result = build_output_result(
                &output,
                &table,
                &inserted,
                &deleted,
                ctx,
                self.catalog,
                self.storage,
                self.clock,
            )?;
            if let Some(target) = stmt.output_into {
                if let Some(result) = result.as_ref() {
                    self.insert_output_into(&target, &stmt.output_into_columns, result, ctx)?;
                } else {
                    return Err(DbError::Execution("OUTPUT INTO produced no result".into()));
                }
                return Ok(None);
            }
            return Ok(result);
        }

        Ok(None)
    }
}
