pub(crate) mod conversion;
pub(crate) mod literal;
pub(crate) mod predicate;
pub(crate) mod special;
pub(crate) mod subquery;
pub(crate) mod udf;

pub(crate) use udf::eval_udf_body;

use crate::ast::Expr;
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::executor::clock::Clock;
use crate::executor::context::ExecutionContext;
use crate::executor::model::ContextTable;
use crate::executor::operators::{eval_binary, eval_unary};
use crate::executor::scalar::eval_function;
use crate::executor::value_ops::coerce_value_to_type_with_dateformat;
use crate::storage::Storage;
use crate::types::{DataType, Value};

pub(crate) fn eval_expr_to_type_constant(
    expr: &Expr,
    ty: &DataType,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let value = eval_constant_expr(expr, ctx, catalog, storage, clock)?;
    coerce_value_to_type_with_dateformat(value, ty, &ctx.options.dateformat)
}

pub(crate) fn eval_expr_to_type_in_context(
    expr: &Expr,
    ty: &DataType,
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let mut sub_ctx = ctx.with_outer_row(row.to_vec());
    let value = eval_expr(expr, row, &mut sub_ctx, catalog, storage, clock)?;
    coerce_value_to_type_with_dateformat(value, ty, &ctx.options.dateformat)
}

pub(crate) fn eval_constant_expr(
    expr: &Expr,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let row: Vec<ContextTable> = vec![];
    eval_expr(expr, &row, ctx, catalog, storage, clock)
}

const MAX_RECURSION_DEPTH: usize = 256;

#[inline]
pub fn eval_expr(
    expr: &Expr,
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if ctx.frame.depth > MAX_RECURSION_DEPTH {
        return Err(DbError::Execution(format!(
            "Maximum recursion depth ({}) exceeded",
            MAX_RECURSION_DEPTH
        )));
    }

    ctx.frame.depth += 1;
    let res = eval_expr_inner(expr, row, ctx, catalog, storage, clock);
    ctx.frame.depth -= 1;
    res
}

