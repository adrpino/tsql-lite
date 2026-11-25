mod cow_registry;
mod id_allocator;
mod index_registry;
pub mod metadata_registry;
mod object_resolver;
mod routine_registry;
mod schema_registry;
mod sequence_registry;
mod synonym_registry;
mod table_registry;
mod trigger_registry;
mod type_registry;
mod view_registry;

use cow_registry::CowRegistry;
pub use cow_registry::{CowRegistryIter, CowRegistryView};
pub use metadata_registry::{
    ColumnEncryptionKeyDef, ColumnEncryptionKeyValueDef, ColumnMasterKeyDef,
    DatabaseRuntimeOptions, FilegroupDef, MaskedColumnDef, PartitionFunctionDef,
    PartitionSchemeDef, PermissionDef, PrincipalDef, RoleMemberDef, SecurityBlockOp,
    SecurityPolicyDef, SecurityPredicateDef, SecurityPredicateKind, UserDatabaseDef,
};

use crate::ast::Expr;
use crate::ast::{DataTypeSpec, FunctionBody, RoutineParam, Statement, TriggerEvent};
use crate::error::DbError;
use crate::executor::string_norm::normalize_identifier;
use crate::types::DataType;
#[cfg(feature = "checkpoint")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct SchemaDef {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct ForeignKeyDef {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_table: crate::ast::ObjectName,
    pub referenced_columns: Vec<String>,
    pub on_delete: crate::ast::ReferentialAction,
    pub on_update: crate::ast::ReferentialAction,
    #[cfg_attr(feature = "checkpoint", serde(default = "default_true"))]
    pub enabled: bool,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub is_not_trusted: bool,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct IdentityDef {
    pub seed: i64,
    pub increment: i64,
    pub current: i64,
}

impl IdentityDef {
    pub fn new(seed: i64, increment: i64) -> Self {
        Self {
            seed,
            increment,
            current: seed,
        }
    }

