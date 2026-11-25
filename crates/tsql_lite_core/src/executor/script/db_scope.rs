use crate::ast::statements::ddl::{
    AlterTableAction, CatalogMetadataAction, CreateIndexStmt, CreateTableStmt, TableConstraintSpec,
};
use crate::ast::statements::dml::{FromClause, MergeAction, MergeSource};
use crate::ast::statements::query::{FromNode, SelectStmt};
use crate::ast::{
    DdlStatement, DmlStatement, Expr, ObjectName, ProceduralStatement, SessionStatement, Statement,
    TableFactor, TableRef,
};
use crate::catalog::Catalog;
use crate::executor::context::ExecutionContext;
use crate::executor::database_catalog::{
    current_database_name, db_schema_key, is_user_database, resolve_effective_schema,
};

fn is_builtin_compat_dbo_object(name: &str) -> bool {
    name.eq_ignore_ascii_case("sysconfigures")
        || name.eq_ignore_ascii_case("sysdatabases")
        || name.eq_ignore_ascii_case("sysobjects")
        || name.eq_ignore_ascii_case("spt_values")
        || name.eq_ignore_ascii_case("syspolicy_configuration")
        || name.eq_ignore_ascii_case("syspolicy_system_health_state")
}

fn qualify_object_name(obj: &mut ObjectName, current_db: &str, catalog: &dyn Catalog) {
    if obj.name.starts_with('#') || obj.name.starts_with('@') {
        return;
    }
    let raw_schema = obj.schema_or_dbo();
    if raw_schema.eq_ignore_ascii_case("sys")
        || raw_schema.eq_ignore_ascii_case("INFORMATION_SCHEMA")
        || (raw_schema.eq_ignore_ascii_case("dbo") && is_builtin_compat_dbo_object(&obj.name))
    {
        return;
    }
    let eff_schema =
        resolve_effective_schema(obj.database.as_deref(), raw_schema, current_db, catalog);
    obj.database = None;
    obj.schema = Some(eff_schema);
}

fn qualify_schema_name(schema: &mut String, current_db: &str, catalog: &dyn Catalog) {
    if schema.eq_ignore_ascii_case("sys") || schema.eq_ignore_ascii_case("INFORMATION_SCHEMA") {
        return;
    }
    if is_user_database(catalog, current_db) {
        *schema = db_schema_key(current_db, schema);
    }
}

pub(crate) fn qualify_statement(
    stmt: &mut Statement,
    ctx: &ExecutionContext<'_>,
    catalog: &dyn Catalog,
) {
    let current_db = current_database_name(ctx);
    qualify_stmt_in_db(stmt, current_db, catalog);
}

pub(crate) fn qualify_select_in_ctx(
    sel: &mut SelectStmt,
    ctx: &ExecutionContext<'_>,
    catalog: &dyn Catalog,
) {
    let current_db = current_database_name(ctx);
    qualify_select(sel, current_db, catalog);
}

fn qualify_stmt_in_db(stmt: &mut Statement, current_db: &str, catalog: &dyn Catalog) {
    match stmt {
        Statement::Ddl(ddl) => qualify_ddl(ddl, current_db, catalog),
        Statement::Dml(dml) => qualify_dml(dml, current_db, catalog),
        Statement::Procedural(p) => qualify_procedural(p, current_db, catalog),
        Statement::WithCte(w) => {
            for cte in &mut w.ctes {
                qualify_stmt_in_db(&mut cte.query, current_db, catalog);
            }
            qualify_stmt_in_db(&mut w.body, current_db, catalog);
        }
        Statement::Session(SessionStatement::SetIdentityInsert(s)) => {
            qualify_object_name(&mut s.table, current_db, catalog);
        }
        Statement::Session(_) | Statement::Transaction(_) | Statement::Cursor(_) => {}
    }
}

