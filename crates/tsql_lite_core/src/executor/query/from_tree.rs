use crate::ast::FromNode;
use crate::catalog::{Catalog, ColumnDef, TableDef};
use crate::error::DbError;
use crate::executor::clock::Clock;
use crate::storage::{Storage, StoredRow};
use crate::types::{DataType, Value};

use super::super::context::ExecutionContext;
use super::super::model::{ContextTable, JoinedRow};
use super::super::physical::{PhysicalPivot, PhysicalScan, PhysicalUnpivot};
pub(crate) use super::pipeline::iterator::{
    FilterIterator, HashJoinIterator, NestedLoopJoinIterator, RowIterator, ScanIterator,
    TableScanIterator,
};
use super::plan::RelationalQuery;
use super::scan::{choose_scan_strategy, execute_scan};
use super::transformer;
use super::QueryExecutor;

pub(crate) struct FromEval {
    pub(crate) iter: Box<dyn RowIterator>,
    pub(crate) shape: Vec<ContextTable>,
    /// Top-level WHERE conjuncts already strictly enforced inside this `FromEval` tree
    /// (on non-null-producing sides of joins or exact same-family `TableScanIterator.fast_filter`),
    /// so `execute_where_stage` and parent joins don't re-evaluate them redundantly.
    pub(crate) enforced_predicates: Vec<crate::ast::Expr>,
}

fn is_exact_same_family(col_type: &DataType, val: &Value) -> bool {
    matches!(
        (col_type, val),
        (
            DataType::Char { .. }
                | DataType::VarChar { .. }
                | DataType::NChar { .. }
                | DataType::NVarChar { .. },
            Value::Char(_) | Value::VarChar(_) | Value::NChar(_) | Value::NVarChar(_)
        ) | (
            DataType::TinyInt | DataType::SmallInt | DataType::Int | DataType::BigInt,
            Value::TinyInt(_) | Value::SmallInt(_) | Value::Int(_) | Value::BigInt(_)
        ) | (DataType::Bit, Value::Bit(_))
    )
}

impl FromEval {
    pub fn materialize(
        &mut self,
        ctx: &mut ExecutionContext,
        catalog: &dyn Catalog,
        storage: &dyn Storage,
        clock: &dyn Clock,
    ) -> Result<Vec<JoinedRow>, DbError> {
        self.materialize_limited(None, ctx, catalog, storage, clock)
    }

    pub fn materialize_limited(
        &mut self,
        limit: Option<usize>,
        ctx: &mut ExecutionContext,
        catalog: &dyn Catalog,
        storage: &dyn Storage,
        clock: &dyn Clock,
    ) -> Result<Vec<JoinedRow>, DbError> {
        if limit == Some(0) {
            return Ok(Vec::new());
        }
        let mut rows = match limit {
            Some(n) => Vec::with_capacity(n.min(1024)),
            None => Vec::new(),
        };
        while let Some(row) = self.iter.next_row(ctx, catalog, storage, clock)? {
            rows.push(row);
            if let Some(cap) = limit {
                if rows.len() >= cap {
                    break;
                }
            }
        }
        Ok(rows)
    }
}

pub(crate) fn execute_from_clause(
    executor: &QueryExecutor<'_>,
    node: &FromNode,
    ctx: &mut ExecutionContext,
) -> Result<FromEval, DbError> {
    execute_from_clause_with_predicate(executor, node, None, true, ctx)
}

