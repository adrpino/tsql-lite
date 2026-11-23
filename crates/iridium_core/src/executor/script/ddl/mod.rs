use super::ScriptExecutor;
use crate::ast::{AlterTableStmt, CreateTableStmt, DropTableStmt, TruncateTableStmt};
use crate::error::DbError;
use crate::executor::context::ExecutionContext;
use crate::executor::mutation::MutationExecutor;
use crate::executor::result::QueryResult;

mod temp_table;
pub(crate) mod triggers;

fn ignore_missing_if_exists(res: Result<(), DbError>, if_exists: bool) -> Result<(), DbError> {
    if !if_exists {
        return res;
    }
    match res {
        Ok(()) => Ok(()),
        Err(
            DbError::TableNotFound { .. }
            | DbError::ObjectNotFound { .. }
            | DbError::ViewNotFound { .. }
            | DbError::TriggerNotFound { .. }
            | DbError::SchemaNotFound { .. }
            | DbError::IndexNotFound { .. }
            | DbError::TypeNotFound { .. },
        ) => Ok(()),
        Err(other) => Err(other),
    }
}

impl<'a> ScriptExecutor<'a> {
    fn ddl_event_meta_for(&self, ddl: &crate::ast::DdlStatement) -> Option<triggers::DdlEventMeta> {
        use crate::ast::{AlterTableAction, DdlStatement};
        use triggers::DdlEventMeta;
        let is_temp = |n: &str| {
            n.starts_with('#')
                || n.starts_with('@')
                || n.starts_with("__temp_")
                || n.starts_with("__tv_")
        };
        match ddl {
            DdlStatement::CreateTable(s) if !is_temp(&s.name.name) => Some(DdlEventMeta {
                event_type: "CREATE_TABLE",
                schema_name: Some(s.name.schema_or_dbo().to_string()),
                object_name: s.name.name.clone(),
                object_type: "TABLE",
                command_text: format!("CREATE TABLE {}.{}", s.name.schema_or_dbo(), s.name.name),
            }),
            DdlStatement::DropTable(s) if !is_temp(&s.name.name) => {
                let sch = s.name.schema_or_dbo();
                if s.if_exists && self.catalog.find_table(sch, &s.name.name).is_none() {
                    return None;
                }
                Some(DdlEventMeta {
                    event_type: "DROP_TABLE",
                    schema_name: Some(sch.to_string()),
                    object_name: s.name.name.clone(),
                    object_type: "TABLE",
                    command_text: format!("DROP TABLE {}.{}", sch, s.name.name),
                })
            }
            DdlStatement::AlterTable(s)
                if !is_temp(&s.table.name)
                    && !matches!(s.action, AlterTableAction::ToggleTrigger { .. }) =>
            {
                Some(DdlEventMeta {
                    event_type: "ALTER_TABLE",
                    schema_name: Some(s.table.schema_or_dbo().to_string()),
                    object_name: s.table.name.clone(),
                    object_type: "TABLE",
                    command_text: format!(
                        "ALTER TABLE {}.{}",
                        s.table.schema_or_dbo(),
                        s.table.name
                    ),
                })
            }
            DdlStatement::DropView(s) => {
                let sch = s.name.schema_or_dbo();
                if s.if_exists && self.catalog.find_view(sch, &s.name.name).is_none() {
                    return None;
                }
                Some(DdlEventMeta {
                    event_type: "DROP_VIEW",
                    schema_name: Some(sch.to_string()),
                    object_name: s.name.name.clone(),
                    object_type: "VIEW",
                    command_text: format!("DROP VIEW {}.{}", sch, s.name.name),
                })
            }
            DdlStatement::CreateIndex(s) if !is_temp(&s.table.name) => Some(DdlEventMeta {
                event_type: "CREATE_INDEX",
                schema_name: Some(s.table.schema_or_dbo().to_string()),
                object_name: s.name.name.clone(),
                object_type: "INDEX",
                command_text: format!(
                    "CREATE INDEX {} ON {}.{}",
                    s.name.name,
                    s.table.schema_or_dbo(),
                    s.table.name
                ),
            }),
            DdlStatement::DropIndex(s) if !is_temp(&s.table.name) => Some(DdlEventMeta {
                event_type: "DROP_INDEX",
                schema_name: Some(s.table.schema_or_dbo().to_string()),
                object_name: s.name.name.clone(),
                object_type: "INDEX",
                command_text: format!(
                    "DROP INDEX {} ON {}.{}",
                    s.name.name,
                    s.table.schema_or_dbo(),
                    s.table.name
                ),
            }),
            DdlStatement::CreateSchema(s) => Some(DdlEventMeta {
                event_type: "CREATE_SCHEMA",
                schema_name: None,
                object_name: s.name.clone(),
                object_type: "SCHEMA",
                command_text: format!("CREATE SCHEMA {}", s.name),
            }),
            DdlStatement::DropSchema(s) => Some(DdlEventMeta {
                event_type: "DROP_SCHEMA",
                schema_name: None,
                object_name: s.name.clone(),
                object_type: "SCHEMA",
                command_text: format!("DROP SCHEMA {}", s.name),
            }),
            DdlStatement::AlterSchemaTransfer(s) => Some(DdlEventMeta {
                event_type: "ALTER_SCHEMA",
                schema_name: None,
                object_name: s.target_schema.clone(),
                object_type: "SCHEMA",
                command_text: format!(
                    "ALTER SCHEMA {} TRANSFER {}.{}",
                    s.target_schema,
                    s.source.schema_or_dbo(),
                    s.source.name
                ),
            }),
            DdlStatement::DropProcedure(s) => {
                let sch = s.name.schema_or_dbo();
                if s.if_exists && self.catalog.find_routine(sch, &s.name.name).is_none() {
                    return None;
                }
                Some(DdlEventMeta {
                    event_type: "DROP_PROCEDURE",
                    schema_name: Some(sch.to_string()),
                    object_name: s.name.name.clone(),
                    object_type: "PROCEDURE",
                    command_text: format!("DROP PROCEDURE {}.{}", sch, s.name.name),
                })
            }
            DdlStatement::DropFunction(s) => {
                let sch = s.name.schema_or_dbo();
                if s.if_exists && self.catalog.find_routine(sch, &s.name.name).is_none() {
                    return None;
                }
                Some(DdlEventMeta {
                    event_type: "DROP_FUNCTION",
                    schema_name: Some(sch.to_string()),
                    object_name: s.name.name.clone(),
                    object_type: "FUNCTION",
                    command_text: format!("DROP FUNCTION {}.{}", sch, s.name.name),
                })
            }
            DdlStatement::DropTrigger(s) => Some(DdlEventMeta {
                event_type: "DROP_TRIGGER",
                schema_name: Some(s.name.schema_or_dbo().to_string()),
                object_name: s.name.name.clone(),
                object_type: "TRIGGER",
                command_text: format!("DROP TRIGGER {}", s.name.name),
            }),
            DdlStatement::CreateType(s) => Some(DdlEventMeta {
                event_type: "CREATE_TYPE",
                schema_name: Some(s.name.schema_or_dbo().to_string()),
                object_name: s.name.name.clone(),
                object_type: "TYPE",
                command_text: format!("CREATE TYPE {}.{}", s.name.schema_or_dbo(), s.name.name),
            }),
            DdlStatement::CreateScalarType(s) => Some(DdlEventMeta {
                event_type: "CREATE_TYPE",
                schema_name: Some(s.name.schema_or_dbo().to_string()),
                object_name: s.name.name.clone(),
                object_type: "TYPE",
                command_text: format!("CREATE TYPE {}.{}", s.name.schema_or_dbo(), s.name.name),
            }),
            DdlStatement::DropType(s) => Some(DdlEventMeta {
                event_type: "DROP_TYPE",
                schema_name: Some(s.name.schema_or_dbo().to_string()),
                object_name: s.name.name.clone(),
                object_type: "TYPE",
                command_text: format!("DROP TYPE {}.{}", s.name.schema_or_dbo(), s.name.name),
            }),
            DdlStatement::CreateSequence(s) => Some(DdlEventMeta {
                event_type: "CREATE_SEQUENCE",
                schema_name: Some(s.name.schema_or_dbo().to_string()),
                object_name: s.name.name.clone(),
                object_type: "SEQUENCE",
                command_text: format!("CREATE SEQUENCE {}.{}", s.name.schema_or_dbo(), s.name.name),
            }),
            DdlStatement::AlterSequence(s) => Some(DdlEventMeta {
                event_type: "ALTER_SEQUENCE",
                schema_name: Some(s.name.schema_or_dbo().to_string()),
                object_name: s.name.name.clone(),
                object_type: "SEQUENCE",
                command_text: format!("ALTER SEQUENCE {}.{}", s.name.schema_or_dbo(), s.name.name),
            }),
            DdlStatement::DropSequence(s) => Some(DdlEventMeta {
                event_type: "DROP_SEQUENCE",
                schema_name: Some(s.name.schema_or_dbo().to_string()),
                object_name: s.name.name.clone(),
                object_type: "SEQUENCE",
                command_text: format!("DROP SEQUENCE {}.{}", s.name.schema_or_dbo(), s.name.name),
            }),
            DdlStatement::CreateSynonym(s) => Some(DdlEventMeta {
                event_type: "CREATE_SYNONYM",
                schema_name: Some(s.name.schema_or_dbo().to_string()),
                object_name: s.name.name.clone(),
                object_type: "SYNONYM",
                command_text: format!("CREATE SYNONYM {}.{}", s.name.schema_or_dbo(), s.name.name),
            }),
            DdlStatement::DropSynonym(s) => Some(DdlEventMeta {
                event_type: "DROP_SYNONYM",
                schema_name: Some(s.name.schema_or_dbo().to_string()),
                object_name: s.name.name.clone(),
                object_type: "SYNONYM",
                command_text: format!("DROP SYNONYM {}.{}", s.name.schema_or_dbo(), s.name.name),
            }),
            _ => None,
        }
    }

