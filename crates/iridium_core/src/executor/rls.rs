use crate::ast::{FunctionBody, RoutineParamType};
use crate::catalog::{
    Catalog, RoutineKind, SecurityBlockOp, SecurityPredicateDef, SecurityPredicateKind, TableDef,
};
use crate::error::DbError;
use crate::executor::clock::Clock;
use crate::executor::context::ExecutionContext;
use crate::executor::model::JoinedRow;
use crate::executor::query::plan::RelationalQuery;
use crate::executor::query::QueryExecutor;
use crate::executor::type_mapping::data_type_spec_to_runtime;
use crate::executor::value_ops::coerce_value_to_type_with_dateformat;
use crate::storage::{Storage, StoredRow};
use crate::types::Value;

/// Returns all enabled FILTER predicates targeting `(table.schema_name, table.name)`.
pub(crate) fn active_filter_predicates<'a>(
    catalog: &'a dyn Catalog,
    table: &TableDef,
) -> Vec<&'a SecurityPredicateDef> {
    let policies = catalog.get_security_policies();
    if policies.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for policy in policies {
        if !policy.is_enabled {
            continue;
        }
        for pred in &policy.predicates {
            if pred.kind == SecurityPredicateKind::Filter
                && pred.target_schema.eq_ignore_ascii_case(&table.schema_name)
                && pred.target_table.eq_ignore_ascii_case(&table.name)
            {
                out.push(pred);
            }
        }
    }
    out
}

/// Returns all enabled BLOCK predicates targeting `(table.schema_name, table.name)` for `op`.
pub(crate) fn active_block_predicates<'a>(
    catalog: &'a dyn Catalog,
    table: &TableDef,
    op: SecurityBlockOp,
) -> Vec<&'a SecurityPredicateDef> {
    let policies = catalog.get_security_policies();
    if policies.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for policy in policies {
        if !policy.is_enabled {
            continue;
        }
        for pred in &policy.predicates {
            if pred.kind == SecurityPredicateKind::Block
                && pred.target_schema.eq_ignore_ascii_case(&table.schema_name)
                && pred.target_table.eq_ignore_ascii_case(&table.name)
                && match pred.block_op {
                    None | Some(SecurityBlockOp::All) => true,
                    Some(o) => o == op,
                }
            {
                out.push(pred);
            }
        }
    }
    out
}

/// Evaluates a single RLS predicate TVF on `row`. Returns `true` if the predicate TVF
/// produces at least one row (allowing access), or `false` if 0 rows (filtering/blocking).
fn eval_predicate_on_row(
    pred: &SecurityPredicateDef,
    table: &TableDef,
    row: &StoredRow,
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<bool, DbError> {
    // Guard against recursive predicate evaluation on the same call chain.
    if ctx.frame.depth >= 8 {
        return Ok(true);
    }
    let routine = catalog
        .find_routine(&pred.function_schema, &pred.function_name)
        .ok_or_else(|| {
            DbError::object_not_found(format!("{}.{}", pred.function_schema, pred.function_name))
        })?
        .clone();
    let RoutineKind::Function { body, .. } = routine.kind else {
        return Err(DbError::Execution(format!(
            "Security predicate '{}.{}' must be an inline table-valued function.",
            pred.function_schema, pred.function_name
        )));
    };
    let select_stmt = match body.as_ref() {
        FunctionBody::InlineTable(sel) => sel.as_ref().clone(),
        _ => {
            return Err(DbError::Execution(format!(
                "Security predicate '{}.{}' must be an inline table-valued function.",
                pred.function_schema, pred.function_name
            )));
        }
    };

    // Bind TVF arguments from `row` columns.
    let mut saved_vars: Vec<(String, Option<(crate::types::DataType, Value)>)> =
        Vec::with_capacity(routine.params.len());
    for (param, arg_col) in routine.params.iter().zip(pred.function_args.iter()) {
        let col_idx = table
            .columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(arg_col))
            .ok_or_else(|| DbError::column_not_found(arg_col))?;
        let raw_val = row.values.get(col_idx).cloned().unwrap_or(Value::Null);
        let ty = match &param.param_type {
            RoutineParamType::Scalar(dt) => data_type_spec_to_runtime(dt),
            RoutineParamType::TableType(_) => {
                return Err(DbError::Execution(
                    "Security predicate TVF does not support table-valued parameters.".into(),
                ));
            }
        };
        let coerced = if raw_val.is_null() {
            Value::Null
        } else {
            coerce_value_to_type_with_dateformat(raw_val, &ty, &ctx.options.dateformat)?
        };
        let prev = ctx
            .session
            .variables
            .insert(param.name.clone(), (ty, coerced));
        saved_vars.push((param.name.clone(), prev));
    }

    ctx.frame.depth += 1;
    let qe = QueryExecutor {
        catalog,
        storage,
        clock,
    };
    let res = qe.execute_select(RelationalQuery::from(select_stmt), ctx);
    ctx.frame.depth -= 1;

    for (name, prev) in saved_vars {
        match prev {
            Some(v) => {
                ctx.session.variables.insert(name, v);
            }
            None => {
                ctx.session.variables.remove(&name);
            }
        }
    }

    let qr = res?;
    Ok(!qr.rows.is_empty())
}

