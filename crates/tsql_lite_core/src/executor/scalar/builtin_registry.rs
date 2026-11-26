use crate::ast::Expr;
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::storage::Storage;
use crate::types::Value;

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::executor::clock::Clock;
use crate::executor::context::ExecutionContext;
use crate::executor::model::ContextTable;
use crate::executor::scalar::vector;
use crate::executor::scalar::{
    bitwise, datetime, logic, math, metadata as tsql_metadata, string, system, tsql_subsystems,
};
use crate::executor::{fuzzy, json, metadata as exec_metadata, regexp};

pub(crate) type ScalarHandler = for<'a> fn(
    &[Expr],
    &[ContextTable],
    &mut ExecutionContext<'a>,
    &dyn Catalog,
    &dyn Storage,
    &dyn Clock,
) -> Result<Value, DbError>;

#[derive(Clone, Copy)]
struct BuiltinScalarFunction {
    name: &'static str,
    handler: ScalarHandler,
}

macro_rules! builtin {
    ($name:literal => $handler:expr) => {
        BuiltinScalarFunction {
            name: $name,
            handler: $handler,
        }
    };
}

const SYSTEM_FUNCTIONS: &[BuiltinScalarFunction] = &[
    builtin!("NEWID" => |args, _row, ctx, _catalog, _storage, _clock| {
        if !args.is_empty() {
            return Err(DbError::Execution("NEWID expects no arguments".into()));
        }
        Ok(system::random::eval_newid(ctx))
    }),
    builtin!("NEWSEQUENTIALID" => |args, _row, _ctx, _catalog, _storage, _clock| {
        if !args.is_empty() {
            return Err(DbError::Execution("NEWSEQUENTIALID expects no arguments".into()));
        }
        Ok(system::random::eval_newsequentialid())
    }),
    builtin!("CONTAINS" => tsql_subsystems::eval_contains),
    builtin!("FREETEXT" => tsql_subsystems::eval_freetext),
    builtin!("GEOGRAPHY::POINT" => |args, row, ctx, catalog, storage, clock| {
        tsql_subsystems::eval_spatial_point(true, args, row, ctx, catalog, storage, clock)
    }),
    builtin!("GEOMETRY::POINT" => |args, row, ctx, catalog, storage, clock| {
        tsql_subsystems::eval_spatial_point(false, args, row, ctx, catalog, storage, clock)
    }),
    builtin!("GEOGRAPHY::STGEOMFROMTEXT" => |args, row, ctx, catalog, storage, clock| {
        tsql_subsystems::eval_spatial_from_text(true, args, row, ctx, catalog, storage, clock)
    }),
    builtin!("GEOMETRY::STGEOMFROMTEXT" => |args, row, ctx, catalog, storage, clock| {
        tsql_subsystems::eval_spatial_from_text(false, args, row, ctx, catalog, storage, clock)
    }),
    builtin!("__METHOD_STDISTANCE" => tsql_subsystems::eval_spatial_distance),
    builtin!("__METHOD_STASTEXT" => tsql_subsystems::eval_spatial_as_text),
    builtin!("__METHOD_LAT" => |args, row, ctx, catalog, storage, clock| {
        tsql_subsystems::eval_spatial_prop("LAT", args, row, ctx, catalog, storage, clock)
    }),
    builtin!("__METHOD_LONG" => |args, row, ctx, catalog, storage, clock| {
        tsql_subsystems::eval_spatial_prop("LONG", args, row, ctx, catalog, storage, clock)
    }),
    builtin!("__METHOD_STX" => |args, row, ctx, catalog, storage, clock| {
        tsql_subsystems::eval_spatial_prop("STX", args, row, ctx, catalog, storage, clock)
    }),
    builtin!("__METHOD_STY" => |args, row, ctx, catalog, storage, clock| {
        tsql_subsystems::eval_spatial_prop("STY", args, row, ctx, catalog, storage, clock)
    }),
    builtin!("__METHOD_STSRID" => |args, row, ctx, catalog, storage, clock| {
        tsql_subsystems::eval_spatial_prop("STSRID", args, row, ctx, catalog, storage, clock)
    }),
    builtin!("HIERARCHYID::PARSE" => tsql_subsystems::eval_hierarchyid_parse),
    builtin!("HIERARCHYID::GETROOT" => tsql_subsystems::eval_hierarchyid_get_root),
    builtin!("__METHOD_TOSTRING" => tsql_subsystems::eval_method_to_string),
    builtin!("__METHOD_GETLEVEL" => tsql_subsystems::eval_hierarchyid_get_level),
    builtin!("__METHOD_ISDESCENDANTOF" => tsql_subsystems::eval_hierarchyid_is_descendant_of),
    builtin!("__METHOD_GETANCESTOR" => tsql_subsystems::eval_hierarchyid_get_ancestor),
    builtin!("__METHOD_GETREPARENTEDVALUE" => tsql_subsystems::eval_hierarchyid_get_reparented_value),
    builtin!("ROWCOUNT_BIG" => |args, _row, ctx, _catalog, _storage, _clock| {
        if !args.is_empty() {
            return Err(DbError::Execution("ROWCOUNT_BIG expects no arguments".into()));
        }
        Ok(Value::BigInt(
            ctx.session
                .variables
                .get("@@ROWCOUNT")
                .and_then(|v| v.1.to_integer_i64())
                .unwrap_or(0),
        ))
    }),
    builtin!("MIN_ACTIVE_ROWVERSION" => |args, _row, _ctx, catalog, _storage, _clock| {
        if !args.is_empty() {
            return Err(DbError::Execution("MIN_ACTIVE_ROWVERSION expects no arguments".into()));
        }
        Ok(Value::Binary(catalog.current_rowversion().to_be_bytes().to_vec()))
    }),
    builtin!("CURRENT_TRANSACTION_ID" => |args, _row, ctx, _catalog, _storage, _clock| {
        if !args.is_empty() {
            return Err(DbError::Execution("CURRENT_TRANSACTION_ID expects no arguments".into()));
        }
        Ok(Value::BigInt(
            (ctx.session_id() as i64) * 1000 + (ctx.frame.trancount as i64) + 1,
        ))
    }),
    builtin!("GETANSINULL" => |args, _row, _ctx, _catalog, _storage, _clock| {
        if args.len() > 1 {
            return Err(DbError::Execution("GETANSINULL expects 0 or 1 arguments".into()));
        }
        Ok(Value::Int(1))
    }),
    builtin!("RAND" => |_args, _row, ctx, _catalog, _storage, _clock| {
        Ok(system::random::eval_rand(ctx))
    }),
    builtin!("OBJECT_ID" => tsql_metadata::eval_object_id),
    builtin!("OBJECT_NAME" => tsql_metadata::eval_object_name),
    builtin!("OBJECT_DEFINITION" => tsql_metadata::eval_object_definition),
    builtin!("OBJECT_SCHEMA_NAME" => tsql_metadata::eval_object_schema_name),
    builtin!("SCHEMA_NAME" => tsql_metadata::eval_schema_name),
    builtin!("SCHEMA_ID" => tsql_metadata::eval_schema_id),
    builtin!("TYPE_NAME" => tsql_metadata::eval_type_name),
    builtin!("TYPE_ID" => tsql_metadata::eval_type_id),
    builtin!("TYPEPROPERTY" => tsql_metadata::eval_typeproperty),
    builtin!("COLUMNPROPERTY" => tsql_metadata::eval_columnproperty),
    builtin!("OBJECTPROPERTY" => tsql_metadata::eval_objectproperty),
    builtin!("OBJECTPROPERTYEX" => tsql_metadata::eval_objectpropertyex),
    builtin!("INDEXPROPERTY" => tsql_metadata::eval_indexproperty),
    builtin!("INDEX_COL" => tsql_metadata::eval_index_col),
    builtin!("INDEXKEY_PROPERTY" => tsql_metadata::eval_indexkey_property),
    builtin!("COL_NAME" => tsql_metadata::eval_col_name),
    builtin!("COL_LENGTH" => tsql_metadata::eval_col_length),
    builtin!("DATABASEPROPERTYEX" => tsql_metadata::eval_databasepropertyex),
    builtin!("FILEGROUP_ID" => tsql_metadata::eval_filegroup_id),
    builtin!("FILEGROUP_NAME" => tsql_metadata::eval_filegroup_name),
    builtin!("FILEGROUPPROPERTY" => tsql_metadata::eval_filegroupproperty),
    builtin!("FILE_ID" => tsql_metadata::eval_file_id),
    builtin!("FILE_NAME" => tsql_metadata::eval_file_name),
    builtin!("FILEPROPERTY" => tsql_metadata::eval_fileproperty),
    builtin!("IDENT_CURRENT" => tsql_metadata::eval_ident_current),
    builtin!("IDENT_SEED" => tsql_metadata::eval_ident_seed),
    builtin!("IDENT_INCR" => tsql_metadata::eval_ident_incr),
    builtin!("SCOPE_IDENTITY" => |_args, _row, ctx, _catalog, _storage, _clock| {
        Ok(system::identity::eval_scope_identity(ctx))
    }),
    builtin!("XACT_STATE" => |args, _row, ctx, _catalog, _storage, _clock| {
        if !args.is_empty() {
            return Err(DbError::Execution("XACT_STATE expects no arguments".into()));
        }
        Ok(Value::Int(ctx.frame.xact_state as i32))
    }),
    builtin!("@@IDENTITY" => |_args, _row, ctx, _catalog, _storage, _clock| {
        Ok(system::identity::eval_identity(ctx))
    }),
    builtin!("@@ROWCOUNT" => |_args, _row, ctx, _catalog, _storage, _clock| {
        Ok(Value::Int(ctx.session.variables.get("@@ROWCOUNT").and_then(|v| v.1.to_integer_i64()).unwrap_or(0) as i32))
    }),
    builtin!("@@TRANCOUNT" => |_args, _row, ctx, _catalog, _storage, _clock| {
        Ok(Value::Int(ctx.frame.trancount as i32))
    }),
    builtin!("@@ERROR" => |_args, _row, ctx, _catalog, _storage, _clock| {
        Ok(Value::Int(if ctx.frame.last_error.is_some() { 1 } else { 0 }))
    }),
    builtin!("@@SPID" => |_args, _row, ctx, _catalog, _storage, _clock| {
        Ok(Value::Int(ctx.metadata.id as i32))
    }),
    builtin!("@@MAX_CONNECTIONS" => |_args, _row, _ctx, _catalog, _storage, _clock| {
        Ok(Value::Int(100))
    }),
    builtin!("@@REMSERVER" => |_args, _row, _ctx, _catalog, _storage, _clock| {
        Ok(Value::Null)
    }),
    builtin!("@@FETCH_STATUS" => |_args, _row, ctx, _catalog, _storage, _clock| {
        Ok(Value::Int(*ctx.session.fetch_status))
    }),
    builtin!("@@NESTLEVEL" => |_args, _row, ctx, _catalog, _storage, _clock| {
        Ok(Value::Int(ctx.frame.depth as i32))
    }),
    builtin!("@@OPTIONS" => |_args, _row, _ctx, _catalog, _storage, _clock| {
        Ok(Value::Int(0))
    }),
    builtin!("@@MAX_PRECISION" => |_args, _row, _ctx, _catalog, _storage, _clock| {
        Ok(Value::TinyInt(38))
    }),
    builtin!("@@DATEFIRST" => |_args, _row, ctx, _catalog, _storage, _clock| {
        Ok(Value::TinyInt(ctx.metadata.datefirst as u8))
    }),
    builtin!("@@MICROSOFTVERSION" => |_args, _row, _ctx, _catalog, _storage, _clock| {
        Ok(system::properties::eval_microsoft_version())
    }),
    builtin!("ERROR_MESSAGE" => |_args, _row, ctx, _catalog, _storage, _clock| {
        system::diagnostics::eval_error_message(ctx)
    }),
    builtin!("ERROR_NUMBER" => |_args, _row, ctx, _catalog, _storage, _clock| {
        system::diagnostics::eval_error_number(ctx)
    }),
    builtin!("ERROR_SEVERITY" => |_args, _row, ctx, _catalog, _storage, _clock| {
        system::diagnostics::eval_error_severity(ctx)
    }),
    builtin!("ERROR_STATE" => |_args, _row, ctx, _catalog, _storage, _clock| {
        system::diagnostics::eval_error_state(ctx)
    }),
    builtin!("ERROR_PROCEDURE" => |_args, _row, ctx, _catalog, _storage, _clock| {
        system::diagnostics::eval_error_procedure(ctx)
    }),
    builtin!("ERROR_LINE" => |_args, _row, ctx, _catalog, _storage, _clock| {
        system::diagnostics::eval_error_line(ctx)
    }),
    builtin!("DB_NAME" => tsql_metadata::eval_db_name),
    builtin!("DB_ID" => tsql_metadata::eval_db_id),
    builtin!("SUSER_SNAME" => |args, _row, ctx, _catalog, _storage, _clock| {
        system::identity::eval_suser_sname(args, ctx)
    }),
    builtin!("SUSER_ID" => |args, _row, ctx, _catalog, _storage, _clock| {
        system::identity::eval_suser_id(args, ctx)
    }),
    builtin!("SUSER_NAME" => |args, _row, ctx, _catalog, _storage, _clock| {
        system::identity::eval_suser_sname(args, ctx)
    }),
    builtin!("SUSER_SID" => system::identity::eval_suser_sid),
    builtin!("USER_NAME" => |args, _row, ctx, _catalog, _storage, _clock| {
        system::identity::eval_user_name(args, ctx)
    }),
    builtin!("USER_ID" => |args, _row, ctx, _catalog, _storage, _clock| {
        system::identity::eval_user_id(args, ctx)
    }),
    builtin!("USER_SID" => system::identity::eval_user_sid),
    builtin!("DATABASE_PRINCIPAL_ID" => system::identity::eval_database_principal_id),
    builtin!("DATABASE_PRINCIPAL_NAME" => system::identity::eval_database_principal_name),
    builtin!("APP_NAME" => |args, _row, ctx, _catalog, _storage, _clock| {
        system::identity::eval_app_name(args, ctx)
    }),
    builtin!("HOST_NAME" => |args, _row, ctx, _catalog, _storage, _clock| {
        system::identity::eval_host_name(args, ctx)
    }),
    builtin!("HOST_ID" => |args, _row, ctx, _catalog, _storage, _clock| {
        system::identity::eval_host_id(args, ctx)
    }),
    builtin!("SYSTEM_USER" => |_args, _row, ctx, _catalog, _storage, _clock| {
        system::identity::eval_system_user(_args, ctx)
    }),
    builtin!("SESSION_USER" => |_args, _row, ctx, _catalog, _storage, _clock| {
        system::identity::eval_session_user(_args, ctx)
    }),
    builtin!("CURRENT_USER" => |_args, _row, ctx, _catalog, _storage, _clock| {
        system::identity::eval_current_user(_args, ctx)
    }),
    builtin!("ORIGINAL_LOGIN" => |_args, _row, ctx, _catalog, _storage, _clock| {
        system::identity::eval_original_login(_args, ctx)
    }),
    builtin!("SQL_VARIANT_PROPERTY" => |args, row, ctx, catalog, storage, clock| {
        if args.len() != 2 {
            return Err(crate::error::DbError::Execution(
                "SQL_VARIANT_PROPERTY expects 2 arguments".into(),
            ));
        }
        let val = crate::executor::evaluator::eval_expr(
            &args[0], row, ctx, catalog, storage, clock,
        )?;
        let prop = crate::executor::evaluator::eval_expr(
            &args[1], row, ctx, catalog, storage, clock,
        )?;
        if val.is_null() || prop.is_null() {
            return Ok(crate::types::Value::Null);
        }
        let prop_str = prop.to_string_value().to_ascii_lowercase();
        let base_type = match &val {
            crate::types::Value::Bit(_) => "bit",
            crate::types::Value::TinyInt(_) => "tinyint",
            crate::types::Value::SmallInt(_) => "smallint",
            crate::types::Value::Int(_) => "int",
            crate::types::Value::BigInt(_) => "bigint",
            crate::types::Value::Float(_) => "float",
            crate::types::Value::Decimal(_, _) => "decimal",
            crate::types::Value::Money(_) => "money",
            crate::types::Value::SmallMoney(_) => "smallmoney",
            crate::types::Value::Char(_) => "char",
            crate::types::Value::VarChar(_) => "varchar",
            crate::types::Value::NChar(_) => "nchar",
            crate::types::Value::NVarChar(_) => "nvarchar",
            crate::types::Value::Binary(_) => "binary",
            crate::types::Value::VarBinary(_) => "varbinary",
            crate::types::Value::Date(_) => "date",
            crate::types::Value::Time(_) => "time",
            crate::types::Value::DateTime(_) | crate::types::Value::DateTime2(_) => "datetime2",
            crate::types::Value::SmallDateTime(_) => "smalldatetime",
            crate::types::Value::DateTimeOffset(_) => "datetimeoffset",
            crate::types::Value::UniqueIdentifier(_) => "uniqueidentifier",
            _ => "sql_variant",
        };
        match prop_str.as_str() {
            "basetype" => Ok(crate::types::Value::NVarChar(base_type.to_string())),
            "scale" => match val {
                crate::types::Value::Decimal(_, s) => Ok(crate::types::Value::Int(s as i32)),
                _ => Ok(crate::types::Value::Int(0)),
            },
            "precision" => match &val {
                crate::types::Value::Int(_) => Ok(crate::types::Value::Int(10)),
                crate::types::Value::BigInt(_) => Ok(crate::types::Value::Int(19)),
                crate::types::Value::SmallInt(_) => Ok(crate::types::Value::Int(5)),
                crate::types::Value::TinyInt(_) => Ok(crate::types::Value::Int(3)),
                crate::types::Value::Decimal(_, _) => Ok(crate::types::Value::Int(38)),
                _ => Ok(crate::types::Value::Int(0)),
            },
            "maxlength" => Ok(crate::types::Value::Int(val.to_string_value().len() as i32)),
            "collation" => Ok(crate::types::Value::NVarChar(
                "SQL_Latin1_General_CP1_CI_AS".to_string(),
            )),
            _ => Ok(crate::types::Value::Null),
        }
    }),
    builtin!("XML_SCHEMA_NAMESPACE" => |args, _row, _ctx, _catalog, _storage, _clock| {
        if args.len() < 2 || args.len() > 3 {
            return Err(crate::error::DbError::Execution(
                "XML_SCHEMA_NAMESPACE expects 2 or 3 arguments".into(),
            ));
        }
        Ok(crate::types::Value::Null)
    }),
    builtin!("ORIGINAL_DB_NAME" => |args, _row, ctx, _catalog, _storage, _clock| {
        tsql_metadata::eval_original_db_name(args, ctx)
    }),
    builtin!("PROCID" => |_args, _row, ctx, _catalog, _storage, _clock| {
        tsql_metadata::eval_procid(ctx)
    }),
    builtin!("SERVERPROPERTY" => system::properties::eval_serverproperty),
    builtin!("FULLTEXTSERVICEPROPERTY" => system::properties::eval_fulltextserviceproperty),
    builtin!("CONNECTIONPROPERTY" => system::properties::eval_connectionproperty),
    builtin!("COLLATIONPROPERTY" => system::properties::eval_collationproperty),
    builtin!("SESSIONPROPERTY" => system::properties::eval_sessionproperty),
    builtin!("CONTEXT_INFO" => system::properties::eval_context_info),
    builtin!("SESSION_CONTEXT" => system::properties::eval_session_context),
    builtin!("IS_SRVROLEMEMBER" => system::identity::eval_is_srvrolemember),
    builtin!("IS_MEMBER" => system::identity::eval_is_member),
    builtin!("PERMISSIONS" => system::identity::eval_permissions),
    builtin!("HAS_DBACCESS" => system::identity::eval_has_dbaccess),
    builtin!("HAS_PERMS_BY_NAME" => system::identity::eval_has_perms_by_name),
    builtin!("PARSENAME" => system::formatting::eval_parsename),
    builtin!("QUOTENAME" => system::formatting::eval_quotename),
    builtin!("HASHBYTES" => system::crypto::eval_hashbytes),
    builtin!("CRYPT_GEN_RANDOM" => system::crypto::eval_crypt_gen_random),
    builtin!("ENCRYPTBYPASSPHRASE" => system::crypto::eval_encryptbypassphrase),
    builtin!("DECRYPTBYPASSPHRASE" => system::crypto::eval_decryptbypassphrase),
    builtin!("COMPRESS" => system::crypto::eval_compress),
    builtin!("DECOMPRESS" => system::crypto::eval_decompress),
    builtin!("BINARY_CHECKSUM" => system::crypto::eval_binary_checksum),
    builtin!("__TRIGGER_UPDATE" => |args, _row, ctx, _catalog, _storage, _clock| {
        if args.len() != 1 {
            return Err(DbError::Execution("UPDATE() expects 1 column argument".into()));
        }
        let col_name = match &args[0] {
            Expr::String(s) | Expr::UnicodeString(s) | Expr::Identifier(s) => s.as_str(),
            _ => return Ok(Value::Bit(false)),
        };
        let Some(frame) = ctx.frame.trigger_frames.last() else {
            return Ok(Value::Bit(false));
        };
        let hit = frame
            .table_columns
            .iter()
            .zip(frame.updated_columns.iter())
            .any(|(c, &u)| u && c.eq_ignore_ascii_case(col_name));
        Ok(Value::Bit(hit))
    }),
    builtin!("COLUMNS_UPDATED" => |args, _row, ctx, _catalog, _storage, _clock| {
        if !args.is_empty() {
            return Err(DbError::Execution("COLUMNS_UPDATED expects 0 arguments".into()));
        }
        let Some(frame) = ctx.frame.trigger_frames.last() else {
            return Ok(Value::VarBinary(Vec::new()));
        };
        let byte_len = frame.updated_columns.len().div_ceil(8);
        let mut mask = vec![0u8; byte_len];
        for (idx, &updated) in frame.updated_columns.iter().enumerate() {
            if updated {
                mask[idx / 8] |= 1u8 << (idx % 8);
            }
        }
        Ok(Value::VarBinary(mask))
    }),
    builtin!("TRIGGER_NESTLEVEL" => |args, row, ctx, catalog, storage, clock| {
        if args.len() > 3 {
            return Err(DbError::Execution("TRIGGER_NESTLEVEL expects 0 to 3 arguments".into()));
        }
        if args.is_empty() {
            return Ok(Value::Int(ctx.frame.trigger_frames.len() as i32));
        }
        let obj_val = crate::executor::evaluator::eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        if obj_val.is_null() {
            return Ok(Value::Int(ctx.frame.trigger_frames.len() as i32));
        }
        let obj_id = obj_val.to_integer_i64().unwrap_or(0) as i32;
        if obj_id == 0 {
            return Ok(Value::Int(ctx.frame.trigger_frames.len() as i32));
        }
        let count = ctx
            .frame
            .trigger_frames
            .iter()
            .filter(|f| f.object_id == obj_id)
            .count();
        Ok(Value::Int(count as i32))
    }),
    builtin!("EVENTDATA" => |args, _row, ctx, _catalog, _storage, _clock| {
        if !args.is_empty() {
            return Err(DbError::Execution("EVENTDATA expects no arguments".into()));
        }
        Ok(ctx
            .frame
            .ddl_event_xml
            .as_ref()
            .map(|xml| Value::VarChar(xml.clone()))
            .unwrap_or(Value::Null))
    }),
    builtin!("APPLOCK_MODE" => |args, row, ctx, catalog, storage, clock| {
        if args.len() != 3 {
            return Err(DbError::Execution(
                "APPLOCK_MODE expects 3 arguments (database_principal, resource_name, lock_owner)".into(),
            ));
        }
        let p = crate::executor::evaluator::eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        let r = crate::executor::evaluator::eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
        let o = crate::executor::evaluator::eval_expr(&args[2], row, ctx, catalog, storage, clock)?;
        if p.is_null() || r.is_null() || o.is_null() {
            return Ok(Value::Null);
        }
        let owner = o.to_string_value();
        if !owner.eq_ignore_ascii_case("Transaction") && !owner.eq_ignore_ascii_case("Session") {
            return Ok(Value::Null);
        }
        let sid = ctx.session_id();
        let mode = ctx.session.extras.app_locks.effective_mode(
            sid,
            &p.to_string_value(),
            &r.to_string_value(),
            &owner,
        );
        Ok(Value::NVarChar(mode.to_string()))
    }),
    builtin!("APPLOCK_TEST" => |args, row, ctx, catalog, storage, clock| {
        if args.len() != 4 {
            return Err(DbError::Execution(
                "APPLOCK_TEST expects 4 arguments (database_principal, resource_name, lock_mode, lock_owner)".into(),
            ));
        }
        let p = crate::executor::evaluator::eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        let r = crate::executor::evaluator::eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
        let m = crate::executor::evaluator::eval_expr(&args[2], row, ctx, catalog, storage, clock)?;
        let o = crate::executor::evaluator::eval_expr(&args[3], row, ctx, catalog, storage, clock)?;
        if p.is_null() || r.is_null() || m.is_null() || o.is_null() {
            return Ok(Value::Null);
        }
        let owner = o.to_string_value();
        if !owner.eq_ignore_ascii_case("Transaction") && !owner.eq_ignore_ascii_case("Session") {
            return Ok(Value::Null);
        }
        if owner.eq_ignore_ascii_case("Transaction") && ctx.frame.trancount == 0 {
            return Err(DbError::Execution(
                "The statement or function must be executed in the context of a user transaction."
                    .into(),
            ));
        }
        let Some(mode) = crate::executor::applock::AppLockMode::parse_request(&m.to_string_value()) else {
            return Ok(Value::Null);
        };
        let sid = ctx.session_id();
        let grantable = ctx.session.extras.app_locks.can_grant(
            sid,
            &p.to_string_value(),
            &r.to_string_value(),
            mode,
        );
        Ok(Value::SmallInt(if grantable { 1 } else { 0 }))
    }),
    builtin!("__IS_DISTINCT_FROM" => |args, row, ctx, catalog, storage, clock| {
        if args.len() != 2 {
            return Err(DbError::Execution("IS DISTINCT FROM expects 2 operands".into()));
        }
        let l = crate::executor::evaluator::eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        let r = crate::executor::evaluator::eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
        let distinct = match (l.is_null(), r.is_null()) {
            (true, true) => false,
            (true, false) | (false, true) => true,
            (false, false) => {
                crate::executor::value_ops::compare_values(&l, &r) != std::cmp::Ordering::Equal
            }
        };
        Ok(Value::Bit(distinct))
    }),
    builtin!("__IS_NOT_DISTINCT_FROM" => |args, row, ctx, catalog, storage, clock| {
        if args.len() != 2 {
            return Err(DbError::Execution("IS NOT DISTINCT FROM expects 2 operands".into()));
        }
        let l = crate::executor::evaluator::eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        let r = crate::executor::evaluator::eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
        let not_distinct = match (l.is_null(), r.is_null()) {
            (true, true) => true,
            (true, false) | (false, true) => false,
            (false, false) => {
                crate::executor::value_ops::compare_values(&l, &r) == std::cmp::Ordering::Equal
            }
        };
        Ok(Value::Bit(not_distinct))
    }),
];

