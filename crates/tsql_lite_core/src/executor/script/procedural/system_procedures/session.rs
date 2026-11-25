use crate::ast::statements::procedural::ExecProcedureStmt;
use crate::error::DbError;
use crate::executor::context::ExecutionContext;
use crate::executor::evaluator::eval_expr;
use crate::executor::result::QueryResult;
use crate::executor::script::ScriptExecutor;
use crate::types::{DataType, Value};

pub(crate) fn execute_sp_who(ctx: &ExecutionContext<'_>) -> Result<QueryResult, DbError> {
    let rows = vec![vec![
        Value::Int(ctx.metadata.id as i32),
        Value::Int(0), // ecid
        Value::NVarChar("running".to_string()),
        Value::NVarChar(
            ctx.metadata
                .user
                .clone()
                .unwrap_or_else(|| "sa".to_string()),
        ),
        Value::NVarChar(
            ctx.metadata
                .host_name
                .clone()
                .unwrap_or_else(|| "localhost".to_string()),
        ),
        Value::Char("0".to_string()), // blk
        Value::NVarChar(ctx.metadata.database.clone().unwrap_or_default()),
        Value::NVarChar("SELECT".to_string()), // cmd
        Value::Int(0),                         // request_id
    ]];

    Ok(QueryResult {
        columns: vec![
            "spid".into(),
            "ecid".into(),
            "status".into(),
            "loginame".into(),
            "hostname".into(),
            "blk".into(),
            "dbname".into(),
            "cmd".into(),
            "request_id".into(),
        ],
        column_types: vec![
            DataType::Int,
            DataType::Int,
            DataType::NVarChar { max_len: 30 },
            DataType::NVarChar { max_len: 128 },
            DataType::NVarChar { max_len: 128 },
            DataType::Char { len: 5 },
            DataType::NVarChar { max_len: 128 },
            DataType::NVarChar { max_len: 16 },
            DataType::Int,
        ],
        column_nullabilities: vec![
            false, false, false, false, false, false, false, false, false,
        ],
        rows,
        ..Default::default()
    })
}

pub(crate) fn execute_sp_databases() -> Result<QueryResult, DbError> {
    let mut rows = Vec::new();
    for db in crate::executor::database_catalog::builtin_databases() {
        rows.push(vec![
            Value::VarChar(db.name.to_string()),
            Value::Int(0), // DATABASE_SIZE
            Value::Null,   // REMARKS
        ]);
    }
    Ok(QueryResult {
        columns: vec![
            "DATABASE_NAME".into(),
            "DATABASE_SIZE".into(),
            "REMARKS".into(),
        ],
        column_types: vec![
            DataType::VarChar { max_len: 128 },
            DataType::Int,
            DataType::VarChar { max_len: 254 },
        ],
        column_nullabilities: vec![false, false, true],
        rows,
        ..Default::default()
    })
}

pub(crate) fn execute_sp_server_info() -> Result<QueryResult, DbError> {
    let rows = vec![
        vec![
            Value::Int(1),
            Value::VarChar("DBMS_NAME".into()),
            Value::VarChar("SQL Server".into()),
        ],
        vec![
            Value::Int(2),
            Value::VarChar("DBMS_VER".into()),
            Value::VarChar("Microsoft SQL Server 2025 - 17.0.1000.0".into()),
        ],
        vec![
            Value::Int(10),
            Value::VarChar("OWNER_TERM".into()),
            Value::VarChar("owner".into()),
        ],
        vec![
            Value::Int(11),
            Value::VarChar("TABLE_TERM".into()),
            Value::VarChar("table".into()),
        ],
        vec![
            Value::Int(12),
            Value::VarChar("MAX_OWNER_NAME_LENGTH".into()),
            Value::VarChar("128".into()),
        ],
        vec![
            Value::Int(13),
            Value::VarChar("TABLE_LENGTH".into()),
            Value::VarChar("128".into()),
        ],
    ];
    Ok(QueryResult {
        columns: vec![
            "ATTRIBUTE_ID".into(),
            "ATTRIBUTE_NAME".into(),
            "ATTRIBUTE_VALUE".into(),
        ],
        column_types: vec![
            DataType::Int,
            DataType::VarChar { max_len: 60 },
            DataType::VarChar { max_len: 255 },
        ],
        column_nullabilities: vec![false, false, false],
        rows,
        ..Default::default()
    })
}

