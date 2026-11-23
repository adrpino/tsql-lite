use std::cmp::Ordering;
use std::collections::HashMap;

use crate::ast::Expr;
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::storage::Storage;
use crate::types::Value;

use super::aggregates::{dispatch_aggregate, Group};
use super::clock::Clock;
use super::context::ExecutionContext;
use super::evaluator::eval_expr;
use super::model::{ContextTable, JoinedRow};
use super::projection::expand_projection_columns;
use super::value_ops::{compare_values, truthy};

pub(crate) struct GroupExecutor<'a> {
    pub catalog: &'a dyn Catalog,
    pub storage: &'a dyn Storage,
    pub clock: &'a dyn Clock,
}

fn eval_having_expr(
    expr: &Expr,
    row: &[ContextTable],
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    match expr {
        Expr::FunctionCall {
            name,
            args,
            within_group,
        } => {
            if super::aggregates::is_aggregate_function(name) {
                let sorted_group;
                let effective_group =
                    if let Some(ob) = within_group.as_deref().filter(|v| !v.is_empty()) {
                        sorted_group = super::aggregates::sort_group_for_within_group(
                            group, ob, ctx, catalog, storage, clock,
                        );
                        &sorted_group
                    } else {
                        group
                    };
                if let Some(res) =
                    dispatch_aggregate(name, args, effective_group, ctx, catalog, storage, clock)
                {
                    return res;
                }
            }
            // Fallback to standard eval_expr if it's not an aggregate
            eval_expr(expr, row, ctx, catalog, storage, clock)
        }
        Expr::Binary { left, op, right } => {
            let lv = eval_having_expr(left, row, group, ctx, catalog, storage, clock)?;
            let rv = eval_having_expr(right, row, group, ctx, catalog, storage, clock)?;
            super::operators::eval_binary(
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
            let val = eval_having_expr(inner, row, group, ctx, catalog, storage, clock)?;
            super::operators::eval_unary(op, val)
        }
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => {
            let operand_val = match operand {
                Some(e) => Some(eval_having_expr(
                    e, row, group, ctx, catalog, storage, clock,
                )?),
                None => None,
            };
            for clause in when_clauses {
                let match_found = if let Some(ref op_val) = operand_val {
                    let when_val = eval_having_expr(
                        &clause.condition,
                        row,
                        group,
                        ctx,
                        catalog,
                        storage,
                        clock,
                    )?;
                    matches!(compare_values(op_val, &when_val), Ordering::Equal)
                } else {
                    let cond = eval_having_expr(
                        &clause.condition,
                        row,
                        group,
                        ctx,
                        catalog,
                        storage,
                        clock,
                    )?;
                    truthy(&cond)
                };
                if match_found {
                    return eval_having_expr(
                        &clause.result,
                        row,
                        group,
                        ctx,
                        catalog,
                        storage,
                        clock,
                    );
                }
            }
            match else_result {
                Some(expr) => eval_having_expr(expr, row, group, ctx, catalog, storage, clock),
                None => Ok(Value::Null),
            }
        }
        Expr::Between {
            expr: between_expr,
            low,
            high,
            negated,
        } => {
            let val = eval_having_expr(between_expr, row, group, ctx, catalog, storage, clock)?;
            let low_val = eval_having_expr(low, row, group, ctx, catalog, storage, clock)?;
            let high_val = eval_having_expr(high, row, group, ctx, catalog, storage, clock)?;
            if val.is_null() || low_val.is_null() || high_val.is_null() {
                return Ok(Value::Null);
            }
            let ge_low = matches!(
                compare_values(&val, &low_val),
                Ordering::Greater | Ordering::Equal
            );
            let le_high = matches!(
                compare_values(&val, &high_val),
                Ordering::Less | Ordering::Equal
            );
            let result = ge_low && le_high;
            Ok(Value::Bit(if *negated { !result } else { result }))
        }
        _ => eval_expr(expr, row, ctx, catalog, storage, clock),
    }
}

fn eval_having_predicate(
    expr: &Expr,
    row: &[ContextTable],
    group: &Group,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<bool, DbError> {
    let value = eval_having_expr(expr, row, group, ctx, catalog, storage, clock)?;
    let result = match &value {
        Value::Bit(v) => *v,
        Value::Null => false,
        other => truthy(other),
    };
    Ok(result)
}

impl<'a> GroupExecutor<'a> {
    pub fn build_groups(
        &self,
        rows: Vec<JoinedRow>,
        group_by: &[Expr],
        ctx: &mut ExecutionContext,
    ) -> Result<Vec<Group>, DbError> {
        if group_by.is_empty() {
            return Ok(vec![Group { key: vec![], rows }]);
        }
        let mut map: HashMap<Vec<super::value_ops::SqlHashKey>, usize> = HashMap::new();
        let mut groups: Vec<Group> = Vec::new();
        for row in rows {
            let mut key = Vec::new();
            for expr in group_by {
                key.push(eval_expr(
                    expr,
                    &row,
                    ctx,
                    self.catalog,
                    self.storage,
                    self.clock,
                )?);
            }
            let hash_key: Vec<super::value_ops::SqlHashKey> = group_by
                .iter()
                .zip(key.iter())
                .map(|(expr, v)| {
                    if let Some(flags) = super::collation::expr_collation(expr) {
                        super::value_ops::SqlHashKey::from_value_with_collation(v, flags)
                    } else {
                        super::value_ops::SqlHashKey::from_value(v)
                    }
                })
                .collect();

            if let Some(&idx) = map.get(&hash_key) {
                groups[idx].rows.push(row);
            } else {
                let idx = groups.len();
                map.insert(hash_key, idx);
                groups.push(Group {
                    key,
                    rows: vec![row],
                });
            }
        }
        Ok(groups)
    }

    pub fn collect_surviving_groups(
        &self,
        rows: Vec<JoinedRow>,
        group_by: &[Expr],
        having: Option<&Expr>,
        ctx: &mut ExecutionContext,
    ) -> Result<Vec<Group>, DbError> {
        let groups = self.build_groups(rows, group_by, ctx)?;
        let mut out = Vec::with_capacity(groups.len());
        for group in groups {
            let sample_row = if group.rows.is_empty() {
                vec![]
            } else {
                group.rows[0].clone()
            };
            if let Some(having_expr) = having {
                if !eval_having_predicate(
                    having_expr,
                    &sample_row,
                    &group,
                    ctx,
                    self.catalog,
                    self.storage,
                    self.clock,
                )? {
                    continue;
                }
            }
            if !sample_row.is_empty() {
                out.push(group);
            }
        }
        Ok(out)
    }

    pub fn execute_grouped_select(
        &self,
        projection: Vec<crate::ast::SelectItem>,
        rows: Vec<JoinedRow>,
        source_shape: Option<&JoinedRow>,
        group_by: Vec<Expr>,
        having: Option<Expr>,
        ctx: &mut ExecutionContext,
    ) -> Result<super::result::QueryResult, DbError> {
        let rewritten_proj: Vec<crate::ast::SelectItem> = projection
            .into_iter()
            .map(|item| crate::ast::SelectItem {
                expr: rewrite_grouping_expr(&item.expr, &group_by, &[]),
                alias: item.alias,
            })
            .collect();
        let rewritten_having = having
            .as_ref()
            .map(|h| rewrite_grouping_expr(h, &group_by, &[]));
        let projection = rewritten_proj;
        let having = rewritten_having;
        if rows.is_empty() {
            if let Some(shape) = source_shape {
                for item in &projection {
                    validate_expr_columns_against_shape(&item.expr, shape, ctx)?;
                }
            }
        }
        let groups = self.build_groups(rows, &group_by, ctx)?;
        let mut projected_rows = Vec::new();

        for group in groups {
            if let Some(having_expr) = &having {
                let sample_row = if group.rows.is_empty() {
                    vec![]
                } else {
                    group.rows[0].clone()
                };
                if !eval_having_predicate(
                    having_expr,
                    &sample_row,
                    &group,
                    ctx,
                    self.catalog,
                    self.storage,
                    self.clock,
                )? {
                    continue;
                }
            }
            projected_rows.push(self.project_group_row(&projection, &group, ctx)?);
        }

        let columns = expand_projection_columns(&projection, None);
        let mut column_types = Vec::new();
        if !projected_rows.is_empty() {
            for val in &projected_rows[0] {
                column_types.push(
                    val.data_type()
                        .unwrap_or(crate::types::DataType::VarChar { max_len: 4000 }),
                );
            }
        } else {
            column_types = vec![crate::types::DataType::VarChar { max_len: 4000 }; columns.len()];
        }
        let column_nullabilities = vec![true; columns.len()];
        Ok(super::result::QueryResult {
            columns,
            column_types,
            column_nullabilities,
            rows: projected_rows,
            ..Default::default()
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn execute_grouping_sets_select(
        &self,
        projection: Vec<crate::ast::SelectItem>,
        rows: Vec<JoinedRow>,
        source_shape: Option<&JoinedRow>,
        all_group_by: &[Expr],
        grouping_sets: &[Vec<Expr>],
        having: Option<&Expr>,
        ctx: &mut ExecutionContext,
    ) -> Result<super::result::QueryResult, DbError> {
        if rows.is_empty() {
            if let Some(shape) = source_shape {
                for item in &projection {
                    let probe = rewrite_grouping_expr(&item.expr, all_group_by, &[]);
                    validate_expr_columns_against_shape(&probe, shape, ctx)?;
                }
            }
        }

        let mut projected_rows: Vec<Vec<Value>> = Vec::new();
        for active_set in grouping_sets {
            let omitted: Vec<Expr> = all_group_by
                .iter()
                .filter(|e| !expr_in_list(e, active_set))
                .cloned()
                .collect();

            let set_proj: Vec<crate::ast::SelectItem> = projection
                .iter()
                .map(|item| crate::ast::SelectItem {
                    expr: rewrite_grouping_expr(&item.expr, active_set, &omitted),
                    alias: item.alias.clone(),
                })
                .collect();
            let set_having = having.map(|h| rewrite_grouping_expr(h, active_set, &omitted));

            let groups = self.build_groups(rows.clone(), active_set, ctx)?;
            for group in groups {
                if group.rows.is_empty() && !active_set.is_empty() {
                    continue;
                }
                if let Some(ref having_expr) = set_having {
                    let sample_row = if group.rows.is_empty() {
                        vec![]
                    } else {
                        group.rows[0].clone()
                    };
                    if !eval_having_predicate(
                        having_expr,
                        &sample_row,
                        &group,
                        ctx,
                        self.catalog,
                        self.storage,
                        self.clock,
                    )? {
                        continue;
                    }
                }
                projected_rows.push(self.project_group_row(&set_proj, &group, ctx)?);
            }
        }

        let columns = expand_projection_columns(&projection, None);
        let mut column_types = Vec::new();
        if !projected_rows.is_empty() {
            for col_idx in 0..columns.len() {
                let ty = projected_rows
                    .iter()
                    .find_map(|r| r.get(col_idx).and_then(|v| v.data_type()))
                    .unwrap_or(crate::types::DataType::VarChar { max_len: 4000 });
                column_types.push(ty);
            }
        } else {
            column_types = vec![crate::types::DataType::VarChar { max_len: 4000 }; columns.len()];
        }
        let column_nullabilities = vec![true; columns.len()];
        Ok(super::result::QueryResult {
            columns,
            column_types,
            column_nullabilities,
            rows: projected_rows,
            ..Default::default()
        })
    }

    fn project_group_row(
        &self,
        projection: &[crate::ast::SelectItem],
        group: &Group,
        ctx: &mut ExecutionContext,
    ) -> Result<Vec<Value>, DbError> {
        let mut out = Vec::new();
        let sample_row = if group.rows.is_empty() {
            None
        } else {
            Some(&group.rows[0])
        };

        ctx.row.current_group = Some(group.clone());
        for item in projection {
            match &item.expr {
                Expr::Wildcard => {
                    if let Some(row) = sample_row {
                        out.extend(super::projection::expand_wildcard_values(row));
                    }
                }
                expr => {
                    let row_to_use = sample_row.cloned().unwrap_or_else(Vec::new);
                    out.push(eval_expr(
                        expr,
                        &row_to_use,
                        ctx,
                        self.catalog,
                        self.storage,
                        self.clock,
                    )?);
                }
            }
        }
        ctx.row.current_group = None;
        Ok(out)
    }
}

fn validate_expr_columns_against_shape(
    expr: &Expr,
    shape: &[ContextTable],
    ctx: &ExecutionContext,
) -> Result<(), DbError> {
    match expr {
        Expr::Identifier(name) => {
            if name.starts_with('@') {
                return Ok(());
            }
            let found_in = |tables: &[ContextTable]| {
                tables.iter().any(|ct| {
                    ct.table
                        .columns
                        .iter()
                        .any(|c| c.name.eq_ignore_ascii_case(name))
                })
            };
            if found_in(shape)
                || ctx.row.outer_stack.iter().any(|r| found_in(r))
                || ctx.row.apply_stack.iter().any(|r| found_in(r))
            {
                return Ok(());
            }
            Err(DbError::column_not_found(name))
        }
        Expr::QualifiedIdentifier(parts) if parts.len() >= 2 => {
            let col_name = parts.last().unwrap();
            let qual = &parts[parts.len() - 2];
            let found_in = |tables: &[ContextTable]| {
                tables.iter().any(|ct| {
                    (ct.alias.eq_ignore_ascii_case(qual)
                        || ct.table.name.eq_ignore_ascii_case(qual))
                        && ct
                            .table
                            .columns
                            .iter()
                            .any(|c| c.name.eq_ignore_ascii_case(col_name))
                })
            };
            if found_in(shape)
                || ctx.row.outer_stack.iter().any(|r| found_in(r))
                || ctx.row.apply_stack.iter().any(|r| found_in(r))
            {
                return Ok(());
            }
            Err(DbError::column_not_found(col_name))
        }
        Expr::FunctionCall { name, args, .. } => {
            let skip_first = matches!(
                name.to_ascii_uppercase().as_str(),
                "DATEADD"
                    | "DATEDIFF"
                    | "DATEDIFF_BIG"
                    | "DATEPART"
                    | "DATENAME"
                    | "DATETRUNC"
                    | "DATE_BUCKET"
            );
            for (idx, a) in args.iter().enumerate() {
                if skip_first && idx == 0 {
                    continue;
                }
                validate_expr_columns_against_shape(a, shape, ctx)?;
            }
            Ok(())
        }
        Expr::Binary { left, right, .. } => {
            validate_expr_columns_against_shape(left, shape, ctx)?;
            validate_expr_columns_against_shape(right, shape, ctx)
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::TryCast { expr: inner, .. }
        | Expr::Convert { expr: inner, .. }
        | Expr::TryConvert { expr: inner, .. }
        | Expr::IsNull(inner)
        | Expr::IsNotNull(inner) => validate_expr_columns_against_shape(inner, shape, ctx),
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => {
            if let Some(op) = operand {
                validate_expr_columns_against_shape(op, shape, ctx)?;
            }
            for wc in when_clauses {
                validate_expr_columns_against_shape(&wc.condition, shape, ctx)?;
                validate_expr_columns_against_shape(&wc.result, shape, ctx)?;
            }
            if let Some(el) = else_result {
                validate_expr_columns_against_shape(el, shape, ctx)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn expr_matches_grouping(a: &Expr, b: &Expr) -> bool {
    if a == b {
        return true;
    }
    match (a, b) {
        (Expr::Identifier(x), Expr::Identifier(y)) => x.eq_ignore_ascii_case(y),
        (Expr::QualifiedIdentifier(xs), Expr::QualifiedIdentifier(ys)) => {
            xs.len() == ys.len()
                && xs
                    .iter()
                    .zip(ys.iter())
                    .all(|(x, y)| x.eq_ignore_ascii_case(y))
        }
        (Expr::QualifiedIdentifier(xs), Expr::Identifier(y))
        | (Expr::Identifier(y), Expr::QualifiedIdentifier(xs)) => {
            xs.last().is_some_and(|last| last.eq_ignore_ascii_case(y))
        }
        _ => false,
    }
}

fn expr_in_list(target: &Expr, list: &[Expr]) -> bool {
    list.iter().any(|e| expr_matches_grouping(target, e))
}

fn rewrite_grouping_expr(expr: &Expr, active_set: &[Expr], omitted: &[Expr]) -> Expr {
    if expr_in_list(expr, omitted) {
        return Expr::Null;
    }
    match expr {
        Expr::FunctionCall {
            name,
            args,
            within_group,
        } => {
            if name.eq_ignore_ascii_case("GROUPING") && args.len() == 1 {
                let is_agg_away = !expr_in_list(&args[0], active_set);
                return Expr::Integer(if is_agg_away { 1 } else { 0 });
            }
            if name.eq_ignore_ascii_case("GROUPING_ID") && !args.is_empty() {
                let mut mask: i64 = 0;
                for arg in args {
                    mask <<= 1;
                    if !expr_in_list(arg, active_set) {
                        mask |= 1;
                    }
                }
                return Expr::Integer(mask);
            }
            // Do NOT rewrite column references inside aggregate functions like SUM(dept) / COUNT(dept).
            if super::aggregates::is_aggregate_function(name) {
                return expr.clone();
            }
            Expr::FunctionCall {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| rewrite_grouping_expr(a, active_set, omitted))
                    .collect(),
                within_group: within_group.clone(),
            }
        }
        Expr::Binary { left, op, right } => Expr::Binary {
            left: Box::new(rewrite_grouping_expr(left, active_set, omitted)),
            op: *op,
            right: Box::new(rewrite_grouping_expr(right, active_set, omitted)),
        },
        Expr::Unary { op, expr: inner } => Expr::Unary {
            op: *op,
            expr: Box::new(rewrite_grouping_expr(inner, active_set, omitted)),
        },
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => Expr::Case {
            operand: operand
                .as_ref()
                .map(|o| Box::new(rewrite_grouping_expr(o, active_set, omitted))),
            when_clauses: when_clauses
                .iter()
                .map(|wc| crate::ast::WhenClause {
                    condition: rewrite_grouping_expr(&wc.condition, active_set, omitted),
                    result: rewrite_grouping_expr(&wc.result, active_set, omitted),
                })
                .collect(),
            else_result: else_result
                .as_ref()
                .map(|e| Box::new(rewrite_grouping_expr(e, active_set, omitted))),
        },
        Expr::Cast {
            expr: inner,
            target,
        } => Expr::Cast {
            expr: Box::new(rewrite_grouping_expr(inner, active_set, omitted)),
            target: target.clone(),
        },
        Expr::TryCast {
            expr: inner,
            target,
        } => Expr::TryCast {
            expr: Box::new(rewrite_grouping_expr(inner, active_set, omitted)),
            target: target.clone(),
        },
        Expr::Convert {
            target,
            expr: inner,
            style,
        } => Expr::Convert {
            target: target.clone(),
            expr: Box::new(rewrite_grouping_expr(inner, active_set, omitted)),
            style: *style,
        },
        Expr::TryConvert {
            target,
            expr: inner,
            style,
        } => Expr::TryConvert {
            target: target.clone(),
            expr: Box::new(rewrite_grouping_expr(inner, active_set, omitted)),
            style: *style,
        },
        Expr::IsNull(inner) => {
            Expr::IsNull(Box::new(rewrite_grouping_expr(inner, active_set, omitted)))
        }
        Expr::IsNotNull(inner) => {
            Expr::IsNotNull(Box::new(rewrite_grouping_expr(inner, active_set, omitted)))
        }
        Expr::Between {
            expr: inner,
            low,
            high,
            negated,
        } => Expr::Between {
            expr: Box::new(rewrite_grouping_expr(inner, active_set, omitted)),
            low: Box::new(rewrite_grouping_expr(low, active_set, omitted)),
            high: Box::new(rewrite_grouping_expr(high, active_set, omitted)),
            negated: *negated,
        },
        Expr::InList {
            expr: inner,
            list,
            negated,
        } => Expr::InList {
            expr: Box::new(rewrite_grouping_expr(inner, active_set, omitted)),
            list: list
                .iter()
                .map(|i| rewrite_grouping_expr(i, active_set, omitted))
                .collect(),
            negated: *negated,
        },
        Expr::Like {
            expr: inner,
            pattern,
            escape,
            negated,
        } => Expr::Like {
            expr: Box::new(rewrite_grouping_expr(inner, active_set, omitted)),
            pattern: Box::new(rewrite_grouping_expr(pattern, active_set, omitted)),
            escape: escape
                .as_ref()
                .map(|e| Box::new(rewrite_grouping_expr(e, active_set, omitted))),
            negated: *negated,
        },
        other => other.clone(),
    }
}
