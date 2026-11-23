use super::types::RpcParam;

/// Build DECLARE preamble for all RPC parameters.
pub fn build_param_preamble(params: &[RpcParam]) -> String {
    let mut out = String::new();
    for p in params {
        if p.name.is_empty() {
            continue;
        }
        if let Some(rows) = &p.tvp_rows {
            let decl_type = normalize_decl_type(&p.type_name);
            out.push_str(&format!("DECLARE {} {};\n", p.name, decl_type));
            for row in rows {
                out.push_str(&format!(
                    "INSERT INTO {} VALUES ({});\n",
                    p.name,
                    row.join(", ")
                ));
            }
        } else {
            out.push_str(&format!(
                "DECLARE {} {} = {};\n",
                p.name, p.type_name, p.value_sql
            ));
        }
    }
    out
}

pub(super) fn normalize_decl_type(type_name: &str) -> String {
    let t = type_name.trim();
    if let Some(idx) = t.to_uppercase().find(" READONLY") {
        t[..idx].trim().to_string()
    } else {
        t.to_string()
    }
}

pub fn build_param_preamble_with_decls(
    params: &[super::types::RpcParam],
    decls: &[(String, String)],
) -> String {
    let mut out = String::new();
    for (i, p) in params.iter().enumerate() {
        if p.name.is_empty() {
            continue;
        }
        let type_name = if i < decls.len() {
            normalize_decl_type(&decls[i].1)
        } else {
            normalize_decl_type(&p.type_name)
        };
        if let Some(rows) = &p.tvp_rows {
            out.push_str(&format!("DECLARE {} {};\n", p.name, type_name));
            for row in rows {
                out.push_str(&format!(
                    "INSERT INTO {} VALUES ({});\n",
                    p.name,
                    row.join(", ")
                ));
            }
        } else {
            out.push_str(&format!(
                "DECLARE {} {} = {};\n",
                p.name, type_name, p.value_sql
            ));
        }
    }
    out
}

/// Build an `EXEC <proc> ...` SQL script (plus any TVP preambles) for a named
/// stored procedure RPC call.
pub fn build_stored_proc_exec(name: &str, params: &[RpcParam]) -> String {
    let mut preamble = String::new();
    let mut call_args = Vec::with_capacity(params.len());

    // Detect duplicate parameter names (e.g. `xp_msver` invoked with repeated `@optname`).
    let mut has_duplicate_names = false;
    for (i, p) in params.iter().enumerate() {
        if !p.name.is_empty()
            && params[..i]
                .iter()
                .any(|prev| !prev.name.is_empty() && prev.name.eq_ignore_ascii_case(&p.name))
        {
            has_duplicate_names = true;
            break;
        }
    }

    for (i, p) in params.iter().enumerate() {
        let arg_value = if let Some(rows) = &p.tvp_rows {
            let tvp_var = format!("@__rpc_tvp_{}", i);
            let decl_type = normalize_decl_type(&p.type_name);
            preamble.push_str(&format!("DECLARE {} {};\n", tvp_var, decl_type));
            for row in rows {
                preamble.push_str(&format!(
                    "INSERT INTO {} VALUES ({});\n",
                    tvp_var,
                    row.join(", ")
                ));
            }
            tvp_var
        } else {
            p.value_sql.clone()
        };

        if !has_duplicate_names && !p.name.is_empty() {
            let pname = if p.name.starts_with('@') {
                p.name.clone()
            } else {
                format!("@{}", p.name)
            };
            call_args.push(format!("{} = {}", pname, arg_value));
        } else {
            call_args.push(arg_value);
        }
    }

    let exec_line = if call_args.is_empty() {
        format!("EXEC {}", name.trim())
    } else {
        format!("EXEC {} {}", name.trim(), call_args.join(", "))
    };

    if preamble.is_empty() {
        exec_line
    } else {
        format!("{}{}", preamble, exec_line)
    }
}