const DATETIME_FUNCTIONS: &[BuiltinScalarFunction] = &[
    builtin!("GETDATE" => datetime::eval_getdate),
    builtin!("CURRENT_TIMESTAMP" => datetime::eval_current_timestamp),
    builtin!("CURRENT_DATE" => datetime::eval_current_date),
    builtin!("CURRENT_TIME" => datetime::eval_current_time),
    builtin!("GETUTCDATE" => datetime::eval_getutcdate),
    builtin!("SYSDATETIME" => datetime::eval_sysdatetime),
    builtin!("SYSUTCDATETIME" => datetime::eval_sysutcdatetime),
    builtin!("SYSDATETIMEOFFSET" => datetime::eval_sysdatetimeoffset),
    builtin!("TODATETIMEOFFSET" => crate::executor::evaluator::eval_todatetimeoffset),
    builtin!("SWITCHOFFSET" => crate::executor::evaluator::eval_switchoffset),
    builtin!("DATEPART" => datetime::eval_datepart),
    builtin!("DATENAME" => datetime::eval_datename),
    builtin!("DATEDIFF" => datetime::eval_datediff),
    builtin!("DATEDIFF_BIG" => datetime::eval_datediff_big),
    builtin!("DATEADD" => datetime::eval_dateadd),
    builtin!("DATETRUNC" => datetime::eval_datetrunc),
    builtin!("DATE_BUCKET" => datetime::eval_date_bucket),
    builtin!("EOMONTH" => datetime::eval_eomonth),
    builtin!("DATEFROMPARTS" => datetime::eval_datefromparts),
    builtin!("TIMEFROMPARTS" => datetime::eval_timefromparts),
    builtin!("SMALLDATETIMEFROMPARTS" => datetime::eval_smalldatetimefromparts),
    builtin!("DATETIMEFROMPARTS" => datetime::eval_datetimefromparts),
    builtin!("DATETIME2FROMPARTS" => datetime::eval_datetime2fromparts),
    builtin!("DATETIMEOFFSETFROMPARTS" => datetime::eval_datetimeoffsetfromparts),
    builtin!("CURRENT_TIMEZONE" => datetime::eval_current_timezone),
    builtin!("CURRENT_TIMEZONE_ID" => datetime::eval_current_timezone_id),
    builtin!("ISDATE" => datetime::eval_isdate),
    builtin!("YEAR" => datetime::eval_year),
    builtin!("MONTH" => datetime::eval_month),
    builtin!("DAY" => datetime::eval_day),
];

