use std::collections::HashSet;

use crate::catalog::Catalog;
use crate::error::DbError;
use crate::executor::context::ExecutionContext;

/// Returns the active effective user name (impersonated user takes precedence over session login).
#[inline]
pub(crate) fn effective_user<'a>(ctx: &'a ExecutionContext<'_>) -> Option<&'a str> {
    ctx.session
        .extras
        .impersonation_stack
        .last()
        .map(String::as_str)
        .or(ctx.metadata.user.as_deref())
}

#[inline]
fn is_privileged_user(user: &str) -> bool {
    user.eq_ignore_ascii_case("dbo") || user.eq_ignore_ascii_case("sa")
}

pub(crate) fn execute_as(
    principal: &str,
    catalog: &dyn Catalog,
    ctx: &mut ExecutionContext<'_>,
) -> Result<(), DbError> {
    let clean = principal.trim();
    if !is_privileged_user(clean)
        && !clean.eq_ignore_ascii_case("guest")
        && !catalog
            .get_principals()
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case(clean))
    {
        return Err(DbError::Execution(format!(
            "Cannot execute as the database principal because the principal \"{}\" does not exist.",
            clean
        )));
    }
    ctx.session
        .extras
        .impersonation_stack
        .push(clean.to_string());
    ctx.metadata.user = Some(clean.to_string());
    Ok(())
}

pub(crate) fn revert(ctx: &mut ExecutionContext<'_>) -> Result<(), DbError> {
    ctx.session.extras.impersonation_stack.pop();
    ctx.metadata.user = ctx.session.extras.impersonation_stack.last().cloned();
    Ok(())
}

/// Enforces GRANT / DENY / REVOKE permissions when running under a non-dbo impersonated user.
/// Fast-path: returns `Ok(())` in a single branch when running as default `dbo` / `sa`.
pub(crate) fn check_object_permission(
    catalog: &dyn Catalog,
    ctx: &ExecutionContext<'_>,
    permission: &str,
    schema: &str,
    object_name: &str,
    object_id: i32,
) -> Result<(), DbError> {
    let Some(user) = effective_user(ctx) else {
        return Ok(());
    };
    if is_privileged_user(user) {
        return Ok(());
    }
    // Always allow system/temp objects
    if schema.eq_ignore_ascii_case("sys")
        || schema.eq_ignore_ascii_case("INFORMATION_SCHEMA")
        || object_name.starts_with('#')
        || object_name.starts_with('@')
    {
        return Ok(());
    }

    let principals = catalog.get_principals();
    let Some(user_principal) = principals
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(user))
    else {
        return Ok(());
    };

    // Collect principal + all transitive role memberships + public (0)
    let role_members = catalog.get_role_members();
    let mut active_ids: HashSet<i32> = HashSet::new();
    active_ids.insert(user_principal.principal_id);
    active_ids.insert(0); // public role
    let mut changed = true;
    while changed {
        changed = false;
        for rm in role_members {
            if active_ids.contains(&rm.member_principal_id)
                && active_ids.insert(rm.role_principal_id)
            {
                changed = true;
            }
        }
    }

    // Check fixed database role names
    let mut is_db_owner = false;
    let mut is_datareader = false;
    let mut is_datawriter = false;
    let mut is_denydatareader = false;
    let mut is_denydatawriter = false;
    for p in principals {
        if active_ids.contains(&p.principal_id) {
            if p.name.eq_ignore_ascii_case("db_owner") {
                is_db_owner = true;
            } else if p.name.eq_ignore_ascii_case("db_datareader") {
                is_datareader = true;
            } else if p.name.eq_ignore_ascii_case("db_datawriter") {
                is_datawriter = true;
            } else if p.name.eq_ignore_ascii_case("db_denydatareader") {
                is_denydatareader = true;
            } else if p.name.eq_ignore_ascii_case("db_denydatawriter") {
                is_denydatawriter = true;
            }
        }
    }
    if is_db_owner {
        return Ok(());
    }

    let perm_upper = permission.to_ascii_uppercase();
    let db_name = ctx.metadata.database.as_deref().unwrap_or("master");
    let deny_err = || {
        DbError::Execution(format!(
            "The {} permission was denied on the object '{}', database '{}', schema '{}'.",
            perm_upper, object_name, db_name, schema
        ))
    };

    if is_denydatareader && perm_upper == "SELECT" {
        return Err(deny_err());
    }
    if is_denydatawriter && matches!(perm_upper.as_str(), "INSERT" | "UPDATE" | "DELETE") {
        return Err(deny_err());
    }

    let mut granted = (is_datareader && perm_upper == "SELECT")
        || (is_datawriter && matches!(perm_upper.as_str(), "INSERT" | "UPDATE" | "DELETE"));

    let schema_id = catalog.get_schema_id(schema).map(|id| id as i32);
    for perm in catalog.get_permissions() {
        if !active_ids.contains(&perm.grantee_principal_id) {
            continue;
        }
        let scope_matches = (perm.class == 0 && perm.major_id == 0)
            || (perm.class == 1 && perm.major_id == object_id)
            || (perm.class == 3 && schema_id == Some(perm.major_id));
        if !scope_matches {
            continue;
        }
        let name_matches = perm.permission_name.eq_ignore_ascii_case(&perm_upper)
            || perm.permission_name.eq_ignore_ascii_case("CONTROL")
            || perm.permission_name.eq_ignore_ascii_case("ALL");
        if !name_matches {
            continue;
        }
        if perm.state.eq_ignore_ascii_case("D") {
            return Err(deny_err());
        }
        if perm.state.eq_ignore_ascii_case("G") || perm.state.eq_ignore_ascii_case("W") {
            granted = true;
        }
    }

    if granted {
        Ok(())
    } else {
        Err(deny_err())
    }
}

