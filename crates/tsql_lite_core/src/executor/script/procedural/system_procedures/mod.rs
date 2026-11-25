use super::super::ScriptExecutor;
use crate::ast::ExecProcedureStmt;
use crate::error::DbError;
use crate::executor::context::ExecutionContext;
use crate::executor::evaluator::eval_expr;
use crate::executor::result::QueryResult;
use crate::types::{DataType, Value};
pub mod describe;
pub mod metadata;
pub mod security;
pub mod session;

use describe::*;
use metadata::*;
use security::*;
use session::*;

const SYSTEM_PROCEDURES: &[&str] = &[
    "sp_rename",
    "sp_help",
    "sp_helptext",
    "sp_columns",
    "sp_tables",
    "sp_tablecollations_100",
    "sp_tablecollations",
    "sp_helpindex",
    "sp_helpconstraint",
    "sp_set_session_context",
    "xp_instance_regread",
    "sp_msgetversion",
    "sp_who",
    "sp_databases",
    "sp_helpdb",
    "sp_server_info",
    "sp_monitor",
    "sp_helpuser",
    "sp_helprole",
    "sp_helprolemember",
    "sp_helpsrvrole",
    "sp_helpsrvrolemember",
    "sp_helpfile",
    "sp_helpfilegroup",
    "sp_describe_first_result_set",
    "sp_describe_undeclared_parameters",
    "sp_refresh_parameter_encryption",
    "sp_refreshsqlmodule",
    "sp_refreshview",
    "sp_getapplock",
    "sp_releaseapplock",
    "sp_addextendedproperty",
    "sp_updateextendedproperty",
    "sp_dropextendedproperty",
    "sp_tableoption",
    "sp_xml_preparedocument",
    "sp_xml_removedocument",
    "sp_msforeachtable",
];

pub(crate) fn is_system_procedure(name: &str) -> bool {
    SYSTEM_PROCEDURES
        .iter()
        .any(|sp| name.eq_ignore_ascii_case(sp))
}