pub(crate) fn execute_from_clause_with_predicate(
    executor: &QueryExecutor<'_>,
    node: &FromNode,
    predicate: Option<&crate::ast::Expr>,
    allow_unqualified: bool,
    ctx: &mut ExecutionContext,
) -> Result<FromEval, DbError> {
    match node {
        FromNode::Table(table_ref) => {
            execute_table_ref_with_predicate(executor, table_ref, predicate, allow_unqualified, ctx)
        }
        FromNode::Aliased { source, alias } => {
            let source_eval = execute_from_clause(executor, source, ctx)?;
            apply_from_alias(executor, source_eval, alias, ctx)
        }
        FromNode::Join {
            left,
            join_type,
            right,
            on,
        } => {
            let join_type = *join_type;
            // Propagate single-table WHERE predicates down into safe join inputs so leaf
            // TableScanIterator::fast_filter / IndexSeek can prune rows before join build/probe:
            //   INNER / CROSS: both left & right are safe to pre-filter
            //   LEFT:          left side ONLY
            //   RIGHT:         right side ONLY
            //   FULL:          neither side
            let push_left_pred = match join_type {
                crate::ast::JoinType::Inner
                | crate::ast::JoinType::Cross
                | crate::ast::JoinType::Left => predicate,
                crate::ast::JoinType::Right | crate::ast::JoinType::Full => None,
            };
            let push_right_pred = match join_type {
                crate::ast::JoinType::Inner
                | crate::ast::JoinType::Cross
                | crate::ast::JoinType::Right => predicate,
                crate::ast::JoinType::Left | crate::ast::JoinType::Full => None,
            };

            let mut left_eval =
                execute_from_clause_with_predicate(executor, left, push_left_pred, false, ctx)?;
            let mut right_eval =
                execute_from_clause_with_predicate(executor, right, push_right_pred, false, ctx)?;

            // Push single-side WHERE conjuncts into FilterIterators on the lazy join inputs
            // before HashJoin builds/probes or NestedLoop iterates, skipping conjuncts already
            // strictly enforced by a child's exact `fast_filter` or nested pushdown.
            let mut enforced_predicates = Vec::new();
            if push_left_pred.is_some() {
                enforced_predicates.extend(left_eval.enforced_predicates.iter().cloned());
            }
            if push_right_pred.is_some() {
                enforced_predicates.extend(right_eval.enforced_predicates.iter().cloned());
            }
            if let Some(pred) = predicate {
                let mut conjuncts = Vec::new();
                crate::executor::joins::collect_and_conjuncts(pred, &mut conjuncts);
                let mut left_filters: Vec<crate::ast::Expr> = Vec::new();
                let mut right_filters: Vec<crate::ast::Expr> = Vec::new();
                for c in conjuncts {
                    match crate::executor::joins::get_expr_side(
                        c,
                        &left_eval.shape,
                        &right_eval.shape,
                    ) {
                        crate::executor::joins::JoinSide::Left
                            if push_left_pred.is_some()
                                && !left_eval.enforced_predicates.contains(c) =>
                        {
                            left_filters.push(c.clone());
                            enforced_predicates.push(c.clone());
                        }
                        crate::executor::joins::JoinSide::Right
                            if push_right_pred.is_some()
                                && !right_eval.enforced_predicates.contains(c) =>
                        {
                            right_filters.push(c.clone());
                            enforced_predicates.push(c.clone());
                        }
                        _ => {}
                    }
                }
                if let Some(lf) = left_filters
                    .into_iter()
                    .reduce(|a, b| crate::ast::Expr::Binary {
                        left: Box::new(a),
                        op: crate::ast::BinaryOp::And,
                        right: Box::new(b),
                    })
                {
                    left_eval.iter = Box::new(FilterIterator {
                        source: left_eval.iter,
                        predicate: lf,
                    });
                }
                if let Some(rf) =
                    right_filters
                        .into_iter()
                        .reduce(|a, b| crate::ast::Expr::Binary {
                            left: Box::new(a),
                            op: crate::ast::BinaryOp::And,
                            right: Box::new(b),
                        })
                {
                    right_eval.iter = Box::new(FilterIterator {
                        source: right_eval.iter,
                        predicate: rf,
                    });
                }
            }

            let mut shape = left_eval.shape.clone();
            shape.extend(right_eval.shape.clone());

            // Try Hash Join for pure or mixed equi-joins (with residual ON predicates).
            if let Some(on_expr) = &on {
                if let Some((left_keys, right_keys, residual_on)) =
                    crate::executor::joins::split_equi_join_conditions(
                        on_expr,
                        &left_eval.shape,
                        &right_eval.shape,
                    )
                {
                    return Ok(FromEval {
                        iter: Box::new(HashJoinIterator::new(
                            left_eval.iter,
                            right_eval.iter,
                            left_keys,
                            right_keys,
                            residual_on,
                            join_type,
                            left_eval.shape,
                            right_eval.shape,
                        )),
                        shape,
                        enforced_predicates,
                    });
                }
            } else if join_type == crate::ast::JoinType::Cross {
                // Promote N-way comma-join WHERE equi-predicates (`FROM a, b, c WHERE a.id = b.id AND b.id = c.id`)
                // into chained O(N+M) HashJoinIterators at every join level.
                if let Some(pred) = predicate {
                    let mut conjuncts = Vec::new();
                    crate::executor::joins::collect_and_conjuncts(pred, &mut conjuncts);
                    let mut left_keys = Vec::new();
                    let mut right_keys = Vec::new();
                    for c in conjuncts {
                        if let Some((lk, rk)) = crate::executor::joins::find_equi_join_conditions(
                            c,
                            &left_eval.shape,
                            &right_eval.shape,
                        ) {
                            left_keys.extend(lk);
                            right_keys.extend(rk);
                        }
                    }
                    if !left_keys.is_empty() {
                        return Ok(FromEval {
                            iter: Box::new(HashJoinIterator::new(
                                left_eval.iter,
                                right_eval.iter,
                                left_keys,
                                right_keys,
                                None,
                                crate::ast::JoinType::Inner,
                                left_eval.shape,
                                right_eval.shape,
                            )),
                            shape,
                            enforced_predicates,
                        });
                    }
                }
            }

            // Fallback to Nested Loop Join
            Ok(FromEval {
                iter: Box::new(NestedLoopJoinIterator {
                    left: left_eval.iter,
                    right: right_eval.iter,
                    current_left: None,
                    join_type,
                    on: on.clone(),
                    right_shape: right_eval.shape,
                    matched_current_left: false,
                }),
                shape,
                enforced_predicates,
            })
        }
    }
}

