use crate::ast::Expr;
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::executor::clock::Clock;
use crate::executor::context::ExecutionContext;
use crate::executor::evaluator::eval_expr;
use crate::executor::model::ContextTable;
use crate::executor::value_ops::{coerce_value_to_type_with_dateformat, compare_values};
use crate::storage::Storage;
use crate::types::Value;
use std::cmp::Ordering;

/// Evaluates `$PARTITION.<pf_name>(expr)` against `catalog.get_partition_functions()`.
///
/// Follows SQL Server partitioning rules:
/// - `RANGE LEFT`: boundary `b_i` belongs to partition `i + 1` (`val <= b_i`).
/// - `RANGE RIGHT`: boundary `b_i` belongs to partition `i + 2` (`val < b_i`).
/// - `NULL` always maps to partition `1`.
pub(crate) fn eval_partition_function(
    pf_name: &str,
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution(format!(
            "$PARTITION.{} expects 1 argument",
            pf_name
        )));
    }
    let pf = catalog
        .get_partition_functions()
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(pf_name))
        .cloned()
        .ok_or_else(|| {
            DbError::Execution(format!("Invalid partition function name '{}'.", pf_name))
        })?;

    let raw_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if raw_val.is_null() {
        return Ok(Value::Int(1));
    }
    let val =
        coerce_value_to_type_with_dateformat(raw_val, &pf.data_type, &ctx.options.dateformat)?;

    let mut boundaries: Vec<Value> = Vec::with_capacity(pf.boundary_values.len());
    for raw_b in &pf.boundary_values {
        let trimmed = raw_b.trim().trim_start_matches('N').trim_matches('\'');
        let b_val = coerce_value_to_type_with_dateformat(
            Value::NVarChar(trimmed.to_string()),
            &pf.data_type,
            &ctx.options.dateformat,
        )?;
        boundaries.push(b_val);
    }
    boundaries.sort_by(|a, b| compare_values(a, b));

    let mut part: i32 = 1;
    for b in &boundaries {
        let ord = compare_values(&val, b);
        if pf.boundary_value_on_right {
            if ord == Ordering::Less {
                break;
            }
        } else if ord == Ordering::Less || ord == Ordering::Equal {
            break;
        }
        part += 1;
    }

    Ok(Value::Int(part))
}

