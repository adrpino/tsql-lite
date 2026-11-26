use crate::ast::{TableFactor, TableRef};
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::storage::Storage;

use super::super::QueryExecutor;
use super::query_result_to_bound_table;
use crate::executor::clock::Clock;
use crate::executor::context::ExecutionContext;
use crate::executor::model::BoundTable;

pub(super) fn bind_view(
    catalog: &dyn Catalog,
    _storage: &dyn Storage,
    _clock: &dyn Clock,
    tref: &TableRef,
    ctx: &mut ExecutionContext,
    executor: &QueryExecutor<'_>,
) -> Result<Option<BoundTable>, DbError> {
    let schema = tref
        .factor
        .as_object_name()
        .map(|o| o.schema_or_dbo())
        .unwrap_or("dbo");
    let name = match &tref.factor {
        TableFactor::Named(o) => &o.name,
        TableFactor::Derived(_) => return Ok(None),
        TableFactor::Values { .. } => return Ok(None),
    };

    let Some(view) = catalog.find_view(schema, name).cloned() else {
        return Ok(None);
    };
    crate::executor::security::check_object_permission(
        catalog,
        ctx,
        "SELECT",
        &view.schema,
        &view.name,
        view.object_id,
    )?;

    let result = match view.query.as_ref() {
        crate::ast::Statement::Dml(crate::ast::DmlStatement::Select(s)) => {
            executor.execute_select(super::super::plan::RelationalQuery::from_ref(s), ctx)?
        }
        crate::ast::Statement::WithCte(_) => {
            let mut temp_cat = catalog.clone_boxed();
            let mut temp_stor = executor.storage.clone_boxed();
            let mut script = crate::executor::script::ScriptExecutor {
                catalog: temp_cat.as_mut(),
                storage: temp_stor.as_mut(),
                clock: executor.clock,
            };
            match script.execute(view.query.as_ref().clone(), ctx)? {
                crate::error::StmtOutcome::Ok(Some(res)) => res,
                _ => crate::executor::result::QueryResult::default(),
            }
        }
        _ => return Err(DbError::Execution("view query must be SELECT".into())),
    };
    Ok(Some(query_result_to_bound_table(
        tref.alias.clone().unwrap_or_else(|| name.clone()),
        name.clone(),
        result,
    )))
}
