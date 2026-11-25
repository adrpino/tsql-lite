use std::collections::HashSet;

use crate::ast::{
    DmlStatement, Expr, FromClause, FromNode, FunctionBody, MergeSource, ObjectName,
    ProceduralStatement, SelectStmt, Statement, TableFactor, TableRef,
};
use crate::catalog::{Catalog, RoutineKind};
use crate::error::DbError;
use crate::storage::StoredRow;
use crate::types::Value;

fn is_pseudo_or_temp_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.is_empty()
        || trimmed.starts_with('#')
        || trimmed.starts_with('@')
        || trimmed.contains('(')
        || trimmed.eq_ignore_ascii_case("inserted")
        || trimmed.eq_ignore_ascii_case("deleted")
}

fn push_ref(refs: &mut Vec<(Option<String>, String)>, schema: Option<&str>, name: &str) {
    if is_pseudo_or_temp_name(name) {
        return;
    }
    let clean_name = name.trim().trim_matches(['[', ']']);
    if is_pseudo_or_temp_name(clean_name) {
        return;
    }
    let clean_schema = schema.map(|s| s.trim().trim_matches(['[', ']']).to_string());
    refs.push((clean_schema, clean_name.to_string()));
}

fn push_obj_name(refs: &mut Vec<(Option<String>, String)>, obj: &ObjectName) {
    push_ref(refs, obj.schema.as_deref(), &obj.name);
}

fn collect_table_ref(
    tr: &TableRef,
    ctes: &HashSet<String>,
    refs: &mut Vec<(Option<String>, String)>,
) {
    match &tr.factor {
        TableFactor::Named(obj) => {
            if !ctes.contains(&obj.name.to_ascii_lowercase()) {
                push_obj_name(refs, obj);
            }
        }
        TableFactor::Derived(query) => collect_select(query, ctes, refs),
        TableFactor::Values { rows, .. } => {
            for row in rows {
                for expr in row {
                    collect_expr(expr, ctes, refs);
                }
            }
        }
    }
}

fn collect_from_clause(
    from: &FromClause,
    ctes: &HashSet<String>,
    refs: &mut Vec<(Option<String>, String)>,
) {
    for tr in &from.tables {
        collect_table_ref(tr, ctes, refs);
    }
    for j in &from.joins {
        collect_table_ref(&j.table, ctes, refs);
        if let Some(expr) = &j.on {
            collect_expr(expr, ctes, refs);
        }
    }
    for a in &from.applies {
        collect_select(&a.subquery, ctes, refs);
    }
}

fn collect_from_node(
    node: &FromNode,
    ctes: &HashSet<String>,
    refs: &mut Vec<(Option<String>, String)>,
) {
    match node {
        FromNode::Table(tr) => collect_table_ref(tr, ctes, refs),
        FromNode::Aliased { source, .. } => collect_from_node(source, ctes, refs),
        FromNode::Join {
            left, right, on, ..
        } => {
            collect_from_node(left, ctes, refs);
            collect_from_node(right, ctes, refs);
            if let Some(expr) = on {
                collect_expr(expr, ctes, refs);
            }
        }
    }
}

fn collect_expr(expr: &Expr, ctes: &HashSet<String>, refs: &mut Vec<(Option<String>, String)>) {
    match expr {
        Expr::Binary { left, right, .. } => {
            collect_expr(left, ctes, refs);
            collect_expr(right, ctes, refs);
        }
        Expr::Unary { expr, .. }
        | Expr::IsNull(expr)
        | Expr::IsNotNull(expr)
        | Expr::Cast { expr, .. }
        | Expr::TryCast { expr, .. }
        | Expr::Convert { expr, .. }
        | Expr::TryConvert { expr, .. } => collect_expr(expr, ctes, refs),
        Expr::FunctionCall { name, args, .. } => {
            if let Some((sch, fn_name)) = name.split_once('.') {
                push_ref(refs, Some(sch), fn_name);
            }
            for a in args {
                collect_expr(a, ctes, refs);
            }
        }
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => {
            if let Some(op) = operand {
                collect_expr(op, ctes, refs);
            }
            for wc in when_clauses {
                collect_expr(&wc.condition, ctes, refs);
                collect_expr(&wc.result, ctes, refs);
            }
            if let Some(e) = else_result {
                collect_expr(e, ctes, refs);
            }
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            collect_expr(expr, ctes, refs);
            collect_expr(low, ctes, refs);
            collect_expr(high, ctes, refs);
        }
        Expr::InList { expr, list, .. } => {
            collect_expr(expr, ctes, refs);
            for item in list {
                collect_expr(item, ctes, refs);
            }
        }
        Expr::InSubquery { expr, subquery, .. } => {
            collect_expr(expr, ctes, refs);
            collect_select(subquery, ctes, refs);
        }
        Expr::Exists { subquery, .. } | Expr::Subquery(subquery) => {
            collect_select(subquery, ctes, refs);
        }
        Expr::Like {
            expr,
            pattern,
            escape,
            ..
        } => {
            collect_expr(expr, ctes, refs);
            collect_expr(pattern, ctes, refs);
            if let Some(esc) = escape {
                collect_expr(esc, ctes, refs);
            }
        }
        Expr::WindowFunction(wf) => {
            for a in &wf.args {
                collect_expr(a, ctes, refs);
            }
            for p in &wf.partition_by {
                collect_expr(p, ctes, refs);
            }
            for o in &wf.order_by {
                collect_expr(&o.expr, ctes, refs);
            }
        }
        Expr::NextValueFor { sequence_name } => push_obj_name(refs, sequence_name),
        _ => {}
    }
}

