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
    FilegroupDef, PartitionFunctionDef, PartitionSchemeDef, PermissionDef, PrincipalDef,
    RoleMemberDef,
};

use crate::ast::Expr;
use crate::ast::{DataTypeSpec, FunctionBody, RoutineParam, Statement, TriggerEvent};
use crate::error::DbError;
use crate::executor::string_norm::normalize_identifier;
use crate::types::DataType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaDef {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeyDef {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_table: crate::ast::ObjectName,
    pub referenced_columns: Vec<String>,
    pub on_delete: crate::ast::ReferentialAction,
    pub on_update: crate::ast::ReferentialAction,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default = "default_ansi_padding_on")]
    pub ansi_padding_on: bool,
    #[serde(default)]
    pub is_rowversion: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckConstraintDef {
    pub name: String,
    pub expr: Expr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDef {
    pub id: u32,
    pub schema_id: u32,
    pub table_id: u32,
    pub name: String,
    pub column_ids: Vec<u32>,
    pub is_unique: bool,
    pub is_clustered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoutineKind {
    Procedure {
        #[serde(with = "serde_arc")]
        body: std::sync::Arc<Vec<Statement>>,
    },
    Function {
        returns: Option<DataTypeSpec>,
        body: Box<FunctionBody>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineDef {
    #[serde(default)]
    pub object_id: i32,
    pub schema: String,
    pub name: String,
    pub params: Vec<RoutineParam>,
    pub kind: RoutineKind,
    #[serde(default)]
    pub definition_sql: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableTypeDef {
    #[serde(default)]
    pub object_id: i32,
    pub schema: String,
    pub name: String,
    pub columns: Vec<crate::ast::ColumnSpec>,
    pub table_constraints: Vec<crate::ast::TableConstraintSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalarTypeDef {
    #[serde(default)]
    pub object_id: i32,
    pub schema: String,
    pub name: String,
    pub base_type: DataType,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewDef {
    #[serde(default)]
    pub object_id: i32,
    pub schema: String,
    pub name: String,
    #[serde(default)]
    pub schema_id: u32,
    #[serde(with = "serde_arc")]
    pub query: std::sync::Arc<Statement>, // Should be Statement::Select
    #[serde(default)]
    pub definition_sql: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerDef {
    #[serde(default)]
    pub object_id: i32,
    pub schema: String,
    pub name: String,
    pub table_schema: String,
    pub table_name: String,
    pub events: Vec<TriggerEvent>,
    pub is_instead_of: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(with = "serde_arc")]
    pub body: std::sync::Arc<Vec<Statement>>,
    #[serde(default)]
    pub definition_sql: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynonymDef {
    #[serde(default)]
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

impl Serialize for SeqCounter {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.load().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SeqCounter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        i64::deserialize(deserializer).map(Self::new)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceDef {
    #[serde(default)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    fn get_schemas(&self) -> &[SchemaDef];
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
    fn get_indexes(&self) -> &[IndexDef];
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

    fn clone_boxed(&self) -> Box<dyn Catalog>;
    fn rebuild_maps(&mut self) {}
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogImpl {
    #[serde(with = "arc_serde")]
    schemas: Arc<Vec<SchemaDef>>,
    tables: CowRegistry<(u32, String), TableDef>,
    #[serde(with = "arc_serde")]
    indexes: Arc<Vec<IndexDef>>,
    routines: CowRegistry<(String, String), RoutineDef>,
    #[serde(with = "arc_serde")]
    table_types: Arc<Vec<TableTypeDef>>,
    #[serde(default, with = "arc_serde")]
    scalar_types: Arc<Vec<ScalarTypeDef>>,
    views: CowRegistry<(String, String), ViewDef>,
    triggers: CowRegistry<(String, String), TriggerDef>,
    #[serde(with = "arc_serde")]
    synonyms: Arc<Vec<SynonymDef>>,
    #[serde(with = "arc_serde")]
    sequences: Arc<Vec<SequenceDef>>,
    #[serde(default, with = "arc_serde")]
    extended_properties: Arc<Vec<ExtendedPropertyDef>>,
    #[serde(default, with = "arc_serde")]
    principals: Arc<Vec<PrincipalDef>>,
    #[serde(default, with = "arc_serde")]
    role_members: Arc<Vec<RoleMemberDef>>,
    #[serde(default, with = "arc_serde")]
    permissions: Arc<Vec<PermissionDef>>,
    #[serde(default, with = "arc_serde")]
    partition_functions: Arc<Vec<PartitionFunctionDef>>,
    #[serde(default, with = "arc_serde")]
    partition_schemes: Arc<Vec<PartitionSchemeDef>>,
    #[serde(default, with = "arc_serde")]
    filegroups: Arc<Vec<FilegroupDef>>,
    #[serde(default, with = "arc_serde")]
    table_large_value_out_of_row: Arc<Vec<u32>>,
    #[serde(with = "atomic_u32_serde")]
    next_schema_id: Arc<AtomicU32>,
    #[serde(with = "atomic_u32_serde")]
    next_table_id: Arc<AtomicU32>,
    #[serde(with = "atomic_u32_serde")]
    next_column_id: Arc<AtomicU32>,
    #[serde(with = "atomic_u32_serde")]
    next_index_id: Arc<AtomicU32>,
    #[serde(default = "default_next_object_id", with = "atomic_i32_serde")]
    next_object_id: Arc<AtomicI32>,
    #[serde(default = "default_next_rowversion", with = "atomic_u64_serde")]
    next_rowversion: Arc<AtomicU64>,

    #[serde(skip)]
    schema_map: Arc<HashMap<String, usize>>,
    #[serde(skip)]
    type_map: Arc<HashMap<(String, String), usize>>,
    #[serde(skip)]
    scalar_type_map: Arc<HashMap<(String, String), usize>>,
    #[serde(skip)]
    synonym_map: Arc<HashMap<(String, String), usize>>,
    #[serde(skip)]
    sequence_map: Arc<HashMap<(String, String), usize>>,
    #[serde(skip)]
    index_map: Arc<HashMap<(u32, String), usize>>,
}

impl CatalogImpl {
    pub fn new() -> Self {
        let mut c = Self::default();
        let dbo_id = c.alloc_schema_id();
        Arc::make_mut(&mut c.schemas).push(SchemaDef {
            id: dbo_id,
            name: "dbo".to_string(),
        });
        c.rebuild_maps();
        c
    }

    pub fn rebuild_maps(&mut self) {
        self.schema_map = Arc::new(
            self.schemas
                .iter()
                .enumerate()
                .map(|(i, s)| (normalize_identifier(&s.name), i))
                .collect(),
        );
        self.tables
            .rebuild_map(|t| (t.schema_id, normalize_identifier(&t.name)));
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
        self.index_map = Arc::new(
            self.indexes
                .iter()
                .enumerate()
                .map(|(i, idx)| ((idx.schema_id, normalize_identifier(&idx.name)), i))
                .collect(),
        );
    }

    /// Remove any indexes belonging to `table_id`, cloning `indexes` only if needed.
    pub(crate) fn remove_indexes_for_table_id(&mut self, table_id: u32) {
        if self.indexes.iter().any(|i| i.table_id == table_id) {
            let indexes = Arc::make_mut(&mut self.indexes);
            indexes.retain(|i| i.table_id != table_id);
            self.index_map = Arc::new(
                indexes
                    .iter()
                    .enumerate()
                    .map(|(i, idx)| ((idx.schema_id, normalize_identifier(&idx.name)), i))
                    .collect(),
            );
        }
    }
}

impl Default for CatalogImpl {
    fn default() -> Self {
        Self {
            schemas: Arc::new(Vec::new()),
            tables: CowRegistry::default(),
            indexes: Arc::new(Vec::new()),
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
            next_schema_id: Arc::new(AtomicU32::new(1)),
            next_table_id: Arc::new(AtomicU32::new(1234567890)),
            next_column_id: Arc::new(AtomicU32::new(1)),
            next_index_id: Arc::new(AtomicU32::new(234567890)),
            next_object_id: Arc::new(AtomicI32::new(-1)),
            next_rowversion: default_next_rowversion(),
            schema_map: Arc::new(HashMap::new()),
            type_map: Arc::new(HashMap::new()),
            scalar_type_map: Arc::new(HashMap::new()),
            synonym_map: Arc::new(HashMap::new()),
            sequence_map: Arc::new(HashMap::new()),
            index_map: Arc::new(HashMap::new()),
        }
    }
}

fn default_next_object_id() -> Arc<AtomicI32> {
    Arc::new(AtomicI32::new(-1))
}

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

    fn clone_boxed(&self) -> Box<dyn Catalog> {
        Box::new(self.clone())
    }

    fn rebuild_maps(&mut self) {
        self.rebuild_maps();
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

        if merge_arc_vec(&mut self.schemas, &workspace.schemas, &base.schemas, |s| {
            normalize_identifier(&s.name)
        }) {
            self.schema_map = Arc::new(
                self.schemas
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (normalize_identifier(&s.name), i))
                    .collect(),
            );
        }
        if merge_arc_vec(&mut self.indexes, &workspace.indexes, &base.indexes, |i| {
            (i.schema_id, normalize_identifier(&i.name))
        }) {
            self.index_map = Arc::new(
                self.indexes
                    .iter()
                    .enumerate()
                    .map(|(i, idx)| ((idx.schema_id, normalize_identifier(&idx.name)), i))
                    .collect(),
            );
        }
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
