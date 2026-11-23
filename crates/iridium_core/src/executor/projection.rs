use std::cmp::Ordering;

use crate::ast::{Expr, OrderByExpr, SelectItem, TopSpec};
use crate::error::DbError;
use crate::types::Value;

use super::clock::Clock;
use super::context::ExecutionContext;
use super::evaluator::eval_constant_expr;
use super::model::JoinedRow;
use super::value_ops::compare_values;
use crate::catalog::Catalog;
use crate::storage::Storage;

pub fn eval_top_count(
    top: &TopSpec,
    total_rows: usize,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<usize, DbError> {
    let val = eval_constant_expr(&top.value, ctx, catalog, storage, clock)?;
    if top.percent {
        let pct = match val {
            Value::TinyInt(v) => v as f64,
            Value::SmallInt(v) => v as f64,
            Value::Int(v) => v as f64,
            Value::BigInt(v) => v as f64,
            Value::Float(bits) => f64::from_bits(bits),
            Value::Decimal(unscaled, scale) => (unscaled as f64) / 10_f64.powi(scale as i32),
            _ => {
                return Err(DbError::Execution(
                    "TOP PERCENT requires a numeric expression".into(),
                ));
            }
        };
        if !(0.0..=100.0).contains(&pct) {
            return Err(DbError::Execution(
                "PERCENT values must be between 0 and 100.".into(),
            ));
        }
        let exact = (pct / 100.0) * (total_rows as f64);
        Ok(exact.ceil().max(0.0) as usize)
    } else {
        match val {
            Value::TinyInt(v) => Ok(v as usize),
            Value::SmallInt(v) => Ok(v.max(0) as usize),
            Value::Int(v) => Ok(v.max(0) as usize),
            Value::BigInt(v) => Ok(v.max(0) as usize),
            Value::Decimal(unscaled, scale) if scale == 0 => Ok(unscaled.max(0) as usize),
            _ => Err(DbError::Execution(
                "TOP currently requires an integer expression".into(),
            )),
        }
    }
}

pub fn expand_projection_columns(
    projection: &[SelectItem],
    sample: Option<&JoinedRow>,
) -> Vec<String> {
    let mut columns = Vec::new();
    for item in projection {
        columns.extend(expand_projection_labels(item, sample));
    }
    columns
}

fn expand_projection_labels(item: &SelectItem, sample: Option<&JoinedRow>) -> Vec<String> {
    match &item.expr {
        Expr::Wildcard => {
            if let Some(row) = sample {
                row.iter()
                    .flat_map(|binding| binding.table.columns.iter().map(|c| c.name.clone()))
                    .collect()
            } else {
                vec!["*".to_string()]
            }
        }
        Expr::QualifiedWildcard(parts) => {
            if let Some(row) = sample {
                let table_name = match parts.last() {
                    Some(name) => name,
                    None => return vec!["*".to_string()],
                };
                row.iter()
                    .filter(|binding| {
                        binding.alias.eq_ignore_ascii_case(table_name)
                            || binding.table.name.eq_ignore_ascii_case(table_name)
                            || binding
                                .source_aliases
                                .iter()
                                .any(|a| a.eq_ignore_ascii_case(table_name))
                    })
                    .flat_map(|binding| binding.table.columns.iter().map(|c| c.name.clone()))
                    .collect()
            } else {
                vec![format!("{}.*", parts.join("."))]
            }
        }
        _ => vec![item.alias.clone().unwrap_or_else(|| expr_label(&item.expr))],
    }
}

pub fn expand_qualified_wildcard_values(row: &JoinedRow, table_name: &str) -> Vec<Value> {
    let mut values = Vec::new();
    for binding in row {
        if binding.alias.eq_ignore_ascii_case(table_name)
            || binding.table.name.eq_ignore_ascii_case(table_name)
            || binding
                .source_aliases
                .iter()
                .any(|a| a.eq_ignore_ascii_case(table_name))
        {
            for (idx, _) in binding.table.columns.iter().enumerate() {
                let value = binding
                    .row
                    .as_ref()
                    .map(|r| r.values[idx].clone())
                    .unwrap_or(Value::Null);
                values.push(value);
            }
        }
    }
    values
}

pub fn expand_wildcard_values(row: &JoinedRow) -> Vec<Value> {
    let mut values = Vec::new();
    for binding in row {
        for (idx, _) in binding.table.columns.iter().enumerate() {
            let value = binding
                .row
                .as_ref()
                .map(|r| r.values[idx].clone())
                .unwrap_or(Value::Null);
            values.push(value);
        }
    }
    values
}

pub fn expr_label(expr: &Expr) -> String {
    match expr {
        Expr::Identifier(name) => name.clone(),
        Expr::QualifiedIdentifier(parts) => {
            parts.last().cloned().unwrap_or_else(|| "expr".to_string())
        }
        Expr::FunctionCall { name, .. } => name.clone(),
        Expr::Cast { .. } => "CAST".to_string(),
        Expr::TryCast { .. } => "TRY_CAST".to_string(),
        Expr::Convert { .. } => "CONVERT".to_string(),
        Expr::TryConvert { .. } => "TRY_CONVERT".to_string(),
        Expr::Wildcard => "*".to_string(),
        Expr::QualifiedWildcard(parts) => format!("{}.*", parts.join(".")),
        Expr::Case { .. } => "CASE".to_string(),
        Expr::InList { .. } => "IN".to_string(),
        Expr::Between { .. } => "BETWEEN".to_string(),
        Expr::Like { .. } => "LIKE".to_string(),
        Expr::Unary { expr: inner, .. } => expr_label(inner),
        Expr::Subquery(_) => "subquery".to_string(),
        Expr::Exists { .. } => "EXISTS".to_string(),
        Expr::InSubquery { .. } => "IN".to_string(),
        _ => "expr".to_string(),
    }
}

pub fn compare_projected_rows(
    a: &[Value],
    b: &[Value],
    columns: &[String],
    order_by: &[OrderByExpr],
) -> Result<Ordering, DbError> {
    compare_projected_rows_with_projection(a, b, columns, &[], order_by)
}

pub fn compare_projected_rows_with_projection(
    a: &[Value],
    b: &[Value],
    columns: &[String],
    projection: &[crate::ast::SelectItem],
    order_by: &[OrderByExpr],
) -> Result<Ordering, DbError> {
    for item in order_by {
        let idx = resolve_projected_order_index_with_projection(columns, projection, item)
            .ok_or_else(|| {
                DbError::invalid_identifier(format!(
                    "invalid column in ORDER BY: {}",
                    expr_label(&item.expr)
                ))
            })?;
        let ord = if let Some(flags) = super::collation::expr_collation(&item.expr) {
            super::collation::compare_values_with_collation(
                a.get(idx).unwrap_or(&Value::Null),
                b.get(idx).unwrap_or(&Value::Null),
                flags,
            )
        } else {
            compare_values(
                a.get(idx).unwrap_or(&Value::Null),
                b.get(idx).unwrap_or(&Value::Null),
            )
        };
        if ord != Ordering::Equal {
            return Ok(if item.asc { ord } else { ord.reverse() });
        }
    }
    Ok(Ordering::Equal)
}

pub fn resolve_projected_order_index(columns: &[String], item: &OrderByExpr) -> Option<usize> {
    resolve_projected_order_index_with_projection(columns, &[], item)
}

pub fn resolve_projected_order_index_with_projection(
    columns: &[String],
    projection: &[crate::ast::SelectItem],
    item: &OrderByExpr,
) -> Option<usize> {
    let target_expr = match &item.expr {
        Expr::FunctionCall { name, args, .. } if name == "__COLLATE" && !args.is_empty() => {
            &args[0]
        }
        other => other,
    };
    let target_label = match target_expr {
        Expr::Identifier(name) => name.clone(),
        Expr::QualifiedIdentifier(parts) => parts.last().cloned().unwrap_or_default(),
        _ => expr_label(target_expr),
    };
    if let Some(idx) = columns
        .iter()
        .position(|c| c.eq_ignore_ascii_case(&target_label))
    {
        return Some(idx);
    }
    // Fall back to matching the unaliased projection expression (e.g.
    // `SELECT [e].[Category] AS [Key] ... GROUP BY [e].[Category] ORDER BY [e].[Category]`).
    if !projection.is_empty() {
        return projection
            .iter()
            .position(|p| expr_label(&p.expr).eq_ignore_ascii_case(&target_label));
    }
    None
}

pub fn deduplicate_projected_rows(rows: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for row in rows {
        let key: Vec<super::value_ops::SqlHashKey> = row
            .iter()
            .map(super::value_ops::SqlHashKey::from_value)
            .collect();
        if seen.insert(key) {
            result.push(row);
        }
    }
    result
}
