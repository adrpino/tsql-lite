mod id_allocator;
mod index_registry;
mod object_resolver;
mod routine_registry;
mod schema_registry;
mod sequence_registry;
mod synonym_registry;
mod table_registry;
mod trigger_registry;
mod type_registry;
mod view_registry;

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
        body: FunctionBody,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceDef {
    #[serde(default)]
    pub object_id: i32,
    pub schema: String,
    pub name: String,
    pub data_type: DataType,
    pub start_value: i64,
    pub increment: i64,
    pub current_value: i64,
    pub minimum_value: i64,
    pub maximum_value: i64,
    pub is_cycling: bool,
}

pub trait IdAllocator {
    fn alloc_table_id(&mut self) -> u32;
    fn alloc_object_id(&mut self) -> i32;
    fn alloc_column_id(&mut self) -> u32;
    fn alloc_index_id(&mut self) -> u32;
    fn alloc_schema_id(&mut self) -> u32;
}

pub trait SchemaRegistry {
    fn get_schemas(&self) -> &[SchemaDef];
    fn get_schema_id(&self, name: &str) -> Option<u32>;
    fn create_schema(&mut self, name: &str) -> Result<(), DbError>;
    fn drop_schema(&mut self, name: &str) -> Result<(), DbError>;
}

pub trait TableRegistry {
    fn get_tables(&self) -> &[Arc<TableDef>];
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
    fn get_routines(&self) -> &[Arc<RoutineDef>];
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
}

pub trait ViewRegistry {
    fn get_views(&self) -> &[Arc<ViewDef>];
    fn find_view(&self, schema: &str, name: &str) -> Option<&ViewDef>;
    fn create_view(&mut self, view: ViewDef) -> Result<(), DbError>;
    fn drop_view(&mut self, schema: &str, name: &str) -> Result<(), DbError>;
}

pub trait TriggerRegistry {
    fn get_triggers(&self) -> &[Arc<TriggerDef>];
    fn find_triggers_for_table(&self, schema: &str, name: &str) -> Vec<&TriggerDef>;
    fn create_trigger(&mut self, trigger: TriggerDef) -> Result<(), DbError>;
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
    fn drop_sequence(&mut self, schema: &str, name: &str) -> Result<(), DbError>;
    fn next_sequence_value(&mut self, schema: &str, name: &str) -> Result<i64, DbError>;
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
    fn clone_boxed(&self) -> Box<dyn Catalog>;
    fn rebuild_maps(&mut self) {}
}

use std::collections::HashMap;
use std::sync::Arc;

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