    pub(crate) fn execute_ddl(
        &mut self,
        ddl: crate::ast::DdlStatement,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        let meta = self.ddl_event_meta_for(&ddl);
        self.run_with_ddl_triggers(meta, ctx, |this, ctx| this.execute_ddl_inner(ddl, ctx))
    }

    fn execute_ddl_inner(
        &mut self,
        ddl: crate::ast::DdlStatement,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        use crate::ast::DdlStatement;
        match ddl {
            DdlStatement::CreateTable(stmt) => self.execute_create_table(stmt, ctx),
            DdlStatement::DropTable(stmt) => self.execute_drop_table(stmt, ctx),
            DdlStatement::CreateType(stmt) => {
                self.schema(ctx).create_type(stmt)?;
                Ok(None)
            }
            DdlStatement::CreateScalarType(stmt) => {
                self.schema(ctx).create_scalar_type(stmt)?;
                Ok(None)
            }
            DdlStatement::DropType(stmt) => {
                let if_exists = stmt.if_exists;
                ignore_missing_if_exists(self.schema(ctx).drop_type(stmt), if_exists)?;
                Ok(None)
            }
            DdlStatement::CreateIndex(mut stmt) => {
                temp_table::resolve_table_name_for_mutation(&mut stmt.table, ctx);
                if stmt.is_unique {
                    let table_schema = stmt.table.schema_or_dbo().to_string();
                    if let Some(table) = self
                        .catalog
                        .find_table(&table_schema, &stmt.table.name)
                        .cloned()
                    {
                        let col_names: Vec<String> =
                            stmt.columns.iter().map(|c| c.name.clone()).collect();
                        crate::executor::mutation::validation::validate_existing_rows_for_unique_index(
                            &table,
                            &stmt.name.name,
                            &col_names,
                            &stmt.filter_predicate,
                            self.catalog,
                            self.storage,
                            ctx,
                            self.clock,
                        )?;
                    }
                }
                self.schema(ctx).create_index(stmt)?;
                Ok(None)
            }
            DdlStatement::DropIndex(mut stmt) => {
                temp_table::resolve_table_name_for_mutation(&mut stmt.table, ctx);
                let if_exists = stmt.if_exists;
                ignore_missing_if_exists(self.schema(ctx).drop_index(stmt), if_exists)?;
                Ok(None)
            }
            DdlStatement::CreateSchema(stmt) => {
                self.schema(ctx).create_schema(stmt)?;
                Ok(None)
            }
            DdlStatement::DropSchema(stmt) => {
                let if_exists = stmt.if_exists;
                ignore_missing_if_exists(self.schema(ctx).drop_schema(stmt), if_exists)?;
                Ok(None)
            }
            DdlStatement::AlterSchemaTransfer(stmt) => {
                self.schema(ctx).alter_schema_transfer(stmt)?;
                Ok(None)
            }
            DdlStatement::DropProcedure(stmt) => {
                let schema = stmt.name.schema_or_dbo().to_string();
                ignore_missing_if_exists(
                    self.catalog.drop_routine(&schema, &stmt.name.name, false),
                    stmt.if_exists,
                )?;
                Ok(None)
            }
            DdlStatement::DropFunction(stmt) => {
                let schema = stmt.name.schema_or_dbo().to_string();
                if let Some(dep) =
                    crate::executor::metadata::sys::dependencies::find_schemabound_dependent(
                        self.catalog,
                        &schema,
                        &stmt.name.name,
                    )
                {
                    return Err(DbError::Execution(format!(
                        "Cannot DROP FUNCTION '{}.{}' because it is being referenced by object '{}'.",
                        schema, stmt.name.name, dep
                    )));
                }
                ignore_missing_if_exists(
                    self.catalog.drop_routine(&schema, &stmt.name.name, true),
                    stmt.if_exists,
                )?;
                Ok(None)
            }
            DdlStatement::DropTrigger(stmt) => {
                let schema = stmt.name.schema_or_dbo().to_string();
                ignore_missing_if_exists(
                    self.catalog.drop_trigger(&schema, &stmt.name.name),
                    stmt.if_exists,
                )?;
                Ok(None)
            }
            DdlStatement::TruncateTable(stmt) => self.execute_truncate_table(stmt, ctx),
            DdlStatement::AlterTable(stmt) => self.execute_alter_table(stmt, ctx),
            DdlStatement::DropView(stmt) => self.execute_drop_view(stmt, ctx),
            DdlStatement::CreateSynonym(stmt) => {
                self.schema(ctx).create_synonym(stmt)?;
                Ok(None)
            }
            DdlStatement::DropSynonym(stmt) => {
                self.schema(ctx).drop_synonym(stmt)?;
                Ok(None)
            }
            DdlStatement::CreateSequence(stmt) => {
                self.schema(ctx).create_sequence(stmt)?;
                Ok(None)
            }
            DdlStatement::DropSequence(stmt) => {
                self.schema(ctx).drop_sequence(stmt)?;
                Ok(None)
            }
            DdlStatement::AlterSequence(stmt) => {
                self.schema(ctx).alter_sequence(stmt)?;
                Ok(None)
            }
            DdlStatement::CatalogMetadata(action) => {
                self.execute_catalog_metadata(*action)?;
                Ok(None)
            }
        }
    }

