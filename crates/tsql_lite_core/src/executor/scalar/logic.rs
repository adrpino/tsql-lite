use crate::ast::Expr;
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::storage::Storage;
use crate::types::Value;

use super::super::clock::Clock;
use super::super::context::ExecutionContext;
use super::super::evaluator::eval_expr;
use super::super::model::ContextTable;

pub(crate) fn eval_coalesce(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.is_empty() {
        return Err(DbError::Execution(
            "COALESCE requires at least one argument".into(),
        ));
    }
    let mut chosen = Value::Null;
    for arg in args {
        let val = eval_expr(arg, row, ctx, catalog, storage, clock)?;
        if !val.is_null() {
            chosen = val;
            break;
        }
    }
    if chosen.is_null() {
        return Ok(Value::Null);
    }
    // SQL Server COALESCE returns the highest-precedence data type among all arguments;
    // when an earlier NULL argument is a numeric column and the fallback is a string (e.g. ''),
    // coerce the fallback to the numeric column's type.
    if matches!(
        chosen,
        Value::Char(_) | Value::VarChar(_) | Value::NChar(_) | Value::NVarChar(_)
    ) {
        if let Some(target_ty) = highest_numeric_arg_type(args, row) {
            if let Ok(coerced) = crate::executor::value_ops::coerce_value_to_type_with_dateformat(
                chosen.clone(),
                &target_ty,
                &ctx.options.dateformat,
            ) {
                return Ok(coerced);
            }
        }
    }
    Ok(chosen)
}

fn highest_numeric_arg_type(args: &[Expr], row: &[ContextTable]) -> Option<crate::types::DataType> {
    let mut best: Option<crate::types::DataType> = None;
    for arg in args {
        let col_ty = match arg {
            Expr::Identifier(col) => row.iter().find_map(|ct| {
                ct.table
                    .columns
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case(col))
                    .map(|c| c.data_type.clone())
            }),
            Expr::QualifiedIdentifier(parts) if parts.len() >= 2 => {
                let col = parts.last().unwrap();
                let qual = &parts[parts.len() - 2];
                row.iter().find_map(|ct| {
                    if ct.alias.eq_ignore_ascii_case(qual)
                        || ct.table.name.eq_ignore_ascii_case(qual)
                    {
                        ct.table
                            .columns
                            .iter()
                            .find(|c| c.name.eq_ignore_ascii_case(col))
                            .map(|c| c.data_type.clone())
                    } else {
                        None
                    }
                })
            }
            _ => None,
        };
        if let Some(ty) = col_ty {
            if matches!(
                ty,
                crate::types::DataType::Bit
                    | crate::types::DataType::TinyInt
                    | crate::types::DataType::SmallInt
                    | crate::types::DataType::Int
                    | crate::types::DataType::BigInt
                    | crate::types::DataType::Decimal { .. }
                    | crate::types::DataType::Money
                    | crate::types::DataType::SmallMoney
                    | crate::types::DataType::Float
            ) {
                best = Some(match best {
                    None => ty,
                    Some(prev) => crate::types::type_precedence_join(&prev, &ty),
                });
            }
        }
    }
    best
}

pub(crate) fn eval_isnull(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution("ISNULL expects 2 arguments".into()));
    }
    let left = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if !left.is_null() {
        Ok(left)
    } else {
        eval_expr(&args[1], row, ctx, catalog, storage, clock)
    }
}

pub(crate) fn eval_iif(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 3 {
        return Err(DbError::Execution("IIF expects 3 arguments".into()));
    }
    let condition = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let is_true = match condition {
        Value::Bit(b) => b,
        Value::Null => false,
        Value::Int(v) => v != 0,
        Value::BigInt(v) => v != 0,
        Value::TinyInt(v) => v != 0,
        Value::SmallInt(v) => v != 0,
        _ => !condition.to_string_value().is_empty(),
    };
    if is_true {
        eval_expr(&args[1], row, ctx, catalog, storage, clock)
    } else {
        eval_expr(&args[2], row, ctx, catalog, storage, clock)
    }
}

pub(crate) fn eval_nullif(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution("NULLIF expects 2 arguments".into()));
    }
    let left = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let right = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if left.is_null() {
        return Ok(Value::Null);
    }
    if !right.is_null()
        && crate::executor::value_ops::compare_values(&left, &right) == std::cmp::Ordering::Equal
    {
        Ok(Value::Null)
    } else {
        Ok(left)
    }
}

pub(crate) fn eval_choose(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() < 2 {
        return Err(DbError::Execution(
            "CHOOSE expects at least 2 arguments".into(),
        ));
    }
    let index_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let idx = index_val.to_integer_i64().unwrap_or(0);
    if idx < 1 || idx as usize > args.len() - 1 {
        return Ok(Value::Null);
    }
    eval_expr(&args[idx as usize], row, ctx, catalog, storage, clock)
}

pub(crate) fn eval_greatest(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.is_empty() {
        return Err(DbError::Execution(
            "GREATEST requires at least 1 argument".into(),
        ));
    }
    let mut best: Option<Value> = None;
    for arg in args {
        let val = eval_expr(arg, row, ctx, catalog, storage, clock)?;
        if val.is_null() {
            continue;
        }
        match &best {
            None => best = Some(val),
            Some(cur)
                if crate::executor::value_ops::compare_values(&val, cur)
                    == std::cmp::Ordering::Greater =>
            {
                best = Some(val);
            }
            Some(_) => {}
        }
    }
    Ok(best.unwrap_or(Value::Null))
}

pub(crate) fn eval_least(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.is_empty() {
        return Err(DbError::Execution(
            "LEAST requires at least 1 argument".into(),
        ));
    }
    let mut best: Option<Value> = None;
    for arg in args {
        let val = eval_expr(arg, row, ctx, catalog, storage, clock)?;
        if val.is_null() {
            continue;
        }
        match &best {
            None => best = Some(val),
            Some(cur)
                if crate::executor::value_ops::compare_values(&val, cur)
                    == std::cmp::Ordering::Less =>
            {
                best = Some(val);
            }
            Some(_) => {}
        }
    }
    Ok(best.unwrap_or(Value::Null))
}
