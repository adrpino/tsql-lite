use serde_json::Value as JsonValue;

use crate::error::DbError;
use crate::types::Value;

pub fn parse_json(s: &str) -> Result<JsonValue, DbError> {
    serde_json::from_str(s).map_err(|e| DbError::Execution(format!("Invalid JSON: {}", e)))
}

pub fn json_to_string(json: &JsonValue) -> String {
    serde_json::to_string(json).unwrap_or_else(|_| "null".to_string())
}

pub fn sql_value_to_json_fragment(val: &Value, embed_raw_json: bool) -> String {
    if embed_raw_json {
        if let Value::VarChar(s) | Value::NVarChar(s) | Value::Char(s) | Value::NChar(s) = val {
            if serde_json::from_str::<JsonValue>(s).is_ok() {
                return s.clone();
            }
        }
    }
    match val {
        Value::Null => "null".to_string(),
        Value::Bit(b) => if *b { "true" } else { "false" }.to_string(),
        Value::TinyInt(n) => n.to_string(),
        Value::SmallInt(n) => n.to_string(),
        Value::Int(n) => n.to_string(),
        Value::BigInt(n) => n.to_string(),
        Value::Float(bits) => {
            let f = f64::from_bits(*bits);
            if f.is_finite() {
                serde_json::Number::from_f64(f)
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "null".to_string())
            } else {
                "null".to_string()
            }
        }
        Value::Decimal(unscaled, scale) => {
            if *scale == 0 {
                unscaled.to_string()
            } else {
                val.to_string_value()
            }
        }
        Value::Money(_) | Value::SmallMoney(_) => val.to_string_value(),
        _ => serde_json::to_string(&val.to_string_value()).unwrap_or_else(|_| "\"\"".to_string()),
    }
}

pub fn is_json_producing_expr(expr: &crate::ast::Expr) -> bool {
    match expr {
        crate::ast::Expr::FunctionCall { name, .. } => {
            let u = name.to_ascii_uppercase();
            matches!(
                u.as_str(),
                "JSON_OBJECT"
                    | "__JSON_OBJECT_NULL"
                    | "__JSON_OBJECT_ABSENT"
                    | "JSON_ARRAY"
                    | "__JSON_ARRAY_NULL"
                    | "__JSON_ARRAY_ABSENT"
                    | "JSON_QUERY"
            )
        }
        crate::ast::Expr::Subquery(sub) => sub.for_json().is_some(),
        _ => false,
    }
}

pub fn build_json_object(pairs: &[(String, Value, bool)], absent_on_null: bool) -> Value {
    let mut out = String::from("{");
    let mut first = true;
    for (k, v, raw) in pairs {
        if absent_on_null && v.is_null() {
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&serde_json::to_string(k).unwrap_or_else(|_| "\"\"".to_string()));
        out.push(':');
        out.push_str(&sql_value_to_json_fragment(v, *raw));
    }
    out.push('}');
    Value::NVarChar(out)
}

pub fn build_json_array(items: &[(Value, bool)], null_on_null: bool) -> Value {
    let mut out = String::from("[");
    let mut first = true;
    for (v, raw) in items {
        if !null_on_null && v.is_null() {
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&sql_value_to_json_fragment(v, *raw));
    }
    out.push(']');
    Value::NVarChar(out)
}

pub fn json_value(json_str: &str, path: &str) -> Result<Value, DbError> {
    let json = parse_json(json_str)?;
    let normalized_path = normalize_json_path(path);

    let result = json_value_at_path(&json, &normalized_path);
    match result {
        Some(v) => Ok(Value::NVarChar(v.to_string().trim_matches('"').to_string())),
        None => Ok(Value::Null),
    }
}

pub fn json_query(json_str: &str, path: &str) -> Result<Value, DbError> {
    let json = parse_json(json_str)?;
    let normalized_path = normalize_json_path(path);

    let result = json_value_at_path(&json, &normalized_path);
    match result {
        Some(v) => Ok(Value::NVarChar(json_to_string(v))),
        None => Ok(Value::Null),
    }
}

