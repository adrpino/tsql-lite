use crate::ast::{TableFactor, TableRef};
use crate::catalog::{Catalog, ColumnDef, TableDef};
use crate::error::DbError;
use crate::storage::StoredRow;

use crate::executor::context::ExecutionContext;
use crate::executor::cte::resolve_cte_table;
use crate::executor::metadata::resolve_virtual_table;
use crate::executor::model::BoundTable;

pub(super) fn bind_plain_table(
    tref: &TableRef,
    catalog: &dyn Catalog,
    ctx: &mut ExecutionContext,
) -> Result<BoundTable, DbError> {
    match &tref.factor {
        TableFactor::Derived(_) => {
            return Err(DbError::Execution(
                "Subquery binding requires QueryExecutor context".into(),
            ));
        }
        TableFactor::Values { rows, columns } => {
            let alias = tref.alias.clone().unwrap_or_else(|| "VALUES".to_string());
            let mut table_def = TableDef {
                id: 0,
                schema_id: 1,
                schema_name: "dbo".to_string(),
                name: alias.clone(),
                columns: Vec::new(),
                check_constraints: Vec::new(),
                foreign_keys: Vec::new(),
            };

            let first_row_len = rows.first().map(|r| r.len()).unwrap_or(0);
            let col_count = if !columns.is_empty() {
                columns.len()
            } else {
                first_row_len
            };

            for i in 0..col_count {
                let name = columns
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("col{}", i + 1));
                table_def.columns.push(ColumnDef {
                    id: (i + 1) as u32,
                    name,
                    data_type: crate::types::DataType::VarChar { max_len: 4000 },
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
                });
            }

            let mut virtual_rows = Vec::new();
            for row_exprs in rows {
                let mut row_values = Vec::new();
                for expr in row_exprs {
                    let val = match crate::executor::evaluator::eval_constant_expr(
                        expr,
                        ctx,
                        catalog,
                        &crate::storage::InMemoryStorage::default(),
                        &crate::executor::clock::SystemClock,
                    ) {
                        Ok(v) => v,
                        Err(_) => crate::types::Value::Null,
                    };
                    row_values.push(val);
                }
                virtual_rows.push(StoredRow {
                    values: row_values,
                    deleted: false,
                });
            }

            return Ok(BoundTable {
                alias: std::sync::Arc::from(alias),
                table: std::sync::Arc::new(table_def),
                virtual_rows: Some(virtual_rows),
            });
        }
        _ => {}
    }

    let orig_obj = tref.factor.as_object_name();
    let orig_name = orig_obj.map(|o| o.name.as_str()).unwrap_or("");
    let mapped_name = ctx.resolve_table_name(orig_name);
    let effective_alias: Option<&str> = match (&tref.alias, mapped_name.as_deref()) {
        (Some(a), _) => Some(a.as_str()),
        (None, Some(_)) => Some(orig_name),
        (None, None) => None,
    };
    let schema = orig_obj.map(|o| o.schema_or_dbo()).unwrap_or("dbo");
    let name = mapped_name.as_deref().unwrap_or(orig_name);

    if let Some(cte) = resolve_cte_table(&ctx.row.ctes, schema, name) {
        return Ok(BoundTable {
            alias: std::sync::Arc::from(effective_alias.unwrap_or(name)),
            table: std::sync::Arc::new(cte.table_def.clone()),
            virtual_rows: None,
        });
    }

    if let Some((table, rows)) = resolve_virtual_table(schema, name, catalog, ctx) {
        return Ok(BoundTable {
            alias: std::sync::Arc::from(effective_alias.unwrap_or(name)),
            table: std::sync::Arc::new(table),
            virtual_rows: Some(rows),
        });
    }

    let table = catalog
        .find_table_arc(schema, name)
        .ok_or_else(|| DbError::table_not_found(schema, name))?;
    if ctx.session.extras.mutation_target_id != Some(table.id) {
        crate::executor::security::check_object_permission(
            catalog,
            ctx,
            "SELECT",
            table.schema_or_dbo(),
            &table.name,
            table.id as i32,
        )?;
    }

    Ok(BoundTable {
        alias: std::sync::Arc::from(effective_alias.unwrap_or(table.name.as_str())),
        table,
        virtual_rows: None,
    })
}
