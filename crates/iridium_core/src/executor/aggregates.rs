use crate::ast::Expr;
use crate::error::DbError;
use crate::types::Value;

use super::clock::Clock;
use super::context::ExecutionContext;
use super::evaluator::eval_expr;
pub use super::model::Group;
use super::string_norm::normalize_identifier;
use super::value_helpers::rescale_raw;
use super::value_ops::compare_values;
use crate::catalog::Catalog;
use crate::storage::Storage;

/// Typed enum for aggregate functions, replacing string-based dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateFn {
    Count,
    CountDistinct,
    CountBig,
    CountBigDistinct,
    Sum,
    SumDistinct,
    Avg,
    AvgDistinct,
    Min,
    Max,
    Stdev,
    StdevDistinct,
    Stdevp,
    StdevpDistinct,
    Var,
    VarDistinct,
    Varp,
    VarpDistinct,
    ChecksumAgg,
    ChecksumAggDistinct,
    StringAgg,
    JsonObjectAggNull,
    JsonObjectAggAbsent,
    JsonArrayAggAbsent,
    JsonArrayAggNull,
    ApproxCountDistinct,
    ApproxPercentileCont,
    ApproxPercentileDisc,
}

impl AggregateFn {
    /// Parse an aggregate function name (case-insensitive) into a typed enum.
    pub fn from_name(name: &str) -> Option<Self> {
        match normalize_identifier(name).as_str() {
            "COUNT" => Some(AggregateFn::Count),
            "COUNT_DISTINCT" => Some(AggregateFn::CountDistinct),
            "COUNT_BIG" => Some(AggregateFn::CountBig),
            "COUNT_BIG_DISTINCT" => Some(AggregateFn::CountBigDistinct),
            "SUM" => Some(AggregateFn::Sum),
            "SUM_DISTINCT" => Some(AggregateFn::SumDistinct),
            "AVG" => Some(AggregateFn::Avg),
            "AVG_DISTINCT" => Some(AggregateFn::AvgDistinct),
            "MIN" => Some(AggregateFn::Min),
            "MAX" => Some(AggregateFn::Max),
            "STDEV" => Some(AggregateFn::Stdev),
            "STDEV_DISTINCT" => Some(AggregateFn::StdevDistinct),
            "STDEVP" => Some(AggregateFn::Stdevp),
            "STDEVP_DISTINCT" => Some(AggregateFn::StdevpDistinct),
            "VAR" => Some(AggregateFn::Var),
            "VAR_DISTINCT" => Some(AggregateFn::VarDistinct),
            "VARP" => Some(AggregateFn::Varp),
            "VARP_DISTINCT" => Some(AggregateFn::VarpDistinct),
            "CHECKSUM_AGG" => Some(AggregateFn::ChecksumAgg),
            "CHECKSUM_AGG_DISTINCT" => Some(AggregateFn::ChecksumAggDistinct),
            "STRING_AGG" => Some(AggregateFn::StringAgg),
            "JSON_OBJECTAGG" | "__JSON_OBJECTAGG_NULL" => Some(AggregateFn::JsonObjectAggNull),
            "__JSON_OBJECTAGG_ABSENT" => Some(AggregateFn::JsonObjectAggAbsent),
            "JSON_ARRAYAGG" | "__JSON_ARRAYAGG_ABSENT" => Some(AggregateFn::JsonArrayAggAbsent),
            "__JSON_ARRAYAGG_NULL" => Some(AggregateFn::JsonArrayAggNull),
            "APPROX_COUNT_DISTINCT" => Some(AggregateFn::ApproxCountDistinct),
            "APPROX_PERCENTILE_CONT" => Some(AggregateFn::ApproxPercentileCont),
            "APPROX_PERCENTILE_DISC" => Some(AggregateFn::ApproxPercentileDisc),
            _ => None,
        }
    }
}

