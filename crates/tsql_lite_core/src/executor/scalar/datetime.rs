use chrono::{NaiveDate, Timelike};

use crate::ast::Expr;
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::storage::Storage;
use crate::types::Value;

use super::super::clock::Clock;
use super::super::context::ExecutionContext;
use super::super::date_time::{
    apply_dateadd, date_to_days, day_of_week_from_date, parse_datetime_parts,
};
use super::super::evaluator::eval_expr;
use super::super::model::ContextTable;

pub(crate) fn extract_datepart(expr: &Expr) -> Result<String, DbError> {
    match expr {
        Expr::Identifier(name) => Ok(name.to_lowercase()),
        Expr::String(s) | Expr::UnicodeString(s) => Ok(s.to_lowercase()),
        _ => Err(DbError::Execution(
            "datepart must be an identifier or string".into(),
        )),
    }
}

pub(crate) fn eval_dateadd(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 3 {
        return Err(DbError::Execution("DATEADD expects 3 arguments".into()));
    }

    let part = extract_datepart(&args[0])?;
    let number = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    let date_val = eval_expr(&args[2], row, ctx, catalog, storage, clock)?;
    if number.is_null() || date_val.is_null() {
        return Ok(Value::Null);
    }
    let num = match number {
        Value::Int(v) => v as i64,
        Value::BigInt(v) => v,
        Value::SmallInt(v) => v as i64,
        Value::TinyInt(v) => v as i64,
        Value::Float(bits) => f64::from_bits(bits).trunc() as i64,
        Value::Decimal(raw, scale) => (raw / 10i128.pow(scale as u32)) as i64,
        Value::Bit(b) => i64::from(b),
        _ => {
            return Err(DbError::Execution(
                "DATEADD number must be an integer".into(),
            ))
        }
    };
    let date_str = date_val.to_string_value();

    let result = apply_dateadd(&part, num, &date_str, &ctx.options.dateformat)?;

    match date_val {
        Value::Date(_) => Ok(Value::Date(result.date())),
        Value::DateTime(_) => Ok(Value::DateTime(result)),
        Value::DateTime2(_) => Ok(Value::DateTime2(result)),
        _ => Ok(Value::DateTime(result)),
    }
}

pub(crate) fn eval_datediff(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 3 {
        return Err(DbError::Execution("DATEDIFF expects 3 arguments".into()));
    }

    let part = extract_datepart(&args[0])?;
    let start_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    let end_val = eval_expr(&args[2], row, ctx, catalog, storage, clock)?;
    if start_val.is_null() || end_val.is_null() {
        return Ok(Value::Null);
    }
    let start_str = start_val.to_string_value();
    let end_str = end_val.to_string_value();

    let (sy, sm, sd, sh, smi, ss) = parse_datetime_parts(&start_str, &ctx.options.dateformat)?;
    let (ey, em, ed, ehi, emi, es) = parse_datetime_parts(&end_str, &ctx.options.dateformat)?;

    let result = match part.as_str() {
        "year" | "yy" | "yyyy" => ey - sy,
        "quarter" | "qq" | "q" => (ey - sy) * 4 + ((em - 1) / 3 - (sm - 1) / 3),
        "month" | "mm" | "m" => (ey - sy) * 12 + (em - sm),
        "week" | "wk" | "ww" => {
            let start_days = date_to_days(sy, sm, sd);
            let end_days = date_to_days(ey, em, ed);
            let start_sunday = start_days - day_of_week_from_date(sy, sm, sd) as i64;
            let end_sunday = end_days - day_of_week_from_date(ey, em, ed) as i64;
            ((end_sunday - start_sunday) / 7) as i32
        }
        "day" | "dd" | "d" | "dayofyear" | "dy" | "y" | "weekday" | "dw" | "w" => {
            (date_to_days(ey, em, ed) - date_to_days(sy, sm, sd)) as i32
        }
        "hour" | "hh" => {
            let day_diff = date_to_days(ey, em, ed) - date_to_days(sy, sm, sd);
            (day_diff * 24 + (ehi - sh) as i64) as i32
        }
        "minute" | "mi" | "n" => {
            let day_diff = date_to_days(ey, em, ed) - date_to_days(sy, sm, sd);
            (day_diff * 1440 + (ehi - sh) as i64 * 60 + (emi - smi) as i64) as i32
        }
        "second" | "ss" | "s" => {
            let day_diff = date_to_days(ey, em, ed) - date_to_days(sy, sm, sd);
            (day_diff * 86400
                + (ehi - sh) as i64 * 3600
                + (emi - smi) as i64 * 60
                + (es - ss) as i64) as i32
        }
        "millisecond" | "ms" => {
            let day_diff = date_to_days(ey, em, ed) - date_to_days(sy, sm, sd);
            ((day_diff * 86400
                + (ehi - sh) as i64 * 3600
                + (emi - smi) as i64 * 60
                + (es - ss) as i64)
                * 1000) as i32
        }
        _ => return Err(DbError::Execution(format!("unknown datepart '{}'", part))),
    };

    Ok(Value::Int(result))
}

