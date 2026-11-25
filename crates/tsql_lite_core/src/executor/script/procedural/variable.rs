use super::super::ScriptExecutor;
use crate::ast::{DeclareStmt, SetStmt};
use crate::error::DbError;
use crate::executor::context::ExecutionContext;
use crate::executor::result::QueryResult;

impl<'a> ScriptExecutor<'a> {
    pub(crate) fn execute_declare(
        &mut self,
        stmt: DeclareStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        if let Some(ref custom_type) = stmt.custom_type {
            if let Some(type_def) = self
                .catalog
                .find_table_type(custom_type.schema_or_dbo(), &custom_type.name)
                .cloned()
            {
                return self.execute_declare_table_var(
                    crate::ast::DeclareTableVarStmt {
                        name: stmt.name,
                        columns: type_def.columns,
                        table_constraints: type_def.table_constraints,
                    },
                    ctx,
                );
            }
        }
        let declared_name = stmt.name.clone();
        let ty = if let Some(ref custom_type) = stmt.custom_type {
            if let Some(scalar_def) = self
                .catalog
                .find_scalar_type(custom_type.schema_or_dbo(), &custom_type.name)
            {
                scalar_def.base_type.clone()
            } else {
                crate::executor::type_mapping::resolve_data_type_spec(&stmt.data_type, self.catalog)
            }
        } else {
            crate::executor::type_mapping::resolve_data_type_spec(&stmt.data_type, self.catalog)
        };
        let value = if let Some(ref default_expr) = stmt.default {
            crate::executor::value_ops::coercion::xml::check_implicit_xml_var_conversion(
                default_expr,
                &ty,
                ctx,
            )?;
            let raw = crate::executor::evaluator::eval_expr(
                default_expr,
                &[],
                ctx,
                self.catalog,
                self.storage,
                self.clock,
            )?;
            if matches!(
                ty,
                crate::types::DataType::Vector { .. } | crate::types::DataType::Xml
            ) && !raw.is_null()
            {
                crate::executor::value_ops::coerce_value_to_type_with_dateformat(
                    raw,
                    &ty,
                    &ctx.options.dateformat,
                )?
            } else {
                raw
            }
        } else {
            crate::types::Value::Null
        };
        ctx.session.variables.insert(stmt.name, (ty, value));
        ctx.register_declared_var(&declared_name);
        Ok(None)
    }

    pub(crate) fn execute_declare_table_var(
        &mut self,
        stmt: crate::ast::DeclareTableVarStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        let unique = ctx.next_table_var_id();
        let physical = format!(
            "__tablevar_{}_{}_{}",
            ctx.frame.depth,
            stmt.name.trim_start_matches('@'),
            unique
        );
        let create = crate::ast::CreateTableStmt {
            name: crate::ast::ObjectName {
                database: None,
                schema: Some("dbo".to_string()),
                name: physical.clone(),
            },
            columns: stmt.columns,
            table_constraints: stmt.table_constraints,
        };
        self.schema(ctx).create_table(create)?;
        ctx.register_table_var(&stmt.name, &physical);
        Ok(None)
    }

    pub(crate) fn execute_set(
        &mut self,
        stmt: SetStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        if let Some((ty, _)) = ctx.session.get_var(&stmt.name) {
            let target_ty = ty.clone();
            crate::executor::value_ops::coercion::xml::check_implicit_xml_var_conversion(
                &stmt.expr, &target_ty, ctx,
            )?;
        }
        let val = crate::executor::evaluator::eval_expr(
            &stmt.expr,
            &[],
            ctx,
            self.catalog,
            self.storage,
            self.clock,
        )?;
        if let Some((ty, var)) = ctx.session.get_var_mut(&stmt.name) {
            let coerced = crate::executor::value_ops::coerce_value_to_type_with_dateformat(
                val,
                ty,
                &ctx.options.dateformat,
            )?;
            *var = coerced;
        } else {
            return Err(DbError::invalid_identifier(&stmt.name));
        }
        Ok(None)
    }
}
