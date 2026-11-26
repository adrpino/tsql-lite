use std::borrow::Cow;
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
    apply_assignments, enforce_checks_on_row, enforce_foreign_keys_on_insert,
    enforce_foreign_keys_on_update, enforce_unique_on_update, validate_row_against_table,
};
use super::MutationExecutor;

impl<'a> MutationExecutor<'a> {
    pub(crate) fn execute_update_with_context(
        &mut self,
        stmt: &UpdateStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        let mut target_table = stmt.table.clone();
        if let Some(mapped) = ctx.resolve_table_name(&target_table.name) {
            target_table.name = mapped;
            if target_table.schema.is_none() {
                target_table.schema = Some("dbo".to_string());
            }
        }
        let mut target_alias = target_table.name.clone();
        let mut view_check_predicates = Vec::new();
        let mut assignments: Cow<'_, [crate::ast::Assignment]> =
            Cow::Borrowed(stmt.assignments.as_slice());
        let resolved_direct =
            resolve_table_for_mutation(stmt.from.as_ref(), &target_table, |schema, name| {
                self.catalog.find_table(schema, name).cloned().or_else(|| {
                    ctx.resolve_table_name(name)
                        .and_then(|mapped| self.catalog.find_table("dbo", &mapped).cloned())
                })
            });

        let (table, joined_rows) = match resolved_direct {
            Ok((table, resolved_name)) => {
                crate::executor::security::check_object_permission(
                    self.catalog,
                    ctx,
                    "UPDATE",
                    table.schema_or_dbo(),
                    &table.name,
                    table.id as i32,
                )?;
                let query = build_mutation_query(
                    stmt.from.as_ref(),
                    &target_table,
                    &table,
                    &resolved_name,
                    stmt.selection.as_ref().map(Cow::Borrowed),
                    stmt.top.as_ref(),
                );
                let query_executor = QueryExecutor {
                    catalog: self.catalog,
                    storage: self.storage,
                    clock: self.clock,
                };
                let prev_target = ctx.session.extras.mutation_target_id.replace(table.id);
                let joined_res = query_executor.execute_to_joined_rows(query, ctx);
                ctx.session.extras.mutation_target_id = prev_target;
                let joined_rows = joined_res?;
                (table, joined_rows)
            }
            Err(orig_err) => {
                let view_schema = target_table.schema_or_dbo().to_string();
                let view_name = target_table.name.clone();
                if stmt.from.is_none() && self.catalog.find_view(&view_schema, &view_name).is_some()
                {
                    let view_target = super::view_dml::resolve_updatable_view(
                        self.catalog,
                        &view_schema,
                        &view_name,
                    )?
                    .ok_or(orig_err)?;
                    let rewritten_assigns = assignments.to_mut();
                    for assign in rewritten_assigns.iter_mut() {
                        if !assign.column.starts_with('@') {
                            assign.column =
                                view_target.map_view_column(&assign.column, &view_name)?;
                        }
                        assign.expr = super::view_dml::rewrite_expr_columns(
                            &assign.expr,
                            &view_target.columns,
                            &view_name,
                            &view_target.base_table.name,
                        );
                    }
                    let rewritten_sel = stmt.selection.as_ref().map(|sel| {
                        super::view_dml::rewrite_expr_columns(
                            sel,
                            &view_target.columns,
                            &view_name,
                            &view_target.base_table.name,
                        )
                    });
                    let combined_sel = view_target.combined_where(rewritten_sel);
                    target_table = view_target.base_table.clone();
                    target_alias = view_target.base_table.name.clone();
                    view_check_predicates = view_target.check_predicates;
                    let table = view_target.base_table_def;
                    crate::executor::security::check_object_permission(
                        self.catalog,
                        ctx,
                        "UPDATE",
                        table.schema_or_dbo(),
                        &table.name,
                        table.id as i32,
                    )?;
                    let query = build_mutation_query(
                        None,
                        &target_table,
                        &table,
                        &target_table.name,
                        combined_sel.map(Cow::Owned),
                        stmt.top.as_ref(),
                    );
                    let query_executor = QueryExecutor {
                        catalog: self.catalog,
                        storage: self.storage,
                        clock: self.clock,
                    };
                    let prev_target = ctx.session.extras.mutation_target_id.replace(table.id);
                    let joined_res = query_executor.execute_to_joined_rows(query, ctx);
                    ctx.session.extras.mutation_target_id = prev_target;
                    let joined_rows = joined_res?;
                    (table, joined_rows)
                } else {
                    let Some((cte, cte_name)) =
                        resolve_cte_for_mutation(stmt.from.as_ref(), &target_table, &ctx.row.ctes)
                    else {
                        return Err(orig_err);
                    };
                    let base_target = cte
                        .base_targets
                        .iter()
                        .find(|bt| {
                            assignments.iter().all(|a| {
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
                        &target_table,
                        &cte.table_def,
                        &cte_name,
                        stmt.selection.as_ref().map(Cow::Borrowed),
                        stmt.top.as_ref(),
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
                        &assignments,
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

            let updated_cols: Vec<String> = stmt
                .assignments
                .iter()
                .filter(|a| !a.column.starts_with('@'))
                .map(|a| a.column.clone())
                .collect();
            self.execute_triggers_with_updated_cols(
                &table,
                crate::ast::TriggerEvent::Update,
                true,
                &inserted_rows,
                &deleted_rows,
                Some(&updated_cols),
                ctx,
            )?;

            if let Some(ref output) = stmt.output {
                let inserted: Vec<&crate::storage::StoredRow> = inserted_rows.iter().collect();
                let deleted: Vec<&crate::storage::StoredRow> = deleted_rows.iter().collect();
                let result = build_output_result(
                    output,
                    &table,
                    &inserted,
                    &deleted,
                    ctx,
                    self.catalog,
                    self.storage,
                    self.clock,
                )?;
                if let Some(ref target) = stmt.output_into {
                    if let Some(result) = result.as_ref() {
                        self.insert_output_into(target, &stmt.output_into_columns, result, ctx)?;
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
        let has_checks =
            !table.check_constraints.is_empty() || table.columns.iter().any(|c| c.check.is_some());
        crate::executor::security::check_object_permission(
            self.catalog,
            ctx,
            "UPDATE",
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
        // Atomic pre-validation of view WITH CHECK OPTION and RLS BEFORE/AFTER UPDATE block predicates
        // before any storage mutation.
        let has_rls_update_blocks = !crate::executor::rls::active_block_predicates(
            self.catalog,
            &table,
            crate::catalog::SecurityBlockOp::BeforeUpdate,
        )
        .is_empty()
            || !crate::executor::rls::active_block_predicates(
                self.catalog,
                &table,
                crate::catalog::SecurityBlockOp::AfterUpdate,
            )
            .is_empty();
        if !view_check_predicates.is_empty() || has_rls_update_blocks {
            let mut old_preview_rows = Vec::new();
            let mut preview_rows = Vec::new();
            let mut preview_seen = HashSet::new();
            let _ = visit_target_rows(
                joined_rows.clone(),
                table_id,
                &target_alias,
                rowcount_limit,
                &mut preview_seen,
                |stored_row, _idx, joined_row| {
                    old_preview_rows.push(stored_row.clone());
                    let mut new_row = stored_row.clone();
                    apply_assignments(
                        &table,
                        &mut new_row,
                        &assignments,
                        joined_row,
                        ctx,
                        self.catalog,
                        self.storage,
                        self.clock,
                    )?;
                    preview_rows.push(new_row);
                    Ok(())
                },
            )?;
            if has_rls_update_blocks {
                crate::executor::rls::enforce_rls_block_predicates(
                    &table,
                    &old_preview_rows,
                    crate::catalog::SecurityBlockOp::BeforeUpdate,
                    ctx,
                    self.catalog,
                    self.storage,
                    self.clock,
                )?;
                crate::executor::rls::enforce_rls_block_predicates(
                    &table,
                    &preview_rows,
                    crate::catalog::SecurityBlockOp::AfterUpdate,
                    ctx,
                    self.catalog,
                    self.storage,
                    self.clock,
                )?;
            }
            if !view_check_predicates.is_empty() {
                super::view_dml::validate_view_check_option(
                    &table,
                    &preview_rows,
                    &view_check_predicates,
                    ctx,
                    self.catalog,
                    self.storage,
                    self.clock,
                )?;
            }
        }
        let mut updated_indices = HashSet::new();
        let updated_count = visit_target_rows(
            joined_rows,
            table_id,
            &target_alias,
            rowcount_limit,
            &mut updated_indices,
            |stored_row, idx, joined_row| {
                if let Some(ref rt) = ctx.session.extras.row_locks {
                    rt.acquire_write_row_lock_and_check_snapshot(
                        table_id,
                        &table.schema_name,
                        &table.name,
                        idx,
                    )?;
                }
                let mut new_row = stored_row.clone();
                apply_assignments(
                    &table,
                    &mut new_row,
                    &assignments,
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
                    if has_unique_index {
                        super::validation::enforce_unique_indexes_on_write(
                            &table,
                            self.catalog,
                            self.storage,
                            table_id,
                            &new_row,
                            Some(idx),
                            ctx,
                            self.clock,
                        )?;
                    }
                }

                crate::executor::temporal::archive_old_row_on_update(
                    &table,
                    stored_row,
                    &mut new_row,
                    self.catalog,
                    self.storage,
                    self.clock,
                )?;
                self.storage.update_row(table_id, idx, new_row.clone())?;

                if !table_indexes.is_empty() {
                    if let Some(index_storage) = self.storage.as_index_storage_mut() {
                        for &idx_id in &table_indexes {
                            if let Some(bi) = index_storage.get_index_mut(table_id, idx_id) {
                                let _ = bi.update(idx, &stored_row.values, idx, &new_row.values);
                            }
                        }
                    }
                }

                let dirty_key = format!("{}\x02{}", table.schema_name, table.name);
                self.push_dirty_update(ctx, &dirty_key, idx, &new_row);

                if collect_rows {
                    inserted_rows_for_output.push(new_row);
                    deleted_rows_for_output.push(stored_row.clone());
                }
                Ok(())
            },
        )?;
        ctx.set_last_rowcount(updated_count);

        let updated_cols: Vec<String> = stmt
            .assignments
            .iter()
            .filter(|a| !a.column.starts_with('@'))
            .map(|a| a.column.clone())
            .collect();
        self.execute_triggers_with_updated_cols(
            &table,
            crate::ast::TriggerEvent::Update,
            false,
            &inserted_rows_for_output,
            &deleted_rows_for_output,
            Some(&updated_cols),
            ctx,
        )?;

        if let Some(ref output) = stmt.output {
            let inserted: Vec<&crate::storage::StoredRow> =
                inserted_rows_for_output.iter().collect();
            let deleted: Vec<&crate::storage::StoredRow> = deleted_rows_for_output.iter().collect();
            let result = build_output_result(
                output,
                &table,
                &inserted,
                &deleted,
                ctx,
                self.catalog,
                self.storage,
                self.clock,
            )?;
            if let Some(ref target) = stmt.output_into {
                if let Some(result) = result.as_ref() {
                    self.insert_output_into(target, &stmt.output_into_columns, result, ctx)?;
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
