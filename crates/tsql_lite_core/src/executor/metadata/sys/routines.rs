use super::super::virtual_table_def;
use super::super::VirtualTable;
use crate::catalog::Catalog;
use crate::executor::context::ExecutionContext;
use crate::storage::StoredRow;
use crate::types::{DataType, Value};

pub(crate) struct SysRoutines;
pub(crate) struct SysProcedures;
pub(crate) struct SysFunctions;

impl VirtualTable for SysRoutines {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "routines",
            vec![
                ("object_id", DataType::Int, false),
                ("schema_id", DataType::Int, false),
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("type", DataType::Char { len: 2 }, false),
                ("type_desc", DataType::VarChar { max_len: 60 }, false),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, ctx: &ExecutionContext) -> Vec<StoredRow> {
        let active_db = crate::executor::database_catalog::current_database_name(ctx);
        let mut rows = Vec::new();
        for r in catalog.get_routines() {
            if !crate::executor::database_catalog::schema_belongs_to_db(
                &r.schema, active_db, catalog,
            ) {
                continue;
            }
            let schema_id = catalog.get_schema_id(&r.schema).unwrap_or(1);
            let (ty, desc) = match &r.kind {
                crate::catalog::RoutineKind::Procedure { .. } => {
                    ("P ".to_string(), "SQL_STORED_PROCEDURE".to_string())
                }
                crate::catalog::RoutineKind::Function { body, .. }
                    if matches!(body.as_ref(), crate::ast::FunctionBody::InlineTable(_)) =>
                {
                    (
                        "IF".to_string(),
                        "SQL_INLINE_TABLE_VALUED_FUNCTION".to_string(),
                    )
                }
                crate::catalog::RoutineKind::Function { .. } => {
                    ("FN".to_string(), "SQL_SCALAR_FUNCTION".to_string())
                }
            };
            rows.push(StoredRow {
                values: vec![
                    Value::Int(r.object_id),
                    Value::Int(schema_id as i32),
                    Value::VarChar(r.name.clone()),
                    Value::Char(ty),
                    Value::VarChar(desc),
                ],
                deleted: false,
            });
        }
        rows
    }
}

impl VirtualTable for SysProcedures {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "procedures",
            vec![
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("object_id", DataType::Int, false),
                ("schema_id", DataType::Int, false),
                ("type", DataType::Char { len: 2 }, false),
                ("type_desc", DataType::VarChar { max_len: 60 }, false),
                ("create_date", DataType::DateTime, false),
                ("modify_date", DataType::DateTime, false),
                ("is_ms_shipped", DataType::Bit, false),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, ctx: &ExecutionContext) -> Vec<StoredRow> {
        let active_db = crate::executor::database_catalog::current_database_name(ctx);
        let created = Value::DateTime(
            chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        );
        catalog
            .get_routines()
            .iter()
            .filter(|r| {
                matches!(r.kind, crate::catalog::RoutineKind::Procedure { .. })
                    && crate::executor::database_catalog::schema_belongs_to_db(
                        &r.schema, active_db, catalog,
                    )
            })
            .map(|r| {
                let schema_id = catalog.get_schema_id(&r.schema).unwrap_or(1);
                StoredRow {
                    values: vec![
                        Value::VarChar(r.name.clone()),
                        Value::Int(r.object_id),
                        Value::Int(schema_id as i32),
                        Value::Char("P ".to_string()),
                        Value::VarChar("SQL_STORED_PROCEDURE".to_string()),
                        created.clone(),
                        created.clone(),
                        Value::Bit(false),
                    ],
                    deleted: false,
                }
            })
            .collect()
    }
}

impl VirtualTable for SysFunctions {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "functions",
            vec![
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("object_id", DataType::Int, false),
                ("schema_id", DataType::Int, false),
                ("type", DataType::Char { len: 2 }, false),
                ("type_desc", DataType::VarChar { max_len: 60 }, false),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, ctx: &ExecutionContext) -> Vec<StoredRow> {
        let active_db = crate::executor::database_catalog::current_database_name(ctx);
        catalog
            .get_routines()
            .iter()
            .filter(|r| {
                matches!(r.kind, crate::catalog::RoutineKind::Function { .. })
                    && crate::executor::database_catalog::schema_belongs_to_db(
                        &r.schema, active_db, catalog,
                    )
            })
            .map(|r| {
                let schema_id = catalog.get_schema_id(&r.schema).unwrap_or(1);
                let (ty, desc) = match &r.kind {
                    crate::catalog::RoutineKind::Function { body, .. }
                        if matches!(body.as_ref(), crate::ast::FunctionBody::InlineTable(_)) =>
                    {
                        (
                            "IF".to_string(),
                            "SQL_INLINE_TABLE_VALUED_FUNCTION".to_string(),
                        )
                    }
                    _ => ("FN".to_string(), "SQL_SCALAR_FUNCTION".to_string()),
                };
                StoredRow {
                    values: vec![
                        Value::VarChar(r.name.clone()),
                        Value::Int(r.object_id),
                        Value::Int(schema_id as i32),
                        Value::Char(ty),
                        Value::VarChar(desc),
                    ],
                    deleted: false,
                }
            })
            .collect()
    }
}