    pub fn next_value(&mut self) -> i64 {
        let value = self.current;
        self.current += self.increment;
        value
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct ColumnDef {
    pub id: u32,
    pub name: String,
    pub data_type: DataType,
    pub nullable: bool,
    pub primary_key: bool,
    pub unique: bool,
    pub identity: Option<IdentityDef>,
    pub default: Option<Expr>,
    pub default_constraint_name: Option<String>,
    pub check: Option<Expr>,
    pub check_constraint_name: Option<String>,
    pub computed_expr: Option<Expr>,
    pub collation: Option<String>,
    pub is_clustered: bool,
    #[cfg_attr(feature = "checkpoint", serde(default = "default_ansi_padding_on"))]
    pub ansi_padding_on: bool,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub is_rowversion: bool,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct CheckConstraintDef {
    pub name: String,
    pub expr: Expr,
    #[cfg_attr(feature = "checkpoint", serde(default = "default_true"))]
    pub enabled: bool,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub is_not_trusted: bool,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct IndexDef {
    pub id: u32,
    pub schema_id: u32,
    pub table_id: u32,
    pub name: String,
    pub column_ids: Vec<u32>,
    pub is_unique: bool,
    pub is_clustered: bool,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub filter_predicate: Option<Box<Expr>>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct TemporalTableConfig {
    pub table_id: u32,
    pub start_col: String,
    pub end_col: String,
    pub history_schema: String,
    pub history_table: String,
    pub system_versioning: bool,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct TableDef {
    pub id: u32,
    pub schema_id: u32,
    pub schema_name: String,
    pub name: String,
    pub columns: Vec<ColumnDef>,
    pub check_constraints: Vec<CheckConstraintDef>,
    pub foreign_keys: Vec<ForeignKeyDef>,
}

impl TableDef {
    pub fn schema_or_dbo(&self) -> &str {
        &self.schema_name
    }
}

#[allow(clippy::large_enum_variant)]
#[cfg(feature = "checkpoint")]
mod serde_arc {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::Arc;

    pub fn serialize<S, T>(value: &Arc<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        value.as_ref().serialize(serializer)
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Arc<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        T::deserialize(deserializer).map(Arc::new)
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub enum RoutineKind {
    Procedure {
        #[cfg_attr(feature = "checkpoint", serde(with = "serde_arc"))]
        body: std::sync::Arc<Vec<Statement>>,
    },
    Function {
        returns: Option<DataTypeSpec>,
        body: Box<FunctionBody>,
    },
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct RoutineDef {
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub object_id: i32,
    pub schema: String,
    pub name: String,
    pub params: Vec<RoutineParam>,
    pub kind: RoutineKind,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub definition_sql: String,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub is_schema_bound: bool,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct TableTypeDef {
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub object_id: i32,
    pub schema: String,
    pub name: String,
    pub columns: Vec<crate::ast::ColumnSpec>,
    pub table_constraints: Vec<crate::ast::TableConstraintSpec>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct ScalarTypeDef {
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub object_id: i32,
    pub schema: String,
    pub name: String,
    pub base_type: DataType,
    pub nullable: bool,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct ViewDef {
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub object_id: i32,
    pub schema: String,
    pub name: String,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub schema_id: u32,
    #[cfg_attr(feature = "checkpoint", serde(with = "serde_arc"))]
    pub query: std::sync::Arc<Statement>, // Should be Statement::Select
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub definition_sql: String,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub is_schema_bound: bool,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub with_check_option: bool,
}

#[cfg(feature = "checkpoint")]
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct TriggerDef {
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub object_id: i32,
    pub schema: String,
    pub name: String,
    pub table_schema: String,
    pub table_name: String,
    pub events: Vec<TriggerEvent>,
    pub is_instead_of: bool,
    #[cfg_attr(feature = "checkpoint", serde(default = "default_true"))]
    pub enabled: bool,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub is_database_scope: bool,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub ddl_events: Vec<String>,
    #[cfg_attr(feature = "checkpoint", serde(with = "serde_arc"))]
    pub body: std::sync::Arc<Vec<Statement>>,
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub definition_sql: String,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct SynonymDef {
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub object_id: i32,
    pub schema: String,
    pub name: String,
    pub base_object: crate::ast::ObjectName,
}

#[derive(Debug)]
pub struct SeqCounter(std::sync::atomic::AtomicI64);

impl SeqCounter {
    pub fn new(v: i64) -> Self {
        Self(std::sync::atomic::AtomicI64::new(v))
    }

    pub fn load(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }

    pub fn store(&self, v: i64) {
        self.0.store(v, Ordering::Relaxed);
    }

    pub fn compare_exchange(&self, current: i64, new: i64) -> Result<i64, i64> {
        self.0
            .compare_exchange(current, new, Ordering::AcqRel, Ordering::Acquire)
    }
}

impl Clone for SeqCounter {
    fn clone(&self) -> Self {
        Self::new(self.load())
    }
}

#[cfg(feature = "checkpoint")]
impl Serialize for SeqCounter {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.load().serialize(serializer)
    }
}

#[cfg(feature = "checkpoint")]
impl<'de> Deserialize<'de> for SeqCounter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        i64::deserialize(deserializer).map(Self::new)
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct SequenceDef {
    #[cfg_attr(feature = "checkpoint", serde(default))]
    pub object_id: i32,
    pub schema: String,
    pub name: String,
    pub data_type: DataType,
    pub start_value: i64,
    pub increment: i64,
    pub current_value: SeqCounter,
    pub minimum_value: i64,
    pub maximum_value: i64,
    pub is_cycling: bool,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct ExtendedPropertyDef {
    pub class: i32,
    pub class_desc: String,
    pub major_id: i32,
    pub minor_id: i32,
    pub name: String,
    pub value: Option<String>,
}

pub trait IdAllocator {
    fn alloc_table_id(&mut self) -> u32;
    fn alloc_object_id(&mut self) -> i32;
    fn alloc_column_id(&mut self) -> u32;
    fn alloc_index_id(&mut self) -> u32;
    fn alloc_schema_id(&mut self) -> u32;
    fn next_rowversion(&self) -> u64;
    fn current_rowversion(&self) -> u64;
}

pub trait SchemaRegistry {
    fn get_schemas(&self) -> CowRegistryView<'_, SchemaDef>;
    fn get_schema_id(&self, name: &str) -> Option<u32>;
    fn create_schema(&mut self, name: &str) -> Result<(), DbError>;
    fn drop_schema(&mut self, name: &str) -> Result<(), DbError>;
    fn transfer_object(
        &mut self,
        target_schema: &str,
        entity_type: Option<&str>,
        source_schema: &str,
        source_name: &str,
    ) -> Result<(), DbError>;
}

pub trait TableRegistry {
    fn get_tables(&self) -> CowRegistryView<'_, TableDef>;
    fn find_table(&self, schema: &str, name: &str) -> Option<&TableDef>;
    fn find_table_arc(&self, schema: &str, name: &str) -> Option<Arc<TableDef>> {
        self.find_table(schema, name).cloned().map(Arc::new)
    }
    fn find_table_mut(&mut self, schema: &str, name: &str) -> Option<&mut TableDef>;
    fn register_table(&mut self, table: TableDef);
    fn unregister_table_by_id(&mut self, id: u32);
    fn drop_table(&mut self, schema: &str, name: &str) -> Result<u32, DbError>;
    fn next_identity_value(&mut self, table_id: u32, column_name: &str) -> Result<i64, DbError>;
}

pub trait IndexRegistry {
    fn get_indexes(&self) -> CowRegistryView<'_, IndexDef>;
    fn register_index(&mut self, index: IndexDef);
    fn drop_index_by_table_id(&mut self, table_id: u32);
    fn create_index(
        &mut self,
        schema: &str,
        name: &str,
        table_schema: &str,
        table_name: &str,
        columns: &[String],
        // Using TableRegistry to find tables instead of passing them
    ) -> Result<(), DbError>;
    #[allow(clippy::too_many_arguments)]
    fn create_index_with_options(
        &mut self,
        schema: &str,
        name: &str,
        table_schema: &str,
        table_name: &str,
        columns: &[String],
        is_clustered: bool,
        is_unique: bool,
    ) -> Result<(), DbError> {
        self.create_index_with_filter(
            schema,
            name,
            table_schema,
            table_name,
            columns,
            is_clustered,
            is_unique,
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn create_index_with_filter(
        &mut self,
        schema: &str,
        name: &str,
        table_schema: &str,
        table_name: &str,
        columns: &[String],
        is_clustered: bool,
        is_unique: bool,
        filter_predicate: Option<Box<Expr>>,
    ) -> Result<(), DbError>;
    fn drop_index(
        &mut self,
        schema: &str,
        name: &str,
        table_schema: &str,
        table_name: &str,
    ) -> Result<(), DbError>;
}

pub trait RoutineRegistry {
    fn get_routines(&self) -> CowRegistryView<'_, RoutineDef>;
    fn find_routine(&self, schema: &str, name: &str) -> Option<&RoutineDef>;
    fn create_routine(&mut self, routine: RoutineDef) -> Result<(), DbError>;
    fn replace_routine(&mut self, routine: RoutineDef) -> Result<(), DbError>;
    fn drop_routine(
        &mut self,
        schema: &str,
        name: &str,
        expect_function: bool,
    ) -> Result<(), DbError>;
}

pub trait TypeRegistry {
    fn get_table_types(&self) -> &[TableTypeDef];
    fn find_table_type(&self, schema: &str, name: &str) -> Option<&TableTypeDef>;
    fn create_table_type(&mut self, def: TableTypeDef) -> Result<(), DbError>;
    fn drop_table_type(&mut self, schema: &str, name: &str) -> Result<(), DbError>;
    fn get_scalar_types(&self) -> &[ScalarTypeDef];
    fn find_scalar_type(&self, schema: &str, name: &str) -> Option<&ScalarTypeDef>;
    fn create_scalar_type(&mut self, def: ScalarTypeDef) -> Result<(), DbError>;
    fn drop_scalar_type(&mut self, schema: &str, name: &str) -> Result<(), DbError>;
}

pub trait ViewRegistry {
    fn get_views(&self) -> CowRegistryView<'_, ViewDef>;
    fn find_view(&self, schema: &str, name: &str) -> Option<&ViewDef>;
    fn create_view(&mut self, view: ViewDef) -> Result<(), DbError>;
    fn replace_view(&mut self, view: ViewDef) -> Result<(), DbError>;
    fn drop_view(&mut self, schema: &str, name: &str) -> Result<(), DbError>;
}

pub trait TriggerRegistry {
    fn get_triggers(&self) -> CowRegistryView<'_, TriggerDef>;
    fn find_triggers_for_table(&self, schema: &str, name: &str) -> Vec<&TriggerDef>;
    fn create_trigger(&mut self, trigger: TriggerDef) -> Result<(), DbError>;
    fn replace_trigger(&mut self, trigger: TriggerDef) -> Result<(), DbError>;
    fn drop_trigger(&mut self, schema: &str, name: &str) -> Result<(), DbError>;
    fn set_trigger_enabled(
        &mut self,
        trigger_schema: Option<&str>,
        trigger_name: Option<&str>,
        table_schema: &str,
        table_name: &str,
        enabled: bool,
    ) -> Result<(), DbError>;
}

pub trait SynonymRegistry {
    fn get_synonyms(&self) -> &[SynonymDef];
    fn find_synonym(&self, schema: &str, name: &str) -> Option<&SynonymDef>;
    fn create_synonym(&mut self, synonym: SynonymDef) -> Result<(), DbError>;
    fn drop_synonym(&mut self, schema: &str, name: &str) -> Result<(), DbError>;
}

pub trait SequenceRegistry {
    fn get_sequences(&self) -> &[SequenceDef];
    fn find_sequence(&self, schema: &str, name: &str) -> Option<&SequenceDef>;
    fn create_sequence(&mut self, sequence: SequenceDef) -> Result<(), DbError>;
    #[allow(clippy::too_many_arguments)]
    fn alter_sequence(
        &mut self,
        schema: &str,
        name: &str,
        restart: Option<Option<i64>>,
        increment: Option<i64>,
        minimum_value: Option<Option<i64>>,
        maximum_value: Option<Option<i64>>,
        is_cycling: Option<bool>,
    ) -> Result<(), DbError>;
    fn drop_sequence(&mut self, schema: &str, name: &str) -> Result<(), DbError>;
    fn next_sequence_value(&mut self, schema: &str, name: &str) -> Result<i64, DbError>;
    fn next_sequence_value_shared(&self, schema: &str, name: &str) -> Result<i64, DbError>;
}

pub trait ObjectResolver {
    fn object_id(&self, schema: &str, name: &str) -> Option<i32>;
}

/// Aggregate convenience trait that composes 9 focused sub-traits: [`IdAllocator`],
/// [`SchemaRegistry`], [`TableRegistry`], [`IndexRegistry`], [`RoutineRegistry`],
/// [`TypeRegistry`], [`ViewRegistry`], [`TriggerRegistry`], and [`ObjectResolver`].
///
/// New code that only needs a subset of catalog capabilities should prefer narrower sub-trait
/// bounds (e.g. `T: TableRegistry + SchemaRegistry`) to keep coupling minimal. This aggregate
/// exists as a facade for contexts that genuinely require full catalog access.
pub trait Catalog:
    IdAllocator
    + SchemaRegistry
    + TableRegistry
    + IndexRegistry
    + RoutineRegistry
    + TypeRegistry
    + ViewRegistry
    + TriggerRegistry
    + SynonymRegistry
    + SequenceRegistry
    + ObjectResolver
    + std::fmt::Debug
    + Send
    + Sync
{
    fn get_extended_properties(&self) -> &[ExtendedPropertyDef] {
        &[]
    }
    fn add_extended_property(&mut self, _prop: ExtendedPropertyDef) -> Result<(), DbError> {
        Ok(())
    }
    fn update_extended_property(&mut self, _prop: ExtendedPropertyDef) -> Result<(), DbError> {
        Ok(())
    }
    fn drop_extended_property(
        &mut self,
        _class: i32,
        _major_id: i32,
        _minor_id: i32,
        _name: &str,
    ) -> Result<(), DbError> {
        Ok(())
    }

    fn get_principals(&self) -> &[PrincipalDef] {
        &[]
    }
    fn get_role_members(&self) -> &[RoleMemberDef] {
        &[]
    }
    fn get_permissions(&self) -> &[PermissionDef] {
        &[]
    }
    fn get_partition_functions(&self) -> &[PartitionFunctionDef] {
        &[]
    }
    fn get_partition_schemes(&self) -> &[PartitionSchemeDef] {
        &[]
    }
    fn get_filegroups(&self) -> &[FilegroupDef] {
        &[]
    }
    fn get_temporal_config(&self, _table_id: u32) -> Option<&TemporalTableConfig> {
        None
    }
    fn set_temporal_config(&mut self, _cfg: TemporalTableConfig) {}
    fn get_all_temporal_configs(&self) -> &[TemporalTableConfig] {
        &[]
    }
    fn filegroup_data_space_id(&self, _name: &str) -> i32 {
        1
    }
    fn find_principal_id(&self, _name: &str) -> Option<i32> {
        None
    }
    fn upsert_principal(&mut self, _def: PrincipalDef) -> i32 {
        0
    }
    fn drop_principal(&mut self, _name: &str, _if_exists: bool) -> Result<(), DbError> {
        Ok(())
    }
    fn alter_role_member(&mut self, _role: &str, _member: &str, _add: bool) {}
    fn apply_permission(&mut self, _def: PermissionDef, _is_revoke: bool) {}
    fn upsert_partition_function(&mut self, _def: PartitionFunctionDef) {}
    fn drop_partition_function(&mut self, _name: &str) {}
    fn upsert_partition_scheme(
        &mut self,
        _name: String,
        _function_name: &str,
        _filegroups: Vec<String>,
    ) {
    }
    fn drop_partition_scheme(&mut self, _name: &str) {}
    fn add_filegroup(&mut self, _name: String) {}
    fn is_table_large_value_out_of_row(&self, _table_id: u32) -> bool {
        false
    }
    fn set_table_large_value_out_of_row(&mut self, _table_id: u32, _enabled: bool) {}
    fn get_masked_columns(&self) -> &[MaskedColumnDef] {
        &[]
    }
    fn get_column_master_keys(&self) -> &[ColumnMasterKeyDef] {
        &[]
    }
    fn get_column_encryption_keys(&self) -> &[ColumnEncryptionKeyDef] {
        &[]
    }
    fn create_column_master_key(&mut self, _def: ColumnMasterKeyDef) -> Result<(), DbError> {
        Ok(())
    }
    fn drop_column_master_key(&mut self, _name: &str) -> Result<(), DbError> {
        Ok(())
    }
    fn create_column_encryption_key(
        &mut self,
        _name: String,
        _values: Vec<(String, String, Vec<u8>)>,
    ) -> Result<(), DbError> {
        Ok(())
    }
    fn alter_column_encryption_key(
        &mut self,
        _name: &str,
        _add: bool,
        _cmk_name: &str,
        _algorithm: String,
        _encrypted_value: Vec<u8>,
    ) -> Result<(), DbError> {
        Ok(())
    }
    fn drop_column_encryption_key(&mut self, _name: &str) -> Result<(), DbError> {
        Ok(())
    }
    fn get_user_databases(&self) -> &[UserDatabaseDef] {
        &[]
    }
    fn get_database_options(&self, _db_name: &str) -> DatabaseRuntimeOptions {
        DatabaseRuntimeOptions::default()
    }
    fn set_database_option(
        &mut self,
        _db_name: &str,
        _opt: &crate::ast::DatabaseOptionChange,
    ) -> Result<(), DbError> {
        Ok(())
    }
    fn create_database(&mut self, _name: String) -> Result<i32, DbError> {
        Ok(0)
    }
    fn clone_database(
        &mut self,
        _source_db: &str,
        _target_db: String,
        _as_snapshot: bool,
    ) -> Result<Vec<(u32, u32)>, DbError> {
        Ok(Vec::new())
    }
    fn drop_database(&mut self, _name: &str, _if_exists: bool) -> Result<Vec<u32>, DbError> {
        Ok(Vec::new())
    }
    fn set_column_mask(
        &mut self,
        _table_id: u32,
        _column_name: &str,
        _masking_function: Option<String>,
    ) {
    }
    fn remove_table_masks(&mut self, _table_id: u32) {}
    fn get_security_policies(&self) -> &[SecurityPolicyDef] {
        &[]
    }
    fn upsert_security_policy(&mut self, _policy: SecurityPolicyDef) -> Result<(), DbError> {
        Ok(())
    }
    fn drop_security_policy(
        &mut self,
        _schema: &str,
        _name: &str,
        _if_exists: bool,
    ) -> Result<(), DbError> {
        Ok(())
    }

    fn clone_boxed(&self) -> Box<dyn Catalog>;
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any>;
    fn restore_from_boxed(&mut self, _snapshot: Box<dyn Catalog>) {}
    fn rebuild_maps(&mut self) {}
    /// Returns `true` when at least one `WITH SCHEMABINDING` view or routine exists in the catalog.
    /// Allows `DROP TABLE` / `ALTER TABLE` dependency guards to short-circuit in O(1) when false.
    fn has_schemabound_objects(&self) -> bool {
        true
    }
    /// Merges changes made in `workspace` relative to `base` into `self` during
    /// a concurrent transaction commit.
    fn merge_workspace(&mut self, workspace: Self, _base: &Self)
    where
        Self: Sized,
    {
        *self = workspace;
    }
}

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

#[cfg(feature = "checkpoint")]
mod atomic_u32_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    pub fn serialize<S>(val: &Arc<AtomicU32>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        val.load(Ordering::Relaxed).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Arc<AtomicU32>, D::Error>
    where
        D: Deserializer<'de>,
    {
        u32::deserialize(deserializer).map(|v| Arc::new(AtomicU32::new(v)))
    }
}

#[cfg(feature = "checkpoint")]
mod atomic_i32_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Arc;

    pub fn serialize<S>(val: &Arc<AtomicI32>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        val.load(Ordering::Relaxed).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Arc<AtomicI32>, D::Error>
    where
        D: Deserializer<'de>,
    {
        i32::deserialize(deserializer).map(|v| Arc::new(AtomicI32::new(v)))
    }
}

#[cfg(feature = "checkpoint")]
mod atomic_u64_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    pub fn serialize<S>(val: &Arc<AtomicU64>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        val.load(Ordering::Relaxed).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Arc<AtomicU64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        u64::deserialize(deserializer).map(|v| Arc::new(AtomicU64::new(v)))
    }
}

fn default_next_rowversion() -> Arc<AtomicU64> {
    Arc::new(AtomicU64::new(1))
}

#[cfg(feature = "checkpoint")]
mod arc_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::Arc;

    pub fn serialize<S, T>(val: &Arc<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        val.as_ref().serialize(serializer)
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Arc<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        T::deserialize(deserializer).map(Arc::new)
    }
}

type TriggerTableIndex = HashMap<(String, String), Vec<(String, String)>>;

#[derive(Debug, Clone)]
#[cfg_attr(feature = "checkpoint", derive(Serialize, Deserialize))]
pub struct CatalogImpl {
    schemas: CowRegistry<String, SchemaDef>,
    tables: CowRegistry<(u32, String), TableDef>,
    indexes: CowRegistry<(u32, u32, String), IndexDef>,
    routines: CowRegistry<(String, String), RoutineDef>,
    #[cfg_attr(feature = "checkpoint", serde(with = "arc_serde"))]
    table_types: Arc<Vec<TableTypeDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    scalar_types: Arc<Vec<ScalarTypeDef>>,
    views: CowRegistry<(String, String), ViewDef>,
    triggers: CowRegistry<(String, String), TriggerDef>,
    #[cfg_attr(feature = "checkpoint", serde(with = "arc_serde"))]
    synonyms: Arc<Vec<SynonymDef>>,
    #[cfg_attr(feature = "checkpoint", serde(with = "arc_serde"))]
    sequences: Arc<Vec<SequenceDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    extended_properties: Arc<Vec<ExtendedPropertyDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    principals: Arc<Vec<PrincipalDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    role_members: Arc<Vec<RoleMemberDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    permissions: Arc<Vec<PermissionDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    partition_functions: Arc<Vec<PartitionFunctionDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    partition_schemes: Arc<Vec<PartitionSchemeDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    filegroups: Arc<Vec<FilegroupDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    table_large_value_out_of_row: Arc<Vec<u32>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    masked_columns: Arc<Vec<MaskedColumnDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    column_master_keys: Arc<Vec<ColumnMasterKeyDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    column_encryption_keys: Arc<Vec<ColumnEncryptionKeyDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    user_databases: Arc<Vec<UserDatabaseDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    builtin_db_options: Arc<Vec<UserDatabaseDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    security_policies: Arc<Vec<SecurityPolicyDef>>,
    #[cfg_attr(feature = "checkpoint", serde(default, with = "arc_serde"))]
    temporal_tables: Arc<Vec<TemporalTableConfig>>,
    #[cfg_attr(feature = "checkpoint", serde(with = "atomic_u32_serde"))]
    next_schema_id: Arc<AtomicU32>,
    #[cfg_attr(feature = "checkpoint", serde(with = "atomic_u32_serde"))]
    next_table_id: Arc<AtomicU32>,
    #[cfg_attr(feature = "checkpoint", serde(with = "atomic_u32_serde"))]
    next_column_id: Arc<AtomicU32>,
    #[cfg_attr(feature = "checkpoint", serde(with = "atomic_u32_serde"))]
    next_index_id: Arc<AtomicU32>,
    #[cfg_attr(feature = "checkpoint", serde(default = "default_next_object_id", with = "atomic_i32_serde"))]
    next_object_id: Arc<AtomicI32>,
    #[cfg_attr(feature = "checkpoint", serde(default = "default_next_rowversion", with = "atomic_u64_serde"))]
    next_rowversion: Arc<AtomicU64>,

    #[cfg_attr(feature = "checkpoint", serde(skip))]
    type_map: Arc<HashMap<(String, String), usize>>,
    #[cfg_attr(feature = "checkpoint", serde(skip))]
    scalar_type_map: Arc<HashMap<(String, String), usize>>,
    #[cfg_attr(feature = "checkpoint", serde(skip))]
    synonym_map: Arc<HashMap<(String, String), usize>>,
    #[cfg_attr(feature = "checkpoint", serde(skip))]
    sequence_map: Arc<HashMap<(String, String), usize>>,
    #[cfg_attr(feature = "checkpoint", serde(skip))]
    schemabound_view_count: usize,
    #[cfg_attr(feature = "checkpoint", serde(skip))]
    schemabound_routine_count: usize,
    #[cfg_attr(feature = "checkpoint", serde(skip))]
    triggers_by_table: Arc<TriggerTableIndex>,
}

impl CatalogImpl {
    pub fn new() -> Self {
        let mut c = Self::default();
        let dbo_id = c.alloc_schema_id();
        c.schemas.insert_or_replace(
            normalize_identifier("dbo"),
            SchemaDef {
                id: dbo_id,
                name: "dbo".to_string(),
            },
        );
        c.rebuild_maps();
        c
    }

    pub fn rebuild_maps(&mut self) {
        self.schemas.rebuild_map(|s| normalize_identifier(&s.name));
        self.tables
            .rebuild_map(|t| (t.schema_id, normalize_identifier(&t.name)));
        self.indexes
            .rebuild_map(|idx| (idx.schema_id, idx.table_id, normalize_identifier(&idx.name)));
        self.routines.rebuild_map(|r| {
            (
                normalize_identifier(&r.schema),
                normalize_identifier(&r.name),
            )
        });
        self.type_map = Arc::new(
            self.table_types
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    (
                        (
                            normalize_identifier(&t.schema),
                            normalize_identifier(&t.name),
                        ),
                        i,
                    )
                })
                .collect(),
        );
        self.scalar_type_map = Arc::new(
            self.scalar_types
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    (
                        (
                            normalize_identifier(&t.schema),
                            normalize_identifier(&t.name),
                        ),
                        i,
                    )
                })
                .collect(),
        );
        self.synonym_map = Arc::new(
            self.synonyms
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    (
                        (
                            normalize_identifier(&s.schema),
                            normalize_identifier(&s.name),
                        ),
                        i,
                    )
                })
                .collect(),
        );
        self.sequence_map = Arc::new(
            self.sequences
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    (
                        (
                            normalize_identifier(&s.schema),
                            normalize_identifier(&s.name),
                        ),
                        i,
                    )
                })
                .collect(),
        );
        self.views.rebuild_map(|v| {
            (
                normalize_identifier(&v.schema),
                normalize_identifier(&v.name),
            )
        });
        self.triggers.rebuild_map(|t| {
            (
                normalize_identifier(&t.schema),
                normalize_identifier(&t.name),
            )
        });
        self.recompute_schemabound_counts();
        self.rebuild_triggers_by_table();
    }

    pub(crate) fn recompute_schemabound_counts(&mut self) {
        self.schemabound_view_count = self
            .views
            .view()
            .iter()
            .filter(|v| v.is_schema_bound)
            .count();
        self.schemabound_routine_count = self
            .routines
            .view()
            .iter()
            .filter(|r| r.is_schema_bound)
            .count();
    }

    pub(crate) fn rebuild_triggers_by_table(&mut self) {
        let mut by_table: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
        for t in self.triggers.view().iter() {
            if !t.is_database_scope {
                let table_key = (
                    normalize_identifier(&t.table_schema),
                    normalize_identifier(&t.table_name),
                );
                let trig_key = (
                    normalize_identifier(&t.schema),
                    normalize_identifier(&t.name),
                );
                by_table.entry(table_key).or_default().push(trig_key);
            }
        }
        self.triggers_by_table = Arc::new(by_table);
    }

    /// Remove any indexes belonging to `table_id` via O(1)-per-entry CoW tombstones.
    pub(crate) fn remove_indexes_for_table_id(&mut self, table_id: u32) {
        let keys: Vec<(u32, u32, String)> = self
            .indexes
            .iter()
            .filter(|i| i.table_id == table_id)
            .map(|i| (i.schema_id, i.table_id, normalize_identifier(&i.name)))
            .collect();
        for key in keys {
            self.indexes.remove(&key, |i| {
                (i.schema_id, i.table_id, normalize_identifier(&i.name))
            });
        }
    }
}

impl Default for CatalogImpl {
    fn default() -> Self {
        Self {
            schemas: CowRegistry::default(),
            tables: CowRegistry::default(),
            indexes: CowRegistry::default(),
            routines: CowRegistry::default(),
            table_types: Arc::new(Vec::new()),
            scalar_types: Arc::new(Vec::new()),
            views: CowRegistry::default(),
            triggers: CowRegistry::default(),
            synonyms: Arc::new(Vec::new()),
            sequences: Arc::new(Vec::new()),
            extended_properties: Arc::new(Vec::new()),
            principals: Arc::new(Vec::new()),
            role_members: Arc::new(Vec::new()),
            permissions: Arc::new(Vec::new()),
            partition_functions: Arc::new(Vec::new()),
            partition_schemes: Arc::new(Vec::new()),
            filegroups: Arc::new(Vec::new()),
            table_large_value_out_of_row: Arc::new(Vec::new()),
            masked_columns: Arc::new(Vec::new()),
            column_master_keys: Arc::new(Vec::new()),
            column_encryption_keys: Arc::new(Vec::new()),
            user_databases: Arc::new(Vec::new()),
            builtin_db_options: Arc::new(Vec::new()),
            security_policies: Arc::new(Vec::new()),
            temporal_tables: Arc::new(Vec::new()),
            next_schema_id: Arc::new(AtomicU32::new(1)),
            next_table_id: Arc::new(AtomicU32::new(1234567890)),
            next_column_id: Arc::new(AtomicU32::new(1)),
            next_index_id: Arc::new(AtomicU32::new(234567890)),
            next_object_id: Arc::new(AtomicI32::new(-1)),
            next_rowversion: default_next_rowversion(),
            type_map: Arc::new(HashMap::new()),
            scalar_type_map: Arc::new(HashMap::new()),
            synonym_map: Arc::new(HashMap::new()),
            sequence_map: Arc::new(HashMap::new()),
            schemabound_view_count: 0,
            schemabound_routine_count: 0,
            triggers_by_table: Arc::new(HashMap::new()),
        }
    }
}

#[cfg(feature = "checkpoint")]
fn default_next_object_id() -> Arc<AtomicI32> {
    Arc::new(AtomicI32::new(-1))
}

#[cfg(feature = "checkpoint")]
fn default_ansi_padding_on() -> bool {
    true
}

fn merge_arc_vec<T, K, F>(
    target: &mut Arc<Vec<T>>,
    ws: &Arc<Vec<T>>,
    base: &Arc<Vec<T>>,
    key_of: F,
) -> bool
where
    T: Clone,
    K: Eq + std::hash::Hash,
    F: Fn(&T) -> K,
{
    if Arc::ptr_eq(ws, base) {
        return false;
    }
    let mut base_keys: HashMap<K, usize> = HashMap::with_capacity(base.len());
    for (i, item) in base.iter().enumerate() {
        base_keys.insert(key_of(item), i);
    }
    let mut ws_keys: HashMap<K, usize> = HashMap::with_capacity(ws.len());
    for (i, item) in ws.iter().enumerate() {
        ws_keys.insert(key_of(item), i);
    }

    let mut changed = false;
    let mut target_idx: HashMap<K, usize> = HashMap::with_capacity(target.len());
    for (i, item) in target.iter().enumerate() {
        target_idx.insert(key_of(item), i);
    }

    // Apply removals (in base, absent from workspace).
    let mut to_remove: Vec<K> = Vec::new();
    for k in base_keys.keys() {
        if !ws_keys.contains_key(k) {
            to_remove.push(key_of(&base[base_keys[k]]));
        }
    }
    if !to_remove.is_empty() {
        let remove_set: std::collections::HashSet<K> = to_remove.into_iter().collect();
        let vec = Arc::make_mut(target);
        vec.retain(|item| !remove_set.contains(&key_of(item)));
        changed = true;
        target_idx.clear();
        for (i, item) in vec.iter().enumerate() {
            target_idx.insert(key_of(item), i);
        }
    }

    // Apply additions (in workspace, absent from base) and updates (in workspace & base,
    // mutated or re-inserted by workspace — e.g. sequence current_value or index replace).
    for item in ws.iter() {
        let k = key_of(item);
        if !base_keys.contains_key(&k) {
            let vec = Arc::make_mut(target);
            if let Some(&pos) = target_idx.get(&k) {
                vec[pos] = item.clone();
            } else {
                let pos = vec.len();
                vec.push(item.clone());
                target_idx.insert(k, pos);
            }
            changed = true;
        }
    }

    changed
}

impl Catalog for CatalogImpl {
    fn get_extended_properties(&self) -> &[ExtendedPropertyDef] {
        &self.extended_properties
    }

    fn add_extended_property(&mut self, prop: ExtendedPropertyDef) -> Result<(), DbError> {
        let props = Arc::make_mut(&mut self.extended_properties);
        if let Some(existing) = props.iter_mut().find(|p| {
            p.class == prop.class
                && p.major_id == prop.major_id
                && p.minor_id == prop.minor_id
                && p.name.eq_ignore_ascii_case(&prop.name)
        }) {
            existing.value = prop.value;
        } else {
            props.push(prop);
        }
        Ok(())
    }

    fn update_extended_property(&mut self, prop: ExtendedPropertyDef) -> Result<(), DbError> {
        let props = Arc::make_mut(&mut self.extended_properties);
        if let Some(existing) = props.iter_mut().find(|p| {
            p.class == prop.class
                && p.major_id == prop.major_id
                && p.minor_id == prop.minor_id
                && p.name.eq_ignore_ascii_case(&prop.name)
        }) {
            existing.value = prop.value;
        } else {
            props.push(prop);
        }
        Ok(())
    }

    fn drop_extended_property(
        &mut self,
        class: i32,
        major_id: i32,
        minor_id: i32,
        name: &str,
    ) -> Result<(), DbError> {
        let props = Arc::make_mut(&mut self.extended_properties);
        props.retain(|p| {
            !(p.class == class
                && p.major_id == major_id
                && p.minor_id == minor_id
                && p.name.eq_ignore_ascii_case(name))
        });
        Ok(())
    }

    fn get_principals(&self) -> &[PrincipalDef] {
        &self.principals
    }
    fn get_role_members(&self) -> &[RoleMemberDef] {
        &self.role_members
    }
    fn get_permissions(&self) -> &[PermissionDef] {
        &self.permissions
    }
    fn get_partition_functions(&self) -> &[PartitionFunctionDef] {
        &self.partition_functions
    }
    fn get_partition_schemes(&self) -> &[PartitionSchemeDef] {
        &self.partition_schemes
    }
    fn get_filegroups(&self) -> &[FilegroupDef] {
        &self.filegroups
    }
    fn get_temporal_config(&self, table_id: u32) -> Option<&TemporalTableConfig> {
        self.temporal_tables.iter().find(|t| t.table_id == table_id)
    }
    fn set_temporal_config(&mut self, cfg: TemporalTableConfig) {
        let list = Arc::make_mut(&mut self.temporal_tables);
        if let Some(existing) = list.iter_mut().find(|t| t.table_id == cfg.table_id) {
            *existing = cfg;
        } else {
            list.push(cfg);
        }
    }
    fn get_all_temporal_configs(&self) -> &[TemporalTableConfig] {
        &self.temporal_tables
    }
    fn filegroup_data_space_id(&self, name: &str) -> i32 {
        CatalogImpl::filegroup_data_space_id(self, name)
    }
    fn find_principal_id(&self, name: &str) -> Option<i32> {
        CatalogImpl::find_principal_id_by_name(self, name)
    }
    fn upsert_principal(&mut self, def: PrincipalDef) -> i32 {
        CatalogImpl::upsert_principal(self, def)
    }
    fn drop_principal(&mut self, name: &str, if_exists: bool) -> Result<(), DbError> {
        CatalogImpl::drop_principal_by_name(self, name, if_exists)
    }
    fn alter_role_member(&mut self, role: &str, member: &str, add: bool) {
        CatalogImpl::alter_role_member(self, role, member, add);
    }
    fn apply_permission(&mut self, def: PermissionDef, is_revoke: bool) {
        CatalogImpl::apply_permission(self, def, is_revoke);
    }
    fn upsert_partition_function(&mut self, def: PartitionFunctionDef) {
        CatalogImpl::upsert_partition_function(self, def);
    }
    fn drop_partition_function(&mut self, name: &str) {
        CatalogImpl::drop_partition_function(self, name);
    }
    fn upsert_partition_scheme(
        &mut self,
        name: String,
        function_name: &str,
        filegroups: Vec<String>,
    ) {
        CatalogImpl::upsert_partition_scheme(self, name, function_name, filegroups);
    }
    fn drop_partition_scheme(&mut self, name: &str) {
        CatalogImpl::drop_partition_scheme(self, name);
    }
    fn add_filegroup(&mut self, name: String) {
        CatalogImpl::add_filegroup(self, name);
    }
    fn is_table_large_value_out_of_row(&self, table_id: u32) -> bool {
        self.table_large_value_out_of_row.contains(&table_id)
    }
    fn set_table_large_value_out_of_row(&mut self, table_id: u32, enabled: bool) {
        let list = Arc::make_mut(&mut self.table_large_value_out_of_row);
        if enabled {
            if !list.contains(&table_id) {
                list.push(table_id);
            }
        } else {
            list.retain(|&id| id != table_id);
        }
    }
    fn get_masked_columns(&self) -> &[MaskedColumnDef] {
        &self.masked_columns
    }
    fn get_column_master_keys(&self) -> &[ColumnMasterKeyDef] {
        &self.column_master_keys
    }
    fn get_column_encryption_keys(&self) -> &[ColumnEncryptionKeyDef] {
        &self.column_encryption_keys
    }
    fn create_column_master_key(&mut self, mut def: ColumnMasterKeyDef) -> Result<(), DbError> {
        let list = Arc::make_mut(&mut self.column_master_keys);
        if list.iter().any(|k| k.name.eq_ignore_ascii_case(&def.name)) {
            return Err(DbError::Execution(format!(
                "There is already a column master key named '{}' in the database.",
                def.name
            )));
        }
        let next_id = list
            .iter()
            .map(|k| k.column_master_key_id)
            .max()
            .unwrap_or(0)
            + 1;
        def.column_master_key_id = next_id;
        list.push(def);
        Ok(())
    }
    fn drop_column_master_key(&mut self, name: &str) -> Result<(), DbError> {
        let cmk_id = self
            .column_master_keys
            .iter()
            .find(|k| k.name.eq_ignore_ascii_case(name))
            .map(|k| k.column_master_key_id)
            .ok_or_else(|| {
                DbError::Execution(format!(
                    "Cannot drop the column master key '{}', because it does not exist.",
                    name
                ))
            })?;
        if self
            .column_encryption_keys
            .iter()
            .any(|cek| cek.values.iter().any(|v| v.column_master_key_id == cmk_id))
        {
            return Err(DbError::Execution(format!(
                "Cannot drop column master key '{}' because it is referenced by a column encryption key.",
                name
            )));
        }
        Arc::make_mut(&mut self.column_master_keys).retain(|k| k.column_master_key_id != cmk_id);
        Ok(())
    }
    fn create_column_encryption_key(
        &mut self,
        name: String,
        values: Vec<(String, String, Vec<u8>)>,
    ) -> Result<(), DbError> {
        if self
            .column_encryption_keys
            .iter()
            .any(|k| k.name.eq_ignore_ascii_case(&name))
        {
            return Err(DbError::Execution(format!(
                "There is already a column encryption key named '{}' in the database.",
                name
            )));
        }
        let mut resolved = Vec::with_capacity(values.len());
        for (cmk_name, algorithm, encrypted_value) in values {
            let cmk_id = self
                .column_master_keys
                .iter()
                .find(|k| k.name.eq_ignore_ascii_case(&cmk_name))
                .map(|k| k.column_master_key_id)
                .ok_or_else(|| {
                    DbError::Execution(format!(
                        "Cannot find the column master key '{}', because it does not exist.",
                        cmk_name
                    ))
                })?;
            resolved.push(ColumnEncryptionKeyValueDef {
                column_master_key_id: cmk_id,
                encrypted_value,
                encryption_algorithm_name: algorithm,
            });
        }
        let list = Arc::make_mut(&mut self.column_encryption_keys);
        let next_id = list
            .iter()
            .map(|k| k.column_encryption_key_id)
            .max()
            .unwrap_or(0)
            + 1;
        list.push(ColumnEncryptionKeyDef {
            column_encryption_key_id: next_id,
            name,
            values: resolved,
        });
        Ok(())
    }
    fn alter_column_encryption_key(
        &mut self,
        name: &str,
        add: bool,
        cmk_name: &str,
        algorithm: String,
        encrypted_value: Vec<u8>,
    ) -> Result<(), DbError> {
        let cmk_id = self
            .column_master_keys
            .iter()
            .find(|k| k.name.eq_ignore_ascii_case(cmk_name))
            .map(|k| k.column_master_key_id)
            .ok_or_else(|| {
                DbError::Execution(format!(
                    "Cannot find the column master key '{}', because it does not exist.",
                    cmk_name
                ))
            })?;
        let list = Arc::make_mut(&mut self.column_encryption_keys);
        let cek = list
            .iter_mut()
            .find(|k| k.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                DbError::Execution(format!(
                    "Cannot alter the column encryption key '{}', because it does not exist.",
                    name
                ))
            })?;
        if add {
            cek.values.push(ColumnEncryptionKeyValueDef {
                column_master_key_id: cmk_id,
                encrypted_value,
                encryption_algorithm_name: algorithm,
            });
        } else {
            cek.values.retain(|v| v.column_master_key_id != cmk_id);
        }
        Ok(())
    }
    fn drop_column_encryption_key(&mut self, name: &str) -> Result<(), DbError> {
        let list = Arc::make_mut(&mut self.column_encryption_keys);
        let before = list.len();
        list.retain(|k| !k.name.eq_ignore_ascii_case(name));
        if list.len() == before {
            return Err(DbError::Execution(format!(
                "Cannot drop the column encryption key '{}', because it does not exist.",
                name
            )));
        }
        Ok(())
    }
    fn get_user_databases(&self) -> &[UserDatabaseDef] {
        &self.user_databases
    }
    fn get_database_options(&self, db_name: &str) -> DatabaseRuntimeOptions {
        if let Some(udb) = self
            .user_databases
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(db_name))
        {
            return DatabaseRuntimeOptions {
                user_access: udb.user_access,
                is_read_only: udb.is_read_only,
                read_committed_snapshot: udb.read_committed_snapshot,
                allow_snapshot_isolation: udb.allow_snapshot_isolation,
                source_database_id: udb.source_database_id,
                recovery_model: udb.recovery_model.clone(),
                compatibility_level: udb.compatibility_level,
            };
        }
        if let Some(bdb) = self
            .builtin_db_options
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(db_name))
        {
            return DatabaseRuntimeOptions {
                user_access: bdb.user_access,
                is_read_only: bdb.is_read_only,
                read_committed_snapshot: bdb.read_committed_snapshot,
                allow_snapshot_isolation: bdb.allow_snapshot_isolation,
                source_database_id: bdb.source_database_id,
                recovery_model: bdb.recovery_model.clone(),
                compatibility_level: bdb.compatibility_level,
            };
        }
        if let Some(def) = crate::executor::database_catalog::builtin_databases()
            .find(|d| d.name.eq_ignore_ascii_case(db_name))
        {
            return DatabaseRuntimeOptions {
                user_access: 0,
                is_read_only: false,
                read_committed_snapshot: false,
                allow_snapshot_isolation: false,
                source_database_id: None,
                recovery_model: def.recovery_model.to_string(),
                compatibility_level: def.compatibility_level,
            };
        }
        DatabaseRuntimeOptions::default()
    }
    fn set_database_option(
        &mut self,
        db_name: &str,
        opt: &crate::ast::DatabaseOptionChange,
    ) -> Result<(), DbError> {
        let apply_opt = |db: &mut UserDatabaseDef, opt: &crate::ast::DatabaseOptionChange| {
            match opt {
                crate::ast::DatabaseOptionChange::UserAccess(v) => db.user_access = *v,
                crate::ast::DatabaseOptionChange::ReadOnly(v) => db.is_read_only = *v,
                crate::ast::DatabaseOptionChange::ReadCommittedSnapshot(v) => {
                    db.read_committed_snapshot = *v
                }
                crate::ast::DatabaseOptionChange::AllowSnapshotIsolation(v) => {
                    db.allow_snapshot_isolation = *v
                }
                crate::ast::DatabaseOptionChange::RecoveryModel(m) => db.recovery_model = m.clone(),
                crate::ast::DatabaseOptionChange::CompatibilityLevel(lvl) => {
                    db.compatibility_level = *lvl
                }
            }
        };
        if self
            .user_databases
            .iter()
            .any(|d| d.name.eq_ignore_ascii_case(db_name))
        {
            let list = Arc::make_mut(&mut self.user_databases);
            if let Some(db) = list
                .iter_mut()
                .find(|d| d.name.eq_ignore_ascii_case(db_name))
            {
                apply_opt(db, opt);
            }
            return Ok(());
        }
        if let Some(bdef) = crate::executor::database_catalog::builtin_databases()
            .find(|d| d.name.eq_ignore_ascii_case(db_name))
        {
            let list = Arc::make_mut(&mut self.builtin_db_options);
            if let Some(db) = list
                .iter_mut()
                .find(|d| d.name.eq_ignore_ascii_case(db_name))
            {
                apply_opt(db, opt);
            } else {
                let mut entry = UserDatabaseDef {
                    id: bdef.id,
                    name: bdef.name.to_string(),
                    compatibility_level: bdef.compatibility_level,
                    recovery_model: bdef.recovery_model.to_string(),
                    user_access: 0,
                    is_read_only: false,
                    read_committed_snapshot: false,
                    allow_snapshot_isolation: false,
                    source_database_id: None,
                };
                apply_opt(&mut entry, opt);
                list.push(entry);
            }
            return Ok(());
        }
        Err(DbError::DatabaseNotFound {
            database: db_name.to_string(),
        })
    }
    fn create_database(&mut self, name: String) -> Result<i32, DbError> {
        if crate::executor::database_catalog::is_builtin_database(&name)
            || self
                .user_databases
                .iter()
                .any(|d| d.name.eq_ignore_ascii_case(&name))
        {
            return Err(DbError::Execution(format!(
                "Database '{}' already exists. Choose a different database name.",
                name
            )));
        }
        let next_id = self.user_databases.iter().map(|d| d.id).max().unwrap_or(5) + 1;
        let dbo_key = crate::executor::database_catalog::db_schema_key(&name, "dbo");
        Arc::make_mut(&mut self.user_databases).push(UserDatabaseDef {
            id: next_id,
            name,
            compatibility_level: 160,
            recovery_model: "FULL".to_string(),
            user_access: 0,
            is_read_only: false,
            read_committed_snapshot: false,
            allow_snapshot_isolation: false,
            source_database_id: None,
        });
        if self.get_schema_id(&dbo_key).is_none() {
            let _ = self.create_schema(&dbo_key);
        }
        Ok(next_id)
    }
    fn clone_database(
        &mut self,
        source_db: &str,
        target_db: String,
        as_snapshot: bool,
    ) -> Result<Vec<(u32, u32)>, DbError> {
        let source_id = crate::executor::database_catalog::database_id_for_name_in(self, source_db)
            .ok_or_else(|| DbError::DatabaseNotFound {
                database: source_db.to_string(),
            })?;
        let source_opts = self.get_database_options(source_db);
        let new_db_id = self.create_database(target_db.clone())?;
        if let Some(udb) = Arc::make_mut(&mut self.user_databases)
            .iter_mut()
            .find(|d| d.id == new_db_id)
        {
            udb.compatibility_level = source_opts.compatibility_level;
            udb.recovery_model = source_opts.recovery_model;
            udb.read_committed_snapshot = source_opts.read_committed_snapshot;
            udb.allow_snapshot_isolation = source_opts.allow_snapshot_isolation;
            udb.is_read_only = as_snapshot;
            udb.source_database_id = if as_snapshot { Some(source_id) } else { None };
        }

        let source_is_user = crate::executor::database_catalog::is_user_database(self, source_db);
        let belongs_to_source = |raw_schema: &str| -> bool {
            let (owner_db, _) = crate::executor::database_catalog::split_db_schema(raw_schema);
            if source_is_user {
                matches!(owner_db, Some(d) if d.eq_ignore_ascii_case(source_db))
            } else {
                owner_db.is_none()
            }
        };
        let remap_schema = |raw_schema: &str| -> String {
            let display = crate::executor::database_catalog::display_schema_name(raw_schema);
            crate::executor::database_catalog::db_schema_key(&target_db, display)
        };

        // 1. Clone schemas
        let src_schemas: Vec<String> = self
            .get_schemas()
            .iter()
            .filter(|s| belongs_to_source(&s.name))
            .map(|s| s.name.clone())
            .collect();
        for sname in src_schemas {
            let mapped = remap_schema(&sname);
            if self.get_schema_id(&mapped).is_none() {
                let _ = self.create_schema(&mapped);
            }
        }

        // 2. Clone tables (skip internal temp/table-var tables) and record (old_id, new_id)
        let src_tables: Vec<TableDef> = self
            .get_tables()
            .iter()
            .filter(|t| {
                belongs_to_source(&t.schema_name)
                    && !t.name.starts_with('#')
                    && !t.name.starts_with("__temp_")
                    && !t.name.starts_with("__var_")
                    && !t.name.starts_with("__tvp_")
            })
            .map(|t| t.as_ref().clone())
            .collect();
        let mut table_id_map: HashMap<u32, u32> = HashMap::new();
        let mut table_pairs: Vec<(u32, u32)> = Vec::with_capacity(src_tables.len());
        for mut tbl in src_tables {
            let old_id = tbl.id;
            let new_id = self.alloc_table_id();
            let mapped_schema = remap_schema(&tbl.schema_name);
            if self.get_schema_id(&mapped_schema).is_none() {
                let _ = self.create_schema(&mapped_schema);
            }
            let mapped_schema_id = self.get_schema_id(&mapped_schema).unwrap_or(tbl.schema_id);
            tbl.id = new_id;
            tbl.schema_name = mapped_schema;
            tbl.schema_id = mapped_schema_id;
            for fk in &mut tbl.foreign_keys {
                let ref_sch = fk.referenced_table.schema_or_dbo().to_string();
                fk.referenced_table.schema = Some(remap_schema(&ref_sch));
            }
            self.register_table(tbl);
            table_id_map.insert(old_id, new_id);
            table_pairs.push((old_id, new_id));
        }

        // 3. Clone indexes for cloned tables
        let src_indexes: Vec<IndexDef> = self
            .get_indexes()
            .iter()
            .filter(|idx| table_id_map.contains_key(&idx.table_id))
            .map(|idx| idx.as_ref().clone())
            .collect();
        for mut idx in src_indexes {
            if let Some(&new_tid) = table_id_map.get(&idx.table_id) {
                idx.id = self.alloc_index_id();
                idx.table_id = new_tid;
                self.register_index(idx);
            }
        }

        #[cfg(feature = "checkpoint")]
        let src_prefix = crate::executor::database_catalog::db_schema_prefix(source_db);
        #[cfg(feature = "checkpoint")]
        let tgt_prefix = crate::executor::database_catalog::db_schema_prefix(&target_db);
        #[cfg(feature = "checkpoint")]
        let remap_serde_prefixes = |val: &mut serde_json::Value| {
            fn walk(v: &mut serde_json::Value, src_p: &str, tgt_p: &str) {
                match v {
                    serde_json::Value::String(s) => {
                        if s.len() >= src_p.len()
                            && s[..src_p.len()].eq_ignore_ascii_case(src_p)
                        {
                            *s = format!("{}{}", tgt_p, &s[src_p.len()..]);
                        }
                    }
                    serde_json::Value::Array(arr) => {
                        for item in arr {
                            walk(item, src_p, tgt_p);
                        }
                    }
                    serde_json::Value::Object(map) => {
                        for item in map.values_mut() {
                            walk(item, src_p, tgt_p);
                        }
                    }
                    _ => {}
                }
            }
            if source_is_user {
                walk(val, &src_prefix, &tgt_prefix);
            }
        };

        // 4. Clone views (remapping internal DB schema prefixes inside AST)
        let src_views: Vec<ViewDef> = self
            .get_views()
            .iter()
            .filter(|v| belongs_to_source(&v.schema))
            .map(|v| v.as_ref().clone())
            .collect();
        for mut v in src_views {
            v.schema = remap_schema(&v.schema);
            #[cfg(feature = "checkpoint")]
            if let Ok(mut jv) = serde_json::to_value(&*v.query) {
                remap_serde_prefixes(&mut jv);
                if let Ok(remapped) = serde_json::from_value::<Statement>(jv) {
                    v.query = Arc::new(remapped);
                }
            }
            let _ = self.create_view(v);
        }

        // 5. Clone routines (remapping internal DB schema prefixes inside AST)
        let src_routines: Vec<RoutineDef> = self
            .get_routines()
            .iter()
            .filter(|r| belongs_to_source(&r.schema))
            .map(|r| r.as_ref().clone())
            .collect();
        for mut r in src_routines {
            r.schema = remap_schema(&r.schema);
            #[cfg(feature = "checkpoint")]
            if let Ok(mut jv) = serde_json::to_value(&r.kind) {
                remap_serde_prefixes(&mut jv);
                if let Ok(remapped) = serde_json::from_value::<RoutineKind>(jv) {
                    r.kind = remapped;
                }
            }
            let _ = self.create_routine(r);
        }

        // 6. Clone triggers (remapping internal DB schema prefixes inside AST)
        let src_triggers: Vec<TriggerDef> = self
            .get_triggers()
            .iter()
            .filter(|t| belongs_to_source(&t.schema))
            .map(|t| t.as_ref().clone())
            .collect();
        for mut tr in src_triggers {
            tr.schema = remap_schema(&tr.schema);
            tr.table_schema = remap_schema(&tr.table_schema);
            #[cfg(feature = "checkpoint")]
            if let Ok(mut jv) = serde_json::to_value(&*tr.body) {
                remap_serde_prefixes(&mut jv);
                if let Ok(remapped) = serde_json::from_value::<Vec<Statement>>(jv) {
                    tr.body = Arc::new(remapped);
                }
            }
            let _ = self.create_trigger(tr);
        }

        // 7. Clone sequences
        let src_seqs: Vec<SequenceDef> = self
            .get_sequences()
            .iter()
            .filter(|s| belongs_to_source(&s.schema))
            .cloned()
            .collect();
        for mut seq in src_seqs {
            seq.schema = remap_schema(&seq.schema);
            let _ = self.create_sequence(seq);
        }

        Ok(table_pairs)
    }
    fn drop_database(&mut self, name: &str, if_exists: bool) -> Result<Vec<u32>, DbError> {
        if crate::executor::database_catalog::is_builtin_database(name) {
            return Err(DbError::Execution(format!(
                "Cannot drop the system database '{}'.",
                name
            )));
        }
        let exists = self
            .user_databases
            .iter()
            .any(|d| d.name.eq_ignore_ascii_case(name));
        if !exists {
            if if_exists {
                return Ok(Vec::new());
            }
            return Err(DbError::Execution(format!(
                "Cannot drop the database '{}', because it does not exist or you do not have permission.",
                name
            )));
        }
        let prefix = crate::executor::database_catalog::db_schema_prefix(name);
        let matches_db = |schema: &str| schema.to_ascii_lowercase().starts_with(&prefix);

        let tables_to_drop: Vec<(String, String, u32)> = self
            .get_tables()
            .iter()
            .filter(|t| matches_db(&t.schema_name))
            .map(|t| (t.schema_name.clone(), t.name.clone(), t.id))
            .collect();
        let mut dropped_table_ids = Vec::with_capacity(tables_to_drop.len());
        for (schema, tname, tid) in tables_to_drop {
            let _ = self.drop_table(&schema, &tname);
            dropped_table_ids.push(tid);
        }

        let views_to_drop: Vec<(String, String)> = self
            .get_views()
            .iter()
            .filter(|v| matches_db(&v.schema))
            .map(|v| (v.schema.clone(), v.name.clone()))
            .collect();
        for (schema, vname) in views_to_drop {
            let _ = self.drop_view(&schema, &vname);
        }

        let routines_to_drop: Vec<(String, String, bool)> = self
            .get_routines()
            .iter()
            .filter(|r| matches_db(&r.schema))
            .map(|r| {
                (
                    r.schema.clone(),
                    r.name.clone(),
                    matches!(r.kind, RoutineKind::Procedure { .. }),
                )
            })
            .collect();
        for (schema, rname, is_proc) in routines_to_drop {
            let _ = self.drop_routine(&schema, &rname, is_proc);
        }

        let triggers_to_drop: Vec<(String, String)> = self
            .get_triggers()
            .iter()
            .filter(|t| matches_db(&t.schema))
            .map(|t| (t.schema.clone(), t.name.clone()))
            .collect();
        for (schema, trname) in triggers_to_drop {
            let _ = self.drop_trigger(&schema, &trname);
        }

        let schemas_to_drop: Vec<String> = self
            .get_schemas()
            .iter()
            .filter(|s| matches_db(&s.name))
            .map(|s| s.name.clone())
            .collect();
        for sname in schemas_to_drop {
            let _ = self.drop_schema(&sname);
        }

        Arc::make_mut(&mut self.user_databases).retain(|d| !d.name.eq_ignore_ascii_case(name));
        Ok(dropped_table_ids)
    }
    fn set_column_mask(
        &mut self,
        table_id: u32,
        column_name: &str,
        masking_function: Option<String>,
    ) {
        let list = Arc::make_mut(&mut self.masked_columns);
        list.retain(|m| {
            !(m.table_id == table_id && m.column_name.eq_ignore_ascii_case(column_name))
        });
        if let Some(func) = masking_function {
            list.push(MaskedColumnDef {
                table_id,
                column_name: column_name.to_string(),
                masking_function: func,
            });
        }
    }
    fn remove_table_masks(&mut self, table_id: u32) {
        if self.masked_columns.iter().any(|m| m.table_id == table_id) {
            Arc::make_mut(&mut self.masked_columns).retain(|m| m.table_id != table_id);
        }
    }
    fn get_security_policies(&self) -> &[SecurityPolicyDef] {
        &self.security_policies
    }
    fn upsert_security_policy(&mut self, policy: SecurityPolicyDef) -> Result<(), DbError> {
        let list = Arc::make_mut(&mut self.security_policies);
        if let Some(existing) = list.iter_mut().find(|p| {
            p.schema.eq_ignore_ascii_case(&policy.schema)
                && p.name.eq_ignore_ascii_case(&policy.name)
        }) {
            *existing = policy;
        } else {
            list.push(policy);
        }
        Ok(())
    }
    fn drop_security_policy(
        &mut self,
        schema: &str,
        name: &str,
        if_exists: bool,
    ) -> Result<(), DbError> {
        let idx = self.security_policies.iter().position(|p| {
            p.schema.eq_ignore_ascii_case(schema) && p.name.eq_ignore_ascii_case(name)
        });
        match idx {
            Some(i) => {
                Arc::make_mut(&mut self.security_policies).remove(i);
                Ok(())
            }
            None if if_exists => Ok(()),
            None => Err(DbError::Execution(format!(
                "Cannot drop the security policy '{}.{}', because it does not exist or you do not have permission.",
                schema, name
            ))),
        }
    }

    fn clone_boxed(&self) -> Box<dyn Catalog> {
        Box::new(self.clone())
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }

    fn restore_from_boxed(&mut self, snapshot: Box<dyn Catalog>) {
        if let Ok(other) = snapshot.into_any().downcast::<CatalogImpl>() {
            *self = *other;
        }
    }

    fn rebuild_maps(&mut self) {
        self.rebuild_maps();
    }

    fn has_schemabound_objects(&self) -> bool {
        self.schemabound_view_count > 0 || self.schemabound_routine_count > 0
    }

    fn merge_workspace(&mut self, workspace: Self, base: &Self) {
        self.tables
            .merge_delta_against_base(&workspace.tables, &base.tables, |t| {
                (t.schema_id, normalize_identifier(&t.name))
            });
        self.routines
            .merge_delta_against_base(&workspace.routines, &base.routines, |r| {
                (
                    normalize_identifier(&r.schema),
                    normalize_identifier(&r.name),
                )
            });
        self.views
            .merge_delta_against_base(&workspace.views, &base.views, |v| {
                (
                    normalize_identifier(&v.schema),
                    normalize_identifier(&v.name),
                )
            });
        self.triggers
            .merge_delta_against_base(&workspace.triggers, &base.triggers, |t| {
                (
                    normalize_identifier(&t.schema),
                    normalize_identifier(&t.name),
                )
            });
        self.recompute_schemabound_counts();
        self.rebuild_triggers_by_table();

        self.schemas
            .merge_delta_against_base(&workspace.schemas, &base.schemas, |s| {
                normalize_identifier(&s.name)
            });
        self.indexes
            .merge_delta_against_base(&workspace.indexes, &base.indexes, |i| {
                (i.schema_id, i.table_id, normalize_identifier(&i.name))
            });
        if merge_arc_vec(
            &mut self.table_types,
            &workspace.table_types,
            &base.table_types,
            |t| {
                (
                    normalize_identifier(&t.schema),
                    normalize_identifier(&t.name),
                )
            },
        ) {
            self.type_map = Arc::new(
                self.table_types
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        (
                            (
                                normalize_identifier(&t.schema),
                                normalize_identifier(&t.name),
                            ),
                            i,
                        )
                    })
                    .collect(),
            );
        }
        if merge_arc_vec(
            &mut self.scalar_types,
            &workspace.scalar_types,
            &base.scalar_types,
            |t| {
                (
                    normalize_identifier(&t.schema),
                    normalize_identifier(&t.name),
                )
            },
        ) {
            self.scalar_type_map = Arc::new(
                self.scalar_types
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        (
                            (
                                normalize_identifier(&t.schema),
                                normalize_identifier(&t.name),
                            ),
                            i,
                        )
                    })
                    .collect(),
            );
        }
        if merge_arc_vec(
            &mut self.synonyms,
            &workspace.synonyms,
            &base.synonyms,
            |s| {
                (
                    normalize_identifier(&s.schema),
                    normalize_identifier(&s.name),
                )
            },
        ) {
            self.synonym_map = Arc::new(
                self.synonyms
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        (
                            (
                                normalize_identifier(&s.schema),
                                normalize_identifier(&s.name),
                            ),
                            i,
                        )
                    })
                    .collect(),
            );
        }
        if merge_arc_vec(
            &mut self.sequences,
            &workspace.sequences,
            &base.sequences,
            |s| {
                (
                    normalize_identifier(&s.schema),
                    normalize_identifier(&s.name),
                )
            },
        ) {
            self.sequence_map = Arc::new(
                self.sequences
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        (
                            (
                                normalize_identifier(&s.schema),
                                normalize_identifier(&s.name),
                            ),
                            i,
                        )
                    })
                    .collect(),
            );
        }
        merge_arc_vec(
            &mut self.extended_properties,
            &workspace.extended_properties,
            &base.extended_properties,
            |p| {
                (
                    p.class,
                    p.major_id,
                    p.minor_id,
                    normalize_identifier(&p.name),
                )
            },
        );
        merge_arc_vec(
            &mut self.principals,
            &workspace.principals,
            &base.principals,
            |p| normalize_identifier(&p.name),
        );
        merge_arc_vec(
            &mut self.role_members,
            &workspace.role_members,
            &base.role_members,
            |m| (m.role_principal_id, m.member_principal_id),
        );
        merge_arc_vec(
            &mut self.permissions,
            &workspace.permissions,
            &base.permissions,
            |p| {
                (
                    p.class,
                    p.major_id,
                    p.minor_id,
                    p.grantee_principal_id,
                    p.permission_name.to_ascii_uppercase(),
                )
            },
        );
        merge_arc_vec(
            &mut self.partition_functions,
            &workspace.partition_functions,
            &base.partition_functions,
            |p| normalize_identifier(&p.name),
        );
        merge_arc_vec(
            &mut self.partition_schemes,
            &workspace.partition_schemes,
            &base.partition_schemes,
            |s| normalize_identifier(&s.name),
        );
        merge_arc_vec(
            &mut self.filegroups,
            &workspace.filegroups,
            &base.filegroups,
            |f| normalize_identifier(&f.name),
        );
        merge_arc_vec(
            &mut self.table_large_value_out_of_row,
            &workspace.table_large_value_out_of_row,
            &base.table_large_value_out_of_row,
            |id| *id,
        );
        merge_arc_vec(
            &mut self.masked_columns,
            &workspace.masked_columns,
            &base.masked_columns,
            |m| (m.table_id, normalize_identifier(&m.column_name)),
        );
        merge_arc_vec(
            &mut self.column_master_keys,
            &workspace.column_master_keys,
            &base.column_master_keys,
            |k| normalize_identifier(&k.name),
        );
        merge_arc_vec(
            &mut self.column_encryption_keys,
            &workspace.column_encryption_keys,
            &base.column_encryption_keys,
            |k| normalize_identifier(&k.name),
        );
        merge_arc_vec(
            &mut self.user_databases,
            &workspace.user_databases,
            &base.user_databases,
            |d| normalize_identifier(&d.name),
        );
        merge_arc_vec(
            &mut self.builtin_db_options,
            &workspace.builtin_db_options,
            &base.builtin_db_options,
            |d| normalize_identifier(&d.name),
        );
        merge_arc_vec(
            &mut self.security_policies,
            &workspace.security_policies,
            &base.security_policies,
            |p| {
                (
                    normalize_identifier(&p.schema),
                    normalize_identifier(&p.name),
                )
            },
        );

        // Advance atomic counters defensively if workspace came from a distinct allocator.
        let _ = self.next_schema_id.fetch_max(
            workspace.next_schema_id.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        let _ = self.next_table_id.fetch_max(
            workspace.next_table_id.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        let _ = self.next_column_id.fetch_max(
            workspace.next_column_id.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        let _ = self.next_index_id.fetch_max(
            workspace.next_index_id.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        let _ = self.next_object_id.fetch_min(
            workspace.next_object_id.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
    }
}
