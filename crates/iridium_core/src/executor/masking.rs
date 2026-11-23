use std::borrow::Cow;

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

use crate::catalog::Catalog;
use crate::executor::context::ExecutionContext;
use crate::executor::model::JoinedRow;
use crate::executor::random::{RandomProvider, ThreadRng};
use crate::executor::security::user_has_unmask;
use crate::types::{DataType, Value};

fn random_i128_in_range(lo: i128, hi: i128) -> i128 {
    if lo >= hi {
        return lo;
    }
    let rng = ThreadRng;
    let span = (hi - lo) as u128;
    let r = ((rng.next_u64() as u128) << 64) | (rng.next_u64() as u128);
    if span == u128::MAX {
        lo.wrapping_add(r as i128)
    } else {
        lo + (r % (span + 1)) as i128
    }
}

/// Applies a SQL Server Dynamic Data Masking function (`default()`, `email()`,
/// `partial(prefix, "padding", suffix)`, `random(start, end)`, `datetime("part")`)
/// to a single column value. `NULL` values always remain `NULL`.
pub(crate) fn apply_mask_function(val: &Value, data_type: &DataType, func: &str) -> Value {
    if val.is_null() {
        return Value::Null;
    }
    let trimmed = func.trim();
    let lower = trimmed.to_ascii_lowercase();

    if lower.starts_with("default") {
        return mask_default(val, data_type);
    }
    if lower.starts_with("email") {
        return mask_email(val);
    }
    if let Some(args) = extract_paren_args(trimmed, "partial") {
        if let Some(masked) = mask_partial(val, args) {
            return masked;
        }
    }
    if let Some(args) = extract_paren_args(trimmed, "random") {
        if let Some(masked) = mask_random(val, data_type, args) {
            return masked;
        }
    }
    if let Some(args) = extract_paren_args(trimmed, "datetime") {
        if let Some(masked) = mask_datetime(val, args) {
            return masked;
        }
    }
    mask_default(val, data_type)
}

fn extract_paren_args<'a>(func: &'a str, name: &str) -> Option<&'a str> {
    let lp = func.find('(')?;
    let rp = func.rfind(')')?;
    if rp <= lp {
        return None;
    }
    if func[..lp].trim().eq_ignore_ascii_case(name) {
        Some(func[lp + 1..rp].trim())
    } else {
        None
    }
}

