use super::super::virtual_table_def;
use super::super::VirtualTable;
use crate::catalog::Catalog;
use crate::executor::context::ExecutionContext;
use crate::storage::StoredRow;
use crate::types::{DataType, Value};

pub(crate) struct SysDatabasePrincipals;
pub(crate) struct SysDatabasePermissions;
pub(crate) struct SysServerPermissions;
pub(crate) struct SysFnBuiltinPermissions;
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

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let created = Value::DateTime(
            chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        );

        let mut rows = vec![
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
        ];

        for p in catalog.get_principals() {
            let auth_type = if p.principal_type == "R" { 0 } else { 1 };
            rows.push(StoredRow {
                values: vec![
                    Value::Int(p.principal_id),
                    Value::VarChar(p.name.clone()),
                    Value::Char(p.principal_type.clone()),
                    Value::VarChar(p.type_desc.clone()),
                    match &p.default_schema_name {
                        Some(s) => Value::VarChar(s.clone()),
                        None => Value::Null,
                    },
                    created.clone(),
                    created.clone(),
                    Value::Int(1),
                    Value::Null,
                    Value::Bit(p.is_fixed_role),
                    Value::Int(auth_type),
                ],
                deleted: false,
            });
        }

        rows
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

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        catalog
            .get_permissions()
            .iter()
            .map(|p| StoredRow {
                values: vec![
                    Value::Int(p.class),
                    Value::VarChar(p.class_desc.clone()),
                    Value::Int(p.major_id),
                    Value::Int(p.minor_id),
                    Value::Int(p.grantee_principal_id),
                    Value::Int(p.grantor_principal_id),
                    Value::VarChar(p.perm_type.clone()),
                    Value::VarChar(p.permission_name.clone()),
                    Value::Char(p.state.clone()),
                    Value::VarChar(p.state_desc.clone()),
                ],
                deleted: false,
            })
            .collect()
    }
}

impl VirtualTable for SysServerPermissions {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "server_permissions",
            vec![
                ("class", DataType::TinyInt, false),
                ("class_desc", DataType::NVarChar { max_len: 60 }, true),
                ("major_id", DataType::Int, false),
                ("minor_id", DataType::Int, false),
                ("grantee_principal_id", DataType::Int, false),
                ("grantor_principal_id", DataType::Int, false),
                ("type", DataType::Char { len: 4 }, false),
                ("permission_name", DataType::NVarChar { max_len: 128 }, true),
                ("state", DataType::Char { len: 1 }, false),
                ("state_desc", DataType::NVarChar { max_len: 60 }, true),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        vec![StoredRow {
            values: vec![
                Value::TinyInt(100),
                Value::NVarChar("SERVER".to_string()),
                Value::Int(0),
                Value::Int(0),
                Value::Int(1),
                Value::Int(1),
                Value::Char("CL  ".to_string()),
                Value::NVarChar("CONTROL SERVER".to_string()),
                Value::Char("G".to_string()),
                Value::NVarChar("GRANT".to_string()),
            ],
            deleted: false,
        }]
    }
}

impl VirtualTable for SysFnBuiltinPermissions {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "fn_builtin_permissions",
            vec![
                ("class_desc", DataType::NVarChar { max_len: 60 }, false),
                (
                    "permission_name",
                    DataType::NVarChar { max_len: 128 },
                    false,
                ),
                ("type", DataType::Char { len: 4 }, false),
                (
                    "covering_permission_name",
                    DataType::NVarChar { max_len: 128 },
                    true,
                ),
                (
                    "parent_class_desc",
                    DataType::NVarChar { max_len: 60 },
                    true,
                ),
                (
                    "parent_covering_permission_name",
                    DataType::NVarChar { max_len: 128 },
                    true,
                ),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let perms: &[(&str, &str, &str)] = &[
            ("SERVER", "CONTROL SERVER", "CL  "),
            ("SERVER", "VIEW ANY DEFINITION", "VWAD"),
            ("SERVER", "VIEW SERVER STATE", "VWSS"),
            ("DATABASE", "CONTROL", "CL  "),
            ("DATABASE", "ALTER", "AL  "),
            ("DATABASE", "VIEW DEFINITION", "VW  "),
            ("DATABASE", "VIEW DATABASE STATE", "VWDS"),
            ("DATABASE", "UNMASK", "UMSK"),
            ("OBJECT", "SELECT", "SL  "),
            ("OBJECT", "INSERT", "IN  "),
            ("OBJECT", "UPDATE", "UP  "),
            ("OBJECT", "DELETE", "DL  "),
            ("OBJECT", "EXECUTE", "EX  "),
            ("OBJECT", "REFERENCES", "RF  "),
        ];
        perms
            .iter()
            .map(|(cls, name, ty)| StoredRow {
                values: vec![
                    Value::NVarChar((*cls).to_string()),
                    Value::NVarChar((*name).to_string()),
                    Value::Char((*ty).to_string()),
                    Value::Null,
                    Value::Null,
                    Value::Null,
                ],
                deleted: false,
            })
            .collect()
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

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        catalog
            .get_role_members()
            .iter()
            .map(|m| StoredRow {
                values: vec![
                    Value::Int(m.role_principal_id),
                    Value::Int(m.member_principal_id),
                ],
                deleted: false,
            })
            .collect()
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

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        catalog
            .get_security_policies()
            .iter()
            .map(|p| StoredRow {
                values: vec![
                    Value::Int(p.object_id),
                    Value::VarChar(p.name.clone()),
                    Value::Int(p.schema_id as i32),
                    Value::Bit(p.is_enabled),
                    Value::Bit(p.is_schema_bound),
                    Value::Bit(false),
                ],
                deleted: false,
            })
            .collect()
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

    fn rows(&self, catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        let mut out = Vec::new();
        for p in catalog.get_security_policies() {
            for pred in &p.predicates {
                let target_id = catalog
                    .find_table(&pred.target_schema, &pred.target_table)
                    .map(|t| t.id as i32)
                    .unwrap_or(0);
                let args = pred
                    .function_args
                    .iter()
                    .map(|a| format!("[{a}]"))
                    .collect::<Vec<_>>()
                    .join(",");
                let def = format!(
                    "([{}].[{}]({}))",
                    pred.function_schema, pred.function_name, args
                );
                let (ptype, pdesc) = match pred.kind {
                    crate::catalog::SecurityPredicateKind::Filter => (0, "FILTER"),
                    crate::catalog::SecurityPredicateKind::Block => (1, "BLOCK"),
                };
                let op = pred
                    .block_op
                    .unwrap_or(crate::catalog::SecurityBlockOp::All);
                out.push(StoredRow {
                    values: vec![
                        Value::Int(p.object_id),
                        Value::Int(pred.predicate_id),
                        Value::Int(target_id),
                        Value::NVarChar(def),
                        Value::Int(ptype),
                        Value::VarChar(pdesc.to_string()),
                        Value::Int(op.op_code()),
                        Value::VarChar(op.op_desc().to_string()),
                    ],
                    deleted: false,
                });
            }
        }
        out
    }
}