pub(crate) fn eval_datepart(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution("DATEPART expects 2 arguments".into()));
    }

    let part = extract_datepart(&args[0])?;
    let date_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if date_val.is_null() {
        return Ok(Value::Null);
    }
    let date_str = date_val.to_string_value();
    let (y, m, d, h, mi, s) = parse_datetime_parts(&date_str, &ctx.options.dateformat)?;

    let result = match part.as_str() {
        "year" | "yy" | "yyyy" => y,
        "month" | "mm" | "m" => m,
        "day" | "dd" | "d" => d,
        "hour" | "hh" => h,
        "minute" | "mi" | "n" => mi,
        "second" | "ss" | "s" => s,
        "weekday" | "dw" | "w" => {
            let dow = day_of_week_from_date(y, m, d);
            let datefirst = ctx.metadata.datefirst;
            (dow - datefirst + 7) % 7 + 1
        }
        "dayofweek" => {
            let dow = day_of_week_from_date(y, m, d);
            dow + 1
        }
        "dayofyear" | "dy" | "y" => {
            let days = date_to_days(y, m, d);
            let jan1 = date_to_days(y, 1, 1);
            ((days - jan1) + 1) as i32
        }
        "week" | "wk" | "ww" => {
            let days = date_to_days(y, m, d);
            let jan1 = date_to_days(y, 1, 1);
            let jan1_dow = day_of_week_from_date(y, 1, 1) as i64;
            (((days - jan1) + jan1_dow) / 7 + 1) as i32
        }
        "quarter" | "qq" | "q" => (m - 1) / 3 + 1,
        "millisecond" | "ms" => 0i32,
        "microsecond" | "mcs" => 0i32,
        "nanosecond" | "ns" => 0i32,
        "tzoffset" | "tz" => extract_tzoffset_minutes(&date_str),
        _ => return Err(DbError::Execution(format!("unknown datepart '{}'", part))),
    };

    Ok(Value::Int(result))
}

fn extract_tzoffset_minutes(raw: &str) -> i32 {
    let trimmed = raw.trim();
    if trimmed.ends_with('Z') || trimmed.ends_with('z') {
        return 0;
    }
    if trimmed.len() >= 6 {
        let tail = &trimmed[trimmed.len() - 6..];
        let bytes = tail.as_bytes();
        if (bytes[0] == b'+' || bytes[0] == b'-') && bytes[3] == b':' {
            if let (Ok(h), Ok(m)) = (tail[1..3].parse::<i32>(), tail[4..6].parse::<i32>()) {
                let total = h * 60 + m;
                return if bytes[0] == b'-' { -total } else { total };
            }
        }
    }
    0
}

