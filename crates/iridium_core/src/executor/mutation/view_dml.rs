use crate::ast::{
    BinaryOp, DmlStatement, Expr, FromNode, ObjectName, SelectStmt, Statement, TableFactor,
};
use crate::catalog::{Catalog, TableDef};
use crate::error::DbError;
use crate::executor::clock::Clock;
use crate::executor::context::ExecutionContext;
use crate::executor::evaluator::eval_expr;
use crate::executor::model::single_row_context_arc;
use crate::storage::{Storage, StoredRow};
use crate::types::Value;

/// Resolved target of a single-table updatable view (including nested updatable views).
#[derive(Debug, Clone)]
pub(crate) struct UpdatableViewTarget {
    pub(crate) base_table: ObjectName,
    pub(crate) base_table_def: TableDef,
    /// Ordered list of `(view_column_name, Some(base_column_name))` (`None` if derived/constant).
    pub(crate) columns: Vec<(String, Option<String>)>,
    /// `WHERE` predicates from all view layers, rewritten to base-table column names.
    pub(crate) where_predicates: Vec<Expr>,
    /// Predicates enforced by `WITH CHECK OPTION` (Error 550) on INSERT/UPDATE.
    pub(crate) check_predicates: Vec<Expr>,
}

impl UpdatableViewTarget {
    pub(crate) fn map_view_column(
        &self,
        view_col: &str,
        view_name: &str,
    ) -> Result<String, DbError> {
        for (vc, bc) in &self.columns {
            if vc.eq_ignore_ascii_case(view_col) {
                return bc.clone().ok_or_else(|| DbError::Custom {
                    class: 16,
                    number: 4406,
                    message: format!(
                        "Update or insert of view or function '{}' failed because it contains a derived or constant field.",
                        view_name
                    ),
                });
            }
        }
        Err(DbError::column_not_found(view_col))
    }

    pub(crate) fn default_insert_columns(&self, view_name: &str) -> Result<Vec<String>, DbError> {
        let mut out = Vec::with_capacity(self.columns.len());
        for (_, bc) in &self.columns {
            let base_col = bc.as_ref().ok_or_else(|| DbError::Custom {
                class: 16,
                number: 4406,
                message: format!(
                    "Update or insert of view or function '{}' failed because it contains a derived or constant field.",
                    view_name
                ),
            })?;
            // Skip computed/identity/rowversion columns when column list is omitted,
            // matching table INSERT behavior.
            if let Some(col_def) = self
                .base_table_def
                .columns
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(base_col))
            {
                if col_def.computed_expr.is_some()
                    || col_def.identity.is_some()
                    || col_def.is_rowversion
                {
                    continue;
                }
            }
            out.push(base_col.clone());
        }
        Ok(out)
    }

    pub(crate) fn combined_where(&self, extra: Option<Expr>) -> Option<Expr> {
        let mut acc: Option<Expr> = None;
        for pred in &self.where_predicates {
            acc = Some(match acc {
                Some(prev) => Expr::Binary {
                    left: Box::new(prev),
                    op: BinaryOp::And,
                    right: Box::new(pred.clone()),
                },
                None => pred.clone(),
            });
        }
        if let Some(e) = extra {
            acc = Some(match acc {
                Some(prev) => Expr::Binary {
                    left: Box::new(prev),
                    op: BinaryOp::And,
                    right: Box::new(e),
                },
                None => e,
            });
        }
        acc
    }
}

fn non_updatable_error(view_name: &str) -> DbError {
    DbError::Custom {
        class: 16,
        number: 4406,
        message: format!(
            "Update or insert of view or function '{}' failed because it contains a derived or constant field or non-updatable clause.",
            view_name
        ),
    }
}

fn single_named_source(from: &FromNode) -> Option<(&ObjectName, Option<&str>)> {
    match from {
        FromNode::Table(tr) => match &tr.factor {
            TableFactor::Named(obj) => Some((obj, None)),
            _ => None,
        },
        FromNode::Aliased { source, alias } => match source.as_ref() {
            FromNode::Table(tr) => match &tr.factor {
                TableFactor::Named(obj) => Some((obj, Some(alias.as_str()))),
                _ => None,
            },
            _ => None,
        },
        FromNode::Join { .. } => None,
    }
}

/// Resolves a view into its underlying single base table, column mappings, and predicates.
/// Returns `Ok(None)` if `(schema, name)` is not a view.
pub(crate) fn resolve_updatable_view(
    catalog: &dyn Catalog,
    schema: &str,
    name: &str,
) -> Result<Option<UpdatableViewTarget>, DbError> {
    if catalog.find_view(schema, name).is_none() {
        return Ok(None);
    }
    resolve_updatable_view_depth(catalog, schema, name, 0).map(Some)
}

