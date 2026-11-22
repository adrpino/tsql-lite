use super::ScriptExecutor;
use crate::ast::{AlterTableStmt, CreateTableStmt, DropTableStmt, TruncateTableStmt};
use crate::error::DbError;
use crate::executor::context::ExecutionContext;
use crate::executor::mutation::MutationExecutor;
use crate::executor::result::QueryResult;

mod temp_table;

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
    pub(crate) fn execute_ddl(
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

        self.schema(ctx).alter_table(stmt)?;

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