pub(crate) fn execute_sp_monitor(exec: &ScriptExecutor<'_>) -> Result<QueryResult, DbError> {
    let now = Value::DateTime(exec.clock.now_datetime_literal());
    let rows = vec![vec![
        now.clone(),   // last_run
        now.clone(),   // current_run
        Value::Int(0), // seconds
        Value::Int(0), // cpu_busy
        Value::Int(0), // io_busy
        Value::Int(0), // idle
        Value::Int(0), // packets_received
        Value::Int(0), // packets_sent
        Value::Int(0), // packet_errors
        Value::Int(0), // total_read
        Value::Int(0), // total_write
        Value::Int(0), // total_errors
        Value::Int(0), // connections
    ]];
    Ok(QueryResult {
        columns: vec![
            "last_run".into(),
            "current_run".into(),
            "seconds".into(),
            "cpu_busy".into(),
            "io_busy".into(),
            "idle".into(),
            "packets_received".into(),
            "packets_sent".into(),
            "packet_errors".into(),
            "total_read".into(),
            "total_write".into(),
            "total_errors".into(),
            "connections".into(),
        ],
        column_types: vec![
            DataType::DateTime,
            DataType::DateTime,
            DataType::Int,
            DataType::Int,
            DataType::Int,
            DataType::Int,
            DataType::Int,
            DataType::Int,
            DataType::Int,
            DataType::Int,
            DataType::Int,
            DataType::Int,
            DataType::Int,
        ],
        column_nullabilities: vec![
            false, false, false, false, false, false, false, false, false, false, false, false,
            false,
        ],
        rows,
        ..Default::default()
    })
}

pub(crate) fn execute_sp_set_session_context(
    stmt: &ExecProcedureStmt,
    ctx: &mut ExecutionContext<'_>,
    exec: &mut ScriptExecutor<'_>,
) -> Result<QueryResult, DbError> {
    let mut key = String::new();
    let mut value = Value::Null;
    let mut read_only = false;

    for arg in &stmt.args {
        let val = eval_expr(&arg.expr, &[], ctx, exec.catalog, exec.storage, exec.clock)?;
        match arg.name.as_ref().map(|s| s.to_ascii_lowercase()) {
            Some(ref n) if n == "@key" => key = val.to_string_value(),
            Some(ref n) if n == "@value" => value = val,
            Some(ref n) if n == "@read_only" => read_only = val.to_bool().unwrap_or(false),
            _ => {
                // Positional arguments fallback if needed, but MSSQL usually uses named for this
            }
        }
    }

    if key.is_empty() {
        return Err(DbError::Execution(
            "sp_set_session_context: @key is required".into(),
        ));
    }

    if let Some((_, is_ro)) = ctx.session.session_context.get(&key) {
        if *is_ro {
            return Err(DbError::Execution(format!(
                "Cannot set value for read-only session context key '{}'",
                key
            )));
        }
    }

    ctx.session.session_context.insert(key, (value, read_only));

    Ok(QueryResult::default())
}