fn resolve_updatable_view_depth(
    catalog: &dyn Catalog,
    schema: &str,
    name: &str,
    depth: usize,
) -> Result<UpdatableViewTarget, DbError> {
    if depth > 16 {
        return Err(non_updatable_error(name));
    }
    let view = catalog
        .find_view(schema, name)
        .ok_or_else(|| DbError::view_not_found(schema, name))?;
    let sel: &SelectStmt = match view.query.as_ref() {
        Statement::Dml(DmlStatement::Select(s)) => s,
        _ => return Err(non_updatable_error(name)),
    };
    if sel.distinct
        || !sel.group_by.is_empty()
        || sel.having.is_some()
        || sel.set_op.is_some()
        || sel.top.is_some()
        || !sel.applies.is_empty()
    {
        return Err(non_updatable_error(name));
    }
    let from = sel
        .from_clause
        .as_ref()
        .ok_or_else(|| non_updatable_error(name))?;
    let (src_obj, src_alias) =
        single_named_source(from).ok_or_else(|| non_updatable_error(name))?;
    let src_schema = src_obj.schema_or_dbo();

    // Recursively resolve if source is another view, or resolve base table directly.
    let (base_table, base_table_def, inner_columns, mut where_predicates, mut check_predicates) =
        if let Some(tbl) = catalog.find_table(src_schema, &src_obj.name) {
            let cols: Vec<(String, Option<String>)> = tbl
                .columns
                .iter()
                .map(|c| (c.name.clone(), Some(c.name.clone())))
                .collect();
            (
                ObjectName {
                    database: None,
                    schema: Some(tbl.schema_name.clone()),
                    name: tbl.name.clone(),
                },
                tbl.clone(),
                cols,
                Vec::new(),
                Vec::new(),
            )
        } else if catalog.find_view(src_schema, &src_obj.name).is_some() {
            let inner =
                resolve_updatable_view_depth(catalog, src_schema, &src_obj.name, depth + 1)?;
            (
                inner.base_table,
                inner.base_table_def,
                inner.columns,
                inner.where_predicates,
                inner.check_predicates,
            )
        } else {
            return Err(DbError::table_not_found(src_schema, &src_obj.name));
        };

    // Map this view's projection items through `inner_columns`.
    let lookup_inner = |col_name: &str| -> Option<String> {
        inner_columns
            .iter()
            .find(|(vc, _)| vc.eq_ignore_ascii_case(col_name))
            .and_then(|(_, bc)| bc.clone())
    };
    let mut columns: Vec<(String, Option<String>)> = Vec::new();
    for item in &sel.projection {
        match &item.expr {
            Expr::Wildcard | Expr::QualifiedWildcard(_) => {
                columns.extend(inner_columns.iter().cloned());
            }
            Expr::Identifier(col) => {
                let out_name = item.alias.clone().unwrap_or_else(|| col.clone());
                columns.push((out_name, lookup_inner(col)));
            }
            Expr::QualifiedIdentifier(parts) if !parts.is_empty() => {
                let col = parts.last().unwrap();
                let out_name = item.alias.clone().unwrap_or_else(|| col.clone());
                columns.push((out_name, lookup_inner(col)));
            }
            _ => {
                let out_name = item.alias.clone().unwrap_or_default();
                columns.push((out_name, None));
            }
        }
    }

    // Rewrite this view's WHERE clause into base-table column names (stripping source alias).
    if let Some(ref pred) = sel.selection {
        let rewritten = rewrite_expr_columns(
            pred,
            &inner_columns,
            src_alias.unwrap_or(&src_obj.name),
            &base_table.name,
        );
        where_predicates.push(rewritten);
    }
    if view.with_check_option {
        // WITH CHECK OPTION enforces this view's WHERE predicate AND any inner views' predicates.
        check_predicates = where_predicates.clone();
    }

    Ok(UpdatableViewTarget {
        base_table,
        base_table_def,
        columns,
        where_predicates,
        check_predicates,
    })
}

