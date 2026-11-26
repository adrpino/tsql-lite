use super::super::ScriptExecutor;
use crate::ast::WithCteStmt;
use crate::catalog::TableDef;
use crate::error::{DbError, StmtOutcome};
use crate::executor::context::ExecutionContext;
use crate::executor::result::QueryResult;
use crate::storage::StoredRow;

fn from_node_references_name(node: &crate::ast::FromNode, target: &str) -> bool {
    match node {
        crate::ast::FromNode::Table(tr) => match &tr.factor {
            crate::ast::TableFactor::Named(obj) => obj.name.eq_ignore_ascii_case(target),
            crate::ast::TableFactor::Derived(sub) => select_references_name(sub, target),
            crate::ast::TableFactor::Values { .. } => false,
        },
        crate::ast::FromNode::Aliased { source, .. } => from_node_references_name(source, target),
        crate::ast::FromNode::Join { left, right, .. } => {
            from_node_references_name(left, target) || from_node_references_name(right, target)
        }
    }
}

fn select_references_name(sel: &crate::ast::SelectStmt, target: &str) -> bool {
    if let Some(ref from) = sel.from_clause {
        if from_node_references_name(from, target) {
            return true;
        }
    }
    if sel
        .applies
        .iter()
        .any(|a| select_references_name(&a.subquery, target))
    {
        return true;
    }
    if let Some(ref set_op) = sel.set_op {
        if select_references_name(&set_op.right, target) {
            return true;
        }
    }
    false
}

fn extract_recursive_cte_parts(
    cte_name: &str,
    query: &crate::ast::Statement,
    explicit_recursive: bool,
) -> Option<(crate::ast::Statement, crate::ast::Statement)> {
    match query {
        crate::ast::Statement::Dml(crate::ast::DmlStatement::SetOp(set_op))
            if matches!(set_op.op, crate::ast::SetOpKind::UnionAll) =>
        {
            if explicit_recursive {
                return Some((*set_op.left.clone(), *set_op.right.clone()));
            }
            if let crate::ast::Statement::Dml(crate::ast::DmlStatement::Select(right_sel)) =
                set_op.right.as_ref()
            {
                if select_references_name(right_sel, cte_name) {
                    return Some((*set_op.left.clone(), *set_op.right.clone()));
                }
            }
            None
        }
        crate::ast::Statement::Dml(crate::ast::DmlStatement::Select(sel)) => {
            let set_op = sel.set_op.as_ref()?;
            if !matches!(set_op.kind, crate::ast::SetOpKind::UnionAll) {
                return None;
            }
            if !explicit_recursive && !select_references_name(&set_op.right, cte_name) {
                return None;
            }
            let mut anchor = sel.clone();
            anchor.set_op = None;
            let recursive = set_op.right.clone();
            Some((
                crate::ast::Statement::Dml(crate::ast::DmlStatement::Select(anchor)),
                crate::ast::Statement::Dml(crate::ast::DmlStatement::Select(Box::new(recursive))),
            ))
        }
        _ => None,
    }
}

fn extract_max_recursion(body: &crate::ast::Statement) -> Option<u16> {
    match body {
        crate::ast::Statement::Dml(crate::ast::DmlStatement::Select(sel)) => sel.max_recursion(),
        crate::ast::Statement::Dml(crate::ast::DmlStatement::SelectAssign(sa)) => sa.max_recursion,
        crate::ast::Statement::Dml(crate::ast::DmlStatement::Insert(ins)) => {
            if let crate::ast::InsertSource::Select(q) = &ins.source {
                q.max_recursion()
            } else {
                None
            }
        }
        _ => None,
    }
}