pub(crate) fn eval_datename(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution("DATENAME expects 2 arguments".into()));
    }

    let part = extract_datepart(&args[0])?;
    let date_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if date_val.is_null() {
        return Ok(Value::Null);
    }
    let date_str = date_val.to_string_value();
    let (y, m, d, _, _, _) = parse_datetime_parts(&date_str, &ctx.options.dateformat)?;

    let result = match part.as_str() {
        "year" | "yy" | "yyyy" => format!("{}", y),
        "month" | "mm" | "m" => {
            let months = [
                "",
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ];
            months.get(m as usize).unwrap_or(&"").to_string()
        }
        "day" | "dd" | "d" => format!("{}", d),
        "dayofweek" | "dw" | "weekday" | "w" => {
            let dow = day_of_week_from_date(y, m, d);
            let datefirst = ctx.metadata.datefirst;
            let adjusted = ((dow - datefirst + 7) % 7) as usize;
            let day_names = [
                "Sunday",
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
            ];
            day_names[adjusted].to_string()
        }
        "dayofyear" | "dy" => {
            let days = date_to_days(y, m, d);
            let jan1 = date_to_days(y, 1, 1);
            format!("{}", (days - jan1) + 1)
        }
        "quarter" | "qq" | "q" => format!("{}", ((m - 1) / 3 + 1)),
        "hour" | "hh" => {
            let time_part = date_str.split('T').nth(1).unwrap_or("00:00:00");
            let hour: i32 = time_part
                .split(':')
                .next()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
            format!("{}", hour)
        }
        "minute" | "mi" | "n" => {
            let time_part = date_str.split('T').nth(1).unwrap_or("00:00:00");
            let parts: Vec<&str> = time_part.split(':').collect();
            let minute: i32 = parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
            format!("{}", minute)
        }
        "second" | "ss" | "s" => {
            let time_part = date_str.split('T').nth(1).unwrap_or("00:00:00");
            let parts: Vec<&str> = time_part.split(':').collect();
            let second: i32 = parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
            format!("{}", second)
        }
        "millisecond" | "ms" => "0".to_string(),
        _ => return Err(DbError::Execution(format!("unknown datepart '{}'", part))),
    };

    Ok(Value::VarChar(result))
}