fn collect_select(
    stmt: &SelectStmt,
    ctes: &HashSet<String>,
    refs: &mut Vec<(Option<String>, String)>,
) {
    if let Some(from) = &stmt.from_clause {
        collect_from_node(from, ctes, refs);
    }
    for apply in &stmt.applies {
        collect_select(&apply.subquery, ctes, refs);
    }
    for item in &stmt.projection {
        collect_expr(&item.expr, ctes, refs);
    }
    if let Some(sel) = &stmt.selection {
        collect_expr(sel, ctes, refs);
    }
    for g in &stmt.group_by {
        collect_expr(g, ctes, refs);
    }
    if let Some(h) = &stmt.having {
        collect_expr(h, ctes, refs);
    }
    for o in &stmt.order_by {
        collect_expr(&o.expr, ctes, refs);
    }
    if let Some(set_op) = &stmt.set_op {
        collect_select(&set_op.right, ctes, refs);
    }
}

fn collect_stmt(
    stmt: &Statement,
    ctes: &HashSet<String>,
    refs: &mut Vec<(Option<String>, String)>,
) {
    match stmt {
        Statement::WithCte(cte) => {
            let mut local_ctes = ctes.clone();
            for def in &cte.ctes {
                collect_stmt(&def.query, &local_ctes, refs);
                local_ctes.insert(def.name.to_ascii_lowercase());
            }
            collect_stmt(&cte.body, &local_ctes, refs);
        }
        Statement::Dml(dml) => match dml {
            DmlStatement::Select(s) => collect_select(s, ctes, refs),
            DmlStatement::SetOp(s) => {
                collect_stmt(&s.left, ctes, refs);
                collect_stmt(&s.right, ctes, refs);
            }
            DmlStatement::Insert(ins) => {
                if !ctes.contains(&ins.table.name.to_ascii_lowercase()) {
                    push_obj_name(refs, &ins.table);
                }
                match &ins.source {
                    crate::ast::InsertSource::Select(q) => collect_select(q, ctes, refs),
                    crate::ast::InsertSource::Values(rows) => {
                        for row in rows {
                            for e in row {
                                collect_expr(e, ctes, refs);
                            }
                        }
                    }
                    crate::ast::InsertSource::Exec(exec_stmt) => {
                        collect_stmt(exec_stmt, ctes, refs)
                    }
                    crate::ast::InsertSource::DefaultValues => {}
                }
            }
            DmlStatement::Update(upd) => {
                if !ctes.contains(&upd.table.name.to_ascii_lowercase()) {
                    push_obj_name(refs, &upd.table);
                }
                if let Some(from) = &upd.from {
                    collect_from_clause(from, ctes, refs);
                }
                for a in &upd.assignments {
                    collect_expr(&a.expr, ctes, refs);
                }
                if let Some(sel) = &upd.selection {
                    collect_expr(sel, ctes, refs);
                }
            }
            DmlStatement::Delete(del) => {
                if !ctes.contains(&del.table.name.to_ascii_lowercase()) {
                    push_obj_name(refs, &del.table);
                }
                if let Some(from) = &del.from {
                    collect_from_clause(from, ctes, refs);
                }
                if let Some(sel) = &del.selection {
                    collect_expr(sel, ctes, refs);
                }
            }
            DmlStatement::Merge(m) => {
                collect_table_ref(&m.target, ctes, refs);
                match &m.source {
                    MergeSource::Table(tr) => collect_table_ref(tr, ctes, refs),
                    MergeSource::Subquery(q, _) => collect_select(q, ctes, refs),
                }
                collect_expr(&m.on_condition, ctes, refs);
            }
            DmlStatement::SelectAssign(sa) => {
                if let Some(tr) = &sa.from {
                    collect_table_ref(tr, ctes, refs);
                }
                for j in &sa.joins {
                    collect_table_ref(&j.table, ctes, refs);
                    if let Some(on) = &j.on {
                        collect_expr(on, ctes, refs);
                    }
                }
                for t in &sa.targets {
                    collect_expr(&t.expr, ctes, refs);
                }
                if let Some(sel) = &sa.selection {
                    collect_expr(sel, ctes, refs);
                }
            }
            DmlStatement::BulkInsert(_) | DmlStatement::InsertBulk(_) => {}
        },
        Statement::Procedural(p) => match p {
            ProceduralStatement::BeginEnd(stmts) => {
                for s in stmts {
                    collect_stmt(s, ctes, refs);
                }
            }
            ProceduralStatement::If(i) => {
                collect_expr(&i.condition, ctes, refs);
                for s in &i.then_body {
                    collect_stmt(s, ctes, refs);
                }
                if let Some(else_body) = &i.else_body {
                    for s in else_body {
                        collect_stmt(s, ctes, refs);
                    }
                }
            }
            ProceduralStatement::While(w) => {
                collect_expr(&w.condition, ctes, refs);
                for s in &w.body {
                    collect_stmt(s, ctes, refs);
                }
            }
            ProceduralStatement::TryCatch(tc) => {
                for s in &tc.try_body {
                    collect_stmt(s, ctes, refs);
                }
                for s in &tc.catch_body {
                    collect_stmt(s, ctes, refs);
                }
            }
            ProceduralStatement::ExecProcedure(ep) => {
                push_obj_name(refs, &ep.name);
            }
            ProceduralStatement::Declare(d) => {
                if let Some(v) = &d.default {
                    collect_expr(v, ctes, refs);
                }
            }
            ProceduralStatement::Set(s) => collect_expr(&s.expr, ctes, refs),
            ProceduralStatement::Return(Some(e)) => collect_expr(e, ctes, refs),
            ProceduralStatement::DeclareCursor(dc) => collect_select(&dc.query, ctes, refs),
            _ => {}
        },
        _ => {}
    }
}