pub fn json_modify(json_str: &str, path: &str, new_value: &str) -> Result<Value, DbError> {
    let mut json = parse_json(json_str)?;
    let normalized_path = normalize_json_path(path);

    if let Ok(new_json) = parse_json(new_value) {
        set_json_value(&mut json, &normalized_path, new_json);
    } else {
        set_json_value(
            &mut json,
            &normalized_path,
            JsonValue::String(new_value.to_string()),
        );
    }

    Ok(Value::NVarChar(json_to_string(&json)))
}

pub fn is_json(json_str: &str) -> Result<Value, DbError> {
    let is_valid = serde_json::from_str::<JsonValue>(json_str).is_ok();
    Ok(Value::Bit(is_valid))
}

pub fn is_json_typed(json_str: &str, json_type: &str) -> Result<Value, DbError> {
    let parsed = match serde_json::from_str::<JsonValue>(json_str) {
        Ok(v) => v,
        Err(_) => return Ok(Value::Bit(false)),
    };
    let matches = match json_type.trim().to_ascii_uppercase().as_str() {
        "VALUE" => true,
        "ARRAY" => parsed.is_array(),
        "OBJECT" => parsed.is_object(),
        "SCALAR" => {
            parsed.is_string() || parsed.is_number() || parsed.is_boolean() || parsed.is_null()
        }
        other => {
            return Err(DbError::Execution(format!(
                "Invalid JSON type constraint '{}' for ISJSON (expected VALUE, ARRAY, OBJECT, or SCALAR)",
                other
            )));
        }
    };
    Ok(Value::Bit(matches))
}

pub fn json_path_exists(json_str: &str, path: &str) -> Result<Value, DbError> {
    let json = parse_json(json_str)?;
    let normalized_path = normalize_json_path(path);

    let exists = json_value_at_path(&json, &normalized_path).is_some();
    Ok(Value::Bit(exists))
}

/// Evaluates SQL Server 2025 `JSON_CONTAINS(target, search_value [, path [, search_mode]])`.
/// - `path` defaults to `"$"` and supports object keys, `[idx]`, and `[*]` wildcards.
/// - `search_mode`: `0` (default) uses equality, `1` uses SQL `LIKE` pattern matching on strings.
pub fn json_contains(
    json_str: &str,
    search_val: &Value,
    path: Option<&str>,
    search_mode: i64,
) -> Result<Value, DbError> {
    let root = parse_json(json_str)?;
    let path_str = path.unwrap_or("$").trim();
    let targets = collect_json_path_matches(&root, path_str)?;
    if targets.is_empty() {
        // SQL Server 2025: returns NULL if an explicit non-root path doesn't match any element,
        // or 0 if wildcard matched empty array.
        return if path_str.contains("[*]") {
            Ok(Value::Int(0))
        } else {
            Ok(Value::Null)
        };
    }

    let use_like = match search_mode {
        0 => false,
        1 => true,
        other => {
            return Err(DbError::Execution(format!(
                "Invalid search_mode {} for JSON_CONTAINS (expected 0 or 1)",
                other
            )))
        }
    };

    let matched = targets
        .iter()
        .any(|node| json_node_contains_value(node, search_val, use_like));
    Ok(Value::Int(if matched { 1 } else { 0 }))
}

