use crate::ast::{MergeSource, MergeStmt};
use crate::catalog::Catalog;
use crate::catalog::{ColumnDef, TableDef};
use crate::error::DbError;
use crate::executor::clock::Clock;
use crate::executor::context::ExecutionContext;
use crate::executor::model::{ContextTable, JoinedRow};
use crate::executor::mutation::MergeOutputRow;
use crate::executor::query::plan::RelationalQuery;
use crate::executor::query::QueryExecutor;
use crate::storage::{Storage, StoredRow};
use crate::types::Value;

pub(crate) fn merge_source_alias(stmt: &MergeStmt) -> Result<String, DbError> {
    match &stmt.source {
        MergeSource::Table(t) => Ok(t
            .alias
            .clone()
            .or_else(|| t.name_as_object().map(|o| o.name.clone()))
            .unwrap_or_else(|| "source".to_string())),
        MergeSource::Subquery(_, alias) => {
            Ok(alias.clone().unwrap_or_else(|| "source".to_string()))
        }
    }
}

fn table_def_from_query_result(
    source_alias: String,
    result: &crate::executor::result::QueryResult,
) -> TableDef {
    TableDef {
        id: 0,
        schema_id: 0,
        schema_name: "sys".to_string(),
        name: source_alias,
        columns: result
            .columns
            .iter()
            .enumerate()
            .map(|(idx, col_name)| ColumnDef {
                id: (idx + 1) as u32,
                name: col_name.clone(),
                data_type: result
                    .column_types
                    .get(idx)
                    .cloned()
                    .unwrap_or(crate::types::DataType::VarChar { max_len: 4000 }),
                nullable: true,
                primary_key: false,
                unique: false,
                identity: None,
                default: None,
                default_constraint_name: None,
                computed_expr: None,
                check: None,
                check_constraint_name: None,
                collation: None,
                is_clustered: false,
                ansi_padding_on: true,
                is_rowversion: false,
            })
            .collect(),
        check_constraints: vec![],
        foreign_keys: vec![],
        unique_constraints: vec![],
    }
}

pub(crate) fn merge_source_rows(
    stmt: &MergeStmt,
    executor: &QueryExecutor<'_>,
    ctx: &mut ExecutionContext<'_>,
) -> Result<(TableDef, Vec<Vec<crate::types::Value>>), DbError> {
    let source_alias = merge_source_alias(stmt)?;
    match &stmt.source {
        MergeSource::Table(source_ref) => match &source_ref.factor {
            crate::ast::TableFactor::Named(source_name) => {
                if let Some(cte) = ctx.row.ctes.get(&source_name.name) {
                    let mut tbl = cte.table_def.clone();
                    tbl.name = source_alias;
                    let rows = cte
                        .rows
                        .iter()
                        .filter(|r| !r.deleted)
                        .map(|r| r.values.clone())
                        .collect();
                    return Ok((tbl, rows));
                }
                let resolved = ctx
                    .resolve_table_name(source_name.name.as_str())
                    .unwrap_or_else(|| source_name.name.clone());
                let source_schema = source_name.schema_or_dbo();
                let mut source_table = executor
                    .catalog
                    .find_table(source_schema, &resolved)
                    .ok_or_else(|| {
                        DbError::Semantic(format!(
                            "table '{}.{}' not found",
                            source_schema, resolved
                        ))
                    })?
                    .clone();

                let mut rows = Vec::new();
                for row in executor.storage.scan_rows(source_table.id)? {
                    let row = row?;
                    if row.deleted {
                        continue;
                    }
                    rows.push(row.values.clone());
                }
                source_table.name = source_alias;
                Ok((source_table, rows))
            }
            crate::ast::TableFactor::Derived(select_stmt) => {
                let result = QueryExecutor {
                    catalog: executor.catalog,
                    storage: executor.storage,
                    clock: executor.clock,
                }
                .execute_select(RelationalQuery::from_ref(select_stmt), ctx)?;
                let table_def = table_def_from_query_result(source_alias, &result);
                Ok((table_def, result.rows))
            }
            crate::ast::TableFactor::Values { .. } => {
                let wrapper = crate::ast::SelectStmt {
                    from_clause: Some(crate::ast::FromNode::Table(source_ref.clone())),
                    applies: Vec::new(),
                    projection: vec![crate::ast::SelectItem {
                        expr: crate::ast::Expr::Wildcard,
                        alias: None,
                    }],
                    into_table: None,
                    distinct: false,
                    top: None,
                    selection: None,
                    group_by: Vec::new(),
                    having: None,
                    order_by: Vec::new(),
                    offset: None,
                    fetch: None,
                    set_op: None,
                    extra: None,
                };
                let result = QueryExecutor {
                    catalog: executor.catalog,
                    storage: executor.storage,
                    clock: executor.clock,
                }
                .execute_select(RelationalQuery::from(wrapper), ctx)?;
                let table_def = table_def_from_query_result(source_alias, &result);
                Ok((table_def, result.rows))
            }
        },
        MergeSource::Subquery(select_stmt, _alias) => {
            let result = QueryExecutor {
                catalog: executor.catalog,
                storage: executor.storage,
                clock: executor.clock,
            }
            .execute_select(RelationalQuery::from_ref(select_stmt), ctx)?;
            let table_def = table_def_from_query_result(source_alias, &result);
            Ok((table_def, result.rows))
        }
    }
}

