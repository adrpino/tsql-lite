use super::super::super::VirtualTable;
use super::super::super::{builtin_types_rows, virtual_table_def};
use crate::catalog::Catalog;
use crate::executor::context::ExecutionContext;
use crate::storage::StoredRow;
use crate::types::{DataType, Value};

pub(crate) struct SysTypes;
pub(crate) struct SysTableTypes;
pub(crate) struct SysAssemblyTypes;

impl VirtualTable for SysTypes {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "types",
            vec![
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("system_type_id", DataType::TinyInt, false),
                ("user_type_id", DataType::Int, false),
                ("schema_id", DataType::Int, false),
                ("max_length", DataType::SmallInt, false),
                ("precision", DataType::TinyInt, false),
                ("scale", DataType::TinyInt, false),
                ("is_user_defined", DataType::Bit, false),
                ("is_assembly_type", DataType::Bit, false),
                ("is_nullable", DataType::Bit, true),
                ("is_table_type", DataType::Bit, false),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let mut rows: Vec<StoredRow> = builtin_types_rows()
            .into_iter()
            .map(|row| {
                let id = match row.values.first() {
                    Some(Value::Int(v)) => *v,
                    _ => 0,
                };
                let name = row
                    .values
                    .get(1)
                    .cloned()
                    .unwrap_or(Value::VarChar(String::new()));
                let max_length = row.values.get(2).cloned().unwrap_or(Value::SmallInt(0));
                let precision = row.values.get(3).cloned().unwrap_or(Value::TinyInt(0));
                let scale = row.values.get(4).cloned().unwrap_or(Value::TinyInt(0));
                StoredRow {
                    values: vec![
                        name,
                        Value::TinyInt(id as u8),
                        Value::Int(id),
                        Value::Int(4), // sys schema
                        max_length,
                        precision,
                        scale,
                        Value::Bit(false),
                        Value::Bit(false),
                        Value::Bit(true),
                        Value::Bit(false),
                    ],
                    deleted: false,
                }
            })
            .collect();

        for tt in catalog.get_table_types() {
            let schema_id = catalog.get_schema_id(&tt.schema).unwrap_or(1);
            rows.push(StoredRow {
                values: vec![
                    Value::VarChar(tt.name.clone()),
                    Value::TinyInt(243), // table type
                    Value::Int(tt.object_id),
                    Value::Int(schema_id as i32),
                    Value::SmallInt(-1),
                    Value::TinyInt(0),
                    Value::TinyInt(0),
                    Value::Bit(true),
                    Value::Bit(false),
                    Value::Bit(false),
                    Value::Bit(true),
                ],
                deleted: false,
            });
        }
        for st in catalog.get_scalar_types() {
            let schema_id = catalog.get_schema_id(&st.schema).unwrap_or(1);
            let (sys_id, max_len, prec, scale) = scalar_type_sys_meta(&st.base_type);
            rows.push(StoredRow {
                values: vec![
                    Value::VarChar(st.name.clone()),
                    Value::TinyInt(sys_id),
                    Value::Int(st.object_id),
                    Value::Int(schema_id as i32),
                    Value::SmallInt(max_len),
                    Value::TinyInt(prec),
                    Value::TinyInt(scale),
                    Value::Bit(true),
                    Value::Bit(false),
                    Value::Bit(st.nullable),
                    Value::Bit(false),
                ],
                deleted: false,
            });
        }
        rows
    }
}

fn scalar_type_sys_meta(dt: &DataType) -> (u8, i16, u8, u8) {
    match dt {
        DataType::Bit => (104, 1, 1, 0),
        DataType::TinyInt => (48, 1, 3, 0),
        DataType::SmallInt => (52, 2, 5, 0),
        DataType::Int => (56, 4, 10, 0),
        DataType::BigInt => (127, 8, 19, 0),
        DataType::Float => (62, 8, 53, 0),
        DataType::Decimal { precision, scale } => (106, 17, *precision, *scale),
        DataType::Money => (60, 8, 19, 4),
        DataType::SmallMoney => (122, 4, 10, 4),
        DataType::Char { len } => (175, *len as i16, 0, 0),
        DataType::VarChar { max_len } => (
            167,
            if *max_len == u16::MAX {
                -1
            } else {
                *max_len as i16
            },
            0,
            0,
        ),
        DataType::NChar { len } => (239, (*len as i16).saturating_mul(2), 0, 0),
        DataType::NVarChar { max_len } => (
            231,
            if *max_len == u16::MAX {
                -1
            } else {
                (*max_len as i16).saturating_mul(2)
            },
            0,
            0,
        ),
        DataType::Binary { len } => (173, *len as i16, 0, 0),
        DataType::VarBinary { max_len } => (
            165,
            if *max_len == u16::MAX {
                -1
            } else {
                *max_len as i16
            },
            0,
            0,
        ),
        DataType::Date => (40, 3, 10, 0),
        DataType::Time => (41, 5, 16, 7),
        DataType::DateTime => (61, 8, 23, 3),
        DataType::DateTime2 => (42, 8, 27, 7),
        DataType::SmallDateTime => (58, 4, 16, 0),
        DataType::DateTimeOffset => (43, 10, 34, 7),
        DataType::UniqueIdentifier => (36, 16, 0, 0),
        DataType::SqlVariant => (98, 8016, 0, 0),
        DataType::Xml => (241, -1, 0, 0),
        DataType::Vector { dimensions } => (244, (*dimensions as i16).saturating_mul(4), 0, 0),
    }
}

impl VirtualTable for SysTableTypes {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "table_types",
            vec![
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("user_type_id", DataType::Int, false),
                ("system_type_id", DataType::TinyInt, false),
                ("schema_id", DataType::Int, false),
                ("type_table_object_id", DataType::Int, false),
                ("is_memory_optimized", DataType::Bit, false),
                ("is_user_defined", DataType::Bit, false),
                ("is_table_type", DataType::Bit, false),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        catalog
            .get_table_types()
            .iter()
            .map(|tt| {
                let schema_id = catalog.get_schema_id(&tt.schema).unwrap_or(1);
                StoredRow {
                    values: vec![
                        Value::VarChar(tt.name.clone()),
                        Value::Int(tt.object_id),
                        Value::TinyInt(243), // table type
                        Value::Int(schema_id as i32),
                        Value::Int(tt.object_id),
                        Value::Bit(false),
                        Value::Bit(true),
                        Value::Bit(true),
                    ],
                    deleted: false,
                }
            })
            .collect()
    }
}

impl VirtualTable for SysAssemblyTypes {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "assembly_types",
            vec![
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("system_type_id", DataType::TinyInt, false),
                ("user_type_id", DataType::Int, false),
                ("schema_id", DataType::Int, false),
                ("principal_id", DataType::Int, true),
                ("assembly_id", DataType::Int, false),
                ("assembly_class", DataType::NVarChar { max_len: 128 }, true),
                ("is_binary_ordered", DataType::Bit, false),
                ("is_fixed_length", DataType::Bit, false),
                ("max_length", DataType::SmallInt, false),
                ("precision", DataType::TinyInt, false),
                ("scale", DataType::TinyInt, false),
                ("collation_name", DataType::VarChar { max_len: 128 }, true),
                ("is_nullable", DataType::Bit, false),
                ("is_user_defined", DataType::Bit, false),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        Vec::new()
    }
}