fn resolve_referenced_id(catalog: &dyn Catalog, schema: &str, name: &str) -> Option<i32> {
    if let Some(t) = catalog.find_table(schema, name) {
        return Some(t.id as i32);
    }
    if let Some(v) = catalog.find_view(schema, name) {
        return Some(v.object_id);
    }
    if let Some(r) = catalog.find_routine(schema, name) {
        return Some(r.object_id);
    }
    if let Some(seq) = catalog.find_sequence(schema, name) {
        return Some(seq.object_id);
    }
    None
}

pub(crate) fn build_expression_dependency_rows(
    catalog: &dyn Catalog,
    db_name: &str,
) -> Vec<StoredRow> {
    let mut rows = Vec::new();
    let empty_ctes = HashSet::new();

    let mut emit_for = |refing_id: i32,
                        refing_schema: &str,
                        refing_name: &str,
                        is_schema_bound: bool,
                        raw_refs: Vec<(Option<String>, String)>| {
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for (opt_schema, ent_name) in raw_refs {
            let eff_schema = opt_schema.as_deref().unwrap_or("dbo");
            if eff_schema.eq_ignore_ascii_case(refing_schema)
                && ent_name.eq_ignore_ascii_case(refing_name)
            {
                continue;
            }
            let dedup_key = (
                eff_schema.to_ascii_lowercase(),
                ent_name.to_ascii_lowercase(),
            );
            if !seen.insert(dedup_key) {
                continue;
            }
            let refd_id = resolve_referenced_id(catalog, eff_schema, &ent_name);
            rows.push(StoredRow {
                values: vec![
                    Value::Int(refing_id),
                    Value::TinyInt(1),
                    Value::VarChar("OBJECT_OR_COLUMN".to_string()),
                    Value::VarChar(refing_name.to_string()),
                    Value::VarChar(refing_schema.to_string()),
                    Value::VarChar(db_name.to_string()),
                    refd_id.map(Value::Int).unwrap_or(Value::Null),
                    Value::TinyInt(1),
                    Value::VarChar("OBJECT_OR_COLUMN".to_string()),
                    Value::VarChar(ent_name.clone()),
                    Value::VarChar(eff_schema.to_string()),
                    Value::Null,
                    Value::Bit(is_schema_bound),
                    Value::VarChar(eff_schema.to_string()),
                    Value::VarChar(ent_name),
                    Value::Null,
                    Value::Bit(is_schema_bound),
                ],
                deleted: false,
            });
        }
    };

    for view in catalog.get_views().iter() {
        let mut refs = Vec::new();
        collect_stmt(&view.query, &empty_ctes, &mut refs);
        emit_for(
            view.object_id,
            &view.schema,
            &view.name,
            view.is_schema_bound,
            refs,
        );
    }

    for routine in catalog.get_routines().iter() {
        let mut refs = Vec::new();
        match &routine.kind {
            RoutineKind::Procedure { body } => {
                for s in body.iter() {
                    collect_stmt(s, &empty_ctes, &mut refs);
                }
            }
            RoutineKind::Function { body, .. } => match body.as_ref() {
                FunctionBody::ScalarReturn(e) => collect_expr(e, &empty_ctes, &mut refs),
                FunctionBody::Scalar(stmts)
                | FunctionBody::MultiStatementTable { body: stmts, .. } => {
                    for s in stmts {
                        collect_stmt(s, &empty_ctes, &mut refs);
                    }
                }
                FunctionBody::InlineTable(q) => collect_select(q, &empty_ctes, &mut refs),
                FunctionBody::InlineTableWithCte(cte) => {
                    let mut local_ctes = empty_ctes.clone();
                    for def in &cte.ctes {
                        collect_stmt(&def.query, &local_ctes, &mut refs);
                        local_ctes.insert(def.name.to_ascii_lowercase());
                    }
                    collect_stmt(&cte.body, &local_ctes, &mut refs);
                }
            },
        }
        emit_for(
            routine.object_id,
            &routine.schema,
            &routine.name,
            routine.is_schema_bound,
            refs,
        );
    }

    for trig in catalog.get_triggers().iter() {
        let mut refs = Vec::new();
        push_ref(&mut refs, Some(&trig.table_schema), &trig.table_name);
        for s in trig.body.iter() {
            collect_stmt(s, &empty_ctes, &mut refs);
        }
        emit_for(trig.object_id, &trig.schema, &trig.name, false, refs);
    }

    rows
}

