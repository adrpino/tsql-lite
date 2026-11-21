use crate::ast::Expr;
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::storage::Storage;
use crate::types::Value;

use super::super::super::clock::Clock;
use super::super::super::context::ExecutionContext;
use super::super::super::evaluator::eval_expr;
use super::super::super::model::ContextTable;

pub(crate) fn eval_concat(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let mut result = String::new();
    for arg in args {
        let val = eval_expr(arg, row, ctx, catalog, storage, clock)?;
        match val {
            Value::Null => {}
            Value::Money(raw) => result.push_str(&crate::types::format_money_varchar(raw)),
            Value::SmallMoney(raw) => {
                result.push_str(&crate::types::format_money_varchar(raw as i128))
            }
            other => result.push_str(&other.to_string_value()),
        }
    }
    Ok(Value::NVarChar(result))
}

pub(crate) fn eval_concat_ws(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() < 2 {
        return Err(DbError::Execution(
            "CONCAT_WS requires at least 2 arguments".into(),
        ));
    }
    let sep_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if sep_val.is_null() {
        return Ok(Value::Null);
    }
    let sep = sep_val.to_string_value();
    let mut parts = Vec::new();
    for arg in &args[1..] {
        let val = eval_expr(arg, row, ctx, catalog, storage, clock)?;
        match val {
            Value::Null => {}
            Value::Money(raw) => parts.push(crate::types::format_money_varchar(raw)),
            Value::SmallMoney(raw) => parts.push(crate::types::format_money_varchar(raw as i128)),
            other => parts.push(other.to_string_value()),
        }
    }
    Ok(Value::NVarChar(parts.join(&sep)))
}

