use crate::ast::RoutineParamType;
use crate::ast::TableFactor;
use crate::ast::TableRef;
use crate::catalog::{Catalog, ColumnDef, RoutineKind, TableDef};
use crate::error::DbError;
use crate::parser::parse_expr_subquery_aware;
use crate::storage::{Storage, StoredRow};
use crate::types::Value;

use super::super::QueryExecutor;
use super::query_result_to_bound_table;
use crate::executor::clock::Clock;
use crate::executor::context::{ExecutionContext, ModuleFrame, ModuleKind};
use crate::executor::evaluator::eval_expr;
use crate::executor::model::BoundTable;
use crate::executor::{type_mapping, value_ops};

pub(super) fn bind_builtin_tvf(
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
    tref: &TableRef,
    ctx: &mut ExecutionContext,
) -> Result<Option<BoundTable>, DbError> {
    let name = match &tref.factor {
        TableFactor::Named(o) => &o.name,
        TableFactor::Derived(_) => return Ok(None),
        TableFactor::Values { .. } => return Ok(None),
    };
    let upper = name.to_uppercase();

    if upper.starts_with("__XML_NODES(") {
        return bind_xml_nodes(catalog, storage, clock, tref, ctx, name);
    }

    if upper.starts_with("OPENJSON(") {
        return bind_openjson(catalog, storage, clock, tref, ctx, name);
    }

    if upper.starts_with("OPENXML(") {
        return super::openxml::bind_openxml(catalog, storage, clock, tref, ctx, name);
    }

    if upper.starts_with("FN_MY_PERMISSIONS(") || upper == "FN_MY_PERMISSIONS()" {
        return bind_fn_my_permissions(tref);
    }

    if upper.starts_with("GENERATE_SERIES(") {
        return bind_generate_series(catalog, storage, clock, tref, ctx, name);
    }

    if !upper.starts_with("STRING_SPLIT(") {
        return Ok(None);
    }

    let inner = name
        .strip_prefix("STRING_SPLIT(")
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| {
            DbError::Parse("STRING_SPLIT requires (string, separator[, enable_ordinal])".into())
        })?;

    let parts = crate::parser::utils::split_csv_top_level(inner);
    if parts.len() < 2 || parts.len() > 3 {
        return Err(DbError::Parse(
            "STRING_SPLIT requires 2 or 3 arguments".into(),
        ));
    }

    let string_expr = parse_expr_subquery_aware(&parts[0])?;
    let separator_expr = parse_expr_subquery_aware(&parts[1])?;

    let enable_ordinal = if parts.len() == 3 {
        let ordinal_expr = parse_expr_subquery_aware(&parts[2])?;
        match eval_expr(&ordinal_expr, &[], ctx, catalog, storage, clock)? {
            Value::Int(v) => v != 0,
            Value::BigInt(v) => v != 0,
            Value::TinyInt(v) => v != 0,
            Value::SmallInt(v) => v != 0,
            Value::Bit(v) => v,
            _ => {
                return Err(DbError::Execution(
                    "STRING_SPLIT third argument (enable_ordinal) must be an integer".into(),
                ))
            }
        }
    } else {
        false
    };

    let string_val = eval_expr(&string_expr, &[], ctx, catalog, storage, clock)?;
    let separator_val = eval_expr(&separator_expr, &[], ctx, catalog, storage, clock)?;

    let string_str = match &string_val {
        Value::VarChar(s) | Value::NVarChar(s) | Value::Char(s) | Value::NChar(s) => s.clone(),
        _ => {
            return Err(DbError::Execution(
                "STRING_SPLIT first argument must be a string".into(),
            ))
        }
    };

    let separator_str = match &separator_val {
        Value::VarChar(s) | Value::NVarChar(s) | Value::Char(s) | Value::NChar(s) => s.clone(),
        _ => {
            return Err(DbError::Execution(
                "STRING_SPLIT second argument must be a string".into(),
            ))
        }
    };

    let split_parts: Vec<&str> = string_str.split(&separator_str).collect();

    let mut columns = vec![ColumnDef {
        id: 1,
        name: "value".to_string(),
        data_type: crate::types::DataType::VarChar { max_len: 4000 },
        nullable: false,
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
    }];

    if enable_ordinal {
        columns.push(ColumnDef {
            id: 2,
            name: "ordinal".to_string(),
            data_type: crate::types::DataType::Int,
            nullable: false,
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

    let rows: Vec<StoredRow> = split_parts
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut values = vec![Value::VarChar(s.to_string())];
            if enable_ordinal {
                values.push(Value::Int((i + 1) as i32));
            }
            StoredRow {
                values,
                deleted: false,
            }
        })
        .collect();

    let table_def = TableDef {
        id: 0,
        schema_id: 1,
        schema_name: "dbo".to_string(),
        name: "STRING_SPLIT".to_string(),
        columns,
        check_constraints: vec![],
        foreign_keys: vec![],
    };

    Ok(Some(BoundTable {
        table: std::sync::Arc::new(table_def),
        alias: tref.alias.clone().unwrap_or_else(|| "STRING_SPLIT".into()),
        virtual_rows: Some(rows),
    }))
}

