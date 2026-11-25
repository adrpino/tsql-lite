use super::*;
use crate::error::DbError;
use crate::executor::string_norm::normalize_identifier;
use crate::types::DataType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrincipalDef {
    pub principal_id: i32,
    pub name: String,
    /// SQL Server single-char type: `"S"` (SQL_USER), `"R"` (DATABASE_ROLE), `"U"` (WINDOWS_USER)
    pub principal_type: String,
    pub type_desc: String,
    pub default_schema_name: Option<String>,
    pub is_fixed_role: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleMemberDef {
    pub role_principal_id: i32,
    pub member_principal_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionDef {
    pub class: i32,
    pub class_desc: String,
    pub major_id: i32,
    pub minor_id: i32,
    pub grantee_principal_id: i32,
    pub grantor_principal_id: i32,
    pub perm_type: String,
    pub permission_name: String,
    /// `"G"` (GRANT), `"W"` (GRANT_WITH_GRANT_OPTION), `"D"` (DENY)
    pub state: String,
    pub state_desc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionFunctionDef {
    pub function_id: i32,
    pub name: String,
    pub data_type: DataType,
    pub boundary_value_on_right: bool,
    pub boundary_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionSchemeDef {
    pub data_space_id: i32,
    pub name: String,
    pub function_id: i32,
    pub filegroups: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilegroupDef {
    pub data_space_id: i32,
    pub name: String,
    pub is_default: bool,
    pub is_read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserDatabaseDef {
    pub id: i32,
    pub name: String,
    pub compatibility_level: u8,
    pub recovery_model: String,
    #[serde(default)]
    pub user_access: u8,
    #[serde(default)]
    pub is_read_only: bool,
    #[serde(default)]
    pub read_committed_snapshot: bool,
    #[serde(default)]
    pub allow_snapshot_isolation: bool,
    #[serde(default)]
    pub source_database_id: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseRuntimeOptions {
    pub user_access: u8,
    pub is_read_only: bool,
    pub read_committed_snapshot: bool,
    pub allow_snapshot_isolation: bool,
    pub source_database_id: Option<i32>,
    pub recovery_model: String,
    pub compatibility_level: u8,
}

impl Default for DatabaseRuntimeOptions {
    fn default() -> Self {
        Self {
            user_access: 0,
            is_read_only: false,
            read_committed_snapshot: false,
            allow_snapshot_isolation: false,
            source_database_id: None,
            recovery_model: "FULL".to_string(),
            compatibility_level: 160,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ColumnMasterKeyDef {
    pub column_master_key_id: i32,
    pub name: String,
    pub key_store_provider_name: String,
    pub key_path: String,
    pub allow_enclave_computations: bool,
    pub signature: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ColumnEncryptionKeyValueDef {
    pub column_master_key_id: i32,
    pub encrypted_value: Vec<u8>,
    pub encryption_algorithm_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ColumnEncryptionKeyDef {
    pub column_encryption_key_id: i32,
    pub name: String,
    pub values: Vec<ColumnEncryptionKeyValueDef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MaskedColumnDef {
    pub table_id: u32,
    pub column_name: String,
    pub masking_function: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SecurityPredicateKind {
    Filter,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SecurityBlockOp {
    /// Applies to all DML writes when no explicit operation is specified.
    All,
    AfterInsert,
    AfterUpdate,
    BeforeUpdate,
    BeforeDelete,
}

impl SecurityBlockOp {
    pub fn op_code(self) -> i32 {
        match self {
            SecurityBlockOp::All => 0,
            SecurityBlockOp::AfterInsert => 1,
            SecurityBlockOp::AfterUpdate => 2,
            SecurityBlockOp::BeforeUpdate => 3,
            SecurityBlockOp::BeforeDelete => 4,
        }
    }

    pub fn op_desc(self) -> &'static str {
        match self {
            SecurityBlockOp::All => "",
            SecurityBlockOp::AfterInsert => "AFTER INSERT",
            SecurityBlockOp::AfterUpdate => "AFTER UPDATE",
            SecurityBlockOp::BeforeUpdate => "BEFORE UPDATE",
            SecurityBlockOp::BeforeDelete => "BEFORE DELETE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SecurityPredicateDef {
    pub predicate_id: i32,
    pub kind: SecurityPredicateKind,
    pub block_op: Option<SecurityBlockOp>,
    /// Predicate TVF schema & name (e.g. `dbo.fn_securitypredicate`).
    pub function_schema: String,
    pub function_name: String,
    /// Column argument names passed from the target table to the predicate TVF.
    pub function_args: Vec<String>,
    /// Target table schema & name.
    pub target_schema: String,
    pub target_table: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SecurityPolicyDef {
    pub object_id: i32,
    pub schema_id: u32,
    pub schema: String,
    pub name: String,
    pub is_enabled: bool,
    pub is_schema_bound: bool,
    pub predicates: Vec<SecurityPredicateDef>,
}

/// Built-in SQL Server database principals (always present even before user DDL).
pub(crate) fn builtin_principals() -> Vec<PrincipalDef> {
    vec![
        PrincipalDef {
            principal_id: 0,
            name: "public".to_string(),
            principal_type: "R".to_string(),
            type_desc: "DATABASE_ROLE".to_string(),
            default_schema_name: None,
            is_fixed_role: false,
        },
        PrincipalDef {
            principal_id: 1,
            name: "dbo".to_string(),
            principal_type: "S".to_string(),
            type_desc: "SQL_USER".to_string(),
            default_schema_name: Some("dbo".to_string()),
            is_fixed_role: false,
        },
        PrincipalDef {
            principal_id: 2,
            name: "guest".to_string(),
            principal_type: "S".to_string(),
            type_desc: "SQL_USER".to_string(),
            default_schema_name: None,
            is_fixed_role: false,
        },
        PrincipalDef {
            principal_id: 16384,
            name: "db_owner".to_string(),
            principal_type: "R".to_string(),
            type_desc: "DATABASE_ROLE".to_string(),
            default_schema_name: None,
            is_fixed_role: true,
        },
        PrincipalDef {
            principal_id: 16390,
            name: "db_datareader".to_string(),
            principal_type: "R".to_string(),
            type_desc: "DATABASE_ROLE".to_string(),
            default_schema_name: None,
            is_fixed_role: true,
        },
        PrincipalDef {
            principal_id: 16391,
            name: "db_datawriter".to_string(),
            principal_type: "R".to_string(),
            type_desc: "DATABASE_ROLE".to_string(),
            default_schema_name: None,
            is_fixed_role: true,
        },
    ]
}

impl CatalogImpl {
    pub(crate) fn find_principal_id_by_name(&self, name: &str) -> Option<i32> {
        let norm = normalize_identifier(name);
        if let Some(p) = self
            .principals
            .iter()
            .find(|p| normalize_identifier(&p.name) == norm)
        {
            return Some(p.principal_id);
        }
        builtin_principals()
            .into_iter()
            .find(|p| normalize_identifier(&p.name) == norm)
            .map(|p| p.principal_id)
    }

    pub(crate) fn upsert_principal(&mut self, mut def: PrincipalDef) -> i32 {
        let norm = normalize_identifier(&def.name);
        let list = Arc::make_mut(&mut self.principals);
        if let Some(existing) = list
            .iter_mut()
            .find(|p| normalize_identifier(&p.name) == norm)
        {
            if def.default_schema_name.is_some() {
                existing.default_schema_name = def.default_schema_name;
            }
            return existing.principal_id;
        }
        if def.principal_id == 0 {
            let next_id = list
                .iter()
                .map(|p| p.principal_id)
                .filter(|&id| id < 16384)
                .max()
                .unwrap_or(4)
                + 1;
            def.principal_id = next_id;
        }
        let id = def.principal_id;
        list.push(def);
        id
    }

    pub(crate) fn drop_principal_by_name(
        &mut self,
        name: &str,
        if_exists: bool,
    ) -> Result<(), DbError> {
        let norm = normalize_identifier(name);
        let list = Arc::make_mut(&mut self.principals);
        let Some(pos) = list
            .iter()
            .position(|p| normalize_identifier(&p.name) == norm)
        else {
            if if_exists {
                return Ok(());
            }
            return Err(DbError::object_not_found(name.to_string()));
        };
        let pid = list.remove(pos).principal_id;
        Arc::make_mut(&mut self.role_members)
            .retain(|rm| rm.role_principal_id != pid && rm.member_principal_id != pid);
        Arc::make_mut(&mut self.permissions).retain(|p| p.grantee_principal_id != pid);
        Ok(())
    }

    pub(crate) fn alter_role_member(&mut self, role: &str, member: &str, add: bool) {
        let role_id = self.find_principal_id_by_name(role).unwrap_or_else(|| {
            self.upsert_principal(PrincipalDef {
                principal_id: 0,
                name: role.to_string(),
                principal_type: "R".to_string(),
                type_desc: "DATABASE_ROLE".to_string(),
                default_schema_name: None,
                is_fixed_role: false,
            })
        });
        let member_id = self.find_principal_id_by_name(member).unwrap_or_else(|| {
            self.upsert_principal(PrincipalDef {
                principal_id: 0,
                name: member.to_string(),
                principal_type: "S".to_string(),
                type_desc: "SQL_USER".to_string(),
                default_schema_name: Some("dbo".to_string()),
                is_fixed_role: false,
            })
        });
        let members = Arc::make_mut(&mut self.role_members);
        if add {
            if !members
                .iter()
                .any(|m| m.role_principal_id == role_id && m.member_principal_id == member_id)
            {
                members.push(RoleMemberDef {
                    role_principal_id: role_id,
                    member_principal_id: member_id,
                });
            }
        } else {
            members.retain(|m| {
                !(m.role_principal_id == role_id && m.member_principal_id == member_id)
            });
        }
    }

    pub(crate) fn apply_permission(&mut self, def: PermissionDef, is_revoke: bool) {
        let perms = Arc::make_mut(&mut self.permissions);
        perms.retain(|p| {
            !(p.class == def.class
                && p.major_id == def.major_id
                && p.minor_id == def.minor_id
                && p.grantee_principal_id == def.grantee_principal_id
                && p.permission_name.eq_ignore_ascii_case(&def.permission_name))
        });
        if !is_revoke {
            perms.push(def);
        }
    }

    pub(crate) fn upsert_partition_function(&mut self, mut def: PartitionFunctionDef) {
        let norm = normalize_identifier(&def.name);
        let pfs = Arc::make_mut(&mut self.partition_functions);
        if let Some(existing) = pfs
            .iter_mut()
            .find(|p| normalize_identifier(&p.name) == norm)
        {
            existing.data_type = def.data_type;
            existing.boundary_value_on_right = def.boundary_value_on_right;
            existing.boundary_values = def.boundary_values;
            return;
        }
        if def.function_id == 0 {
            def.function_id = pfs.iter().map(|p| p.function_id).max().unwrap_or(65535) + 1;
        }
        pfs.push(def);
    }

    pub(crate) fn drop_partition_function(&mut self, name: &str) {
        let norm = normalize_identifier(name);
        Arc::make_mut(&mut self.partition_functions)
            .retain(|p| normalize_identifier(&p.name) != norm);
    }

    pub(crate) fn upsert_partition_scheme(
        &mut self,
        name: String,
        function_name: &str,
        filegroups: Vec<String>,
    ) {
        let fn_norm = normalize_identifier(function_name);
        let function_id = self
            .partition_functions
            .iter()
            .find(|p| normalize_identifier(&p.name) == fn_norm)
            .map(|p| p.function_id)
            .unwrap_or(0);
        let norm = normalize_identifier(&name);
        let max_fg = self
            .filegroups
            .iter()
            .map(|f| f.data_space_id)
            .max()
            .unwrap_or(1);
        let schemes = Arc::make_mut(&mut self.partition_schemes);
        if let Some(existing) = schemes
            .iter_mut()
            .find(|s| normalize_identifier(&s.name) == norm)
        {
            existing.function_id = function_id;
            existing.filegroups = filegroups;
            return;
        }
        let max_ps = schemes
            .iter()
            .map(|s| s.data_space_id)
            .max()
            .unwrap_or(65600);
        let data_space_id = max_ps.max(max_fg) + 1;
        schemes.push(PartitionSchemeDef {
            data_space_id,
            name,
            function_id,
            filegroups,
        });
    }

    pub(crate) fn drop_partition_scheme(&mut self, name: &str) {
        let norm = normalize_identifier(name);
        Arc::make_mut(&mut self.partition_schemes)
            .retain(|s| normalize_identifier(&s.name) != norm);
    }

    pub(crate) fn add_filegroup(&mut self, name: String) {
        if name.eq_ignore_ascii_case("PRIMARY") {
            return;
        }
        let norm = normalize_identifier(&name);
        let fgs = Arc::make_mut(&mut self.filegroups);
        if fgs.iter().any(|f| normalize_identifier(&f.name) == norm) {
            return;
        }
        let data_space_id = fgs.iter().map(|f| f.data_space_id).max().unwrap_or(1) + 1;
        fgs.push(FilegroupDef {
            data_space_id,
            name,
            is_default: false,
            is_read_only: false,
        });
    }

    pub(crate) fn filegroup_data_space_id(&self, name: &str) -> i32 {
        if name.eq_ignore_ascii_case("PRIMARY") {
            return 1;
        }
        let norm = normalize_identifier(name);
        self.filegroups
            .iter()
            .find(|f| normalize_identifier(&f.name) == norm)
            .map(|f| f.data_space_id)
            .unwrap_or(1)
    }
}