pub(crate) fn eval_format(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() < 2 || args.len() > 3 {
        return Err(DbError::Execution("FORMAT expects 2 or 3 arguments".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let format_str = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;

    if val.is_null() {
        return Ok(Value::Null);
    }

    let fmt = format_str.to_string_value();

    match val {
        Value::DateTime(dt) | Value::DateTime2(dt) | Value::SmallDateTime(dt) => {
            let chrono_fmt = translate_dotnet_datetime_format(&fmt);
            Ok(Value::NVarChar(dt.format(&chrono_fmt).to_string()))
        }
        Value::Date(d) => {
            let dt = d.and_hms_opt(0, 0, 0).unwrap();
            let chrono_fmt = translate_dotnet_datetime_format(&fmt);
            Ok(Value::NVarChar(dt.format(&chrono_fmt).to_string()))
        }
        Value::TinyInt(v) => Ok(Value::NVarChar(format_integer(v as i64, &fmt))),
        Value::SmallInt(v) => Ok(Value::NVarChar(format_integer(v as i64, &fmt))),
        Value::Int(v) => Ok(Value::NVarChar(format_integer(v as i64, &fmt))),
        Value::BigInt(v) => Ok(Value::NVarChar(format_integer(v, &fmt))),
        Value::Decimal(raw, scale) => {
            let f = raw as f64 / 10f64.powi(scale as i32);
            Ok(Value::NVarChar(format_float_value(f, &fmt)))
        }
        Value::Money(raw) => {
            let f = raw as f64 / 10000.0;
            Ok(Value::NVarChar(format_float_value(f, &fmt)))
        }
        Value::SmallMoney(raw) => {
            let f = raw as f64 / 10000.0;
            Ok(Value::NVarChar(format_float_value(f, &fmt)))
        }
        Value::Float(bits) => {
            let f = f64::from_bits(bits);
            Ok(Value::NVarChar(format_float_value(f, &fmt)))
        }
        _ => Ok(Value::NVarChar(val.to_string_value())),
    }
}

fn translate_dotnet_datetime_format(fmt: &str) -> String {
    if fmt.contains('%') {
        return fmt.to_string();
    }
    let bytes = fmt.as_bytes();
    let mut out = String::with_capacity(fmt.len() + 8);
    let mut i = 0;
    while i < bytes.len() {
        let rest = &fmt[i..];
        if rest.starts_with("yyyy") {
            out.push_str("%Y");
            i += 4;
        } else if rest.starts_with("yy") {
            out.push_str("%y");
            i += 2;
        } else if rest.starts_with("MMMM") {
            out.push_str("%B");
            i += 4;
        } else if rest.starts_with("MMM") {
            out.push_str("%b");
            i += 3;
        } else if rest.starts_with("MM") {
            out.push_str("%m");
            i += 2;
        } else if rest.starts_with('M') {
            out.push_str("%-m");
            i += 1;
        } else if rest.starts_with("dddd") {
            out.push_str("%A");
            i += 4;
        } else if rest.starts_with("ddd") {
            out.push_str("%a");
            i += 3;
        } else if rest.starts_with("dd") {
            out.push_str("%d");
            i += 2;
        } else if rest.starts_with('d') {
            out.push_str("%-d");
            i += 1;
        } else if rest.starts_with("HH") {
            out.push_str("%H");
            i += 2;
        } else if rest.starts_with('H') {
            out.push_str("%-H");
            i += 1;
        } else if rest.starts_with("hh") {
            out.push_str("%I");
            i += 2;
        } else if rest.starts_with('h') {
            out.push_str("%-I");
            i += 1;
        } else if rest.starts_with("mm") {
            out.push_str("%M");
            i += 2;
        } else if rest.starts_with('m') {
            out.push_str("%-M");
            i += 1;
        } else if rest.starts_with("ss") {
            out.push_str("%S");
            i += 2;
        } else if rest.starts_with('s') {
            out.push_str("%-S");
            i += 1;
        } else if rest.starts_with("fff") {
            out.push_str("%3f");
            i += 3;
        } else if rest.starts_with("tt") {
            out.push_str("%p");
            i += 2;
        } else {
            let ch = rest.chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

pub(crate) fn eval_isnumeric(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution("ISNUMERIC expects 1 argument".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let is_num = match val {
        Value::Null => false,
        Value::Bit(_)
        | Value::TinyInt(_)
        | Value::SmallInt(_)
        | Value::Int(_)
        | Value::BigInt(_)
        | Value::Float(_)
        | Value::Decimal(_, _)
        | Value::Money(_)
        | Value::SmallMoney(_) => true,
        Value::Char(s) | Value::VarChar(s) | Value::NChar(s) | Value::NVarChar(s) => {
            let t = s.trim();
            if t.is_empty() {
                false
            } else {
                let no_commas = t.replace(',', "");
                let stripped = no_commas
                    .trim_start_matches('$')
                    .trim_start_matches(['+', '-'])
                    .trim_start_matches('$');
                !stripped.is_empty()
                    && (no_commas.parse::<f64>().is_ok()
                        || crate::executor::value_ops::coercion::numeric::parse_money_string(t)
                            .is_ok())
            }
        }
        _ => false,
    };
    Ok(Value::Int(if is_num { 1 } else { 0 }))
}

pub(crate) fn eval_formatmessage(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.is_empty() {
        return Err(DbError::Execution(
            "FORMATMESSAGE expects at least 1 argument".into(),
        ));
    }
    let template = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if template.is_null() {
        return Ok(Value::Null);
    }
    let mut formatted_args = Vec::with_capacity(args.len().saturating_sub(1));
    for arg in &args[1..] {
        let v = eval_expr(arg, row, ctx, catalog, storage, clock)?;
        formatted_args.push(if v.is_null() {
            "(null)".to_string()
        } else {
            v.to_string_value()
        });
    }
    let out = crate::executor::script::procedural::raiserror::format_raiserror_message(
        &template.to_string_value(),
        &formatted_args,
    );
    Ok(Value::NVarChar(out))
}

pub(crate) fn eval_str(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.is_empty() || args.len() > 3 {
        return Err(DbError::Execution("STR expects 1 to 3 arguments".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let length = if args.len() >= 2 {
        eval_expr(&args[1], row, ctx, catalog, storage, clock)?
            .to_integer_i64()
            .unwrap_or(10) as usize
    } else {
        10
    };
    let decimals = if args.len() == 3 {
        eval_expr(&args[2], row, ctx, catalog, storage, clock)?
            .to_integer_i64()
            .unwrap_or(0) as usize
    } else {
        0
    };

    let f = val.to_f64().unwrap_or(0.0);
    let formatted = format!("{:.*}", decimals, f);
    let trimmed = formatted.trim();
    if trimmed.len() >= length {
        Ok(Value::VarChar(trimmed.to_string()))
    } else {
        Ok(Value::VarChar(format!(
            "{:>width$}",
            trimmed,
            width = length
        )))
    }
}

pub(crate) fn eval_replicate(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution("REPLICATE expects 2 arguments".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let count = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if val.is_null() || count.is_null() {
        return Ok(Value::Null);
    }
    let s = val.to_string_value();
    let n = count.to_integer_i64().unwrap_or(0).max(0) as usize;
    Ok(Value::VarChar(s.repeat(n)))
}

pub(crate) fn eval_reverse(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution("REVERSE expects 1 argument".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let s = val.to_string_value();
    Ok(Value::NVarChar(s.chars().rev().collect()))
}

pub(crate) fn eval_stuff(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 4 {
        return Err(DbError::Execution("STUFF expects 4 arguments".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let start = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    let length = eval_expr(&args[2], row, ctx, catalog, storage, clock)?;
    let replacement = eval_expr(&args[3], row, ctx, catalog, storage, clock)?;

    if val.is_null() || start.is_null() || length.is_null() {
        return Ok(Value::Null);
    }

    let s = val.to_string_value();
    let start_i = start.to_integer_i64().unwrap_or(1) as i32;
    let length_i = length.to_integer_i64().unwrap_or(0) as i32;
    let repl = replacement.to_string_value();

    let chars: Vec<char> = s.chars().collect();
    if start_i <= 0 || (start_i as usize) > chars.len() || length_i < 0 {
        return Ok(Value::Null);
    }
    let start_idx = (start_i - 1) as usize;
    let delete_count = length_i as usize;
    let end_idx = (start_idx + delete_count).min(chars.len());

    let mut result: String = chars[..start_idx].iter().collect();
    result.push_str(&repl);
    result.extend(chars[end_idx..].iter());

    Ok(Value::NVarChar(result))
}

pub(crate) fn eval_space(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution("SPACE expects 1 argument".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let n = val.to_integer_i64().unwrap_or(0).max(0) as usize;
    Ok(Value::VarChar(" ".repeat(n)))
}

pub(crate) fn eval_translate(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 3 {
        return Err(DbError::Execution("TRANSLATE expects 3 arguments".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let from = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    let to = eval_expr(&args[2], row, ctx, catalog, storage, clock)?;

    if val.is_null() || from.is_null() || to.is_null() {
        return Ok(Value::Null);
    }

    let s = val.to_string_value();
    let from_chars: Vec<char> = from.to_string_value().chars().collect();
    let to_chars: Vec<char> = to.to_string_value().chars().collect();

    if from_chars.len() != to_chars.len() {
        return Err(DbError::Execution(
            "TRANSLATE: the second and third arguments must have the same length".into(),
        ));
    }

    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        if let Some(pos) = from_chars.iter().position(|&fc| fc == c) {
            result.push(to_chars[pos]);
        } else {
            result.push(c);
        }
    }
    Ok(Value::NVarChar(result))
}

pub(crate) fn eval_unistr(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution("UNISTR expects 1 argument".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let s = val.to_string_value();

    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(&next) = chars.peek() {
                if next == 'u' || next == 'U' {
                    chars.next();
                    let hex_len = if next == 'u' { 4 } else { 8 };
                    let mut hex = String::with_capacity(hex_len);
                    for _ in 0..hex_len {
                        if let Some(h) = chars.next() {
                            hex.push(h);
                        }
                    }
                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                        if let Some(unicode_char) = std::char::from_u32(code) {
                            result.push(unicode_char);
                            continue;
                        }
                    }
                }
            }
        }
        result.push(ch);
    }

    Ok(Value::NVarChar(result))
}

pub(crate) fn eval_string_escape(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution(
            "STRING_ESCAPE expects 2 arguments".into(),
        ));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let type_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;

    if val.is_null() {
        return Ok(Value::Null);
    }

    let s = val.to_string_value();
    let escape_type = type_val.to_string_value().to_uppercase();

    if escape_type == "JSON" {
        let mut result = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '"' => result.push_str("\\\""),
                '\\' => result.push_str("\\\\"),
                '\u{0008}' => result.push_str("\\b"),
                '\u{000C}' => result.push_str("\\f"),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                _ if c.is_control() => {
                    result.push_str(&format!("\\u{:04x}", c as u32));
                }
                _ => result.push(c),
            }
        }
        Ok(Value::NVarChar(result))
    } else if escape_type == "HTML" || escape_type == "XML" {
        let mut result = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '&' => result.push_str("&amp;"),
                '<' => result.push_str("&lt;"),
                '>' => result.push_str("&gt;"),
                '"' => result.push_str("&quot;"),
                '\'' => result.push_str("&apos;"),
                _ => result.push(c),
            }
        }
        Ok(Value::NVarChar(result))
    } else if escape_type == "CSV" {
        let mut result = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                ',' => result.push_str("\\,"),
                '"' => result.push_str("\\\""),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\\' => result.push_str("\\\\"),
                _ => result.push(c),
            }
        }
        Ok(Value::NVarChar(result))
    } else {
        Err(DbError::Execution(format!(
            "STRING_ESCAPE: unsupported escape type '{}'",
            escape_type
        )))
    }
}

fn format_integer(v: i64, fmt: &str) -> String {
    if fmt.starts_with('D') || fmt.starts_with('d') {
        let width: usize = fmt[1..].parse().unwrap_or(1);
        if v < 0 {
            format!("-{:0>width$}", v.unsigned_abs(), width = width)
        } else {
            format!("{:0>width$}", v, width = width)
        }
    } else if fmt == "N" || fmt == "n" {
        format_number_with_commas(&format!("{}", v))
    } else if fmt == "C" || fmt == "c" {
        format!("${}", v)
    } else if fmt == "P" || fmt == "p" {
        format!("{}%", v)
    } else if fmt.contains('0')
        || fmt.contains('#')
        || fmt.contains('.')
        || fmt.starts_with(['F', 'f', 'N', 'n', 'C', 'c', 'P', 'p', 'G', 'g'])
    {
        format_float_value(v as f64, fmt)
    } else {
        format!("{}", v)
    }
}

fn format_float_value(f: f64, fmt: &str) -> String {
    if fmt.starts_with('N') || fmt.starts_with('n') {
        let decimals: usize = if fmt.len() > 1 {
            fmt[1..].parse().unwrap_or(2)
        } else {
            2
        };
        format_number_with_commas(&format!("{:.*}", decimals, f))
    } else if fmt.starts_with('F') || fmt.starts_with('f') {
        let decimals: usize = if fmt.len() > 1 {
            fmt[1..].parse().unwrap_or(2)
        } else {
            2
        };
        format!("{:.*}", decimals, f)
    } else if fmt.starts_with('C') || fmt.starts_with('c') {
        let decimals: usize = if fmt.len() > 1 {
            fmt[1..].parse().unwrap_or(2)
        } else {
            2
        };
        let num = format_number_with_commas(&format!("{:.*}", decimals, f.abs()));
        if f < 0.0 {
            format!("-${}", num)
        } else {
            format!("${}", num)
        }
    } else if fmt.starts_with('P') || fmt.starts_with('p') {
        let decimals: usize = if fmt.len() > 1 {
            fmt[1..].parse().unwrap_or(2)
        } else {
            2
        };
        format!("{:.*}%", decimals, f * 100.0)
    } else if fmt.starts_with('G') || fmt.starts_with('g') {
        format!("{}", f)
    } else if fmt.contains('0') || fmt.contains('#') {
        format_custom_numeric(f, fmt)
    } else {
        format!("{}", f)
    }
}

fn format_custom_numeric(f: f64, fmt: &str) -> String {
    let section = fmt.split(';').next().unwrap_or(fmt);
    let mut parts = section.splitn(2, '.');
    let int_spec = parts.next().unwrap_or("0");
    let frac_spec = parts.next().unwrap_or("");

    let use_commas = int_spec.contains(',');
    let min_int_digits = int_spec.chars().filter(|&c| c == '0').count();
    let min_frac_digits = frac_spec.chars().filter(|&c| c == '0').count();
    let max_frac_digits = frac_spec
        .chars()
        .filter(|&c| c == '0' || c == '#')
        .count();

    let negative = f < 0.0;
    let rounded = format!("{:.*}", max_frac_digits, f.abs());
    let mut r_parts = rounded.splitn(2, '.');
    let mut int_str = r_parts.next().unwrap_or("0").to_string();
    let mut frac_str = r_parts.next().unwrap_or("").to_string();

    while frac_str.len() > min_frac_digits && frac_str.ends_with('0') {
        frac_str.pop();
    }
    if int_str == "0" && min_int_digits == 0 && !frac_str.is_empty() {
        int_str.clear();
    } else if int_str.len() < min_int_digits {
        int_str = format!("{:0>width$}", int_str, width = min_int_digits);
    }

    let formatted_int = if use_commas && !int_str.is_empty() {
        format_number_with_commas(&int_str)
    } else {
        int_str
    };
    let sign = if negative && (formatted_int != "0" || !frac_str.is_empty()) {
        "-"
    } else {
        ""
    };
    if frac_str.is_empty() {
        format!("{}{}", sign, formatted_int)
    } else {
        format!("{}{}.{}", sign, formatted_int, frac_str)
    }
}

fn format_number_with_commas(s: &str) -> String {
    let (negative, abs) = if let Some(stripped) = s.strip_prefix('-') {
        (true, stripped)
    } else {
        (false, s)
    };
    let parts: Vec<&str> = abs.splitn(2, '.').collect();
    let int_part = parts[0];
    let frac_part = parts.get(1).copied();

    let mut result = String::new();
    let chars: Vec<char> = int_part.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(*c);
    }
    if let Some(frac) = frac_part {
        result.push('.');
        result.push_str(frac);
    }
    if negative {
        format!("-{}", result)
    } else {
        result
    }
}
