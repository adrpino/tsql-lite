use std::collections::HashMap;

use crate::ast::{DmlStatement, FromNode, ObjectName, SelectStmt, Statement, TableRef};
use crate::error::DbError;
use crate::executor::string_norm::normalize_identifier;

/// Parsed SQL Server table-level locking hints from `WITH (...)` / `(NOLOCK)` clauses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableLockHints {
    pub nolock: bool,
    pub readcommittedlock: bool,
    pub readpast: bool,
    pub updlock: bool,
    pub xlock: bool,
    pub holdlock: bool,
    pub rowlock: bool,
    pub tablock: bool,
    pub nowait: bool,
}

impl TableLockHints {
    pub fn from_raw(raw_hints: &[String]) -> Self {
        let mut out = Self::default();
        for raw in raw_hints {
            match raw.trim().to_ascii_uppercase().as_str() {
                "NOLOCK" | "READUNCOMMITTED" => out.nolock = true,
                "READCOMMITTEDLOCK" => out.readcommittedlock = true,
                "READPAST" => out.readpast = true,
                "UPDLOCK" => out.updlock = true,
                "XLOCK" => out.xlock = true,
                "HOLDLOCK" | "SERIALIZABLE" | "REPEATABLEREAD" => out.holdlock = true,
                "ROWLOCK" => out.rowlock = true,
                "TABLOCK" => out.tablock = true,
                "TABLOCKX" => {
                    out.tablock = true;
                    out.xlock = true;
                }
                "NOWAIT" => out.nowait = true,
                _ => {}
            }
        }
        out
    }

    pub fn merge_from(&mut self, other: &TableLockHints) {
        self.nolock |= other.nolock;
        self.readcommittedlock |= other.readcommittedlock;
        self.readpast |= other.readpast;
        self.updlock |= other.updlock;
        self.xlock |= other.xlock;
        self.holdlock |= other.holdlock;
        self.rowlock |= other.rowlock;
        self.tablock |= other.tablock;
        self.nowait |= other.nowait;
    }

    pub fn validate(&self, table_display: &str) -> Result<(), DbError> {
        if self.nolock
            && (self.holdlock
                || self.updlock
                || self.xlock
                || self.readpast
                || self.readcommittedlock)
        {
            return Err(DbError::Execution(format!(
                "Conflicting locking hints are specified for table \"{}\". This may be caused by a conflicting hint specified for a view.",
                table_display
            )));
        }
        if self.readpast && self.tablock {
            return Err(DbError::Execution(format!(
                "Conflicting locking hints are specified for table \"{}\": READPAST cannot be combined with TABLOCK or TABLOCKX.",
                table_display
            )));
        }
        Ok(())
    }

    pub fn requires_write_row_lock(&self) -> bool {
        self.updlock || self.xlock
    }

    pub fn requires_any_row_lock_held(&self) -> bool {
        self.updlock || self.xlock || self.holdlock
    }

    pub fn uses_row_locking_path(&self) -> bool {
        self.rowlock || self.readpast || (self.updlock && !self.tablock)
    }

    pub fn is_empty(&self) -> bool {
        !self.nolock
            && !self.readcommittedlock
            && !self.readpast
            && !self.updlock
            && !self.xlock
            && !self.holdlock
            && !self.rowlock
            && !self.tablock
            && !self.nowait
    }
}

/// Canonical table key used consistently by `LockTable` across statement-level
/// locking, row-level locking, and multi-database isolation.
pub(crate) fn canonical_lock_key_from_parts(
    schema_raw: &str,
    table_name: &str,
    active_db: &str,
) -> String {
    let (owner_db, display_schema) = crate::executor::database_catalog::split_db_schema(schema_raw);
    let db_part = owner_db.unwrap_or(active_db).to_ascii_lowercase();
    format!(
        "{}.{}.{}",
        db_part,
        normalize_identifier(display_schema),
        normalize_identifier(table_name)
    )
}

pub(crate) fn canonical_lock_key_from_obj(obj: &ObjectName, active_db: &str) -> String {
    let db_part = obj.database.as_deref().unwrap_or(active_db);
    canonical_lock_key_from_parts(obj.schema_or_dbo(), &obj.name, db_part)
}

/// Collects and validates per-table `TableLockHints` referenced by `stmt`.
/// Keys are canonical lock keys (`db.schema.table` lowercased).
pub(crate) fn collect_and_validate_statement_hints(
    stmt: &Statement,
    active_db: &str,
) -> Result<HashMap<String, TableLockHints>, DbError> {
    let mut map: HashMap<String, TableLockHints> = HashMap::new();
    collect_stmt_hints(stmt, active_db, &mut map)?;
    Ok(map)
}