const STRING_FUNCTIONS: &[BuiltinScalarFunction] = &[
    builtin!("LEN" => string::eval_len),
    builtin!("DATALENGTH" => string::eval_datalength),
    builtin!("SUBSTRING" => string::eval_substring),
    builtin!("UPPER" => string::eval_upper),
    builtin!("LOWER" => string::eval_lower),
    builtin!("LTRIM" => |args, row, ctx, catalog, storage, clock| {
        string::eval_trim(args, row, ctx, catalog, storage, clock, true, false)
    }),
    builtin!("RTRIM" => |args, row, ctx, catalog, storage, clock| {
        string::eval_trim(args, row, ctx, catalog, storage, clock, false, true)
    }),
    builtin!("TRIM" => |args, row, ctx, catalog, storage, clock| {
        string::eval_trim(args, row, ctx, catalog, storage, clock, true, true)
    }),
    builtin!("REPLACE" => string::eval_replace),
    builtin!("LEFT" => string::eval_left),
    builtin!("RIGHT" => string::eval_right),
    builtin!("CHARINDEX" => string::eval_charindex),
    builtin!("UNISTR" => string::eval_unistr),
    builtin!("ASCII" => string::eval_ascii),
    builtin!("CHAR" => string::eval_char),
    builtin!("NCHAR" => string::eval_nchar),
    builtin!("UNICODE" => string::eval_unicode),
    builtin!("CONCAT" => string::eval_concat),
    builtin!("CONCAT_WS" => string::eval_concat_ws),
    builtin!("REPLICATE" => string::eval_replicate),
    builtin!("REVERSE" => string::eval_reverse),
    builtin!("STUFF" => string::eval_stuff),
    builtin!("SPACE" => string::eval_space),
    builtin!("STR" => string::eval_str),
    builtin!("TRANSLATE" => string::eval_translate),
    builtin!("FORMAT" => string::eval_format),
    builtin!("FORMATMESSAGE" => string::eval_formatmessage),
    builtin!("ISNUMERIC" => string::eval_isnumeric),
    builtin!("PATINDEX" => string::eval_patindex),
    builtin!("SOUNDEX" => string::eval_soundex),
    builtin!("DIFFERENCE" => string::eval_difference),
    builtin!("STRING_ESCAPE" => string::eval_string_escape),
];