fn bind_xml_nodes(
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
    tref: &TableRef,
    ctx: &mut ExecutionContext,
    name: &str,
) -> Result<Option<BoundTable>, DbError> {
    let inner = name
        .strip_prefix("__XML_NODES(")
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| DbError::Parse("XML .nodes() requires (xpath)".into()))?;
    let parts = crate::parser::utils::split_csv_top_level(inner);
    if parts.len() < 2 {
        return Err(DbError::Parse(
            "XML .nodes() requires receiver and xpath".into(),
        ));
    }

    let recv_expr = parse_expr_subquery_aware(&parts[0])?;
    let xpath_expr = parse_expr_subquery_aware(&parts[1])?;
    let col_name = if let Some(col_raw) = parts.get(2) {
        let col_expr = parse_expr_subquery_aware(col_raw)?;
        eval_expr(&col_expr, &[], ctx, catalog, storage, clock)?.to_string_value()
    } else {
        "value".to_string()
    };

    let xml_val = eval_expr(&recv_expr, &[], ctx, catalog, storage, clock)?;
    let xpath_val = eval_expr(&xpath_expr, &[], ctx, catalog, storage, clock)?;

    let rows: Vec<StoredRow> = if xml_val.is_null() || xpath_val.is_null() {
        Vec::new()
    } else {
        crate::executor::evaluator::eval_xml_nodes(
            &xml_val.to_string_value(),
            &xpath_val.to_string_value(),
        )
        .into_iter()
        .map(|frag| StoredRow {
            values: vec![Value::NVarChar(frag)],
            deleted: false,
        })
        .collect()
    };

    let columns = vec![ColumnDef {
        id: 1,
        name: col_name,
        data_type: crate::types::DataType::NVarChar { max_len: 0 },
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
    }];

    let table_def = TableDef {
        id: 0,
        schema_id: 1,
        schema_name: "dbo".to_string(),
        name: "nodes".to_string(),
        columns,
        check_constraints: vec![],
        foreign_keys: vec![],
    };

    Ok(Some(BoundTable {
        table: std::sync::Arc::new(table_def),
        alias: tref.alias.clone().unwrap_or_else(|| "nodes".into()),
        virtual_rows: Some(rows),
    }))
}

