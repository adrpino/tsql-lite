use super::super::ScriptExecutor;
use crate::ast::{CreateFunctionStmt, CreateProcedureStmt, CreateTriggerStmt, CreateViewStmt};
use crate::error::DbError;

impl<'a> ScriptExecutor<'a> {
    fn validate_routine_params(&self, params: &[crate::ast::RoutineParam]) -> Result<(), DbError> {
        for p in params {
            if let crate::ast::RoutineParamType::Scalar(crate::ast::DataTypeSpec::Custom(obj)) =
                &p.param_type
            {
                if self
                    .catalog
                    .find_table_type(obj.schema_or_dbo(), &obj.name)
                    .is_some()
                    && !p.is_readonly
                {
                    return Err(DbError::Parse(format!(
                        "table-valued parameter '{}' must be READONLY",
                        p.name
                    )));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn execute_create_procedure(
        &mut self,
        stmt: CreateProcedureStmt,
    ) -> Result<(), DbError> {
        self.validate_routine_params(&stmt.params)?;
        let schema = stmt.name.schema_or_dbo().to_string();
        let routine = crate::catalog::RoutineDef {
            object_id: self.catalog.alloc_object_id(),
            schema,
            name: stmt.name.name,
            params: stmt.params,
            kind: crate::catalog::RoutineKind::Procedure {
                body: std::sync::Arc::new(stmt.body),
            },
            definition_sql: stmt.definition_sql.unwrap_or_default(),
            is_schema_bound: false,
        };
        if stmt.or_alter {
            self.catalog.replace_routine(routine)?;
        } else {
            self.catalog.create_routine(routine)?;
        }
        Ok(())
    }

    pub(crate) fn execute_create_function(
        &mut self,
        stmt: CreateFunctionStmt,
    ) -> Result<(), DbError> {
        self.validate_routine_params(&stmt.params)?;
        let schema = stmt.name.schema_or_dbo().to_string();
        if stmt.is_schema_bound {
            crate::executor::metadata::sys::dependencies::validate_schemabinding_function(
                &schema,
                &stmt.name.name,
                &stmt.body,
            )?;
        }
        let routine = crate::catalog::RoutineDef {
            object_id: self.catalog.alloc_object_id(),
            schema,
            name: stmt.name.name,
            params: stmt.params,
            kind: crate::catalog::RoutineKind::Function {
                returns: stmt.returns,
                body: Box::new(stmt.body),
            },
            definition_sql: stmt.definition_sql.unwrap_or_default(),
            is_schema_bound: stmt.is_schema_bound,
        };
        if stmt.or_alter {
            self.catalog.replace_routine(routine)?;
        } else {
            self.catalog.create_routine(routine)?;
        }
        Ok(())
    }

    pub(crate) fn execute_create_view(
        &mut self,
        stmt: CreateViewStmt,
        ctx: &mut crate::executor::context::ExecutionContext<'_>,
    ) -> Result<(), DbError> {
        self.schema(ctx).create_view(stmt)
    }

    pub(crate) fn execute_create_trigger(
        &mut self,
        stmt: CreateTriggerStmt,
    ) -> Result<(), DbError> {
        let schema = stmt.name.schema_or_dbo().to_string();
        let trigger = crate::catalog::TriggerDef {
            object_id: self.catalog.alloc_object_id(),
            schema,
            name: stmt.name.name,
            table_schema: stmt.table.schema_or_dbo().to_string(),
            table_name: stmt.table.name,
            events: stmt.events,
            is_instead_of: stmt.is_instead_of,
            enabled: true,
            is_database_scope: stmt.is_database_scope,
            ddl_events: stmt.ddl_events,
            body: std::sync::Arc::new(stmt.body),
            definition_sql: stmt.definition_sql.unwrap_or_default(),
        };
        if stmt.or_alter {
            self.catalog.replace_trigger(trigger)?;
        } else {
            self.catalog.create_trigger(trigger)?;
        }
        Ok(())
    }
}