pub(crate) fn execute_system_procedure(
    exec: &mut ScriptExecutor<'_>,
    stmt: &ExecProcedureStmt,
    ctx: &mut ExecutionContext<'_>,
) -> Result<Option<QueryResult>, DbError> {
    let name = &stmt.name.name;
    let args = eval_args(exec, &stmt.args, ctx)?;

    let result = if name.eq_ignore_ascii_case("sp_rename") {
        execute_sp_rename(exec, &args)?
    } else if name.eq_ignore_ascii_case("sp_help") {
        execute_sp_help(exec, &args)?
    } else if name.eq_ignore_ascii_case("sp_helptext") {
        execute_sp_helptext(exec, &args)?
    } else if name.eq_ignore_ascii_case("sp_columns") {
        execute_sp_columns(exec, &args)?
    } else if name.eq_ignore_ascii_case("sp_tables") {
        execute_sp_tables(exec)?
    } else if name.eq_ignore_ascii_case("sp_tablecollations_100")
        || name.eq_ignore_ascii_case("sp_tablecollations")
    {
        execute_sp_tablecollations_100(exec, &args, ctx)?
    } else if name.eq_ignore_ascii_case("sp_helpindex") {
        execute_sp_helpindex(exec, &args)?
    } else if name.eq_ignore_ascii_case("sp_helpconstraint") {
        execute_sp_helpconstraint(exec, &args)?
    } else if name.eq_ignore_ascii_case("sp_set_session_context") {
        execute_sp_set_session_context(stmt, ctx, exec)?
    } else if name.eq_ignore_ascii_case("sp_who") {
        execute_sp_who(ctx)?
    } else if name.eq_ignore_ascii_case("sp_databases") {
        execute_sp_databases()?
    } else if name.eq_ignore_ascii_case("sp_helpdb") {
        execute_sp_helpdb(exec, &args)?
    } else if name.eq_ignore_ascii_case("sp_server_info") {
        execute_sp_server_info()?
    } else if name.eq_ignore_ascii_case("sp_monitor") {
        execute_sp_monitor(exec)?
    } else if name.eq_ignore_ascii_case("sp_helpuser") {
        execute_sp_helpuser(exec, ctx)?
    } else if name.eq_ignore_ascii_case("sp_helprole") {
        execute_sp_helprole(exec, ctx)?
    } else if name.eq_ignore_ascii_case("sp_helprolemember") {
        execute_sp_helprolemember(exec, ctx)?
    } else if name.eq_ignore_ascii_case("sp_helpsrvrole") {
        execute_sp_helpsrvrole(exec, ctx)?
    } else if name.eq_ignore_ascii_case("sp_helpsrvrolemember") {
        execute_sp_helpsrvrolemember(exec, ctx)?
    } else if name.eq_ignore_ascii_case("sp_helpfile") {
        execute_sp_helpfile(exec, ctx)?
    } else if name.eq_ignore_ascii_case("sp_helpfilegroup") {
        execute_sp_helpfilegroup(exec, ctx)?
    } else if name.eq_ignore_ascii_case("xp_instance_regread") {
        // Stub for registry reads. If it has an output parameter, set it to a default.
        for arg in &stmt.args {
            if arg.is_output {
                if let crate::ast::Expr::Identifier(ref var_name) = arg.expr {
                    if let Some((ty, val)) = ctx.session.variables.get_mut(var_name) {
                        *val = crate::executor::value_ops::coerce_value_to_type_with_dateformat(
                            Value::Int(0),
                            ty,
                            &ctx.options.dateformat,
                        )?;
                    }
                }
            }
        }
        QueryResult::default()
    } else if name.eq_ignore_ascii_case("sp_msgetversion") {
        // Stub for version check
        QueryResult {
            columns: vec!["Character_Value".into()],
            column_types: vec![DataType::NVarChar { max_len: 128 }],
            column_nullabilities: vec![false],
            rows: vec![vec![Value::NVarChar("16.0.1000.0".into())]],
            ..Default::default()
        }
    } else if name.eq_ignore_ascii_case("sp_describe_first_result_set") {
        execute_sp_describe_first_result_set(exec, &args, ctx)?
    } else if name.eq_ignore_ascii_case("sp_describe_undeclared_parameters") {
        execute_sp_describe_undeclared_parameters(exec, &args, ctx)?
    } else if name.eq_ignore_ascii_case("sp_refresh_parameter_encryption")
        || name.eq_ignore_ascii_case("sp_refreshsqlmodule")
        || name.eq_ignore_ascii_case("sp_refreshview")
    {
        QueryResult::default()
    } else if name.eq_ignore_ascii_case("sp_tableoption") {
        execute_sp_tableoption(exec, &args)?
    } else if name.eq_ignore_ascii_case("sp_addextendedproperty") {
        execute_extended_property_proc(exec, stmt, ctx, ExtendedPropOp::Add)?
    } else if name.eq_ignore_ascii_case("sp_updateextendedproperty") {
        execute_extended_property_proc(exec, stmt, ctx, ExtendedPropOp::Update)?
    } else if name.eq_ignore_ascii_case("sp_dropextendedproperty") {
        execute_extended_property_proc(exec, stmt, ctx, ExtendedPropOp::Drop)?
    } else if name.eq_ignore_ascii_case("sp_getapplock") {
        execute_sp_getapplock(stmt, ctx, exec)?
    } else if name.eq_ignore_ascii_case("sp_releaseapplock") {
        execute_sp_releaseapplock(stmt, ctx, exec)?
    } else if name.eq_ignore_ascii_case("sp_xml_preparedocument") {
        execute_sp_xml_preparedocument(stmt, ctx, exec)?
    } else if name.eq_ignore_ascii_case("sp_xml_removedocument") {
        execute_sp_xml_removedocument(stmt, ctx, exec)?
    } else if name.eq_ignore_ascii_case("sp_msforeachtable") {
        execute_sp_msforeachtable(stmt, ctx, exec)?
    } else {
        return Err(DbError::Execution(format!(
            "unknown system procedure '{}'",
            name
        )));
    };

    let mut res = result;
    res.return_status.get_or_insert(0);
    res.is_procedure = true;
    Ok(Some(res))
}

fn eval_args(
    exec: &mut ScriptExecutor<'_>,
    args: &[crate::ast::ExecArgument],
    ctx: &mut ExecutionContext<'_>,
) -> Result<Vec<String>, DbError> {
    let mut result = Vec::new();
    for arg in args {
        let val = eval_expr(&arg.expr, &[], ctx, exec.catalog, exec.storage, exec.clock)?;
        result.push(val.to_string_value());
    }
    Ok(result)
}

fn execute_sp_tableoption(
    exec: &mut ScriptExecutor<'_>,
    args: &[String],
) -> Result<QueryResult, DbError> {
    if args.len() >= 3
        && args[1]
            .trim()
            .eq_ignore_ascii_case("large value types out of row")
    {
        let raw_table = args[0].trim();
        let (schema, name) = if let Some((s, n)) = raw_table.rsplit_once('.') {
            (
                s.trim().trim_matches(&['[', ']'][..]),
                n.trim().trim_matches(&['[', ']'][..]),
            )
        } else {
            ("dbo", raw_table.trim_matches(&['[', ']'][..]))
        };
        if let Some(table_id) = exec.catalog.find_table(schema, name).map(|t| t.id) {
            let v = args[2].trim();
            let enabled = matches!(v.to_ascii_lowercase().as_str(), "1" | "on" | "true");
            exec.catalog
                .set_table_large_value_out_of_row(table_id, enabled);
        }
    }
    Ok(QueryResult::default())
}