fn collect_json_path_matches<'a>(
    root: &'a JsonValue,
    path: &str,
) -> Result<Vec<&'a JsonValue>, DbError> {
    let p = path
        .trim()
        .strip_prefix("lax ")
        .or_else(|| path.trim().strip_prefix("strict "))
        .unwrap_or(path.trim())
        .trim();
    if p == "$" || p.is_empty() {
        return Ok(vec![root]);
    }
    let rest = p
        .strip_prefix('$')
        .ok_or_else(|| DbError::Execution(format!("Invalid JSON path '{}'", path)))?;

    let mut current: Vec<&'a JsonValue> = vec![root];
    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '.' => {
                i += 1;
                if i < chars.len() && chars[i] == '"' {
                    i += 1;
                    let start = i;
                    while i < chars.len() && chars[i] != '"' {
                        i += 1;
                    }
                    let key: String = chars[start..i].iter().collect();
                    if i < chars.len() && chars[i] == '"' {
                        i += 1;
                    }
                    current = current
                        .into_iter()
                        .filter_map(|n| n.as_object()?.get(&key))
                        .collect();
                } else {
                    let start = i;
                    while i < chars.len() && chars[i] != '.' && chars[i] != '[' {
                        i += 1;
                    }
                    let key: String = chars[start..i].iter().collect();
                    if key == "*" {
                        current = current
                            .into_iter()
                            .flat_map(|n| {
                                n.as_object()
                                    .map(|m| m.values().collect::<Vec<_>>())
                                    .unwrap_or_default()
                            })
                            .collect();
                    } else {
                        current = current
                            .into_iter()
                            .filter_map(|n| n.as_object()?.get(&key))
                            .collect();
                    }
                }
            }
            '[' => {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != ']' {
                    i += 1;
                }
                let inner: String = chars[start..i].iter().collect();
                if i < chars.len() && chars[i] == ']' {
                    i += 1;
                }
                let inner_trim = inner.trim();
                if inner_trim == "*" {
                    current = current
                        .into_iter()
                        .flat_map(|n| {
                            n.as_array()
                                .map(|a| a.iter().collect::<Vec<_>>())
                                .unwrap_or_default()
                        })
                        .collect();
                } else if let Ok(idx) = inner_trim.parse::<usize>() {
                    current = current
                        .into_iter()
                        .filter_map(|n| n.as_array()?.get(idx))
                        .collect();
                } else {
                    return Err(DbError::Execution(format!("Invalid JSON path '{}'", path)));
                }
            }
            _ => {
                return Err(DbError::Execution(format!("Invalid JSON path '{}'", path)));
            }
        }
    }
    Ok(current)
}

fn json_scalar_matches(node: &JsonValue, search: &Value, use_like: bool) -> bool {
    match node {
        JsonValue::String(s) => {
            let needle = search.to_string_value();
            if use_like {
                super::predicates::like_match_default(s, &needle)
            } else {
                s == &needle
            }
        }
        JsonValue::Number(num) => {
            if let Some(target_f) = search.to_f64() {
                if let Some(nf) = num.as_f64() {
                    return (nf - target_f).abs() < 1e-12;
                }
            }
            false
        }
        JsonValue::Bool(b) => match search {
            Value::Bit(sb) => sb == b,
            _ => false,
        },
        JsonValue::Null => search.is_null(),
        _ => false,
    }
}

fn json_node_contains_value(node: &JsonValue, search: &Value, use_like: bool) -> bool {
    match node {
        JsonValue::Array(arr) => arr
            .iter()
            .any(|el| json_node_contains_value(el, search, use_like)),
        JsonValue::Object(map) => map
            .values()
            .any(|v| json_node_contains_value(v, search, use_like)),
        scalar => json_scalar_matches(scalar, search, use_like),
    }
}

pub fn json_array_length(json_str: &str) -> Result<Value, DbError> {
    let json = parse_json(json_str)?;
    match json {
        JsonValue::Array(arr) => Ok(Value::Int(arr.len() as i32)),
        _ => Ok(Value::Null),
    }
}

pub fn json_keys(json_str: &str, path: Option<&str>) -> Result<Value, DbError> {
    let json = parse_json(json_str)?;
    let target = if let Some(p) = path {
        let normalized = normalize_json_path(p);
        match json_value_at_path(&json, &normalized) {
            Some(v) => v.clone(),
            None => return Ok(Value::Null),
        }
    } else {
        json
    };

    match target {
        JsonValue::Object(map) => {
            let keys: Vec<String> = map.keys().map(|k| format!("\"{}\"", k)).collect();
            Ok(Value::NVarChar(format!("[{}]", keys.join(","))))
        }
        _ => Ok(Value::Null),
    }
}

