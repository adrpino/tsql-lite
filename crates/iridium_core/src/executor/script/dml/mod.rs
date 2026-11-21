pub(crate) mod cte;
pub(crate) mod merge;
mod merge_helpers;

use super::ScriptExecutor;
use crate::ast::{DeleteStmt, InsertStmt, UpdateStmt};
use crate::catalog::{Catalog, TableDef};
use crate::error::DbError;
use crate::executor::context::ExecutionContext;
use crate::executor::mutation::MutationExecutor;
use crate::executor::query::plan::RelationalQuery;
use crate::executor::query::QueryExecutor;
use crate::executor::result::QueryResult;
use crate::storage::{Storage, StoredRow};

fn take_leftmost_into_table(
    stmt: &mut crate::ast::Statement,
) -> Option<crate::ast::ObjectName> {
    match stmt {
        crate::ast::Statement::Dml(crate::ast::DmlStatement::Select(s)) => s.into_table.take(),
        crate::ast::Statement::Dml(crate::ast::DmlStatement::SetOp(op)) => {
            take_leftmost_into_table(&mut op.left)
        }
        _ => None,
    }
}

impl<'a> ScriptExecutor<'a> {
    pub(crate) fn execute_dml(
        &mut self,
        dml: crate::ast::DmlStatement,
        ctx: &mut ExecutionContext<'_>,
    ) -> crate::error::StmtResult<Option<QueryResult>> {
        use crate::ast::DmlStatement;
        use crate::error::StmtOutcome;
        match dml {
            DmlStatement::Insert(stmt) => self.execute_insert(stmt, ctx).map(StmtOutcome::Ok),
            DmlStatement::Select(stmt) => self.execute_select_into(stmt, ctx).map(StmtOutcome::Ok),
            DmlStatement::Update(stmt) => self.execute_update(stmt, ctx).map(StmtOutcome::Ok),
            DmlStatement::Delete(stmt) => self.execute_delete(stmt, ctx).map(StmtOutcome::Ok),
            DmlStatement::Merge(stmt) => self.execute_merge(stmt, ctx).map(StmtOutcome::Ok),
            DmlStatement::SelectAssign(stmt) => {
                self.execute_select_assign(stmt, ctx).map(StmtOutcome::Ok)
            }
            DmlStatement::BulkInsert(stmt) => {
                self.execute_bulk_insert(stmt, ctx).map(StmtOutcome::Ok)
            }
            DmlStatement::InsertBulk(stmt) => {
                self.execute_insert_bulk(stmt, ctx).map(StmtOutcome::Ok)
            }
            DmlStatement::SetOp(mut stmt) => {
                // In T-SQL, `SELECT ... INTO #tbl FROM a UNION [ALL] SELECT ... FROM b`
                // attaches INTO to the leftmost SELECT; lift it so the combined result
                // populates the target table.
                let into_table = take_leftmost_into_table(&mut stmt.left);
                let left_outcome = self.execute(*stmt.left, ctx)?;
                let right_outcome = self.execute(*stmt.right, ctx)?;

                match (left_outcome, right_outcome) {
                    (StmtOutcome::Ok(Some(left)), StmtOutcome::Ok(Some(right))) => {
                        let result = crate::executor::engine::execute_set_op(left, right, stmt.op)?;
                        ctx.set_last_rowcount(result.rows.len());
                        if let Some(target) = into_table {
                            self.materialize_select_into_table(target, &result, ctx)?;
                            return Ok(StmtOutcome::Ok(None));
                        }
                        if !result.columns.is_empty() {
                            ctx.frame.last_proc_result = Some(result.clone());
                            if let Some(ref sink) = ctx.frame.result_set_sink {
                                sink.lock().push(result.clone());
                            }
                        }
                        Ok(StmtOutcome::Ok(Some(result)))
                    }
                    (StmtOutcome::Break, _) | (_, StmtOutcome::Break) => Ok(StmtOutcome::Break),
                    (StmtOutcome::Continue, _) | (_, StmtOutcome::Continue) => {
                        Ok(StmtOutcome::Continue)
                    }
                    (StmtOutcome::Return(v), _) | (_, StmtOutcome::Return(v)) => {
                        Ok(StmtOutcome::Return(v))
                    }
                    _ => Err(DbError::Execution(
                        "set operations require both sides to return results".into(),
                    )),
                }
            }
        }
    }