fn mask_default(val: &Value, data_type: &DataType) -> Value {
    let default_dt = NaiveDate::from_ymd_opt(1900, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    match val {
        Value::Null => Value::Null,
        Value::Char(_) | Value::VarChar(_) | Value::NChar(_) | Value::NVarChar(_) => {
            let max_len = match data_type {
                DataType::Char { len } | DataType::NChar { len } => *len as usize,
                DataType::VarChar { max_len } | DataType::NVarChar { max_len } => *max_len as usize,
                _ => 0,
            };
            let masked = if max_len > 0 && max_len < 4 {
                "x".repeat(max_len)
            } else {
                "xxxx".to_string()
            };
            match val {
                Value::Char(_) => Value::Char(masked),
                Value::NChar(_) => Value::NChar(masked),
                Value::NVarChar(_) => Value::NVarChar(masked),
                _ => Value::VarChar(masked),
            }
        }
        Value::Bit(_) => Value::Bit(false),
        Value::TinyInt(_) => Value::TinyInt(0),
        Value::SmallInt(_) => Value::SmallInt(0),
        Value::Int(_) => Value::Int(0),
        Value::BigInt(_) => Value::BigInt(0),
        Value::Float(_) => Value::Float(0.0f64.to_bits()),
        Value::Decimal(_, scale) => Value::Decimal(0, *scale),
        Value::Money(_) => Value::Money(0),
        Value::SmallMoney(_) => Value::SmallMoney(0),
        Value::Date(_) => Value::Date(NaiveDate::from_ymd_opt(1900, 1, 1).unwrap()),
        Value::DateTime(_) => Value::DateTime(default_dt),
        Value::DateTime2(_) => Value::DateTime2(default_dt),
        Value::SmallDateTime(_) => Value::SmallDateTime(default_dt),
        Value::DateTimeOffset(_) => Value::DateTimeOffset("1900-01-01 00:00:00 +00:00".to_string()),
        Value::Time(_) => Value::Time(NaiveTime::from_hms_opt(0, 0, 0).unwrap()),
        Value::UniqueIdentifier(_) => Value::UniqueIdentifier(uuid::Uuid::nil()),
        Value::Binary(_) => Value::Binary(vec![0u8]),
        Value::VarBinary(_) => Value::VarBinary(vec![0u8]),
        Value::Vector(bits) => Value::Vector(vec![0.0f32.to_bits(); bits.len()]),
        Value::SqlVariant(inner) => Value::SqlVariant(Box::new(mask_default(inner, data_type))),
    }
}

fn mask_email(val: &Value) -> Value {
    let s = val.to_string_value();
    let masked = match s.chars().next() {
        Some(first) => format!("{first}XXX@XXXX.com"),
        None => "XXX@XXXX.com".to_string(),
    };
    match val {
        Value::NVarChar(_) => Value::NVarChar(masked),
        _ => Value::VarChar(masked),
    }
}

fn parse_partial_args(args: &str) -> Option<(usize, String, usize)> {
    // partial(prefix, "padding", suffix)
    let first_comma = args.find(',')?;
    let last_comma = args.rfind(',')?;
    if first_comma >= last_comma {
        return None;
    }
    let prefix: usize = args[..first_comma].trim().parse().ok()?;
    let suffix: usize = args[last_comma + 1..].trim().parse().ok()?;
    let pad_raw = args[first_comma + 1..last_comma].trim();
    let pad = if (pad_raw.starts_with('"') && pad_raw.ends_with('"') && pad_raw.len() >= 2)
        || (pad_raw.starts_with('\'') && pad_raw.ends_with('\'') && pad_raw.len() >= 2)
    {
        pad_raw[1..pad_raw.len() - 1].to_string()
    } else {
        pad_raw.to_string()
    };
    Some((prefix, pad, suffix))
}

fn mask_partial(val: &Value, args: &str) -> Option<Value> {
    let (prefix, pad, suffix) = parse_partial_args(args)?;
    let s = val.to_string_value();
    let chars: Vec<char> = s.chars().collect();
    let masked = if chars.len() <= prefix.saturating_add(suffix) {
        pad
    } else {
        let mut out = String::new();
        for c in chars.iter().take(prefix) {
            out.push(*c);
        }
        out.push_str(&pad);
        for c in chars.iter().skip(chars.len() - suffix) {
            out.push(*c);
        }
        out
    };
    Some(match val {
        Value::NVarChar(_) => Value::NVarChar(masked),
        _ => Value::VarChar(masked),
    })
}

fn mask_random(val: &Value, data_type: &DataType, args: &str) -> Option<Value> {
    let (lo_s, hi_s) = args.split_once(',')?;
    let lo: f64 = lo_s.trim().parse().ok()?;
    let hi: f64 = hi_s.trim().parse().ok()?;
    let (min_v, max_v) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    match val {
        Value::TinyInt(_) => {
            let v =
                random_i128_in_range((min_v as i128).clamp(0, 255), (max_v as i128).clamp(0, 255));
            Some(Value::TinyInt(v as u8))
        }
        Value::SmallInt(_) => {
            let v = random_i128_in_range(
                (min_v as i128).clamp(i16::MIN as i128, i16::MAX as i128),
                (max_v as i128).clamp(i16::MIN as i128, i16::MAX as i128),
            );
            Some(Value::SmallInt(v as i16))
        }
        Value::Int(_) => {
            let v = random_i128_in_range(
                (min_v as i128).clamp(i32::MIN as i128, i32::MAX as i128),
                (max_v as i128).clamp(i32::MIN as i128, i32::MAX as i128),
            );
            Some(Value::Int(v as i32))
        }
        Value::BigInt(_) => {
            let v = random_i128_in_range(min_v as i128, max_v as i128);
            Some(Value::BigInt(v as i64))
        }
        Value::Float(_) => {
            let v = if (max_v - min_v).abs() < f64::EPSILON {
                min_v
            } else {
                let frac = ThreadRng.next_f64();
                min_v + frac * (max_v - min_v)
            };
            Some(Value::Float(v.to_bits()))
        }
        Value::Decimal(_, scale) => {
            let scale_pow = 10i128.pow(*scale as u32);
            let lo_i = (min_v * scale_pow as f64).round() as i128;
            let hi_i = (max_v * scale_pow as f64).round() as i128;
            let v = random_i128_in_range(lo_i, hi_i);
            Some(Value::Decimal(v, *scale))
        }
        Value::Money(_) => {
            let lo_i = (min_v * 10_000.0).round() as i128;
            let hi_i = (max_v * 10_000.0).round() as i128;
            Some(Value::Money(random_i128_in_range(lo_i, hi_i)))
        }
        Value::SmallMoney(_) => {
            let lo_i = (min_v * 10_000.0).round() as i128;
            let hi_i = (max_v * 10_000.0).round() as i128;
            Some(Value::SmallMoney(random_i128_in_range(lo_i, hi_i) as i64))
        }
        _ => Some(mask_default(val, data_type)),
    }
}

fn mask_datetime(val: &Value, args: &str) -> Option<Value> {
    let part = args
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string();
    match val {
        Value::Date(d) => {
            let (y, m, day) = match part.as_str() {
                "Y" => (2000, d.month(), d.day()),
                "M" => (d.year(), 1, d.day()),
                "D" => (d.year(), d.month(), 1),
                _ => (d.year(), d.month(), d.day()),
            };
            let clamped = NaiveDate::from_ymd_opt(y, m, day)
                .or_else(|| NaiveDate::from_ymd_opt(y, m, 28))
                .unwrap_or(*d);
            Some(Value::Date(clamped))
        }
        Value::DateTime(dt) => Some(Value::DateTime(mask_naive_datetime(*dt, &part))),
        Value::DateTime2(dt) => Some(Value::DateTime2(mask_naive_datetime(*dt, &part))),
        Value::SmallDateTime(dt) => Some(Value::SmallDateTime(mask_naive_datetime(*dt, &part))),
        Value::DateTimeOffset(s) => {
            // Preserve trailing offset if present (e.g. "2024-06-15 14:30:00 +00:00")
            if let Ok(dt) = chrono::DateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S %:z") {
                let masked = mask_naive_datetime(dt.naive_local(), &part);
                Some(Value::DateTimeOffset(format!(
                    "{} {}",
                    masked.format("%Y-%m-%d %H:%M:%S"),
                    dt.offset()
                )))
            } else {
                Some(Value::DateTimeOffset(s.clone()))
            }
        }
        Value::Time(t) => {
            let masked = match part.as_str() {
                "h" => NaiveTime::from_hms_nano_opt(0, t.minute(), t.second(), t.nanosecond()),
                "m" => NaiveTime::from_hms_nano_opt(t.hour(), 0, t.second(), t.nanosecond()),
                "s" => NaiveTime::from_hms_nano_opt(t.hour(), t.minute(), 0, t.nanosecond()),
                _ => Some(*t),
            }
            .unwrap_or(*t);
            Some(Value::Time(masked))
        }
        _ => None,
    }
}

