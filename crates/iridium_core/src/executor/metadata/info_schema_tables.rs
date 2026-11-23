use super::VirtualTable;
use super::{schema_name_by_id, virtual_table_def, DB_CATALOG};
use crate::catalog::Catalog;
use crate::executor::context::ExecutionContext;
use crate::storage::StoredRow;
use crate::types::{DataType, Value};

pub(super) fn lookup(name: &str) -> Option<Box<dyn VirtualTable>> {
    match name {
        n if n.eq_ignore_ascii_case("SCHEMATA") => Some(Box::new(Schemata)),
        n if n.eq_ignore_ascii_case("TABLES") => Some(Box::new(Tables)),
        n if n.eq_ignore_ascii_case("VIEWS") => Some(Box::new(Views)),
        _ => None,
    }
}

struct Schemata;
struct Tables;
struct Views;

impl VirtualTable for Schemata {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "SCHEMATA",
            vec![
                ("CATALOG_NAME", DataType::VarChar { max_len: 128 }, false),
                ("SCHEMA_NAME", DataType::VarChar { max_len: 128 }, false),
                ("SCHEMA_OWNER", DataType::VarChar { max_len: 128 }, true),
                (
                    "DEFAULT_CHARACTER_SET_CATALOG",
                    DataType::VarChar { max_len: 6 },
                    true,
                ),
                (
                    "DEFAULT_CHARACTER_SET_SCHEMA",
                    DataType::VarChar { max_len: 3 },
                    true,
                ),
                (
                    "DEFAULT_CHARACTER_SET_NAME",
                    DataType::VarChar { max_len: 128 },
                    true,
                ),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, ctx: &ExecutionContext) -> Vec<StoredRow> {
        let active_db = crate::executor::database_catalog::current_database_name(ctx);
        let catalog_name =
            if crate::executor::database_catalog::is_user_database(catalog, active_db) {
                active_db
            } else {
                DB_CATALOG
            };
        catalog
            .get_schemas()
            .iter()
            .filter(|s| {
                crate::executor::database_catalog::schema_belongs_to_db(&s.name, active_db, catalog)
            })
            .map(|s| StoredRow {
                values: vec![
                    Value::VarChar(catalog_name.to_string()),
                    Value::VarChar(
                        crate::executor::database_catalog::display_schema_name(&s.name).to_string(),
                    ),
                    Value::VarChar("dbo".to_string()),
                    Value::Null,
                    Value::Null,
                    Value::VarChar("iso_1".to_string()),
                ],
                deleted: false,
            })
            .collect()
    }
}

impl VirtualTable for Tables {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "TABLES",
            vec![
                ("TABLE_CATALOG", DataType::VarChar { max_len: 128 }, false),
                ("TABLE_SCHEMA", DataType::VarChar { max_len: 128 }, false),
                ("TABLE_NAME", DataType::VarChar { max_len: 128 }, false),
                ("TABLE_TYPE", DataType::VarChar { max_len: 10 }, false),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, ctx: &ExecutionContext) -> Vec<StoredRow> {
        let active_db = crate::executor::database_catalog::current_database_name(ctx);
        let catalog_name =
            if crate::executor::database_catalog::is_user_database(catalog, active_db) {
                active_db
            } else {
                DB_CATALOG
            };
        let mut rows: Vec<StoredRow> = catalog
            .get_tables()
            .iter()
            .filter(|t| {
                crate::executor::database_catalog::schema_belongs_to_db(
                    &t.schema_name,
                    active_db,
                    catalog,
                )
            })
            .map(|t| {
                let raw_schema = schema_name_by_id(catalog, t.schema_id);
                let disp_schema =
                    crate::executor::database_catalog::display_schema_name(&raw_schema).to_string();
                StoredRow {
                    values: vec![
                        Value::VarChar(catalog_name.to_string()),
                        Value::VarChar(disp_schema),
                        Value::VarChar(t.name.clone()),
                        Value::VarChar("BASE TABLE".to_string()),
                    ],
                    deleted: false,
                }
            })
            .collect();
        for v in catalog.get_views() {
            if !crate::executor::database_catalog::schema_belongs_to_db(
                &v.schema, active_db, catalog,
            ) {
                continue;
            }
            let disp_schema =
                crate::executor::database_catalog::display_schema_name(&v.schema).to_string();
            rows.push(StoredRow {
                values: vec![
                    Value::VarChar(catalog_name.to_string()),
                    Value::VarChar(disp_schema),
                    Value::VarChar(v.name.clone()),
                    Value::VarChar("VIEW".to_string()),
                ],
                deleted: false,
            });
        }
        rows
    }
}

impl VirtualTable for Views {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "VIEWS",
            vec![
                ("TABLE_CATALOG", DataType::VarChar { max_len: 128 }, false),
                ("TABLE_SCHEMA", DataType::VarChar { max_len: 128 }, false),
                ("TABLE_NAME", DataType::VarChar { max_len: 128 }, false),
                ("VIEW_DEFINITION", DataType::VarChar { max_len: 128 }, true),
                ("CHECK_OPTION", DataType::VarChar { max_len: 7 }, false),
                ("IS_UPDATABLE", DataType::VarChar { max_len: 2 }, false),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, ctx: &ExecutionContext) -> Vec<StoredRow> {
        let active_db = crate::executor::database_catalog::current_database_name(ctx);
        let catalog_name =
            if crate::executor::database_catalog::is_user_database(catalog, active_db) {
                active_db
            } else {
                DB_CATALOG
            };
        catalog
            .get_views()
            .iter()
            .filter(|v| {
                crate::executor::database_catalog::schema_belongs_to_db(
                    &v.schema, active_db, catalog,
                )
            })
            .map(|v| {
                let disp_schema =
                    crate::executor::database_catalog::display_schema_name(&v.schema).to_string();
                StoredRow {
                    values: vec![
                        Value::VarChar(catalog_name.to_string()),
                        Value::VarChar(disp_schema),
                        Value::VarChar(v.name.clone()),
                        Value::Null,
                        Value::VarChar("NONE".to_string()),
                        Value::VarChar("NO".to_string()),
                    ],
                    deleted: false,
                }
            })
            .collect()
    }
}
