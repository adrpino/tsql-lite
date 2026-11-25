use crate::ast::Expr;
use crate::error::DbError;
use crate::types::Value;

use super::context::ExecutionContext;
use super::model::ContextTable;

static NULL_VALUE: Value = Value::Null;

/// Resolves a simple column reference (`Identifier` or `QualifiedIdentifier`) against a fixed
/// row `shape` into a direct `(table_idx, col_idx)` slot so hot loops (joins, GROUP BY,
/// aggregates) can index `&row[table_idx].row.values[col_idx]` without per-row string scans
/// or `Value::clone()`. Returns `None` for variables, `IDENTITYCOL`, complex expressions,
/// ambiguous names, or outer-scope references (caller falls back to `eval_expr`).
pub(crate) fn resolve_column_slot(expr: &Expr, shape: &[ContextTable]) -> Option<(usize, usize)> {
    match expr {
        Expr::Identifier(name) => {
            if name.starts_with('@') || (name.len() == 11 && name.eq_ignore_ascii_case("IDENTITYCOL"))
            {
                return None;
            }
            let mut found: Option<(usize, usize)> = None;
            for (t_idx, binding) in shape.iter().enumerate() {
                if let Some(c_idx) = binding
                    .table
                    .columns
                    .iter()
                    .position(|c| c.name.eq_ignore_ascii_case(name))
                {
                    if found.is_some() {
                        return None; // Ambiguous — defer to eval_expr for proper error
                    }
                    found = Some((t_idx, c_idx));
                }
            }
            found
        }
        Expr::QualifiedIdentifier(parts) => {
            if parts.len() < 2 || parts.len() > 4 {
                return None;
            }
            let (schema_filter, table_name, column_name) = match parts.len() {
                2 => (None, &parts[0], &parts[1]),
                3 => (Some(&parts[0]), &parts[1], &parts[2]),
                _ => (Some(&parts[1]), &parts[2], &parts[3]),
            };
            if column_name.len() == 11 && column_name.eq_ignore_ascii_case("IDENTITYCOL") {
                return None;
            }
            for (t_idx, binding) in shape.iter().enumerate() {
                if let Some(schema) = schema_filter {
                    if !binding.table.schema_name.eq_ignore_ascii_case(schema) {
                        continue;
                    }
                }
                if binding.alias.eq_ignore_ascii_case(table_name)
                    || binding.table.name.eq_ignore_ascii_case(table_name)
                    || binding
                        .source_aliases
                        .iter()
                        .any(|a| a.eq_ignore_ascii_case(table_name))
                {
                    return binding
                        .table
                        .columns
                        .iter()
                        .position(|c| c.name.eq_ignore_ascii_case(column_name))
                        .map(|c_idx| (t_idx, c_idx));
                }
            }
            None
        }
        _ => None,
    }
}

#[inline]
pub(crate) fn value_at_slot(row: &[ContextTable], slot: (usize, usize)) -> &Value {
    row.get(slot.0)
        .and_then(|ct| ct.row.as_deref())
        .and_then(|r| r.values.get(slot.1))
        .unwrap_or(&NULL_VALUE)
}

#[inline]
fn find_column_in_scope(
    scope: &[ContextTable],
    name: &str,
    is_identity_col: bool,
) -> Result<Option<Value>, DbError> {
    let mut found: Option<(&ContextTable, usize)> = None;
    for binding in scope {
        let col_idx = if is_identity_col {
            binding
                .table
                .columns
                .iter()
                .position(|c| c.identity.is_some())
        } else {
            binding
                .table
                .columns
                .iter()
                .position(|c| c.name.eq_ignore_ascii_case(name))
        };
        if let Some(idx) = col_idx {
            if found.is_some() {
                return Err(DbError::invalid_identifier(format!(
                    "ambiguous column '{}'",
                    name
                )));
            }
            found = Some((binding, idx));
        }
    }
    Ok(found.map(|(binding, col_idx)| {
        binding
            .row
            .as_ref()
            .map(|r| r.values[col_idx].clone())
            .unwrap_or(Value::Null)
    }))
}

pub(crate) fn resolve_identifier(
    row: &[ContextTable],
    name: &str,
    ctx: &ExecutionContext,
) -> Result<Value, DbError> {
    if name.starts_with("@@") {
        if let Some(val) = super::metadata::system_vars::resolve_system_variable(name, ctx)? {
            return Ok(val);
        }
    }
    if name.starts_with('@') {
        match ctx.session.get_var(name) {
            Some((_, val)) => return Ok(val.clone()),
            None => {
                return Err(DbError::invalid_identifier(name));
            }
        }
    }

    let is_identity_col = name.len() == 11 && name.eq_ignore_ascii_case("IDENTITYCOL");

    if let Some(val) = find_column_in_scope(row, name, is_identity_col)? {
        return Ok(val);
    }

    for apply_row in ctx.row.apply_stack.iter().rev() {
        if let Some(val) = find_column_in_scope(apply_row, name, is_identity_col)? {
            return Ok(val);
        }
    }

    for outer_row in ctx.row.outer_stack.iter().rev() {
        if let Some(val) = find_column_in_scope(outer_row, name, is_identity_col)? {
            return Ok(val);
        }
    }

    Err(DbError::column_not_found(name))
}

pub(crate) fn resolve_qualified_identifier(
    row: &[ContextTable],
    parts: &[String],
    ctx: &ExecutionContext,
) -> Result<Value, DbError> {
    if parts.len() < 2 || parts.len() > 4 {
        return Err(DbError::invalid_identifier(
            "only two- to four-part column identifiers are supported",
        ));
    }

    let (schema_filter, table_name, column_name) = match parts.len() {
        2 => (None, &parts[0], &parts[1]),
        3 => (Some(&parts[0]), &parts[1], &parts[2]),
        _ => (Some(&parts[1]), &parts[2], &parts[3]),
    };
    let is_identity_col = column_name.eq_ignore_ascii_case("IDENTITYCOL");

    let search_row = |row: &[ContextTable]| -> Result<Option<Value>, DbError> {
        for binding in row {
            if let Some(schema) = schema_filter {
                if !binding.table.schema_name.eq_ignore_ascii_case(schema) {
                    continue;
                }
            }
            if binding.alias.eq_ignore_ascii_case(table_name)
                || binding.table.name.eq_ignore_ascii_case(table_name)
                || binding
                    .source_aliases
                    .iter()
                    .any(|a| a.eq_ignore_ascii_case(table_name))
            {
                let idx = if is_identity_col {
                    binding
                        .table
                        .columns
                        .iter()
                        .position(|c| c.identity.is_some())
                } else {
                    binding
                        .table
                        .columns
                        .iter()
                        .position(|c| c.name.eq_ignore_ascii_case(column_name))
                };

                if let Some(idx) = idx {
                    return Ok(Some(
                        binding
                            .row
                            .as_ref()
                            .map(|r| r.values[idx].clone())
                            .unwrap_or(Value::Null),
                    ));
                } else {
                    return Err(DbError::column_not_found_qualified(table_name, column_name));
                }
            }
        }
        Ok(None)
    };

    if let Some(val) = search_row(row)? {
        return Ok(val);
    }

    for apply_row in ctx.row.apply_stack.iter().rev() {
        if let Some(val) = search_row(apply_row)? {
            return Ok(val);
        }
    }

    for outer_row in ctx.row.outer_stack.iter().rev() {
        if let Some(val) = search_row(outer_row)? {
            return Ok(val);
        }
    }

    Err(DbError::object_not_found(table_name))
}