fn eval_text_target(
    arg: &Expr,
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Option<String>, DbError> {
    if matches!(arg, Expr::Wildcard) {
        let mut combined = String::new();
        for tbl in row {
            if let Some(ref r) = tbl.row {
                for v in &r.values {
                    if !v.is_null() {
                        if !combined.is_empty() {
                            combined.push(' ');
                        }
                        combined.push_str(&v.to_string_value());
                    }
                }
            }
        }
        return Ok(Some(combined));
    }
    let v = eval_expr(arg, row, ctx, catalog, storage, clock)?;
    if v.is_null() {
        Ok(None)
    } else {
        Ok(Some(v.to_string_value()))
    }
}

fn tokenize_words_lower(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

fn contains_term_match(haystack_lower: &str, words: &[String], term: &str) -> bool {
    let trimmed = term.trim();
    if trimmed.is_empty() {
        return false;
    }
    let inner = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(trimmed)
        .trim();
    if inner.is_empty() {
        return false;
    }
    if let Some(prefix) = inner.strip_suffix('*') {
        let p = prefix.to_lowercase();
        if p.is_empty() {
            return !words.is_empty();
        }
        return words.iter().any(|w| w.starts_with(&p));
    }
    if inner.contains(char::is_whitespace) {
        let needle_words = tokenize_words_lower(inner);
        if needle_words.is_empty() {
            return false;
        }
        return words
            .windows(needle_words.len())
            .any(|win| win == needle_words.as_slice());
    }
    let needle = inner.to_lowercase();
    words.iter().any(|w| w == &needle) || haystack_lower.split_whitespace().any(|w| w == needle)
}

fn eval_contains_condition(haystack_lower: &str, words: &[String], cond: &str) -> bool {
    // Split on top-level OR outside double quotes.
    for or_part in split_boolean_top(cond, "OR") {
        // Split each OR branch on top-level AND.
        let and_parts = split_boolean_top(or_part, "AND");
        let mut all_ok = true;
        for raw_part in and_parts {
            let p = raw_part.trim();
            if p.is_empty() {
                continue;
            }
            if let Some(rest) = strip_leading_keyword(p, "NOT") {
                if contains_term_match(haystack_lower, words, rest) {
                    all_ok = false;
                    break;
                }
            } else if !contains_term_match(haystack_lower, words, p) {
                all_ok = false;
                break;
            }
        }
        if all_ok {
            return true;
        }
    }
    false
}

fn strip_leading_keyword<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    let trimmed = s.trim_start();
    if trimmed.len() > kw.len()
        && trimmed[..kw.len()].eq_ignore_ascii_case(kw)
        && trimmed.as_bytes()[kw.len()].is_ascii_whitespace()
    {
        Some(trimmed[kw.len()..].trim_start())
    } else {
        None
    }
}

fn split_boolean_top<'a>(s: &'a str, op: &str) -> Vec<&'a str> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut in_quotes = false;
    let mut start = 0usize;
    let mut i = 0usize;
    let op_len = op.len();
    while i < bytes.len() {
        if bytes[i] == b'"' {
            in_quotes = !in_quotes;
            i += 1;
            continue;
        }
        if !in_quotes
            && i + op_len <= bytes.len()
            && s[i..i + op_len].eq_ignore_ascii_case(op)
            && (i == 0 || bytes[i - 1].is_ascii_whitespace())
            && (i + op_len == bytes.len() || bytes[i + op_len].is_ascii_whitespace())
        {
            out.push(&s[start..i]);
            i += op_len;
            start = i;
            continue;
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

/// Evaluates `CONTAINS(column_or_star, 'search_condition')`.
pub(crate) fn eval_contains(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() < 2 {
        return Err(DbError::Execution(
            "CONTAINS expects at least 2 arguments".into(),
        ));
    }
    let Some(haystack) = eval_text_target(&args[0], row, ctx, catalog, storage, clock)? else {
        return Ok(Value::Bit(false));
    };
    let cond_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if cond_val.is_null() {
        return Ok(Value::Bit(false));
    }
    let cond = cond_val.to_string_value();
    let lower = haystack.to_lowercase();
    let words = tokenize_words_lower(&lower);
    Ok(Value::Bit(eval_contains_condition(&lower, &words, &cond)))
}

/// Evaluates `FREETEXT(column_or_star, 'freetext_string')`.
pub(crate) fn eval_freetext(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() < 2 {
        return Err(DbError::Execution(
            "FREETEXT expects at least 2 arguments".into(),
        ));
    }
    let Some(haystack) = eval_text_target(&args[0], row, ctx, catalog, storage, clock)? else {
        return Ok(Value::Bit(false));
    };
    let phrase_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if phrase_val.is_null() {
        return Ok(Value::Bit(false));
    }
    let hay_words = tokenize_words_lower(&haystack);
    let query_words = tokenize_words_lower(&phrase_val.to_string_value());
    let matched = query_words
        .iter()
        .any(|qw| hay_words.iter().any(|hw| hw == qw));
    Ok(Value::Bit(matched))
}

// ============================================================================
// Spatial (GEOGRAPHY / GEOMETRY) & HIERARCHYID support
// ============================================================================

#[derive(Debug, Clone, Copy)]
struct SpatialPoint {
    is_geography: bool,
    srid: i32,
    /// For geography: latitude; for geometry: X.
    a: f64,
    /// For geography: longitude; for geometry: Y.
    b: f64,
}

impl SpatialPoint {
    fn encode(self) -> String {
        let tag = if self.is_geography { "GEOG" } else { "GEOM" };
        format!("__{}:{}:{}:{}", tag, self.srid, self.a, self.b)
    }

    fn decode(raw: &str) -> Option<Self> {
        let rest = raw.strip_prefix("__")?;
        let mut parts = rest.split(':');
        let tag = parts.next()?;
        let is_geography = match tag {
            "GEOG" => true,
            "GEOM" => false,
            _ => return None,
        };
        let srid = parts.next()?.parse::<i32>().ok()?;
        let a = parts.next()?.parse::<f64>().ok()?;
        let b = parts.next()?.parse::<f64>().ok()?;
        Some(Self {
            is_geography,
            srid,
            a,
            b,
        })
    }

    fn as_wkt(self) -> String {
        if self.is_geography {
            // WKT POINT is (longitude latitude)
            format!("POINT ({} {})", trim_f64(self.b), trim_f64(self.a))
        } else {
            format!("POINT ({} {})", trim_f64(self.a), trim_f64(self.b))
        }
    }
}

fn trim_f64(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

fn parse_wkt_point(wkt: &str, srid: i32, is_geography: bool) -> Result<SpatialPoint, DbError> {
    let trimmed = wkt.trim();
    let upper = trimmed.to_ascii_uppercase();
    let inner = upper
        .strip_prefix("POINT")
        .map(|s| s.trim())
        .and_then(|s| s.strip_prefix('('))
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| DbError::Execution(format!("Unsupported WKT geometry: '{}'", wkt)))?;
    let coords: Vec<&str> = inner.split_whitespace().collect();
    if coords.len() < 2 {
        return Err(DbError::Execution(format!(
            "Invalid WKT POINT coordinates: '{}'",
            wkt
        )));
    }
    let first: f64 = coords[0]
        .parse()
        .map_err(|_| DbError::Execution(format!("Invalid WKT coordinate '{}'", coords[0])))?;
    let second: f64 = coords[1]
        .parse()
        .map_err(|_| DbError::Execution(format!("Invalid WKT coordinate '{}'", coords[1])))?;
    if is_geography {
        // WKT POINT(lon lat) -> (a=lat, b=lon)
        Ok(SpatialPoint {
            is_geography: true,
            srid,
            a: second,
            b: first,
        })
    } else {
        Ok(SpatialPoint {
            is_geography: false,
            srid,
            a: first,
            b: second,
        })
    }
}

fn value_to_f64(v: &Value) -> Result<f64, DbError> {
    match v {
        Value::Float(bits) => Ok(f64::from_bits(*bits)),
        Value::Int(i) => Ok(*i as f64),
        Value::BigInt(i) => Ok(*i as f64),
        Value::SmallInt(i) => Ok(*i as f64),
        Value::TinyInt(i) => Ok(*i as f64),
        Value::Decimal(d, s) => Ok((*d as f64) / 10f64.powi(*s as i32)),
        other => other
            .to_string_value()
            .trim()
            .parse::<f64>()
            .map_err(|_| DbError::Execution(format!("Cannot convert {:?} to FLOAT", other))),
    }
}

pub(crate) fn eval_spatial_point(
    is_geography: bool,
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 3 {
        return Err(DbError::Execution("Point expects 3 arguments".into()));
    }
    let a_v = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let b_v = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    let srid_v = eval_expr(&args[2], row, ctx, catalog, storage, clock)?;
    if a_v.is_null() || b_v.is_null() || srid_v.is_null() {
        return Ok(Value::Null);
    }
    let a = value_to_f64(&a_v)?;
    let b = value_to_f64(&b_v)?;
    let srid = srid_v.to_integer_i64().unwrap_or(4326) as i32;
    if is_geography && (!(-90.0..=90.0).contains(&a) || !(-180.0..=180.0).contains(&b)) {
        return Err(DbError::Execution(
            "Latitude values must be between -90 and 90 degrees and longitude between -180 and 180."
                .into(),
        ));
    }
    Ok(Value::NVarChar(
        SpatialPoint {
            is_geography,
            srid,
            a,
            b,
        }
        .encode(),
    ))
}

pub(crate) fn eval_spatial_from_text(
    is_geography: bool,
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution(
            "STGeomFromText expects 2 arguments".into(),
        ));
    }
    let wkt_v = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let srid_v = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if wkt_v.is_null() || srid_v.is_null() {
        return Ok(Value::Null);
    }
    let srid = srid_v.to_integer_i64().unwrap_or(0) as i32;
    let pt = parse_wkt_point(&wkt_v.to_string_value(), srid, is_geography)?;
    Ok(Value::NVarChar(pt.encode()))
}

