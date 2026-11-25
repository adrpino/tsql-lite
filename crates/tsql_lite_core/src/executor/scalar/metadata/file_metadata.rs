use crate::ast::Expr;
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::storage::Storage;
use crate::types::Value;

use super::super::super::clock::Clock;
use super::super::super::context::ExecutionContext;
use super::super::super::evaluator::eval_expr;
use super::super::super::model::ContextTable;

fn current_db_logical_name(ctx: &ExecutionContext) -> String {
    ctx.metadata
        .database
        .as_deref()
        .unwrap_or(&ctx.metadata.original_database)
        .to_string()
}

fn resolve_file_id(name: &str, db_name: &str) -> Option<i16> {
    let trimmed = name.trim();
    if trimmed.eq_ignore_ascii_case(db_name) || trimmed.eq_ignore_ascii_case("iridium_sql") {
        Some(1)
    } else if trimmed.eq_ignore_ascii_case(&format!("{db_name}_log"))
        || trimmed.eq_ignore_ascii_case("iridium_sql_log")
    {
        Some(2)
    } else {
        None
    }
}

pub(crate) fn eval_filegroup_id(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution("FILEGROUP_ID expects 1 argument".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let name = val.to_string_value();
    let trimmed = name.trim();
    if trimmed.eq_ignore_ascii_case("PRIMARY") {
        return Ok(Value::Int(1));
    }
    if let Some(fg) = catalog
        .get_filegroups()
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case(trimmed))
    {
        return Ok(Value::Int(fg.data_space_id));
    }
    Ok(Value::Null)
}

pub(crate) fn eval_filegroup_name(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution(
            "FILEGROUP_NAME expects 1 argument".into(),
        ));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let Some(id) = val.to_integer_i64() else {
        return Ok(Value::Null);
    };
    if id == 1 {
        return Ok(Value::NVarChar("PRIMARY".to_string()));
    }
    if let Some(fg) = catalog
        .get_filegroups()
        .iter()
        .find(|f| i64::from(f.data_space_id) == id)
    {
        return Ok(Value::NVarChar(fg.name.clone()));
    }
    Ok(Value::Null)
}

pub(crate) fn eval_filegroupproperty(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution(
            "FILEGROUPPROPERTY expects 2 arguments".into(),
        ));
    }
    let name_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let prop_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if name_val.is_null() || prop_val.is_null() {
        return Ok(Value::Null);
    }
    let fg_name = name_val.to_string_value();
    let trimmed = fg_name.trim();
    let prop = prop_val.to_string_value().trim().to_ascii_lowercase();

    let (is_read_only, is_user_defined, is_default) = if trimmed.eq_ignore_ascii_case("PRIMARY") {
        let other_default = catalog.get_filegroups().iter().any(|f| f.is_default);
        (false, false, !other_default)
    } else if let Some(fg) = catalog
        .get_filegroups()
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case(trimmed))
    {
        (fg.is_read_only, true, fg.is_default)
    } else {
        return Ok(Value::Null);
    };

    let res = match prop.as_str() {
        "isreadonly" => i32::from(is_read_only),
        "isuserdefinedfg" => i32::from(is_user_defined),
        "isdefault" => i32::from(is_default),
        "isautogrowallfiles" => 0,
        _ => return Ok(Value::Null),
    };
    Ok(Value::Int(res))
}

pub(crate) fn eval_file_id(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution("FILE_ID expects 1 argument".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let db_name = current_db_logical_name(ctx);
    match resolve_file_id(&val.to_string_value(), &db_name) {
        Some(id) => Ok(Value::SmallInt(id)),
        None => Ok(Value::Null),
    }
}

pub(crate) fn eval_file_name(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution("FILE_NAME expects 1 argument".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let Some(id) = val.to_integer_i64() else {
        return Ok(Value::Null);
    };
    let db_name = current_db_logical_name(ctx);
    match id {
        1 => Ok(Value::NVarChar(db_name)),
        2 => Ok(Value::NVarChar(format!("{db_name}_log"))),
        _ => Ok(Value::Null),
    }
}

pub(crate) fn eval_fileproperty(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution(
            "FILEPROPERTY expects 2 arguments".into(),
        ));
    }
    let name_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let prop_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if name_val.is_null() || prop_val.is_null() {
        return Ok(Value::Null);
    }
    let db_name = current_db_logical_name(ctx);
    let Some(file_id) = resolve_file_id(&name_val.to_string_value(), &db_name) else {
        return Ok(Value::Null);
    };
    let prop = prop_val.to_string_value().trim().to_ascii_lowercase();
    let res = match prop.as_str() {
        "isreadonly" => 0,
        "isprimaryfile" => i32::from(file_id == 1),
        "islogfile" => i32::from(file_id == 2),
        "spaceused" => {
            if file_id == 1 {
                64
            } else {
                32
            }
        }
        "isdefaultfile" => i32::from(file_id == 1),
        _ => return Ok(Value::Null),
    };
    Ok(Value::Int(res))
}
