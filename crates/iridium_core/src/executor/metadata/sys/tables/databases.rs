use super::super::super::virtual_table_def;
use super::super::super::VirtualTable;
use crate::catalog::Catalog;
use crate::executor::context::ExecutionContext;
use crate::executor::database_catalog::all_databases;
use crate::storage::StoredRow;
use crate::types::{DataType, Value};

pub(crate) struct SysDatabases;
pub(crate) struct SysSysDatabases;
pub(crate) struct SysConfigurations;
pub(crate) struct SysConfigures;
pub(crate) struct SysDatabaseScopedConfigurations;

impl VirtualTable for SysDatabases {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "databases",
            vec![
                ("database_id", DataType::Int, false),
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("source_database_id", DataType::Int, true),
                ("owner_sid", DataType::VarBinary { max_len: 85 }, true),
                ("create_date", DataType::DateTime, false),
                ("compatibility_level", DataType::TinyInt, false),
                ("collation_name", DataType::VarChar { max_len: 128 }, false),
                ("state", DataType::TinyInt, false),
                ("state_desc", DataType::VarChar { max_len: 60 }, false),
                ("containment", DataType::TinyInt, false),
                ("user_access", DataType::TinyInt, false),
                ("user_access_desc", DataType::VarChar { max_len: 60 }, false),
                ("is_read_only", DataType::Bit, false),
                ("is_fulltext_enabled", DataType::Bit, false),
                ("is_ledger_on", DataType::Bit, false),
                ("recovery_model", DataType::TinyInt, false),
                (
                    "recovery_model_desc",
                    DataType::VarChar { max_len: 60 },
                    false,
                ),
                ("is_auto_close_on", DataType::Bit, false),
                ("is_auto_shrink_on", DataType::Bit, false),
                ("is_in_standby", DataType::Bit, false),
                ("is_distributor", DataType::Bit, false),
                ("is_cleanly_shutdown", DataType::Bit, false),
                ("snapshot_isolation_state", DataType::TinyInt, false),
                (
                    "snapshot_isolation_state_desc",
                    DataType::VarChar { max_len: 60 },
                    false,
                ),
                ("is_read_committed_snapshot_on", DataType::Bit, false),
                ("catalog_collation_type", DataType::TinyInt, false),
                (
                    "catalog_collation_type_desc",
                    DataType::VarChar { max_len: 60 },
                    false,
                ),
                ("is_query_store_on", DataType::Bit, false),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let created = Value::DateTime(
            chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        );
        all_databases(catalog)
            .into_iter()
            .map(|db| {
                let user_access_desc = match db.user_access {
                    1 => "SINGLE_USER",
                    2 => "RESTRICTED_USER",
                    _ => "MULTI_USER",
                };
                StoredRow {
                    values: vec![
                        Value::Int(db.id),
                        Value::VarChar(db.name.clone()),
                        db.source_database_id.map(Value::Int).unwrap_or(Value::Null),
                        Value::VarBinary(vec![0x01]),
                        created.clone(),
                        Value::TinyInt(db.compatibility_level),
                        Value::VarChar("SQL_Latin1_General_CP1_CI_AS".to_string()),
                        Value::TinyInt(0),
                        Value::VarChar("ONLINE".to_string()),
                        Value::TinyInt(0),
                        Value::TinyInt(db.user_access),
                        Value::VarChar(user_access_desc.to_string()),
                        Value::Bit(db.is_read_only),
                        Value::Bit(false),
                        Value::Bit(false),
                        Value::TinyInt(match db.recovery_model.as_str() {
                            "SIMPLE" => 3,
                            "BULK_LOGGED" => 2,
                            _ => 1,
                        }),
                        Value::VarChar(db.recovery_model.clone()),
                        Value::Bit(false),
                        Value::Bit(false),
                        Value::Bit(false),
                        Value::Bit(false),
                        Value::Bit(db.name.eq_ignore_ascii_case("master")),
                        Value::TinyInt(if db.allow_snapshot_isolation { 1 } else { 0 }),
                        Value::VarChar(
                            if db.allow_snapshot_isolation {
                                "ON"
                            } else {
                                "OFF"
                            }
                            .to_string(),
                        ),
                        Value::Bit(db.read_committed_snapshot),
                        Value::TinyInt(0),
                        Value::VarChar("DATABASE_DEFAULT".to_string()),
                        Value::Bit(false),
                    ],
                    deleted: false,
                }
            })
            .collect()
    }
}

