mod columns;
mod databases;
mod encryption;
mod identity_columns;
mod objects_misc;
mod tables;
mod types;

pub(crate) use columns::{SysAllColumns, SysColumns, SysComputedColumns, SysViewColumns};
pub(crate) use databases::{
    SysConfigurations, SysConfigures, SysDatabaseScopedConfigurations, SysDatabases,
    SysSysDatabases,
};
pub(crate) use encryption::{
    SysColumnEncryptionKeyValues, SysColumnEncryptionKeys, SysColumnMasterKeys,
};
pub(crate) use identity_columns::SysIdentityColumns;
pub(crate) use objects_misc::{
    SysAllSqlModules, SysAssemblyModules, SysDataSpaces, SysEdgeConstraintClauses,
    SysEdgeConstraints, SysExtendedProperties, SysFiletableSystemDefinedObjects,
    SysForeignKeyColumns, SysIndexColumns, SysInternalTables, SysMaskedColumns, SysMessages,
    SysPeriods, SysSequences, SysServerPrincipals, SysServerRoleMembers,
    SysSqlExpressionDependencies, SysSqlModules, SysStats, SysStatsColumns, SysSynonyms,
    SysSysLanguages, SysSystemSqlModules, SysTriggerEvents, SysTriggers, SysXmlIndexes,
    SysXmlSchemaCollections,
};
pub(crate) use tables::{SysFileTables, SysTables};
pub(crate) use types::{SysAssemblyTypes, SysTableTypes, SysTypes};

use super::super::virtual_table_def;
use super::super::VirtualTable;
use crate::catalog::Catalog;
use crate::executor::context::ExecutionContext;
use crate::storage::StoredRow;
use crate::types::{DataType, Value};

pub(crate) struct SysSchemas;

impl VirtualTable for SysSchemas {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "schemas",
            vec![
                ("schema_id", DataType::Int, false),
                ("name", DataType::VarChar { max_len: 128 }, false),
                ("principal_id", DataType::Int, true),
            ],
        )
    }

    fn rows(&self, catalog: &dyn Catalog, ctx: &ExecutionContext) -> Vec<StoredRow> {
        let active_db = crate::executor::database_catalog::current_database_name(ctx);
        catalog
            .get_schemas()
            .iter()
            .filter(|s| {
                crate::executor::database_catalog::schema_belongs_to_db(&s.name, active_db, catalog)
            })
            .map(|s| StoredRow {
                values: vec![
                    Value::Int(s.id as i32),
                    Value::VarChar(
                        crate::executor::database_catalog::display_schema_name(&s.name).to_string(),
                    ),
                    Value::Null,
                ],
                deleted: false,
            })
            .collect()
    }
}