    pub(crate) fn execute_drop_view(
        &mut self,
        stmt: crate::ast::DropViewStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        let if_exists = stmt.if_exists;
        ignore_missing_if_exists(self.schema(ctx).drop_view(stmt), if_exists)?;
        Ok(None)
    }

    pub(crate) fn execute_create_table(
        &mut self,
        mut stmt: CreateTableStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        temp_table::map_create_temp_table(&mut stmt, ctx);
        self.schema(ctx).create_table(stmt)?;
        Ok(None)
    }

    pub(crate) fn execute_drop_table(
        &mut self,
        mut stmt: DropTableStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        let if_exists = stmt.if_exists;
        temp_table::resolve_drop_table_name(&mut stmt, ctx);
        ignore_missing_if_exists(self.schema(ctx).drop_table(stmt), if_exists)?;
        Ok(None)
    }

    pub(crate) fn execute_truncate_table(
        &mut self,
        mut stmt: TruncateTableStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        if let Some(_mapped) = temp_table::resolve_table_name_for_mutation(&mut stmt.name, ctx) {
            // resolved
        }
        let schema = stmt.name.schema_or_dbo();
        let table_name = &stmt.name.name;
        let table = self
            .catalog
            .find_table(schema, table_name)
            .ok_or_else(|| DbError::table_not_found(schema, table_name))?
            .clone();
        self.storage.clear_table(table.id)?;

        let mut_exec = MutationExecutor {
            catalog: self.catalog,
            storage: self.storage,
            clock: self.clock,
        };
        mut_exec.push_dirty_truncate(ctx, &table.name);
        Ok(None)
    }

