use crate::executor::result::QueryResult;
use crate::types::Value;

pub(crate) fn execute_xp_msver(filters: &[String]) -> QueryResult {
    let all_rows: &[(i32, &str, Option<i32>, Option<&str>)] = &[
        (1, "ProductName", None, Some("Microsoft SQL Server")),
        (2, "ProductVersion", Some(1_048_576), Some("16.0.1000.6")),
        (3, "Language", Some(1033), Some("English (United States)")),
        (4, "Platform", None, Some("NT x64")),
        (5, "Comments", None, Some("SQL")),
        (6, "CompanyName", None, Some("Microsoft Corporation")),
        (
            7,
            "FileDescription",
            None,
            Some("SQL Server Windows NT - 64 Bit"),
        ),
        (8, "FileVersion", None, Some("2022.0160.1000.06")),
        (9, "InternalName", None, Some("SQLSERVR")),
        (
            10,
            "LegalCopyright",
            None,
            Some("Microsoft Corp. All rights reserved."),
        ),
        (
            11,
            "LegalTrademarks",
            None,
            Some("Microsoft SQL Server is a registered trademark of Microsoft Corporation."),
        ),
        (12, "OriginalFilename", None, Some("SQLSERVR.EXE")),
        (13, "PrivateBuild", None, None),
        (14, "SpecialBuild", Some(65_536_000), None),
        (15, "WindowsVersion", Some(2600), Some("10.0 (20348)")),
        (16, "ProcessorCount", Some(4), Some("4")),
        (17, "ProcessorActiveMask", None, Some("000000000000000f")),
        (18, "ProcessorType", Some(8664), Some("PROCESSOR_AMD_X8664")),
        (
            19,
            "PhysicalMemory",
            Some(16384),
            Some("16384 (17179869184)"),
        ),
        (20, "Product ID", None, None),
    ];

    let rows: Vec<Vec<Value>> = all_rows
        .iter()
        .filter(|(_, name, _, _)| {
            filters.is_empty()
                || filters
                    .iter()
                    .any(|req| req.trim().eq_ignore_ascii_case(name))
        })
        .map(|(idx, name, iv, cv)| {
            vec![
                Value::Int(*idx),
                Value::NVarChar((*name).to_string()),
                iv.map(Value::Int).unwrap_or(Value::Null),
                cv.map(|s| Value::NVarChar(s.to_string()))
                    .unwrap_or(Value::Null),
            ]
        })
        .collect();

    QueryResult {
        columns: vec![
            "Index".to_string(),
            "Name".to_string(),
            "Internal_Value".to_string(),
            "Character_Value".to_string(),
        ],
        column_types: vec![
            crate::types::DataType::Int,
            crate::types::DataType::NVarChar { max_len: 128 },
            crate::types::DataType::Int,
            crate::types::DataType::NVarChar { max_len: 512 },
        ],
        column_nullabilities: vec![false, false, true, true],
        rows,
        ..Default::default()
    }
}

pub(crate) fn procedure_return_value(value: Option<Value>) -> Value {
    match value {
        Some(v) => match v {
            Value::Null => Value::Int(0),
            Value::Int(_) => v,
            other => Value::Int(other.to_integer_i64().unwrap_or(0) as i32),
        },
        None => Value::Int(0),
    }
}