fn refs_contain(
    refs: &[(Option<String>, String)],
    owner_schema: &str,
    owner_name: &str,
    target_schema: &str,
    target_name: &str,
) -> bool {
    refs.iter().any(|(opt_schema, ent_name)| {
        let eff_schema = opt_schema.as_deref().unwrap_or("dbo");
        !(eff_schema.eq_ignore_ascii_case(owner_schema)
            && ent_name.eq_ignore_ascii_case(owner_name))
            && eff_schema.eq_ignore_ascii_case(target_schema)
            && ent_name.eq_ignore_ascii_case(target_name)
    })
}

fn collect_routine_refs(routine: &crate::catalog::RoutineDef) -> Vec<(Option<String>, String)> {
    let empty_ctes = HashSet::new();
    let mut refs = Vec::new();
    match &routine.kind {
        RoutineKind::Procedure { body } => {
            for s in body.iter() {
                collect_stmt(s, &empty_ctes, &mut refs);
            }
        }
        RoutineKind::Function { body, .. } => match body.as_ref() {
            FunctionBody::ScalarReturn(e) => collect_expr(e, &empty_ctes, &mut refs),
            FunctionBody::Scalar(stmts) | FunctionBody::MultiStatementTable { body: stmts, .. } => {
                for s in stmts {
                    collect_stmt(s, &empty_ctes, &mut refs);
                }
            }
            FunctionBody::InlineTable(q) => collect_select(q, &empty_ctes, &mut refs),
            FunctionBody::InlineTableWithCte(cte) => {
                let mut local_ctes = empty_ctes.clone();
                for def in &cte.ctes {
                    collect_stmt(&def.query, &local_ctes, &mut refs);
                    local_ctes.insert(def.name.to_ascii_lowercase());
                }
                collect_stmt(&cte.body, &local_ctes, &mut refs);
            }
        },
    }
    refs
}

fn expr_references_column(expr: &Expr, col_name: &str) -> bool {
    match expr {
        Expr::Wildcard | Expr::QualifiedWildcard(_) => true,
        Expr::Identifier(id) => id.trim_matches(['[', ']']).eq_ignore_ascii_case(col_name),
        Expr::QualifiedIdentifier(parts) => parts
            .last()
            .is_some_and(|p| p.trim_matches(['[', ']']).eq_ignore_ascii_case(col_name)),
        Expr::Binary { left, right, .. } => {
            expr_references_column(left, col_name) || expr_references_column(right, col_name)
        }
        Expr::Unary { expr, .. }
        | Expr::IsNull(expr)
        | Expr::IsNotNull(expr)
        | Expr::Cast { expr, .. }
        | Expr::TryCast { expr, .. }
        | Expr::Convert { expr, .. }
        | Expr::TryConvert { expr, .. } => expr_references_column(expr, col_name),
        Expr::Between {
            expr, low, high, ..
        } => {
            expr_references_column(expr, col_name)
                || expr_references_column(low, col_name)
                || expr_references_column(high, col_name)
        }
        Expr::InList { expr, list, .. } => {
            expr_references_column(expr, col_name)
                || list.iter().any(|e| expr_references_column(e, col_name))
        }
        Expr::InSubquery { expr, subquery, .. } => {
            expr_references_column(expr, col_name) || select_references_column(subquery, col_name)
        }
        Expr::Like {
            expr,
            pattern,
            escape,
            ..
        } => {
            expr_references_column(expr, col_name)
                || expr_references_column(pattern, col_name)
                || escape
                    .as_deref()
                    .is_some_and(|e| expr_references_column(e, col_name))
        }
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => {
            operand
                .as_deref()
                .is_some_and(|e| expr_references_column(e, col_name))
                || when_clauses.iter().any(|wc| {
                    expr_references_column(&wc.condition, col_name)
                        || expr_references_column(&wc.result, col_name)
                })
                || else_result
                    .as_deref()
                    .is_some_and(|e| expr_references_column(e, col_name))
        }
        Expr::FunctionCall { args, .. } => args.iter().any(|a| expr_references_column(a, col_name)),
        Expr::WindowFunction(wf) => {
            wf.args.iter().any(|a| expr_references_column(a, col_name))
                || wf
                    .partition_by
                    .iter()
                    .any(|e| expr_references_column(e, col_name))
                || wf
                    .order_by
                    .iter()
                    .any(|o| expr_references_column(&o.expr, col_name))
        }
        Expr::Subquery(q) | Expr::Exists { subquery: q, .. } => {
            select_references_column(q, col_name)
        }
        _ => false,
    }
}