    pub(crate) fn execute_alter_table(
        &mut self,
        mut stmt: AlterTableStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        if matches!(
            stmt.action,
            crate::ast::AlterTableAction::ToggleTrigger { .. }
        ) && stmt.table.name.eq_ignore_ascii_case("__DATABASE__")
        {
            self.schema(ctx).alter_table(stmt)?;
            return Ok(None);
        }
        if let Some(_mapped) = temp_table::resolve_table_name_for_mutation(&mut stmt.table, ctx) {
            // resolved
        }
        let schema = stmt.table.schema_or_dbo();
        let table_name = &stmt.table.name;
        let table = self
            .catalog
            .find_table(schema, table_name)
            .ok_or_else(|| DbError::table_not_found(schema, table_name))?
            .clone();

        // Retroactive validation for WITH CHECK CHECK CONSTRAINT and [WITH CHECK] ADD CONSTRAINT
        match &stmt.action {
            crate::ast::AlterTableAction::CheckConstraint {
                check: true,
                with_check: Some(true),
                name,
            } => {
                for chk in &table.check_constraints {
                    if name
                        .as_ref()
                        .is_none_or(|n| chk.name.eq_ignore_ascii_case(n))
                    {
                        crate::executor::mutation::validation::validate_check_constraint_on_rows(
                            &table,
                            &chk.name,
                            &chk.expr,
                            ctx,
                            self.catalog,
                            self.storage,
                            self.clock,
                        )?;
                    }
                }
                for fk in &table.foreign_keys {
                    if name
                        .as_ref()
                        .is_none_or(|n| fk.name.eq_ignore_ascii_case(n))
                    {
                        crate::executor::mutation::validation::validate_foreign_key_on_rows(
                            &table,
                            fk,
                            self.catalog,
                            self.storage,
                        )?;
                    }
                }
            }
            crate::ast::AlterTableAction::AddConstraint {
                constraint,
                with_check,
            } if *with_check != Some(false) => match constraint {
                crate::ast::TableConstraintSpec::Check { name, expr } => {
                    crate::executor::mutation::validation::validate_check_constraint_on_rows(
                        &table,
                        name,
                        expr,
                        ctx,
                        self.catalog,
                        self.storage,
                        self.clock,
                    )?;
                }
                crate::ast::TableConstraintSpec::ForeignKey {
                    name,
                    columns,
                    referenced_table,
                    referenced_columns,
                    on_delete,
                    on_update,
                } => {
                    let fk = crate::catalog::ForeignKeyDef {
                        name: name.clone(),
                        columns: columns.clone(),
                        referenced_table: referenced_table.clone(),
                        referenced_columns: referenced_columns.clone(),
                        on_delete: on_delete.unwrap_or(crate::ast::ReferentialAction::NoAction),
                        on_update: on_update.unwrap_or(crate::ast::ReferentialAction::NoAction),
                        enabled: true,
                        is_not_trusted: false,
                    };
                    crate::executor::mutation::validation::validate_foreign_key_on_rows(
                        &table,
                        &fk,
                        self.catalog,
                        self.storage,
                    )?;
                }
                _ => {}
            },
            _ => {}
        }

        let mut add_col_backfill: Option<(crate::ast::Expr, crate::types::DataType)> = None;
        let mut add_col_identity_backfill: Option<(i64, i64)> = None;
        if let crate::ast::AlterTableAction::AddColumn(ref col_spec) = stmt.action {
            let is_rowversion = matches!(col_spec.data_type, crate::ast::DataTypeSpec::RowVersion);
            let (resolved_dt, udt_nullable) =
                crate::executor::type_mapping::resolve_data_type_spec_with_nullability(
                    &col_spec.data_type,
                    self.catalog,
                );
            let effective_nullable = if col_spec.nullable_explicit {
                col_spec.nullable
            } else if is_rowversion {
                false
            } else if let Some(u) = udt_nullable {
                u
            } else {
                ctx.options.ansi_null_dflt_on
            };
            let has_rows = self
                .storage
                .scan_rows(table.id)
                .ok()
                .and_then(|iter| iter.collect::<Result<Vec<_>, _>>().ok())
                .is_some_and(|rows| rows.iter().any(|r| !r.deleted));
            if has_rows
                && !effective_nullable
                && col_spec.default.is_none()
                && col_spec.identity.is_none()
                && col_spec.computed_expr.is_none()
                && !is_rowversion
            {
                return Err(DbError::Custom {
                    class: 16,
                    number: 4901,
                    message: format!(
                        "ALTER TABLE only allows columns to be added that can contain nulls, or have a DEFAULT definition specified, or the column being added is an identity or timestamp column, or alternatively if none of the previous conditions are satisfied the table must be empty to allow addition of this column. Column '{}' cannot be added to non-empty table '{}' because it does not satisfy these conditions.",
                        col_spec.name, table.name
                    ),
                });
            }
            if has_rows && (!effective_nullable || col_spec.with_values) {
                if let Some(ref def_expr) = col_spec.default {
                    add_col_backfill = Some((def_expr.clone(), resolved_dt));
                }
            }
            if has_rows && col_spec.identity.is_some() {
                add_col_identity_backfill = col_spec.identity;
            }
        }

        self.schema(ctx).alter_table(stmt)?;

        if let Some((def_expr, target_dt)) = add_col_backfill {
            let mut rows_vec = self
                .storage
                .scan_rows(table.id)?
                .collect::<Result<Vec<_>, DbError>>()?;
            for row in rows_vec.iter_mut() {
                if row.deleted {
                    continue;
                }
                let raw = crate::executor::evaluator::eval_expr(
                    &def_expr,
                    &[],
                    ctx,
                    self.catalog,
                    self.storage,
                    self.clock,
                )?;
                let coerced = if raw.is_null() {
                    raw
                } else {
                    crate::executor::value_ops::coerce_value_to_type_with_dateformat(
                        raw,
                        &target_dt,
                        &ctx.options.dateformat,
                    )?
                };
                if let Some(last) = row.values.last_mut() {
                    *last = coerced;
                }
            }
            self.storage.replace_table(table.id, rows_vec)?;
        } else if let Some((seed, inc)) = add_col_identity_backfill {
            let mut rows_vec = self
                .storage
                .scan_rows(table.id)?
                .collect::<Result<Vec<_>, DbError>>()?;
            let mut cur = seed;
            for row in rows_vec.iter_mut() {
                if row.deleted {
                    continue;
                }
                if let Some(last) = row.values.last_mut() {
                    *last = crate::types::Value::Int(cur as i32);
                }
                cur += inc;
            }
            self.storage.replace_table(table.id, rows_vec)?;
            if let Some(tbl_mut) = self.catalog.find_table_mut(&table.schema_name, &table.name) {
                if let Some(last_col) = tbl_mut.columns.last_mut() {
                    if let Some(ref mut id) = last_col.identity {
                        id.current = cur;
                    }
                }
            }
        }

        let rows = {
            let rows = match self.storage.scan_rows(table.id) {
                Ok(rows) => rows,
                Err(_) => return Ok(None),
            };
            match rows.collect::<Result<Vec<_>, DbError>>() {
                Ok(rows) => rows,
                Err(_) => return Ok(None),
            }
        };

        let mut_exec = MutationExecutor {
            catalog: self.catalog,
            storage: self.storage,
            clock: self.clock,
        };
        mut_exec.push_dirty_replace(ctx, &table.name, rows);

        Ok(None)
    }