pub(super) fn bind_inline_tvf(
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
    tref: &TableRef,
    ctx: &mut ExecutionContext,
    executor: &QueryExecutor<'_>,
) -> Result<Option<BoundTable>, DbError> {
    let name = match &tref.factor {
        TableFactor::Named(o) => &o.name,
        TableFactor::Derived(_) => return Ok(None),
        TableFactor::Values { .. } => return Ok(None),
    };
    let Some(open) = name.find('(') else {
        return Ok(None);
    };
    if !name.ends_with(')') {
        return Ok(None);
    }
    let fname = name[..open].trim();
    let args_raw = &name[open + 1..name.len() - 1];
    let schema = tref
        .factor
        .as_object_name()
        .map(|o| o.schema_or_dbo())
        .unwrap_or("dbo");
    let Some(routine) = catalog.find_routine(schema, fname).cloned() else {
        return Ok(None);
    };
    let crate::catalog::RoutineDef {
        object_id,
        schema: routine_schema,
        name: routine_name,
        params,
        kind,
        ..
    } = routine;
    let RoutineKind::Function { body, .. } = kind else {
        return Ok(None);
    };
    if !matches!(
        body.as_ref(),
        crate::ast::FunctionBody::InlineTable(_)
            | crate::ast::FunctionBody::InlineTableWithCte(_)
            | crate::ast::FunctionBody::MultiStatementTable { .. }
    ) {
        return Ok(None);
    }
    let arg_exprs = split_csv_top_level_local(args_raw);
    if arg_exprs.len() != params.len() {
        return Err(DbError::Execution(format!(
            "TVF '{}.{}' expected {} args, got {}",
            schema,
            fname,
            params.len(),
            arg_exprs.len()
        )));
    }

    enum TvfArgValue {
        Scalar {
            name: String,
            ty: crate::types::DataType,
            val: Value,
        },
        Tvp {
            name: String,
            physical: String,
            is_readonly: bool,
        },
    }

    // Evaluate argument expressions in the caller's variable scope first.
    let mut evaluated_args = Vec::with_capacity(params.len());
    for (param, arg_raw) in params.iter().zip(arg_exprs.iter()) {
        match &param.param_type {
            RoutineParamType::Scalar(dt) => {
                let expr = parse_expr_subquery_aware(arg_raw)?;
                let val = eval_expr(&expr, &[], ctx, catalog, storage, clock)?;
                let ty = type_mapping::data_type_spec_to_runtime(dt);
                let coerced = value_ops::coerce_value_to_type_with_dateformat(
                    val,
                    &ty,
                    &ctx.options.dateformat,
                )?;
                evaluated_args.push(TvfArgValue::Scalar {
                    name: param.name.clone(),
                    ty,
                    val: coerced,
                });
            }
            RoutineParamType::TableType(_) => {
                let logical = arg_raw.trim().to_string();
                let Some(physical) = ctx.resolve_table_name(&logical) else {
                    return Err(DbError::Execution(format!(
                        "TVF '{}.{}' TVP argument '{}' is not a table variable",
                        schema, fname, logical
                    )));
                };
                evaluated_args.push(TvfArgValue::Tvp {
                    name: param.name.clone(),
                    physical,
                    is_readonly: param.is_readonly,
                });
            }
        }
    }

    ctx.push_module(ModuleFrame {
        object_id,
        schema: routine_schema.clone(),
        name: routine_name.clone(),
        kind: ModuleKind::Function,
    });
    let saved_caller_vars = std::mem::take(ctx.session.variables);
    for (k, v) in &saved_caller_vars {
        if k.starts_with("@@") {
            ctx.session.variables.insert(k.clone(), v.clone());
        }
    }
    let scope_depth = ctx.frame.scope_vars.len();
    let result = (|| {
        ctx.enter_scope();
        for arg_val in evaluated_args {
            match arg_val {
                TvfArgValue::Scalar { name, ty, val } => {
                    ctx.session.variables.insert(name.clone(), (ty, val));
                    ctx.register_declared_var(&name);
                }
                TvfArgValue::Tvp {
                    name,
                    physical,
                    is_readonly,
                } => {
                    ctx.register_table_var(&name, &physical);
                    if is_readonly {
                        ctx.mark_table_var_readonly(&name);
                    }
                }
            }
        }

        let res = match *body {
            crate::ast::FunctionBody::InlineTable(query) => {
                executor.execute_select((*query).into(), ctx)?
            }
            crate::ast::FunctionBody::InlineTableWithCte(with_cte) => {
                let mut scratch_catalog = catalog.clone_boxed();
                let mut scratch_storage = storage.clone_boxed();
                let mut script_exec = crate::executor::script::ScriptExecutor {
                    catalog: scratch_catalog.as_mut(),
                    storage: scratch_storage.as_mut(),
                    clock,
                };
                match script_exec.execute(crate::ast::Statement::WithCte(with_cte), ctx)? {
                    crate::error::StmtOutcome::Ok(Some(res)) => res,
                    _ => {
                        return Err(DbError::Execution(
                            "inline table function CTE produced no result".into(),
                        ))
                    }
                }
            }
            crate::ast::FunctionBody::MultiStatementTable {
                table_var,
                columns,
                constraints,
                body: stmts,
            } => {
                let mut scratch_catalog = catalog.clone_boxed();
                let mut scratch_storage = storage.clone_boxed();
                let mut script_exec = crate::executor::script::ScriptExecutor {
                    catalog: scratch_catalog.as_mut(),
                    storage: scratch_storage.as_mut(),
                    clock,
                };
                let decl_stmt = crate::ast::Statement::Procedural(
                    crate::ast::ProceduralStatement::DeclareTableVar(Box::new(
                        crate::ast::DeclareTableVarStmt {
                            name: table_var.clone(),
                            columns,
                            table_constraints: constraints,
                        },
                    )),
                );
                let _ = script_exec.execute(decl_stmt, ctx)?;
                let _ = script_exec.execute_batch(&stmts, ctx)?;

                let physical = ctx.resolve_table_name(&table_var).ok_or_else(|| {
                    DbError::Execution(format!(
                        "multi-statement TVF return table '{}' not found",
                        table_var
                    ))
                })?;
                let tbl = scratch_catalog
                    .find_table("dbo", &physical)
                    .ok_or_else(|| DbError::table_not_found("dbo", &physical))?;
                let stored_rows = scratch_storage.get_rows(tbl.id)?;
                let rows = stored_rows
                    .into_iter()
                    .filter(|r| !r.deleted)
                    .map(|r| r.values)
                    .collect();
                let col_names = tbl.columns.iter().map(|c| c.name.clone()).collect();
                let col_types = tbl.columns.iter().map(|c| c.data_type.clone()).collect();
                let col_nulls = tbl.columns.iter().map(|c| c.nullable).collect();
                crate::executor::result::QueryResult {
                    columns: col_names,
                    column_types: col_types,
                    column_nullabilities: col_nulls,
                    rows,
                    is_procedure: false,
                    return_status: None,
                }
            }
            _ => unreachable!(),
        };
        ctx.leave_scope();
        Ok(res)
    })();
    while ctx.frame.scope_vars.len() > scope_depth {
        ctx.leave_scope();
    }
    *ctx.session.variables = saved_caller_vars;
    ctx.pop_module();
    let result = result?;

    Ok(Some(query_result_to_bound_table(
        tref.alias.clone().unwrap_or_else(|| fname.to_string()),
        fname.to_string(),
        result,
    )))
}