fn qualify_ddl(ddl: &mut DdlStatement, current_db: &str, catalog: &dyn Catalog) {
    match ddl {
        DdlStatement::CreateTable(t) => qualify_create_table(t, current_db, catalog),
        DdlStatement::DropTable(t) => qualify_object_name(&mut t.name, current_db, catalog),
        DdlStatement::TruncateTable(t) => qualify_object_name(&mut t.name, current_db, catalog),
        DdlStatement::AlterTable(a) => {
            qualify_object_name(&mut a.table, current_db, catalog);
            match &mut a.action {
                AlterTableAction::AddColumn(c) => {
                    if let Some(fk) = &mut c.foreign_key {
                        qualify_object_name(&mut fk.referenced_table, current_db, catalog);
                    }
                }
                AlterTableAction::AddConstraint { constraint, .. } => {
                    qualify_table_constraint(constraint, current_db, catalog);
                }
                AlterTableAction::ToggleTrigger {
                    trigger_name: Some(tn),
                    ..
                } => qualify_object_name(tn, current_db, catalog),
                AlterTableAction::SetSystemVersioning {
                    history_table: Some(ht),
                    ..
                } => qualify_object_name(ht, current_db, catalog),
                AlterTableAction::SwitchPartition { target_table } => {
                    qualify_object_name(target_table, current_db, catalog);
                }
                _ => {}
            }
        }
        DdlStatement::CreateIndex(idx) => qualify_create_index(idx, current_db, catalog),
        DdlStatement::DropIndex(d) => qualify_object_name(&mut d.table, current_db, catalog),
        DdlStatement::CreateSchema(s) => qualify_schema_name(&mut s.name, current_db, catalog),
        DdlStatement::DropSchema(s) => qualify_schema_name(&mut s.name, current_db, catalog),
        DdlStatement::AlterSchemaTransfer(astmt) => {
            qualify_schema_name(&mut astmt.target_schema, current_db, catalog);
            qualify_object_name(&mut astmt.source, current_db, catalog);
        }
        DdlStatement::DropView(v) => qualify_object_name(&mut v.name, current_db, catalog),
        DdlStatement::DropProcedure(p) => qualify_object_name(&mut p.name, current_db, catalog),
        DdlStatement::DropFunction(f) => qualify_object_name(&mut f.name, current_db, catalog),
        DdlStatement::DropTrigger(tr) => qualify_object_name(&mut tr.name, current_db, catalog),
        DdlStatement::CreateType(tt) => qualify_object_name(&mut tt.name, current_db, catalog),
        DdlStatement::CreateScalarType(st) => {
            qualify_object_name(&mut st.name, current_db, catalog);
        }
        DdlStatement::DropType(dt) => qualify_object_name(&mut dt.name, current_db, catalog),
        DdlStatement::CreateSynonym(sy) => {
            qualify_object_name(&mut sy.name, current_db, catalog);
            qualify_object_name(&mut sy.base_object, current_db, catalog);
        }
        DdlStatement::DropSynonym(s) => qualify_object_name(&mut s.name, current_db, catalog),
        DdlStatement::CreateSequence(sq) => qualify_object_name(&mut sq.name, current_db, catalog),
        DdlStatement::AlterSequence(sq) => qualify_object_name(&mut sq.name, current_db, catalog),
        DdlStatement::DropSequence(sq) => qualify_object_name(&mut sq.name, current_db, catalog),
        DdlStatement::CatalogMetadata(meta) => match meta.as_mut() {
            CatalogMetadataAction::Permission {
                target_name: Some(t),
                ..
            } => qualify_object_name(t, current_db, catalog),
            CatalogMetadataAction::CreateSecurityPolicy {
                name, predicates, ..
            } => {
                qualify_object_name(name, current_db, catalog);
                for p in predicates {
                    qualify_object_name(&mut p.function, current_db, catalog);
                    qualify_object_name(&mut p.target, current_db, catalog);
                }
            }
            CatalogMetadataAction::AlterSecurityPolicy { name, actions } => {
                qualify_object_name(name, current_db, catalog);
                for act in actions {
                    match act {
                        crate::ast::statements::ddl::AlterSecurityPolicyAction::AddPredicate(p)
                        | crate::ast::statements::ddl::AlterSecurityPolicyAction::AlterPredicate(
                            p,
                        ) => {
                            qualify_object_name(&mut p.function, current_db, catalog);
                            qualify_object_name(&mut p.target, current_db, catalog);
                        }
                        crate::ast::statements::ddl::AlterSecurityPolicyAction::DropPredicate {
                            target,
                            ..
                        } => qualify_object_name(target, current_db, catalog),
                        _ => {}
                    }
                }
            }
            CatalogMetadataAction::DropSecurityPolicy { name, .. } => {
                qualify_object_name(name, current_db, catalog);
            }
            _ => {}
        },
    }
}