/// Returns `true` if the active effective user is allowed to see unmasked data on `(schema, table_id)`.
/// `dbo`, `sa`, `db_owner` members, and principals with `UNMASK` (at DATABASE, SCHEMA, or OBJECT scope)
/// see unmasked values; explicit `DENY UNMASK` overrides `GRANT UNMASK`.
pub(crate) fn user_has_unmask(
    catalog: &dyn Catalog,
    ctx: &ExecutionContext<'_>,
    schema: &str,
    table_id: u32,
) -> bool {
    let Some(user) = effective_user(ctx) else {
        return true;
    };
    if is_privileged_user(user) {
        return true;
    }
    let principals = catalog.get_principals();
    let Some(user_principal) = principals
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(user))
    else {
        return true;
    };
    let role_members = catalog.get_role_members();
    let mut active_ids: HashSet<i32> = HashSet::new();
    active_ids.insert(user_principal.principal_id);
    active_ids.insert(0); // public
    let mut changed = true;
    while changed {
        changed = false;
        for rm in role_members {
            if active_ids.contains(&rm.member_principal_id)
                && active_ids.insert(rm.role_principal_id)
            {
                changed = true;
            }
        }
    }
    for p in principals {
        if active_ids.contains(&p.principal_id) && p.name.eq_ignore_ascii_case("db_owner") {
            return true;
        }
    }
    let schema_id = catalog.get_schema_id(schema).map(|id| id as i32);
    let mut granted = false;
    for perm in catalog.get_permissions() {
        if !active_ids.contains(&perm.grantee_principal_id) {
            continue;
        }
        let scope_matches = (perm.class == 0 && perm.major_id == 0)
            || (perm.class == 1 && perm.major_id == table_id as i32)
            || (perm.class == 3 && schema_id == Some(perm.major_id));
        if !scope_matches {
            continue;
        }
        if !perm.permission_name.eq_ignore_ascii_case("UNMASK")
            && !perm.permission_name.eq_ignore_ascii_case("CONTROL")
        {
            continue;
        }
        if perm.state.eq_ignore_ascii_case("D") {
            return false;
        }
        if perm.state.eq_ignore_ascii_case("G") || perm.state.eq_ignore_ascii_case("W") {
            granted = true;
        }
    }
    granted
}