impl VirtualTable for SysConfigurations {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "configurations",
            vec![
                ("configuration_id", DataType::Int, false),
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("value", DataType::Int, false),
                ("value_in_use", DataType::Int, false),
                ("minimum", DataType::Int, false),
                ("maximum", DataType::Int, false),
                ("is_dynamic", DataType::Bit, false),
                ("is_advanced", DataType::Bit, false),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        vec![StoredRow {
            // configuration_id=16384 used by SSMS probe (contained db auth).
            values: vec![
                Value::Int(16384),
                Value::VarChar("contained database authentication".to_string()),
                Value::Int(0),
                Value::Int(0),
                Value::Int(0),
                Value::Int(1),
                Value::Bit(true),
                Value::Bit(true),
            ],
            deleted: false,
        }]
    }
}

impl VirtualTable for SysSysDatabases {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "sysdatabases",
            vec![
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("dbid", DataType::SmallInt, false),
                ("sid", DataType::VarBinary { max_len: 85 }, true),
                ("mode", DataType::SmallInt, false),
                ("status", DataType::Int, false),
                ("status2", DataType::Int, false),
                ("crdate", DataType::DateTime, false),
                ("cmptlevel", DataType::TinyInt, false),
                ("filename", DataType::VarChar { max_len: 260 }, true),
                ("version", DataType::Int, false),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let created = Value::DateTime(
            chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        );
        all_databases(catalog)
            .into_iter()
            .map(|db| StoredRow {
                values: vec![
                    Value::VarChar(db.name),
                    Value::SmallInt(db.id as i16),
                    Value::VarBinary(vec![0x01]),
                    Value::SmallInt(0),
                    Value::Int(0),
                    Value::Int(0),
                    created.clone(),
                    Value::TinyInt(db.compatibility_level),
                    Value::Null,
                    Value::Int(0),
                ],
                deleted: false,
            })
            .collect()
    }
}

impl VirtualTable for SysConfigures {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "sysconfigures",
            vec![
                ("value", DataType::Int, true),
                ("config", DataType::Int, false),
                ("comment", DataType::NVarChar { max_len: 255 }, false),
                ("status", DataType::SmallInt, true),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let entries: &[(i32, i32, &str, i16)] = &[
            (1033, 1126, "default full-text language", 2),
            (1033, 124, "default language", 2),
            (0, 16384, "contained database authentication", 3),
        ];
        entries
            .iter()
            .map(|(val, cfg, comment, status)| StoredRow {
                values: vec![
                    Value::Int(*val),
                    Value::Int(*cfg),
                    Value::NVarChar((*comment).to_string()),
                    Value::SmallInt(*status),
                ],
                deleted: false,
            })
            .collect()
    }
}

impl VirtualTable for SysDatabaseScopedConfigurations {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "database_scoped_configurations",
            vec![
                ("configuration_id", DataType::Int, false),
                ("name", DataType::NVarChar { max_len: 60 }, false),
                ("value", DataType::NVarChar { max_len: 128 }, true),
                (
                    "value_for_secondary",
                    DataType::NVarChar { max_len: 128 },
                    true,
                ),
                ("is_value_default", DataType::Bit, false),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let entries: &[(i32, &str, &str)] = &[
            (1, "MAXDOP", "0"),
            (2, "LEGACY_CARDINALITY_ESTIMATION", "0"),
            (3, "PARAMETER_SNIFFING", "1"),
            (4, "QUERY_OPTIMIZER_HOTFIXES", "0"),
        ];
        entries
            .iter()
            .map(|(id, name, val)| StoredRow {
                values: vec![
                    Value::Int(*id),
                    Value::NVarChar((*name).to_string()),
                    Value::NVarChar((*val).to_string()),
                    Value::Null,
                    Value::Bit(true),
                ],
                deleted: false,
            })
            .collect()
    }
}