const MATH_FUNCTIONS: &[BuiltinScalarFunction] = &[
    builtin!("ABS" => math::eval_abs),
    builtin!("CEILING" => math::eval_ceiling),
    builtin!("FLOOR" => math::eval_floor),
    builtin!("ROUND" => math::eval_round),
    builtin!("SQRT" => math::eval_sqrt),
    builtin!("SQUARE" => math::eval_square),
    builtin!("POWER" => math::eval_power),
    builtin!("VECTOR_DISTANCE" => vector::eval_vector_distance),
    builtin!("VECTOR_NORM" => vector::eval_vector_norm),
    builtin!("VECTOR_NORMALIZE" => vector::eval_vector_normalize),
    builtin!("VECTORPROPERTY" => vector::eval_vector_property),
    builtin!("EXP" => math::eval_exp),
    builtin!("LOG" => math::eval_log),
    builtin!("LOG10" => math::eval_log10),
    builtin!("SIN" => math::eval_sin),
    builtin!("COS" => math::eval_cos),
    builtin!("TAN" => math::eval_tan),
    builtin!("ASIN" => math::eval_asin),
    builtin!("ACOS" => math::eval_acos),
    builtin!("ATAN" => math::eval_atan),
    builtin!("ATN2" => math::eval_atn2),
    builtin!("COT" => math::eval_cot),
    builtin!("DEGREES" => math::eval_degrees),
    builtin!("RADIANS" => math::eval_radians),
    builtin!("PI" => math::eval_pi),
    builtin!("SIGN" => math::eval_sign),
    builtin!("RAND" => math::eval_rand),
    builtin!("CHECKSUM" => math::eval_checksum),
    builtin!("LEFT_SHIFT" => bitwise::eval_left_shift),
    builtin!("RIGHT_SHIFT" => bitwise::eval_right_shift),
    builtin!("BIT_COUNT" => bitwise::eval_bit_count),
    builtin!("GET_BIT" => bitwise::eval_get_bit),
    builtin!("SET_BIT" => bitwise::eval_set_bit),
];