pub(crate) fn execute_sp_getapplock(
    stmt: &ExecProcedureStmt,
    ctx: &mut ExecutionContext<'_>,
    exec: &mut ScriptExecutor<'_>,
) -> Result<QueryResult, DbError> {
    let mut resource: Option<String> = None;
    let mut lock_mode: Option<String> = None;
    let mut lock_owner: Option<String> = None;
    let mut db_principal: Option<String> = None;

    for (idx, arg) in stmt.args.iter().enumerate() {
        let val = eval_expr(&arg.expr, &[], ctx, exec.catalog, exec.storage, exec.clock)?;
        if let Some(ref name) = arg.name {
            let norm = name.trim_start_matches('@');
            if norm.eq_ignore_ascii_case("resource") {
                resource = Some(val.to_string_value());
            } else if norm.eq_ignore_ascii_case("lockmode") {
                lock_mode = Some(val.to_string_value());
            } else if norm.eq_ignore_ascii_case("lockowner") {
                lock_owner = Some(val.to_string_value());
            } else if norm.eq_ignore_ascii_case("dbprincipal") {
                db_principal = Some(val.to_string_value());
            }
        } else if idx == 0 && resource.is_none() {
            resource = Some(val.to_string_value());
        } else if idx == 1 && lock_mode.is_none() {
            lock_mode = Some(val.to_string_value());
        } else if idx == 2 && lock_owner.is_none() {
            lock_owner = Some(val.to_string_value());
        } else if idx == 4 && db_principal.is_none() {
            db_principal = Some(val.to_string_value());
        }
    }

    let Some(res_name) = resource.filter(|r| !r.is_empty()) else {
        return Err(DbError::Execution(
            "sp_getapplock: @Resource is required".into(),
        ));
    };
    let mode_str = lock_mode.as_deref().unwrap_or("Exclusive");
    let Some(mode) = crate::executor::applock::AppLockMode::parse_request(mode_str) else {
        return Err(DbError::Execution(format!(
            "sp_getapplock: invalid @LockMode '{}'",
            mode_str
        )));
    };
    let owner = lock_owner.as_deref().unwrap_or("Transaction");
    if !owner.eq_ignore_ascii_case("Transaction") && !owner.eq_ignore_ascii_case("Session") {
        return Err(DbError::Execution(format!(
            "sp_getapplock: invalid @LockOwner '{}'",
            owner
        )));
    }
    let principal = db_principal.as_deref().unwrap_or("public");
    let rc = if owner.eq_ignore_ascii_case("Transaction") && ctx.frame.trancount == 0 {
        -999
    } else {
        let sid = ctx.session_id();
        ctx.session
            .extras
            .app_locks
            .acquire(sid, principal, &res_name, mode, owner)
    };

    Ok(QueryResult {
        return_status: Some(rc),
        is_procedure: true,
        ..Default::default()
    })
}

pub(crate) fn execute_sp_releaseapplock(
    stmt: &ExecProcedureStmt,
    ctx: &mut ExecutionContext<'_>,
    exec: &mut ScriptExecutor<'_>,
) -> Result<QueryResult, DbError> {
    let mut resource: Option<String> = None;
    let mut lock_owner: Option<String> = None;
    let mut db_principal: Option<String> = None;
    for (idx, arg) in stmt.args.iter().enumerate() {
        let val = eval_expr(&arg.expr, &[], ctx, exec.catalog, exec.storage, exec.clock)?;
        if let Some(ref name) = arg.name {
            let norm = name.trim_start_matches('@');
            if norm.eq_ignore_ascii_case("resource") {
                resource = Some(val.to_string_value());
            } else if norm.eq_ignore_ascii_case("lockowner") {
                lock_owner = Some(val.to_string_value());
            } else if norm.eq_ignore_ascii_case("dbprincipal") {
                db_principal = Some(val.to_string_value());
            }
        } else if idx == 0 && resource.is_none() {
            resource = Some(val.to_string_value());
        } else if idx == 1 && lock_owner.is_none() {
            lock_owner = Some(val.to_string_value());
        } else if idx == 2 && db_principal.is_none() {
            db_principal = Some(val.to_string_value());
        }
    }
    let Some(res_name) = resource.filter(|r| !r.is_empty()) else {
        return Err(DbError::Execution(
            "sp_releaseapplock: @Resource is required".into(),
        ));
    };
    let owner = lock_owner.as_deref().unwrap_or("Transaction");
    let principal = db_principal.as_deref().unwrap_or("public");
    let sid = ctx.session_id();
    let rc = ctx
        .session
        .extras
        .app_locks
        .release(sid, principal, &res_name, owner);
    Ok(QueryResult {
        return_status: Some(rc),
        is_procedure: true,
        ..Default::default()
    })
}

