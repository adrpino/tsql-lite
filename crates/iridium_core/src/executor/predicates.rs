use std::cmp::Ordering;

use crate::ast::{Expr, WhenClause};
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::types::Value;

use super::clock::Clock;
use super::context::ExecutionContext;
use super::evaluator::eval_expr;
use super::model::ContextTable;
use super::operators::compare_bool;
use super::query::plan::RelationalQuery;
use super::query::QueryExecutor;
use super::result::QueryResult;
use super::value_ops::truthy;
use crate::storage::Storage;

#[allow(clippy::too_many_arguments)]
pub(crate) fn eval_case(
    operand: Option<&Expr>,
    when_clauses: &[WhenClause],
    else_result: Option<&Expr>,
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let operand_val = match operand {
        Some(e) => Some(eval_expr(e, row, ctx, catalog, storage, clock)?),
        None => None,
    };

    for clause in when_clauses {
        let match_found = if let Some(ref op_val) = operand_val {
            let when_val = eval_expr(&clause.condition, row, ctx, catalog, storage, clock)?;
            compare_bool(
                op_val.clone(),
                when_val,
                |o| o == Ordering::Equal,
                ctx.metadata.ansi_nulls,
            )
        } else {
            let cond = eval_expr(&clause.condition, row, ctx, catalog, storage, clock)?;
            Value::Bit(truthy(&cond))
        };

        if let Value::Bit(true) = match_found {
            return eval_expr(&clause.result, row, ctx, catalog, storage, clock);
        }
    }

    match else_result {
        Some(expr) => eval_expr(expr, row, ctx, catalog, storage, clock),
        None => Ok(Value::Null),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn eval_in_list(
    in_expr: &Expr,
    list: &[Expr],
    negated: bool,
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let val = eval_expr(in_expr, row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let lhs_cs = super::evaluator::expr_collation_case_sensitive(in_expr);
    let mut found = false;
    for item in list {
        let item_val = eval_expr(item, row, ctx, catalog, storage, clock)?;
        if item_val.is_null() {
            return Ok(Value::Null);
        }
        let is_cs = lhs_cs
            .or_else(|| super::evaluator::expr_collation_case_sensitive(item))
            .unwrap_or(false);
        let matched = if is_cs {
            super::evaluator::compare_values_case_sensitive(&val, &item_val) == Ordering::Equal
        } else {
            compare_bool(
                val.clone(),
                item_val,
                |o| o == Ordering::Equal,
                ctx.metadata.ansi_nulls,
            ) == Value::Bit(true)
        };
        if matched {
            found = true;
            break;
        }
    }
    Ok(Value::Bit(if negated { !found } else { found }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn eval_between(
    between_expr: &Expr,
    low: &Expr,
    high: &Expr,
    negated: bool,
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let val = eval_expr(between_expr, row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let low_val = eval_expr(low, row, ctx, catalog, storage, clock)?;
    let high_val = eval_expr(high, row, ctx, catalog, storage, clock)?;

    let ge_low = compare_bool(
        val.clone(),
        low_val,
        |o| matches!(o, Ordering::Greater | Ordering::Equal),
        ctx.metadata.ansi_nulls,
    ) == Value::Bit(true);
    let le_high = compare_bool(
        val,
        high_val,
        |o| matches!(o, Ordering::Less | Ordering::Equal),
        ctx.metadata.ansi_nulls,
    ) == Value::Bit(true);

    let result = ge_low && le_high;
    Ok(Value::Bit(if negated { !result } else { result }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn eval_like(
    like_expr: &Expr,
    pattern: &Expr,
    escape: Option<&Expr>,
    negated: bool,
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let val = eval_expr(like_expr, row, ctx, catalog, storage, clock)?;
    let pat = eval_expr(pattern, row, ctx, catalog, storage, clock)?;

    if val.is_null() || pat.is_null() {
        return Ok(Value::Null);
    }

    let s = val.to_string_value();
    let p = pat.to_string_value();
    let esc = match escape {
        Some(e) => {
            let ev = eval_expr(e, row, ctx, catalog, storage, clock)?;
            if ev.is_null() {
                return Ok(Value::Null);
            }
            Some(ev.to_string_value())
        }
        None => None,
    };
    let case_sensitive = super::evaluator::expr_collation_case_sensitive(like_expr)
        .or_else(|| super::evaluator::expr_collation_case_sensitive(pattern))
        .unwrap_or(false);
    let matched = like_match(&s, &p, esc.as_deref(), case_sensitive);
    Ok(Value::Bit(if negated { !matched } else { matched }))
}

#[derive(Debug, Clone)]
enum LikeToken {
    AnySeq,
    AnyChar,
    Literal(char),
    CharClass {
        negated: bool,
        singles: Vec<char>,
        ranges: Vec<(char, char)>,
    },
}

fn like_token_matches_char(tok: &LikeToken, ch: char) -> bool {
    match tok {
        LikeToken::AnyChar => true,
        LikeToken::Literal(c) => *c == ch,
        LikeToken::CharClass {
            negated,
            singles,
            ranges,
        } => {
            let hit = singles.contains(&ch) || ranges.iter().any(|&(lo, hi)| ch >= lo && ch <= hi);
            if *negated {
                !hit
            } else {
                hit
            }
        }
        LikeToken::AnySeq => false,
    }
}

fn like_match(s: &str, pattern: &str, escape: Option<&str>, case_sensitive: bool) -> bool {
    let s: Vec<char> = if case_sensitive {
        s.chars().collect()
    } else {
        s.to_ascii_uppercase().chars().collect()
    };
    let p_raw: Vec<char> = if case_sensitive {
        pattern.chars().collect()
    } else {
        pattern.to_ascii_uppercase().chars().collect()
    };
    let esc_char = escape.and_then(|e| {
        e.chars().next().map(|c| {
            if case_sensitive {
                c
            } else {
                c.to_ascii_uppercase()
            }
        })
    });

    let mut tokens: Vec<LikeToken> = Vec::with_capacity(p_raw.len());
    let mut i = 0;
    while i < p_raw.len() {
        if let Some(ec) = esc_char {
            if p_raw[i] == ec && i + 1 < p_raw.len() {
                tokens.push(LikeToken::Literal(p_raw[i + 1]));
                i += 2;
                continue;
            }
        }
        match p_raw[i] {
            '%' => {
                tokens.push(LikeToken::AnySeq);
                i += 1;
            }
            '_' => {
                tokens.push(LikeToken::AnyChar);
                i += 1;
            }
            '[' => {
                // T-SQL LIKE character class: `[abc]`, `[a-z]`, `[^abc]`, `[%]`, `[_]`, `[[]`
                let mut j = i + 1;
                let mut negated = false;
                if j < p_raw.len() && p_raw[j] == '^' {
                    negated = true;
                    j += 1;
                }
                let mut singles = Vec::new();
                let mut ranges = Vec::new();
                let mut closed = false;
                while j < p_raw.len() {
                    if p_raw[j] == ']' {
                        closed = true;
                        j += 1;
                        break;
                    }
                    if j + 2 < p_raw.len() && p_raw[j + 1] == '-' && p_raw[j + 2] != ']' {
                        ranges.push((p_raw[j], p_raw[j + 2]));
                        j += 3;
                    } else {
                        singles.push(p_raw[j]);
                        j += 1;
                    }
                }
                if closed {
                    tokens.push(LikeToken::CharClass {
                        negated,
                        singles,
                        ranges,
                    });
                    i = j;
                } else {
                    tokens.push(LikeToken::Literal('['));
                    i += 1;
                }
            }
            c => {
                tokens.push(LikeToken::Literal(c));
                i += 1;
            }
        }
    }

    let pn = tokens.len();

    let mut dp = vec![false; pn + 1];
    dp[0] = true;
    for j in 0..pn {
        if matches!(tokens[j], LikeToken::AnySeq) {
            dp[j + 1] = dp[j];
        } else {
            break;
        }
    }

    for &ch in &s {
        let mut prev = dp[0];
        dp[0] = false;
        for j in 0..pn {
            let tmp = dp[j + 1];
            dp[j + 1] = if matches!(tokens[j], LikeToken::AnySeq) {
                tmp || dp[j]
            } else if like_token_matches_char(&tokens[j], ch) {
                prev
            } else {
                false
            };
            prev = tmp;
        }
    }

    dp[pn]
}

pub(crate) fn eval_scalar_subquery(
    stmt: &crate::ast::SelectStmt,
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let mut sub_ctx = ctx.with_outer_row(row.to_vec());
    let query_result = execute_subquery_select(stmt, &mut sub_ctx, catalog, storage, clock)?;

    if query_result.rows.is_empty() {
        return Ok(Value::Null);
    }

    let first_row = &query_result.rows[0];
    if first_row.is_empty() {
        return Ok(Value::Null);
    }

    let val = first_row[0].clone();
    Ok(val)
}

pub(crate) fn eval_exists(
    stmt: &crate::ast::SelectStmt,
    negated: bool,
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let mut sub_ctx = ctx.with_outer_row(row.to_vec());
    let exists = if stmt.group_by.is_empty()
        && stmt.having.is_none()
        && stmt.set_op.is_none()
        && stmt.extra.is_none()
        && stmt.offset.is_none()
        && stmt.fetch.is_none()
        && stmt.top.is_none()
    {
        // Short-circuit on the first matching row without materializing the full subquery.
        let qe = QueryExecutor {
            catalog,
            storage,
            clock,
        };
        let query = RelationalQuery::from(stmt.clone());
        let mut src = crate::executor::query::source::execute_source(&qe, &query, &mut sub_ctx)?;
        src.iter
            .next_row(&mut sub_ctx, catalog, storage, clock)?
            .is_some()
    } else {
        let query_result = execute_subquery_select(stmt, &mut sub_ctx, catalog, storage, clock)?;
        !query_result.rows.is_empty()
    };
    Ok(Value::Bit(if negated { !exists } else { exists }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn eval_in_subquery(
    in_expr: &Expr,
    stmt: &crate::ast::SelectStmt,
    negated: bool,
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let val = eval_expr(in_expr, row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }

    let mut sub_ctx = ctx.with_outer_row(row.to_vec());
    let query_result = execute_subquery_select(stmt, &mut sub_ctx, catalog, storage, clock)?;

    if query_result.rows.is_empty() {
        return Ok(Value::Bit(negated));
    }

    let mut found = false;
    let mut has_null = false;

    for row_data in &query_result.rows {
        if row_data.is_empty() {
            continue;
        }
        let subq_val = &row_data[0];

        if subq_val.is_null() {
            has_null = true;
            continue;
        }

        if compare_bool(
            val.clone(),
            subq_val.clone(),
            |o| o == Ordering::Equal,
            ctx.metadata.ansi_nulls,
        ) == Value::Bit(true)
        {
            found = true;
            break;
        }
    }

    if !found && has_null && !negated {
        return Ok(Value::Null);
    }

    Ok(Value::Bit(if negated { !found } else { found }))
}

fn execute_subquery_select(
    stmt: &crate::ast::SelectStmt,
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<QueryResult, DbError> {
    if is_uncorrelated(stmt) {
        let key = format!("{:?}", stmt);
        {
            let cache = ctx.subquery_cache.lock();
            if let Some(cached) = cache.get(&key) {
                return Ok(cached.clone());
            }
        }

        let mut sub_ctx = ctx.subquery();
        let qe = QueryExecutor {
            catalog,
            storage,
            clock,
        };
        let result = qe.execute_select(RelationalQuery::from(stmt.clone()), &mut sub_ctx)?;

        {
            let mut cache = ctx.subquery_cache.lock();
            cache.insert(key, result.clone());
        }
        return Ok(result);
    }

    let mut sub_ctx = ctx.subquery();
    let qe = QueryExecutor {
        catalog,
        storage,
        clock,
    };

    qe.execute_select(RelationalQuery::from(stmt.clone()), &mut sub_ctx)
}

fn is_uncorrelated(stmt: &crate::ast::SelectStmt) -> bool {
    // If it has NO FromNode, it's uncorrelated if selection/projection are uncorrelated.
    if stmt.from_clause.is_none() {
        return stmt
            .projection
            .iter()
            .all(|i| is_expr_uncorrelated(&i.expr))
            && stmt.selection.as_ref().is_none_or(is_expr_uncorrelated);
    }
    // For now, only literals are considered uncorrelated.
    false
}

fn is_expr_uncorrelated(expr: &Expr) -> bool {
    match expr {
        Expr::Integer(_)
        | Expr::FloatLiteral(_)
        | Expr::BinaryLiteral(_)
        | Expr::String(_)
        | Expr::UnicodeString(_)
        | Expr::Null => true,
        Expr::Binary { left, right, .. } => {
            is_expr_uncorrelated(left) && is_expr_uncorrelated(right)
        }
        Expr::Unary { expr, .. } => is_expr_uncorrelated(expr),
        Expr::Cast { expr, .. }
        | Expr::TryCast { expr, .. }
        | Expr::Convert { expr, .. }
        | Expr::TryConvert { expr, .. } => is_expr_uncorrelated(expr),
        Expr::FunctionCall { args, .. } => args.iter().all(is_expr_uncorrelated),
        _ => false,
    }
}