fn split_openjson_call_and_with(name: &str) -> Result<(String, Option<String>), DbError> {
    let start = name
        .find('(')
        .ok_or_else(|| DbError::Parse("OPENJSON requires (json_expr[, path])".into()))?;
    let bytes = name.as_bytes();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut i = start;
    let mut close_idx = None;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if b == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    i += 2;
                    continue;
                }
                in_str = false;
            }
        } else {
            match b {
                b'\'' => in_str = true,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        close_idx = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    let close_idx =
        close_idx.ok_or_else(|| DbError::Parse("OPENJSON requires (json_expr[, path])".into()))?;
    let inner = name[start + 1..close_idx].to_string();
    let tail = name[close_idx + 1..].trim();
    if tail.is_empty() {
        return Ok((inner, None));
    }
    let upper_tail = tail.to_ascii_uppercase();
    if upper_tail.starts_with("WITH") {
        let after_with = tail[4..].trim();
        if let Some(schema_inner) = after_with
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
        {
            return Ok((inner, Some(schema_inner.to_string())));
        }
    }
    Err(DbError::Parse(format!("Invalid OPENJSON clause: {tail}")))
}

fn parse_openjson_with_schema(
    schema_sql: &str,
) -> Result<Vec<(String, crate::types::DataType, String, bool)>, DbError> {
    let mut input = schema_sql;
    let tokens = crate::parser::lexer::lex(&mut input, true)
        .map_err(|_| DbError::Parse("Failed to lex OPENJSON WITH schema".into()))?;
    let mut parser = crate::parser::state::Parser::new(tokens);
    let mut cols = Vec::new();
    while !parser.is_empty() {
        let col_name = match parser.next() {
            Some(crate::parser::ast::Token::Identifier(id)) => id.clone(),
            Some(crate::parser::ast::Token::Keyword(kw)) => kw.as_ref().to_string(),
            _ => {
                return Err(DbError::Parse(
                    "Expected column name in OPENJSON WITH".into(),
                ))
            }
        };
        let ast_dt = crate::parser::parse::expressions::parse_data_type(&mut parser)
            .map_err(|e| DbError::Parse(format!("{e}")))?;
        let dt_spec = crate::parser::lower::common::lower_data_type(ast_dt)?;
        let data_type = crate::executor::type_mapping::data_type_spec_to_runtime(&dt_spec);
        let path = match parser.peek() {
            Some(crate::parser::ast::Token::String(s))
            | Some(crate::parser::ast::Token::NString(s)) => {
                let s = s.clone();
                let _ = parser.next();
                s
            }
            _ => format!("$.{col_name}"),
        };
        let mut as_json = false;
        if parser.at_keyword(crate::parser::token::Keyword::As)
            && matches!(
                parser.peek_at(1),
                Some(crate::parser::ast::Token::Identifier(id)) if id.eq_ignore_ascii_case("JSON")
            )
        {
            let _ = parser.next();
            let _ = parser.next();
            as_json = true;
        }
        cols.push((col_name, data_type, path, as_json));
        if matches!(parser.peek(), Some(crate::parser::ast::Token::Comma)) {
            let _ = parser.next();
        } else {
            break;
        }
    }
    Ok(cols)
}

fn bind_openjson(
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
    tref: &TableRef,
    ctx: &mut ExecutionContext,
    name: &str,
) -> Result<Option<BoundTable>, DbError> {
    let (inner, with_schema_sql) = split_openjson_call_and_with(name)?;

    let parts = crate::parser::utils::split_csv_top_level(&inner);
    if parts.is_empty() || parts.len() > 2 {
        return Err(DbError::Parse("OPENJSON requires 1 or 2 arguments".into()));
    }

    let json_expr = parse_expr_subquery_aware(&parts[0])?;
    let json_val = eval_expr(&json_expr, &[], ctx, catalog, storage, clock)?;

    let json_str = match &json_val {
        Value::VarChar(s) | Value::NVarChar(s) | Value::Char(s) | Value::NChar(s) => s.clone(),
        _ => {
            return Err(DbError::Execution(
                "OPENJSON first argument must be a string".into(),
            ))
        }
    };

    let parsed: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| DbError::Execution(format!("OPENJSON: invalid JSON: {}", e)))?;

    // If a path argument is provided, navigate to that path
    let target = if parts.len() == 2 {
        let path_expr = parse_expr_subquery_aware(&parts[1])?;
        let path_val = eval_expr(&path_expr, &[], ctx, catalog, storage, clock)?;
        let path_str = match &path_val {
            Value::VarChar(s) | Value::NVarChar(s) | Value::Char(s) | Value::NChar(s) => s.clone(),
            _ => {
                return Err(DbError::Execution(
                    "OPENJSON second argument (path) must be a string".into(),
                ))
            }
        };
        navigate_json_path(&parsed, &path_str)?
    } else {
        parsed
    };

    if let Some(schema_sql) = with_schema_sql {
        let schema_cols = parse_openjson_with_schema(&schema_sql)?;
        let items: Vec<&serde_json::Value> = match &target {
            serde_json::Value::Array(arr) => arr.iter().collect(),
            serde_json::Value::Object(_) => vec![&target],
            _ => Vec::new(),
        };
        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            let mut vals = Vec::with_capacity(schema_cols.len());
            for (_col_name, data_type, path, as_json) in &schema_cols {
                let extracted = navigate_json_path(item, path).ok();
                let raw_val = match extracted {
                    None | Some(serde_json::Value::Null) => Value::Null,
                    Some(v) if *as_json => Value::NVarChar(v.to_string()),
                    Some(serde_json::Value::String(s)) => Value::NVarChar(s),
                    Some(serde_json::Value::Bool(b)) => Value::Bit(b),
                    Some(serde_json::Value::Number(n)) => {
                        if let Some(i) = n.as_i64() {
                            Value::BigInt(i)
                        } else if let Some(f) = n.as_f64() {
                            Value::Float(f.to_bits())
                        } else {
                            Value::NVarChar(n.to_string())
                        }
                    }
                    Some(other) => Value::NVarChar(other.to_string()),
                };
                let coerced = if matches!(raw_val, Value::Null) {
                    Value::Null
                } else {
                    crate::executor::value_ops::coercion::coerce_value_to_type(raw_val, data_type)?
                };
                vals.push(coerced);
            }
            rows.push(StoredRow {
                values: vals,
                deleted: false,
            });
        }
        let columns: Vec<ColumnDef> = schema_cols
            .into_iter()
            .enumerate()
            .map(|(idx, (col_name, data_type, _, _))| ColumnDef {
                id: (idx + 1) as u32,
                name: col_name,
                data_type,
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
            })
            .collect();
        let table_def = TableDef {
            id: 0,
            schema_id: 1,
            schema_name: "dbo".to_string(),
            name: "OPENJSON".to_string(),
            columns,
            check_constraints: vec![],
            foreign_keys: vec![],
        };
        return Ok(Some(BoundTable {
            table: std::sync::Arc::new(table_def),
            alias: tref.alias.clone().unwrap_or_else(|| "OPENJSON".into()),
            virtual_rows: Some(rows),
        }));
    }

    let rows = openjson_to_rows(&target);

    let columns = vec![
        ColumnDef {
            id: 1,
            name: "key".to_string(),
            data_type: crate::types::DataType::VarChar { max_len: 4000 },
            nullable: false,
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
        },
        ColumnDef {
            id: 2,
            name: "value".to_string(),
            data_type: crate::types::DataType::VarChar { max_len: 4000 },
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
        },
        ColumnDef {
            id: 3,
            name: "type".to_string(),
            data_type: crate::types::DataType::Int,
            nullable: false,
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
        },
    ];

    let table_def = TableDef {
        id: 0,
        schema_id: 1,
        schema_name: "dbo".to_string(),
        name: "OPENJSON".to_string(),
        columns,
        check_constraints: vec![],
        foreign_keys: vec![],
    };

    Ok(Some(BoundTable {
        table: std::sync::Arc::new(table_def),
        alias: tref.alias.clone().unwrap_or_else(|| "OPENJSON".into()),
        virtual_rows: Some(rows),
    }))
}

