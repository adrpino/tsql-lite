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

/// Streams non-NULL values for `expr` over `group.rows` into `visitor` without allocating
/// a temporary `Vec<Value>`, using a pre-resolved `(table_idx, col_idx)` slot whenever
/// `expr` is a simple column reference.
#[inline]
fn visit_group_values<F>(
    expr: &Expr,
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
    mut visitor: F,
) -> Result<(), DbError>
where
    F: FnMut(&Value) -> Result<(), DbError>,
{
    if group.rows.is_empty() {
        return Ok(());
    }
    if let Some(slot) = super::identifier::resolve_column_slot(expr, &group.rows[0]) {
        for row in &group.rows {
            let v = super::identifier::value_at_slot(row, slot);
            if !v.is_null() {
                visitor(v)?;
            }
        }
        return Ok(());
    }
    for row in &group.rows {
        if let Ok(v) = eval_expr(expr, row, ctx, catalog, storage, clock) {
            if !v.is_null() {
                visitor(&v)?;
            }
        }
    }
    Ok(())
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
        let mut cnt = 0i64;
        let _ = visit_group_values(expr, group, ctx, catalog, storage, clock, |_| {
            cnt += 1;
            Ok(())
        });
        cnt
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
    let count = if let Some(expr) = args.first() {
        let values = collect_group_values(expr, group, ctx, catalog, storage, clock);
        let mut seen = std::collections::HashSet::new();
        values
            .into_iter()
            .filter(|v| seen.insert(format!("{:?}", v)))
            .count() as i64
    } else {
        group.rows.len() as i64
    };
    Value::BigInt(count)
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

#[derive(Default)]
struct SumAcc {
    sum_i64: i64,
    sum_f64: f64,
    sum_i128: i128,
    current_scale: u8,
    count: usize,
    is_decimal: bool,
    is_float: bool,
    is_money: bool,
    first_kind: u8, // 1=TinyInt, 2=SmallInt, 3=Int, 4=BigInt
}

impl SumAcc {
    #[inline]
    fn push(&mut self, val: &Value) -> Result<(), DbError> {
        if self.count == 0 {
            match val {
                Value::Decimal(_, s) => {
                    self.is_decimal = true;
                    self.current_scale = *s;
                }
                Value::Float(_) => self.is_float = true,
                Value::Money(_) | Value::SmallMoney(_) => self.is_money = true,
                Value::TinyInt(_) => self.first_kind = 1,
                Value::SmallInt(_) => self.first_kind = 2,
                Value::Int(_) => self.first_kind = 3,
                Value::BigInt(_) => self.first_kind = 4,
                _ => {}
            }
        }
        self.count += 1;

        match val {
            Value::Float(v) => {
                self.is_float = true;
                self.sum_f64 += f64::from_bits(*v);
            }
            Value::Decimal(raw, scale) => {
                self.is_decimal = true;
                if *scale == self.current_scale {
                    self.sum_i128 += *raw;
                } else {
                    let max_scale = self.current_scale.max(*scale);
                    self.sum_i128 =
                        super::value_helpers::rescale_raw(self.sum_i128, self.current_scale, max_scale)
                            + super::value_helpers::rescale_raw(*raw, *scale, max_scale);
                    self.current_scale = max_scale;
                }
            }
            Value::Money(v) => {
                self.is_money = true;
                self.sum_i128 += *v;
            }
            Value::SmallMoney(v) => {
                self.is_money = true;
                self.sum_i128 += *v as i128;
            }
            Value::TinyInt(v) => self.sum_i64 += *v as i64,
            Value::SmallInt(v) => self.sum_i64 += *v as i64,
            Value::Int(v) => self.sum_i64 += *v as i64,
            Value::BigInt(v) => self.sum_i64 += *v,
            _ => return Err(DbError::Execution("SUM requires numeric argument".into())),
        }
        Ok(())
    }

    fn finish_sum(&self) -> Value {
        if self.count == 0 {
            return Value::Null;
        }
        if self.is_float {
            Value::Float(self.sum_f64.to_bits())
        } else if self.is_money {
            Value::Money(self.sum_i128)
        } else if self.is_decimal {
            Value::Decimal(self.sum_i128, self.current_scale)
        } else {
            Value::BigInt(self.sum_i64)
        }
    }

    fn finish_avg(&self) -> Value {
        if self.count == 0 {
            return Value::Null;
        }
        let n = self.count as i128;
        if self.is_float {
            Value::Float((self.sum_f64 / self.count as f64).to_bits())
        } else if self.is_money {
            Value::Money(self.sum_i128 / n)
        } else if self.is_decimal {
            let result_scale = self.current_scale.max(6);
            let dividend = rescale_raw(self.sum_i128, self.current_scale, result_scale);
            Value::Decimal(dividend / n, result_scale)
        } else {
            let res = self.sum_i64 / self.count as i64;
            match self.first_kind {
                1 => Value::TinyInt(res as u8),
                2 => Value::SmallInt(res as i16),
                3 => Value::Int(res as i32),
                _ => Value::BigInt(res),
            }
        }
    }
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
    if !distinct {
        let mut acc = SumAcc::default();
        visit_group_values(expr, group, ctx, catalog, storage, clock, |v| acc.push(v))?;
        return Ok(acc.finish_sum());
    }
    let values = dedup_aggregate_values(collect_group_values(
        expr, group, ctx, catalog, storage, clock,
    ));
    let mut acc = SumAcc::default();
    for v in &values {
        acc.push(v)?;
    }
    Ok(acc.finish_sum())
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
    if !distinct {
        let mut acc = SumAcc::default();
        visit_group_values(expr, group, ctx, catalog, storage, clock, |v| acc.push(v))?;
        return Ok(acc.finish_avg());
    }
    let values = dedup_aggregate_values(collect_group_values(
        expr, group, ctx, catalog, storage, clock,
    ));
    let mut acc = SumAcc::default();
    for v in &values {
        acc.push(v)?;
    }
    Ok(acc.finish_avg())
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
    let mut best: Option<Value> = None;
    visit_group_values(expr, group, ctx, catalog, storage, clock, |v| {
        if best
            .as_ref()
            .is_none_or(|b| compare_values(v, b) == std::cmp::Ordering::Less)
        {
            best = Some(v.clone());
        }
        Ok(())
    })?;
    Ok(best.unwrap_or(Value::Null))
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
    let mut best: Option<Value> = None;
    visit_group_values(expr, group, ctx, catalog, storage, clock, |v| {
        if best
            .as_ref()
            .is_none_or(|b| compare_values(v, b) == std::cmp::Ordering::Greater)
        {
            best = Some(v.clone());
        }
        Ok(())
    })?;
    Ok(best.unwrap_or(Value::Null))
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