impl<'a> ScriptExecutor<'a> {
    pub(crate) fn execute_with_cte(
        &mut self,
        stmt: &WithCteStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        let old_ctes = ctx.row.ctes.clone();
        // SQL Server default MAXRECURSION is 100; OPTION (MAXRECURSION 0) allows unlimited recursion.
        let requested_max_recursion = extract_max_recursion(&stmt.body);
        let max_recursion: usize = match requested_max_recursion {
            Some(0) => 0,
            Some(n) => n as usize,
            None => 100,
        };
        const UNLIMITED_SAFETY_CAP: usize = 100_000;

        for cte_def in &stmt.ctes {
            let recursive_parts =
                extract_recursive_cte_parts(&cte_def.name, &cte_def.query, stmt.recursive);
            if let Some((anchor_stmt, recursive_stmt)) = recursive_parts {
                let saved_sink = ctx.frame.result_set_sink.take();
                let saved_last = ctx.frame.last_proc_result.take();
                let anchor_outcome = self.execute(anchor_stmt, ctx);
                ctx.frame.result_set_sink = saved_sink;
                ctx.frame.last_proc_result = saved_last;
                let anchor_outcome = anchor_outcome?;
                let result = match anchor_outcome {
                    StmtOutcome::Ok(Some(r)) => r,
                    StmtOutcome::Ok(None) => {
                        return Err(DbError::Execution(
                            "CTE anchor member must return a result set".into(),
                        ))
                    }
                    other => return other.into_result(),
                };

                let table_def = TableDef {
                    id: 0,
                    schema_id: 1,
                    schema_name: "dbo".to_string(),
                    name: cte_def.name.clone(),
                    columns: result
                        .columns
                        .iter()
                        .enumerate()
                        .map(|(i, name)| crate::catalog::ColumnDef {
                            id: (i + 1) as u32,
                            name: cte_def.columns.get(i).unwrap_or(name).clone(),
                            data_type: result.column_types[i].clone(),
                            nullable: true,
                            primary_key: false,
                            unique: false,
                            identity: None,
                            default: None,
                            default_constraint_name: None,
                            computed_expr: None,
                            check: None,
                            check_constraint_name: None,
                            collation: None,
                            is_clustered: false,
                            ansi_padding_on: true,
                            is_rowversion: false,
                        })
                        .collect(),
                    check_constraints: vec![],
                    foreign_keys: vec![],
                    unique_constraints: vec![],
                };

                let mut all_rows: Vec<StoredRow> = result
                    .rows
                    .iter()
                    .map(|v| StoredRow {
                        values: v.clone(),
                        deleted: false,
                    })
                    .collect();
                let mut working_set = all_rows.clone();

                let mut iteration = 0usize;
                let table_def_inner = table_def.clone();

                while !working_set.is_empty() {
                    let mut iteration_ctes = ctx.row.ctes.clone();
                    iteration_ctes.insert(&cte_def.name, table_def_inner.clone(), working_set);
                    let prev_ctes = std::mem::replace(&mut ctx.row.ctes, iteration_ctes);
                    let saved_sink = ctx.frame.result_set_sink.take();
                    let saved_last = ctx.frame.last_proc_result.take();
                    let step_outcome = self.execute_ref(&recursive_stmt, ctx);
                    ctx.frame.result_set_sink = saved_sink;
                    ctx.frame.last_proc_result = saved_last;
                    ctx.row.ctes = prev_ctes;
                    let step_outcome = step_outcome?;

                    let res = match step_outcome {
                        StmtOutcome::Ok(Some(r)) => r,
                        StmtOutcome::Ok(None) => break,
                        other => return other.into_result(),
                    };

                    if res.rows.is_empty() {
                        break;
                    }

                    iteration += 1;
                    if max_recursion > 0 && iteration > max_recursion {
                        return Err(DbError::Custom {
                            class: 16,
                            number: 530,
                            message: format!(
                                "The statement terminated. The maximum recursion {} has been exhausted before statement completion.",
                                max_recursion
                            ),
                        });
                    }
                    if max_recursion == 0 && iteration > UNLIMITED_SAFETY_CAP {
                        return Err(DbError::Custom {
                            class: 16,
                            number: 530,
                            message: format!(
                                "The statement terminated. The maximum recursion {} has been exhausted before statement completion.",
                                UNLIMITED_SAFETY_CAP
                            ),
                        });
                    }

                    working_set = res
                        .rows
                        .iter()
                        .map(|v| StoredRow {
                            values: v.clone(),
                            deleted: false,
                        })
                        .collect();
                    all_rows.extend(working_set.iter().cloned());
                }

                ctx.row.ctes.insert(&cte_def.name, table_def, all_rows);
            } else if stmt.recursive {
                return Err(DbError::Execution(
                    "Recursive CTE must be a UNION ALL of anchor and recursive members".into(),
                ));
            } else {
                // Standard non-recursive CTE
                let track_base = matches!(
                    stmt.body.as_ref(),
                    crate::ast::Statement::Dml(
                        crate::ast::DmlStatement::Update(_)
                            | crate::ast::DmlStatement::Delete(_)
                            | crate::ast::DmlStatement::Merge(_)
                    )
                );
                let (result, base_targets) = if track_base {
                    if let crate::ast::Statement::Dml(crate::ast::DmlStatement::Select(
                        select_stmt,
                    )) = &cte_def.query
                    {
                        let qe = crate::executor::query::QueryExecutor {
                            catalog: self.catalog,
                            storage: self.storage,
                            clock: self.clock,
                        };
                        qe.execute_select_with_base_targets(
                            crate::executor::query::plan::RelationalQuery::from_ref(select_stmt),
                            ctx,
                        )?
                    } else {
                        let saved_sink = ctx.frame.result_set_sink.take();
                        let saved_last = ctx.frame.last_proc_result.take();
                        let cte_outcome = self.execute_ref(&cte_def.query, ctx);
                        ctx.frame.result_set_sink = saved_sink;
                        ctx.frame.last_proc_result = saved_last;
                        let cte_outcome = cte_outcome?;
                        let r = match cte_outcome {
                            StmtOutcome::Ok(Some(r)) => r,
                            StmtOutcome::Ok(None) => {
                                return Err(DbError::Execution(
                                    "CTE query must return a result set".into(),
                                ))
                            }
                            other => return other.into_result(),
                        };
                        (r, Vec::new())
                    }
                } else {
                    let saved_sink = ctx.frame.result_set_sink.take();
                    let saved_last = ctx.frame.last_proc_result.take();
                    let cte_outcome = self.execute_ref(&cte_def.query, ctx);
                    ctx.frame.result_set_sink = saved_sink;
                    ctx.frame.last_proc_result = saved_last;
                    let cte_outcome = cte_outcome?;
                    let r = match cte_outcome {
                        StmtOutcome::Ok(Some(r)) => r,
                        StmtOutcome::Ok(None) => {
                            return Err(DbError::Execution(
                                "CTE query must return a result set".into(),
                            ))
                        }
                        // Propagate control flow
                        other => return other.into_result(),
                    };
                    (r, Vec::new())
                };

                let table_def = TableDef {
                    id: 0,
                    schema_id: 1,
                    schema_name: "dbo".to_string(),
                    name: cte_def.name.clone(),
                    columns: result
                        .columns
                        .iter()
                        .enumerate()
                        .map(|(i, name)| crate::catalog::ColumnDef {
                            id: (i + 1) as u32,
                            name: cte_def.columns.get(i).unwrap_or(name).clone(),
                            data_type: result.column_types[i].clone(),
                            nullable: true,
                            primary_key: false,
                            unique: false,
                            identity: None,
                            default: None,
                            default_constraint_name: None,
                            computed_expr: None,
                            check: None,
                            check_constraint_name: None,
                            collation: None,
                            is_clustered: false,
                            ansi_padding_on: true,
                            is_rowversion: false,
                        })
                        .collect(),
                    check_constraints: vec![],
                    foreign_keys: vec![],
                    unique_constraints: vec![],
                };

                let rows: Vec<StoredRow> = result
                    .rows
                    .into_iter()
                    .map(|values| StoredRow {
                        values,
                        deleted: false,
                    })
                    .collect();

                ctx.row
                    .ctes
                    .insert_with_base(&cte_def.name, table_def, rows, base_targets);
            }
        }

        let res = self.execute_ref(&stmt.body, ctx);
        ctx.row.ctes = old_ctes;
        // Propagate control flow from the body; unwrap Ok values
        res.map(|outcome| outcome.into_result()).and_then(|r| r)
    }
}
