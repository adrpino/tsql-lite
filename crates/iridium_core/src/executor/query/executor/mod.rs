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
        if !row_tag.is_empty() {
            xml.push('<');
            xml.push_str(row_tag);
            xml.push('>');
        }
        for (i, val) in row.iter().enumerate() {
            if matches!(val, crate::types::Value::Null) {
                continue;
            }
            let text = val.to_string_value();
            let is_unnamed_expr = projection_items.get(i).is_some_and(|item| {
                item.alias.is_none()
                    && !matches!(
                        item.expr,
                        crate::ast::Expr::Identifier(_)
                            | crate::ast::Expr::QualifiedIdentifier(_)
                    )
            });
            let col_name = result.columns.get(i).map(String::as_str).unwrap_or("");
            let is_raw_text = is_unnamed_expr
                || col_name.is_empty()
                || col_name.eq_ignore_ascii_case("text()");
            if is_raw_text {
                xml.push_str(&text);
            } else {
                xml.push('<');
                xml.push_str(col_name);
                xml.push('>');
                xml.push_str(&text);
                xml.push_str("</");
                xml.push_str(col_name);
                xml.push('>');
            }
        }
        if !row_tag.is_empty() {
            xml.push_str("</");
            xml.push_str(row_tag);
            xml.push('>');
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