fn qualify_create_table(t: &mut CreateTableStmt, current_db: &str, catalog: &dyn Catalog) {
    qualify_object_name(&mut t.name, current_db, catalog);
    for col in &mut t.columns {
        if let Some(fk) = &mut col.foreign_key {
            qualify_object_name(&mut fk.referenced_table, current_db, catalog);
        }
    }
    for tc in &mut t.table_constraints {
        qualify_table_constraint(tc, current_db, catalog);
    }
}

fn qualify_table_constraint(tc: &mut TableConstraintSpec, current_db: &str, catalog: &dyn Catalog) {
    match tc {
        TableConstraintSpec::ForeignKey {
            referenced_table, ..
        } => qualify_object_name(referenced_table, current_db, catalog),
        TableConstraintSpec::SystemVersioning {
            history_table: Some(ht),
            ..
        } => qualify_object_name(ht, current_db, catalog),
        _ => {}
    }
}

fn qualify_create_index(idx: &mut CreateIndexStmt, current_db: &str, catalog: &dyn Catalog) {
    qualify_object_name(&mut idx.table, current_db, catalog);
    if let Some(w) = &mut idx.filter_predicate {
        qualify_expr(w, current_db, catalog);
    }
}

fn qualify_from_clause(from: &mut FromClause, current_db: &str, catalog: &dyn Catalog) {
    for tr in &mut from.tables {
        qualify_table_ref(tr, current_db, catalog);
    }
    for j in &mut from.joins {
        qualify_table_ref(&mut j.table, current_db, catalog);
        if let Some(on) = &mut j.on {
            qualify_expr(on, current_db, catalog);
        }
    }
    for a in &mut from.applies {
        qualify_select(&mut a.subquery, current_db, catalog);
    }
}

