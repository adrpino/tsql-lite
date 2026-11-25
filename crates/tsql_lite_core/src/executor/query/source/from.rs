use crate::ast::{BinaryOp, Expr, FromNode, JoinType};
use crate::error::DbError;

use super::super::from_tree::{self, FromEval};
use super::super::plan::RelationalQuery;
use super::super::QueryExecutor;

fn split_and_conjuncts<'a>(expr: &'a Expr, out: &mut Vec<&'a Expr>) {
    if let Expr::Binary {
        left,
        op: BinaryOp::And,
        right,
    } = expr
    {
        split_and_conjuncts(left, out);
        split_and_conjuncts(right, out);
    } else {
        out.push(expr);
    }
}

pub(crate) fn execute_from_stage(
    executor: &QueryExecutor<'_>,
    query: &RelationalQuery,
    ctx: &mut crate::executor::context::ExecutionContext,
) -> Result<FromEval, DbError> {
    if let Some(from_clause) = query.from_clause.clone() {
        // When a comma-join (CROSS JOIN with no ON) is paired with equi-join predicates
        // in WHERE, promote matching WHERE conjuncts into a HashJoin so large cross joins
        // (e.g. `FROM sys.schemas s, sys.objects o WHERE s.schema_id = o.schema_id`) run in O(N+M).
        if query.applies.is_empty() {
            if let (
                FromNode::Join {
                    left,
                    join_type: JoinType::Cross,
                    right,
                    on: None,
                },
                Some(where_expr),
            ) = (&from_clause, query.filter.selection.as_ref())
            {
                let mut left_eval =
                    from_tree::execute_from_clause(executor, (**left).clone(), ctx)?;
                let mut right_eval =
                    from_tree::execute_from_clause(executor, (**right).clone(), ctx)?;
                let mut conjuncts = Vec::new();
                split_and_conjuncts(where_expr, &mut conjuncts);
                let mut left_keys = Vec::new();
                let mut right_keys = Vec::new();
                let mut left_filters: Vec<Expr> = Vec::new();
                let mut right_filters: Vec<Expr> = Vec::new();
                for c in conjuncts {
                    if let Some((lk, rk)) = crate::executor::joins::find_equi_join_conditions(
                        c,
                        &left_eval.shape,
                        &right_eval.shape,
                    ) {
                        left_keys.extend(lk);
                        right_keys.extend(rk);
                    } else {
                        match crate::executor::joins::get_expr_side(
                            c,
                            &left_eval.shape,
                            &right_eval.shape,
                        ) {
                            crate::executor::joins::JoinSide::Left => left_filters.push(c.clone()),
                            crate::executor::joins::JoinSide::Right => {
                                right_filters.push(c.clone())
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(lf) = left_filters.into_iter().reduce(|a, b| Expr::Binary {
                    left: Box::new(a),
                    op: BinaryOp::And,
                    right: Box::new(b),
                }) {
                    left_eval.iter = Box::new(from_tree::FilterIterator {
                        source: left_eval.iter,
                        predicate: lf,
                    });
                }
                if let Some(rf) = right_filters.into_iter().reduce(|a, b| Expr::Binary {
                    left: Box::new(a),
                    op: BinaryOp::And,
                    right: Box::new(b),
                }) {
                    right_eval.iter = Box::new(from_tree::FilterIterator {
                        source: right_eval.iter,
                        predicate: rf,
                    });
                }
                let mut shape = left_eval.shape.clone();
                shape.extend(right_eval.shape.clone());
                if !left_keys.is_empty() {
                    return Ok(FromEval {
                        iter: Box::new(from_tree::HashJoinIterator {
                            left: left_eval.iter,
                            right: right_eval.iter,
                            left_keys,
                            right_keys,
                            residual_on: None,
                            join_type: JoinType::Inner,
                            left_shape: left_eval.shape,
                            right_shape: right_eval.shape,
                            build_done: false,
                            right_materialized: Vec::new(),
                            right_raw_keys: Vec::new(),
                            right_key_families: Vec::new(),
                            hash_map: std::collections::HashMap::new(),
                            right_matched: Vec::new(),
                            current_left: None,
                            current_matches: Vec::new(),
                            current_match_idx: 0,
                            matched_current_left: false,
                            finishing_right: false,
                            finishing_idx: 0,
                        }),
                        shape,
                    });
                }
                return Ok(FromEval {
                    iter: Box::new(from_tree::NestedLoopJoinIterator {
                        left: left_eval.iter,
                        right: right_eval.iter,
                        current_left: None,
                        join_type: JoinType::Cross,
                        on: None,
                        right_shape: right_eval.shape,
                        matched_current_left: false,
                    }),
                    shape,
                });
            }
        }
        let pushable_pred = if query.applies.is_empty() {
            query.filter.selection.as_ref()
        } else {
            None
        };
        let allow_unqual = matches!(from_clause, FromNode::Table(_));
        from_tree::execute_from_clause_with_predicate(
            executor,
            from_clause,
            pushable_pred,
            allow_unqual,
            ctx,
        )
    } else {
        Ok(FromEval {
            iter: Box::new(from_tree::ScanIterator::new(vec![vec![]])),
            shape: vec![],
        })
    }
}
