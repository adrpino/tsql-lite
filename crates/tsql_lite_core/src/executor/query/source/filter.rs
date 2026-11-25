use std::cmp::Ordering;
use std::collections::HashSet;

use crate::ast::{BinaryOp, Expr};
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::executor::clock::Clock;
use crate::executor::context::ExecutionContext;
use crate::executor::evaluator::eval_expr;
use crate::executor::joins::{eval_key, get_expr_side, JoinSide};
use crate::executor::model::{ContextTable, JoinedRow};
use crate::executor::value_ops::{compare_values, HashKeyFamily, SqlHashKey};
use crate::storage::Storage;
use crate::types::Value;

use super::super::from_tree::{self, FromEval, RowIterator};
use super::super::plan::RelationalQuery;
use super::super::QueryExecutor;
use super::from;

struct HashSemiJoinIterator {
    source: Box<dyn RowIterator>,
    outer_keys: Vec<Expr>,
    inner_keys: HashSet<Vec<SqlHashKey>>,
    negated: bool,
}

impl RowIterator for HashSemiJoinIterator {
    fn next_row(
        &mut self,
        ctx: &mut ExecutionContext,
        catalog: &dyn Catalog,
        storage: &dyn Storage,
        clock: &dyn Clock,
    ) -> Result<Option<JoinedRow>, DbError> {
        while let Some(row) = self.source.next_row(ctx, catalog, storage, clock)? {
            let raw = eval_key(&self.outer_keys, &row, ctx, catalog, storage, clock)?;
            let exists = if raw.iter().any(|v| v.is_null()) {
                false
            } else {
                let norm: Vec<SqlHashKey> = raw.iter().map(SqlHashKey::from_value).collect();
                self.inner_keys.contains(&norm)
            };
            if if self.negated { !exists } else { exists } {
                return Ok(Some(row));
            }
        }
        Ok(None)
    }

    fn reset(&mut self) -> Result<(), DbError> {
        self.source.reset()
    }
}

struct HashInSubqueryIterator {
    source: Box<dyn RowIterator>,
    lhs_expr: Expr,
    inner_set: HashSet<SqlHashKey>,
    inner_raw: Vec<Value>,
    inner_family: Option<HashKeyFamily>,
    has_null: bool,
    is_empty: bool,
    negated: bool,
}

impl RowIterator for HashInSubqueryIterator {
    fn next_row(
        &mut self,
        ctx: &mut ExecutionContext,
        catalog: &dyn Catalog,
        storage: &dyn Storage,
        clock: &dyn Clock,
    ) -> Result<Option<JoinedRow>, DbError> {
        while let Some(row) = self.source.next_row(ctx, catalog, storage, clock)? {
            let lhs = eval_expr(&self.lhs_expr, &row, ctx, catalog, storage, clock)?;
            let matched = if self.is_empty {
                self.negated
            } else if lhs.is_null() {
                false
            } else {
                let hk = SqlHashKey::from_value(&lhs);
                let found = matches!(
                    self.inner_family,
                    Some(fam) if fam == hk.family() && fam != HashKeyFamily::Mixed
                )
                .then(|| self.inner_set.contains(&hk))
                .unwrap_or_else(|| {
                    self.inner_raw
                        .iter()
                        .any(|rv| compare_values(&lhs, rv) == Ordering::Equal)
                });
                if self.negated {
                    !found && !self.has_null
                } else {
                    found
                }
            };
            if matched {
                return Ok(Some(row));
            }
        }
        Ok(None)
    }

    fn reset(&mut self) -> Result<(), DbError> {
        self.source.reset()
    }
}

