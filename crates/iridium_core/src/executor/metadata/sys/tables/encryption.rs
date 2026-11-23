use super::super::super::virtual_table_def;
use super::super::super::VirtualTable;
use crate::catalog::Catalog;
use crate::executor::context::ExecutionContext;
use crate::storage::StoredRow;
use crate::types::{DataType, Value};

pub(crate) struct SysColumnEncryptionKeys;
pub(crate) struct SysColumnMasterKeys;
pub(crate) struct SysColumnEncryptionKeyValues;

fn default_key_timestamp() -> Value {
    Value::DateTime(
        chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap(),
    )
}

impl VirtualTable for SysColumnEncryptionKeys {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "column_encryption_keys",
            vec![
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("column_encryption_key_id", DataType::Int, false),
                ("create_date", DataType::DateTime, false),
                ("modify_date", DataType::DateTime, false),
                ("is_ms_shipped", DataType::Bit, false),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let ts = default_key_timestamp();
        catalog
            .get_column_encryption_keys()
            .iter()
            .map(|k| StoredRow {
                values: vec![
                    Value::VarChar(k.name.clone()),
                    Value::Int(k.column_encryption_key_id),
                    ts.clone(),
                    ts.clone(),
                    Value::Bit(false),
                ],
                deleted: false,
            })
            .collect()
    }
}

impl VirtualTable for SysColumnMasterKeys {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "column_master_keys",
            vec![
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("column_master_key_id", DataType::Int, false),
                ("create_date", DataType::DateTime, false),
                ("modify_date", DataType::DateTime, false),
                (
                    "key_store_provider_name",
                    DataType::VarChar { max_len: 128 },
                    false,
                ),
                ("key_path", DataType::VarChar { max_len: 512 }, false),
                ("allow_enclave_computations", DataType::Bit, false),
                ("signature", DataType::VarBinary { max_len: 8000 }, true),
                ("is_ms_shipped", DataType::Bit, false),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let ts = default_key_timestamp();
        catalog
            .get_column_master_keys()
            .iter()
            .map(|k| StoredRow {
                values: vec![
                    Value::VarChar(k.name.clone()),
                    Value::Int(k.column_master_key_id),
                    ts.clone(),
                    ts.clone(),
                    Value::VarChar(k.key_store_provider_name.clone()),
                    Value::VarChar(k.key_path.clone()),
                    Value::Bit(k.allow_enclave_computations),
                    k.signature
                        .clone()
                        .map(Value::VarBinary)
                        .unwrap_or(Value::Null),
                    Value::Bit(false),
                ],
                deleted: false,
            })
            .collect()
    }
}

impl VirtualTable for SysColumnEncryptionKeyValues {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "column_encryption_key_values",
            vec![
                ("column_encryption_key_id", DataType::Int, false),
                ("column_master_key_id", DataType::Int, false),
                (
                    "encrypted_value",
                    DataType::VarBinary { max_len: 8000 },
                    false,
                ),
                (
                    "encryption_algorithm_name",
                    DataType::VarChar { max_len: 128 },
                    false,
                ),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let mut out = Vec::new();
        for cek in catalog.get_column_encryption_keys() {
            for v in &cek.values {
                out.push(StoredRow {
                    values: vec![
                        Value::Int(cek.column_encryption_key_id),
                        Value::Int(v.column_master_key_id),
                        Value::VarBinary(v.encrypted_value.clone()),
                        Value::VarChar(v.encryption_algorithm_name.clone()),
                    ],
                    deleted: false,
                });
            }
        }
        out
    }
}