fn from_node_references_column(node: &FromNode, col_name: &str) -> bool {
    match node {
        FromNode::Table(tr) => match &tr.factor {
            TableFactor::Named(_) => false,
            TableFactor::Derived(q) => select_references_column(q, col_name),
            TableFactor::Values { rows, .. } => rows
                .iter()
                .any(|r| r.iter().any(|e| expr_references_column(e, col_name))),
        },
        FromNode::Aliased { source, .. } => from_node_references_column(source, col_name),
        FromNode::Join {
            left, right, on, ..
        } => {
            from_node_references_column(left, col_name)
                || from_node_references_column(right, col_name)
                || on
                    .as_ref()
                    .is_some_and(|e| expr_references_column(e, col_name))
        }
    }
}

fn select_references_column(sel: &SelectStmt, col_name: &str) -> bool {
    sel.projection
        .iter()
        .any(|p| expr_references_column(&p.expr, col_name))
        || sel
            .from_clause
            .as_ref()
            .is_some_and(|f| from_node_references_column(f, col_name))
        || sel
            .selection
            .as_ref()
            .is_some_and(|e| expr_references_column(e, col_name))
        || sel
            .group_by
            .iter()
            .any(|e| expr_references_column(e, col_name))
        || sel
            .having
            .as_ref()
            .is_some_and(|e| expr_references_column(e, col_name))
        || sel
            .order_by
            .iter()
            .any(|o| expr_references_column(&o.expr, col_name))
        || sel
            .applies
            .iter()
            .any(|a| select_references_column(&a.subquery, col_name))
        || sel
            .set_op
            .as_ref()
            .is_some_and(|s| select_references_column(&s.right, col_name))
}

fn stmt_references_column(stmt: &Statement, col_name: &str) -> bool {
    match stmt {
        Statement::Dml(DmlStatement::Select(s)) => select_references_column(s, col_name),
        Statement::Dml(DmlStatement::SelectAssign(sa)) => {
            sa.targets
                .iter()
                .any(|t| expr_references_column(&t.expr, col_name))
                || sa
                    .selection
                    .as_ref()
                    .is_some_and(|e| expr_references_column(e, col_name))
        }
        Statement::WithCte(cte) => {
            cte.ctes
                .iter()
                .any(|c| stmt_references_column(&c.query, col_name))
                || stmt_references_column(&cte.body, col_name)
        }
        Statement::Procedural(ProceduralStatement::Return(Some(e))) => {
            expr_references_column(e, col_name)
        }
        Statement::Procedural(ProceduralStatement::Set(s)) => {
            expr_references_column(&s.expr, col_name)
        }
        Statement::Procedural(ProceduralStatement::If(s)) => {
            expr_references_column(&s.condition, col_name)
                || s.then_body
                    .iter()
                    .any(|st| stmt_references_column(st, col_name))
                || s.else_body
                    .as_ref()
                    .is_some_and(|b| b.iter().any(|st| stmt_references_column(st, col_name)))
        }
        Statement::Procedural(ProceduralStatement::BeginEnd(b)) => {
            b.iter().any(|st| stmt_references_column(st, col_name))
        }
        _ => false,
    }
}

/// Returns the first `WITH SCHEMABINDING` view or function that references `(target_schema, target_name)`.
pub(crate) fn find_schemabound_dependent(
    catalog: &dyn Catalog,
    target_schema: &str,
    target_name: &str,
) -> Option<String> {
    let empty_ctes = HashSet::new();
    for view in catalog.get_views().iter() {
        if !view.is_schema_bound {
            continue;
        }
        let mut refs = Vec::new();
        collect_stmt(&view.query, &empty_ctes, &mut refs);
        if refs_contain(&refs, &view.schema, &view.name, target_schema, target_name) {
            return Some(format!("{}.{}", view.schema, view.name));
        }
    }
    for routine in catalog.get_routines().iter() {
        if !routine.is_schema_bound {
            continue;
        }
        let refs = collect_routine_refs(routine);
        if refs_contain(
            &refs,
            &routine.schema,
            &routine.name,
            target_schema,
            target_name,
        ) {
            return Some(format!("{}.{}", routine.schema, routine.name));
        }
    }
    None
}

/// Returns the first `WITH SCHEMABINDING` view or function referencing `(target_schema, target_name)`
/// that also accesses `col_name`.
pub(crate) fn find_schemabound_column_dependent(
    catalog: &dyn Catalog,
    target_schema: &str,
    target_name: &str,
    col_name: &str,
) -> Option<String> {
    let empty_ctes = HashSet::new();
    for view in catalog.get_views().iter() {
        if !view.is_schema_bound {
            continue;
        }
        let mut refs = Vec::new();
        collect_stmt(&view.query, &empty_ctes, &mut refs);
        if refs_contain(&refs, &view.schema, &view.name, target_schema, target_name)
            && stmt_references_column(&view.query, col_name)
        {
            return Some(format!("{}.{}", view.schema, view.name));
        }
    }
    for routine in catalog.get_routines().iter() {
        if !routine.is_schema_bound {
            continue;
        }
        let refs = collect_routine_refs(routine);
        if !refs_contain(
            &refs,
            &routine.schema,
            &routine.name,
            target_schema,
            target_name,
        ) {
            continue;
        }
        let accesses_col = match &routine.kind {
            RoutineKind::Procedure { body } => {
                body.iter().any(|s| stmt_references_column(s, col_name))
            }
            RoutineKind::Function { body, .. } => match body.as_ref() {
                FunctionBody::ScalarReturn(e) => expr_references_column(e, col_name),
                FunctionBody::Scalar(stmts)
                | FunctionBody::MultiStatementTable { body: stmts, .. } => {
                    stmts.iter().any(|s| stmt_references_column(s, col_name))
                }
                FunctionBody::InlineTable(q) => select_references_column(q, col_name),
                FunctionBody::InlineTableWithCte(cte) => {
                    cte.ctes
                        .iter()
                        .any(|c| stmt_references_column(&c.query, col_name))
                        || stmt_references_column(&cte.body, col_name)
                }
            },
        };
        if accesses_col {
            return Some(format!("{}.{}", routine.schema, routine.name));
        }
    }
    None
}