/// Returns the MSSQL OPENJSON type code for a JSON value.
/// 0=null, 1=string, 2=number, 3=bool, 4=array, 5=object
fn openjson_type_code(v: &serde_json::Value) -> i32 {
    match v {
        serde_json::Value::Null => 0,
        serde_json::Value::String(_) => 1,
        serde_json::Value::Number(_) => 2,
        serde_json::Value::Bool(_) => 3,
        serde_json::Value::Array(_) => 4,
        serde_json::Value::Object(_) => 5,
    }
}

/// Converts a JSON value (object or array) into OPENJSON result rows.
fn openjson_to_rows(val: &serde_json::Value) -> Vec<StoredRow> {
    match val {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let type_code = openjson_type_code(v);
                let value_str = match v {
                    serde_json::Value::Null => Value::Null,
                    serde_json::Value::String(s) => Value::NVarChar(s.clone()),
                    serde_json::Value::Number(n) => Value::NVarChar(n.to_string()),
                    serde_json::Value::Bool(b) => {
                        Value::NVarChar(if *b { "true" } else { "false" }.to_string())
                    }
                    serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                        Value::NVarChar(v.to_string())
                    }
                };
                StoredRow {
                    values: vec![Value::NVarChar(k.clone()), value_str, Value::Int(type_code)],
                    deleted: false,
                }
            })
            .collect(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let type_code = openjson_type_code(v);
                let value_str = match v {
                    serde_json::Value::Null => Value::Null,
                    serde_json::Value::String(s) => Value::NVarChar(s.clone()),
                    serde_json::Value::Number(n) => Value::NVarChar(n.to_string()),
                    serde_json::Value::Bool(b) => {
                        Value::NVarChar(if *b { "true" } else { "false" }.to_string())
                    }
                    serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                        Value::NVarChar(v.to_string())
                    }
                };
                StoredRow {
                    values: vec![
                        Value::NVarChar(i.to_string()),
                        value_str,
                        Value::Int(type_code),
                    ],
                    deleted: false,
                }
            })
            .collect(),
        _ => vec![StoredRow {
            values: vec![
                Value::Null,
                match val {
                    serde_json::Value::Null => Value::Null,
                    serde_json::Value::String(s) => Value::NVarChar(s.clone()),
                    serde_json::Value::Number(n) => Value::NVarChar(n.to_string()),
                    serde_json::Value::Bool(b) => {
                        Value::NVarChar(if *b { "true" } else { "false" }.to_string())
                    }
                    _ => Value::Null,
                },
                Value::Int(openjson_type_code(val)),
            ],
            deleted: false,
        }],
    }
}