pub(crate) fn eval_year(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution("YEAR expects 1 argument".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let date_str = val.to_string_value();
    match parse_datetime_parts(&date_str, &ctx.options.dateformat) {
        Ok((y, _, _, _, _, _)) => Ok(Value::Int(y)),
        Err(_) => Ok(Value::Null),
    }
}

pub(crate) fn eval_month(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution("MONTH expects 1 argument".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let date_str = val.to_string_value();
    match parse_datetime_parts(&date_str, &ctx.options.dateformat) {
        Ok((_, m, _, _, _, _)) => Ok(Value::Int(m)),
        Err(_) => Ok(Value::Null),
    }
}

pub(crate) fn eval_day(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution("DAY expects 1 argument".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let date_str = val.to_string_value();
    match parse_datetime_parts(&date_str, &ctx.options.dateformat) {
        Ok((_, _, d, _, _, _)) => Ok(Value::Int(d)),
        Err(_) => Ok(Value::Null),
    }
}

pub(crate) fn eval_getdate(
    _args: &[Expr],
    _row: &[ContextTable],
    _ctx: &mut ExecutionContext,
    _catalog: &dyn Catalog,
    _storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    Ok(Value::DateTime(clock.now_datetime_literal()))
}

pub(crate) fn eval_current_timestamp(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    eval_getdate(args, row, ctx, catalog, storage, clock)
}

pub(crate) fn eval_current_date(
    _args: &[Expr],
    _row: &[ContextTable],
    _ctx: &mut ExecutionContext,
    _catalog: &dyn Catalog,
    _storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    Ok(Value::Date(clock.now_datetime_literal().date()))
}

pub(crate) fn eval_current_time(
    _args: &[Expr],
    _row: &[ContextTable],
    _ctx: &mut ExecutionContext,
    _catalog: &dyn Catalog,
    _storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    Ok(Value::Time(clock.now_datetime_literal().time()))
}

pub(crate) fn eval_getutcdate(
    _args: &[Expr],
    _row: &[ContextTable],
    _ctx: &mut ExecutionContext,
    _catalog: &dyn Catalog,
    _storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    Ok(Value::DateTime(clock.now_datetime_literal())) // Simplified
}

pub(crate) fn eval_sysdatetime(
    _args: &[Expr],
    _row: &[ContextTable],
    _ctx: &mut ExecutionContext,
    _catalog: &dyn Catalog,
    _storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    Ok(Value::DateTime2(clock.now_datetime_literal()))
}

pub(crate) fn eval_sysutcdatetime(
    _args: &[Expr],
    _row: &[ContextTable],
    _ctx: &mut ExecutionContext,
    _catalog: &dyn Catalog,
    _storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    Ok(Value::DateTime2(clock.now_datetime_literal())) // Simplified
}

pub(crate) fn eval_sysdatetimeoffset(
    _args: &[Expr],
    _row: &[ContextTable],
    _ctx: &mut ExecutionContext,
    _catalog: &dyn Catalog,
    _storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    // Value doesn't have DateTimeOffset, returning DateTime2
    Ok(Value::DateTime2(clock.now_datetime_literal()))
}

pub(crate) fn eval_eomonth(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.is_empty() || args.len() > 2 {
        return Err(DbError::Execution(
            "EOMONTH expects 1 or 2 arguments".into(),
        ));
    }
    let date_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if date_val.is_null() {
        return Ok(Value::Null);
    }
    let date_str = date_val.to_string_value();
    let (mut y, mut m, _, _, _, _) = parse_datetime_parts(&date_str, &ctx.options.dateformat)?;

    if args.len() == 2 {
        let add = eval_expr(&args[1], row, ctx, catalog, storage, clock)?
            .to_integer_i64()
            .unwrap_or(0);
        m += add as i32;
        while m > 12 {
            y += 1;
            m -= 12;
        }
        while m <= 0 {
            y -= 1;
            m += 12;
        }
    }

    let is_leap = (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap {
                29
            } else {
                28
            }
        }
        _ => 30,
    };
    let last_day = format!("{:04}-{:02}-{:02}T00:00:00", y, m, days);
    let dt = chrono::NaiveDateTime::parse_from_str(&last_day, "%Y-%m-%dT%H:%M:%S")
        .map_err(|e| DbError::Execution(format!("Invalid date: {}", e)))?;
    Ok(Value::Date(dt.date()))
}

pub(crate) fn eval_isdate(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.is_empty() {
        return Ok(Value::Int(0));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Int(0));
    }
    let s = val.to_string_value();
    if s.trim().is_empty() {
        return Ok(Value::Int(0));
    }
    let is_valid = parse_datetime_parts(&s, &ctx.options.dateformat).is_ok();
    Ok(Value::Int(if is_valid { 1 } else { 0 }))
}

pub(crate) fn eval_datediff_big(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    match eval_datediff(args, row, ctx, catalog, storage, clock)? {
        Value::Int(v) => Ok(Value::BigInt(v as i64)),
        other => Ok(other),
    }
}

pub(crate) fn eval_datefromparts(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 3 {
        return Err(DbError::Execution(
            "DATEFROMPARTS expects 3 arguments".into(),
        ));
    }
    let yv = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let mv = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    let dv = eval_expr(&args[2], row, ctx, catalog, storage, clock)?;
    if yv.is_null() || mv.is_null() || dv.is_null() {
        return Ok(Value::Null);
    }
    let y = yv.to_integer_i64().unwrap_or(0) as i32;
    let m = mv.to_integer_i64().unwrap_or(0) as u32;
    let d = dv.to_integer_i64().unwrap_or(0) as u32;
    let date = chrono::NaiveDate::from_ymd_opt(y, m, d).ok_or_else(|| {
        DbError::Execution(format!(
            "Cannot construct data type date, some of the arguments have values which are not valid ({y}-{m}-{d})."
        ))
    })?;
    Ok(Value::Date(date))
}

pub(crate) fn eval_datetimefromparts(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 7 {
        return Err(DbError::Execution(
            "DATETIMEFROMPARTS expects 7 arguments".into(),
        ));
    }
    let mut vals = Vec::with_capacity(7);
    for a in args {
        let v = eval_expr(a, row, ctx, catalog, storage, clock)?;
        if v.is_null() {
            return Ok(Value::Null);
        }
        vals.push(v.to_integer_i64().unwrap_or(0));
    }
    let date = chrono::NaiveDate::from_ymd_opt(vals[0] as i32, vals[1] as u32, vals[2] as u32)
        .ok_or_else(|| DbError::Execution("Invalid date arguments to DATETIMEFROMPARTS".into()))?;
    let dt = date
        .and_hms_milli_opt(
            vals[3] as u32,
            vals[4] as u32,
            vals[5] as u32,
            vals[6] as u32,
        )
        .ok_or_else(|| DbError::Execution("Invalid time arguments to DATETIMEFROMPARTS".into()))?;
    Ok(Value::DateTime(dt))
}

pub(crate) fn eval_datetime2fromparts(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 8 {
        return Err(DbError::Execution(
            "DATETIME2FROMPARTS expects 8 arguments".into(),
        ));
    }
    let mut vals = Vec::with_capacity(8);
    for a in args {
        let v = eval_expr(a, row, ctx, catalog, storage, clock)?;
        if v.is_null() {
            return Ok(Value::Null);
        }
        vals.push(v.to_integer_i64().unwrap_or(0));
    }
    let date = chrono::NaiveDate::from_ymd_opt(vals[0] as i32, vals[1] as u32, vals[2] as u32)
        .ok_or_else(|| DbError::Execution("Invalid date arguments to DATETIME2FROMPARTS".into()))?;
    let precision = vals[7].clamp(0, 7) as u32;
    let nanos = if precision == 0 {
        0u32
    } else {
        let factor = 10u64.pow(9 - precision);
        ((vals[6] as u64) * factor).min(999_999_999) as u32
    };
    let dt = date
        .and_hms_nano_opt(vals[3] as u32, vals[4] as u32, vals[5] as u32, nanos)
        .ok_or_else(|| DbError::Execution("Invalid time arguments to DATETIME2FROMPARTS".into()))?;
    Ok(Value::DateTime2(dt))
}

fn value_to_naive_datetime(
    val: &Value,
    dateformat: &str,
) -> Result<chrono::NaiveDateTime, DbError> {
    match val {
        Value::Date(d) => Ok(d.and_hms_opt(0, 0, 0).unwrap()),
        Value::DateTime(dt) | Value::DateTime2(dt) | Value::SmallDateTime(dt) => Ok(*dt),
        Value::Time(t) => Ok(chrono::NaiveDate::from_ymd_opt(1900, 1, 1)
            .unwrap()
            .and_time(*t)),
        _ => {
            let s = val.to_string_value();
            let (y, m, d, hh, mm, ss) = parse_datetime_parts(&s, dateformat)?;
            let date = chrono::NaiveDate::from_ymd_opt(y, m as u32, d as u32)
                .ok_or_else(|| DbError::Execution(format!("invalid date '{}'", s)))?;
            date.and_hms_opt(hh as u32, mm as u32, ss as u32)
                .ok_or_else(|| DbError::Execution(format!("invalid datetime '{}'", s)))
        }
    }
}

fn wrap_matching_date_type(orig: &Value, dt: chrono::NaiveDateTime) -> Value {
    match orig {
        Value::Date(_) => Value::Date(dt.date()),
        Value::DateTime(_) => Value::DateTime(dt),
        Value::DateTime2(_) => Value::DateTime2(dt),
        Value::SmallDateTime(_) => Value::SmallDateTime(dt),
        Value::Time(_) => Value::Time(dt.time()),
        _ => Value::DateTime2(dt),
    }
}

pub(crate) fn eval_datetrunc(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    use chrono::{Datelike, Duration, NaiveDate, Timelike};
    if args.len() != 2 {
        return Err(DbError::Execution("DATETRUNC expects 2 arguments".into()));
    }
    let part = extract_datepart(&args[0])?;
    let date_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if date_val.is_null() {
        return Ok(Value::Null);
    }
    let dt = value_to_naive_datetime(&date_val, &ctx.options.dateformat)?;
    let d = dt.date();
    let t = dt.time();

    let truncated = match part.as_str() {
        "year" | "yy" | "yyyy" => NaiveDate::from_ymd_opt(d.year(), 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap(),
        "quarter" | "qq" | "q" => {
            let q_month = ((d.month() - 1) / 3) * 3 + 1;
            NaiveDate::from_ymd_opt(d.year(), q_month, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
        }
        "month" | "mm" | "m" => NaiveDate::from_ymd_opt(d.year(), d.month(), 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap(),
        "dayofyear" | "dy" | "y" | "day" | "dd" | "d" => d.and_hms_opt(0, 0, 0).unwrap(),
        "week" | "wk" | "ww" => {
            // SQL Server DATEFIRST: 1 = Monday .. 7 = Sunday (default 7).
            let dow_mon1 = d.weekday().number_from_monday() as i64; // 1..=7
            let datefirst = ctx.options.datefirst.clamp(1, 7) as i64;
            let offset = (dow_mon1 - datefirst).rem_euclid(7);
            (d - Duration::days(offset)).and_hms_opt(0, 0, 0).unwrap()
        }
        "iso_week" | "isowk" | "isoww" => {
            let offset = (d.weekday().number_from_monday() - 1) as i64;
            (d - Duration::days(offset)).and_hms_opt(0, 0, 0).unwrap()
        }
        "hour" | "hh" => d.and_hms_opt(t.hour(), 0, 0).unwrap(),
        "minute" | "mi" | "n" => d.and_hms_opt(t.hour(), t.minute(), 0).unwrap(),
        "second" | "ss" | "s" => d.and_hms_opt(t.hour(), t.minute(), t.second()).unwrap(),
        "millisecond" | "ms" => {
            let ms_nanos = (t.nanosecond() / 1_000_000) * 1_000_000;
            d.and_hms_nano_opt(t.hour(), t.minute(), t.second(), ms_nanos)
                .unwrap()
        }
        "microsecond" | "mcs" => {
            let us_nanos = (t.nanosecond() / 1_000) * 1_000;
            d.and_hms_nano_opt(t.hour(), t.minute(), t.second(), us_nanos)
                .unwrap()
        }
        _ => {
            return Err(DbError::Execution(format!(
                "Invalid datepart '{}' for DATETRUNC",
                part
            )));
        }
    };

    Ok(wrap_matching_date_type(&date_val, truncated))
}

pub(crate) fn eval_date_bucket(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    use chrono::{Datelike, Duration, NaiveDate};
    if args.len() < 3 || args.len() > 4 {
        return Err(DbError::Execution(
            "DATE_BUCKET expects 3 or 4 arguments".into(),
        ));
    }
    let part = extract_datepart(&args[0])?;
    let num_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    let date_val = eval_expr(&args[2], row, ctx, catalog, storage, clock)?;
    let origin_val = if args.len() == 4 {
        Some(eval_expr(&args[3], row, ctx, catalog, storage, clock)?)
    } else {
        None
    };

    if num_val.is_null() || date_val.is_null() || origin_val.as_ref().is_some_and(|v| v.is_null()) {
        return Ok(Value::Null);
    }

    let width = num_val
        .to_integer_i64()
        .ok_or_else(|| DbError::Execution("DATE_BUCKET number must be an integer".into()))?;
    if width <= 0 {
        return Err(DbError::Execution(
            "Invalid number parameter value for DATE_BUCKET: must be greater than zero.".into(),
        ));
    }

    let dt = value_to_naive_datetime(&date_val, &ctx.options.dateformat)?;
    let origin = match &origin_val {
        Some(v) => value_to_naive_datetime(v, &ctx.options.dateformat)?,
        None => NaiveDate::from_ymd_opt(1900, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap(),
    };

    let add_months_clamped =
        |base: chrono::NaiveDateTime, months: i64| -> Result<chrono::NaiveDateTime, DbError> {
            let total_months = (base.year() as i64) * 12 + (base.month() as i64 - 1) + months;
            let new_year = total_months.div_euclid(12) as i32;
            let new_month = (total_months.rem_euclid(12) + 1) as u32;
            let mut day = base.day();
            let mut target_date = NaiveDate::from_ymd_opt(new_year, new_month, day);
            while target_date.is_none() && day > 28 {
                day -= 1;
                target_date = NaiveDate::from_ymd_opt(new_year, new_month, day);
            }
            let d = target_date
                .ok_or_else(|| DbError::Execution("DATE_BUCKET date overflow".into()))?;
            Ok(d.and_time(base.time()))
        };

    let bucketed = match part.as_str() {
        "year" | "yy" | "yyyy" => {
            let mut diff = (dt.year() - origin.year()) as i64;
            let cand = add_months_clamped(origin, diff * 12)?;
            if dt < cand {
                diff -= 1;
            }
            let k = diff.div_euclid(width);
            add_months_clamped(origin, k * width * 12)?
        }
        "quarter" | "qq" | "q" => {
            let mut diff_m = (dt.year() - origin.year()) as i64 * 12
                + (dt.month() as i64 - origin.month() as i64);
            let cand = add_months_clamped(origin, diff_m)?;
            if dt < cand {
                diff_m -= 1;
            }
            let diff_q = diff_m.div_euclid(3);
            let k = diff_q.div_euclid(width);
            add_months_clamped(origin, k * width * 3)?
        }
        "month" | "mm" | "m" => {
            let mut diff_m = (dt.year() - origin.year()) as i64 * 12
                + (dt.month() as i64 - origin.month() as i64);
            let cand = add_months_clamped(origin, diff_m)?;
            if dt < cand {
                diff_m -= 1;
            }
            let k = diff_m.div_euclid(width);
            add_months_clamped(origin, k * width)?
        }
        "week" | "wk" | "ww" => {
            let step_us = width * 7 * 86_400_000_000i64;
            let delta = (dt - origin).num_microseconds().unwrap_or(0);
            let k = delta.div_euclid(step_us);
            origin + Duration::microseconds(k * step_us)
        }
        "day" | "dd" | "d" | "dayofyear" | "dy" | "y" => {
            let step_us = width * 86_400_000_000i64;
            let delta = (dt - origin).num_microseconds().unwrap_or(0);
            let k = delta.div_euclid(step_us);
            origin + Duration::microseconds(k * step_us)
        }
        "hour" | "hh" => {
            let step_us = width * 3_600_000_000i64;
            let delta = (dt - origin).num_microseconds().unwrap_or(0);
            let k = delta.div_euclid(step_us);
            origin + Duration::microseconds(k * step_us)
        }
        "minute" | "mi" | "n" => {
            let step_us = width * 60_000_000i64;
            let delta = (dt - origin).num_microseconds().unwrap_or(0);
            let k = delta.div_euclid(step_us);
            origin + Duration::microseconds(k * step_us)
        }
        "second" | "ss" | "s" => {
            let step_us = width * 1_000_000i64;
            let delta = (dt - origin).num_microseconds().unwrap_or(0);
            let k = delta.div_euclid(step_us);
            origin + Duration::microseconds(k * step_us)
        }
        "millisecond" | "ms" => {
            let step_us = width * 1_000i64;
            let delta = (dt - origin).num_microseconds().unwrap_or(0);
            let k = delta.div_euclid(step_us);
            origin + Duration::microseconds(k * step_us)
        }
        "microsecond" | "mcs" => {
            let delta = (dt - origin).num_microseconds().unwrap_or(0);
            let k = delta.div_euclid(width);
            origin + Duration::microseconds(k * width)
        }
        _ => {
            return Err(DbError::Execution(format!(
                "Invalid datepart '{}' for DATE_BUCKET",
                part
            )));
        }
    };

    Ok(wrap_matching_date_type(&date_val, bucketed))
}

fn scale_fraction_to_nanos(fractions: i64, precision: i64) -> Result<u32, DbError> {
    if !(0..=7).contains(&precision) {
        return Err(DbError::Execution(format!(
            "Invalid precision '{}' (must be 0..=7)",
            precision
        )));
    }
    if fractions < 0 {
        return Err(DbError::Execution(
            "Invalid negative fractions value".into(),
        ));
    }
    if precision == 0 {
        if fractions != 0 {
            return Err(DbError::Execution(
                "Fractions must be 0 when precision is 0".into(),
            ));
        }
        return Ok(0);
    }
    let max_val = 10_i64.pow(precision as u32);
    if fractions >= max_val {
        return Err(DbError::Execution(format!(
            "Fractions '{}' out of range for precision {}",
            fractions, precision
        )));
    }
    let scale = 10_i64.pow((9 - precision) as u32);
    Ok((fractions * scale) as u32)
}

pub(crate) fn eval_timefromparts(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 5 {
        return Err(DbError::Execution(
            "TIMEFROMPARTS expects 5 arguments".into(),
        ));
    }
    let mut vals = Vec::with_capacity(5);
    for a in args {
        let v = eval_expr(a, row, ctx, catalog, storage, clock)?;
        if v.is_null() {
            return Ok(Value::Null);
        }
        vals.push(v.to_integer_i64().ok_or_else(|| {
            DbError::Execution("TIMEFROMPARTS arguments must be integers".into())
        })?);
    }
    let (h, m, s, frac, prec) = (vals[0], vals[1], vals[2], vals[3], vals[4]);
    let nanos = scale_fraction_to_nanos(frac, prec)?;
    let time = chrono::NaiveTime::from_hms_nano_opt(h as u32, m as u32, s as u32, nanos)
        .ok_or_else(|| DbError::Execution("Cannot construct time from invalid parts".into()))?;
    Ok(Value::Time(time))
}

pub(crate) fn eval_smalldatetimefromparts(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 5 {
        return Err(DbError::Execution(
            "SMALLDATETIMEFROMPARTS expects 5 arguments".into(),
        ));
    }
    let mut vals = Vec::with_capacity(5);
    for a in args {
        let v = eval_expr(a, row, ctx, catalog, storage, clock)?;
        if v.is_null() {
            return Ok(Value::Null);
        }
        vals.push(v.to_integer_i64().ok_or_else(|| {
            DbError::Execution("SMALLDATETIMEFROMPARTS arguments must be integers".into())
        })?);
    }
    let (y, mo, d, h, mi) = (vals[0], vals[1], vals[2], vals[3], vals[4]);
    if !(1900..=2079).contains(&y) {
        return Err(DbError::Execution(
            "SMALLDATETIMEFROMPARTS year out of range (1900..=2079)".into(),
        ));
    }
    let date = NaiveDate::from_ymd_opt(y as i32, mo as u32, d as u32).ok_or_else(|| {
        DbError::Execution("Cannot construct smalldatetime from invalid date parts".into())
    })?;
    let dt = date.and_hms_opt(h as u32, mi as u32, 0).ok_or_else(|| {
        DbError::Execution("Cannot construct smalldatetime from invalid time parts".into())
    })?;
    Ok(Value::SmallDateTime(dt))
}

pub(crate) fn eval_datetimeoffsetfromparts(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 10 {
        return Err(DbError::Execution(
            "DATETIMEOFFSETFROMPARTS expects 10 arguments".into(),
        ));
    }
    let mut vals = Vec::with_capacity(10);
    for a in args {
        let v = eval_expr(a, row, ctx, catalog, storage, clock)?;
        if v.is_null() {
            return Ok(Value::Null);
        }
        vals.push(v.to_integer_i64().ok_or_else(|| {
            DbError::Execution("DATETIMEOFFSETFROMPARTS arguments must be integers".into())
        })?);
    }
    let (y, mo, d, h, mi, s, frac, off_h, off_m, prec) = (
        vals[0], vals[1], vals[2], vals[3], vals[4], vals[5], vals[6], vals[7], vals[8], vals[9],
    );
    if !(-14..=14).contains(&off_h) || !(-59..=59).contains(&off_m) {
        return Err(DbError::Execution(
            "DATETIMEOFFSETFROMPARTS offset out of range".into(),
        ));
    }
    if (off_h > 0 && off_m < 0) || (off_h < 0 && off_m > 0) {
        return Err(DbError::Execution(
            "DATETIMEOFFSETFROMPARTS hour_offset and minute_offset must have the same sign".into(),
        ));
    }
    let total_off_min = off_h * 60 + off_m;
    if !(-840..=840).contains(&total_off_min) {
        return Err(DbError::Execution(
            "DATETIMEOFFSETFROMPARTS offset must be between -14:00 and +14:00".into(),
        ));
    }
    let nanos = scale_fraction_to_nanos(frac, prec)?;
    let date = NaiveDate::from_ymd_opt(y as i32, mo as u32, d as u32).ok_or_else(|| {
        DbError::Execution("Cannot construct datetimeoffset from invalid date parts".into())
    })?;
    let time = chrono::NaiveTime::from_hms_nano_opt(h as u32, mi as u32, s as u32, nanos)
        .ok_or_else(|| {
            DbError::Execution("Cannot construct datetimeoffset from invalid time parts".into())
        })?;
    let sign = if total_off_min < 0 { '-' } else { '+' };
    let abs_h = (total_off_min.abs()) / 60;
    let abs_m = (total_off_min.abs()) % 60;
    let base = format!(
        "{} {:02}:{:02}:{:02}",
        date.format("%Y-%m-%d"),
        time.hour(),
        time.minute(),
        time.second()
    );
    let formatted = if prec > 0 {
        format!(
            "{}.{:0width$} {}{:02}:{:02}",
            base,
            frac,
            sign,
            abs_h,
            abs_m,
            width = prec as usize
        )
    } else {
        format!("{} {}{:02}:{:02}", base, sign, abs_h, abs_m)
    };
    Ok(Value::DateTimeOffset(formatted))
}

pub(crate) fn eval_current_timezone(
    args: &[Expr],
    _row: &[ContextTable],
    _ctx: &mut ExecutionContext,
    _catalog: &dyn Catalog,
    _storage: &dyn Storage,
    _clock: &dyn Clock,
) -> Result<Value, DbError> {
    if !args.is_empty() {
        return Err(DbError::Execution(
            "CURRENT_TIMEZONE expects 0 arguments".into(),
        ));
    }
    Ok(Value::NVarChar(
        "(UTC+00:00) Coordinated Universal Time".into(),
    ))
}

pub(crate) fn eval_current_timezone_id(
    args: &[Expr],
    _row: &[ContextTable],
    _ctx: &mut ExecutionContext,
    _catalog: &dyn Catalog,
    _storage: &dyn Storage,
    _clock: &dyn Clock,
) -> Result<Value, DbError> {
    if !args.is_empty() {
        return Err(DbError::Execution(
            "CURRENT_TIMEZONE_ID expects 0 arguments".into(),
        ));
    }
    Ok(Value::NVarChar("UTC".into()))
}
