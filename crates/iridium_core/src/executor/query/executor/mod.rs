mod select;

use crate::error::DbError;
use crate::executor::result::QueryResult;

use super::plan::RelationalQuery;
use super::source;
use super::QueryExecutor;
use crate::executor::context::ExecutionContext;
use crate::executor::model::JoinedRow;

impl<'a> QueryExecutor<'a> {
    pub fn execute_select(
        &self,
        query: RelationalQuery,
        ctx: &mut ExecutionContext,
    ) -> Result<QueryResult, DbError> {
        let set_op = query.set_op.clone();
        let into_table = query.into_table.clone();
        let for_xml = query.for_xml.clone();
        let for_json = query.for_json.clone();
        let result = select::execute_select_internal(self, &query, ctx)?;
        let mut result = result;

        if let Some(set_op) = set_op {
            let right = self.execute_select(RelationalQuery::from(set_op.right), ctx)?;
            result = crate::executor::engine::execute_set_op(result, right, set_op.kind)?;
        }

        if ctx.options.rowcount > 0 && result.rows.len() > ctx.options.rowcount as usize {
            result.rows.truncate(ctx.options.rowcount as usize);
        }

        if let Some(fx) = for_xml {
            result = format_for_xml_result(&result, &query.projection.items, &fx);
        } else if let Some(fj) = for_json {
            result = format_for_json_result(&result, &query.projection.items, &fj);
        }

        if into_table.is_some() {
            return Err(DbError::Execution(
                "SELECT INTO is handled by ScriptExecutor".into(),
            ));
        }

        Ok(result)
    }

    pub fn execute_to_joined_rows(
        &self,
        query: RelationalQuery,
        ctx: &mut ExecutionContext,
    ) -> Result<Vec<JoinedRow>, DbError> {
        let mut source_eval = source::execute_source(self, &query, ctx)?;
        source_eval.materialize(ctx, self.catalog, self.storage, self.clock)
    }

    /// Executes a simple non-grouped, non-distinct SELECT and also captures
    /// the underlying base-table row indices for each output row so that
    /// updatable CTEs (`WITH cte AS (...) UPDATE/DELETE cte`) can map rows
    /// back to storage.
    pub fn execute_select_with_base_targets(
        &self,
        query: RelationalQuery,
        ctx: &mut ExecutionContext,
    ) -> Result<(QueryResult, Vec<crate::executor::cte::CteBaseTarget>), DbError> {
        let can_track = query.set_op.is_none()
            && query.into_table.is_none()
            && query.for_xml.is_none()
            && query.for_json.is_none()
            && query.filter.group_by.is_empty()
            && !query.projection.distinct
            && query.sort.order_by.is_empty()
            && query.pagination.top.is_none()
            && query.pagination.offset.is_none()
            && query.pagination.fetch.is_none();

        if !can_track {
            let res = self.execute_select(query, ctx)?;
            return Ok((res, Vec::new()));
        }

        super::from_tree::enforce_query_governor_cost_limit(&query, ctx)?;
        let mut source_eval = source::execute_source(self, &query, ctx)?;
        let source_shape = source_eval.shape.clone();
        let source_rows = source_eval.materialize(ctx, self.catalog, self.storage, self.clock)?;

        // Collect base-table storage indices aligned 1:1 with source_rows before projection.
        let mut base_targets: Vec<crate::executor::cte::CteBaseTarget> = Vec::new();
        for (pos, ct) in source_shape.iter().enumerate() {
            if ct.table.id > 0 {
                let row_indices = source_rows
                    .iter()
                    .map(|jr| jr.get(pos).and_then(|c| c.storage_index))
                    .collect();
                base_targets.push(crate::executor::cte::CteBaseTarget {
                    table_def: ct.table.as_ref().clone(),
                    row_indices,
                });
            }
        }

        let shape_opt = if source_shape.is_empty() {
            None
        } else {
            Some(&source_shape)
        };
        let mut result = super::finalize::finalize_rows(self, &query, source_rows, shape_opt, ctx)?;
        if ctx.options.rowcount > 0 && result.rows.len() > ctx.options.rowcount as usize {
            result.rows.truncate(ctx.options.rowcount as usize);
            for bt in &mut base_targets {
                bt.row_indices.truncate(result.rows.len());
            }
        }
        // Only expose base targets when row count stayed 1:1 (no implicit aggregate collapse).
        if base_targets
            .first()
            .is_some_and(|bt| bt.row_indices.len() != result.rows.len())
        {
            base_targets.clear();
        }
        Ok((result, base_targets))
    }
}