/// Filters `rows` in-place according to all active RLS FILTER predicates on `table`.
/// Fast path: returns immediately when no security policies exist.
pub(crate) fn filter_rows_by_rls(
    table: &TableDef,
    rows: &mut Vec<JoinedRow>,
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<(), DbError> {
    if catalog.get_security_policies().is_empty() || rows.is_empty() {
        return Ok(());
    }
    let preds: Vec<SecurityPredicateDef> = active_filter_predicates(catalog, table)
        .into_iter()
        .cloned()
        .collect();
    if preds.is_empty() {
        return Ok(());
    }
    let mut kept = Vec::with_capacity(rows.len());
    for joined in rows.drain(..) {
        let Some(stored) = joined.first().and_then(|b| b.row.as_ref()) else {
            continue;
        };
        let mut allowed = true;
        for pred in &preds {
            if !eval_predicate_on_row(pred, table, stored, ctx, catalog, storage, clock)? {
                allowed = false;
                break;
            }
        }
        if allowed {
            kept.push(joined);
        }
    }
    *rows = kept;
    Ok(())
}

/// Checks whether a single `StoredRow` passes all active RLS FILTER predicates on `table`.
pub(crate) fn row_passes_rls_filter(
    table: &TableDef,
    row: &StoredRow,
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<bool, DbError> {
    if catalog.get_security_policies().is_empty() {
        return Ok(true);
    }
    let preds: Vec<SecurityPredicateDef> = active_filter_predicates(catalog, table)
        .into_iter()
        .cloned()
        .collect();
    for pred in &preds {
        if !eval_predicate_on_row(pred, table, row, ctx, catalog, storage, clock)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Enforces all active RLS BLOCK predicates for `op` on `rows`, raising SQL Server Error 33504
/// if any row is rejected.
pub(crate) fn enforce_rls_block_predicates(
    table: &TableDef,
    rows: &[StoredRow],
    op: SecurityBlockOp,
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<(), DbError> {
    if catalog.get_security_policies().is_empty() || rows.is_empty() {
        return Ok(());
    }
    let preds: Vec<SecurityPredicateDef> = active_block_predicates(catalog, table, op)
        .into_iter()
        .cloned()
        .collect();
    if preds.is_empty() {
        return Ok(());
    }
    for row in rows {
        for pred in &preds {
            if !eval_predicate_on_row(pred, table, row, ctx, catalog, storage, clock)? {
                return Err(DbError::Custom {
                    class: 16,
                    number: 33504,
                    message: format!(
                        "The attempted operation failed because the target object '{}.{}' has a block predicate that conflicts with this operation. If the operation is performed on a view, the block predicate might be enforced on the underlying table. Modify the operation to target only the rows that are allowed by the block predicate.",
                        table.schema_name, table.name
                    ),
                });
            }
        }
    }
    Ok(())
}