pub(crate) fn eval_spatial_distance(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution("STDistance expects 1 argument".into()));
    }
    let v1 = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let v2 = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if v1.is_null() || v2.is_null() {
        return Ok(Value::Null);
    }
    let p1 = SpatialPoint::decode(&v1.to_string_value())
        .ok_or_else(|| DbError::Execution("Invalid spatial value in STDistance".into()))?;
    let p2 = SpatialPoint::decode(&v2.to_string_value())
        .ok_or_else(|| DbError::Execution("Invalid spatial value in STDistance".into()))?;
    if p1.srid != p2.srid {
        return Ok(Value::Null);
    }
    let dist = if p1.is_geography {
        // WGS84 mean spherical radius (6371008.7714 m) Haversine
        const EARTH_RADIUS_M: f64 = 6_371_008.7714;
        let lat1 = p1.a.to_radians();
        let lat2 = p2.a.to_radians();
        let dlat = (p2.a - p1.a).to_radians();
        let dlon = (p2.b - p1.b).to_radians();
        let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
        2.0 * EARTH_RADIUS_M * h.sqrt().min(1.0).asin()
    } else {
        ((p2.a - p1.a).powi(2) + (p2.b - p1.b).powi(2)).sqrt()
    };
    Ok(Value::Float(dist.to_bits()))
}

