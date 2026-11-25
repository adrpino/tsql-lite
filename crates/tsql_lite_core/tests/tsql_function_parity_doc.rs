use tsql_lite_core::executor::{
    is_aggregate_function, is_builtin_scalar_registered, is_system_variable_registered,
};
use std::path::PathBuf;

/// Functions parsed via dedicated syntax/clauses rather than `builtin_registry` or `AggregateFn`.
const SPECIAL_SYNTAX_FUNCTIONS: &[&str] = &[
    "CAST",
    "CONVERT",
    "TRY_CAST",
    "TRY_CONVERT",
    "PARSE",
    "TRY_PARSE",
    "GROUPING",
    "GROUPING_ID",
    "UPDATE",
];

const IMPLEMENTED_WINDOW_FUNCTIONS: &[&str] = &[
    "ROW_NUMBER",
    "RANK",
    "DENSE_RANK",
    "NTILE",
    "LAG",
    "LEAD",
    "FIRST_VALUE",
    "LAST_VALUE",
    "PERCENT_RANK",
    "CUME_DIST",
    "PERCENTILE_CONT",
    "PERCENTILE_DISC",
];

const IMPLEMENTED_TVFS: &[&str] = &["OPENJSON", "STRING_SPLIT", "GENERATE_SERIES", "OPENXML"];

fn is_implemented_in_engine(section: &str, name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    match section {
        "Aggregate Functions" => {
            is_aggregate_function(&upper) || SPECIAL_SYNTAX_FUNCTIONS.contains(&upper.as_str())
        }
        "Window Functions" => IMPLEMENTED_WINDOW_FUNCTIONS.contains(&upper.as_str()),
        "Built-in Table-Valued Functions (TVFs)" => IMPLEMENTED_TVFS.contains(&upper.as_str()),
        "System Variables (`@@`)" => {
            is_system_variable_registered(&upper) || is_builtin_scalar_registered(&upper)
        }
        _ => {
            is_builtin_scalar_registered(&upper)
                || SPECIAL_SYNTAX_FUNCTIONS.contains(&upper.as_str())
        }
    }
}

#[test]
fn tsql_function_parity_doc_matches_live_engine() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let doc_path = manifest_dir
        .join("../..")
        .join("docs/tsql-function-parity.md");
    let content = std::fs::read_to_string(&doc_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", doc_path.display(), e));

    let mut current_section = String::new();
    let mut checked_rows = 0usize;
    let mut mismatches = Vec::new();

    for (line_idx, raw_line) in content.lines().enumerate() {
        let line = raw_line.trim();
        if let Some(heading) = line.strip_prefix("## ") {
            current_section = heading.trim().to_string();
            continue;
        }
        if !line.starts_with("| `") {
            continue;
        }
        let cols: Vec<&str> = line.split('|').map(str::trim).collect();
        if cols.len() < 3 {
            continue;
        }
        let status = cols[2];
        if status != "✅" && status != "❌" {
            continue;
        }
        let name = cols[1].trim_matches('`');
        let doc_says_implemented = status == "✅";
        let engine_has_it = is_implemented_in_engine(&current_section, name);
        checked_rows += 1;

        if doc_says_implemented != engine_has_it {
            mismatches.push(format!(
                "line {} [{}] `{}`: doc says {}, live engine says {}",
                line_idx + 1,
                current_section,
                name,
                status,
                if engine_has_it { "✅" } else { "❌" }
            ));
        }
    }

    assert!(
        checked_rows >= 150,
        "expected to verify at least 150 documented functions, only parsed {}",
        checked_rows
    );
    assert!(
        mismatches.is_empty(),
        "docs/tsql-function-parity.md drifted from live engine:\n{}",
        mismatches.join("\n")
    );
}