fn is_builtin_tvf_name(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "OPENJSON"
            | "OPENXML"
            | "STRING_SPLIT"
            | "GENERATE_SERIES"
            | "CHANGETABLE"
            | "CONTAINSTABLE"
            | "FREETEXTTABLE"
            | "OPENDATASOURCE"
            | "OPENQUERY"
            | "OPENROWSET"
    )
}

fn schemabinding_star_err() -> DbError {
    DbError::Custom {
        class: 15,
        number: 1054,
        message: "Syntax '*' is not allowed in schema-bound objects.".to_string(),
    }
}

fn schemabinding_name_err(kind: &str, schema: &str, name: &str, bad_ref: &str) -> DbError {
    DbError::Custom {
        class: 16,
        number: 4512,
        message: format!(
            "Cannot schema bind {} '{}.{}' because name '{}' is invalid for schema binding. Names must be in two-part format and an object cannot reference itself.",
            kind, schema, name, bad_ref
        ),
    }
}

fn validate_sb_object_ref(
    obj: &ObjectName,
    ctes: &HashSet<String>,
    kind: &str,
    self_schema: &str,
    self_name: &str,
) -> Result<(), DbError> {
    let clean_name = obj.name.trim().trim_matches(['[', ']']);
    if is_pseudo_or_temp_name(clean_name) || is_builtin_tvf_name(clean_name) {
        return Ok(());
    }
    if obj.schema.is_none() && ctes.contains(&clean_name.to_ascii_lowercase()) {
        return Ok(());
    }
    if obj.database.is_some() {
        let full = format!(
            "{}.{}.{}",
            obj.database.as_deref().unwrap_or(""),
            obj.schema.as_deref().unwrap_or(""),
            clean_name
        );
        return Err(schemabinding_name_err(kind, self_schema, self_name, &full));
    }
    let Some(ref_schema) = obj.schema.as_deref() else {
        return Err(schemabinding_name_err(
            kind,
            self_schema,
            self_name,
            clean_name,
        ));
    };
    let clean_schema = ref_schema.trim().trim_matches(['[', ']']);
    if clean_schema.eq_ignore_ascii_case(self_schema) && clean_name.eq_ignore_ascii_case(self_name)
    {
        return Err(schemabinding_name_err(
            kind,
            self_schema,
            self_name,
            &format!("{}.{}", clean_schema, clean_name),
        ));
    }
    Ok(())
}

fn validate_sb_table_ref(
    tr: &TableRef,
    ctes: &HashSet<String>,
    kind: &str,
    self_schema: &str,
    self_name: &str,
) -> Result<(), DbError> {
    match &tr.factor {
        TableFactor::Named(obj) => validate_sb_object_ref(obj, ctes, kind, self_schema, self_name),
        TableFactor::Derived(q) => validate_sb_select(q, ctes, kind, self_schema, self_name),
        TableFactor::Values { rows, .. } => {
            for row in rows {
                for expr in row {
                    validate_sb_expr(expr, ctes, kind, self_schema, self_name)?;
                }
            }
            Ok(())
        }
    }
}

fn validate_sb_from_node(
    node: &FromNode,
    ctes: &HashSet<String>,
    kind: &str,
    self_schema: &str,
    self_name: &str,
) -> Result<(), DbError> {
    match node {
        FromNode::Table(tr) => validate_sb_table_ref(tr, ctes, kind, self_schema, self_name),
        FromNode::Aliased { source, .. } => {
            validate_sb_from_node(source, ctes, kind, self_schema, self_name)
        }
        FromNode::Join {
            left, right, on, ..
        } => {
            validate_sb_from_node(left, ctes, kind, self_schema, self_name)?;
            validate_sb_from_node(right, ctes, kind, self_schema, self_name)?;
            if let Some(expr) = on {
                validate_sb_expr(expr, ctes, kind, self_schema, self_name)?;
            }
            Ok(())
        }
    }
}