fn format_for_xml_result(
    result: &QueryResult,
    projection_items: &[crate::ast::SelectItem],
    for_xml: &crate::ast::ForXmlClause,
) -> QueryResult {
    let xml_col = "XML_F52E2B61-18A1-11d1-B105-00805F49916B".to_string();
    if result.rows.is_empty() && for_xml.root.is_none() {
        return QueryResult {
            columns: vec![xml_col],
            column_types: vec![crate::types::DataType::NVarChar { max_len: 0 }],
            column_nullabilities: vec![true],
            rows: Vec::new(),
            ..Default::default()
        };
    }

    let row_tag = match for_xml.path.as_deref() {
        Some(p) => p,
        None if for_xml.mode == "PATH" || for_xml.mode == "RAW" => "row",
        None => "",
    };

    let mut xml = String::new();
    if let Some(ref root) = for_xml.root {
        xml.push('<');
        xml.push_str(root);
        xml.push('>');
    }
    for row in &result.rows {
        let is_raw_mode = for_xml.mode.eq_ignore_ascii_case("RAW");
        let mut attrs = Vec::new();
        let mut children = Vec::new();
        for (i, val) in row.iter().enumerate() {
            if matches!(val, crate::types::Value::Null) {
                continue;
            }
            let text = val.to_string_value();
            let is_unnamed_expr = projection_items.get(i).is_some_and(|item| {
                item.alias.is_none()
                    && !matches!(
                        item.expr,
                        crate::ast::Expr::Identifier(_) | crate::ast::Expr::QualifiedIdentifier(_)
                    )
            });
            let col_name = result.columns.get(i).map(String::as_str).unwrap_or("");
            if let Some(attr_name) = col_name.strip_prefix('@') {
                attrs.push((attr_name.to_string(), text));
            } else if is_raw_mode && !is_unnamed_expr && !col_name.is_empty() {
                attrs.push((col_name.to_string(), text));
            } else {
                let is_raw_text = is_unnamed_expr
                    || col_name.is_empty()
                    || col_name.eq_ignore_ascii_case("text()");
                if is_raw_text {
                    children.push(text);
                } else {
                    children.push(format!("<{col_name}>{text}</{col_name}>"));
                }
            }
        }
        if !row_tag.is_empty() {
            xml.push('<');
            xml.push_str(row_tag);
            for (k, v) in &attrs {
                let escaped = v
                    .replace('&', "&amp;")
                    .replace('"', "&quot;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                xml.push(' ');
                xml.push_str(k);
                xml.push_str("=\"");
                xml.push_str(&escaped);
                xml.push('"');
            }
            if children.is_empty() {
                xml.push_str("/>");
            } else {
                xml.push('>');
                for child in &children {
                    xml.push_str(child);
                }
                xml.push_str("</");
                xml.push_str(row_tag);
                xml.push('>');
            }
        } else {
            for child in &children {
                xml.push_str(child);
            }
        }
    }
    if let Some(ref root) = for_xml.root {
        xml.push_str("</");
        xml.push_str(root);
        xml.push('>');
    }

    QueryResult {
        columns: vec![xml_col],
        column_types: vec![crate::types::DataType::NVarChar { max_len: 0 }],
        column_nullabilities: vec![true],
        rows: vec![vec![crate::types::Value::VarChar(xml)]],
        ..Default::default()
    }
}

fn value_to_for_json(val: &crate::types::Value) -> serde_json::Value {
    match val {
        crate::types::Value::Null => serde_json::Value::Null,
        crate::types::Value::Bit(b) => serde_json::Value::Bool(*b),
        crate::types::Value::TinyInt(n) => serde_json::Value::Number((*n).into()),
        crate::types::Value::SmallInt(n) => serde_json::Value::Number((*n).into()),
        crate::types::Value::Int(n) => serde_json::Value::Number((*n).into()),
        crate::types::Value::BigInt(n) => serde_json::Value::Number((*n).into()),
        crate::types::Value::Float(bits) => {
            let f = f64::from_bits(*bits);
            serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null)
        }
        crate::types::Value::Decimal(unscaled, scale) => {
            if *scale == 0 {
                serde_json::from_str::<serde_json::Number>(&unscaled.to_string())
                    .map(serde_json::Value::Number)
                    .unwrap_or_else(|_| serde_json::Value::String(val.to_string_value()))
            } else {
                let factor = 10i128.pow(*scale as u32);
                let sign = if *unscaled < 0 { "-" } else { "" };
                let abs = unscaled.unsigned_abs();
                let int_part = abs / (factor as u128);
                let frac_part = abs % (factor as u128);
                let raw = format!(
                    "{sign}{int_part}.{frac_part:0width$}",
                    width = *scale as usize
                );
                let trimmed = raw.trim_end_matches('0').trim_end_matches('.');
                let normalized = if trimmed.is_empty() || trimmed == "-" {
                    "0"
                } else {
                    trimmed
                };
                serde_json::from_str::<serde_json::Number>(normalized)
                    .map(serde_json::Value::Number)
                    .unwrap_or_else(|_| serde_json::Value::String(raw))
            }
        }
        other => serde_json::Value::String(other.to_string_value()),
    }
}