pub(crate) fn eval_spatial_as_text(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let v = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if v.is_null() {
        return Ok(Value::Null);
    }
    let pt = SpatialPoint::decode(&v.to_string_value())
        .ok_or_else(|| DbError::Execution("Invalid spatial value in STAsText".into()))?;
    Ok(Value::NVarChar(pt.as_wkt()))
}

pub(crate) fn eval_spatial_prop(
    prop: &str,
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let v = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if v.is_null() {
        return Ok(Value::Null);
    }
    let pt = SpatialPoint::decode(&v.to_string_value())
        .ok_or_else(|| DbError::Execution(format!("Invalid spatial value for .{}", prop)))?;
    match prop {
        "LAT" | "STX" => Ok(Value::Float(pt.a.to_bits())),
        "LONG" | "STY" => Ok(Value::Float(pt.b.to_bits())),
        "STSRID" => Ok(Value::Int(pt.srid)),
        _ => Err(DbError::Execution(format!(
            "Unsupported spatial property '{}'",
            prop
        ))),
    }
}

fn normalize_hierarchyid(raw: &str) -> Result<String, DbError> {
    let s = raw.trim();
    if s == "/" {
        return Ok("/".to_string());
    }
    if !s.starts_with('/') || !s.ends_with('/') {
        return Err(DbError::Execution(format!(
            "Invalid hierarchyid format: '{}'",
            raw
        )));
    }
    for seg in s[1..s.len() - 1].split('/') {
        if seg.is_empty()
            || !seg
                .split('.')
                .all(|part| !part.is_empty() && part.parse::<i64>().is_ok())
        {
            return Err(DbError::Execution(format!(
                "Invalid hierarchyid segment '{}' in '{}'",
                seg, raw
            )));
        }
    }
    Ok(s.to_string())
}

pub(crate) fn eval_hierarchyid_parse(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution(
            "hierarchyid::Parse expects 1 argument".into(),
        ));
    }
    let v = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if v.is_null() {
        return Ok(Value::Null);
    }
    Ok(Value::NVarChar(normalize_hierarchyid(
        &v.to_string_value(),
    )?))
}