pub fn json_extract(json_str: &str, path: &str) -> Result<Value, DbError> {
    let json = parse_json(json_str)?;
    let normalized_path = normalize_json_path(path);

    match json_value_at_path(&json, &normalized_path) {
        Some(v) => Ok(Value::NVarChar(json_to_string(v))),
        None => Ok(Value::Null),
    }
}

fn normalize_json_path(path: &str) -> String {
    path.trim_start_matches('$')
        .trim_start_matches('.')
        .to_string()
}

fn json_value_at_path<'a>(json: &'a JsonValue, path: &str) -> Option<&'a JsonValue> {
    if path.is_empty() {
        return Some(json);
    }

    let parts: Vec<&str> = path.split('.').collect();
    let mut current = json;

    for part in parts {
        let (key, index) = parse_path_part(part);

        if let Some(idx) = index {
            match current {
                JsonValue::Array(arr) => {
                    if idx < arr.len() {
                        current = &arr[idx];
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }
        } else {
            match current {
                JsonValue::Object(map) => {
                    if let Some(val) = map.get(&key) {
                        current = val;
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }
        }
    }

    Some(current)
}

fn parse_path_part(part: &str) -> (String, Option<usize>) {
    if part.ends_with(']') {
        if let Some(bracket_pos) = part.find('[') {
            let key = part[..bracket_pos].to_string();
            let index_str = &part[bracket_pos + 1..part.len() - 1];
            if let Ok(idx) = index_str.parse::<usize>() {
                return (key, Some(idx));
            }
        }
    }
    (part.to_string(), None)
}

fn set_json_value(json: &mut JsonValue, path: &str, new_value: JsonValue) {
    if path.is_empty() {
        *json = new_value;
        return;
    }

    let parts: Vec<&str> = path.split('.').collect();
    let last_idx = parts.len() - 1;
    let mut current = json;

    for (i, part) in parts.iter().enumerate() {
        let (key, index) = parse_path_part(part);

        if i == last_idx {
            // Last part - set the value
            if let Some(idx) = index {
                if let JsonValue::Array(arr) = current {
                    if idx >= arr.len() {
                        arr.resize(idx + 1, JsonValue::Null);
                    }
                    arr[idx] = new_value.clone();
                }
            } else if let JsonValue::Object(map) = current {
                map.insert(key, new_value.clone());
            }
        } else {
            // Navigate to next level
            if let Some(idx) = index {
                if let JsonValue::Array(arr) = current {
                    if idx >= arr.len() {
                        arr.resize(idx + 1, JsonValue::Null);
                    }
                    current = &mut arr[idx];
                }
            } else if let JsonValue::Object(map) = current {
                current = if map.contains_key(&key) {
                    map.get_mut(&key).unwrap()
                } else {
                    let next_part = &parts[i + 1];
                    let (_, next_index) = parse_path_part(next_part);
                    let default = if next_index.is_some() {
                        JsonValue::Array(vec![])
                    } else {
                        JsonValue::Object(serde_json::Map::new())
                    };
                    map.entry(key).or_insert(default)
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_value_simple() {
        let json = r#"{"name": "test", "value": 42}"#;
        let result = json_value(json, "$.name").unwrap();
        assert_eq!(result, Value::NVarChar("test".to_string()));
    }

    #[test]
    fn test_json_query_array() {
        let json = r#"[1, 2, 3]"#;
        let result = json_query(json, "$[0]").unwrap();
        assert_eq!(result, Value::NVarChar("1".to_string()));
    }

    #[test]
    fn test_is_json() {
        assert_eq!(is_json(r#"{"valid": true}"#).unwrap(), Value::Bit(true));
        assert_eq!(is_json("not json").unwrap(), Value::Bit(false));
    }

    #[test]
    fn test_json_modify() {
        let json = r#"{"name": "old"}"#;
        let result = json_modify(json, "name", "new").unwrap();
        assert!(matches!(result, Value::NVarChar(_)));
    }
}