fn validate_sb_expr(
    expr: &Expr,
    ctes: &HashSet<String>,
    kind: &str,
    self_schema: &str,
    self_name: &str,
) -> Result<(), DbError> {
    match expr {
        Expr::Binary { left, right, .. } => {
            validate_sb_expr(left, ctes, kind, self_schema, self_name)?;
            validate_sb_expr(right, ctes, kind, self_schema, self_name)
        }
        Expr::Unary { expr, .. }
        | Expr::IsNull(expr)
        | Expr::IsNotNull(expr)
        | Expr::Cast { expr, .. }
        | Expr::TryCast { expr, .. }
        | Expr::Convert { expr, .. }
        | Expr::TryConvert { expr, .. } => {
            validate_sb_expr(expr, ctes, kind, self_schema, self_name)
        }
        Expr::FunctionCall { name, args, .. } => {
            let parts: Vec<&str> = name.split('.').collect();
            if parts.len() > 2 {
                return Err(schemabinding_name_err(kind, self_schema, self_name, name));
            }
            if parts.len() == 2 {
                let fn_schema = parts[0].trim().trim_matches(['[', ']']);
                let fn_name = parts[1].trim().trim_matches(['[', ']']);
                if fn_schema.eq_ignore_ascii_case(self_schema)
                    && fn_name.eq_ignore_ascii_case(self_name)
                {
                    return Err(schemabinding_name_err(kind, self_schema, self_name, name));
                }
            }
            for a in args {
                validate_sb_expr(a, ctes, kind, self_schema, self_name)?;
            }
            Ok(())
        }
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => {
            if let Some(op) = operand {
                validate_sb_expr(op, ctes, kind, self_schema, self_name)?;
            }
            for wc in when_clauses {
                validate_sb_expr(&wc.condition, ctes, kind, self_schema, self_name)?;
                validate_sb_expr(&wc.result, ctes, kind, self_schema, self_name)?;
            }
            if let Some(e) = else_result {
                validate_sb_expr(e, ctes, kind, self_schema, self_name)?;
            }
            Ok(())
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            validate_sb_expr(expr, ctes, kind, self_schema, self_name)?;
            validate_sb_expr(low, ctes, kind, self_schema, self_name)?;
            validate_sb_expr(high, ctes, kind, self_schema, self_name)
        }
        Expr::InList { expr, list, .. } => {
            validate_sb_expr(expr, ctes, kind, self_schema, self_name)?;
            for item in list {
                validate_sb_expr(item, ctes, kind, self_schema, self_name)?;
            }
            Ok(())
        }
        Expr::InSubquery { expr, subquery, .. } => {
            validate_sb_expr(expr, ctes, kind, self_schema, self_name)?;
            validate_sb_select(subquery, ctes, kind, self_schema, self_name)
        }
        Expr::Exists { subquery, .. } | Expr::Subquery(subquery) => {
            validate_sb_select(subquery, ctes, kind, self_schema, self_name)
        }
        Expr::Like {
            expr,
            pattern,
            escape,
            ..
        } => {
            validate_sb_expr(expr, ctes, kind, self_schema, self_name)?;
            validate_sb_expr(pattern, ctes, kind, self_schema, self_name)?;
            if let Some(esc) = escape {
                validate_sb_expr(esc, ctes, kind, self_schema, self_name)?;
            }
            Ok(())
        }
        Expr::WindowFunction(wf) => {
            for a in &wf.args {
                validate_sb_expr(a, ctes, kind, self_schema, self_name)?;
            }
            for p in &wf.partition_by {
                validate_sb_expr(p, ctes, kind, self_schema, self_name)?;
            }
            for o in &wf.order_by {
                validate_sb_expr(&o.expr, ctes, kind, self_schema, self_name)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn validate_sb_select(
    stmt: &SelectStmt,
    ctes: &HashSet<String>,
    kind: &str,
    self_schema: &str,
    self_name: &str,
) -> Result<(), DbError> {
    for item in &stmt.projection {
        if matches!(item.expr, Expr::Wildcard | Expr::QualifiedWildcard(_)) {
            return Err(schemabinding_star_err());
        }
        validate_sb_expr(&item.expr, ctes, kind, self_schema, self_name)?;
    }
    if let Some(from) = &stmt.from_clause {
        validate_sb_from_node(from, ctes, kind, self_schema, self_name)?;
    }
    for apply in &stmt.applies {
        validate_sb_select(&apply.subquery, ctes, kind, self_schema, self_name)?;
    }
    if let Some(sel) = &stmt.selection {
        validate_sb_expr(sel, ctes, kind, self_schema, self_name)?;
    }
    for g in &stmt.group_by {
        validate_sb_expr(g, ctes, kind, self_schema, self_name)?;
    }
    if let Some(h) = &stmt.having {
        validate_sb_expr(h, ctes, kind, self_schema, self_name)?;
    }
    for o in &stmt.order_by {
        validate_sb_expr(&o.expr, ctes, kind, self_schema, self_name)?;
    }
    if let Some(set_op) = &stmt.set_op {
        validate_sb_select(&set_op.right, ctes, kind, self_schema, self_name)?;
    }
    Ok(())
}

fn validate_sb_stmt(
    stmt: &Statement,
    ctes: &HashSet<String>,
    kind: &str,
    self_schema: &str,
    self_name: &str,
) -> Result<(), DbError> {
    match stmt {
        Statement::WithCte(cte) => {
            let mut local_ctes = ctes.clone();
            for def in &cte.ctes {
                validate_sb_stmt(&def.query, &local_ctes, kind, self_schema, self_name)?;
                local_ctes.insert(def.name.to_ascii_lowercase());
            }
            validate_sb_stmt(&cte.body, &local_ctes, kind, self_schema, self_name)
        }
        Statement::Dml(dml) => match dml {
            DmlStatement::Select(s) => validate_sb_select(s, ctes, kind, self_schema, self_name),
            DmlStatement::SetOp(s) => {
                validate_sb_stmt(&s.left, ctes, kind, self_schema, self_name)?;
                validate_sb_stmt(&s.right, ctes, kind, self_schema, self_name)
            }
            DmlStatement::Insert(ins) => {
                validate_sb_object_ref(&ins.table, ctes, kind, self_schema, self_name)?;
                match &ins.source {
                    crate::ast::InsertSource::Select(q) => {
                        validate_sb_select(q, ctes, kind, self_schema, self_name)
                    }
                    crate::ast::InsertSource::Values(rows) => {
                        for row in rows {
                            for e in row {
                                validate_sb_expr(e, ctes, kind, self_schema, self_name)?;
                            }
                        }
                        Ok(())
                    }
                    _ => Ok(()),
                }
            }
            DmlStatement::SelectAssign(sa) => {
                if let Some(tr) = &sa.from {
                    validate_sb_table_ref(tr, ctes, kind, self_schema, self_name)?;
                }
                for j in &sa.joins {
                    validate_sb_table_ref(&j.table, ctes, kind, self_schema, self_name)?;
                    if let Some(on) = &j.on {
                        validate_sb_expr(on, ctes, kind, self_schema, self_name)?;
                    }
                }
                for t in &sa.targets {
                    validate_sb_expr(&t.expr, ctes, kind, self_schema, self_name)?;
                }
                if let Some(sel) = &sa.selection {
                    validate_sb_expr(sel, ctes, kind, self_schema, self_name)?;
                }
                Ok(())
            }
            _ => Ok(()),
        },
        Statement::Procedural(p) => match p {
            ProceduralStatement::BeginEnd(stmts) => {
                for s in stmts {
                    validate_sb_stmt(s, ctes, kind, self_schema, self_name)?;
                }
                Ok(())
            }
            ProceduralStatement::If(i) => {
                validate_sb_expr(&i.condition, ctes, kind, self_schema, self_name)?;
                for s in &i.then_body {
                    validate_sb_stmt(s, ctes, kind, self_schema, self_name)?;
                }
                if let Some(else_body) = &i.else_body {
                    for s in else_body {
                        validate_sb_stmt(s, ctes, kind, self_schema, self_name)?;
                    }
                }
                Ok(())
            }
            ProceduralStatement::While(w) => {
                validate_sb_expr(&w.condition, ctes, kind, self_schema, self_name)?;
                for s in &w.body {
                    validate_sb_stmt(s, ctes, kind, self_schema, self_name)?;
                }
                Ok(())
            }
            ProceduralStatement::Declare(d) => {
                if let Some(v) = &d.default {
                    validate_sb_expr(v, ctes, kind, self_schema, self_name)?;
                }
                Ok(())
            }
            ProceduralStatement::Set(s) => {
                validate_sb_expr(&s.expr, ctes, kind, self_schema, self_name)
            }
            ProceduralStatement::Return(Some(e)) => {
                validate_sb_expr(e, ctes, kind, self_schema, self_name)
            }
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

/// Validates SQL Server creation-time `WITH SCHEMABINDING` rules on a view:
/// rejects `*` / `t.*` wildcards (error 1054) and 1-part, 3-part, or self-referencing
/// object names (error 4512).
pub(crate) fn validate_schemabinding_view(
    schema: &str,
    name: &str,
    stmt: &Statement,
) -> Result<(), DbError> {
    let empty_ctes = HashSet::new();
    validate_sb_stmt(stmt, &empty_ctes, "view", schema, name)
}

/// Validates SQL Server creation-time `WITH SCHEMABINDING` rules on a function:
/// rejects `*` / `t.*` wildcards (error 1054) and 1-part, 3-part, or self-referencing
/// object names (error 4512).
pub(crate) fn validate_schemabinding_function(
    schema: &str,
    name: &str,
    body: &FunctionBody,
) -> Result<(), DbError> {
    let empty_ctes = HashSet::new();
    match body {
        FunctionBody::ScalarReturn(e) => validate_sb_expr(e, &empty_ctes, "function", schema, name),
        FunctionBody::Scalar(stmts) | FunctionBody::MultiStatementTable { body: stmts, .. } => {
            for s in stmts {
                validate_sb_stmt(s, &empty_ctes, "function", schema, name)?;
            }
            Ok(())
        }
        FunctionBody::InlineTable(q) => {
            validate_sb_select(q, &empty_ctes, "function", schema, name)
        }
        FunctionBody::InlineTableWithCte(cte) => {
            let mut local_ctes = empty_ctes;
            for def in &cte.ctes {
                validate_sb_stmt(&def.query, &local_ctes, "function", schema, name)?;
                local_ctes.insert(def.name.to_ascii_lowercase());
            }
            validate_sb_stmt(&cte.body, &local_ctes, "function", schema, name)
        }
    }
}