fn mask_naive_datetime(dt: NaiveDateTime, part: &str) -> NaiveDateTime {
    let d = dt.date();
    let t = dt.time();
    let (y, m, day) = match part {
        "Y" => (2000, d.month(), d.day()),
        "M" => (d.year(), 1, d.day()),
        "D" => (d.year(), d.month(), 1),
        _ => (d.year(), d.month(), d.day()),
    };
    let (hh, mm, ss) = match part {
        "h" => (0, t.minute(), t.second()),
        "m" => (t.hour(), 0, t.second()),
        "s" => (t.hour(), t.minute(), 0),
        _ => (t.hour(), t.minute(), t.second()),
    };
    let date = NaiveDate::from_ymd_opt(y, m, day)
        .or_else(|| NaiveDate::from_ymd_opt(y, m, 28))
        .unwrap_or(d);
    let time = NaiveTime::from_hms_nano_opt(hh, mm, ss, t.nanosecond()).unwrap_or(t);
    NaiveDateTime::new(date, time)
}

/// Applies Dynamic Data Masking to all masked columns in `rows` when the active user lacks
/// `UNMASK` privilege on the corresponding table. Fast-path returns `Cow::Borrowed` with zero
/// allocations when no masked columns are defined or the caller has `UNMASK`.
pub(crate) fn apply_dynamic_masking_to_rows<'a>(
    catalog: &dyn Catalog,
    ctx: &ExecutionContext<'_>,
    rows: &'a [JoinedRow],
) -> Cow<'a, [JoinedRow]> {
    let masked_defs = catalog.get_masked_columns();
    if masked_defs.is_empty() || rows.is_empty() || ctx.session.extras.mutation_target_id.is_some()
    {
        return Cow::Borrowed(rows);
    }

    // Build per-binding masking plan once from the first row's table bindings.
    // Plan entry: (binding_idx, col_idx, data_type, masking_function)
    let first = &rows[0];
    let mut plan: Vec<(usize, usize, DataType, &str)> = Vec::new();
    for (b_idx, binding) in first.iter().enumerate() {
        let tid = binding.table.id;
        if tid == 0 {
            continue;
        }
        let table_masks: Vec<&crate::catalog::MaskedColumnDef> =
            masked_defs.iter().filter(|m| m.table_id == tid).collect();
        if table_masks.is_empty() {
            continue;
        }
        if user_has_unmask(catalog, ctx, &binding.table.schema_name, tid) {
            continue;
        }
        for (c_idx, col) in binding.table.columns.iter().enumerate() {
            if let Some(m) = table_masks
                .iter()
                .find(|m| m.column_name.eq_ignore_ascii_case(&col.name))
            {
                plan.push((
                    b_idx,
                    c_idx,
                    col.data_type.clone(),
                    m.masking_function.as_str(),
                ));
            }
        }
    }

    if plan.is_empty() {
        return Cow::Borrowed(rows);
    }

    let mut out = rows.to_vec();
    for row in &mut out {
        for &(b_idx, c_idx, ref dt, func) in &plan {
            if let Some(binding) = row.get_mut(b_idx) {
                if let Some(ref mut stored) = binding.row {
                    if let Some(val) = stored.values.get_mut(c_idx) {
                        *val = apply_mask_function(val, dt, func);
                    }
                }
            }
        }
    }
    Cow::Owned(out)
}