const LOGIC_FUNCTIONS: &[BuiltinScalarFunction] = &[
    builtin!("ISNULL" => logic::eval_isnull),
    builtin!("COALESCE" => logic::eval_coalesce),
    builtin!("IIF" => logic::eval_iif),
    builtin!("NULLIF" => logic::eval_nullif),
    builtin!("CHOOSE" => logic::eval_choose),
    builtin!("GREATEST" => logic::eval_greatest),
    builtin!("LEAST" => logic::eval_least),
];

const JSON_FUNCTIONS: &[BuiltinScalarFunction] = &[
    builtin!("JSON_VALUE" => |args, row, ctx, catalog, storage, clock| {
        if args.len() != 2 {
            return Err(DbError::Execution("JSON_VALUE expects 2 arguments".into()));
        }
        let json_val = crate::executor::evaluator::eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        let path_val = crate::executor::evaluator::eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
        if json_val.is_null() || path_val.is_null() {
            return Ok(Value::Null);
        }
        json::json_value(&json_val.to_string_value(), &path_val.to_string_value())
    }),
    builtin!("JSON_QUERY" => |args, row, ctx, catalog, storage, clock| {
        if args.len() != 2 {
            return Err(DbError::Execution("JSON_QUERY expects 2 arguments".into()));
        }
        let json_val = crate::executor::evaluator::eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        let path_val = crate::executor::evaluator::eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
        if json_val.is_null() || path_val.is_null() {
            return Ok(Value::Null);
        }
        json::json_query(&json_val.to_string_value(), &path_val.to_string_value())
    }),
    builtin!("JSON_MODIFY" => |args, row, ctx, catalog, storage, clock| {
        if args.len() != 3 {
            return Err(DbError::Execution("JSON_MODIFY expects 3 arguments".into()));
        }
        let json_val = crate::executor::evaluator::eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        let path_val = crate::executor::evaluator::eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
        let new_val = crate::executor::evaluator::eval_expr(&args[2], row, ctx, catalog, storage, clock)?;
        if json_val.is_null() || path_val.is_null() {
            return Ok(Value::Null);
        }
        json::json_modify(
            &json_val.to_string_value(),
            &path_val.to_string_value(),
            &new_val.to_string_value(),
        )
    }),
    builtin!("JSON_PATH_EXISTS" => |args, row, ctx, catalog, storage, clock| {
        if args.len() != 2 {
            return Err(DbError::Execution("JSON_PATH_EXISTS expects 2 arguments".into()));
        }
        let json_val = crate::executor::evaluator::eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        let path_val = crate::executor::evaluator::eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
        if json_val.is_null() || path_val.is_null() {
            return Ok(Value::Null);
        }
        json::json_path_exists(&json_val.to_string_value(), &path_val.to_string_value())
    }),
    builtin!("JSON_CONTAINS" => |args, row, ctx, catalog, storage, clock| {
        if args.len() < 2 || args.len() > 4 {
            return Err(DbError::Execution("JSON_CONTAINS expects 2 to 4 arguments".into()));
        }
        let json_val = crate::executor::evaluator::eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        let search_val = crate::executor::evaluator::eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
        if json_val.is_null() || search_val.is_null() {
            return Ok(Value::Null);
        }
        let path_opt = if let Some(p_arg) = args.get(2) {
            let pv = crate::executor::evaluator::eval_expr(p_arg, row, ctx, catalog, storage, clock)?;
            if pv.is_null() {
                return Ok(Value::Null);
            }
            Some(pv.to_string_value())
        } else {
            None
        };
        let search_mode = if let Some(m_arg) = args.get(3) {
            let mv = crate::executor::evaluator::eval_expr(m_arg, row, ctx, catalog, storage, clock)?;
            if mv.is_null() {
                return Ok(Value::Null);
            }
            mv.to_integer_i64().ok_or_else(|| {
                DbError::Execution("JSON_CONTAINS search_mode must be an integer (0 or 1)".into())
            })?
        } else {
            0
        };
        json::json_contains(&json_val.to_string_value(), &search_val, path_opt.as_deref(), search_mode)
    }),
    builtin!("ISJSON" => |args, row, ctx, catalog, storage, clock| {
        if args.is_empty() || args.len() > 2 {
            return Err(DbError::Execution("ISJSON expects 1 or 2 arguments".into()));
        }
        let json_val = crate::executor::evaluator::eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        if json_val.is_null() {
            return Ok(Value::Null);
        }
        if args.len() == 2 {
            let type_val = crate::executor::evaluator::eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
            if type_val.is_null() {
                return Ok(Value::Null);
            }
            json::is_json_typed(&json_val.to_string_value(), &type_val.to_string_value())
        } else {
            json::is_json(&json_val.to_string_value())
        }
    }),
    builtin!("JSON_ARRAY_LENGTH" => |args, row, ctx, catalog, storage, clock| {
        if args.len() != 1 {
            return Err(DbError::Execution("JSON_ARRAY_LENGTH expects 1 argument".into()));
        }
        let json_val = crate::executor::evaluator::eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        if json_val.is_null() {
            return Ok(Value::Null);
        }
        json::json_array_length(&json_val.to_string_value())
    }),
    builtin!("JSON_KEYS" => |args, row, ctx, catalog, storage, clock| {
        if args.is_empty() || args.len() > 2 {
            return Err(DbError::Execution("JSON_KEYS expects 1 or 2 arguments".into()));
        }
        let json_val = crate::executor::evaluator::eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
        if json_val.is_null() {
            return Ok(Value::Null);
        }
        let path = if args.len() == 2 {
            let path_val = crate::executor::evaluator::eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
            if path_val.is_null() {
                return Ok(Value::Null);
            }
            Some(path_val.to_string_value())
        } else {
            None
        };
        json::json_keys(&json_val.to_string_value(), path.as_deref())
    }),
    builtin!("__JSON_OBJECT_NULL" => |args, row, ctx, catalog, storage, clock| {
        eval_json_object_fn(args, false, row, ctx, catalog, storage, clock)
    }),
    builtin!("__JSON_OBJECT_ABSENT" => |args, row, ctx, catalog, storage, clock| {
        eval_json_object_fn(args, true, row, ctx, catalog, storage, clock)
    }),
    builtin!("JSON_OBJECT" => |args, row, ctx, catalog, storage, clock| {
        eval_json_object_fn(args, false, row, ctx, catalog, storage, clock)
    }),
    builtin!("__JSON_ARRAY_ABSENT" => |args, row, ctx, catalog, storage, clock| {
        eval_json_array_fn(args, false, row, ctx, catalog, storage, clock)
    }),
    builtin!("__JSON_ARRAY_NULL" => |args, row, ctx, catalog, storage, clock| {
        eval_json_array_fn(args, true, row, ctx, catalog, storage, clock)
    }),
    builtin!("JSON_ARRAY" => |args, row, ctx, catalog, storage, clock| {
        eval_json_array_fn(args, false, row, ctx, catalog, storage, clock)
    }),
];

