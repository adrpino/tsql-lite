use super::super::virtual_table_def;
use super::super::VirtualTable;
use crate::catalog::Catalog;
use crate::executor::context::ExecutionContext;
use crate::storage::StoredRow;
use crate::types::{DataType, Value};

pub(crate) struct SysDatabasePrincipals;
pub(crate) struct SysDatabasePermissions;
pub(crate) struct SysDatabaseRoleMembers;
pub(crate) struct SysSecurityPolicies;
pub(crate) struct SysSecurityPredicates;

impl VirtualTable for SysDatabasePrincipals {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "database_principals",
            vec![
                ("principal_id", DataType::Int, false),
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("type", DataType::Char { len: 2 }, false),
                ("type_desc", DataType::VarChar { max_len: 60 }, false),
                (
                    "default_schema_name",
                    DataType::VarChar { max_len: 128 },
                    true,
                ),
                ("create_date", DataType::DateTime, false),
                ("modify_date", DataType::DateTime, false),
                ("owning_principal_id", DataType::Int, true),
                ("sid", DataType::VarBinary { max_len: 85 }, true),
                ("is_fixed_role", DataType::Bit, false),
                ("authentication_type", DataType::Int, false),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let created = Value::DateTime(
            chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        );

        vec![
            StoredRow {
                values: vec![
                    Value::Int(1),
                    Value::VarChar("dbo".to_string()),
                    Value::Char("S".to_string()),
                    Value::VarChar("SQL_USER".to_string()),
                    Value::VarChar("dbo".to_string()),
                    created.clone(),
                    created.clone(),
                    Value::Null,
                    Value::VarBinary(vec![0x01]),
                    Value::Bit(false),
                    Value::Int(1),
                ],
                deleted: false,
            },
            StoredRow {
                values: vec![
                    Value::Int(2),
                    Value::VarChar("guest".to_string()),
                    Value::Char("S".to_string()),
                    Value::VarChar("SQL_USER".to_string()),
                    Value::Null,
                    created.clone(),
                    created.clone(),
                    Value::Null,
                    Value::VarBinary(vec![0x00]),
                    Value::Bit(false),
                    Value::Int(0),
                ],
                deleted: false,
            },
        ]
    }
}

impl VirtualTable for SysDatabasePermissions {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "database_permissions",
            vec![
                ("class", DataType::Int, false),
                ("class_desc", DataType::VarChar { max_len: 60 }, false),
                ("major_id", DataType::Int, false),
                ("minor_id", DataType::Int, false),
                ("grantee_principal_id", DataType::Int, false),
                ("grantor_principal_id", DataType::Int, false),
                ("type", DataType::VarChar { max_len: 60 }, false),
                ("permission_name", DataType::VarChar { max_len: 128 }, false),
                ("state", DataType::Char { len: 2 }, false),
                ("state_desc", DataType::VarChar { max_len: 60 }, false),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        vec![]
    }
}

impl VirtualTable for SysDatabaseRoleMembers {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "database_role_members",
            vec![
                ("role_principal_id", DataType::Int, false),
                ("member_principal_id", DataType::Int, false),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        vec![]
    }
}

impl VirtualTable for SysSecurityPolicies {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "security_policies",
            vec![
                ("object_id", DataType::Int, false),
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("schema_id", DataType::Int, false),
                ("is_enabled", DataType::Bit, false),
                ("is_schema_bound", DataType::Bit, false),
                ("is_ms_shipped", DataType::Bit, false),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        vec![]
    }
}

impl VirtualTable for SysSecurityPredicates {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "security_predicates",
            vec![
                ("object_id", DataType::Int, false),
                ("security_predicate_id", DataType::Int, false),
                ("target_object_id", DataType::Int, false),
                (
                    "predicate_definition",
                    DataType::NVarChar { max_len: 0 },
                    false,
                ),
                ("predicate_type", DataType::Int, false),
                (
                    "predicate_type_desc",
                    DataType::VarChar { max_len: 60 },
                    false,
                ),
                ("operation", DataType::Int, false),
                ("operation_desc", DataType::VarChar { max_len: 60 }, false),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        vec![]
    }
}