fn qualify_dml(dml: &mut DmlStatement, current_db: &str, catalog: &dyn Catalog) {
    match dml {
        DmlStatement::Select(s) => qualify_select(s, current_db, catalog),
        DmlStatement::SetOp(so) => {
            qualify_stmt_in_db(&mut so.left, current_db, catalog);
            qualify_stmt_in_db(&mut so.right, current_db, catalog);
        }
        DmlStatement::SelectAssign(sa) => {
            for t in &mut sa.targets {
                qualify_expr(&mut t.expr, current_db, catalog);
            }
            if let Some(tr) = &mut sa.from {
                qualify_table_ref(tr, current_db, catalog);
            }
            for j in &mut sa.joins {
                qualify_table_ref(&mut j.table, current_db, catalog);
                if let Some(on) = &mut j.on {
                    qualify_expr(on, current_db, catalog);
                }
            }
            for a in &mut sa.applies {
                qualify_select(&mut a.subquery, current_db, catalog);
            }
            if let Some(sel) = &mut sa.selection {
                qualify_expr(sel, current_db, catalog);
            }
            for o in &mut sa.order_by {
                qualify_expr(&mut o.expr, current_db, catalog);
            }
        }
        DmlStatement::Insert(ins) => {
            qualify_object_name(&mut ins.table, current_db, catalog);
            if let Some(target) = &mut ins.output_into {
                qualify_object_name(target, current_db, catalog);
            }
            match &mut ins.source {
                crate::ast::InsertSource::Values(rows) => {
                    for row in rows {
                        for e in row {
                            qualify_expr(e, current_db, catalog);
                        }
                    }
                }
                crate::ast::InsertSource::Select(sel) => qualify_select(sel, current_db, catalog),
                crate::ast::InsertSource::Exec(stmt) => {
                    qualify_stmt_in_db(stmt, current_db, catalog);
                }
                crate::ast::InsertSource::DefaultValues => {}
            }
        }
        DmlStatement::Update(u) => {
            qualify_object_name(&mut u.table, current_db, catalog);
            if let Some(target) = &mut u.output_into {
                qualify_object_name(target, current_db, catalog);
            }
            for a in &mut u.assignments {
                qualify_expr(&mut a.expr, current_db, catalog);
            }
            if let Some(from) = &mut u.from {
                qualify_from_clause(from, current_db, catalog);
            }
            if let Some(w) = &mut u.selection {
                qualify_expr(w, current_db, catalog);
            }
        }
        DmlStatement::Delete(d) => {
            qualify_object_name(&mut d.table, current_db, catalog);
            if let Some(target) = &mut d.output_into {
                qualify_object_name(target, current_db, catalog);
            }
            if let Some(from) = &mut d.from {
                qualify_from_clause(from, current_db, catalog);
            }
            if let Some(w) = &mut d.selection {
                qualify_expr(w, current_db, catalog);
            }
        }
        DmlStatement::Merge(m) => {
            qualify_table_ref(&mut m.target, current_db, catalog);
            if let Some(target) = &mut m.output_into {
                qualify_object_name(target, current_db, catalog);
            }
            match &mut m.source {
                MergeSource::Table(tr) => qualify_table_ref(tr, current_db, catalog),
                MergeSource::Subquery(sel, _) => qualify_select(sel, current_db, catalog),
            }
            qualify_expr(&mut m.on_condition, current_db, catalog);
            for wc in &mut m.when_clauses {
                if let Some(c) = &mut wc.condition {
                    qualify_expr(c, current_db, catalog);
                }
                match &mut wc.action {
                    MergeAction::Update { assignments } => {
                        for a in assignments {
                            qualify_expr(&mut a.expr, current_db, catalog);
                        }
                    }
                    MergeAction::Insert { values, .. } => {
                        for e in values {
                            qualify_expr(e, current_db, catalog);
                        }
                    }
                    MergeAction::Delete => {}
                }
            }
        }
        DmlStatement::BulkInsert(b) => qualify_object_name(&mut b.table, current_db, catalog),
        DmlStatement::InsertBulk(b) => qualify_object_name(&mut b.table, current_db, catalog),
    }
}