pub(crate) fn execute_sp_xml_preparedocument(
    stmt: &ExecProcedureStmt,
    ctx: &mut ExecutionContext<'_>,
    exec: &mut ScriptExecutor<'_>,
) -> Result<QueryResult, DbError> {
    if stmt.args.len() < 2 {
        return Err(DbError::Execution(
            "sp_xml_preparedocument requires at least 2 arguments: @hdoc OUTPUT, @xmltext".into(),
        ));
    }
    let mut handle_var: Option<String> = None;
    let mut xml_text: Option<String> = None;
    for (idx, arg) in stmt.args.iter().enumerate() {
        if let Some(ref pname) = arg.name {
            let p = pname.trim_start_matches('@');
            if p.eq_ignore_ascii_case("hdoc") {
                if let crate::ast::Expr::Identifier(var) = &arg.expr {
                    handle_var = Some(var.clone());
                }
            } else if p.eq_ignore_ascii_case("xmltext") {
                let v = eval_expr(&arg.expr, &[], ctx, exec.catalog, exec.storage, exec.clock)?;
                xml_text = Some(v.to_string_value());
            }
        } else if idx == 0 {
            if let crate::ast::Expr::Identifier(var) = &arg.expr {
                handle_var = Some(var.clone());
            }
        } else if idx == 1 {
            let v = eval_expr(&arg.expr, &[], ctx, exec.catalog, exec.storage, exec.clock)?;
            xml_text = Some(v.to_string_value());
        }
    }
    let xml = xml_text.ok_or_else(|| {
        DbError::Execution("sp_xml_preparedocument: @xmltext argument is required".into())
    })?;
    crate::executor::query::binder::openxml::validate_xml_document(&xml)?;

    let handle = ctx.session.extras.next_xml_handle;
    ctx.session.extras.next_xml_handle = handle.saturating_add(1);
    ctx.session.extras.xml_documents.insert(handle, xml);

    if let Some(var_name) = handle_var {
        let existing_key = ctx
            .session
            .variables
            .keys()
            .find(|k| k.eq_ignore_ascii_case(&var_name))
            .cloned()
            .unwrap_or(var_name);
        ctx.session.variables.insert(
            existing_key,
            (crate::types::DataType::Int, Value::Int(handle)),
        );
    }
    Ok(QueryResult {
        return_status: Some(0),
        is_procedure: true,
        ..Default::default()
    })
}

pub(crate) fn execute_sp_xml_removedocument(
    stmt: &ExecProcedureStmt,
    ctx: &mut ExecutionContext<'_>,
    exec: &mut ScriptExecutor<'_>,
) -> Result<QueryResult, DbError> {
    if stmt.args.is_empty() {
        return Err(DbError::Execution(
            "sp_xml_removedocument requires @hdoc argument".into(),
        ));
    }
    let val = eval_expr(
        &stmt.args[0].expr,
        &[],
        ctx,
        exec.catalog,
        exec.storage,
        exec.clock,
    )?;
    let handle = val.to_integer_i64().ok_or_else(|| {
        DbError::Execution("sp_xml_removedocument: @hdoc must be an integer".into())
    })? as i32;
    ctx.session.extras.xml_documents.remove(&handle);
    Ok(QueryResult {
        return_status: Some(0),
        is_procedure: true,
        ..Default::default()
    })
}

fn quote_bracket_ident(ident: &str) -> String {
    format!("[{}]", ident.replace(']', "]]"))
}

fn exec_dynamic_batch(
    exec: &mut ScriptExecutor<'_>,
    ctx: &mut ExecutionContext<'_>,
    sql: &str,
) -> Result<Option<QueryResult>, DbError> {
    let stmts = crate::parse_batch(sql)?;
    let mut last_result = None;
    for s in stmts {
        match exec.execute(s, ctx)? {
            crate::error::StmtOutcome::Ok(Some(res)) => last_result = Some(res),
            crate::error::StmtOutcome::Ok(None) => {}
            crate::error::StmtOutcome::Return(_) => break,
            _ => {}
        }
    }
    Ok(last_result)
}