fn expr_references_outer(
    expr: &Expr,
    inner_shape: &[ContextTable],
    outer_shape: &[ContextTable],
) -> bool {
    match expr {
        Expr::Identifier(name) => {
            if name.starts_with('@') {
                return false;
            }
            let in_inner = inner_shape.iter().any(|b| {
                b.table
                    .columns
                    .iter()
                    .any(|c| c.name.eq_ignore_ascii_case(name))
            });
            if in_inner {
                return false;
            }
            outer_shape.iter().any(|b| {
                b.table
                    .columns
                    .iter()
                    .any(|c| c.name.eq_ignore_ascii_case(name))
            })
        }
        Expr::QualifiedIdentifier(parts) => {
            if parts.len() < 2 {
                return true;
            }
            let tbl = &parts[parts.len() - 2];
            !inner_shape.iter().any(|b| {
                b.alias.eq_ignore_ascii_case(tbl)
                    || b.table.name.eq_ignore_ascii_case(tbl)
                    || b.source_aliases.iter().any(|a| a.eq_ignore_ascii_case(tbl))
            })
        }
        Expr::Binary { left, right, .. } => {
            expr_references_outer(left, inner_shape, outer_shape)
                || expr_references_outer(right, inner_shape, outer_shape)
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::TryCast { expr: inner, .. }
        | Expr::Convert { expr: inner, .. }
        | Expr::TryConvert { expr: inner, .. }
        | Expr::IsNull(inner)
        | Expr::IsNotNull(inner) => expr_references_outer(inner, inner_shape, outer_shape),
        Expr::FunctionCall { args, .. } => args
            .iter()
            .any(|a| expr_references_outer(a, inner_shape, outer_shape)),
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => {
            operand
                .as_deref()
                .is_some_and(|o| expr_references_outer(o, inner_shape, outer_shape))
                || when_clauses.iter().any(|wc| {
                    expr_references_outer(&wc.condition, inner_shape, outer_shape)
                        || expr_references_outer(&wc.result, inner_shape, outer_shape)
                })
                || else_result
                    .as_deref()
                    .is_some_and(|e| expr_references_outer(e, inner_shape, outer_shape))
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            expr_references_outer(expr, inner_shape, outer_shape)
                || expr_references_outer(low, inner_shape, outer_shape)
                || expr_references_outer(high, inner_shape, outer_shape)
        }
        Expr::InList { expr, list, .. } => {
            expr_references_outer(expr, inner_shape, outer_shape)
                || list
                    .iter()
                    .any(|i| expr_references_outer(i, inner_shape, outer_shape))
        }
        Expr::Like {
            expr,
            pattern,
            escape,
            ..
        } => {
            expr_references_outer(expr, inner_shape, outer_shape)
                || expr_references_outer(pattern, inner_shape, outer_shape)
                || escape
                    .as_deref()
                    .is_some_and(|e| expr_references_outer(e, inner_shape, outer_shape))
        }
        Expr::Subquery(_) | Expr::Exists { .. } | Expr::InSubquery { .. } => true,
        _ => false,
    }
}

fn select_is_uncorrelated(
    executor: &QueryExecutor<'_>,
    subquery: &crate::ast::SelectStmt,
    outer_shape: &[ContextTable],
    ctx: &mut ExecutionContext,
) -> Result<bool, DbError> {
    if subquery.set_op.is_some()
        || !subquery.applies.is_empty()
        || subquery.projection.len() != 1
        || subquery.from_clause.is_none()
    {
        return Ok(false);
    }
    let probe_query = RelationalQuery {
        from_clause: subquery.from_clause.clone(),
        applies: Vec::new(),
        projection: crate::executor::query::plan::ProjectionPlan {
            items: Vec::new(),
            distinct: false,
        },
        filter: crate::executor::query::plan::FilterPlan {
            selection: None,
            group_by: Vec::new(),
            grouping_sets: Vec::new(),
            having: None,
        },
        sort: crate::executor::query::plan::SortPlan {
            order_by: Vec::new(),
        },
        pagination: crate::executor::query::plan::PaginationPlan {
            top: None,
            offset: None,
            fetch: None,
        },
        into_table: None,
        set_op: None,
        for_xml: None,
        for_json: None,
    };
    let probe_from = from::execute_from_stage(executor, &probe_query, ctx)?;
    let inner_shape = &probe_from.shape;

    if expr_references_outer(&subquery.projection[0].expr, inner_shape, outer_shape) {
        return Ok(false);
    }
    if subquery
        .selection
        .as_ref()
        .is_some_and(|e| expr_references_outer(e, inner_shape, outer_shape))
    {
        return Ok(false);
    }
    if subquery
        .group_by
        .iter()
        .any(|e| expr_references_outer(e, inner_shape, outer_shape))
    {
        return Ok(false);
    }
    if subquery
        .having
        .as_ref()
        .is_some_and(|e| expr_references_outer(e, inner_shape, outer_shape))
    {
        return Ok(false);
    }
    if subquery
        .order_by
        .iter()
        .any(|o| expr_references_outer(&o.expr, inner_shape, outer_shape))
    {
        return Ok(false);
    }
    Ok(true)
}

fn try_build_hash_in_subquery(
    executor: &QueryExecutor<'_>,
    lhs_expr: &Expr,
    subquery: &crate::ast::SelectStmt,
    negated: bool,
    outer_shape: &[ContextTable],
    ctx: &mut ExecutionContext,
) -> Result<Option<HashInSubqueryIterator>, DbError> {
    if expr_contains_subquery(lhs_expr)
        || !select_is_uncorrelated(executor, subquery, outer_shape, ctx)?
    {
        return Ok(None);
    }
    let sub_query = RelationalQuery::from(subquery.clone());
    let result = executor.execute_select(sub_query, ctx)?;
    if result.columns.len() != 1 {
        return Ok(None);
    }

    let is_empty = result.rows.is_empty();
    let mut has_null = false;
    let mut inner_set = HashSet::with_capacity(result.rows.len());
    let mut inner_raw = Vec::with_capacity(result.rows.len());
    let mut inner_family: Option<HashKeyFamily> = None;

    for row in result.rows {
        let val = row.into_iter().next().unwrap_or(Value::Null);
        if val.is_null() {
            has_null = true;
            continue;
        }
        let hk = SqlHashKey::from_value(&val);
        let fam = hk.family();
        inner_family = match inner_family {
            None => Some(fam),
            Some(prev) if prev == fam => Some(prev),
            Some(_) => Some(HashKeyFamily::Mixed),
        };
        inner_set.insert(hk);
        inner_raw.push(val);
    }

    Ok(Some(HashInSubqueryIterator {
        // Placeholder; caller attaches actual `source` iterator immediately.
        source: Box::new(from_tree::ScanIterator::new(Vec::new())),
        lhs_expr: lhs_expr.clone(),
        inner_set,
        inner_raw,
        inner_family,
        has_null,
        is_empty,
        negated,
    }))
}

fn expr_contains_subquery(expr: &Expr) -> bool {
    match expr {
        Expr::Subquery(_) | Expr::Exists { .. } | Expr::InSubquery { .. } => true,
        Expr::Binary { left, right, .. } => {
            expr_contains_subquery(left) || expr_contains_subquery(right)
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::TryCast { expr: inner, .. }
        | Expr::Convert { expr: inner, .. }
        | Expr::TryConvert { expr: inner, .. }
        | Expr::IsNull(inner)
        | Expr::IsNotNull(inner) => expr_contains_subquery(inner),
        Expr::FunctionCall { args, .. } => args.iter().any(expr_contains_subquery),
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => {
            operand.as_deref().is_some_and(expr_contains_subquery)
                || when_clauses.iter().any(|wc| {
                    expr_contains_subquery(&wc.condition) || expr_contains_subquery(&wc.result)
                })
                || else_result.as_deref().is_some_and(expr_contains_subquery)
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            expr_contains_subquery(expr)
                || expr_contains_subquery(low)
                || expr_contains_subquery(high)
        }
        Expr::InList { expr, list, .. } => {
            expr_contains_subquery(expr) || list.iter().any(expr_contains_subquery)
        }
        Expr::Like {
            expr,
            pattern,
            escape,
            ..
        } => {
            expr_contains_subquery(expr)
                || expr_contains_subquery(pattern)
                || escape.as_deref().is_some_and(expr_contains_subquery)
        }
        _ => false,
    }
}

fn split_and_owned(expr: &Expr, out: &mut Vec<Expr>) {
    if let Expr::Binary {
        left,
        op: BinaryOp::And,
        right,
    } = expr
    {
        split_and_owned(left, out);
        split_and_owned(right, out);
    } else {
        out.push(expr.clone());
    }
}

fn combine_and(exprs: Vec<Expr>) -> Option<Expr> {
    exprs.into_iter().reduce(|a, b| Expr::Binary {
        left: Box::new(a),
        op: BinaryOp::And,
        right: Box::new(b),
    })
}

type SemiJoinProbe = (Vec<Expr>, HashSet<Vec<SqlHashKey>>, bool);

fn try_build_hash_semi_join(
    executor: &QueryExecutor<'_>,
    subquery: &crate::ast::SelectStmt,
    negated: bool,
    outer_shape: &[ContextTable],
    ctx: &mut ExecutionContext,
) -> Result<Option<SemiJoinProbe>, DbError> {
    if !subquery.group_by.is_empty()
        || subquery.having.is_some()
        || subquery.set_op.is_some()
        || subquery.extra.is_some()
        || subquery.top.is_some()
        || subquery.offset.is_some()
        || subquery.fetch.is_some()
        || !subquery.applies.is_empty()
        || subquery.from_clause.is_none()
    {
        return Ok(None);
    }
    let Some(ref inner_where) = subquery.selection else {
        return Ok(None);
    };

    // Determine inner shape without evaluating outer references yet.
    let probe_query = RelationalQuery {
        from_clause: subquery.from_clause.clone(),
        applies: Vec::new(),
        projection: super::super::plan::ProjectionPlan {
            items: Vec::new(),
            distinct: false,
        },
        into_table: None,
        filter: super::super::plan::FilterPlan {
            selection: None,
            group_by: Vec::new(),
            grouping_sets: Vec::new(),
            having: None,
        },
        sort: super::super::plan::SortPlan {
            order_by: Vec::new(),
        },
        pagination: super::super::plan::PaginationPlan {
            top: None,
            offset: None,
            fetch: None,
        },
        set_op: None,
        for_xml: None,
        for_json: None,
    };
    let probe_from = from::execute_from_stage(executor, &probe_query, ctx)?;
    let inner_shape = &probe_from.shape;

    let mut conjuncts = Vec::new();
    split_and_owned(inner_where, &mut conjuncts);

    let mut inner_only = Vec::new();
    let mut inner_key_exprs = Vec::new();
    let mut outer_key_exprs = Vec::new();

    for c in conjuncts {
        if expr_contains_subquery(&c) {
            return Ok(None);
        }
        match &c {
            Expr::Binary {
                left,
                op: BinaryOp::Eq,
                right,
            } => {
                let ls = get_expr_side(left, outer_shape, inner_shape);
                let rs = get_expr_side(right, outer_shape, inner_shape);
                match (ls, rs) {
                    (JoinSide::Left, JoinSide::Right) => {
                        outer_key_exprs.push((**left).clone());
                        inner_key_exprs.push((**right).clone());
                    }
                    (JoinSide::Right, JoinSide::Left) => {
                        inner_key_exprs.push((**left).clone());
                        outer_key_exprs.push((**right).clone());
                    }
                    (JoinSide::Right | JoinSide::None, JoinSide::Right | JoinSide::None) => {
                        inner_only.push(c);
                    }
                    _ => return Ok(None),
                }
            }
            _ => match get_expr_side(&c, outer_shape, inner_shape) {
                JoinSide::Right | JoinSide::None => inner_only.push(c),
                _ => return Ok(None),
            },
        }
    }

    if inner_key_exprs.is_empty() {
        return Ok(None);
    }

    // Check that outer_shape and inner_shape column families match on the equi-join slots
    // (avoiding cross-family SQL coercions like INT = VARCHAR where SqlHashKey differs).
    let mut inner_query = probe_query;
    inner_query.filter.selection = combine_and(inner_only);
    let mut inner_eval = super::execute_source(executor, &inner_query, ctx)?;
    let mut inner_keys: HashSet<Vec<SqlHashKey>> = HashSet::new();
    let mut families: Vec<Option<HashKeyFamily>> = vec![None; inner_key_exprs.len()];

    while let Some(irow) =
        inner_eval
            .iter
            .next_row(ctx, executor.catalog, executor.storage, executor.clock)?
    {
        let raw = eval_key(
            &inner_key_exprs,
            &irow,
            ctx,
            executor.catalog,
            executor.storage,
            executor.clock,
        )?;
        if raw.iter().any(|v| v.is_null()) {
            continue;
        }
        let norm: Vec<SqlHashKey> = raw.iter().map(SqlHashKey::from_value).collect();
        for (slot, nk) in norm.iter().enumerate() {
            let fam = nk.family();
            match families[slot] {
                None => families[slot] = Some(fam),
                Some(prev) if prev != fam => return Ok(None),
                _ => {}
            }
        }
        inner_keys.insert(norm);
    }

    Ok(Some((outer_key_exprs, inner_keys, negated)))
}

pub(crate) fn execute_where_stage(
    executor: &QueryExecutor<'_>,
    query: &RelationalQuery,
    mut source_eval: FromEval,
    ctx: &mut ExecutionContext,
) -> Result<FromEval, DbError> {
    if let Some(where_clause) = &query.filter.selection {
        let shape = source_eval.shape.clone();
        let enforced = std::mem::take(&mut source_eval.enforced_predicates);
        // Decorrelate top-level `[NOT] EXISTS (...)` conjuncts into O(N+M) HashSemiJoinIterators.
        let mut conjuncts = Vec::new();
        split_and_owned(where_clause, &mut conjuncts);
        let mut remaining = Vec::new();
        for c in conjuncts {
            if enforced.contains(&c) {
                continue;
            }
            if let Expr::Exists {
                ref subquery,
                negated,
            } = c
            {
                if let Some((outer_keys, inner_keys, neg)) =
                    try_build_hash_semi_join(executor, subquery, negated, &shape, ctx)?
                {
                    source_eval.iter = Box::new(HashSemiJoinIterator {
                        source: source_eval.iter,
                        outer_keys,
                        inner_keys,
                        negated: neg,
                    });
                    continue;
                }
            }
            if let Expr::InSubquery {
                ref expr,
                ref subquery,
                negated,
            } = c
            {
                if let Some(mut hash_in) =
                    try_build_hash_in_subquery(executor, expr, subquery, negated, &shape, ctx)?
                {
                    hash_in.source = source_eval.iter;
                    source_eval.iter = Box::new(hash_in);
                    continue;
                }
            }
            remaining.push(c);
        }
        if let Some(pred) = combine_and(remaining) {
            let iter = Box::new(from_tree::FilterIterator {
                source: source_eval.iter,
                predicate: pred,
            });
            Ok(FromEval {
                iter,
                shape,
                enforced_predicates: Vec::new(),
            })
        } else {
            Ok(FromEval {
                iter: source_eval.iter,
                shape,
                enforced_predicates: Vec::new(),
            })
        }
    } else {
        Ok(source_eval)
    }
}