fn qualify_procedural(p: &mut ProceduralStatement, current_db: &str, catalog: &dyn Catalog) {
    match p {
        ProceduralStatement::Declare(v) => {
            if let Some(e) = &mut v.default {
                qualify_expr(e, current_db, catalog);
            }
            if let Some(ct) = &mut v.custom_type {
                qualify_object_name(ct, current_db, catalog);
            }
        }
        ProceduralStatement::Set(s) => qualify_expr(&mut s.expr, current_db, catalog),
        ProceduralStatement::If(i) => {
            qualify_expr(&mut i.condition, current_db, catalog);
            for s in &mut i.then_body {
                qualify_stmt_in_db(s, current_db, catalog);
            }
            if let Some(eb) = &mut i.else_body {
                for s in eb {
                    qualify_stmt_in_db(s, current_db, catalog);
                }
            }
        }
        ProceduralStatement::While(w) => {
            qualify_expr(&mut w.condition, current_db, catalog);
            for s in &mut w.body {
                qualify_stmt_in_db(s, current_db, catalog);
            }
        }
        ProceduralStatement::BeginEnd(stmts) => {
            for s in stmts {
                qualify_stmt_in_db(s, current_db, catalog);
            }
        }
        ProceduralStatement::TryCatch(tc) => {
            for s in &mut tc.try_body {
                qualify_stmt_in_db(s, current_db, catalog);
            }
            for s in &mut tc.catch_body {
                qualify_stmt_in_db(s, current_db, catalog);
            }
        }
        ProceduralStatement::CreateView(v) => {
            qualify_object_name(&mut v.name, current_db, catalog);
            for cte in &mut v.ctes {
                qualify_stmt_in_db(&mut cte.query, current_db, catalog);
            }
            qualify_select(&mut v.query, current_db, catalog);
        }
        ProceduralStatement::CreateProcedure(proc_stmt) => {
            qualify_object_name(&mut proc_stmt.name, current_db, catalog);
            for s in &mut proc_stmt.body {
                qualify_stmt_in_db(s, current_db, catalog);
            }
        }
        ProceduralStatement::CreateFunction(f) => {
            qualify_object_name(&mut f.name, current_db, catalog);
            match &mut f.body {
                crate::ast::FunctionBody::ScalarReturn(e) => {
                    qualify_expr(e, current_db, catalog);
                }
                crate::ast::FunctionBody::Scalar(stmts)
                | crate::ast::FunctionBody::MultiStatementTable { body: stmts, .. } => {
                    for s in stmts {
                        qualify_stmt_in_db(s, current_db, catalog);
                    }
                }
                crate::ast::FunctionBody::InlineTable(sel) => {
                    qualify_select(sel, current_db, catalog);
                }
                crate::ast::FunctionBody::InlineTableWithCte(w) => {
                    for cte in &mut w.ctes {
                        qualify_stmt_in_db(&mut cte.query, current_db, catalog);
                    }
                    qualify_stmt_in_db(&mut w.body, current_db, catalog);
                }
            }
        }
        ProceduralStatement::CreateTrigger(tr) => {
            qualify_object_name(&mut tr.name, current_db, catalog);
            qualify_object_name(&mut tr.table, current_db, catalog);
            for s in &mut tr.body {
                qualify_stmt_in_db(s, current_db, catalog);
            }
        }
        ProceduralStatement::ExecProcedure(ep) => {
            if !ep.name.name.eq_ignore_ascii_case("xp_msver")
                && !ep.name.name.eq_ignore_ascii_case("xp_qv")
                && !crate::executor::script::procedural::system_procedures::is_system_procedure(
                    &ep.name.name,
                )
            {
                qualify_object_name(&mut ep.name, current_db, catalog);
            }
            for arg in &mut ep.args {
                qualify_expr(&mut arg.expr, current_db, catalog);
            }
        }
        ProceduralStatement::Return(Some(e)) | ProceduralStatement::Print(e) => {
            qualify_expr(e, current_db, catalog);
        }
        ProceduralStatement::DeclareCursor(c) => qualify_select(&mut c.query, current_db, catalog),
        _ => {}
    }
}

fn qualify_select(sel: &mut SelectStmt, current_db: &str, catalog: &dyn Catalog) {
    if let Some(into) = &mut sel.into_table {
        qualify_object_name(into, current_db, catalog);
    }
    for item in &mut sel.projection {
        qualify_expr(&mut item.expr, current_db, catalog);
    }
    if let Some(from) = &mut sel.from_clause {
        qualify_from_node(from, current_db, catalog);
    }
    for a in &mut sel.applies {
        qualify_select(&mut a.subquery, current_db, catalog);
    }
    if let Some(w) = &mut sel.selection {
        qualify_expr(w, current_db, catalog);
    }
    if let Some(h) = &mut sel.having {
        qualify_expr(h, current_db, catalog);
    }
    for g in &mut sel.group_by {
        qualify_expr(g, current_db, catalog);
    }
    for o in &mut sel.order_by {
        qualify_expr(&mut o.expr, current_db, catalog);
    }
    if let Some(so) = &mut sel.set_op {
        qualify_select(&mut so.right, current_db, catalog);
    }
}