pub(crate) fn enforce_query_governor_cost_limit(
    query: &RelationalQuery<'_>,
    ctx: &ExecutionContext,
) -> Result<(), DbError> {
    let limit = ctx.options.query_governor_cost_limit;
    if limit <= 0 {
        return Ok(());
    }

    let mut cost = 1i64;
    cost += count_joins(query.from_clause.as_deref()) as i64;
    cost += query.applies.len() as i64;
    cost += query.filter.group_by.len() as i64;
    if query.filter.selection.is_some() {
        cost += 1;
    }
    if query.filter.having.is_some() {
        cost += 1;
    }
    if query.projection.distinct {
        cost += 1;
    }
    if !query.sort.order_by.is_empty() {
        cost += 1;
    }
    if query.pagination.top.is_some() {
        cost += 1;
    }
    if query.pagination.offset.is_some() {
        cost += 1;
    }
    if query.pagination.fetch.is_some() {
        cost += 1;
    }

    if cost > limit {
        return Err(DbError::Execution(format!(
            "Query governor cost limit {} exceeded by estimated cost {}",
            limit, cost
        )));
    }

    Ok(())
}

fn execute_table_ref_with_predicate(
    executor: &QueryExecutor<'_>,
    table_ref: &crate::ast::TableRef,
    predicate: Option<&crate::ast::Expr>,
    allow_unqualified: bool,
    ctx: &mut ExecutionContext,
) -> Result<FromEval, DbError> {
    let bound = super::binding::bind_table(executor, executor.catalog, table_ref, ctx)?;

    // Check if it's a CTE
    if let Some(cte) = ctx
        .row
        .ctes
        .get(&crate::executor::string_norm::normalize_identifier(
            &bound.table.name,
        ))
    {
        let rows = crate::executor::cte::cte_to_context_rows(cte, &bound.alias);
        let shape = rows.first().cloned().unwrap_or_else(|| {
            vec![ContextTable {
                table: bound.table.clone(),
                alias: bound.alias.clone(),
                row: None,
                storage_index: None,
                source_aliases: Vec::new(),
            }
            .null_row()]
        });
        return Ok(FromEval {
            iter: Box::new(ScanIterator::new(rows)),
            shape,
            enforced_predicates: Vec::new(),
        });
    }

    // Check if it's virtual rows (VALUES or metadata virtual tables)
    if let Some(rows) = &bound.virtual_rows {
        let ctx_rows: Vec<JoinedRow> = rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                vec![ContextTable {
                    table: bound.table.clone(),
                    alias: bound.alias.clone(),
                    row: Some(std::sync::Arc::new(row.clone())),
                    storage_index: Some(i),
                    source_aliases: Vec::new(),
                }]
            })
            .collect();
        let mut rows = ctx_rows;

        if let Some(pivot) = &table_ref.pivot {
            rows = transformer::execute_pivot(
                executor.catalog,
                executor.storage,
                executor.clock,
                rows,
                &PhysicalPivot {
                    spec: (**pivot).clone(),
                    alias: pivot.alias.clone().unwrap_or_else(|| "pivoted".to_string()),
                },
                ctx,
            )?;
        }

        if let Some(unpivot) = &table_ref.unpivot {
            let unpivot_alias = unpivot
                .alias
                .clone()
                .unwrap_or_else(|| "unpivoted".to_string());
            let mut source_aliases: Vec<String> = rows
                .first()
                .map(|r| {
                    r.iter()
                        .flat_map(|ct| {
                            let mut aliases = vec![ct.alias.to_string()];
                            if !ct.alias.eq_ignore_ascii_case(&ct.table.name) {
                                aliases.push(ct.table.name.clone());
                            }
                            aliases
                        })
                        .collect()
                })
                .unwrap_or_default();
            if let Some(sa) = &unpivot.source_alias {
                if !source_aliases.iter().any(|a| a.eq_ignore_ascii_case(sa)) {
                    source_aliases.push(sa.clone());
                }
            }
            rows = transformer::execute_unpivot(
                rows,
                &PhysicalUnpivot {
                    spec: (**unpivot).clone(),
                    alias: unpivot_alias,
                    source_aliases,
                },
                ctx,
            )?;
        }

        let shape = rows.first().cloned().unwrap_or_else(|| {
            vec![ContextTable {
                table: bound.table.clone(),
                alias: bound.alias.clone(),
                row: None,
                storage_index: None,
                source_aliases: Vec::new(),
            }
            .null_row()]
        });
        return Ok(FromEval {
            iter: Box::new(ScanIterator::new(rows)),
            shape,
            enforced_predicates: Vec::new(),
        });
    }

    let strategy = if executor.storage.as_index_storage().is_some() {
        choose_scan_strategy(&bound, predicate, &[], executor.catalog)
    } else {
        crate::executor::physical::ScanStrategy::TableScan
    };

    // Only use truly lazy TableScanIterator if it's a simple TableScan strategy
    // and we don't have complex PIVOT/UNPIVOT (which still materialize for now)
    if matches!(strategy, crate::executor::physical::ScanStrategy::TableScan)
        && table_ref.pivot.is_none()
        && table_ref.unpivot.is_none()
    {
        let mut enforced_predicates = Vec::new();
        let fast_filter = if ctx.options.ansi_nulls {
            predicate.and_then(|pred| {
                let (col_idx, op, rhs_expr) =
                    super::scan::extract_fast_scan_predicate(pred, &bound, allow_unqualified)?;
                let rhs_val = crate::executor::evaluator::eval_expr(
                    &rhs_expr,
                    &[],
                    ctx,
                    executor.catalog,
                    executor.storage,
                    executor.clock,
                )
                .ok()?;
                if rhs_val.is_null() {
                    return None;
                }
                if let Some(col) = bound.table.columns.get(col_idx) {
                    if is_exact_same_family(&col.data_type, &rhs_val) {
                        let mut conjuncts = Vec::new();
                        crate::executor::joins::collect_and_conjuncts(pred, &mut conjuncts);
                        for c in conjuncts {
                            if let Some((ci, cop, crhs)) = super::scan::extract_fast_scan_predicate(
                                c,
                                &bound,
                                allow_unqualified,
                            ) {
                                if ci == col_idx && cop == op && crhs == rhs_expr {
                                    enforced_predicates.push(c.clone());
                                    break;
                                }
                            }
                        }
                    }
                }
                Some((col_idx, op, rhs_val))
            })
        } else {
            None
        };
        let table_iter = TableScanIterator {
            bound: bound.clone(),
            next_index: 0,
            fast_filter,
        };

        let shape = vec![ContextTable {
            table: bound.table.clone(),
            alias: bound.alias.clone(),
            row: None,
            storage_index: None,
            source_aliases: Vec::new(),
        }
        .null_row()];

        return Ok(FromEval {
            iter: Box::new(table_iter),
            shape,
            enforced_predicates,
        });
    }

    // Fallback to materialized execute_scan for complex cases (IndexSeek, Pivot, etc.)
    let base_shape = vec![ContextTable {
        table: bound.table.clone(),
        alias: bound.alias.clone(),
        row: None,
        storage_index: None,
        source_aliases: Vec::new(),
    }
    .null_row()];
    let scan = PhysicalScan {
        bound,
        strategy,
        pushed_predicate: predicate.cloned(),
    };
    let mut rows = execute_scan(
        &scan,
        ctx,
        executor.catalog,
        executor.storage,
        executor.clock,
    )?;

    if let Some(pivot) = &table_ref.pivot {
        rows = transformer::execute_pivot(
            executor.catalog,
            executor.storage,
            executor.clock,
            rows,
            &PhysicalPivot {
                spec: (**pivot).clone(),
                alias: pivot.alias.clone().unwrap_or_else(|| "pivoted".to_string()),
            },
            ctx,
        )?;
    }

    if let Some(unpivot) = &table_ref.unpivot {
        let unpivot_alias = unpivot
            .alias
            .clone()
            .unwrap_or_else(|| "unpivoted".to_string());
        let mut source_aliases: Vec<String> = rows
            .first()
            .map(|r| {
                r.iter()
                    .flat_map(|ct| {
                        let mut aliases = vec![ct.alias.to_string()];
                        if !ct.alias.eq_ignore_ascii_case(&ct.table.name) {
                            aliases.push(ct.table.name.clone());
                        }
                        aliases
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(sa) = &unpivot.source_alias {
            if !source_aliases.iter().any(|a| a.eq_ignore_ascii_case(sa)) {
                source_aliases.push(sa.clone());
            }
        }
        rows = transformer::execute_unpivot(
            rows,
            &PhysicalUnpivot {
                spec: (**unpivot).clone(),
                alias: unpivot_alias,
                source_aliases,
            },
            ctx,
        )?;
    }

    let shape = rows.first().cloned().unwrap_or(base_shape);
    Ok(FromEval {
        iter: Box::new(ScanIterator::new(rows)),
        shape,
        enforced_predicates: Vec::new(),
    })
}

fn apply_from_alias(
    executor: &QueryExecutor<'_>,
    mut source: FromEval,
    alias: &str,
    ctx: &mut ExecutionContext,
) -> Result<FromEval, DbError> {
    let mut columns = Vec::new();
    for ctx_table in &source.shape {
        for col in &ctx_table.table.columns {
            columns.push(ColumnDef {
                id: (columns.len() + 1) as u32,
                name: col.name.clone(),
                data_type: col.data_type.clone(),
                nullable: true,
                primary_key: false,
                unique: false,
                identity: None,
                default: None,
                default_constraint_name: None,
                check: None,
                check_constraint_name: None,
                computed_expr: None,
                collation: None,
                is_clustered: false,
                ansi_padding_on: true,
                is_rowversion: false,
            });
        }
    }
    let alias_table = std::sync::Arc::new(TableDef {
        id: 0,
        schema_id: 1,
        schema_name: "dbo".to_string(),
        name: alias.to_string(),
        columns,
        check_constraints: vec![],
        foreign_keys: vec![],
    });

    let source_rows =
        source.materialize(ctx, executor.catalog, executor.storage, executor.clock)?;
    let mut aliased_rows = Vec::with_capacity(source_rows.len());
    for row in source_rows {
        let mut values = Vec::new();
        for ctx_table in &row {
            if let Some(stored) = &ctx_table.row {
                values.extend(stored.values.clone());
            } else {
                values
                    .extend((0..ctx_table.table.columns.len()).map(|_| crate::types::Value::Null));
            }
        }
        let alias_arc: std::sync::Arc<str> = std::sync::Arc::from(alias);
        aliased_rows.push(vec![ContextTable {
            table: alias_table.clone(),
            alias: std::sync::Arc::clone(&alias_arc),
            row: Some(std::sync::Arc::new(StoredRow {
                values,
                deleted: false,
            })),
            storage_index: None,
            source_aliases: Vec::new(),
        }]);
    }

    let shape = vec![ContextTable {
        table: alias_table,
        alias: std::sync::Arc::from(alias),
        row: None,
        storage_index: None,
        source_aliases: Vec::new(),
    }
    .null_row()];

    Ok(FromEval {
        iter: Box::new(ScanIterator::new(aliased_rows)),
        shape,
        enforced_predicates: Vec::new(),
    })
}

fn count_joins(from_clause: Option<&FromNode>) -> usize {
    match from_clause {
        None | Some(FromNode::Table(_)) => 0,
        Some(FromNode::Aliased { source, .. }) => count_joins(Some(source)),
        Some(FromNode::Join { left, right, .. }) => {
            1 + count_joins(Some(left)) + count_joins(Some(right))
        }
    }
}