fn record_table_ref_hints(
    tref: &TableRef,
    active_db: &str,
    out: &mut HashMap<String, TableLockHints>,
) -> Result<(), DbError> {
    if tref.hints.is_empty() {
        return Ok(());
    }
    let parsed = TableLockHints::from_raw(&tref.hints);
    if let Some(obj) = tref.name_as_object() {
        let key = canonical_lock_key_from_obj(obj, active_db);
        let entry = out.entry(key).or_default();
        entry.merge_from(&parsed);
        entry.validate(&obj.name)?;
    }
    Ok(())
}

fn collect_from_node_hints(
    node: &FromNode,
    active_db: &str,
    out: &mut HashMap<String, TableLockHints>,
) -> Result<(), DbError> {
    match node {
        FromNode::Table(tref) => record_table_ref_hints(tref, active_db, out),
        FromNode::Aliased { source, .. } => collect_from_node_hints(source, active_db, out),
        FromNode::Join { left, right, .. } => {
            collect_from_node_hints(left, active_db, out)?;
            collect_from_node_hints(right, active_db, out)
        }
    }
}

fn collect_select_hints(
    select: &SelectStmt,
    active_db: &str,
    out: &mut HashMap<String, TableLockHints>,
) -> Result<(), DbError> {
    if let Some(ref from) = select.from_clause {
        collect_from_node_hints(from, active_db, out)?;
    }
    if let Some(ref set_op) = select.set_op {
        collect_select_hints(&set_op.right, active_db, out)?;
    }
    Ok(())
}

fn collect_stmt_hints(
    stmt: &Statement,
    active_db: &str,
    out: &mut HashMap<String, TableLockHints>,
) -> Result<(), DbError> {
    match stmt {
        Statement::Dml(DmlStatement::Select(s)) => collect_select_hints(s, active_db, out),
        Statement::Dml(DmlStatement::SetOp(s)) => {
            collect_stmt_hints(&s.left, active_db, out)?;
            collect_stmt_hints(&s.right, active_db, out)
        }
        Statement::Dml(DmlStatement::SelectAssign(s)) => {
            if let Some(ref f) = s.from {
                record_table_ref_hints(f, active_db, out)?;
            }
            for j in &s.joins {
                record_table_ref_hints(&j.table, active_db, out)?;
            }
            Ok(())
        }
        Statement::Dml(DmlStatement::Update(s)) => {
            if let Some(ref fc) = s.from {
                for t in &fc.tables {
                    record_table_ref_hints(t, active_db, out)?;
                }
                for j in &fc.joins {
                    record_table_ref_hints(&j.table, active_db, out)?;
                }
            }
            Ok(())
        }
        Statement::Dml(DmlStatement::Delete(s)) => {
            if let Some(ref fc) = s.from {
                for t in &fc.tables {
                    record_table_ref_hints(t, active_db, out)?;
                }
                for j in &fc.joins {
                    record_table_ref_hints(&j.table, active_db, out)?;
                }
            }
            Ok(())
        }
        Statement::Dml(DmlStatement::Merge(s)) => {
            record_table_ref_hints(&s.target, active_db, out)?;
            match &s.source {
                crate::ast::MergeSource::Table(tr) => record_table_ref_hints(tr, active_db, out),
                crate::ast::MergeSource::Subquery(sel, _) => {
                    collect_select_hints(sel, active_db, out)
                }
            }
        }
        Statement::WithCte(cte_stmt) => {
            for cte in &cte_stmt.ctes {
                collect_stmt_hints(&cte.query, active_db, out)?;
            }
            collect_stmt_hints(&cte_stmt.body, active_db, out)
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_worker_queue_hints() {
        let h = TableLockHints::from_raw(&["UPDLOCK".into(), "ROWLOCK".into(), "READPAST".into()]);
        assert!(h.updlock);
        assert!(h.rowlock);
        assert!(h.readpast);
        assert!(h.validate("Jobs").is_ok());
        assert!(h.requires_write_row_lock());
        assert!(h.uses_row_locking_path());
    }

    #[test]
    fn rejects_conflicting_nolock_and_holdlock() {
        let h = TableLockHints::from_raw(&["NOLOCK".into(), "HOLDLOCK".into()]);
        assert!(h.validate("Jobs").is_err());
    }

    #[test]
    fn rejects_conflicting_readpast_and_tablock() {
        let h = TableLockHints::from_raw(&["READPAST".into(), "TABLOCK".into()]);
        assert!(h.validate("Jobs").is_err());
    }

    #[test]
    fn rejects_conflicting_nolock_and_readpast() {
        let h = TableLockHints::from_raw(&["NOLOCK".into(), "READPAST".into()]);
        assert!(h.validate("Jobs").is_err());
    }
}