    pub(crate) fn execute_catalog_metadata(
        &mut self,
        action: crate::ast::CatalogMetadataAction,
    ) -> Result<(), DbError> {
        use crate::ast::CatalogMetadataAction;
        match action {
            CatalogMetadataAction::CreateUser {
                name,
                default_schema,
            } => {
                self.catalog.upsert_principal(crate::catalog::PrincipalDef {
                    principal_id: 0,
                    name,
                    principal_type: "S".to_string(),
                    type_desc: "SQL_USER".to_string(),
                    default_schema_name: default_schema.or_else(|| Some("dbo".to_string())),
                    is_fixed_role: false,
                });
            }
            CatalogMetadataAction::DropUser { name, if_exists }
            | CatalogMetadataAction::DropRole { name, if_exists } => {
                self.catalog.drop_principal(&name, if_exists)?;
            }
            CatalogMetadataAction::CreateRole { name } => {
                self.catalog.upsert_principal(crate::catalog::PrincipalDef {
                    principal_id: 0,
                    name,
                    principal_type: "R".to_string(),
                    type_desc: "DATABASE_ROLE".to_string(),
                    default_schema_name: None,
                    is_fixed_role: false,
                });
            }
            CatalogMetadataAction::AlterRoleMember { role, member, add } => {
                self.catalog.alter_role_member(&role, &member, add);
            }
            CatalogMetadataAction::Permission {
                action,
                permissions,
                target_class,
                target_name,
                grantees,
                with_grant_option,
            } => {
                let is_revoke = action.eq_ignore_ascii_case("REVOKE");
                let (state, state_desc) = if action.eq_ignore_ascii_case("DENY") {
                    ("D", "DENY")
                } else if with_grant_option {
                    ("W", "GRANT_WITH_GRANT_OPTION")
                } else {
                    ("G", "GRANT")
                };

                let (class, class_desc, major_id) = match (&target_class, &target_name) {
                    (_, None) => (0, "DATABASE", 0),
                    (Some(cls), Some(obj)) if cls.eq_ignore_ascii_case("SCHEMA") => (
                        3,
                        "SCHEMA",
                        self.catalog.get_schema_id(&obj.name).unwrap_or(1) as i32,
                    ),
                    (Some(cls), Some(obj)) if cls.eq_ignore_ascii_case("TYPE") => {
                        let sch = obj.schema_or_dbo();
                        let id = self
                            .catalog
                            .find_table_type(sch, &obj.name)
                            .map(|t| t.object_id)
                            .or_else(|| {
                                self.catalog
                                    .find_scalar_type(sch, &obj.name)
                                    .map(|t| t.object_id)
                            })
                            .unwrap_or(0);
                        (6, "TYPE", id)
                    }
                    (_, Some(obj)) => {
                        let sch = obj.schema_or_dbo();
                        let id = self
                            .catalog
                            .object_id(sch, &obj.name)
                            .or_else(|| {
                                self.catalog
                                    .find_table_type(sch, &obj.name)
                                    .map(|t| t.object_id)
                            })
                            .unwrap_or(0);
                        (1, "OBJECT_OR_COLUMN", id)
                    }
                };

                for grantee in grantees {
                    let grantee_id =
                        self.catalog.find_principal_id(&grantee).unwrap_or_else(|| {
                            self.catalog.upsert_principal(crate::catalog::PrincipalDef {
                                principal_id: 0,
                                name: grantee.clone(),
                                principal_type: "S".to_string(),
                                type_desc: "SQL_USER".to_string(),
                                default_schema_name: Some("dbo".to_string()),
                                is_fixed_role: false,
                            })
                        });
                    for perm in &permissions {
                        let perm_upper = perm.trim().to_ascii_uppercase();
                        if perm_upper.is_empty() {
                            continue;
                        }
                        let perm_type = short_perm_code(&perm_upper);
                        self.catalog.apply_permission(
                            crate::catalog::PermissionDef {
                                class,
                                class_desc: class_desc.to_string(),
                                major_id,
                                minor_id: 0,
                                grantee_principal_id: grantee_id,
                                grantor_principal_id: 1,
                                perm_type: perm_type.to_string(),
                                permission_name: perm_upper,
                                state: state.to_string(),
                                state_desc: state_desc.to_string(),
                            },
                            is_revoke,
                        );
                    }
                }
            }
            CatalogMetadataAction::CreatePartitionFunction {
                name,
                data_type,
                range_right,
                boundary_values,
            } => {
                let dt =
                    crate::executor::type_mapping::resolve_data_type_spec(&data_type, self.catalog);
                self.catalog
                    .upsert_partition_function(crate::catalog::PartitionFunctionDef {
                        function_id: 0,
                        name,
                        data_type: dt,
                        boundary_value_on_right: range_right,
                        boundary_values,
                    });
            }
            CatalogMetadataAction::DropPartitionFunction { name } => {
                self.catalog.drop_partition_function(&name);
            }
            CatalogMetadataAction::CreatePartitionScheme {
                name,
                function_name,
                all_to,
                filegroups,
            } => {
                let expanded = if all_to && filegroups.len() == 1 {
                    let fanout = self
                        .catalog
                        .get_partition_functions()
                        .iter()
                        .find(|p| p.name.eq_ignore_ascii_case(&function_name))
                        .map(|p| p.boundary_values.len() + 1)
                        .unwrap_or(1);
                    vec![filegroups[0].clone(); fanout]
                } else {
                    filegroups
                };
                self.catalog
                    .upsert_partition_scheme(name, &function_name, expanded);
            }
            CatalogMetadataAction::DropPartitionScheme { name } => {
                self.catalog.drop_partition_scheme(&name);
            }
            CatalogMetadataAction::AddFilegroup { name } => {
                self.catalog.add_filegroup(name);
            }
            CatalogMetadataAction::CreateSecurityPolicy {
                name,
                predicates,
                state_on,
                schema_binding,
            } => {
                let schema = name.schema_or_dbo().to_string();
                let schema_id = self
                    .catalog
                    .get_schema_id(&schema)
                    .ok_or_else(|| DbError::schema_not_found(&schema))?;
                if self.catalog.get_security_policies().iter().any(|p| {
                    p.schema.eq_ignore_ascii_case(&schema)
                        && p.name.eq_ignore_ascii_case(&name.name)
                }) {
                    return Err(DbError::Execution(format!(
                        "There is already an object named '{}' in the database.",
                        name.name
                    )));
                }
                let object_id = self.catalog.alloc_object_id();
                let mut preds = Vec::with_capacity(predicates.len());
                for (i, p) in predicates.into_iter().enumerate() {
                    preds.push(crate::catalog::SecurityPredicateDef {
                        predicate_id: (i + 1) as i32,
                        kind: if p.is_block {
                            crate::catalog::SecurityPredicateKind::Block
                        } else {
                            crate::catalog::SecurityPredicateKind::Filter
                        },
                        block_op: p.block_op,
                        function_schema: p.function.schema_or_dbo().to_string(),
                        function_name: p.function.name,
                        function_args: p.function_args,
                        target_schema: p.target.schema_or_dbo().to_string(),
                        target_table: p.target.name,
                    });
                }
                self.catalog
                    .upsert_security_policy(crate::catalog::SecurityPolicyDef {
                        object_id,
                        schema_id,
                        schema,
                        name: name.name,
                        is_enabled: state_on,
                        is_schema_bound: schema_binding,
                        predicates: preds,
                    })?;
            }
            CatalogMetadataAction::AlterSecurityPolicy { name, actions } => {
                let schema = name.schema_or_dbo().to_string();
                let mut policy = self
                    .catalog
                    .get_security_policies()
                    .iter()
                    .find(|p| {
                        p.schema.eq_ignore_ascii_case(&schema)
                            && p.name.eq_ignore_ascii_case(&name.name)
                    })
                    .cloned()
                    .ok_or_else(|| {
                        DbError::Execution(format!(
                            "Cannot alter the security policy '{}.{}', because it does not exist or you do not have permission.",
                            schema, name.name
                        ))
                    })?;
                for act in actions {
                    match act {
                        crate::ast::AlterSecurityPolicyAction::SetState(on) => {
                            policy.is_enabled = on;
                        }
                        crate::ast::AlterSecurityPolicyAction::AddPredicate(p) => {
                            let next_id = policy
                                .predicates
                                .iter()
                                .map(|pr| pr.predicate_id)
                                .max()
                                .unwrap_or(0)
                                + 1;
                            policy
                                .predicates
                                .push(crate::catalog::SecurityPredicateDef {
                                    predicate_id: next_id,
                                    kind: if p.is_block {
                                        crate::catalog::SecurityPredicateKind::Block
                                    } else {
                                        crate::catalog::SecurityPredicateKind::Filter
                                    },
                                    block_op: p.block_op,
                                    function_schema: p.function.schema_or_dbo().to_string(),
                                    function_name: p.function.name,
                                    function_args: p.function_args,
                                    target_schema: p.target.schema_or_dbo().to_string(),
                                    target_table: p.target.name,
                                });
                        }
                        crate::ast::AlterSecurityPolicyAction::AlterPredicate(p) => {
                            let kind = if p.is_block {
                                crate::catalog::SecurityPredicateKind::Block
                            } else {
                                crate::catalog::SecurityPredicateKind::Filter
                            };
                            let t_sch = p.target.schema_or_dbo().to_string();
                            if let Some(existing) = policy.predicates.iter_mut().find(|pr| {
                                pr.kind == kind
                                    && pr.target_schema.eq_ignore_ascii_case(&t_sch)
                                    && pr.target_table.eq_ignore_ascii_case(&p.target.name)
                                    && pr.block_op == p.block_op
                            }) {
                                existing.function_schema = p.function.schema_or_dbo().to_string();
                                existing.function_name = p.function.name;
                                existing.function_args = p.function_args;
                            }
                        }
                        crate::ast::AlterSecurityPolicyAction::DropPredicate {
                            is_block,
                            target,
                            block_op,
                        } => {
                            let kind = if is_block {
                                crate::catalog::SecurityPredicateKind::Block
                            } else {
                                crate::catalog::SecurityPredicateKind::Filter
                            };
                            let t_sch = target.schema_or_dbo().to_string();
                            policy.predicates.retain(|pr| {
                                !(pr.kind == kind
                                    && pr.target_schema.eq_ignore_ascii_case(&t_sch)
                                    && pr.target_table.eq_ignore_ascii_case(&target.name)
                                    && (!is_block || pr.block_op == block_op))
                            });
                        }
                    }
                }
                self.catalog.upsert_security_policy(policy)?;
            }
            CatalogMetadataAction::DropSecurityPolicy { name, if_exists } => {
                let schema = name.schema_or_dbo().to_string();
                self.catalog
                    .drop_security_policy(&schema, &name.name, if_exists)?;
            }
            CatalogMetadataAction::CreateColumnMasterKey {
                name,
                key_store_provider_name,
                key_path,
                allow_enclave_computations,
                signature,
            } => {
                self.catalog
                    .create_column_master_key(crate::catalog::ColumnMasterKeyDef {
                        column_master_key_id: 0,
                        name,
                        key_store_provider_name,
                        key_path,
                        allow_enclave_computations,
                        signature,
                    })?;
            }
            CatalogMetadataAction::DropColumnMasterKey { name } => {
                self.catalog.drop_column_master_key(&name)?;
            }
            CatalogMetadataAction::CreateColumnEncryptionKey { name, values } => {
                self.catalog.create_column_encryption_key(name, values)?;
            }
            CatalogMetadataAction::AlterColumnEncryptionKey {
                name,
                add,
                cmk_name,
                algorithm,
                encrypted_value,
            } => {
                self.catalog.alter_column_encryption_key(
                    &name,
                    add,
                    &cmk_name,
                    algorithm,
                    encrypted_value,
                )?;
            }
            CatalogMetadataAction::DropColumnEncryptionKey { name } => {
                self.catalog.drop_column_encryption_key(&name)?;
            }
            CatalogMetadataAction::CreateDatabase { name } => {
                self.catalog.create_database(name)?;
            }
            CatalogMetadataAction::DropDatabase { name, if_exists } => {
                let dropped_ids = self.catalog.drop_database(&name, if_exists)?;
                for tid in dropped_ids {
                    let _ = self.storage.remove_table(tid);
                }
            }
        }
        Ok(())
    }
}

fn short_perm_code(perm: &str) -> &'static str {
    match perm {
        "SELECT" => "SL",
        "INSERT" => "IN",
        "UPDATE" => "UP",
        "DELETE" => "DL",
        "EXECUTE" | "EXEC" => "EX",
        "REFERENCES" => "RF",
        "ALTER" => "AL",
        "CONTROL" => "CL",
        "VIEW DEFINITION" => "VW",
        "TAKE OWNERSHIP" => "TO",
        "CONNECT" => "CO",
        _ => "EX",
    }
}