fn qualify_from_node(node: &mut FromNode, current_db: &str, catalog: &dyn Catalog) {
    match node {
        FromNode::Table(tr) => qualify_table_ref(tr, current_db, catalog),
        FromNode::Aliased { source, .. } => qualify_from_node(source, current_db, catalog),
        FromNode::Join {
            left, right, on, ..
        } => {
            qualify_from_node(left, current_db, catalog);
            qualify_from_node(right, current_db, catalog);
            if let Some(on_expr) = on {
                qualify_expr(on_expr, current_db, catalog);
            }
        }
    }
}

fn qualify_table_ref(tr: &mut TableRef, current_db: &str, catalog: &dyn Catalog) {
    match &mut tr.factor {
        TableFactor::Named(obj) => qualify_object_name(obj, current_db, catalog),
        TableFactor::Derived(sel) => qualify_select(sel, current_db, catalog),
        TableFactor::Values { rows, .. } => {
            for r in rows {
                for e in r {
                    qualify_expr(e, current_db, catalog);
                }
            }
        }
    }
}

fn qualify_expr(expr: &mut Expr, current_db: &str, catalog: &dyn Catalog) {
    match expr {
        Expr::Binary { left, right, .. } => {
            qualify_expr(left, current_db, catalog);
            qualify_expr(right, current_db, catalog);
        }
        Expr::Unary { expr, .. }
        | Expr::IsNull(expr)
        | Expr::IsNotNull(expr)
        | Expr::Cast { expr, .. }
        | Expr::TryCast { expr, .. }
        | Expr::Convert { expr, .. }
        | Expr::TryConvert { expr, .. } => qualify_expr(expr, current_db, catalog),
        Expr::Between {
            expr, low, high, ..
        } => {
            qualify_expr(expr, current_db, catalog);
            qualify_expr(low, current_db, catalog);
            qualify_expr(high, current_db, catalog);
        }
        Expr::Like {
            expr,
            pattern,
            escape,
            ..
        } => {
            qualify_expr(expr, current_db, catalog);
            qualify_expr(pattern, current_db, catalog);
            if let Some(esc) = escape {
                qualify_expr(esc, current_db, catalog);
            }
        }
        Expr::InList { expr, list, .. } => {
            qualify_expr(expr, current_db, catalog);
            for item in list {
                qualify_expr(item, current_db, catalog);
            }
        }
        Expr::InSubquery { expr, subquery, .. } => {
            qualify_expr(expr, current_db, catalog);
            qualify_select(subquery, current_db, catalog);
        }
        Expr::Subquery(sel) | Expr::Exists { subquery: sel, .. } => {
            qualify_select(sel, current_db, catalog);
        }
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => {
            if let Some(op) = operand {
                qualify_expr(op, current_db, catalog);
            }
            for wc in when_clauses {
                qualify_expr(&mut wc.condition, current_db, catalog);
                qualify_expr(&mut wc.result, current_db, catalog);
            }
            if let Some(el) = else_result {
                qualify_expr(el, current_db, catalog);
            }
        }
        Expr::FunctionCall { name, args, .. } => {
            for a in args {
                qualify_expr(a, current_db, catalog);
            }
            if name.contains('.') && !name.to_ascii_uppercase().starts_with("$PARTITION.") {
                let parts: Vec<&str> = name.split('.').collect();
                let (obj_db, raw_schema, fn_name) = match parts.as_slice() {
                    [s, f] => (None, *s, *f),
                    [d, s, f] => (Some(*d), *s, *f),
                    _ => return,
                };
                if !raw_schema.eq_ignore_ascii_case("sys") {
                    let eff = resolve_effective_schema(obj_db, raw_schema, current_db, catalog);
                    *name = format!("{}.{}", eff, fn_name);
                }
            }
        }
        Expr::WindowFunction(wf) => {
            for a in &mut wf.args {
                qualify_expr(a, current_db, catalog);
            }
            for p in &mut wf.partition_by {
                qualify_expr(p, current_db, catalog);
            }
            for o in &mut wf.order_by {
                qualify_expr(&mut o.expr, current_db, catalog);
            }
        }
        Expr::NextValueFor { sequence_name } => {
            qualify_object_name(sequence_name, current_db, catalog);
        }
        _ => {}
    }
}