/// Navigate a JSON value using a SQL Server JSON path like '$.key.subkey' or '$[0]'.
fn navigate_json_path(root: &serde_json::Value, path: &str) -> Result<serde_json::Value, DbError> {
    let path = path.trim();
    if path == "$" {
        return Ok(root.clone());
    }

    let rest = path.strip_prefix('$').unwrap_or(path);
    let mut current = root.clone();

    let mut chars = rest.chars().peekable();
    while chars.peek().is_some() {
        match chars.peek() {
            Some('.') => {
                chars.next(); // consume '.'
                let mut key = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '.' || c == '[' {
                        break;
                    }
                    key.push(c);
                    chars.next();
                }
                if key.is_empty() {
                    return Err(DbError::Execution(
                        "OPENJSON: empty key in JSON path".into(),
                    ));
                }
                current = current
                    .get(&key)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
            }
            Some('[') => {
                chars.next(); // consume '['
                let mut idx_str = String::new();
                while let Some(&c) = chars.peek() {
                    if c == ']' {
                        chars.next();
                        break;
                    }
                    idx_str.push(c);
                    chars.next();
                }
                let idx: usize = idx_str.parse().map_err(|_| {
                    DbError::Execution(format!(
                        "OPENJSON: invalid array index '{}' in path",
                        idx_str
                    ))
                })?;
                current = current.get(idx).cloned().unwrap_or(serde_json::Value::Null);
            }
            _ => {
                return Err(DbError::Execution(format!(
                    "OPENJSON: unexpected character in path '{}'",
                    path
                )));
            }
        }
    }

    Ok(current)
}