pub fn sort_group_for_within_group(
    group: &Group,
    order_by: &[crate::ast::OrderByExpr],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Group {
    let mut rows = group.rows.clone();
    rows.sort_by(|a, b| {
        for item in order_by {
            let va = eval_expr(&item.expr, a, ctx, catalog, storage, clock).unwrap_or(Value::Null);
            let vb = eval_expr(&item.expr, b, ctx, catalog, storage, clock).unwrap_or(Value::Null);
            let ord = compare_values(&va, &vb);
            if ord != std::cmp::Ordering::Equal {
                return if item.asc { ord } else { ord.reverse() };
            }
        }
        std::cmp::Ordering::Equal
    });
    Group {
        key: group.key.clone(),
        rows,
    }
}

pub fn is_aggregate_function(name: &str) -> bool {
    AggregateFn::from_name(name).is_some()
}

/// Centralized aggregate dispatch. Returns None if the function is not a recognized aggregate.
/// This enables OCP: add new aggregates here without modifying caller code.
#[allow(clippy::too_many_arguments)]
pub fn dispatch_aggregate(
    name: &str,
    args: &[Expr],
    within_group: Option<&[crate::ast::OrderByExpr]>,
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Option<Result<Value, DbError>> {
    let agg = AggregateFn::from_name(name)?;
    match agg {
        AggregateFn::Count | AggregateFn::CountBig => Some(Ok(eval_aggregate_count(
            args, group, ctx, catalog, storage, clock,
        ))),
        AggregateFn::CountDistinct
        | AggregateFn::CountBigDistinct
        | AggregateFn::ApproxCountDistinct => Some(Ok(eval_aggregate_count_distinct(
            args, group, ctx, catalog, storage, clock,
        ))),
        AggregateFn::Sum => Some(eval_aggregate_sum_inner(
            args, false, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::SumDistinct => Some(eval_aggregate_sum_inner(
            args, true, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::Avg => Some(eval_aggregate_avg_inner(
            args, false, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::AvgDistinct => Some(eval_aggregate_avg_inner(
            args, true, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::Min => Some(eval_aggregate_min(
            args, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::Max => Some(eval_aggregate_max(
            args, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::Stdev => Some(eval_aggregate_stat(
            args, true, true, false, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::StdevDistinct => Some(eval_aggregate_stat(
            args, true, true, true, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::Stdevp => Some(eval_aggregate_stat(
            args, false, true, false, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::StdevpDistinct => Some(eval_aggregate_stat(
            args, false, true, true, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::Var => Some(eval_aggregate_stat(
            args, true, false, false, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::VarDistinct => Some(eval_aggregate_stat(
            args, true, false, true, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::Varp => Some(eval_aggregate_stat(
            args, false, false, false, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::VarpDistinct => Some(eval_aggregate_stat(
            args, false, false, true, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::ChecksumAgg => Some(eval_aggregate_checksum_agg(
            args, false, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::ChecksumAggDistinct => Some(eval_aggregate_checksum_agg(
            args, true, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::StringAgg => Some(eval_aggregate_string_agg(
            args, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::JsonObjectAggNull => Some(eval_aggregate_json_objectagg(
            args, false, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::JsonObjectAggAbsent => Some(eval_aggregate_json_objectagg(
            args, true, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::JsonArrayAggAbsent => Some(eval_aggregate_json_arrayagg(
            args, false, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::JsonArrayAggNull => Some(eval_aggregate_json_arrayagg(
            args, true, group, ctx, catalog, storage, clock,
        )),
        AggregateFn::ApproxPercentileCont => Some(eval_aggregate_approx_percentile(
            args,
            within_group,
            true,
            group,
            ctx,
            catalog,
            storage,
            clock,
        )),
        AggregateFn::ApproxPercentileDisc => Some(eval_aggregate_approx_percentile(
            args,
            within_group,
            false,
            group,
            ctx,
            catalog,
            storage,
            clock,
        )),
    }
}

pub fn collect_group_values<'a>(
    expr: &'a Expr,
    group: &'a Group,
    ctx: &'a mut ExecutionContext,
    catalog: &'a dyn Catalog,
    storage: &'a dyn Storage,
    clock: &'a dyn Clock,
) -> Vec<Value> {
    group
        .rows
        .iter()
        .filter_map(move |row| eval_expr(expr, row, ctx, catalog, storage, clock).ok())
        .filter(|v| !v.is_null())
        .collect()
}

pub fn eval_aggregate_count(
    args: &[Expr],
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Value {
    let count = if args.first().is_some_and(|a| matches!(a, Expr::Wildcard)) {
        group.rows.len() as i64
    } else if let Some(expr) = args.first() {
        collect_group_values(expr, group, ctx, catalog, storage, clock).len() as i64
    } else {
        group.rows.len() as i64
    };
    Value::BigInt(count)
}

pub fn eval_aggregate_count_distinct(
    args: &[Expr],
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Value {
    if let Some(expr) = args.first() {
        let values = collect_group_values(expr, group, ctx, catalog, storage, clock);
        let mut seen = std::collections::HashSet::new();
        let count = values
            .into_iter()
            .filter(|v| seen.insert(format!("{:?}", v)))
            .count() as i64;
        Value::BigInt(count)
    } else {
        Value::BigInt(group.rows.len() as i64)
    }
}

fn dedup_aggregate_values(values: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(values.len());
    for v in values {
        if !out
            .iter()
            .any(|existing| compare_values(existing, &v) == std::cmp::Ordering::Equal)
        {
            out.push(v);
        }
    }
    out
}

fn eval_aggregate_sum_inner(
    args: &[Expr],
    distinct: bool,
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let expr = args
        .first()
        .ok_or_else(|| DbError::Execution("SUM requires 1 argument".into()))?;
    let mut values = collect_group_values(expr, group, ctx, catalog, storage, clock);
    if distinct {
        values = dedup_aggregate_values(values);
    }
    sum_values(&values)
}

fn sum_values(values: &[Value]) -> Result<Value, DbError> {
    let mut sum_i64: i64 = 0;
    let mut sum_f64: f64 = 0.0;
    let mut sum_i128: i128 = 0;
    let mut current_scale: u8 = 0;
    let mut has_values = false;
    let mut is_decimal = false;
    let mut is_float = false;
    let mut is_money = false;

    for val in values {
        if !has_values {
            has_values = true;
            match val {
                Value::Decimal(_, s) => {
                    is_decimal = true;
                    current_scale = *s;
                }
                Value::Float(_) => {
                    is_float = true;
                }
                Value::Money(_) | Value::SmallMoney(_) => {
                    is_money = true;
                }
                _ => {}
            }
        }

        match val {
            Value::Float(v) => {
                is_float = true;
                sum_f64 += f64::from_bits(*v);
            }
            Value::Decimal(raw, scale) => {
                is_decimal = true;
                let max_scale = current_scale.max(*scale);
                sum_i128 = super::value_helpers::rescale_raw(sum_i128, current_scale, max_scale)
                    + super::value_helpers::rescale_raw(*raw, *scale, max_scale);
                current_scale = max_scale;
            }
            Value::Money(v) => {
                is_money = true;
                sum_i128 += *v;
            }
            Value::SmallMoney(v) => {
                is_money = true;
                sum_i128 += *v as i128;
            }
            Value::TinyInt(v) => sum_i64 += *v as i64,
            Value::SmallInt(v) => sum_i64 += *v as i64,
            Value::Int(v) => sum_i64 += *v as i64,
            Value::BigInt(v) => sum_i64 += *v,
            _ => return Err(DbError::Execution("SUM requires numeric argument".into())),
        }
    }

    if !has_values {
        return Ok(Value::Null);
    }

    if is_float {
        Ok(Value::Float(sum_f64.to_bits()))
    } else if is_money {
        Ok(Value::Money(sum_i128))
    } else if is_decimal {
        Ok(Value::Decimal(sum_i128, current_scale))
    } else {
        Ok(Value::BigInt(sum_i64))
    }
}

fn eval_aggregate_avg_inner(
    args: &[Expr],
    distinct: bool,
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let expr = args
        .first()
        .ok_or_else(|| DbError::Execution("AVG requires 1 argument".into()))?;
    let mut values = collect_group_values(expr, group, ctx, catalog, storage, clock);
    if distinct {
        values = dedup_aggregate_values(values);
    }
    if values.is_empty() {
        return Ok(Value::Null);
    }
    let first_val = &values[0];
    let is_float = matches!(first_val, Value::Float(_));
    let is_money = matches!(first_val, Value::Money(_) | Value::SmallMoney(_));

    if is_float {
        let mut sum_f64 = 0.0;
        for val in &values {
            if let Value::Float(v) = val {
                sum_f64 += f64::from_bits(*v);
            }
        }
        return Ok(Value::Float((sum_f64 / values.len() as f64).to_bits()));
    }

    if is_money {
        let mut sum_i128: i128 = 0;
        for val in &values {
            match val {
                Value::Money(v) => sum_i128 += *v,
                Value::SmallMoney(v) => sum_i128 += *v as i128,
                _ => {}
            }
        }
        return Ok(Value::Money(sum_i128 / values.len() as i128));
    }

    let sum = sum_values(&values)?;
    let n = values.len() as i128;
    match sum {
        Value::BigInt(v) => {
            let res = v / n as i64;
            match first_val {
                Value::TinyInt(_) => Ok(Value::TinyInt(res as u8)),
                Value::SmallInt(_) => Ok(Value::SmallInt(res as i16)),
                Value::Int(_) => Ok(Value::Int(res as i32)),
                _ => Ok(Value::BigInt(res)),
            }
        }
        Value::Decimal(v, s) => {
            let result_scale = s.max(6);
            let dividend = rescale_raw(v, s, result_scale);
            Ok(Value::Decimal(dividend / n, result_scale))
        }
        Value::Float(v) => Ok(Value::Float((f64::from_bits(v) / n as f64).to_bits())),
        Value::Money(v) => Ok(Value::Money(v / n)),
        Value::SmallMoney(v) => Ok(Value::SmallMoney((v as i128 / n) as i64)),
        _ => Ok(Value::Null),
    }
}

fn value_to_f64_numeric(val: &Value) -> Result<f64, DbError> {
    match val {
        Value::TinyInt(v) => Ok(*v as f64),
        Value::SmallInt(v) => Ok(*v as f64),
        Value::Int(v) => Ok(*v as f64),
        Value::BigInt(v) => Ok(*v as f64),
        Value::Float(bits) => Ok(f64::from_bits(*bits)),
        Value::Decimal(raw, scale) => Ok(*raw as f64 / 10_f64.powi(*scale as i32)),
        Value::Money(v) => Ok(*v as f64 / 10_000.0),
        Value::SmallMoney(v) => Ok(*v as f64 / 10_000.0),
        _ => Err(DbError::Execution(
            "Statistical aggregate requires numeric argument".into(),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn eval_aggregate_stat(
    args: &[Expr],
    sample: bool,
    take_sqrt: bool,
    distinct: bool,
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let expr = args
        .first()
        .ok_or_else(|| DbError::Execution("Statistical aggregate requires 1 argument".into()))?;
    let mut values = collect_group_values(expr, group, ctx, catalog, storage, clock);
    if distinct {
        values = dedup_aggregate_values(values);
    }
    let n = values.len();
    if (sample && n < 2) || (!sample && n == 0) {
        return Ok(Value::Null);
    }
    let mut nums = Vec::with_capacity(n);
    for v in &values {
        nums.push(value_to_f64_numeric(v)?);
    }
    let mean = nums.iter().sum::<f64>() / n as f64;
    let sum_sq: f64 = nums.iter().map(|x| (x - mean) * (x - mean)).sum();
    let denom = if sample { (n - 1) as f64 } else { n as f64 };
    let variance = sum_sq / denom;
    let result = if take_sqrt { variance.sqrt() } else { variance };
    Ok(Value::Float(result.to_bits()))
}

fn eval_aggregate_checksum_agg(
    args: &[Expr],
    distinct: bool,
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let expr = args
        .first()
        .ok_or_else(|| DbError::Execution("CHECKSUM_AGG requires 1 argument".into()))?;
    let mut values = collect_group_values(expr, group, ctx, catalog, storage, clock);
    if distinct {
        values = dedup_aggregate_values(values);
    }
    let mut acc: i32 = 0;
    for v in &values {
        let i = v
            .to_integer_i64()
            .ok_or_else(|| DbError::Execution("CHECKSUM_AGG requires integer argument".into()))?;
        acc ^= i as i32;
    }
    Ok(Value::Int(acc))
}

pub fn eval_aggregate_min(
    args: &[Expr],
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let expr = args
        .first()
        .ok_or_else(|| DbError::Execution("MIN requires 1 argument".into()))?;
    let values = collect_group_values(expr, group, ctx, catalog, storage, clock);
    Ok(values
        .into_iter()
        .min_by(compare_values)
        .unwrap_or(Value::Null))
}

pub fn eval_aggregate_max(
    args: &[Expr],
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let expr = args
        .first()
        .ok_or_else(|| DbError::Execution("MAX requires 1 argument".into()))?;
    let values = collect_group_values(expr, group, ctx, catalog, storage, clock);
    Ok(values
        .into_iter()
        .max_by(compare_values)
        .unwrap_or(Value::Null))
}

pub fn eval_aggregate_string_agg(
    args: &[Expr],
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() < 2 {
        return Err(DbError::Execution(
            "STRING_AGG requires at least 2 arguments: expression and separator".into(),
        ));
    }

    let expr = &args[0];
    let separator_expr = &args[1];

    let separator = eval_expr(separator_expr, &[], ctx, catalog, storage, clock)?;
    let separator_str = match separator {
        Value::VarChar(s) => s,
        Value::NVarChar(s) => s,
        Value::Char(s) => s,
        Value::NChar(s) => s,
        _ => {
            return Err(DbError::Execution(
                "STRING_AGG separator must be a string".into(),
            ))
        }
    };

    let values = collect_group_values(expr, group, ctx, catalog, storage, clock);
    if values.is_empty() {
        return Ok(Value::Null);
    }

    let string_values: Vec<String> = values.iter().map(|v| v.to_string_value()).collect();

    let result = string_values.join(&separator_str);
    Ok(Value::VarChar(result))
}

fn eval_aggregate_json_objectagg(
    args: &[Expr],
    absent_on_null: bool,
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution(
            "JSON_OBJECTAGG requires key and value expressions".into(),
        ));
    }
    let embed_raw = super::json::is_json_producing_expr(&args[1]);
    let mut pairs = Vec::with_capacity(group.rows.len());
    for row in &group.rows {
        let k = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        if k.is_null() {
            return Err(DbError::Execution(
                "JSON_OBJECTAGG key cannot be NULL".into(),
            ));
        }
        let v = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
        if absent_on_null && v.is_null() {
            continue;
        }
        pairs.push((k.to_string_value(), v, embed_raw));
    }
    if pairs.is_empty() {
        return Ok(Value::Null);
    }
    Ok(super::json::build_json_object(&pairs, absent_on_null))
}

fn eval_aggregate_json_arrayagg(
    args: &[Expr],
    null_on_null: bool,
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution(
            "JSON_ARRAYAGG requires 1 expression argument".into(),
        ));
    }
    let embed_raw = super::json::is_json_producing_expr(&args[0]);
    let mut items = Vec::with_capacity(group.rows.len());
    for row in &group.rows {
        let v = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        if !null_on_null && v.is_null() {
            continue;
        }
        items.push((v, embed_raw));
    }
    if items.is_empty() {
        return Ok(Value::Null);
    }
    Ok(super::json::build_json_array(&items, null_on_null))
}

#[allow(clippy::too_many_arguments)]
fn eval_aggregate_approx_percentile(
    args: &[Expr],
    within_group: Option<&[crate::ast::OrderByExpr]>,
    continuous: bool,
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let fn_name = if continuous {
        "APPROX_PERCENTILE_CONT"
    } else {
        "APPROX_PERCENTILE_DISC"
    };
    let p_expr = args
        .first()
        .ok_or_else(|| DbError::Execution(format!("{fn_name} requires a percentile argument")))?;
    let order_by = within_group.filter(|ob| !ob.is_empty()).ok_or_else(|| {
        DbError::Execution(format!("{fn_name} requires WITHIN GROUP (ORDER BY ...)"))
    })?;

    let p_val = eval_expr(p_expr, &[], ctx, catalog, storage, clock)?;
    if p_val.is_null() {
        return Ok(Value::Null);
    }
    let p = p_val
        .to_f64()
        .ok_or_else(|| DbError::Execution(format!("{fn_name} percentile must be numeric")))?;
    if !(0.0..=1.0).contains(&p) {
        return Err(DbError::Execution(format!(
            "{fn_name} percentile argument must be between 0.0 and 1.0"
        )));
    }

    let values = collect_group_values(&order_by[0].expr, group, ctx, catalog, storage, clock);
    if values.is_empty() {
        return Ok(Value::Null);
    }
    let n = values.len();
    if !continuous {
        let idx = if p == 0.0 {
            0
        } else {
            ((p * n as f64).ceil() as usize)
                .saturating_sub(1)
                .min(n - 1)
        };
        return Ok(values[idx].clone());
    }

    let pos = p * (n - 1) as f64;
    let lo = (pos.floor() as usize).min(n - 1);
    let hi = (pos.ceil() as usize).min(n - 1);
    let frac = pos - lo as f64;
    let v_lo = values[lo].to_f64().ok_or_else(|| {
        DbError::Execution(format!("{fn_name} requires numeric ORDER BY expression"))
    })?;
    let v_hi = values[hi].to_f64().ok_or_else(|| {
        DbError::Execution(format!("{fn_name} requires numeric ORDER BY expression"))
    })?;
    let interp = v_lo + (v_hi - v_lo) * frac;
    Ok(Value::Float(interp.to_bits()))
}
