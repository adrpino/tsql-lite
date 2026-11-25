use super::super::virtual_table_def;
use super::super::VirtualTable;
use crate::catalog::Catalog;
use crate::executor::context::ExecutionContext;
use crate::storage::StoredRow;
use crate::types::{DataType, Value};

pub(crate) struct SysPartitionFunctions;
pub(crate) struct SysPartitionParameters;
pub(crate) struct SysPartitionRangeValues;
pub(crate) struct SysPartitionSchemes;
pub(crate) struct SysDestinationDataSpaces;
pub(crate) struct SysFilegroups;
pub(crate) struct SysPartitions;
pub(crate) struct SysAllocationUnits;

impl VirtualTable for SysPartitionFunctions {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "partition_functions",
            vec![
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("function_id", DataType::Int, false),
                ("type", DataType::TinyInt, false),
                ("type_desc", DataType::VarChar { max_len: 60 }, false),
                ("fanout", DataType::Int, false),
                ("boundary_value_on_right", DataType::Bit, false),
                ("create_date", DataType::DateTime, false),
                ("modify_date", DataType::DateTime, false),
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
        catalog
            .get_partition_functions()
            .iter()
            .map(|pf| StoredRow {
                values: vec![
                    Value::VarChar(pf.name.clone()),
                    Value::Int(pf.function_id),
                    Value::TinyInt(1),
                    Value::VarChar("RANGE".to_string()),
                    Value::Int((pf.boundary_values.len() + 1) as i32),
                    Value::Bit(pf.boundary_value_on_right),
                    created.clone(),
                    created.clone(),
                ],
                deleted: false,
            })
            .collect()
    }
}

impl VirtualTable for SysPartitionParameters {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "partition_parameters",
            vec![
                ("parameter_id", DataType::Int, false),
                ("function_id", DataType::Int, false),
                ("system_type_id", DataType::Int, false),
                ("user_type_id", DataType::Int, false),
                ("max_length", DataType::SmallInt, false),
                ("precision", DataType::TinyInt, false),
                ("scale", DataType::TinyInt, false),
                ("collation_name", DataType::VarChar { max_len: 128 }, true),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        catalog
            .get_partition_functions()
            .iter()
            .map(|pf| {
                let sys_id = super::super::system_type_id(&pf.data_type);
                let max_len = super::super::type_max_length(&pf.data_type);
                let (prec, scale) = match &pf.data_type {
                    DataType::Decimal { precision, scale } => (*precision, *scale),
                    DataType::Int => (10, 0),
                    DataType::BigInt => (19, 0),
                    DataType::SmallInt => (5, 0),
                    DataType::TinyInt => (3, 0),
                    _ => (0, 0),
                };
                StoredRow {
                    values: vec![
                        Value::Int(1),
                        Value::Int(pf.function_id),
                        Value::Int(sys_id),
                        Value::Int(sys_id),
                        Value::SmallInt(max_len),
                        Value::TinyInt(prec),
                        Value::TinyInt(scale),
                        Value::Null,
                    ],
                    deleted: false,
                }
            })
            .collect()
    }
}

impl VirtualTable for SysPartitionSchemes {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "partition_schemes",
            vec![
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("data_space_id", DataType::Int, false),
                ("function_id", DataType::Int, false),
                ("type", DataType::Char { len: 2 }, false),
                ("type_desc", DataType::VarChar { max_len: 60 }, false),
                ("create_date", DataType::DateTime, false),
                ("modify_date", DataType::DateTime, false),
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
        catalog
            .get_partition_schemes()
            .iter()
            .map(|ps| StoredRow {
                values: vec![
                    Value::VarChar(ps.name.clone()),
                    Value::Int(ps.data_space_id),
                    Value::Int(ps.function_id),
                    Value::Char("PS".to_string()),
                    Value::VarChar("PARTITION_SCHEME".to_string()),
                    created.clone(),
                    created.clone(),
                ],
                deleted: false,
            })
            .collect()
    }
}

impl VirtualTable for SysDestinationDataSpaces {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "destination_data_spaces",
            vec![
                ("partition_scheme_id", DataType::Int, false),
                ("destination_id", DataType::Int, false),
                ("data_space_id", DataType::Int, false),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let mut rows = Vec::new();
        for ps in catalog.get_partition_schemes() {
            for (idx, fg) in ps.filegroups.iter().enumerate() {
                rows.push(StoredRow {
                    values: vec![
                        Value::Int(ps.data_space_id),
                        Value::Int((idx + 1) as i32),
                        Value::Int(catalog.filegroup_data_space_id(fg)),
                    ],
                    deleted: false,
                });
            }
        }
        rows
    }
}

