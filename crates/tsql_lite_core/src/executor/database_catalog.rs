use crate::catalog::Catalog;
use crate::executor::context::ExecutionContext;

pub(crate) const DB_SCHEMA_SEP: char = '\x01';

#[derive(Debug, Clone, Copy)]
pub(crate) struct DatabaseDef {
    pub id: i32,
    pub name: &'static str,
    pub compatibility_level: u8,
    pub recovery_model: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) struct DatabaseEntry {
    pub id: i32,
    pub name: String,
    pub compatibility_level: u8,
    pub recovery_model: String,
    pub user_access: u8,
    pub is_read_only: bool,
    pub read_committed_snapshot: bool,
    pub allow_snapshot_isolation: bool,
    pub source_database_id: Option<i32>,
}

const BUILTIN_DATABASES: &[DatabaseDef] = &[
    DatabaseDef {
        id: 1,
        name: "master",
        compatibility_level: 160,
        recovery_model: "FULL",
    },
    DatabaseDef {
        id: 2,
        name: "tempdb",
        compatibility_level: 160,
        recovery_model: "SIMPLE",
    },
    DatabaseDef {
        id: 3,
        name: "model",
        compatibility_level: 160,
        recovery_model: "FULL",
    },
    DatabaseDef {
        id: 4,
        name: "msdb",
        compatibility_level: 160,
        recovery_model: "FULL",
    },
    DatabaseDef {
        id: 5,
        name: "tsql_lite",
        compatibility_level: 160,
        recovery_model: "FULL",
    },
];

pub(crate) fn builtin_databases() -> impl Iterator<Item = &'static DatabaseDef> {
    BUILTIN_DATABASES.iter()
}

pub(crate) fn is_builtin_database(name: &str) -> bool {
    BUILTIN_DATABASES
        .iter()
        .any(|db| db.name.eq_ignore_ascii_case(name))
}

pub(crate) fn all_databases(catalog: &dyn Catalog) -> Vec<DatabaseEntry> {
    let mut out: Vec<DatabaseEntry> = BUILTIN_DATABASES
        .iter()
        .map(|db| {
            let opts = catalog.get_database_options(db.name);
            DatabaseEntry {
                id: db.id,
                name: db.name.to_string(),
                compatibility_level: opts.compatibility_level,
                recovery_model: opts.recovery_model,
                user_access: opts.user_access,
                is_read_only: opts.is_read_only,
                read_committed_snapshot: opts.read_committed_snapshot,
                allow_snapshot_isolation: opts.allow_snapshot_isolation,
                source_database_id: opts.source_database_id,
            }
        })
        .collect();
    for udb in catalog.get_user_databases() {
        out.push(DatabaseEntry {
            id: udb.id,
            name: udb.name.clone(),
            compatibility_level: udb.compatibility_level,
            recovery_model: udb.recovery_model.clone(),
            user_access: udb.user_access,
            is_read_only: udb.is_read_only,
            read_committed_snapshot: udb.read_committed_snapshot,
            allow_snapshot_isolation: udb.allow_snapshot_isolation,
            source_database_id: udb.source_database_id,
        });
    }
    out
}

pub(crate) fn is_user_database(catalog: &dyn Catalog, db: &str) -> bool {
    catalog
        .get_user_databases()
        .iter()
        .any(|u| u.name.eq_ignore_ascii_case(db))
}

pub(crate) fn db_schema_prefix(db: &str) -> String {
    format!(
        "{}{}{}",
        DB_SCHEMA_SEP,
        db.to_ascii_lowercase(),
        DB_SCHEMA_SEP
    )
}

pub(crate) fn db_schema_key(db: &str, schema: &str) -> String {
    if schema.starts_with(DB_SCHEMA_SEP) {
        schema.to_string()
    } else {
        format!("{}{}", db_schema_prefix(db), schema)
    }
}

pub(crate) fn split_db_schema(raw: &str) -> (Option<&str>, &str) {
    if let Some(rest) = raw.strip_prefix(DB_SCHEMA_SEP) {
        if let Some(pos) = rest.find(DB_SCHEMA_SEP) {
            return (Some(&rest[..pos]), &rest[pos + 1..]);
        }
    }
    (None, raw)
}

pub(crate) fn display_schema_name(raw: &str) -> &str {
    split_db_schema(raw).1
}

pub(crate) fn schema_belongs_to_db(
    raw_schema: &str,
    active_db: &str,
    catalog: &dyn Catalog,
) -> bool {
    let (owner_db, _) = split_db_schema(raw_schema);
    if is_user_database(catalog, active_db) {
        matches!(owner_db, Some(d) if d.eq_ignore_ascii_case(active_db))
    } else {
        owner_db.is_none()
    }
}

pub(crate) fn resolve_effective_schema(
    obj_db: Option<&str>,
    raw_schema: &str,
    current_db: &str,
    catalog: &dyn Catalog,
) -> String {
    if raw_schema.starts_with(DB_SCHEMA_SEP)
        || raw_schema.eq_ignore_ascii_case("sys")
        || raw_schema.eq_ignore_ascii_case("INFORMATION_SCHEMA")
    {
        return raw_schema.to_string();
    }
    let target_db = obj_db.unwrap_or(current_db);
    if is_user_database(catalog, target_db) {
        db_schema_key(target_db, raw_schema)
    } else {
        raw_schema.to_string()
    }
}

pub(crate) fn database_id_for_name(name: &str) -> Option<i32> {
    builtin_databases()
        .find(|db| db.name.eq_ignore_ascii_case(name))
        .map(|db| db.id)
}

pub(crate) fn database_id_for_name_in(catalog: &dyn Catalog, name: &str) -> Option<i32> {
    database_id_for_name(name).or_else(|| {
        catalog
            .get_user_databases()
            .iter()
            .find(|u| u.name.eq_ignore_ascii_case(name))
            .map(|u| u.id)
    })
}

pub(crate) fn database_name_for_id(id: i32) -> Option<&'static str> {
    builtin_databases().find(|db| db.id == id).map(|db| db.name)
}

pub(crate) fn database_name_for_id_in(catalog: &dyn Catalog, id: i32) -> Option<String> {
    database_name_for_id(id).map(str::to_string).or_else(|| {
        catalog
            .get_user_databases()
            .iter()
            .find(|u| u.id == id)
            .map(|u| u.name.clone())
    })
}

pub(crate) fn recovery_model_for_name(name: &str) -> Option<&'static str> {
    builtin_databases()
        .find(|db| db.name.eq_ignore_ascii_case(name))
        .map(|db| db.recovery_model)
}

pub(crate) fn current_database_name<'a>(ctx: &'a ExecutionContext<'a>) -> &'a str {
    ctx.metadata
        .database
        .as_deref()
        .unwrap_or(&ctx.metadata.original_database)
}

pub(crate) fn current_database_id(ctx: &ExecutionContext<'_>) -> i32 {
    database_id_for_name(current_database_name(ctx)).unwrap_or(0)
}

pub(crate) fn current_database_id_in(catalog: &dyn Catalog, ctx: &ExecutionContext<'_>) -> i32 {
    database_id_for_name_in(catalog, current_database_name(ctx)).unwrap_or(0)
}

pub(crate) fn authenticating_database_id(ctx: &ExecutionContext<'_>) -> i32 {
    database_id_for_name(&ctx.metadata.original_database).unwrap_or(0)
}
