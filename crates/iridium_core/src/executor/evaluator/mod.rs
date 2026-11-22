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

        Expr::Case { .. } | Expr::InList { .. } | Expr::Between { .. } | Expr::Like { .. } => {
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
            if name == "__XML_MODIFY" && args.len() >= 2 {
                let target_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
                let dml_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
                if target_val.is_null() || dml_val.is_null() {
                    return Ok(target_val);
                }
                let injected = if args.len() >= 3 {
                    let v = eval_expr(&args[2], row, ctx, catalog, storage, clock)?;
                    if v.is_null() {
                        None
                    } else {
                        Some(v.to_string_value())
                    }
                } else {
                    None
                };
                let modified = eval_xml_modify(
                    &target_val.to_string_value(),
                    &dml_val.to_string_value(),
                    injected.as_deref(),
                );
                return Ok(Value::NVarChar(modified));
            }
            if let Some((receiver, method)) = name.rsplit_once('.') {
                if args.len() == 2 && method.eq_ignore_ascii_case("value") {
                    let recv_expr = if receiver.contains('.') {
                        Expr::QualifiedIdentifier(
                            receiver.split('.').map(|s| s.to_string()).collect(),
                        )
                    } else {
                        Expr::Identifier(receiver.to_string())
                    };
                    let xml_val = eval_expr(&recv_expr, row, ctx, catalog, storage, clock)?;
                    if xml_val.is_null() {
                        return Ok(Value::Null);
                    }
                    let xpath_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
                    let sql_ty_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
                    if xpath_val.is_null() || sql_ty_val.is_null() {
                        return Ok(Value::Null);
                    }
                    let Some(extracted) =
                        eval_xml_value(&xml_val.to_string_value(), &xpath_val.to_string_value())
                    else {
                        return Ok(Value::Null);
                    };
                    let target_ty = parse_xml_value_target_type(&sql_ty_val.to_string_value());
                    return crate::executor::value_ops::coerce_value_to_type_with_dateformat(
                        Value::NVarChar(extracted),
                        &target_ty,
                        &ctx.options.dateformat,
                    );
                }
                if args.len() == 1 && method.eq_ignore_ascii_case("exist") {
                    let recv_expr = if receiver.contains('.') {
                        Expr::QualifiedIdentifier(
                            receiver.split('.').map(|s| s.to_string()).collect(),
                        )
                    } else {
                        Expr::Identifier(receiver.to_string())
                    };
                    let xml_val = eval_expr(&recv_expr, row, ctx, catalog, storage, clock)?;
                    if xml_val.is_null() {
                        return Ok(Value::Null);
                    }
                    let xpath_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
                    if xpath_val.is_null() {
                        return Ok(Value::Null);
                    }
                    let exists =
                        eval_xml_exist(&xml_val.to_string_value(), &xpath_val.to_string_value());
                    return Ok(Value::Bit(exists));
                }
                if args.len() == 1 && method.eq_ignore_ascii_case("query") {
                    let recv_expr = if receiver.contains('.') {
                        Expr::QualifiedIdentifier(
                            receiver.split('.').map(|s| s.to_string()).collect(),
                        )
                    } else {
                        Expr::Identifier(receiver.to_string())
                    };
                    let xml_val = eval_expr(&recv_expr, row, ctx, catalog, storage, clock)?;
                    if xml_val.is_null() {
                        return Ok(Value::Null);
                    }
                    let xpath_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
                    if xpath_val.is_null() {
                        return Ok(Value::Null);
                    }
                    let fragment =
                        eval_xml_query(&xml_val.to_string_value(), &xpath_val.to_string_value());
                    return Ok(Value::NVarChar(fragment));
                }
            }
            eval_function(name, args, row, ctx, catalog, storage, clock)
        }

        Expr::Binary { left, op, right } => {
            let lv = eval_expr(left, row, ctx, catalog, storage, clock)?;
            let rv = eval_expr(right, row, ctx, catalog, storage, clock)?;
            if let Some(cs) =
                expr_collation_case_sensitive(left).or_else(|| expr_collation_case_sensitive(right))
            {
                if cs
                    && matches!(
                        op,
                        crate::ast::BinaryOp::Eq
                            | crate::ast::BinaryOp::NotEq
                            | crate::ast::BinaryOp::Gt
                            | crate::ast::BinaryOp::Lt
                            | crate::ast::BinaryOp::Gte
                            | crate::ast::BinaryOp::Lte
                    )
                {
                    if let (Some(ls), Some(rs)) = (value_as_str(&lv), value_as_str(&rv)) {
                        let ord = ls.trim_end_matches(' ').cmp(rs.trim_end_matches(' '));
                        let b = match op {
                            crate::ast::BinaryOp::Eq => ord == std::cmp::Ordering::Equal,
                            crate::ast::BinaryOp::NotEq => ord != std::cmp::Ordering::Equal,
                            crate::ast::BinaryOp::Gt => ord == std::cmp::Ordering::Greater,
                            crate::ast::BinaryOp::Lt => ord == std::cmp::Ordering::Less,
                            crate::ast::BinaryOp::Gte => {
                                matches!(
                                    ord,
                                    std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
                                )
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

pub(crate) fn expr_collation_case_sensitive(expr: &Expr) -> Option<bool> {
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

pub(crate) fn compare_values_case_sensitive(a: &Value, b: &Value) -> std::cmp::Ordering {
    if let (Some(ls), Some(rs)) = (value_as_str(a), value_as_str(b)) {
        return ls.trim_end_matches(' ').cmp(rs.trim_end_matches(' '));
    }
    crate::executor::value_ops::compare_values(a, b)
}

fn value_as_str(v: &Value) -> Option<&str> {
    match v {
        Value::Char(s) | Value::VarChar(s) | Value::NChar(s) | Value::NVarChar(s) => Some(s),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum DstRule {
    None,
    /// US/Canada: 2nd Sunday of March 02:00 local -> 1st Sunday of November 02:00 local.
    Us,
    /// EU/UK: last Sunday of March 01:00 UTC -> last Sunday of October 01:00 UTC.
    Eu,
    /// Australia (AEST/AEDT): 1st Sunday of October 02:00 local -> 1st Sunday of April 03:00 local.
    Au,
    /// New Zealand: last Sunday of September 02:00 local -> 1st Sunday of April 03:00 local.
    Nz,
}

fn nth_sunday(year: i32, month: u32, nth: u32) -> chrono::NaiveDate {
    use chrono::Datelike;
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1).expect("valid first of month");
    let offset = (7 - first.weekday().num_days_from_sunday()) % 7;
    chrono::NaiveDate::from_ymd_opt(year, month, 1 + offset + (nth - 1) * 7).expect("valid date")
}

fn last_sunday(year: i32, month: u32) -> chrono::NaiveDate {
    use chrono::Datelike;
    let next_month_first = if month == 12 {
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .expect("valid next month");
    let last = next_month_first - chrono::Duration::days(1);
    let back = last.weekday().num_days_from_sunday() as i64;
    last - chrono::Duration::days(back)
}

fn resolve_timezone_spec(tz_name: &str) -> Option<(i64, DstRule)> {
    let trimmed = tz_name.trim();
    if let Some(mins) = parse_offset_minutes(trimmed) {
        return Some((mins, DstRule::None));
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "utc" | "etc/utc" | "greenwich standard time" | "coordinated universal time" => {
            Some((0, DstRule::None))
        }
        "gmt standard time" | "europe/london" | "europe/dublin" => Some((0, DstRule::Eu)),
        "eastern standard time" | "america/new_york" | "america/toronto" => {
            Some((-300, DstRule::Us))
        }
        "us eastern standard time" | "america/indiana/indianapolis" => Some((-300, DstRule::Us)),
        "central standard time" | "america/chicago" => Some((-360, DstRule::Us)),
        "central america standard time" => Some((-360, DstRule::None)),
        "mountain standard time" | "america/denver" => Some((-420, DstRule::Us)),
        "us mountain standard time" | "america/phoenix" => Some((-420, DstRule::None)),
        "pacific standard time" | "america/los_angeles" => Some((-480, DstRule::Us)),
        "alaskan standard time" | "america/anchorage" => Some((-540, DstRule::Us)),
        "hawaiian standard time" | "pacific/honolulu" => Some((-600, DstRule::None)),
        "atlantic standard time" | "america/halifax" => Some((-240, DstRule::Us)),
        "central european standard time"
        | "romance standard time"
        | "w. europe standard time"
        | "central europe standard time"
        | "europe/berlin"
        | "europe/paris"
        | "europe/madrid"
        | "europe/rome"
        | "europe/amsterdam" => Some((60, DstRule::Eu)),
        "e. europe standard time"
        | "fle standard time"
        | "gtb standard time"
        | "europe/athens"
        | "europe/helsinki" => Some((120, DstRule::Eu)),
        "russian standard time" | "europe/moscow" => Some((180, DstRule::None)),
        "arabian standard time" | "asia/dubai" => Some((240, DstRule::None)),
        "india standard time" | "asia/kolkata" | "asia/calcutta" => Some((330, DstRule::None)),
        "china standard time" | "singapore standard time" | "asia/shanghai" | "asia/singapore" => {
            Some((480, DstRule::None))
        }
        "tokyo standard time" | "korea standard time" | "asia/tokyo" | "asia/seoul" => {
            Some((540, DstRule::None))
        }
        "aus eastern standard time" | "australia/sydney" | "australia/melbourne" => {
            Some((600, DstRule::Au))
        }
        "new zealand standard time" | "pacific/auckland" => Some((720, DstRule::Nz)),
        "e. south america standard time" | "america/sao_paulo" => Some((-180, DstRule::None)),
        "argentina standard time" | "america/argentina/buenos_aires" => Some((-180, DstRule::None)),
        "sa pacific standard time" | "america/bogota" | "america/lima" => {
            Some((-300, DstRule::None))
        }
        _ => None,
    }
}

fn is_dst_for_utc(utc_dt: chrono::NaiveDateTime, std_offset_mins: i64, rule: DstRule) -> bool {
    use chrono::Datelike;
    let year = utc_dt.year();
    match rule {
        DstRule::None => false,
        DstRule::Us => {
            let start_local = nth_sunday(year, 3, 2)
                .and_hms_opt(2, 0, 0)
                .expect("valid hms");
            let start_utc = start_local - chrono::Duration::minutes(std_offset_mins);
            let end_local = nth_sunday(year, 11, 1)
                .and_hms_opt(2, 0, 0)
                .expect("valid hms");
            // End occurs while in DST (std_offset + 60)
            let end_utc = end_local - chrono::Duration::minutes(std_offset_mins + 60);
            utc_dt >= start_utc && utc_dt < end_utc
        }
        DstRule::Eu => {
            let start_utc = last_sunday(year, 3)
                .and_hms_opt(1, 0, 0)
                .expect("valid hms");
            let end_utc = last_sunday(year, 10)
                .and_hms_opt(1, 0, 0)
                .expect("valid hms");
            utc_dt >= start_utc && utc_dt < end_utc
        }
        DstRule::Au => {
            let end_local = nth_sunday(year, 4, 1)
                .and_hms_opt(3, 0, 0)
                .expect("valid hms");
            let end_utc = end_local - chrono::Duration::minutes(std_offset_mins + 60);
            let start_local = nth_sunday(year, 10, 1)
                .and_hms_opt(2, 0, 0)
                .expect("valid hms");
            let start_utc = start_local - chrono::Duration::minutes(std_offset_mins);
            utc_dt < end_utc || utc_dt >= start_utc
        }
        DstRule::Nz => {
            let end_local = nth_sunday(year, 4, 1)
                .and_hms_opt(3, 0, 0)
                .expect("valid hms");
            let end_utc = end_local - chrono::Duration::minutes(std_offset_mins + 60);
            let start_local = last_sunday(year, 9)
                .and_hms_opt(2, 0, 0)
                .expect("valid hms");
            let start_utc = start_local - chrono::Duration::minutes(std_offset_mins);
            utc_dt < end_utc || utc_dt >= start_utc
        }
    }
}

fn is_dst_for_local(local_dt: chrono::NaiveDateTime, std_offset_mins: i64, rule: DstRule) -> bool {
    let approx_utc = local_dt - chrono::Duration::minutes(std_offset_mins);
    is_dst_for_utc(approx_utc, std_offset_mins, rule)
}

fn parse_offset_minutes(raw: &str) -> Option<i64> {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("Z") {
        return Some(0);
    }
    let (sign, rest) = if let Some(r) = s.strip_prefix('+') {
        (1i64, r)
    } else if let Some(r) = s.strip_prefix('-') {
        (-1i64, r)
    } else {
        return None;
    };
    let (h_str, m_str) = rest.split_once(':')?;
    let h: i64 = h_str.parse().ok()?;
    let m: i64 = m_str.parse().ok()?;
    if !(0..=14).contains(&h) || !(0..60).contains(&m) || (h == 14 && m > 0) {
        return None;
    }
    Some(sign * (h * 60 + m))
}

fn format_datetimeoffset(dt: chrono::NaiveDateTime, offset_mins: i64) -> String {
    let sign = if offset_mins < 0 { '-' } else { '+' };
    let abs_mins = offset_mins.abs();
    let off_h = abs_mins / 60;
    let off_m = abs_mins % 60;
    format!(
        "{} {}{:02}:{:02}",
        dt.format("%Y-%m-%d %H:%M:%S%.f"),
        sign,
        off_h,
        off_m
    )
}

fn parse_datetimeoffset_utc(s: &str) -> Option<chrono::NaiveDateTime> {
    let trimmed = s.trim();
    chrono::DateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S%.f %:z")
        .or_else(|_| chrono::DateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S %:z"))
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(trimmed))
        .ok()
        .map(|fixed| fixed.naive_utc())
}

fn parse_offset_arg(val: &Value) -> Result<i64, DbError> {
    if let Some(mins) = val.to_integer_i64() {
        if !(-840..=840).contains(&mins) {
            return Err(DbError::Execution(format!(
                "Time zone offset '{}' is out of range (-840..840 minutes)",
                mins
            )));
        }
        return Ok(mins);
    }
    let s = val.to_string_value();
    parse_offset_minutes(&s)
        .ok_or_else(|| DbError::Execution(format!("Invalid time zone offset '{s}'")))
}

pub(crate) fn eval_todatetimeoffset(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution(
            "TODATETIMEOFFSET expects 2 arguments".into(),
        ));
    }
    let dt_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let off_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if dt_val.is_null() || off_val.is_null() {
        return Ok(Value::Null);
    }
    let offset_mins = parse_offset_arg(&off_val)?;
    let naive = match dt_val {
        Value::DateTime(dt) | Value::DateTime2(dt) | Value::SmallDateTime(dt) => dt,
        Value::Date(d) => d.and_hms_opt(0, 0, 0).expect("valid midnight"),
        Value::DateTimeOffset(s) => {
            // TODATETIMEOFFSET keeps wall-clock local time and replaces the offset.
            let trimmed = s.trim();
            chrono::DateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S%.f %:z")
                .or_else(|_| chrono::DateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S %:z"))
                .or_else(|_| chrono::DateTime::parse_from_rfc3339(trimmed))
                .map(|f| f.naive_local())
                .map_err(|_| DbError::Execution(format!("Invalid datetimeoffset '{s}'")))?
        }
        other => {
            let coerced = crate::executor::value_ops::coerce_value_to_type_with_dateformat(
                other,
                &crate::types::DataType::DateTime2,
                &ctx.options.dateformat,
            )?;
            match coerced {
                Value::DateTime2(dt) | Value::DateTime(dt) => dt,
                _ => {
                    return Err(DbError::Execution(
                        "TODATETIMEOFFSET expects a datetime".into(),
                    ))
                }
            }
        }
    };
    Ok(Value::DateTimeOffset(format_datetimeoffset(
        naive,
        offset_mins,
    )))
}

pub(crate) fn eval_switchoffset(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution(
            "SWITCHOFFSET expects 2 arguments".into(),
        ));
    }
    let dt_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let off_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if dt_val.is_null() || off_val.is_null() {
        return Ok(Value::Null);
    }
    let offset_mins = parse_offset_arg(&off_val)?;
    let utc_dt = match dt_val {
        Value::DateTimeOffset(ref s) => parse_datetimeoffset_utc(s)
            .ok_or_else(|| DbError::Execution(format!("Invalid datetimeoffset '{s}'")))?,
        Value::DateTime(dt) | Value::DateTime2(dt) | Value::SmallDateTime(dt) => dt,
        Value::Date(d) => d.and_hms_opt(0, 0, 0).expect("valid midnight"),
        other => {
            let s = other.to_string_value();
            parse_datetimeoffset_utc(&s)
                .ok_or_else(|| DbError::Execution(format!("Invalid datetimeoffset '{s}'")))?
        }
    };
    let shifted = utc_dt + chrono::Duration::minutes(offset_mins);
    Ok(Value::DateTimeOffset(format_datetimeoffset(
        shifted,
        offset_mins,
    )))
}

fn eval_at_time_zone(dt_val: Value, tz_name: &str) -> Result<Value, DbError> {
    let Some((std_offset_mins, dst_rule)) = resolve_timezone_spec(tz_name) else {
        return Err(DbError::Execution(format!(
            "The time zone parameter '{}' provided to AT TIME ZONE clause is invalid.",
            tz_name
        )));
    };
    match dt_val {
        Value::DateTime(dt) | Value::DateTime2(dt) | Value::SmallDateTime(dt) => {
            let offset_mins = if is_dst_for_local(dt, std_offset_mins, dst_rule) {
                std_offset_mins + 60
            } else {
                std_offset_mins
            };
            Ok(Value::DateTimeOffset(format_datetimeoffset(
                dt,
                offset_mins,
            )))
        }
        Value::Date(d) => {
            let dt = d.and_hms_opt(0, 0, 0).expect("valid midnight");
            let offset_mins = if is_dst_for_local(dt, std_offset_mins, dst_rule) {
                std_offset_mins + 60
            } else {
                std_offset_mins
            };
            Ok(Value::DateTimeOffset(format_datetimeoffset(
                dt,
                offset_mins,
            )))
        }
        Value::DateTimeOffset(s) => {
            if let Some(utc_dt) = parse_datetimeoffset_utc(&s) {
                let offset_mins = if is_dst_for_utc(utc_dt, std_offset_mins, dst_rule) {
                    std_offset_mins + 60
                } else {
                    std_offset_mins
                };
                let shifted = utc_dt + chrono::Duration::minutes(offset_mins);
                return Ok(Value::DateTimeOffset(format_datetimeoffset(
                    shifted,
                    offset_mins,
                )));
            }
            Ok(Value::DateTimeOffset(s))
        }
        other => Ok(other),
    }
}

fn eval_xml_modify(xml: &str, dml: &str, injected: Option<&str>) -> String {
    let trimmed = dml.trim();
    let lower = trimmed.to_ascii_lowercase();

    // Handle `delete (/Root/Child)` or `delete /Root/Child`
    if lower.starts_with("delete ") {
        let path_part = trimmed["delete ".len()..]
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim();
        let path_clean = path_part.split('[').next().unwrap_or(path_part).trim();
        let target_tag = path_clean.rsplit('/').next().unwrap_or("").trim();
        if !target_tag.is_empty() {
            return remove_xml_elements_by_tag(xml, target_tag);
        }
        return xml.to_string();
    }

    // Handle `insert <node|sql:column(...)> [as first|as last] into (/Root)[1]`
    if lower.starts_with("insert ") {
        if let Some(into_pos) = lower.rfind(" into ") {
            let before_into = trimmed["insert ".len()..into_pos].trim();
            let as_first = before_into.to_ascii_lowercase().ends_with("as first");
            let fragment = if let Some(inj) = injected {
                inj.to_string()
            } else {
                let b_lower = before_into.to_ascii_lowercase();
                let frag = if b_lower.ends_with("as first") {
                    before_into[..before_into.len() - "as first".len()].trim()
                } else if b_lower.ends_with("as last") {
                    before_into[..before_into.len() - "as last".len()].trim()
                } else {
                    before_into
                };
                frag.to_string()
            };
            let target_part = trimmed[into_pos + " into ".len()..].trim();
            let target_no_idx = target_part.split('[').next().unwrap_or(target_part).trim();
            let target_clean = target_no_idx
                .trim_start_matches('(')
                .trim_end_matches(')')
                .trim();
            let target_tag = target_clean.rsplit('/').next().unwrap_or("").trim();
            if !target_tag.is_empty() && !fragment.is_empty() {
                if as_first {
                    let open_prefix = format!("<{}", target_tag);
                    if let Some(pos) = xml.find(&open_prefix) {
                        if let Some(rel_end) = xml[pos..].find('>') {
                            let end_idx = pos + rel_end;
                            if xml.as_bytes().get(end_idx.saturating_sub(1)) == Some(&b'/') {
                                // Self-closing `<Tag/>` -> `<Tag>{fragment}</Tag>`
                                let mut out = String::with_capacity(
                                    xml.len() + fragment.len() + target_tag.len() + 4,
                                );
                                out.push_str(&xml[..end_idx - 1]);
                                out.push('>');
                                out.push_str(&fragment);
                                out.push_str("</");
                                out.push_str(target_tag);
                                out.push('>');
                                out.push_str(&xml[end_idx + 1..]);
                                return out;
                            }
                            let mut out = String::with_capacity(xml.len() + fragment.len());
                            out.push_str(&xml[..=end_idx]);
                            out.push_str(&fragment);
                            out.push_str(&xml[end_idx + 1..]);
                            return out;
                        }
                    }
                } else {
                    let close_tag = format!("</{}>", target_tag);
                    if let Some(pos) = xml.rfind(&close_tag) {
                        let mut out = String::with_capacity(xml.len() + fragment.len());
                        out.push_str(&xml[..pos]);
                        out.push_str(&fragment);
                        out.push_str(&xml[pos..]);
                        return out;
                    }
                    // Self-closing `<Tag/>` -> `<Tag>{fragment}</Tag>`
                    let open_prefix = format!("<{}", target_tag);
                    if let Some(pos) = xml.find(&open_prefix) {
                        if let Some(rel_end) = xml[pos..].find("/>") {
                            let mut out = String::with_capacity(
                                xml.len() + fragment.len() + target_tag.len() + 4,
                            );
                            out.push_str(&xml[..pos + rel_end]);
                            out.push('>');
                            out.push_str(&fragment);
                            out.push_str("</");
                            out.push_str(target_tag);
                            out.push('>');
                            out.push_str(&xml[pos + rel_end + 2..]);
                            return out;
                        }
                    }
                }
            }
        }
    }

    xml.to_string()
}

fn remove_xml_elements_by_tag(xml: &str, tag: &str) -> String {
    let open_prefix = format!("<{}", tag);
    let close_tag = format!("</{}>", tag);
    let mut result = xml.to_string();
    loop {
        let Some(start) = result.find(&open_prefix) else {
            break;
        };
        let after_name = start + open_prefix.len();
        let next_ch = result.as_bytes().get(after_name).copied();
        if !matches!(
            next_ch,
            Some(b'>') | Some(b'/') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
        ) {
            break;
        }
        let Some(gt_rel) = result[start..].find('>') else {
            break;
        };
        let gt_idx = start + gt_rel;
        if result.as_bytes().get(gt_idx.saturating_sub(1)) == Some(&b'/') {
            result.replace_range(start..=gt_idx, "");
            continue;
        }
        let Some(close_rel) = result[gt_idx + 1..].find(&close_tag) else {
            break;
        };
        let close_end = gt_idx + 1 + close_rel + close_tag.len();
        result.replace_range(start..close_end, "");
    }
    result
}

fn unescape_xml_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn strip_xml_tags(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut in_tag = false;
    for ch in xml.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    unescape_xml_entities(&out)
}

fn extract_xml_attr(open_tag: &str, attr: &str) -> Option<String> {
    let needle_dq = format!("{attr}=\"");
    if let Some(pos) = open_tag.find(&needle_dq) {
        let rest = &open_tag[pos + needle_dq.len()..];
        let end = rest.find('"')?;
        return Some(unescape_xml_entities(&rest[..end]));
    }
    let needle_sq = format!("{attr}='");
    if let Some(pos) = open_tag.find(&needle_sq) {
        let rest = &open_tag[pos + needle_sq.len()..];
        let end = rest.find('\'')?;
        return Some(unescape_xml_entities(&rest[..end]));
    }
    None
}

fn find_matching_xml_elements<'a>(xml: &'a str, tag: &str) -> Vec<(&'a str, &'a str, &'a str)> {
    let open_prefix = format!("<{tag}");
    let close_tag = format!("</{tag}>");
    let mut out = Vec::new();
    let mut search_pos = 0usize;

    while let Some(rel) = xml[search_pos..].find(&open_prefix) {
        let start = search_pos + rel;
        let after_name = start + open_prefix.len();
        let next_ch = xml.as_bytes().get(after_name).copied();
        if !matches!(
            next_ch,
            Some(b'>') | Some(b'/') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
        ) {
            search_pos = after_name;
            continue;
        }
        let Some(gt_rel) = xml[start..].find('>') else {
            break;
        };
        let gt_idx = start + gt_rel;
        let self_closing = xml.as_bytes().get(gt_idx.saturating_sub(1)) == Some(&b'/');
        if self_closing {
            let full = &xml[start..=gt_idx];
            out.push((full, full, ""));
            search_pos = gt_idx + 1;
        } else {
            // Depth-tracked scan for the matching closing tag of the same name.
            let mut depth = 1usize;
            let mut cursor = gt_idx + 1;
            let mut close_start = None;
            while cursor < xml.len() && depth > 0 {
                let next_open = xml[cursor..].find(&open_prefix).map(|p| cursor + p);
                let next_close = xml[cursor..].find(&close_tag).map(|p| cursor + p);
                match (next_open, next_close) {
                    (Some(o), Some(c)) if o < c => {
                        let after_o = o + open_prefix.len();
                        let ch = xml.as_bytes().get(after_o).copied();
                        if matches!(
                            ch,
                            Some(b'>')
                                | Some(b'/')
                                | Some(b' ')
                                | Some(b'\t')
                                | Some(b'\n')
                                | Some(b'\r')
                        ) {
                            if let Some(ogt) = xml[o..].find('>') {
                                let is_sc =
                                    xml.as_bytes().get((o + ogt).saturating_sub(1)) == Some(&b'/');
                                if !is_sc {
                                    depth += 1;
                                }
                                cursor = o + ogt + 1;
                                continue;
                            }
                        }
                        cursor = after_o;
                    }
                    (_, Some(c)) => {
                        depth -= 1;
                        if depth == 0 {
                            close_start = Some(c);
                        }
                        cursor = c + close_tag.len();
                    }
                    _ => break,
                }
            }
            if let Some(c_start) = close_start {
                let full_end = c_start + close_tag.len();
                let open_tag = &xml[start..=gt_idx];
                let inner = &xml[gt_idx + 1..c_start];
                let full = &xml[start..full_end];
                out.push((full, open_tag, inner));
                search_pos = full_end;
            } else {
                break;
            }
        }
    }
    out
}

fn match_xml_predicate(open_tag: &str, inner: &str, pred: &str) -> bool {
    let p = pred.trim();
    if p.is_empty() {
        return true;
    }
    if let Some((lhs, rhs)) = p.split_once('=') {
        let lhs = lhs.trim();
        let rhs_clean = rhs.trim().trim_matches('"').trim_matches('\'').to_string();
        if let Some(attr) = lhs.strip_prefix('@') {
            return extract_xml_attr(open_tag, attr).is_some_and(|v| v == rhs_clean);
        }
        if lhs.eq_ignore_ascii_case("text()") || lhs == "." {
            return strip_xml_tags(inner) == rhs_clean;
        }
        return find_matching_xml_elements(inner, lhs)
            .into_iter()
            .any(|(_, _, child_inner)| strip_xml_tags(child_inner) == rhs_clean);
    }
    if let Some(attr) = p.strip_prefix('@') {
        return extract_xml_attr(open_tag, attr).is_some();
    }
    !find_matching_xml_elements(inner, p).is_empty()
}

fn strip_outer_parens(s: &str) -> &str {
    let mut cur = s.trim();
    while let Some(inner) = cur.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
        // Only strip if the opening '(' matches the final ')'.
        let mut depth = 0i32;
        let mut wraps_all = true;
        for (idx, b) in cur.bytes().enumerate() {
            match b {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 && idx + 1 < cur.len() {
                        wraps_all = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if wraps_all && depth == 0 {
            cur = inner.trim();
        } else {
            break;
        }
    }
    cur
}

/// Evaluates an XPath expression against an XML document and returns the matching
/// outer XML element fragments.
pub(crate) fn eval_xml_nodes(xml: &str, xpath: &str) -> Vec<String> {
    let trimmed = strip_outer_parens(xpath);
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed == "." {
        let t = xml.trim();
        return if t.is_empty() {
            Vec::new()
        } else {
            vec![t.to_string()]
        };
    }

    let clean = trimmed.trim_start_matches('/');
    let raw_segments: Vec<&str> = clean.split('/').filter(|s| !s.is_empty()).collect();
    if raw_segments.is_empty() {
        return Vec::new();
    }

    let mut current: Vec<(&str, &str, &str)> = vec![(xml, xml, xml)];
    for seg in raw_segments {
        let seg = seg.trim();
        if seg == "." {
            continue;
        }
        if seg.eq_ignore_ascii_case("text()") || seg.starts_with('@') {
            break;
        }
        let (tag, pred) = if let Some(lb) = seg.find('[') {
            if let Some(rb) = seg[lb..].rfind(']') {
                (seg[..lb].trim(), Some(seg[lb + 1..lb + rb].trim()))
            } else {
                (seg, None)
            }
        } else {
            (seg, None)
        };
        let mut next_nodes = Vec::new();
        for &(_, _, scope_inner) in &current {
            let mut matched = find_matching_xml_elements(scope_inner, tag);
            if let Some(p) = pred {
                if let Ok(ord) = p.parse::<usize>() {
                    if ord >= 1 && ord <= matched.len() {
                        matched = vec![matched[ord - 1]];
                    } else {
                        matched.clear();
                    }
                } else {
                    matched.retain(|&(_, open_tag, inner)| match_xml_predicate(open_tag, inner, p));
                }
            }
            next_nodes.extend(matched);
        }
        current = next_nodes;
        if current.is_empty() {
            break;
        }
    }

    current
        .into_iter()
        .map(|(full, _, _)| full.to_string())
        .collect()
}

pub(crate) fn eval_xml_exist(xml: &str, xpath: &str) -> bool {
    let trimmed = strip_outer_parens(xpath);
    if trimmed.is_empty() {
        return false;
    }
    if let Some((prefix, last)) = trimmed.rsplit_once('/') {
        if let Some(attr) = last.trim().strip_prefix('@') {
            let parents = if prefix.trim().is_empty() {
                vec![xml.to_string()]
            } else {
                eval_xml_nodes(xml, prefix)
            };
            return parents.iter().any(|frag| {
                let open = frag
                    .find('>')
                    .map(|gt| &frag[..=gt])
                    .unwrap_or(frag.as_str());
                extract_xml_attr(open, attr).is_some()
            });
        }
    } else if let Some(attr) = trimmed.strip_prefix('@') {
        let open = xml.find('>').map(|gt| &xml[..=gt]).unwrap_or(xml);
        return extract_xml_attr(open, attr).is_some();
    }
    !eval_xml_nodes(xml, trimmed).is_empty()
}

pub(crate) fn eval_xml_query(xml: &str, xpath: &str) -> String {
    eval_xml_nodes(xml, xpath).concat()
}

fn eval_xml_value(xml: &str, xpath: &str) -> Option<String> {
    let trimmed = xpath.trim();
    if trimmed == "." || trimmed == "text()" || trimmed == "(./text())[1]" || trimmed == "(.)[1]" {
        return Some(strip_xml_tags(xml));
    }

    // Parse optional trailing `[N]` 1-based ordinal index and outer parens.
    let (path_body, ordinal) = if let Some(lb) = trimmed.rfind('[') {
        if let Some(rb) = trimmed[lb..].find(']') {
            let idx = trimmed[lb + 1..lb + rb]
                .trim()
                .parse::<usize>()
                .ok()
                .unwrap_or(1);
            (&trimmed[..lb], idx.max(1))
        } else {
            (trimmed, 1)
        }
    } else {
        (trimmed, 1)
    };
    let clean_path = strip_outer_parens(path_body).trim_start_matches('/');

    let segments: Vec<&str> = clean_path
        .split('/')
        .map(|s| s.split('[').next().unwrap_or(s).trim())
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("text()") && *s != ".")
        .collect();
    if segments.is_empty() {
        return Some(strip_xml_tags(xml));
    }

    let (elem_segments, attr_name) = if let Some(last) = segments.last() {
        if let Some(attr) = last.strip_prefix('@') {
            (&segments[..segments.len() - 1], Some(attr))
        } else {
            (segments.as_slice(), None)
        }
    } else {
        (segments.as_slice(), None)
    };

    // If only `@attr` was requested (e.g. `.value('@id', 'INT')` on a `.nodes()` fragment),
    // read directly from the root element's opening tag.
    if elem_segments.is_empty() {
        if let Some(attr) = attr_name {
            let open_tag = xml.find('>').map(|gt| &xml[..=gt]).unwrap_or(xml);
            return extract_xml_attr(open_tag, attr);
        }
        return Some(strip_xml_tags(xml));
    }

    // Walk element path segments in order, narrowing slice to the matching element.
    let mut slice = xml;
    let mut matched_open_tag = "";
    let mut matched_inner = "";
    for (seg_idx, tag) in elem_segments.iter().enumerate() {
        let target_ord = if seg_idx + 1 == elem_segments.len() {
            ordinal
        } else {
            1
        };
        let matches = find_matching_xml_elements(slice, tag);
        let Some(&(_, elem_open, elem_inner)) = matches.get(target_ord - 1) else {
            return None;
        };
        matched_open_tag = elem_open;
        matched_inner = elem_inner;
        slice = elem_inner;
    }

    if let Some(attr) = attr_name {
        return extract_xml_attr(matched_open_tag, attr);
    }

    Some(strip_xml_tags(matched_inner))
}

fn parse_xml_value_target_type(sql_type: &str) -> crate::types::DataType {
    let t = sql_type.trim();
    let base = t.split('(').next().unwrap_or(t).trim().to_ascii_uppercase();
    match base.as_str() {
        "BIT" => crate::types::DataType::Bit,
        "TINYINT" => crate::types::DataType::TinyInt,
        "SMALLINT" => crate::types::DataType::SmallInt,
        "INT" | "INTEGER" => crate::types::DataType::Int,
        "BIGINT" => crate::types::DataType::BigInt,
        "FLOAT" | "REAL" => crate::types::DataType::Float,
        "MONEY" => crate::types::DataType::Money,
        "SMALLMONEY" => crate::types::DataType::SmallMoney,
        "DECIMAL" | "NUMERIC" => crate::types::DataType::Decimal {
            precision: 18,
            scale: 4,
        },
        "DATE" => crate::types::DataType::Date,
        "TIME" => crate::types::DataType::Time,
        "DATETIME" => crate::types::DataType::DateTime,
        "DATETIME2" => crate::types::DataType::DateTime2,
        "UNIQUEIDENTIFIER" => crate::types::DataType::UniqueIdentifier,
        "VARCHAR" | "CHAR" => crate::types::DataType::VarChar { max_len: u16::MAX },
        _ => crate::types::DataType::NVarChar { max_len: u16::MAX },
    }
}