impl VirtualTable for SysFilegroups {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "filegroups",
            vec![
                ("data_space_id", DataType::Int, false),
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("type", DataType::Char { len: 2 }, false),
                ("type_desc", DataType::VarChar { max_len: 60 }, false),
                ("is_read_only", DataType::Bit, false),
                ("is_default", DataType::Bit, false),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let mut rows = vec![StoredRow {
            values: vec![
                Value::Int(1),
                Value::VarChar("PRIMARY".to_string()),
                Value::Char("FG".to_string()),
                Value::VarChar("ROWS_FILEGROUP".to_string()),
                Value::Bit(false),
                Value::Bit(true),
            ],
            deleted: false,
        }];
        for fg in catalog.get_filegroups() {
            rows.push(StoredRow {
                values: vec![
                    Value::Int(fg.data_space_id),
                    Value::VarChar(fg.name.clone()),
                    Value::Char("FG".to_string()),
                    Value::VarChar("ROWS_FILEGROUP".to_string()),
                    Value::Bit(fg.is_read_only),
                    Value::Bit(fg.is_default),
                ],
                deleted: false,
            });
        }
        rows
    }
}

impl VirtualTable for SysPartitions {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "partitions",
            vec![
                ("partition_id", DataType::BigInt, false),
                ("object_id", DataType::Int, false),
                ("index_id", DataType::Int, false),
                ("partition_number", DataType::Int, false),
                ("hobt_id", DataType::BigInt, false),
                ("rows", DataType::BigInt, false),
                ("filestream_filegroup_id", DataType::Int, false),
                ("data_compression", DataType::TinyInt, false),
                (
                    "data_compression_desc",
                    DataType::VarChar { max_len: 60 },
                    false,
                ),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let mut rows = Vec::new();

        for t in catalog.get_tables() {
            // For now, assume one partition per index
            let table_indexes: Vec<_> = catalog
                .get_indexes()
                .iter()
                .filter(|idx| idx.table_id == t.id)
                .collect();

            if table_indexes.is_empty() {
                // Heap
                let partition_id = 72057594040000000 + (t.id as i64);
                rows.push(StoredRow {
                    values: vec![
                        Value::BigInt(partition_id),
                        Value::Int(t.id as i32),
                        Value::Int(0), // Heap
                        Value::Int(1), // Partition 1
                        Value::BigInt(partition_id),
                        Value::BigInt(0), // rows
                        Value::Int(0),
                        Value::TinyInt(0), // NONE
                        Value::VarChar("NONE".to_string()),
                    ],
                    deleted: false,
                });
            } else {
                for idx in table_indexes {
                    let partition_id = 72057594040000000 + (idx.id as i64);
                    rows.push(StoredRow {
                        values: vec![
                            Value::BigInt(partition_id),
                            Value::Int(t.id as i32),
                            Value::Int(idx.id as i32),
                            Value::Int(1), // Partition 1
                            Value::BigInt(partition_id),
                            Value::BigInt(0), // rows
                            Value::Int(0),
                            Value::TinyInt(0), // NONE
                            Value::VarChar("NONE".to_string()),
                        ],
                        deleted: false,
                    });
                }
            }
        }

        rows
    }
}

impl VirtualTable for SysAllocationUnits {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "allocation_units",
            vec![
                ("allocation_unit_id", DataType::BigInt, false),
                ("type", DataType::TinyInt, false),
                ("type_desc", DataType::VarChar { max_len: 60 }, false),
                ("container_id", DataType::BigInt, false),
                ("data_space_id", DataType::Int, false),
                ("total_pages", DataType::BigInt, false),
                ("used_pages", DataType::BigInt, false),
                ("data_pages", DataType::BigInt, false),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let partitions = SysPartitions;
        let mut rows = Vec::new();

        for (au_id, p_row) in (72057594045000000i64..).zip(partitions.rows(catalog, _ctx)) {
            let container_id = match p_row.values[0] {
                Value::BigInt(v) => v,
                _ => 0,
            };

            rows.push(StoredRow {
                values: vec![
                    Value::BigInt(au_id),
                    Value::TinyInt(1), // IN_ROW_DATA
                    Value::VarChar("IN_ROW_DATA".to_string()),
                    Value::BigInt(container_id),
                    Value::Int(1), // PRIMARY filegroup
                    Value::BigInt(0),
                    Value::BigInt(0),
                    Value::BigInt(0),
                ],
                deleted: false,
            });
        }

        rows
    }
}

impl VirtualTable for SysPartitionRangeValues {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "partition_range_values",
            vec![
                ("function_id", DataType::Int, false),
                ("boundary_id", DataType::Int, false),
                ("parameter_id", DataType::Int, false),
                ("value", DataType::SqlVariant, true),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let mut rows = Vec::new();
        for pf in catalog.get_partition_functions() {
            for (idx, val_str) in pf.boundary_values.iter().enumerate() {
                rows.push(StoredRow {
                    values: vec![
                        Value::Int(pf.function_id),
                        Value::Int((idx + 1) as i32),
                        Value::Int(1),
                        Value::SqlVariant(Box::new(Value::NVarChar(val_str.clone()))),
                    ],
                    deleted: false,
                });
            }
        }
        rows
    }
}