pub(crate) fn execute_sp_msforeachtable(
    stmt: &ExecProcedureStmt,
    ctx: &mut ExecutionContext<'_>,
    exec: &mut ScriptExecutor<'_>,
) -> Result<QueryResult, DbError> {
    let mut command1: Option<String> = None;
    let mut replacechar: Option<String> = None;
    let mut command2: Option<String> = None;
    let mut command3: Option<String> = None;
    let mut whereand: Option<String> = None;
    let mut precommand: Option<String> = None;
    let mut postcommand: Option<String> = None;

    for (idx, arg) in stmt.args.iter().enumerate() {
        let val = eval_expr(&arg.expr, &[], ctx, exec.catalog, exec.storage, exec.clock)?;
        let str_opt = if val.is_null() {
            None
        } else {
            Some(val.to_string_value())
        };
        if let Some(ref raw_name) = arg.name {
            match raw_name.trim().to_ascii_lowercase().as_str() {
                "@command1" => command1 = str_opt,
                "@replacechar" => replacechar = str_opt,
                "@command2" => command2 = str_opt,
                "@command3" => command3 = str_opt,
                "@whereand" => whereand = str_opt,
                "@precommand" => precommand = str_opt,
                "@postcommand" => postcommand = str_opt,
                _ => {}
            }
        } else {
            match idx {
                0 => command1 = str_opt,
                1 => replacechar = str_opt,
                2 => command2 = str_opt,
                3 => command3 = str_opt,
                4 => whereand = str_opt,
                5 => precommand = str_opt,
                6 => postcommand = str_opt,
                _ => {}
            }
        }
    }

    let cmd1 = command1.ok_or_else(|| {
        DbError::Execution("sp_MSforeachtable: @command1 parameter is required".into())
    })?;
    let placeholder = replacechar
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("?");

    if let Some(ref pre) = precommand {
        if !pre.trim().is_empty() {
            let _ = exec_dynamic_batch(exec, ctx, pre)?;
        }
    }

    let tables: Vec<(String, String)> = if let Some(ref extra_where) = whereand {
        if extra_where.trim().is_empty() {
            collect_user_tables_for_db(exec, ctx)
        } else {
            let filter_sql = format!(
                "SELECT SCHEMA_NAME(o.schema_id) AS SchemaName, o.name AS TableName \
                 FROM sys.objects o \
                 WHERE o.type = 'U' {} \
                 ORDER BY SCHEMA_NAME(o.schema_id), o.name;",
                extra_where
            );
            let res = exec_dynamic_batch(exec, ctx, &filter_sql)?;
            let mut out = Vec::new();
            if let Some(qr) = res {
                for row in qr.rows {
                    if row.len() >= 2 {
                        let schema = row[0].to_string_value();
                        let name = row[1].to_string_value();
                        if !is_internal_temp_table_name(&name) {
                            out.push((schema, name));
                        }
                    }
                }
            }
            out
        }
    } else {
        collect_user_tables_for_db(exec, ctx)
    };

    let mut last_result = QueryResult::default();
    for (schema, name) in tables {
        let qualified = format!(
            "{}.{}",
            quote_bracket_ident(&schema),
            quote_bracket_ident(&name)
        );
        for template in [Some(&cmd1), command2.as_ref(), command3.as_ref()]
            .into_iter()
            .flatten()
        {
            if template.trim().is_empty() {
                continue;
            }
            let rendered = template.replace(placeholder, &qualified);
            if let Some(res) = exec_dynamic_batch(exec, ctx, &rendered)? {
                last_result = res;
            }
        }
    }

    if let Some(ref post) = postcommand {
        if !post.trim().is_empty() {
            if let Some(res) = exec_dynamic_batch(exec, ctx, post)? {
                last_result = res;
            }
        }
    }

    last_result.return_status = Some(0);
    last_result.is_procedure = true;
    Ok(last_result)
}

fn is_internal_temp_table_name(name: &str) -> bool {
    name.starts_with('#')
        || name.starts_with("__temp_")
        || name.starts_with("__var_")
        || name.starts_with("__tvp_")
}

fn collect_user_tables_for_db(
    exec: &ScriptExecutor<'_>,
    ctx: &ExecutionContext<'_>,
) -> Vec<(String, String)> {
    let active_db = ctx.metadata.database.as_deref().unwrap_or("master");
    let mut tables: Vec<(String, String)> = exec
        .catalog
        .get_tables()
        .iter()
        .filter(|t| {
            crate::executor::database_catalog::schema_belongs_to_db(
                &t.schema_name,
                active_db,
                exec.catalog,
            ) && !is_internal_temp_table_name(&t.name)
        })
        .map(|t| {
            let schema =
                crate::executor::database_catalog::display_schema_name(&t.schema_name).to_string();
            (schema, t.name.clone())
        })
        .collect();
    tables.sort();
    tables
}