/// Rewrites column references in `expr` using `(alias_col -> base_col)` mapping and normalizes
/// qualifier prefixes matching `source_alias` to unqualified base-table column names.
pub(crate) fn rewrite_expr_columns(
    expr: &Expr,
    col_map: &[(String, Option<String>)],
    source_alias: &str,
    _base_table_name: &str,
) -> Expr {
    let map_col = |name: &str| -> String {
        col_map
            .iter()
            .find(|(vc, _)| vc.eq_ignore_ascii_case(name))
            .and_then(|(_, bc)| bc.clone())
            .unwrap_or_else(|| name.to_string())
    };
    match expr {
        Expr::Identifier(name) => Expr::Identifier(map_col(name)),
        Expr::QualifiedIdentifier(parts) if parts.len() == 2 => {
            if parts[0].eq_ignore_ascii_case(source_alias) {
                Expr::Identifier(map_col(&parts[1]))
            } else {
                Expr::QualifiedIdentifier(vec![parts[0].clone(), map_col(&parts[1])])
            }
        }
        Expr::Binary { left, op, right } => Expr::Binary {
            left: Box::new(rewrite_expr_columns(
                left,
                col_map,
                source_alias,
                _base_table_name,
            )),
            op: *op,
            right: Box::new(rewrite_expr_columns(
                right,
                col_map,
                source_alias,
                _base_table_name,
            )),
        },
        Expr::Unary { op, expr: inner } => Expr::Unary {
            op: *op,
            expr: Box::new(rewrite_expr_columns(
                inner,
                col_map,
                source_alias,
                _base_table_name,
            )),
        },
        Expr::IsNull(inner) => Expr::IsNull(Box::new(rewrite_expr_columns(
            inner,
            col_map,
            source_alias,
            _base_table_name,
        ))),
        Expr::IsNotNull(inner) => Expr::IsNotNull(Box::new(rewrite_expr_columns(
            inner,
            col_map,
            source_alias,
            _base_table_name,
        ))),
        Expr::InList {
            expr: inner,
            list,
            negated,
        } => Expr::InList {
            expr: Box::new(rewrite_expr_columns(
                inner,
                col_map,
                source_alias,
                _base_table_name,
            )),
            list: list
                .iter()
                .map(|e| rewrite_expr_columns(e, col_map, source_alias, _base_table_name))
                .collect(),
            negated: *negated,
        },
        Expr::Between {
            expr: inner,
            low,
            high,
            negated,
        } => Expr::Between {
            expr: Box::new(rewrite_expr_columns(
                inner,
                col_map,
                source_alias,
                _base_table_name,
            )),
            low: Box::new(rewrite_expr_columns(
                low,
                col_map,
                source_alias,
                _base_table_name,
            )),
            high: Box::new(rewrite_expr_columns(
                high,
                col_map,
                source_alias,
                _base_table_name,
            )),
            negated: *negated,
        },
        Expr::Like {
            expr: inner,
            pattern,
            escape,
            negated,
        } => Expr::Like {
            expr: Box::new(rewrite_expr_columns(
                inner,
                col_map,
                source_alias,
                _base_table_name,
            )),
            pattern: Box::new(rewrite_expr_columns(
                pattern,
                col_map,
                source_alias,
                _base_table_name,
            )),
            escape: escape.as_ref().map(|e| {
                Box::new(rewrite_expr_columns(
                    e,
                    col_map,
                    source_alias,
                    _base_table_name,
                ))
            }),
            negated: *negated,
        },
        Expr::Cast {
            expr: inner,
            target,
        } => Expr::Cast {
            expr: Box::new(rewrite_expr_columns(
                inner,
                col_map,
                source_alias,
                _base_table_name,
            )),
            target: target.clone(),
        },
        Expr::TryCast {
            expr: inner,
            target,
        } => Expr::TryCast {
            expr: Box::new(rewrite_expr_columns(
                inner,
                col_map,
                source_alias,
                _base_table_name,
            )),
            target: target.clone(),
        },
        Expr::FunctionCall {
            name,
            args,
            within_group,
        } => Expr::FunctionCall {
            name: name.clone(),
            args: args
                .iter()
                .map(|a| rewrite_expr_columns(a, col_map, source_alias, _base_table_name))
                .collect(),
            within_group: within_group.clone(),
        },
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => Expr::Case {
            operand: operand.as_ref().map(|o| {
                Box::new(rewrite_expr_columns(
                    o,
                    col_map,
                    source_alias,
                    _base_table_name,
                ))
            }),
            when_clauses: when_clauses
                .iter()
                .map(|w| crate::ast::WhenClause {
                    condition: rewrite_expr_columns(
                        &w.condition,
                        col_map,
                        source_alias,
                        _base_table_name,
                    ),
                    result: rewrite_expr_columns(
                        &w.result,
                        col_map,
                        source_alias,
                        _base_table_name,
                    ),
                })
                .collect(),
            else_result: else_result.as_ref().map(|e| {
                Box::new(rewrite_expr_columns(
                    e,
                    col_map,
                    source_alias,
                    _base_table_name,
                ))
            }),
        },
        other => other.clone(),
    }
}

/// Validates every row against a view's `WITH CHECK OPTION` predicates, raising SQL Server
/// Error 550 if any row fails to qualify.
pub(crate) fn validate_view_check_option(
    table: &TableDef,
    rows: &[StoredRow],
    check_predicates: &[Expr],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<(), DbError> {
    if check_predicates.is_empty() {
        return Ok(());
    }
    let table_arc = std::sync::Arc::new(table.clone());
    let alias_arc: std::sync::Arc<str> = std::sync::Arc::from(table.name.as_str());
    for row in rows {
        let row_ctx = single_row_context_arc(
            std::sync::Arc::clone(&table_arc),
            std::sync::Arc::clone(&alias_arc),
            row.clone(),
        );
        for pred in check_predicates {
            let val = eval_expr(pred, &row_ctx, ctx, catalog, storage, clock)?;
            let passes = matches!(val, Value::Bit(true));
            if !passes {
                return Err(DbError::Custom {
                    class: 16,
                    number: 550,
                    message: "The attempted insert or update failed because the target view either specifies WITH CHECK OPTION or spans a view that specifies WITH CHECK OPTION and one or more rows resulting from the operation did not qualify under the CHECK OPTION constraint.".to_string(),
                });
            }
        }
    }
    Ok(())
}
