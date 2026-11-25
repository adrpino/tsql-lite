use crate::catalog::{ColumnDef, TableDef};
use crate::error::DbError;
use crate::storage::StoredRow;

use super::super::QueryExecutor;
use crate::executor::context::ExecutionContext;
use crate::executor::model::{ContextTable, JoinedRow};
use crate::executor::result::QueryResult;

pub(crate) fn execute_apply(
    rows: Vec<JoinedRow>,
    apply: &crate::ast::ApplyClause,
    ctx: &mut ExecutionContext,
    executor: &QueryExecutor<'_>,
) -> Result<Vec<JoinedRow>, DbError> {
    let mut result_rows = Vec::new();
    let mut cached_table: Option<std::sync::Arc<TableDef>> = None;

    for left_row in &rows {
        ctx.push_apply_row(left_row.clone());
        let sub_result = executor.execute_select((*apply.subquery).clone().into(), ctx)?;
        ctx.pop_apply_row();

        if sub_result.rows.is_empty() {
            if apply.apply_type == crate::ast::ApplyType::Outer {
                let mut combined = left_row.clone();
                let vtable = if !sub_result.columns.is_empty()
                    && !sub_result.columns.iter().any(|c| c == "*")
                {
                    std::sync::Arc::new(build_virtual_table(&apply.alias, &sub_result))
                } else if let Some(ref known) = cached_table {
                    std::sync::Arc::clone(known)
                } else {
                    std::sync::Arc::new(
                        resolve_empty_apply_table(apply, executor, ctx)
                            .unwrap_or_else(|| build_virtual_table(&apply.alias, &sub_result)),
                    )
                };
                combined.push(
                    ContextTable {
                        table: vtable,
                        alias: std::sync::Arc::from(apply.alias.as_str()),
                        row: None,
                        storage_index: None,
                        source_aliases: Vec::new(),
                    }
                    .null_row(),
                );
                result_rows.push(combined);
            }
        } else {
            let apply_table = std::sync::Arc::new(build_virtual_table(&apply.alias, &sub_result));
            let apply_alias: std::sync::Arc<str> = std::sync::Arc::from(apply.alias.as_str());
            if cached_table.is_none() {
                cached_table = Some(std::sync::Arc::clone(&apply_table));
            }
            for (idx, sub_row_values) in sub_result.rows.iter().enumerate() {
                let mut combined = left_row.clone();
                combined.push(ContextTable {
                    table: std::sync::Arc::clone(&apply_table),
                    alias: std::sync::Arc::clone(&apply_alias),
                    row: Some(StoredRow {
                        values: sub_row_values.clone(),
                        deleted: false,
                    }),
                    storage_index: Some(idx),
                    source_aliases: Vec::new(),
                });
                result_rows.push(combined);
            }
        }
    }

    Ok(result_rows)
}

fn resolve_empty_apply_table(
    apply: &crate::ast::ApplyClause,
    executor: &QueryExecutor<'_>,
    ctx: &ExecutionContext,
) -> Option<TableDef> {
    if let Some(crate::ast::FromNode::Table(tref)) = &apply.subquery.from_clause {
        if let Some(obj) = tref.factor.as_object_name() {
            let name = ctx
                .resolve_table_name(&obj.name)
                .unwrap_or_else(|| obj.name.clone());
            let schema = obj.schema_or_dbo();
            if let Some(t) = executor.catalog.find_table(schema, &name) {
                let mut cloned = t.clone();
                cloned.name = apply.alias.clone();
                return Some(cloned);
            }
        }
    }
    None
}

fn build_virtual_table(alias: &str, sub_result: &QueryResult) -> TableDef {
    TableDef {
        id: 0,
        schema_id: 1,
        schema_name: "dbo".to_string(),
        name: alias.to_string(),
        columns: sub_result
            .columns
            .iter()
            .enumerate()
            .map(|(i, cname)| ColumnDef {
                id: (i + 1) as u32,
                name: cname.clone(),
                data_type: sub_result.column_types[i].clone(),
                nullable: true,
                primary_key: false,
                unique: false,
                identity: None,
                default: None,
                default_constraint_name: None,
                check: None,
                check_constraint_name: None,
                computed_expr: None,
                collation: None,
                is_clustered: false,
                ansi_padding_on: true,
                is_rowversion: false,
            })
            .collect(),
        check_constraints: vec![],
        foreign_keys: vec![],
    }
}
