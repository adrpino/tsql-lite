pub(crate) mod bitwise;
mod builtin_registry;
pub(crate) mod datetime;
pub(crate) mod logic;
pub(crate) mod math;
pub(crate) mod metadata;
pub(crate) mod string;
pub(crate) mod system;
pub(crate) mod tsql_subsystems;
pub(crate) mod udf;
pub(crate) mod vector;

/// Returns `true` if `name` (case-insensitive) is registered as a built-in scalar function.
pub fn is_builtin_scalar_registered(name: &str) -> bool {
    builtin_registry::lookup_builtin_handler(&name.to_ascii_uppercase()).is_some()
}

/// Returns a sorted list of all registered built-in scalar function names (uppercase).
pub fn registered_builtin_scalar_names() -> Vec<&'static str> {
    builtin_registry::registered_builtin_scalar_names()
}

use crate::ast::Expr;
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::storage::Storage;
use crate::types::Value;

use super::aggregates::{dispatch_aggregate, is_aggregate_function};
use super::clock::Clock;
use super::context::ExecutionContext;
use super::model::ContextTable;
use super::string_norm::normalize_identifier;
use builtin_registry::{lookup_builtin_handler, lookup_system_variable};

#[allow(clippy::too_many_arguments)]
pub(crate) fn eval_function(
    name: &str,
    args: &[Expr],
    within_group: Option<&[crate::ast::OrderByExpr]>,
    row: &[ContextTable],
    ctx: &mut ExecutionContext<'_>,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let mut stack_buf = [0u8; 64];
    let owned_upper: String;
    let upper_str: &str = if name.is_ascii() && name.len() <= stack_buf.len() {
        for (dst, &b) in stack_buf.iter_mut().zip(name.as_bytes()) {
            *dst = b.to_ascii_uppercase();
        }
        std::str::from_utf8(&stack_buf[..name.len()]).unwrap()
    } else {
        owned_upper = normalize_identifier(name);
        owned_upper.as_str()
    };

    if is_aggregate_function(upper_str) {
        if let Some(group) = ctx.current_group().clone() {
            let effective = if let Some(ob) = within_group.filter(|v| !v.is_empty()) {
                super::aggregates::sort_group_for_within_group(
                    &group, ob, ctx, catalog, storage, clock,
                )
            } else {
                group
            };
            if let Some(res) = dispatch_aggregate(
                upper_str,
                args,
                within_group,
                &effective,
                ctx,
                catalog,
                storage,
                clock,
            ) {
                return res;
            }
        }
    }

    // Check $PARTITION.<pf_name>(value) calls
    if let Some(pf_name) = name
        .strip_prefix("$PARTITION.")
        .or_else(|| name.strip_prefix("$partition."))
        .or_else(|| {
            if upper_str.starts_with("$PARTITION.") {
                Some(&name["$PARTITION.".len()..])
            } else {
                None
            }
        })
    {
        return tsql_subsystems::eval_partition_function(
            pf_name, args, row, ctx, catalog, storage, clock,
        );
    }

    // Check built-in functions first
    if let Some(handler) = lookup_builtin_handler(upper_str) {
        return handler(args, row, ctx, catalog, storage, clock);
    }

    // Check system variables (@@VAR)
    if let Some(result) = lookup_system_variable(upper_str, ctx) {
        return result;
    }

    // Handle aggregates not in grouped context
    if is_aggregate_function(upper_str) {
        return match upper_str {
            "MIN" | "MAX" => Err(DbError::Execution(
                "MIN/MAX require a FROM clause when used in scalar context (use in GROUP BY)"
                    .into(),
            )),
            _ => Err(DbError::Execution(format!(
                "{} is only supported in grouped projection",
                name
            ))),
        };
    }
    udf::eval_user_scalar_function(name, args, row, ctx, catalog, storage, clock)
}