pub(super) fn split_csv_top_level_local(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut depth = 0usize;
    let mut in_string = false;
    for ch in input.chars() {
        match ch {
            '\'' => {
                in_string = !in_string;
                buf.push(ch);
            }
            '(' if !in_string => {
                depth += 1;
                buf.push(ch);
            }
            ')' if !in_string => {
                depth = depth.saturating_sub(1);
                buf.push(ch);
            }
            ',' if !in_string && depth == 0 => {
                if !buf.trim().is_empty() {
                    out.push(buf.trim().to_string());
                }
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf.trim().to_string());
    }
    out
}

fn make_permission_column_def(id: u32, name: &str) -> ColumnDef {
    ColumnDef {
        id,
        name: name.to_string(),
        data_type: crate::types::DataType::NVarChar { max_len: 128 },
        nullable: false,
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
    }
}

fn bind_fn_my_permissions(tref: &TableRef) -> Result<Option<BoundTable>, DbError> {
    let columns = vec![
        make_permission_column_def(1, "subentity_name"),
        make_permission_column_def(2, "permission_name"),
    ];

    let permissions: Vec<&str> = vec![
        "ALTER ANY CONNECTION",
        "ALTER ANY CREDENTIAL",
        "ALTER ANY DATABASE",
        "ALTER ANY EVENT NOTIFICATION",
        "ALTER ANY LINKED SERVER",
        "ALTER ANY LOGIN",
        "ALTER ANY SERVER ROLE",
        "ALTER ANY SETTINGS",
        "ALTER RESOURCES",
        "ALTER SERVER STATE",
        "ALTER TRACE",
        "CONNECT SQL",
        "CREATE ANY DATABASE",
        "CREATE DDL EVENT NOTIFICATION",
        "CREATE ENDPOINT",
        "CREATE SERVER ROLE",
        "CREATE TRACE EVENT NOTIFICATION",
        "EXTERNAL ACCESS",
        "VIEW ANY DATABASE",
        "VIEW ANY DEFINITION",
        "VIEW SERVER STATE",
    ];

    let rows: Vec<StoredRow> = permissions
        .iter()
        .map(|perm| StoredRow {
            values: vec![
                Value::NVarChar(String::new()),
                Value::NVarChar(perm.to_string()),
            ],
            deleted: false,
        })
        .collect();

    let table_def = TableDef {
        id: 0,
        schema_id: 1,
        schema_name: "dbo".to_string(),
        name: "fn_my_permissions".to_string(),
        columns,
        check_constraints: vec![],
        foreign_keys: vec![],
    };

    Ok(Some(BoundTable {
        table: std::sync::Arc::new(table_def),
        alias: tref
            .alias
            .clone()
            .unwrap_or_else(|| "fn_my_permissions".into()),
        virtual_rows: Some(rows),
    }))
}

fn bind_generate_series(
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
    tref: &TableRef,
    ctx: &mut ExecutionContext,
    name: &str,
) -> Result<Option<BoundTable>, DbError> {
    if !name.ends_with(')') || name.len() < "GENERATE_SERIES(".len() + 1 {
        return Err(DbError::Parse(
            "GENERATE_SERIES requires (start, stop[, step])".into(),
        ));
    }
    let inner = &name["GENERATE_SERIES(".len()..name.len() - 1];
    let parts = crate::parser::utils::split_csv_top_level(inner);
    if parts.len() < 2 || parts.len() > 3 {
        return Err(DbError::Parse(
            "GENERATE_SERIES requires 2 or 3 arguments".into(),
        ));
    }

    let start_expr = parse_expr_subquery_aware(&parts[0])?;
    let stop_expr = parse_expr_subquery_aware(&parts[1])?;
    let start_val = eval_expr(&start_expr, &[], ctx, catalog, storage, clock)?;
    let stop_val = eval_expr(&stop_expr, &[], ctx, catalog, storage, clock)?;
    let step_val = if parts.len() == 3 {
        let step_expr = parse_expr_subquery_aware(&parts[2])?;
        Some(eval_expr(&step_expr, &[], ctx, catalog, storage, clock)?)
    } else {
        None
    };

    let any_null =
        start_val.is_null() || stop_val.is_null() || step_val.as_ref().is_some_and(|v| v.is_null());

    let is_decimal = matches!(start_val, Value::Decimal(_, _))
        || matches!(stop_val, Value::Decimal(_, _))
        || matches!(step_val, Some(Value::Decimal(_, _)));

    let mut rows: Vec<StoredRow> = Vec::new();
    let col_type: crate::types::DataType;

    if is_decimal {
        fn to_scaled_decimal(v: &Value) -> Result<(i128, u8), DbError> {
            match v {
                Value::TinyInt(n) => Ok((*n as i128, 0)),
                Value::SmallInt(n) => Ok((*n as i128, 0)),
                Value::Int(n) => Ok((*n as i128, 0)),
                Value::BigInt(n) => Ok((*n as i128, 0)),
                Value::Decimal(u, s) => Ok((*u, *s)),
                _ => Err(DbError::Execution(
                    "GENERATE_SERIES arguments must be numeric".into(),
                )),
            }
        }
        if any_null {
            col_type = crate::types::DataType::Decimal {
                precision: 38,
                scale: 0,
            };
        } else {
            let (s_u, s_sc) = to_scaled_decimal(&start_val)?;
            let (e_u, e_sc) = to_scaled_decimal(&stop_val)?;
            let (st_opt, st_sc) = match &step_val {
                Some(v) => {
                    let (u, sc) = to_scaled_decimal(v)?;
                    (Some(u), sc)
                }
                None => (None, 0),
            };
            let max_scale = s_sc.max(e_sc).max(st_sc);
            let scale_up = |u: i128, sc: u8| -> i128 { u * 10_i128.pow((max_scale - sc) as u32) };
            let start = scale_up(s_u, s_sc);
            let stop = scale_up(e_u, e_sc);
            let step = match st_opt {
                Some(u) => scale_up(u, st_sc),
                None => {
                    let one = 10_i128.pow(max_scale as u32);
                    if start <= stop {
                        one
                    } else {
                        -one
                    }
                }
            };
            if step == 0 {
                return Err(DbError::Execution(
                    "The step parameter cannot be zero.".into(),
                ));
            }
            col_type = crate::types::DataType::Decimal {
                precision: 38,
                scale: max_scale,
            };
            let mut cur = start;
            const MAX_SERIES_ROWS: usize = 1_000_000;
            while (step > 0 && cur <= stop) || (step < 0 && cur >= stop) {
                if rows.len() >= MAX_SERIES_ROWS {
                    return Err(DbError::Execution(
                        "GENERATE_SERIES exceeded maximum row limit".into(),
                    ));
                }
                rows.push(StoredRow {
                    values: vec![Value::Decimal(cur, max_scale)],
                    deleted: false,
                });
                match cur.checked_add(step) {
                    Some(next) => cur = next,
                    None => break,
                }
            }
        }
    } else {
        let is_bigint = matches!(start_val, Value::BigInt(_))
            || matches!(stop_val, Value::BigInt(_))
            || matches!(step_val, Some(Value::BigInt(_)));
        col_type = if is_bigint {
            crate::types::DataType::BigInt
        } else {
            crate::types::DataType::Int
        };
        if !any_null {
            let start = start_val.to_integer_i64().ok_or_else(|| {
                DbError::Execution("GENERATE_SERIES start must be an integer".into())
            })?;
            let stop = stop_val.to_integer_i64().ok_or_else(|| {
                DbError::Execution("GENERATE_SERIES stop must be an integer".into())
            })?;
            let step = match &step_val {
                Some(v) => v.to_integer_i64().ok_or_else(|| {
                    DbError::Execution("GENERATE_SERIES step must be an integer".into())
                })?,
                None => {
                    if start <= stop {
                        1
                    } else {
                        -1
                    }
                }
            };
            if step == 0 {
                return Err(DbError::Execution(
                    "The step parameter cannot be zero.".into(),
                ));
            }
            let mut cur = start;
            const MAX_SERIES_ROWS: usize = 1_000_000;
            while (step > 0 && cur <= stop) || (step < 0 && cur >= stop) {
                if rows.len() >= MAX_SERIES_ROWS {
                    return Err(DbError::Execution(
                        "GENERATE_SERIES exceeded maximum row limit".into(),
                    ));
                }
                let val = if is_bigint {
                    Value::BigInt(cur)
                } else {
                    Value::Int(cur as i32)
                };
                rows.push(StoredRow {
                    values: vec![val],
                    deleted: false,
                });
                match cur.checked_add(step) {
                    Some(next) => cur = next,
                    None => break,
                }
            }
        }
    }

    let columns = vec![ColumnDef {
        id: 1,
        name: "value".to_string(),
        data_type: col_type,
        nullable: false,
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
    }];

    let table_def = TableDef {
        id: 0,
        schema_id: 1,
        schema_name: "dbo".to_string(),
        name: "GENERATE_SERIES".to_string(),
        columns,
        check_constraints: vec![],
        foreign_keys: vec![],
    };

    Ok(Some(BoundTable {
        table: std::sync::Arc::new(table_def),
        alias: tref
            .alias
            .clone()
            .unwrap_or_else(|| "GENERATE_SERIES".into()),
        virtual_rows: Some(rows),
    }))
}
