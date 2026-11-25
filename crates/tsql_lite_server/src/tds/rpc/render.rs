use super::types::{OutputParamBinding, RpcParam};

/// Build DECLARE preamble for all RPC parameters.
pub fn build_param_preamble(params: &[RpcParam]) -> String {
    build_param_preamble_with_outputs(params).0
}

pub fn build_param_preamble_with_outputs(params: &[RpcParam]) -> (String, Vec<OutputParamBinding>) {
    let mut out = String::new();
    let mut outputs = Vec::new();
    for (i, p) in params.iter().enumerate() {
        if p.name.is_empty() {
            continue;
        }
        let decl_type = normalize_decl_type(&p.type_name);
        if let Some(rows) = &p.tvp_rows {
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
                p.name, decl_type, p.value_sql
            ));
            if p.is_output {
                outputs.push(OutputParamBinding {
                    ordinal: (i + 2) as u16, // after @stmt and @params in sp_executesql
                    param_name: p.name.clone(),
                    var_name: p.name.clone(),
                });
            }
        }
    }
    (out, outputs)
}

pub(super) fn normalize_decl_type(type_name: &str) -> String {
    let mut t = type_name.trim().to_string();
    let upper = t.to_uppercase();
    for suffix in [" READONLY", " OUTPUT", " OUT"] {
        if let Some(idx) = upper.find(suffix) {
            t = t[..idx].trim().to_string();
            break;
        }
    }
    t
}

pub fn build_param_preamble_with_decls(
    params: &[super::types::RpcParam],
    decls: &[(String, String)],
) -> String {
    let mut out = String::new();
    for (i, p) in params.iter().enumerate() {
        let name = if !p.name.is_empty() {
            p.name.as_str()
        } else if let Some((decl_name, _)) = decls.get(i) {
            decl_name.as_str()
        } else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let type_name = if i < decls.len() {
            normalize_decl_type(&decls[i].1)
        } else {
            normalize_decl_type(&p.type_name)
        };
        if let Some(rows) = &p.tvp_rows {
            out.push_str(&format!("DECLARE {} {};\n", name, type_name));
            for row in rows {
                out.push_str(&format!(
                    "INSERT INTO {} VALUES ({});\n",
                    name,
                    row.join(", ")
                ));
            }
        } else {
            out.push_str(&format!(
                "DECLARE {} {} = {};\n",
                name, type_name, p.value_sql
            ));
        }
    }
    out
}

/// Build an `EXEC <proc> ...` SQL script (plus any TVP preambles) for a named
/// stored procedure RPC call.
pub fn build_stored_proc_exec(name: &str, params: &[RpcParam]) -> String {
    build_stored_proc_exec_with_outputs(name, params).0
}

/// Build an `EXEC <proc> ...` SQL script along with `OUTPUT` parameter bindings.
/// When `outputs` is non-empty, appends a trailing sentinel `SELECT` so the caller
/// can extract each output parameter's final value and emit TDS `RETURNVALUE` (`0xAC`).
pub fn build_stored_proc_exec_with_outputs(
    name: &str,
    params: &[RpcParam],
) -> (String, Vec<OutputParamBinding>) {
    let mut preamble = String::new();
    let mut call_args = Vec::with_capacity(params.len());
    let mut outputs = Vec::new();

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
        let pname = if p.name.is_empty() {
            String::new()
        } else if p.name.starts_with('@') {
            p.name.clone()
        } else {
            format!("@{}", p.name)
        };

        if p.is_output && p.tvp_rows.is_none() {
            let out_var = format!("@__rpc_out_{}", i);
            let decl_type = normalize_decl_type(&p.type_name);
            preamble.push_str(&format!(
                "DECLARE {} {} = {};\n",
                out_var, decl_type, p.value_sql
            ));
            if !has_duplicate_names && !pname.is_empty() {
                call_args.push(format!("{} = {} OUTPUT", pname, out_var));
            } else {
                call_args.push(format!("{} OUTPUT", out_var));
            }
            outputs.push(OutputParamBinding {
                ordinal: i as u16,
                param_name: pname,
                var_name: out_var,
            });
            continue;
        }

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

        if !has_duplicate_names && !pname.is_empty() {
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

    let mut script = if preamble.is_empty() {
        exec_line
    } else {
        format!("{}{}", preamble, exec_line)
    };

    if !outputs.is_empty() {
        let select_cols: Vec<String> = outputs
            .iter()
            .enumerate()
            .map(|(k, b)| format!("{} AS [__rpc_out_{}]", b.var_name, k))
            .collect();
        script.push_str(";\nSELECT ");
        script.push_str(&select_cols.join(", "));
        script.push(';');
    }

    (script, outputs)
}

pub fn append_output_sentinel(sql: &str, outputs: &[OutputParamBinding]) -> String {
    if outputs.is_empty() {
        return sql.to_string();
    }
    let select_cols: Vec<String> = outputs
        .iter()
        .enumerate()
        .map(|(k, b)| format!("{} AS [__rpc_out_{}]", b.var_name, k))
        .collect();
    format!(
        "{};\nSELECT {};",
        sql.trim_end_matches(';'),
        select_cols.join(", ")
    )
}