pub(crate) fn eval_hierarchyid_get_root(
    args: &[Expr],
    _row: &[ContextTable],
    _ctx: &mut ExecutionContext<'_>,
    _catalog: &dyn Catalog,
    _storage: &dyn Storage,
    _clock: &dyn Clock,
) -> Result<Value, DbError> {
    if !args.is_empty() {
        return Err(DbError::Execution(
            "hierarchyid::GetRoot expects no arguments".into(),
        ));
    }
    Ok(Value::NVarChar("/".to_string()))
}

pub(crate) fn eval_method_to_string(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let v = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if v.is_null() {
        return Ok(Value::Null);
    }
    let raw = v.to_string_value();
    if let Some(pt) = SpatialPoint::decode(&raw) {
        return Ok(Value::NVarChar(pt.as_wkt()));
    }
    Ok(Value::NVarChar(raw))
}

pub(crate) fn eval_hierarchyid_get_level(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let v = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if v.is_null() {
        return Ok(Value::Null);
    }
    let norm = normalize_hierarchyid(&v.to_string_value())?;
    if norm == "/" {
        return Ok(Value::SmallInt(0));
    }
    let depth = norm[1..norm.len() - 1].split('/').count() as i16;
    Ok(Value::SmallInt(depth))
}

pub(crate) fn eval_hierarchyid_is_descendant_of(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution(
            "IsDescendantOf expects 1 argument".into(),
        ));
    }
    let child_v = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let parent_v = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if child_v.is_null() || parent_v.is_null() {
        return Ok(Value::Null);
    }
    let child = normalize_hierarchyid(&child_v.to_string_value())?;
    let parent = normalize_hierarchyid(&parent_v.to_string_value())?;
    Ok(Value::Bit(child.starts_with(&parent)))
}

pub(crate) fn eval_hierarchyid_get_ancestor(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution("GetAncestor expects 1 argument".into()));
    }
    let node_v = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let n_v = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if node_v.is_null() || n_v.is_null() {
        return Ok(Value::Null);
    }
    let norm = normalize_hierarchyid(&node_v.to_string_value())?;
    let n = n_v.to_integer_i64().unwrap_or(0);
    if n < 0 {
        return Err(DbError::Execution(
            "GetAncestor argument must be non-negative".into(),
        ));
    }
    let n = n as usize;
    let segs: Vec<&str> = if norm == "/" {
        Vec::new()
    } else {
        norm[1..norm.len() - 1].split('/').collect()
    };
    if n > segs.len() {
        return Ok(Value::Null);
    }
    let kept = &segs[..segs.len() - n];
    if kept.is_empty() {
        Ok(Value::NVarChar("/".to_string()))
    } else {
        Ok(Value::NVarChar(format!("/{}/", kept.join("/"))))
    }
}

pub(crate) fn eval_hierarchyid_get_reparented_value(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 3 {
        return Err(DbError::Execution(
            "GetReparentedValue expects 2 arguments".into(),
        ));
    }
    let node_v = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let old_v = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    let new_v = eval_expr(&args[2], row, ctx, catalog, storage, clock)?;
    if node_v.is_null() || old_v.is_null() || new_v.is_null() {
        return Ok(Value::Null);
    }
    let node = normalize_hierarchyid(&node_v.to_string_value())?;
    let old_root = normalize_hierarchyid(&old_v.to_string_value())?;
    let new_root = normalize_hierarchyid(&new_v.to_string_value())?;
    if !node.starts_with(&old_root) {
        return Err(DbError::Execution(
            "GetReparentedValue: oldRoot is not an ancestor of the node".into(),
        ));
    }
    let suffix = &node[old_root.len()..];
    if suffix.is_empty() {
        Ok(Value::NVarChar(new_root))
    } else {
        Ok(Value::NVarChar(format!("{}{}", new_root, suffix)))
    }
}