#[inline(always)]
fn eval_expr_inner(
    expr: &Expr,
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    match expr {
        Expr::Identifier(_)
        | Expr::QualifiedIdentifier(_)
        | Expr::Wildcard
        | Expr::QualifiedWildcard(_)
        | Expr::Integer(_)
        | Expr::FloatLiteral(_)
        | Expr::BinaryLiteral(_)
        | Expr::String(_)
        | Expr::UnicodeString(_)
        | Expr::Null => literal::eval_literal_expr(expr, row, ctx),

        Expr::Cast { .. }
        | Expr::TryCast { .. }
        | Expr::Convert { .. }
        | Expr::TryConvert { .. }
        | Expr::IsNull(_)
        | Expr::IsNotNull(_) => {
            conversion::eval_conversion_expr(expr, row, ctx, catalog, storage, clock)
        }

        Expr::Case { .. }
        | Expr::InList { .. }
        | Expr::Between { .. }
        | Expr::Like { .. } => {
            predicate::eval_predicate_expr(expr, row, ctx, catalog, storage, clock)
        }

        Expr::Subquery(_) | Expr::Exists { .. } | Expr::InSubquery { .. } => {
            subquery::eval_subquery_expr(expr, row, ctx, catalog, storage, clock)
        }

        Expr::WindowFunction { .. } | Expr::NextValueFor { .. } => {
            special::eval_special_runtime_expr(expr, ctx)
        }

        Expr::FunctionCall { name, args, .. } => {
            if name == "__COLLATE" && !args.is_empty() {
                return eval_expr(&args[0], row, ctx, catalog, storage, clock);
            }
            if name == "__AT_TIME_ZONE" && args.len() == 2 {
                let dt_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
                let tz_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
                if dt_val.is_null() || tz_val.is_null() {
                    return Ok(Value::Null);
                }
                return eval_at_time_zone(dt_val, &tz_val.to_string_value());
            }
            eval_function(name, args, row, ctx, catalog, storage, clock)
        }

        Expr::Binary { left, op, right } => {
            let lv = eval_expr(left, row, ctx, catalog, storage, clock)?;
            let rv = eval_expr(right, row, ctx, catalog, storage, clock)?;
            if let Some(cs) = expr_collation_case_sensitive(left)
                .or_else(|| expr_collation_case_sensitive(right))
            {
                if cs && matches!(
                    op,
                    crate::ast::BinaryOp::Eq
                        | crate::ast::BinaryOp::NotEq
                        | crate::ast::BinaryOp::Gt
                        | crate::ast::BinaryOp::Lt
                        | crate::ast::BinaryOp::Gte
                        | crate::ast::BinaryOp::Lte
                ) {
                    if let (Some(ls), Some(rs)) = (value_as_str(&lv), value_as_str(&rv)) {
                        let ord = ls.trim_end_matches(' ').cmp(rs.trim_end_matches(' '));
                        let b = match op {
                            crate::ast::BinaryOp::Eq => ord == std::cmp::Ordering::Equal,
                            crate::ast::BinaryOp::NotEq => ord != std::cmp::Ordering::Equal,
                            crate::ast::BinaryOp::Gt => ord == std::cmp::Ordering::Greater,
                            crate::ast::BinaryOp::Lt => ord == std::cmp::Ordering::Less,
                            crate::ast::BinaryOp::Gte => {
                                matches!(ord, std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
                            }
                            crate::ast::BinaryOp::Lte => {
                                matches!(ord, std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
                            }
                            _ => unreachable!(),
                        };
                        return Ok(Value::Bit(b));
                    }
                }
            }
            eval_binary(
                op,
                lv,
                rv,
                ctx.metadata.ansi_nulls,
                ctx.options.concat_null_yields_null,
                ctx.options.arithabort,
                ctx.options.ansi_warnings,
            )
        }

        Expr::Unary { op, expr: inner } => {
            let val = eval_expr(inner, row, ctx, catalog, storage, clock)?;
            eval_unary(op, val)
        }
    }
}

pub(crate) fn eval_predicate(
    expr: &Expr,
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<bool, DbError> {
    let value = eval_expr(expr, row, ctx, catalog, storage, clock)?;
    let result = match &value {
        Value::Bit(v) => *v,
        Value::Null => false,
        other => crate::executor::value_ops::truthy(other),
    };
    Ok(result)
}

fn expr_collation_case_sensitive(expr: &Expr) -> Option<bool> {
    if let Expr::FunctionCall { name, args, .. } = expr {
        if name == "__COLLATE" {
            if let Some(Expr::String(col)) = args.get(1) {
                let upper = col.to_ascii_uppercase();
                return Some(
                    upper.contains("_CS_")
                        || upper.ends_with("_CS")
                        || upper.contains("_BIN")
                        || upper.ends_with("_BIN2"),
                );
            }
        }
    }
    None
}

fn value_as_str(v: &Value) -> Option<&str> {
    match v {
        Value::Char(s) | Value::VarChar(s) | Value::NChar(s) | Value::NVarChar(s) => Some(s),
        _ => None,
    }
}

fn windows_tz_offset_minutes(tz_name: &str) -> Option<i64> {
    match tz_name.trim().to_ascii_lowercase().as_str() {
        "utc" | "gmt standard time" | "greenwich standard time" => Some(0),
        "eastern standard time" | "us eastern standard time" => Some(-300),
        "central standard time" | "central america standard time" => Some(-360),
        "mountain standard time" | "us mountain standard time" => Some(-420),
        "pacific standard time" => Some(-480),
        "alaskan standard time" => Some(-540),
        "hawaiian standard time" => Some(-600),
        "atlantic standard time" => Some(-240),
        "central european standard time" | "w. europe standard time" => Some(60),
        _ => None,
    }
}

fn eval_at_time_zone(dt_val: Value, tz_name: &str) -> Result<Value, DbError> {
    let offset_mins = windows_tz_offset_minutes(tz_name).unwrap_or(0);
    let sign = if offset_mins < 0 { '-' } else { '+' };
    let abs_mins = offset_mins.abs();
    let off_h = abs_mins / 60;
    let off_m = abs_mins % 60;
    match dt_val {
        Value::DateTime(dt) | Value::DateTime2(dt) | Value::SmallDateTime(dt) => {
            Ok(Value::DateTimeOffset(format!(
                "{} {}{:02}:{:02}",
                dt.format("%Y-%m-%d %H:%M:%S%.f"),
                sign,
                off_h,
                off_m
            )))
        }
        Value::Date(d) => Ok(Value::DateTimeOffset(format!(
            "{} 00:00:00 {}{:02}:{:02}",
            d.format("%Y-%m-%d"),
            sign,
            off_h,
            off_m
        ))),
        Value::DateTimeOffset(s) => {
            // Convert existing datetimeoffset to target timezone by adjusting UTC instant.
            let trimmed = s.trim();
            if let Ok(fixed) = chrono::DateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S%.f %:z")
                .or_else(|_| chrono::DateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S %:z"))
            {
                let shifted = fixed.naive_utc() + chrono::Duration::minutes(offset_mins);
                return Ok(Value::DateTimeOffset(format!(
                    "{} {}{:02}:{:02}",
                    shifted.format("%Y-%m-%d %H:%M:%S%.f"),
                    sign,
                    off_h,
                    off_m
                )));
            }
            Ok(Value::DateTimeOffset(s))
        }
        other => Ok(other),
    }
}