fn eval_json_object_fn(
    args: &[Expr],
    absent_on_null: bool,
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if !args.len().is_multiple_of(2) {
        return Err(DbError::Execution(
            "JSON_OBJECT requires key:value pairs".into(),
        ));
    }
    let mut pairs = Vec::with_capacity(args.len() / 2);
    for pair in args.as_chunks::<2>().0 {
        let key_val =
            crate::executor::evaluator::eval_expr(&pair[0], row, ctx, catalog, storage, clock)?;
        if key_val.is_null() {
            return Err(DbError::Execution("JSON_OBJECT key cannot be NULL".into()));
        }
        let val =
            crate::executor::evaluator::eval_expr(&pair[1], row, ctx, catalog, storage, clock)?;
        pairs.push((
            key_val.to_string_value(),
            val,
            json::is_json_producing_expr(&pair[1]),
        ));
    }
    Ok(json::build_json_object(&pairs, absent_on_null))
}

fn eval_json_array_fn(
    args: &[Expr],
    null_on_null: bool,
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let mut items = Vec::with_capacity(args.len());
    for arg in args {
        let val = crate::executor::evaluator::eval_expr(arg, row, ctx, catalog, storage, clock)?;
        items.push((val, json::is_json_producing_expr(arg)));
    }
    Ok(json::build_json_array(&items, null_on_null))
}