fn insert_json_nested_path(
    map: &mut serde_json::Map<String, serde_json::Value>,
    segments: &[&str],
    val: serde_json::Value,
) {
    let Some((first, rest)) = segments.split_first() else {
        return;
    };
    if rest.is_empty() {
        map.insert((*first).to_string(), val);
        return;
    }
    let entry = map
        .entry((*first).to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if !entry.is_object() {
        *entry = serde_json::Value::Object(serde_json::Map::new());
    }
    if let serde_json::Value::Object(child_map) = entry {
        insert_json_nested_path(child_map, rest, val);
    }
}

fn expr_produces_raw_json(expr: &crate::ast::Expr) -> bool {
    match expr {
        crate::ast::Expr::Subquery(sub) => sub.for_json().is_some(),
        crate::ast::Expr::FunctionCall { name, args, .. } => {
            if name.eq_ignore_ascii_case("JSON_QUERY") {
                true
            } else if name.eq_ignore_ascii_case("ISNULL") || name.eq_ignore_ascii_case("COALESCE") {
                args.iter().any(expr_produces_raw_json)
            } else {
                false
            }
        }
        _ => false,
    }
}

fn format_for_json_result(
    result: &QueryResult,
    projection: &[crate::ast::SelectItem],
    for_json: &crate::ast::ForJsonClause,
) -> QueryResult {
    let json_col = "JSON_F52E2B61-18A1-11d1-B105-00805F49916B".to_string();
    if result.rows.is_empty() && for_json.root.is_none() {
        return QueryResult {
            columns: vec![json_col],
            column_types: vec![crate::types::DataType::NVarChar { max_len: 0 }],
            column_nullabilities: vec![true],
            rows: Vec::new(),
            ..Default::default()
        };
    }

    let raw_json_cols: Vec<bool> = (0..result.columns.len())
        .map(|i| {
            projection
                .get(i)
                .is_some_and(|item| expr_produces_raw_json(&item.expr))
        })
        .collect();

    let is_path_mode = for_json.mode.eq_ignore_ascii_case("PATH");
    let mut objects: Vec<serde_json::Value> = Vec::with_capacity(result.rows.len());

    for row in &result.rows {
        let mut obj = serde_json::Map::new();
        for (i, val) in row.iter().enumerate() {
            if matches!(val, crate::types::Value::Null) && !for_json.include_null_values {
                continue;
            }
            let col_name = result
                .columns
                .get(i)
                .map(String::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("col");
            let jv = if raw_json_cols.get(i).copied().unwrap_or(false) {
                match val {
                    crate::types::Value::VarChar(s) | crate::types::Value::NVarChar(s) => {
                        serde_json::from_str::<serde_json::Value>(s)
                            .unwrap_or_else(|_| value_to_for_json(val))
                    }
                    _ => value_to_for_json(val),
                }
            } else {
                value_to_for_json(val)
            };
            if is_path_mode && col_name.contains('.') {
                let segments: Vec<&str> = col_name.split('.').collect();
                insert_json_nested_path(&mut obj, &segments, jv);
            } else {
                obj.insert(col_name.to_string(), jv);
            }
        }
        objects.push(serde_json::Value::Object(obj));
    }

    let json_text = if let Some(ref root_name) = for_json.root {
        let mut root_map = serde_json::Map::new();
        root_map.insert(root_name.clone(), serde_json::Value::Array(objects));
        serde_json::to_string(&serde_json::Value::Object(root_map)).unwrap_or_default()
    } else if for_json.without_array_wrapper {
        objects
            .iter()
            .map(|o| serde_json::to_string(o).unwrap_or_default())
            .collect::<Vec<_>>()
            .join(",")
    } else {
        serde_json::to_string(&serde_json::Value::Array(objects)).unwrap_or_default()
    };

    QueryResult {
        columns: vec![json_col],
        column_types: vec![crate::types::DataType::NVarChar { max_len: 0 }],
        column_nullabilities: vec![true],
        rows: vec![vec![crate::types::Value::NVarChar(json_text)]],
        ..Default::default()
    }
}