    pub(crate) fn execute_insert(
        &mut self,
        stmt: InsertStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        if ctx.is_readonly_table_var(&stmt.table.name) {
            return Err(DbError::Execution(format!(
                "table-valued parameter '{}' is READONLY",
                stmt.table.name
            )));
        }
        let mut mut_exec = MutationExecutor {
            catalog: self.catalog,
            storage: self.storage,
            clock: self.clock,
        };
        mut_exec.execute_insert_with_context(stmt, ctx)
    }

    pub(crate) fn execute_select_into(
        &mut self,
        mut stmt: crate::ast::SelectStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        let into_table = stmt.into_table.take();
        let result = QueryExecutor {
            catalog: self.catalog as &dyn Catalog,
            storage: self.storage as &dyn Storage,
            clock: self.clock,
        }
        .execute_select(RelationalQuery::from(stmt), ctx)?;

        ctx.set_last_rowcount(result.rows.len());

        if into_table.is_none() && !result.columns.is_empty() {
            ctx.frame.last_proc_result = Some(result.clone());
            if let Some(ref sink) = ctx.frame.result_set_sink {
                sink.lock().push(result.clone());
            }
        }

        if let Some(target) = into_table {
            self.materialize_select_into_table(target, &result, ctx)?;
            return Ok(None);
        }

        Ok(Some(result))
    }

    fn materialize_select_into_table(
        &mut self,
        mut target: crate::ast::ObjectName,
        result: &QueryResult,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<(), DbError> {
        if target.name.starts_with('#') {
            let logical = target.name.clone();
            let physical = ctx.register_temp_table(&logical);
            target.schema = Some("dbo".to_string());
            target.name = physical;
        }

        let schema_name = target.schema_or_dbo();
        if self.catalog.find_table(schema_name, &target.name).is_some() {
            return Err(DbError::duplicate_table(schema_name, &target.name));
        }

        let schema_id = self
            .catalog
            .get_schema_id(schema_name)
            .ok_or_else(|| DbError::schema_not_found(schema_name))?;

        let mut columns = Vec::new();
        for (i, name) in result.columns.iter().enumerate() {
            columns.push(crate::catalog::ColumnDef {
                id: self.catalog.alloc_column_id(),
                name: name.clone(),
                data_type: result.column_types[i].clone(),
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
            });
        }

        let table_id = self.catalog.alloc_table_id();
        let table = TableDef {
            id: table_id,
            schema_id,
            schema_name: schema_name.to_string(),
            name: target.name.clone(),
            columns,
            check_constraints: vec![],
            foreign_keys: vec![],
        };
        self.catalog.register_table(table);
        self.storage.ensure_table(table_id)?;

        for row_values in &result.rows {
            let row = StoredRow {
                values: row_values.clone(),
                deleted: false,
            };
            self.storage.insert_row(table_id, row.clone())?;
            if let Some(db) = &ctx.session.dirty_buffer {
                db.lock().push_op(
                    ctx.session_id(),
                    target.name.to_string(),
                    crate::executor::dirty_buffer::DirtyOp::Insert { row: row.clone() },
                );
            }
        }
        Ok(())
    }

    pub(crate) fn execute_update(
        &mut self,
        stmt: UpdateStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        if ctx.is_readonly_table_var(&stmt.table.name) {
            return Err(DbError::Execution(format!(
                "table-valued parameter '{}' is READONLY",
                stmt.table.name
            )));
        }
        let mut mut_exec = MutationExecutor {
            catalog: self.catalog,
            storage: self.storage,
            clock: self.clock,
        };
        mut_exec.execute_update_with_context(stmt, ctx)
    }

    pub(crate) fn execute_delete(
        &mut self,
        stmt: DeleteStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        if ctx.is_readonly_table_var(&stmt.table.name) {
            return Err(DbError::Execution(format!(
                "table-valued parameter '{}' is READONLY",
                stmt.table.name
            )));
        }
        let mut mut_exec = MutationExecutor {
            catalog: self.catalog,
            storage: self.storage,
            clock: self.clock,
        };
        mut_exec.execute_delete_with_context(stmt, ctx)
    }

    pub(crate) fn execute_bulk_insert(
        &mut self,
        stmt: crate::ast::BulkInsertStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        let mut mut_exec = MutationExecutor {
            catalog: self.catalog,
            storage: self.storage,
            clock: self.clock,
        };
        mut_exec.execute_bulk_insert(stmt, ctx)
    }

    pub(crate) fn execute_insert_bulk(
        &mut self,
        stmt: crate::ast::InsertBulkStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<QueryResult>, DbError> {
        let mut mut_exec = MutationExecutor {
            catalog: self.catalog,
            storage: self.storage,
            clock: self.clock,
        };
        mut_exec.execute_insert_bulk(stmt, ctx)
    }
}
