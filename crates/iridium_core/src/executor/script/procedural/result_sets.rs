use std::sync::Arc;

use parking_lot::Mutex;

use crate::ast::{ExecResultSets, ResultSetDefinition};
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::executor::context::ExecutionContext;
use crate::executor::result::QueryResult;
use crate::executor::type_mapping::resolve_data_type_spec;
use crate::executor::value_ops::coerce_value_to_type_with_dateformat;
use crate::types::{DataType, Value};

fn truncate_result_string(val: Value, ty: &DataType) -> Value {
    let max_len = match ty {
        DataType::VarChar { max_len } | DataType::NVarChar { max_len } if *max_len < u16::MAX => {
            Some(*max_len as usize)
        }
        DataType::Char { len } | DataType::NChar { len } => Some(*len as usize),
        _ => None,
    };
    let Some(limit) = max_len else {
        return val;
    };
    match val {
        Value::VarChar(s) => Value::VarChar(s.chars().take(limit).collect()),
        Value::NVarChar(s) => Value::NVarChar(s.chars().take(limit).collect()),
        Value::Char(s) => Value::Char(s.chars().take(limit).collect()),
        Value::NChar(s) => Value::NChar(s.chars().take(limit).collect()),
        other => other,
    }
}

fn resolve_result_set_columns(
    def: &ResultSetDefinition,
    catalog: &dyn Catalog,
) -> Result<Option<Vec<(String, DataType, Option<bool>)>>, DbError> {
    match def {
        ResultSetDefinition::Columns(cols) => {
            let mut out = Vec::with_capacity(cols.len());
            for col in cols {
                let ty = resolve_data_type_spec(&col.data_type, catalog);
                out.push((col.name.clone(), ty, col.nullable));
            }
            Ok(Some(out))
        }
        ResultSetDefinition::AsObject(obj) => {
            let schema = obj.schema_or_dbo();
            if let Some(tbl) = catalog.find_table(schema, &obj.name) {
                let cols = tbl
                    .columns
                    .iter()
                    .map(|c| (c.name.clone(), c.data_type.clone(), Some(c.nullable)))
                    .collect();
                return Ok(Some(cols));
            }
            if let Some(view) = catalog.find_view(schema, &obj.name) {
                let mut cols = Vec::new();
                if let crate::ast::Statement::Dml(crate::ast::DmlStatement::Select(sel)) =
                    view.query.as_ref()
                {
                    for (idx, item) in sel.projection.iter().enumerate() {
                        if matches!(
                            item.expr,
                            crate::ast::Expr::Wildcard | crate::ast::Expr::QualifiedWildcard(_)
                        ) {
                            return Ok(None);
                        }
                        let name = item
                            .alias
                            .clone()
                            .or_else(|| match &item.expr {
                                crate::ast::Expr::Identifier(id) => Some(id.clone()),
                                crate::ast::Expr::QualifiedIdentifier(parts) => {
                                    parts.last().cloned()
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| format!("col_{}", idx + 1));
                        cols.push((name, DataType::NVarChar { max_len: u16::MAX }, Some(true)));
                    }
                }
                if !cols.is_empty() {
                    return Ok(Some(cols));
                }
            }
            Err(DbError::Execution(format!(
                "Invalid object name '{}.{}' in WITH RESULT SETS AS OBJECT.",
                schema, obj.name
            )))
        }
        ResultSetDefinition::AsType(obj) => {
            let schema = obj.schema_or_dbo();
            let Some(tdef) = catalog.find_table_type(schema, &obj.name) else {
                return Err(DbError::Execution(format!(
                    "Invalid table type '{}.{}' in WITH RESULT SETS AS TYPE.",
                    schema, obj.name
                )));
            };
            let cols = tdef
                .columns
                .iter()
                .map(|c| {
                    let ty = resolve_data_type_spec(&c.data_type, catalog);
                    (c.name.clone(), ty, Some(c.nullable))
                })
                .collect();
            Ok(Some(cols))
        }
        ResultSetDefinition::AsForXml => Ok(None),
    }
}

pub(crate) fn reshape_result_sets(
    captured: Vec<QueryResult>,
    clause: &ExecResultSets,
    catalog: &dyn Catalog,
    dateformat: &str,
) -> Result<Vec<QueryResult>, DbError> {
    match clause {
        ExecResultSets::Undefined => Ok(captured),
        ExecResultSets::None => {
            if !captured.is_empty() {
                return Err(DbError::Execution(
                    "EXECUTE statement failed because its WITH RESULT SETS clause specified 0 result set(s), and the statement tried to send more result sets than this."
                        .into(),
                ));
            }
            Ok(Vec::new())
        }
        ExecResultSets::Definitions(defs) => {
            if captured.len() > defs.len() {
                return Err(DbError::Execution(format!(
                    "EXECUTE statement failed because its WITH RESULT SETS clause specified {} result set(s), and the statement tried to send more result sets than this.",
                    defs.len()
                )));
            }
            let mut reshaped = Vec::with_capacity(captured.len());
            for (idx, (mut qr, def)) in captured.into_iter().zip(defs.iter()).enumerate() {
                let rs_num = idx + 1;
                let Some(expected_cols) = resolve_result_set_columns(def, catalog)? else {
                    // AS FOR XML expects a single column
                    if qr.columns.len() != 1 {
                        return Err(DbError::Execution(format!(
                            "EXECUTE statement failed because its WITH RESULT SETS clause specified 1 column(s) for result set number {}, but the statement sent {} column(s) at run time.",
                            rs_num,
                            qr.columns.len()
                        )));
                    }
                    reshaped.push(qr);
                    continue;
                };
                if qr.columns.len() != expected_cols.len() {
                    return Err(DbError::Execution(format!(
                        "EXECUTE statement failed because its WITH RESULT SETS clause specified {} column(s) for result set number {}, but the statement sent {} column(s) at run time.",
                        expected_cols.len(),
                        rs_num,
                        qr.columns.len()
                    )));
                }
                qr.columns = expected_cols.iter().map(|(n, _, _)| n.clone()).collect();
                qr.column_types = expected_cols.iter().map(|(_, ty, _)| ty.clone()).collect();
                qr.column_nullabilities = expected_cols
                    .iter()
                    .map(|(_, _, nul)| nul.unwrap_or(true))
                    .collect();

                for row in &mut qr.rows {
                    for (col_idx, (col_name, col_ty, col_nul)) in expected_cols.iter().enumerate() {
                        if let Some(cell) = row.get_mut(col_idx) {
                            let val = std::mem::replace(cell, Value::Null);
                            let coerced = truncate_result_string(
                                coerce_value_to_type_with_dateformat(val, col_ty, dateformat)?,
                                col_ty,
                            );
                            if *col_nul == Some(false) && coerced.is_null() {
                                return Err(DbError::Execution(format!(
                                    "EXECUTE statement failed because its WITH RESULT SETS clause specified NOT NULL for column '{}' in result set number {}, but a NULL value was returned.",
                                    col_name, rs_num
                                )));
                            }
                            *cell = coerced;
                        }
                    }
                }
                reshaped.push(qr);
            }
            Ok(reshaped)
        }
    }
}

pub(crate) struct ResultSetCapture {
    saved_sink: Option<Arc<Mutex<Vec<QueryResult>>>>,
    local_sink: Arc<Mutex<Vec<QueryResult>>>,
}

pub(crate) fn begin_result_set_capture(
    clause: Option<&ExecResultSets>,
    ctx: &mut ExecutionContext<'_>,
) -> Option<ResultSetCapture> {
    let clause = clause?;
    if matches!(clause, ExecResultSets::Undefined) {
        return None;
    }
    let local_sink = Arc::new(Mutex::new(Vec::new()));
    let saved_sink = ctx.frame.result_set_sink.replace(local_sink.clone());
    Some(ResultSetCapture {
        saved_sink,
        local_sink,
    })
}

pub(crate) fn abort_result_set_capture(
    capture: Option<ResultSetCapture>,
    ctx: &mut ExecutionContext<'_>,
) {
    if let Some(cap) = capture {
        ctx.frame.result_set_sink = cap.saved_sink;
    }
}

pub(crate) fn finish_result_set_capture(
    capture: Option<ResultSetCapture>,
    clause: Option<&ExecResultSets>,
    raw_opt: Option<QueryResult>,
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
) -> Result<Option<QueryResult>, DbError> {
    let Some(cap) = capture else {
        return Ok(raw_opt);
    };
    ctx.frame.result_set_sink = cap.saved_sink;
    let Some(clause) = clause else {
        return Ok(raw_opt);
    };

    let mut captured: Vec<QueryResult> = cap.local_sink.lock().drain(..).collect();
    if captured.is_empty() {
        if let Some(ref qr) = raw_opt {
            if !qr.columns.is_empty() {
                captured.push(qr.clone());
            }
        }
    }

    let reshaped = reshape_result_sets(captured, clause, catalog, &ctx.options.dateformat)?;
    if let Some(ref outer_sink) = ctx.frame.result_set_sink {
        outer_sink.lock().extend(reshaped.iter().cloned());
    }

    let return_status = raw_opt.as_ref().and_then(|r| r.return_status);
    let is_procedure = raw_opt.as_ref().is_some_and(|r| r.is_procedure);
    if let Some(mut last) = reshaped.into_iter().last() {
        last.return_status = return_status;
        last.is_procedure = is_procedure;
        Ok(Some(last))
    } else if return_status.is_some() || is_procedure {
        Ok(Some(QueryResult {
            return_status,
            is_procedure,
            ..Default::default()
        }))
    } else {
        Ok(None)
    }
}
