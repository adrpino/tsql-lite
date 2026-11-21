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
            if args.len() == 2 {
                if let Some((receiver, method)) = name.rsplit_once('.') {
                    if method.eq_ignore_ascii_case("value") {
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
                        let Some(extracted) = eval_xml_value(
                            &xml_val.to_string_value(),
                            &xpath_val.to_string_value(),
                        ) else {
                            return Ok(Value::Null);
                        };
                        let target_ty = parse_xml_value_target_type(&sql_ty_val.to_string_value());
                        return crate::executor::value_ops::coerce_value_to_type_with_dateformat(
                            Value::NVarChar(extracted),
                            &target_ty,
                            &ctx.options.dateformat,
                        );
                    }
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

fn eval_xml_value(xml: &str, xpath: &str) -> Option<String> {
    let trimmed = xpath.trim();
    if trimmed == "." || trimmed == "text()" || trimmed == "(./text())[1]" {
        return Some(unescape_xml_entities(xml));
    }

    // Parse optional trailing `[N]` 1-based ordinal index and outer parens.
    let (path_body, ordinal) = if let Some(lb) = trimmed.rfind('[') {
        if let Some(rb) = trimmed[lb..].find(']') {
            let idx = trimmed[lb + 1..lb + rb].trim().parse::<usize>().ok().unwrap_or(1);
            (&trimmed[..lb], idx.max(1))
        } else {
            (trimmed, 1)
        }
    } else {
        (trimmed, 1)
    };
    let clean_path = path_body
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim()
        .trim_start_matches('/');

    let segments: Vec<&str> = clean_path
        .split('/')
        .map(|s| s.split('[').next().unwrap_or(s).trim())
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("text()"))
        .collect();
    if segments.is_empty() {
        return Some(unescape_xml_entities(xml));
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
        let open_prefix = format!("<{}", tag);
        let close_tag = format!("</{}>", tag);
        let mut search_pos = 0usize;
        let mut seen = 0usize;
        let mut found = false;
        while let Some(rel) = slice[search_pos..].find(&open_prefix) {
            let start = search_pos + rel;
            let after_name = start + open_prefix.len();
            let next_ch = slice.as_bytes().get(after_name).copied();
            if !matches!(
                next_ch,
                Some(b'>') | Some(b'/') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
            ) {
                search_pos = after_name;
                continue;
            }
            let gt_rel = slice[start..].find('>')?;
            let gt_idx = start + gt_rel;
            let self_closing = slice.as_bytes().get(gt_idx.saturating_sub(1)) == Some(&b'/');
            let (elem_open, elem_inner, elem_end) = if self_closing {
                (&slice[start..=gt_idx], "", gt_idx + 1)
            } else {
                let close_rel = slice[gt_idx + 1..].find(&close_tag)?;
                let inner = &slice[gt_idx + 1..gt_idx + 1 + close_rel];
                (
                    &slice[start..=gt_idx],
                    inner,
                    gt_idx + 1 + close_rel + close_tag.len(),
                )
            };
            seen += 1;
            if seen == target_ord {
                matched_open_tag = elem_open;
                matched_inner = elem_inner;
                slice = elem_inner;
                found = true;
                break;
            }
            search_pos = elem_end;
        }
        if !found {
            return None;
        }
    }

    if let Some(attr) = attr_name {
        let needle_dq = format!("{}=\"", attr);
        if let Some(pos) = matched_open_tag.find(&needle_dq) {
            let rest = &matched_open_tag[pos + needle_dq.len()..];
            let end = rest.find('"')?;
            return Some(unescape_xml_entities(&rest[..end]));
        }
        let needle_sq = format!("{}='", attr);
        if let Some(pos) = matched_open_tag.find(&needle_sq) {
            let rest = &matched_open_tag[pos + needle_sq.len()..];
            let end = rest.find('\'')?;
            return Some(unescape_xml_entities(&rest[..end]));
        }
        return None;
    }

    Some(unescape_xml_entities(matched_inner))
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