mod arc_vec_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::Arc;

    pub fn serialize<S, T>(val: &Arc<Vec<Arc<T>>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(val.len()))?;
        for item in val.iter() {
            seq.serialize_element(item.as_ref())?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Arc<Vec<Arc<T>>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        let v: Vec<T> = Vec::deserialize(deserializer)?;
        Ok(Arc::new(v.into_iter().map(Arc::new).collect()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogImpl {
    #[serde(with = "arc_serde")]
    schemas: Arc<Vec<SchemaDef>>,
    #[serde(with = "arc_vec_serde")]
    tables: Arc<Vec<Arc<TableDef>>>,
    #[serde(with = "arc_serde")]
    indexes: Arc<Vec<IndexDef>>,
    #[serde(with = "arc_vec_serde")]
    routines: Arc<Vec<Arc<RoutineDef>>>,
    #[serde(with = "arc_serde")]
    table_types: Arc<Vec<TableTypeDef>>,
    #[serde(with = "arc_vec_serde")]
    views: Arc<Vec<Arc<ViewDef>>>,
    #[serde(with = "arc_vec_serde")]
    triggers: Arc<Vec<Arc<TriggerDef>>>,
    #[serde(with = "arc_serde")]
    synonyms: Arc<Vec<SynonymDef>>,
    #[serde(with = "arc_serde")]
    sequences: Arc<Vec<SequenceDef>>,
    next_schema_id: u32,
    next_table_id: u32,
    next_column_id: u32,
    next_index_id: u32,
    #[serde(default = "default_next_object_id")]
    next_object_id: i32,

    #[serde(skip)]
    schema_map: Arc<HashMap<String, usize>>,
    #[serde(skip)]
    table_map: Arc<HashMap<(u32, String), usize>>,
    #[serde(skip)]
    routine_map: Arc<HashMap<(String, String), usize>>,
    #[serde(skip)]
    type_map: Arc<HashMap<(String, String), usize>>,
    #[serde(skip)]
    view_map: Arc<HashMap<(String, String), usize>>,
    #[serde(skip)]
    trigger_map: Arc<HashMap<(String, String), usize>>,
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
        self.table_map = Arc::new(
            self.tables
                .iter()
                .enumerate()
                .map(|(i, t)| ((t.schema_id, normalize_identifier(&t.name)), i))
                .collect(),
        );
        self.routine_map = Arc::new(
            self.routines
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    (
                        (
                            normalize_identifier(&r.schema),
                            normalize_identifier(&r.name),
                        ),
                        i,
                    )
                })
                .collect(),
        );
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
        self.view_map = Arc::new(
            self.views
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    (
                        (
                            normalize_identifier(&v.schema),
                            normalize_identifier(&v.name),
                        ),
                        i,
                    )
                })
                .collect(),
        );
        self.trigger_map = Arc::new(
            self.triggers
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
        self.index_map = Arc::new(
            self.indexes
                .iter()
                .enumerate()
                .map(|(i, idx)| ((idx.schema_id, normalize_identifier(&idx.name)), i))
                .collect(),
        );
    }

    /// Remove the table at `idx` using swap_remove (O(1)) and fix up only the
    /// table_map and index_map entries that were affected.
    pub(crate) fn remove_table_at(&mut self, idx: usize) {
        let tables = Arc::make_mut(&mut self.tables);
        let removed = tables.swap_remove(idx);
        let table_map = Arc::make_mut(&mut self.table_map);
        table_map.remove(&(removed.schema_id, normalize_identifier(&removed.name)));

        // If swap_remove moved the last element into `idx`, update its map entry.
        if idx < tables.len() {
            let swapped = &tables[idx];
            table_map.insert(
                (swapped.schema_id, normalize_identifier(&swapped.name)),
                idx,
            );
        }

        // Only clone indexes if any actually belonged to this table.
        if self.indexes.iter().any(|i| i.table_id == removed.id) {
            let indexes = Arc::make_mut(&mut self.indexes);
            indexes.retain(|i| i.table_id != removed.id);
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
            tables: Arc::new(Vec::new()),
            indexes: Arc::new(Vec::new()),
            routines: Arc::new(Vec::new()),
            table_types: Arc::new(Vec::new()),
            views: Arc::new(Vec::new()),
            triggers: Arc::new(Vec::new()),
            synonyms: Arc::new(Vec::new()),
            sequences: Arc::new(Vec::new()),
            next_schema_id: 1,
            next_table_id: 1234567890,
            next_column_id: 1,
            next_index_id: 234567890,
            next_object_id: -1,
            schema_map: Arc::new(HashMap::new()),
            table_map: Arc::new(HashMap::new()),
            routine_map: Arc::new(HashMap::new()),
            type_map: Arc::new(HashMap::new()),
            view_map: Arc::new(HashMap::new()),
            trigger_map: Arc::new(HashMap::new()),
            synonym_map: Arc::new(HashMap::new()),
            sequence_map: Arc::new(HashMap::new()),
            index_map: Arc::new(HashMap::new()),
        }
    }
}

fn default_next_object_id() -> i32 {
    -1
}

fn default_ansi_padding_on() -> bool {
    true
}

impl Catalog for CatalogImpl {
    fn clone_boxed(&self) -> Box<dyn Catalog> {
        Box::new(self.clone())
    }

    fn rebuild_maps(&mut self) {
        self.rebuild_maps();
    }
}