#[derive(Clone, Copy)]
enum ExtendedPropOp {
    Add,
    Update,
    Drop,
}

fn execute_extended_property_proc(
    exec: &mut ScriptExecutor<'_>,
    stmt: &ExecProcedureStmt,
    ctx: &mut ExecutionContext<'_>,
    op: ExtendedPropOp,
) -> Result<QueryResult, DbError> {
    let positional_order: &[&str] = match op {
        ExtendedPropOp::Add | ExtendedPropOp::Update => &[
            "name",
            "value",
            "level0type",
            "level0name",
            "level1type",
            "level1name",
            "level2type",
            "level2name",
        ],
        ExtendedPropOp::Drop => &[
            "name",
            "level0type",
            "level0name",
            "level1type",
            "level1name",
            "level2type",
            "level2name",
        ],
    };

    let mut named: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    for (idx, arg) in stmt.args.iter().enumerate() {
        let key = if let Some(ref p) = arg.name {
            p.trim_start_matches('@').to_ascii_lowercase()
        } else if let Some(&pos_name) = positional_order.get(idx) {
            pos_name.to_string()
        } else {
            continue;
        };
        let val = eval_expr(&arg.expr, &[], ctx, exec.catalog, exec.storage, exec.clock)?;
        let opt = if val.is_null() {
            None
        } else {
            Some(val.to_string_value())
        };
        named.insert(key, opt);
    }

    let Some(prop_name) = named.get("name").and_then(|v| v.clone()) else {
        return Ok(QueryResult::default());
    };
    let prop_value = named.get("value").and_then(|v| v.clone());
    let l0_type = named.get("level0type").and_then(|v| v.as_deref());
    let l0_name = named.get("level0name").and_then(|v| v.as_deref());
    let l1_type = named.get("level1type").and_then(|v| v.as_deref());
    let l1_name = named.get("level1name").and_then(|v| v.as_deref());
    let l2_type = named.get("level2type").and_then(|v| v.as_deref());
    let l2_name = named.get("level2name").and_then(|v| v.as_deref());

    let resolved: Option<(i32, &'static str, i32, i32)> = match (l0_type, l0_name, l1_type, l1_name)
    {
        (None, _, _, _) => Some((0, "DATABASE", 0, 0)),
        (Some(t0), Some(n0), None, _) if t0.eq_ignore_ascii_case("SCHEMA") => exec
            .catalog
            .get_schema_id(n0)
            .map(|sid| (3, "SCHEMA", sid as i32, 0)),
        (Some(t0), Some(n0), Some(_t1), Some(n1)) if t0.eq_ignore_ascii_case("SCHEMA") => {
            let major_opt = exec
                .catalog
                .object_id(n0, n1)
                .or_else(|| exec.catalog.find_table_type(n0, n1).map(|tt| tt.object_id))
                .or_else(|| exec.catalog.find_scalar_type(n0, n1).map(|st| st.object_id));
            major_opt.map(|major_id| {
                let minor_id = match (l2_type, l2_name) {
                    (Some(t2), Some(n2)) if t2.eq_ignore_ascii_case("COLUMN") => exec
                        .catalog
                        .find_table(n0, n1)
                        .and_then(|tbl| {
                            tbl.columns
                                .iter()
                                .find(|c| c.name.eq_ignore_ascii_case(n2))
                                .map(|c| c.id as i32)
                        })
                        .unwrap_or(0),
                    _ => 0,
                };
                (1, "OBJECT_OR_COLUMN", major_id, minor_id)
            })
        }
        _ => None,
    };

    if let Some((class, class_desc, major_id, minor_id)) = resolved {
        match op {
            ExtendedPropOp::Add => {
                exec.catalog
                    .add_extended_property(crate::catalog::ExtendedPropertyDef {
                        class,
                        class_desc: class_desc.to_string(),
                        major_id,
                        minor_id,
                        name: prop_name,
                        value: prop_value,
                    })?;
            }
            ExtendedPropOp::Update => {
                exec.catalog
                    .update_extended_property(crate::catalog::ExtendedPropertyDef {
                        class,
                        class_desc: class_desc.to_string(),
                        major_id,
                        minor_id,
                        name: prop_name,
                        value: prop_value,
                    })?;
            }
            ExtendedPropOp::Drop => {
                exec.catalog
                    .drop_extended_property(class, major_id, minor_id, &prop_name)?;
            }
        }
    }

    Ok(QueryResult::default())
}