pub(crate) fn merge_target_context(
    table: &TableDef,
    alias: String,
    row: StoredRow,
    storage_index: Option<usize>,
) -> JoinedRow {
    vec![ContextTable {
        table: std::sync::Arc::new(table.clone()),
        alias: std::sync::Arc::from(alias),
        row: Some(std::sync::Arc::new(row)),
        storage_index,
        source_aliases: Vec::new(),
    }]
}

pub(crate) fn merge_source_context(
    table: &TableDef,
    alias: String,
    row: StoredRow,
    storage_index: Option<usize>,
) -> JoinedRow {
    vec![ContextTable {
        table: std::sync::Arc::new(table.clone()),
        alias: std::sync::Arc::from(alias),
        row: Some(std::sync::Arc::new(row)),
        storage_index,
        source_aliases: Vec::new(),
    }]
}

fn evaluate_computed_columns(
    target_table: &TableDef,
    values: &mut [Value],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<(), DbError> {
    if !target_table
        .columns
        .iter()
        .any(|c| c.computed_expr.is_some())
    {
        return Ok(());
    }
    let table_arc = std::sync::Arc::new(target_table.clone());
    let alias_arc: std::sync::Arc<str> = std::sync::Arc::from(target_table.name.as_str());
    for (idx, col) in target_table.columns.iter().enumerate() {
        if let Some(ref computed) = col.computed_expr {
            let joined = vec![ContextTable {
                table: std::sync::Arc::clone(&table_arc),
                alias: std::sync::Arc::clone(&alias_arc),
                row: Some(std::sync::Arc::new(StoredRow {
                    values: values.to_vec(),
                    deleted: false,
                })),
                storage_index: None,
                source_aliases: Vec::new(),
            }];
            values[idx] = crate::executor::evaluator::eval_expr(
                computed, &joined, ctx, catalog, storage, clock,
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn merge_apply_update_action(
    stmt: &MergeStmt,
    target_table: &TableDef,
    ctx: &mut ExecutionContext<'_>,
    catalog: &mut dyn crate::catalog::Catalog,
    storage: &mut dyn crate::storage::Storage,
    clock: &dyn crate::executor::clock::Clock,
    combined_ctx: &JoinedRow,
    target_row: &StoredRow,
    assignments: &[crate::ast::Assignment],
    updated_target_rows: &mut [StoredRow],
    row_index: usize,
    merge_output_rows: &mut Vec<crate::executor::mutation::MergeOutputRow>,
    inserted_rows_for_trigger: &mut Vec<StoredRow>,
    deleted_rows_for_trigger: &mut Vec<StoredRow>,
) -> Result<(), DbError> {
    let old_values = target_row.values.clone();
    let mut temp_row = target_row.clone();
    for assign in assignments {
        let val = crate::executor::evaluator::eval_expr(
            &assign.expr,
            combined_ctx,
            ctx,
            catalog,
            storage,
            clock,
        )?;
        if assign.column.starts_with('@') {
            if let Some((ty, var)) = ctx.session.get_var_mut(&assign.column) {
                *var = crate::executor::value_ops::coerce_value_to_type_with_dateformat(
                    val,
                    ty,
                    &ctx.options.dateformat,
                )?;
            } else {
                return Err(DbError::invalid_identifier(&assign.column));
            }
            continue;
        }
        let col_idx = target_table
            .columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(&assign.column))
            .ok_or_else(|| DbError::Semantic(format!("column '{}' not found", assign.column)))?;
        let col = &target_table.columns[col_idx];
        let val = if !val.is_null() {
            crate::executor::value_ops::coerce_value_to_type_with_dateformat(
                val,
                &col.data_type,
                &ctx.options.dateformat,
            )?
        } else {
            val
        };
        temp_row.values[col_idx] = val;
    }
    evaluate_computed_columns(
        target_table,
        &mut temp_row.values,
        ctx,
        catalog,
        storage,
        clock,
    )?;
    crate::executor::mutation::validation::enforce_foreign_keys_on_delete(
        target_table,
        catalog,
        storage,
        target_row,
    )?;
    crate::executor::mutation::validation::enforce_foreign_keys_on_insert(
        target_table,
        catalog,
        storage,
        &temp_row,
    )?;
    updated_target_rows[row_index] = temp_row.clone();
    if stmt.output.is_some() {
        merge_output_rows.push(crate::executor::mutation::MergeOutputRow {
            inserted_values: Some(temp_row.values.clone()),
            deleted_values: Some(old_values),
            source_row: combined_ctx.clone(),
            action: "UPDATE",
        });
    }
    inserted_rows_for_trigger.push(temp_row);
    deleted_rows_for_trigger.push(target_row.clone());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn merge_apply_delete_action(
    stmt: &MergeStmt,
    target_table: &TableDef,
    catalog: &mut dyn crate::catalog::Catalog,
    storage: &mut dyn crate::storage::Storage,
    target_row: &StoredRow,
    updated_target_rows: &mut [StoredRow],
    row_index: usize,
    merge_output_rows: &mut Vec<crate::executor::mutation::MergeOutputRow>,
    deleted_rows_for_trigger: &mut Vec<StoredRow>,
) -> Result<(), DbError> {
    crate::executor::mutation::validation::enforce_foreign_keys_on_delete(
        target_table,
        catalog,
        storage,
        target_row,
    )?;
    if stmt.output.is_some() {
        merge_output_rows.push(crate::executor::mutation::MergeOutputRow {
            inserted_values: None,
            deleted_values: Some(target_row.values.clone()),
            source_row: Vec::new(),
            action: "DELETE",
        });
    }
    deleted_rows_for_trigger.push(target_row.clone());
    updated_target_rows[row_index].deleted = true;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn merge_apply_insert_action(
    stmt: &MergeStmt,
    target_table: &TableDef,
    ctx: &mut ExecutionContext<'_>,
    catalog: &mut dyn crate::catalog::Catalog,
    storage: &mut dyn crate::storage::Storage,
    clock: &dyn crate::executor::clock::Clock,
    source_ctx: &JoinedRow,
    columns: &[String],
    values: &[crate::ast::Expr],
    merge_output_rows: &mut Vec<crate::executor::mutation::MergeOutputRow>,
    inserted_rows_for_trigger: &mut Vec<StoredRow>,
) -> Result<StoredRow, DbError> {
    let mut final_values = vec![crate::types::Value::Null; target_table.columns.len()];
    for (col_name, val_expr) in columns.iter().zip(values.iter()) {
        let col_idx = target_table
            .columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(col_name))
            .ok_or_else(|| DbError::Semantic(format!("column '{}' not found", col_name)))?;
        let val = crate::executor::evaluator::eval_expr(
            val_expr, source_ctx, ctx, catalog, storage, clock,
        )?;
        let col = &target_table.columns[col_idx];
        let val = if !val.is_null() {
            crate::executor::value_ops::coerce_value_to_type_with_dateformat(
                val,
                &col.data_type,
                &ctx.options.dateformat,
            )?
        } else {
            val
        };
        final_values[col_idx] = val;
    }

    let mut temp_row = StoredRow {
        values: final_values.clone(),
        deleted: false,
    };
    for (idx, col) in target_table.columns.iter().enumerate() {
        if matches!(final_values[idx], crate::types::Value::Null) {
            if col.identity.is_some() {
                let next_val = catalog.next_identity_value(target_table.id, &col.name)?;
                ctx.set_last_identity(next_val);
                final_values[idx] = match &col.data_type {
                    crate::types::DataType::Int => crate::types::Value::Int(next_val as i32),
                    crate::types::DataType::BigInt => crate::types::Value::BigInt(next_val),
                    _ => crate::types::Value::Int(next_val as i32),
                };
            } else if let Some(ref default_expr) = col.default {
                final_values[idx] = crate::executor::evaluator::eval_expr(
                    default_expr,
                    &[],
                    ctx,
                    catalog,
                    storage,
                    clock,
                )?;
            }
        }
    }
    evaluate_computed_columns(
        target_table,
        &mut final_values,
        ctx,
        catalog,
        storage,
        clock,
    )?;
    temp_row.values = final_values.clone();
    crate::executor::mutation::validation::enforce_foreign_keys_on_insert(
        target_table,
        catalog,
        storage,
        &temp_row,
    )?;
    if stmt.output.is_some() {
        merge_output_rows.push(crate::executor::mutation::MergeOutputRow {
            inserted_values: Some(final_values),
            deleted_values: None,
            source_row: source_ctx.clone(),
            action: "INSERT",
        });
    }
    inserted_rows_for_trigger.push(temp_row.clone());
    Ok(temp_row)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn merge_process_matched_phase(
    stmt: &MergeStmt,
    target_table: &TableDef,
    target_alias: &str,
    source_table_def: &TableDef,
    source_rows: &[Vec<Value>],
    target_rows: &[StoredRow],
    ctx: &mut ExecutionContext<'_>,
    catalog: &mut dyn Catalog,
    storage: &mut dyn Storage,
    clock: &dyn Clock,
    source_matched_to_target: &mut [bool],
    updated_target_rows: &mut [StoredRow],
    merge_output_rows: &mut Vec<MergeOutputRow>,
    inserted_rows_for_trigger: &mut Vec<StoredRow>,
    deleted_rows_for_trigger: &mut Vec<StoredRow>,
) -> Result<(), DbError> {
    let source_alias = merge_source_alias(stmt)?;

    for (i, target_row) in target_rows.iter().enumerate() {
        if target_row.deleted {
            continue;
        }
        let mut target_modified = false;

        for (s_idx, source_row) in source_rows.iter().enumerate() {
            let mut combined_ctx = merge_target_context(
                target_table,
                target_alias.to_string(),
                target_rows[i].clone(),
                Some(i),
            );
            combined_ctx.extend(merge_source_context(
                source_table_def,
                source_alias.clone(),
                StoredRow {
                    values: source_row.clone(),
                    deleted: false,
                },
                Some(s_idx),
            ));

            let on_matches_val = crate::executor::evaluator::eval_expr(
                &stmt.on_condition,
                &combined_ctx,
                ctx,
                catalog,
                storage,
                clock,
            )?;

            let on_matches = match on_matches_val {
                Value::Bit(b) => b,
                Value::Null => false,
                _ => crate::executor::value_ops::truthy(&on_matches_val),
            };

            if !on_matches {
                continue;
            }

            source_matched_to_target[s_idx] = true;

            for when_clause in &stmt.when_clauses {
                if !matches!(when_clause.when, crate::ast::MergeWhen::Matched) {
                    continue;
                }
                if let Some(cond) = &when_clause.condition {
                    let cond_val = crate::executor::evaluator::eval_predicate(
                        cond,
                        &combined_ctx,
                        ctx,
                        catalog,
                        storage,
                        clock,
                    )?;
                    if !cond_val {
                        continue;
                    }
                }

                if target_modified {
                    return Err(DbError::Execution(
                        "The MERGE statement attempted to UPDATE or DELETE the same row more than once. This happens when a target row matches more than one source row. (Error 8672)".into(),
                    ));
                }
                target_modified = true;

                match &when_clause.action {
                    crate::ast::MergeAction::Update { assignments } => {
                        merge_apply_update_action(
                            stmt,
                            target_table,
                            ctx,
                            catalog,
                            storage,
                            clock,
                            &combined_ctx,
                            &target_rows[i],
                            assignments,
                            updated_target_rows,
                            i,
                            merge_output_rows,
                            inserted_rows_for_trigger,
                            deleted_rows_for_trigger,
                        )?;
                    }
                    crate::ast::MergeAction::Delete => {
                        merge_apply_delete_action(
                            stmt,
                            target_table,
                            catalog,
                            storage,
                            &target_rows[i],
                            updated_target_rows,
                            i,
                            merge_output_rows,
                            deleted_rows_for_trigger,
                        )?;
                    }
                    _ => {
                        return Err(DbError::Execution(
                            "Invalid action for NOT MATCHED BY SOURCE".into(),
                        ));
                    }
                }
                break;
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn merge_process_not_matched_phase(
    stmt: &MergeStmt,
    target_table: &TableDef,
    source_table_def: &TableDef,
    source_rows: &[Vec<Value>],
    ctx: &mut ExecutionContext<'_>,
    catalog: &mut dyn Catalog,
    storage: &mut dyn Storage,
    clock: &dyn Clock,
    source_matched_to_target: &mut [bool],
    merge_output_rows: &mut Vec<MergeOutputRow>,
    inserted_rows_for_trigger: &mut Vec<StoredRow>,
    inserted_new_rows: &mut Vec<StoredRow>,
) -> Result<(), DbError> {
    let source_alias = merge_source_alias(stmt)?;

    for (s_idx, source_row) in source_rows.iter().enumerate() {
        if source_matched_to_target[s_idx] {
            continue;
        }

        let source_ctx = merge_source_context(
            source_table_def,
            source_alias.clone(),
            StoredRow {
                values: source_row.clone(),
                deleted: false,
            },
            Some(s_idx),
        );

        for when_clause in &stmt.when_clauses {
            if !matches!(when_clause.when, crate::ast::MergeWhen::NotMatched) {
                continue;
            }
            if let Some(cond) = &when_clause.condition {
                let cond_val = crate::executor::evaluator::eval_predicate(
                    cond,
                    &source_ctx,
                    ctx,
                    catalog,
                    storage,
                    clock,
                )?;
                if !cond_val {
                    continue;
                }
            }

            source_matched_to_target[s_idx] = true;
            match &when_clause.action {
                crate::ast::MergeAction::Insert { columns, values } => {
                    let temp_row = merge_apply_insert_action(
                        stmt,
                        target_table,
                        ctx,
                        catalog,
                        storage,
                        clock,
                        &source_ctx,
                        columns,
                        values,
                        merge_output_rows,
                        inserted_rows_for_trigger,
                    )?;
                    inserted_new_rows.push(temp_row);
                }
                _ => {
                    return Err(DbError::Execution(
                        "only INSERT is allowed in WHEN NOT MATCHED".into(),
                    ));
                }
            }
            break;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn merge_process_not_matched_by_source_phase(
    stmt: &MergeStmt,
    target_table: &TableDef,
    target_alias: &str,
    source_table_def: &TableDef,
    source_rows: &[Vec<Value>],
    target_rows: &[StoredRow],
    ctx: &mut ExecutionContext<'_>,
    catalog: &mut dyn Catalog,
    storage: &mut dyn Storage,
    clock: &dyn Clock,
    updated_target_rows: &mut [StoredRow],
    merge_output_rows: &mut Vec<MergeOutputRow>,
    inserted_rows_for_trigger: &mut Vec<StoredRow>,
    deleted_rows_for_trigger: &mut Vec<StoredRow>,
) -> Result<(), DbError> {
    let source_alias = merge_source_alias(stmt)?;

    let has_not_matched_by_source = stmt
        .when_clauses
        .iter()
        .any(|wc| matches!(wc.when, crate::ast::MergeWhen::NotMatchedBySource));

    if !has_not_matched_by_source {
        return Ok(());
    }

    for (i, target_row) in target_rows.iter().enumerate() {
        if target_row.deleted || updated_target_rows[i].deleted {
            continue;
        }

        let mut any_source_match = false;
        for source_row in source_rows.iter() {
            let mut combined_ctx = merge_target_context(
                target_table,
                target_alias.to_string(),
                updated_target_rows[i].clone(),
                Some(i),
            );
            combined_ctx.extend(merge_source_context(
                source_table_def,
                source_alias.clone(),
                StoredRow {
                    values: source_row.clone(),
                    deleted: false,
                },
                None,
            ));

            let on_matches_val = crate::executor::evaluator::eval_expr(
                &stmt.on_condition,
                &combined_ctx,
                ctx,
                catalog,
                storage,
                clock,
            )?;

            let on_matches = match on_matches_val {
                Value::Bit(b) => b,
                Value::Null => false,
                _ => crate::executor::value_ops::truthy(&on_matches_val),
            };

            if on_matches {
                any_source_match = true;
                break;
            }
        }

        if any_source_match {
            continue;
        }

        let target_ctx = merge_target_context(
            target_table,
            target_alias.to_string(),
            updated_target_rows[i].clone(),
            Some(i),
        );

        for when_clause in &stmt.when_clauses {
            if !matches!(when_clause.when, crate::ast::MergeWhen::NotMatchedBySource) {
                continue;
            }
            if let Some(cond) = &when_clause.condition {
                let cond_val = crate::executor::evaluator::eval_predicate(
                    cond,
                    &target_ctx,
                    ctx,
                    catalog,
                    storage,
                    clock,
                )?;
                if !cond_val {
                    continue;
                }
            }

            match &when_clause.action {
                crate::ast::MergeAction::Update { assignments } => {
                    let target_row_snapshot = updated_target_rows[i].clone();
                    merge_apply_update_action(
                        stmt,
                        target_table,
                        ctx,
                        catalog,
                        storage,
                        clock,
                        &target_ctx,
                        &target_row_snapshot,
                        assignments,
                        updated_target_rows,
                        i,
                        merge_output_rows,
                        inserted_rows_for_trigger,
                        deleted_rows_for_trigger,
                    )?;
                }
                crate::ast::MergeAction::Delete => {
                    let target_row_snapshot = updated_target_rows[i].clone();
                    merge_apply_delete_action(
                        stmt,
                        target_table,
                        catalog,
                        storage,
                        &target_row_snapshot,
                        updated_target_rows,
                        i,
                        merge_output_rows,
                        deleted_rows_for_trigger,
                    )?;
                }
                _ => {
                    return Err(DbError::Execution(
                        "only UPDATE or DELETE is allowed in WHEN NOT MATCHED BY SOURCE".into(),
                    ));
                }
            }
            break;
        }
    }

    Ok(())
}
