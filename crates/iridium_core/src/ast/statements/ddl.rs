use crate::ast::data_types::DataTypeSpec;
use crate::ast::expressions::Expr;
use crate::ast::statements::query::SelectStmt;
use crate::ast::ObjectName;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTableStmt {
    pub name: ObjectName,
    pub columns: Vec<ColumnSpec>,
    pub table_constraints: Vec<TableConstraintSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSynonymStmt {
    pub name: ObjectName,
    pub base_object: ObjectName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropSynonymStmt {
    pub name: ObjectName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSequenceStmt {
    pub name: ObjectName,
    pub data_type: DataTypeSpec,
    pub start_value: i64,
    pub increment: i64,
    pub minimum_value: i64,
    pub maximum_value: i64,
    pub is_cycling: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropSequenceStmt {
    pub name: ObjectName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlterSequenceStmt {
    pub name: ObjectName,
    pub restart: Option<Option<i64>>,
    pub increment: Option<i64>,
    pub minimum_value: Option<Option<i64>>,
    pub maximum_value: Option<Option<i64>>,
    pub is_cycling: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CatalogMetadataAction {
    CreateUser {
        name: String,
        default_schema: Option<String>,
    },
    DropUser {
        name: String,
        if_exists: bool,
    },
    CreateRole {
        name: String,
    },
    DropRole {
        name: String,
        if_exists: bool,
    },
    AlterRoleMember {
        role: String,
        member: String,
        add: bool,
    },
    Permission {
        action: String,
        permissions: Vec<String>,
        target_class: Option<String>,
        target_name: Option<ObjectName>,
        grantees: Vec<String>,
        with_grant_option: bool,
    },
    CreatePartitionFunction {
        name: String,
        data_type: DataTypeSpec,
        range_right: bool,
        boundary_values: Vec<String>,
    },
    DropPartitionFunction {
        name: String,
    },
    CreatePartitionScheme {
        name: String,
        function_name: String,
        all_to: bool,
        filegroups: Vec<String>,
    },
    DropPartitionScheme {
        name: String,
    },
    AddFilegroup {
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropTableStmt {
    pub name: ObjectName,
    #[serde(default)]
    pub if_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIndexStmt {
    pub name: ObjectName,
    pub table: ObjectName,
    pub is_unique: bool,
    pub is_clustered: bool,
    pub columns: Vec<IndexColumnSpec>,
    pub options: Vec<IndexOptionSpec>,
    #[serde(default)]
    pub filter_predicate: Option<Box<Expr>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexColumnSpec {
    pub name: String,
    pub is_desc: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexOptionSpec {
    FillFactor(u8),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropIndexStmt {
    pub name: ObjectName,
    pub table: ObjectName,
    #[serde(default)]
    pub if_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSchemaStmt {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropSchemaStmt {
    pub name: String,
    #[serde(default)]
    pub if_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlterSchemaTransferStmt {
    pub target_schema: String,
    pub entity_type: Option<String>,
    pub source: ObjectName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruncateTableStmt {
    pub name: ObjectName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlterTableStmt {
    pub table: ObjectName,
    pub action: AlterTableAction,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlterTableAction {
    AddColumn(ColumnSpec),
    DropColumn(String),
    AlterColumn {
        name: String,
        data_type: DataTypeSpec,
        nullable: Option<bool>,
    },
    AddConstraint {
        constraint: TableConstraintSpec,
        #[serde(default)]
        with_check: Option<bool>,
    },
    DropConstraint(String),
    CheckConstraint {
        check: bool,
        #[serde(default)]
        with_check: Option<bool>,
        name: Option<String>,
    },
    ToggleTrigger {
        trigger_name: Option<ObjectName>,
        enable: bool,
    },
    SetSystemVersioning {
        enabled: bool,
        history_table: Option<ObjectName>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateViewStmt {
    pub name: ObjectName,
    #[serde(default)]
    pub ctes: Vec<crate::ast::statements::procedural::CteDef>,
    pub query: SelectStmt,
    #[serde(default)]
    pub or_alter: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropViewStmt {
    pub name: ObjectName,
    #[serde(default)]
    pub if_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTypeStmt {
    pub name: ObjectName,
    pub columns: Vec<ColumnSpec>,
    pub table_constraints: Vec<TableConstraintSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateScalarTypeStmt {
    pub name: ObjectName,
    pub base_type: DataTypeSpec,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropTypeStmt {
    pub name: ObjectName,
    #[serde(default)]
    pub if_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnSpec {
    pub name: String,
    pub data_type: DataTypeSpec,
    pub nullable: bool,
    pub nullable_explicit: bool,
    pub primary_key: bool,
    pub unique: bool,
    pub identity: Option<(i64, i64)>,
    pub default: Option<Expr>,
    pub default_constraint_name: Option<String>,
    pub check: Option<Expr>,
    pub check_constraint_name: Option<String>,
    pub computed_expr: Option<Expr>,
    pub foreign_key: Option<ForeignKeyRef>,
    pub collation: Option<String>,
    pub is_clustered: bool,
    pub ansi_padding_on: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferentialAction {
    NoAction,
    Cascade,
    SetNull,
    SetDefault,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeyRef {
    pub referenced_table: ObjectName,
    pub referenced_columns: Vec<String>,
    pub on_delete: Option<ReferentialAction>,
    pub on_update: Option<ReferentialAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TableConstraintSpec {
    Default {
        name: String,
        column: String,
        expr: Expr,
    },
    Check {
        name: String,
        expr: Expr,
    },
    ForeignKey {
        name: String,
        columns: Vec<String>,
        referenced_table: ObjectName,
        referenced_columns: Vec<String>,
        on_delete: Option<ReferentialAction>,
        on_update: Option<ReferentialAction>,
    },
    PrimaryKey {
        name: String,
        columns: Vec<IndexColumnSpec>,
        is_clustered: bool,
    },
    Unique {
        name: String,
        columns: Vec<IndexColumnSpec>,
        is_clustered: bool,
    },
    SystemTimePeriod {
        start_col: String,
        end_col: String,
    },
    SystemVersioning {
        enabled: bool,
        history_table: Option<ObjectName>,
    },
}