const REGEXP_FUNCTIONS: &[BuiltinScalarFunction] = &[
    builtin!("REGEXP_LIKE" => regexp::eval_regexp_like),
    builtin!("REGEXP_REPLACE" => regexp::eval_regexp_replace),
    builtin!("REGEXP_SUBSTR" => regexp::eval_regexp_substr),
    builtin!("REGEXP_INSTR" => regexp::eval_regexp_instr),
    builtin!("REGEXP_COUNT" => regexp::eval_regexp_count),
];

const FUZZY_FUNCTIONS: &[BuiltinScalarFunction] = &[
    builtin!("EDIT_DISTANCE" => fuzzy::eval_edit_distance),
    builtin!("EDIT_DISTANCE_SIMILARITY" => fuzzy::eval_edit_distance_similarity),
    builtin!("JARO_WINKLER_DISTANCE" => fuzzy::eval_jaro_winkler_distance),
    builtin!("JARO_WINKLER_SIMILARITY" => fuzzy::eval_jaro_winkler_similarity),
];

fn builtin_handler_map() -> &'static HashMap<&'static str, ScalarHandler> {
    static HANDLERS: OnceLock<HashMap<&'static str, ScalarHandler>> = OnceLock::new();
    HANDLERS.get_or_init(|| {
        let mut handlers = HashMap::new();
        for group in [
            SYSTEM_FUNCTIONS,
            DATETIME_FUNCTIONS,
            STRING_FUNCTIONS,
            MATH_FUNCTIONS,
            LOGIC_FUNCTIONS,
            JSON_FUNCTIONS,
            REGEXP_FUNCTIONS,
            FUZZY_FUNCTIONS,
        ] {
            for entry in group {
                handlers.entry(entry.name).or_insert(entry.handler);
            }
        }
        handlers
    })
}

pub(crate) fn lookup_builtin_handler(name: &str) -> Option<ScalarHandler> {
    builtin_handler_map().get(name).copied()
}

pub(crate) fn registered_builtin_scalar_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = builtin_handler_map().keys().copied().collect();
    names.sort_unstable();
    names
}

pub(crate) fn lookup_system_variable(
    name: &str,
    ctx: &ExecutionContext<'_>,
) -> Option<Result<Value, DbError>> {
    if !name.starts_with("@@") {
        return None;
    }
    match exec_metadata::system_vars::resolve_system_variable(name, ctx) {
        Ok(Some(val)) => Some(Ok(val)),
        Ok(None) => None,
        Err(err) => Some(Err(err)),
    }
}
