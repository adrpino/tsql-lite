use std::collections::HashSet;
use tsql_lite_core::executor::registered_builtin_scalar_names;
use tsql_lite_core::types::Value;
use tsql_lite_core::{parse_batch, Engine, QueryResult};

fn exec_batch(engine: &Engine, sql: &str) -> Result<Option<QueryResult>, String> {
    let stmts = parse_batch(sql).map_err(|e| format!("parse error: {e}"))?;
    engine
        .execute_batch(stmts)
        .map_err(|e| format!("exec error: {e}"))
}

// ============================================================================
// Matrix 1: Built-in Scalar NULL-Propagation & Registry Completeness Matrix
// ============================================================================

enum NullSpec {
    /// Replacing any argument index in `strict_positions` with `NULL` must evaluate to `NULL`.
    Strict {
        sample_args: &'static [&'static str],
        strict_positions: &'static [usize],
    },
    /// Function has custom/non-strict NULL semantics verified in `verify_special_null_semantics`.
    SpecialNull,
    /// Zero-argument, internal compiler-generated, metadata, security, or subsystem function.
    Exempt,
}

const BUILTIN_NULL_MATRIX: &[(&str, NullSpec)] = &[
    // --- String functions ---
    (
        "LEN",
        NullSpec::Strict {
            sample_args: &["'hello'"],
            strict_positions: &[0],
        },
    ),
    (
        "DATALENGTH",
        NullSpec::Strict {
            sample_args: &["'hello'"],
            strict_positions: &[0],
        },
    ),
    (
        "SUBSTRING",
        NullSpec::Strict {
            sample_args: &["'abcdef'", "2", "3"],
            strict_positions: &[0, 1, 2],
        },
    ),
    (
        "UPPER",
        NullSpec::Strict {
            sample_args: &["'abc'"],
            strict_positions: &[0],
        },
    ),
    (
        "LOWER",
        NullSpec::Strict {
            sample_args: &["'ABC'"],
            strict_positions: &[0],
        },
    ),
    (
        "LTRIM",
        NullSpec::Strict {
            sample_args: &["'  abc'"],
            strict_positions: &[0],
        },
    ),
    (
        "RTRIM",
        NullSpec::Strict {
            sample_args: &["'abc  '"],
            strict_positions: &[0],
        },
    ),
    (
        "TRIM",
        NullSpec::Strict {
            sample_args: &["'  abc  '"],
            strict_positions: &[0],
        },
    ),
    (
        "REPLACE",
        NullSpec::Strict {
            sample_args: &["'abc'", "'b'", "'x'"],
            strict_positions: &[0, 1, 2],
        },
    ),
    (
        "LEFT",
        NullSpec::Strict {
            sample_args: &["'abcdef'", "3"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "RIGHT",
        NullSpec::Strict {
            sample_args: &["'abcdef'", "3"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "CHARINDEX",
        NullSpec::Strict {
            sample_args: &["'b'", "'abc'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "UNISTR",
        NullSpec::Strict {
            sample_args: &["'abc'"],
            strict_positions: &[0],
        },
    ),
    (
        "ASCII",
        NullSpec::Strict {
            sample_args: &["'A'"],
            strict_positions: &[0],
        },
    ),
    (
        "CHAR",
        NullSpec::Strict {
            sample_args: &["65"],
            strict_positions: &[0],
        },
    ),
    (
        "NCHAR",
        NullSpec::Strict {
            sample_args: &["65"],
            strict_positions: &[0],
        },
    ),
    (
        "UNICODE",
        NullSpec::Strict {
            sample_args: &["N'A'"],
            strict_positions: &[0],
        },
    ),
    ("CONCAT", NullSpec::SpecialNull),
    ("CONCAT_WS", NullSpec::SpecialNull),
    (
        "REPLICATE",
        NullSpec::Strict {
            sample_args: &["'a'", "3"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "REVERSE",
        NullSpec::Strict {
            sample_args: &["'abc'"],
            strict_positions: &[0],
        },
    ),
    (
        "STUFF",
        NullSpec::Strict {
            sample_args: &["'abcdef'", "2", "3", "'xy'"],
            strict_positions: &[0, 1, 2],
        },
    ),
    (
        "SPACE",
        NullSpec::Strict {
            sample_args: &["4"],
            strict_positions: &[0],
        },
    ),
    (
        "STR",
        NullSpec::Strict {
            sample_args: &["12.34", "6", "2"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "TRANSLATE",
        NullSpec::Strict {
            sample_args: &["'abc'", "'ab'", "'xy'"],
            strict_positions: &[0, 1, 2],
        },
    ),
    (
        "FORMAT",
        NullSpec::Strict {
            sample_args: &["1234", "'N0'"],
            strict_positions: &[0],
        },
    ),
    (
        "FORMATMESSAGE",
        NullSpec::Strict {
            sample_args: &["'hello %s'", "'world'"],
            strict_positions: &[0],
        },
    ),
    ("ISNUMERIC", NullSpec::SpecialNull),
    (
        "PATINDEX",
        NullSpec::Strict {
            sample_args: &["'%bc%'", "'abcd'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "SOUNDEX",
        NullSpec::Strict {
            sample_args: &["'Smith'"],
            strict_positions: &[0],
        },
    ),
    (
        "DIFFERENCE",
        NullSpec::Strict {
            sample_args: &["'Smith'", "'Smythe'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "STRING_ESCAPE",
        NullSpec::Strict {
            sample_args: &["'a/b'", "'json'"],
            strict_positions: &[0],
        },
    ),
    (
        "PARSENAME",
        NullSpec::Strict {
            sample_args: &["'a.b.c'", "1"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "QUOTENAME",
        NullSpec::Strict {
            sample_args: &["'abc'"],
            strict_positions: &[0],
        },
    ),
    // --- Math & Bitwise functions ---
    (
        "ABS",
        NullSpec::Strict {
            sample_args: &["-5"],
            strict_positions: &[0],
        },
    ),
    (
        "CEILING",
        NullSpec::Strict {
            sample_args: &["1.2"],
            strict_positions: &[0],
        },
    ),
    (
        "FLOOR",
        NullSpec::Strict {
            sample_args: &["1.8"],
            strict_positions: &[0],
        },
    ),
    (
        "ROUND",
        NullSpec::Strict {
            sample_args: &["1.234", "2"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "SQRT",
        NullSpec::Strict {
            sample_args: &["9.0"],
            strict_positions: &[0],
        },
    ),
    (
        "SQUARE",
        NullSpec::Strict {
            sample_args: &["4.0"],
            strict_positions: &[0],
        },
    ),
    (
        "POWER",
        NullSpec::Strict {
            sample_args: &["2.0", "3.0"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "EXP",
        NullSpec::Strict {
            sample_args: &["1.0"],
            strict_positions: &[0],
        },
    ),
    (
        "LOG",
        NullSpec::Strict {
            sample_args: &["10.0"],
            strict_positions: &[0],
        },
    ),
    (
        "LOG10",
        NullSpec::Strict {
            sample_args: &["100.0"],
            strict_positions: &[0],
        },
    ),
    (
        "SIN",
        NullSpec::Strict {
            sample_args: &["0.5"],
            strict_positions: &[0],
        },
    ),
    (
        "COS",
        NullSpec::Strict {
            sample_args: &["0.5"],
            strict_positions: &[0],
        },
    ),
    (
        "TAN",
        NullSpec::Strict {
            sample_args: &["0.5"],
            strict_positions: &[0],
        },
    ),
    (
        "ASIN",
        NullSpec::Strict {
            sample_args: &["0.5"],
            strict_positions: &[0],
        },
    ),
    (
        "ACOS",
        NullSpec::Strict {
            sample_args: &["0.5"],
            strict_positions: &[0],
        },
    ),
    (
        "ATAN",
        NullSpec::Strict {
            sample_args: &["0.5"],
            strict_positions: &[0],
        },
    ),
    (
        "ATN2",
        NullSpec::Strict {
            sample_args: &["1.0", "1.0"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "COT",
        NullSpec::Strict {
            sample_args: &["1.0"],
            strict_positions: &[0],
        },
    ),
    (
        "DEGREES",
        NullSpec::Strict {
            sample_args: &["1.0"],
            strict_positions: &[0],
        },
    ),
    (
        "RADIANS",
        NullSpec::Strict {
            sample_args: &["90.0"],
            strict_positions: &[0],
        },
    ),
    (
        "SIGN",
        NullSpec::Strict {
            sample_args: &["-4"],
            strict_positions: &[0],
        },
    ),
    (
        "LEFT_SHIFT",
        NullSpec::Strict {
            sample_args: &["1", "2"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "RIGHT_SHIFT",
        NullSpec::Strict {
            sample_args: &["8", "1"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "BIT_COUNT",
        NullSpec::Strict {
            sample_args: &["7"],
            strict_positions: &[0],
        },
    ),
    (
        "GET_BIT",
        NullSpec::Strict {
            sample_args: &["7", "1"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "SET_BIT",
        NullSpec::Strict {
            sample_args: &["4", "0"],
            strict_positions: &[0, 1],
        },
    ),
    ("PI", NullSpec::Exempt),
    ("RAND", NullSpec::Exempt),
    ("CHECKSUM", NullSpec::SpecialNull),
    ("BINARY_CHECKSUM", NullSpec::SpecialNull),
    // --- Date/Time functions ---
    (
        "YEAR",
        NullSpec::Strict {
            sample_args: &["'2025-03-15'"],
            strict_positions: &[0],
        },
    ),
    (
        "MONTH",
        NullSpec::Strict {
            sample_args: &["'2025-03-15'"],
            strict_positions: &[0],
        },
    ),
    (
        "DAY",
        NullSpec::Strict {
            sample_args: &["'2025-03-15'"],
            strict_positions: &[0],
        },
    ),
    (
        "DATEPART",
        NullSpec::Strict {
            sample_args: &["DAY", "'2025-03-15'"],
            strict_positions: &[1],
        },
    ),
    (
        "DATENAME",
        NullSpec::Strict {
            sample_args: &["MONTH", "'2025-03-15'"],
            strict_positions: &[1],
        },
    ),
    (
        "DATEDIFF",
        NullSpec::Strict {
            sample_args: &["DAY", "'2025-01-01'", "'2025-01-10'"],
            strict_positions: &[1, 2],
        },
    ),
    (
        "DATEDIFF_BIG",
        NullSpec::Strict {
            sample_args: &["DAY", "'2025-01-01'", "'2025-01-10'"],
            strict_positions: &[1, 2],
        },
    ),
    (
        "DATEADD",
        NullSpec::Strict {
            sample_args: &["DAY", "5", "'2025-01-01'"],
            strict_positions: &[1, 2],
        },
    ),
    (
        "DATETRUNC",
        NullSpec::Strict {
            sample_args: &["MONTH", "'2025-03-15'"],
            strict_positions: &[1],
        },
    ),
    (
        "DATE_BUCKET",
        NullSpec::Strict {
            sample_args: &["DAY", "1", "'2025-03-15'"],
            strict_positions: &[1, 2],
        },
    ),
    (
        "EOMONTH",
        NullSpec::Strict {
            sample_args: &["'2025-03-15'"],
            strict_positions: &[0],
        },
    ),
    (
        "DATEFROMPARTS",
        NullSpec::Strict {
            sample_args: &["2025", "3", "15"],
            strict_positions: &[0, 1, 2],
        },
    ),
    (
        "TIMEFROMPARTS",
        NullSpec::Strict {
            sample_args: &["12", "30", "0", "0", "0"],
            strict_positions: &[0, 1, 2, 3],
        },
    ),
    (
        "SMALLDATETIMEFROMPARTS",
        NullSpec::Strict {
            sample_args: &["2025", "3", "15", "12", "30"],
            strict_positions: &[0, 1, 2, 3, 4],
        },
    ),
    (
        "DATETIMEFROMPARTS",
        NullSpec::Strict {
            sample_args: &["2025", "3", "15", "12", "30", "0", "0"],
            strict_positions: &[0, 1, 2, 3, 4, 5, 6],
        },
    ),
    (
        "DATETIME2FROMPARTS",
        NullSpec::Strict {
            sample_args: &["2025", "3", "15", "12", "30", "0", "0", "0"],
            strict_positions: &[0, 1, 2, 3, 4, 5, 6],
        },
    ),
    (
        "DATETIMEOFFSETFROMPARTS",
        NullSpec::Strict {
            sample_args: &["2025", "3", "15", "12", "30", "0", "0", "0", "0", "0"],
            strict_positions: &[0, 1, 2, 3, 4, 5, 6, 7, 8],
        },
    ),
    (
        "TODATETIMEOFFSET",
        NullSpec::Strict {
            sample_args: &["'2025-03-15 10:00:00'", "'+02:00'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "SWITCHOFFSET",
        NullSpec::Strict {
            sample_args: &["'2025-03-15 10:00:00+00:00'", "'+02:00'"],
            strict_positions: &[0, 1],
        },
    ),
    ("ISDATE", NullSpec::SpecialNull),
    ("GETDATE", NullSpec::Exempt),
    ("CURRENT_TIMESTAMP", NullSpec::Exempt),
    ("CURRENT_DATE", NullSpec::Exempt),
    ("CURRENT_TIME", NullSpec::Exempt),
    ("GETUTCDATE", NullSpec::Exempt),
    ("SYSDATETIME", NullSpec::Exempt),
    ("SYSUTCDATETIME", NullSpec::Exempt),
    ("SYSDATETIMEOFFSET", NullSpec::Exempt),
    ("CURRENT_TIMEZONE", NullSpec::Exempt),
    ("CURRENT_TIMEZONE_ID", NullSpec::Exempt),
    // --- Logic functions ---
    ("ISNULL", NullSpec::SpecialNull),
    ("COALESCE", NullSpec::SpecialNull),
    ("IIF", NullSpec::SpecialNull),
    ("NULLIF", NullSpec::SpecialNull),
    ("CHOOSE", NullSpec::SpecialNull),
    ("GREATEST", NullSpec::SpecialNull),
    ("LEAST", NullSpec::SpecialNull),
    // --- JSON functions ---
    (
        "JSON_VALUE",
        NullSpec::Strict {
            sample_args: &["'{\"a\":1}'", "'$.a'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "JSON_QUERY",
        NullSpec::Strict {
            sample_args: &["'{\"a\":[1]}'", "'$.a'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "JSON_MODIFY",
        NullSpec::Strict {
            sample_args: &["'{\"a\":1}'", "'$.a'", "2"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "JSON_PATH_EXISTS",
        NullSpec::Strict {
            sample_args: &["'{\"a\":1}'", "'$.a'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "JSON_CONTAINS",
        NullSpec::Strict {
            sample_args: &["'[1,2]'", "1"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "ISJSON",
        NullSpec::Strict {
            sample_args: &["'{\"a\":1}'"],
            strict_positions: &[0],
        },
    ),
    (
        "JSON_ARRAY_LENGTH",
        NullSpec::Strict {
            sample_args: &["'[1,2]'"],
            strict_positions: &[0],
        },
    ),
    (
        "JSON_KEYS",
        NullSpec::Strict {
            sample_args: &["'{\"a\":1}'"],
            strict_positions: &[0],
        },
    ),
    ("JSON_OBJECT", NullSpec::Exempt),
    ("JSON_ARRAY", NullSpec::Exempt),
    ("__JSON_OBJECT_NULL", NullSpec::Exempt),
    ("__JSON_OBJECT_ABSENT", NullSpec::Exempt),
    ("__JSON_ARRAY_ABSENT", NullSpec::Exempt),
    ("__JSON_ARRAY_NULL", NullSpec::Exempt),
    // --- Regexp & Fuzzy & Vector ---
    (
        "REGEXP_LIKE",
        NullSpec::Strict {
            sample_args: &["'abc'", "'a.*'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "REGEXP_REPLACE",
        NullSpec::Strict {
            sample_args: &["'abc'", "'b'", "'x'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "REGEXP_SUBSTR",
        NullSpec::Strict {
            sample_args: &["'abc123'", "'[0-9]+'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "REGEXP_INSTR",
        NullSpec::Strict {
            sample_args: &["'abc123'", "'[0-9]+'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "REGEXP_COUNT",
        NullSpec::Strict {
            sample_args: &["'a1b2'", "'[0-9]'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "EDIT_DISTANCE",
        NullSpec::Strict {
            sample_args: &["'cat'", "'cut'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "EDIT_DISTANCE_SIMILARITY",
        NullSpec::Strict {
            sample_args: &["'cat'", "'cut'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "JARO_WINKLER_DISTANCE",
        NullSpec::Strict {
            sample_args: &["'cat'", "'cut'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "JARO_WINKLER_SIMILARITY",
        NullSpec::Strict {
            sample_args: &["'cat'", "'cut'"],
            strict_positions: &[0, 1],
        },
    ),
    ("VECTOR_DISTANCE", NullSpec::Exempt),
    ("VECTOR_NORM", NullSpec::Exempt),
    ("VECTOR_NORMALIZE", NullSpec::Exempt),
    ("VECTORPROPERTY", NullSpec::Exempt),
    // --- Crypto / Compression ---
    (
        "HASHBYTES",
        NullSpec::Strict {
            sample_args: &["'SHA2_256'", "'abc'"],
            strict_positions: &[0, 1],
        },
    ),
    (
        "COMPRESS",
        NullSpec::Strict {
            sample_args: &["'abc'"],
            strict_positions: &[0],
        },
    ),
    (
        "DECOMPRESS",
        NullSpec::Strict {
            sample_args: &["COMPRESS('abc')"],
            strict_positions: &[0],
        },
    ),
    ("CRYPT_GEN_RANDOM", NullSpec::Exempt),
    ("ENCRYPTBYPASSPHRASE", NullSpec::Exempt),
    ("DECRYPTBYPASSPHRASE", NullSpec::Exempt),
    // --- System / Metadata / Security / Internal ---
    ("NEWID", NullSpec::Exempt),
    ("NEWSEQUENTIALID", NullSpec::Exempt),
    ("CONTAINS", NullSpec::Exempt),
    ("FREETEXT", NullSpec::Exempt),
    ("GEOGRAPHY::POINT", NullSpec::Exempt),
    ("GEOMETRY::POINT", NullSpec::Exempt),
    ("GEOGRAPHY::STGEOMFROMTEXT", NullSpec::Exempt),
    ("GEOMETRY::STGEOMFROMTEXT", NullSpec::Exempt),
    ("HIERARCHYID::PARSE", NullSpec::Exempt),
    ("HIERARCHYID::GETROOT", NullSpec::Exempt),
    ("__METHOD_STDISTANCE", NullSpec::Exempt),
    ("__METHOD_STASTEXT", NullSpec::Exempt),
    ("__METHOD_LAT", NullSpec::Exempt),
    ("__METHOD_LONG", NullSpec::Exempt),
    ("__METHOD_STX", NullSpec::Exempt),
    ("__METHOD_STY", NullSpec::Exempt),
    ("__METHOD_STSRID", NullSpec::Exempt),
    ("__METHOD_TOSTRING", NullSpec::Exempt),
    ("__METHOD_GETLEVEL", NullSpec::Exempt),
    ("__METHOD_ISDESCENDANTOF", NullSpec::Exempt),
    ("__METHOD_GETANCESTOR", NullSpec::Exempt),
    ("__METHOD_GETREPARENTEDVALUE", NullSpec::Exempt),
    ("__TRIGGER_UPDATE", NullSpec::Exempt),
    ("__IS_DISTINCT_FROM", NullSpec::Exempt),
    ("__IS_NOT_DISTINCT_FROM", NullSpec::Exempt),
    ("ROWCOUNT_BIG", NullSpec::Exempt),
    ("MIN_ACTIVE_ROWVERSION", NullSpec::Exempt),
    ("CURRENT_TRANSACTION_ID", NullSpec::Exempt),
    ("GETANSINULL", NullSpec::Exempt),
    ("OBJECT_ID", NullSpec::Exempt),
    ("OBJECT_NAME", NullSpec::Exempt),
    ("OBJECT_DEFINITION", NullSpec::Exempt),
    ("OBJECT_SCHEMA_NAME", NullSpec::Exempt),
    ("SCHEMA_NAME", NullSpec::Exempt),
    ("SCHEMA_ID", NullSpec::Exempt),
    ("TYPE_NAME", NullSpec::Exempt),
    ("TYPE_ID", NullSpec::Exempt),
    ("TYPEPROPERTY", NullSpec::Exempt),
    ("COLUMNPROPERTY", NullSpec::Exempt),
    ("OBJECTPROPERTY", NullSpec::Exempt),
    ("OBJECTPROPERTYEX", NullSpec::Exempt),
    ("INDEXPROPERTY", NullSpec::Exempt),
    ("INDEX_COL", NullSpec::Exempt),
    ("INDEXKEY_PROPERTY", NullSpec::Exempt),
    ("COL_NAME", NullSpec::Exempt),
    ("COL_LENGTH", NullSpec::Exempt),
    ("DATABASEPROPERTYEX", NullSpec::Exempt),
    ("FILEGROUP_ID", NullSpec::Exempt),
    ("FILEGROUP_NAME", NullSpec::Exempt),
    ("FILEGROUPPROPERTY", NullSpec::Exempt),
    ("FILE_ID", NullSpec::Exempt),
    ("FILE_NAME", NullSpec::Exempt),
    ("FILEPROPERTY", NullSpec::Exempt),
    ("IDENT_CURRENT", NullSpec::Exempt),
    ("IDENT_SEED", NullSpec::Exempt),
    ("IDENT_INCR", NullSpec::Exempt),
    ("SCOPE_IDENTITY", NullSpec::Exempt),
    ("XACT_STATE", NullSpec::Exempt),
    ("@@IDENTITY", NullSpec::Exempt),
    ("@@ROWCOUNT", NullSpec::Exempt),
    ("@@TRANCOUNT", NullSpec::Exempt),
    ("@@ERROR", NullSpec::Exempt),
    ("@@SPID", NullSpec::Exempt),
    ("@@MAX_CONNECTIONS", NullSpec::Exempt),
    ("@@REMSERVER", NullSpec::Exempt),
    ("@@FETCH_STATUS", NullSpec::Exempt),
    ("@@NESTLEVEL", NullSpec::Exempt),
    ("@@OPTIONS", NullSpec::Exempt),
    ("@@MAX_PRECISION", NullSpec::Exempt),
    ("@@DATEFIRST", NullSpec::Exempt),
    ("@@MICROSOFTVERSION", NullSpec::Exempt),
    ("ERROR_MESSAGE", NullSpec::Exempt),
    ("ERROR_NUMBER", NullSpec::Exempt),
    ("ERROR_SEVERITY", NullSpec::Exempt),
    ("ERROR_STATE", NullSpec::Exempt),
    ("ERROR_PROCEDURE", NullSpec::Exempt),
    ("ERROR_LINE", NullSpec::Exempt),
    ("DB_NAME", NullSpec::Exempt),
    ("DB_ID", NullSpec::Exempt),
    ("SUSER_SNAME", NullSpec::Exempt),
    ("SUSER_ID", NullSpec::Exempt),
    ("SUSER_NAME", NullSpec::Exempt),
    ("SUSER_SID", NullSpec::Exempt),
    ("USER_NAME", NullSpec::Exempt),
    ("USER_ID", NullSpec::Exempt),
    ("USER_SID", NullSpec::Exempt),
    ("DATABASE_PRINCIPAL_ID", NullSpec::Exempt),
    ("DATABASE_PRINCIPAL_NAME", NullSpec::Exempt),
    ("APP_NAME", NullSpec::Exempt),
    ("HOST_NAME", NullSpec::Exempt),
    ("HOST_ID", NullSpec::Exempt),
    ("SYSTEM_USER", NullSpec::Exempt),
    ("SESSION_USER", NullSpec::Exempt),
    ("CURRENT_USER", NullSpec::Exempt),
    ("ORIGINAL_LOGIN", NullSpec::Exempt),
    ("SQL_VARIANT_PROPERTY", NullSpec::Exempt),
    ("XML_SCHEMA_NAMESPACE", NullSpec::Exempt),
    ("ORIGINAL_DB_NAME", NullSpec::Exempt),
    ("PROCID", NullSpec::Exempt),
    ("SERVERPROPERTY", NullSpec::Exempt),
    ("FULLTEXTSERVICEPROPERTY", NullSpec::Exempt),
    ("CONNECTIONPROPERTY", NullSpec::Exempt),
    ("COLLATIONPROPERTY", NullSpec::Exempt),
    ("SESSIONPROPERTY", NullSpec::Exempt),
    ("CONTEXT_INFO", NullSpec::Exempt),
    ("SESSION_CONTEXT", NullSpec::Exempt),
    ("IS_SRVROLEMEMBER", NullSpec::Exempt),
    ("IS_MEMBER", NullSpec::Exempt),
    ("PERMISSIONS", NullSpec::Exempt),
    ("HAS_DBACCESS", NullSpec::Exempt),
    ("HAS_PERMS_BY_NAME", NullSpec::Exempt),
    ("COLUMNS_UPDATED", NullSpec::Exempt),
    ("TRIGGER_NESTLEVEL", NullSpec::Exempt),
    ("EVENTDATA", NullSpec::Exempt),
    ("APPLOCK_MODE", NullSpec::Exempt),
    ("APPLOCK_TEST", NullSpec::Exempt),
];

#[test]
fn matrix_01_builtin_scalar_null_propagation_and_registry_completeness() {
    // 1. Registry completeness guard: every registered scalar builtin must be classified.
    let classified: HashSet<&str> = BUILTIN_NULL_MATRIX.iter().map(|(n, _)| *n).collect();
    let registered = registered_builtin_scalar_names();
    let missing: Vec<&str> = registered
        .iter()
        .copied()
        .filter(|n| !classified.contains(n))
        .collect();
    assert!(
        missing.is_empty(),
        "Unclassified built-in scalar functions (add them to BUILTIN_NULL_MATRIX): {missing:?}"
    );

    // 2. Strict NULL propagation across every classified argument position.
    let engine = Engine::new();
    let mut failures = Vec::new();
    for (name, spec) in BUILTIN_NULL_MATRIX {
        let NullSpec::Strict {
            sample_args,
            strict_positions,
        } = spec
        else {
            continue;
        };
        for &pos in *strict_positions {
            let mut args: Vec<&str> = sample_args.to_vec();
            args[pos] = "NULL";
            let sql = format!("SELECT {}({});", name, args.join(", "));
            match exec_batch(&engine, &sql) {
                Ok(Some(qr)) => {
                    if !qr.rows[0][0].is_null() {
                        failures.push(format!("{sql} -> {:?} (expected NULL)", qr.rows[0][0]));
                    }
                }
                Ok(None) => failures.push(format!("{sql} -> no result set")),
                Err(e) => failures.push(format!("{sql} -> {e}")),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Built-in scalar NULL propagation failures:\n{}",
        failures.join("\n")
    );

    // 3. Special NULL-handling semantics.
    let qr = exec_batch(
        &engine,
        "SELECT
             ISNULL(NULL, 'fallback') AS c_isnull,
             COALESCE(NULL, NULL, 42) AS c_coalesce,
             CONCAT('a', NULL, 'b') AS c_concat,
             CONCAT_WS(',', 'a', NULL, 'b') AS c_concat_ws,
             CONCAT_WS(NULL, 'a', 'b') AS c_concat_ws_null_sep,
             NULLIF(5, 5) AS c_nullif_eq,
             NULLIF(5, NULL) AS c_nullif_null,
             IIF(NULL = 1, 'yes', 'no') AS c_iif_unknown,
             CHOOSE(NULL, 'a', 'b') AS c_choose_null,
             GREATEST(1, NULL, 5) AS c_greatest,
             LEAST(1, NULL, 5) AS c_least,
             ISNUMERIC(NULL) AS c_isnumeric,
             ISDATE(NULL) AS c_isdate;",
    )
    .unwrap()
    .unwrap();
    let row = &qr.rows[0];
    assert_eq!(row[0].to_string_value(), "fallback");
    assert_eq!(row[1].to_integer_i64(), Some(42));
    assert_eq!(row[2].to_string_value(), "ab");
    assert_eq!(row[3].to_string_value(), "a,b");
    assert_eq!(row[4].to_string_value(), "");
    assert!(row[5].is_null());
    assert_eq!(row[6].to_integer_i64(), Some(5));
    assert_eq!(row[7].to_string_value(), "no");
    assert!(row[8].is_null());
    assert_eq!(row[9].to_integer_i64(), Some(5));
    assert_eq!(row[10].to_integer_i64(), Some(1));
    assert_eq!(row[11].to_integer_i64(), Some(0));
    assert_eq!(row[12].to_integer_i64(), Some(0));
}

// ============================================================================
// Matrix 2: DML x Type-Coercion Cross-Path Consistency Matrix
// ============================================================================

#[test]
fn matrix_02_dml_type_coercion_cross_path_consistency() {
    struct Case {
        label: &'static str,
        src_type: &'static str,
        dst_type: &'static str,
        literal: &'static str,
    }

    let cases = [
        Case {
            label: "dec_13_4_to_18_6",
            src_type: "DECIMAL(13, 4)",
            dst_type: "DECIMAL(18, 6)",
            literal: "5000.00",
        },
        Case {
            label: "dec_18_6_to_18_2",
            src_type: "DECIMAL(18, 6)",
            dst_type: "DECIMAL(18, 2)",
            literal: "123.456789",
        },
        Case {
            label: "int_to_dec_10_4",
            src_type: "INT",
            dst_type: "DECIMAL(10, 4)",
            literal: "42",
        },
        Case {
            label: "varchar_to_int",
            src_type: "VARCHAR(20)",
            dst_type: "INT",
            literal: "'987'",
        },
        Case {
            label: "varchar_to_dec_12_3",
            src_type: "VARCHAR(20)",
            dst_type: "DECIMAL(12, 3)",
            literal: "'12.5'",
        },
        Case {
            label: "nvarchar_to_char_8",
            src_type: "NVARCHAR(20)",
            dst_type: "CHAR(8)",
            literal: "N'abc'",
        },
        Case {
            label: "datetime2_to_date",
            src_type: "DATETIME2(0)",
            dst_type: "DATE",
            literal: "'2025-07-19 14:30:00'",
        },
        Case {
            label: "varchar_to_xml",
            src_type: "VARCHAR(100)",
            dst_type: "XML",
            literal: "'<Box  id = ''1'' />'",
        },
    ];

    for case in cases {
        let engine = Engine::new();
        let setup_tables = format!(
            "CREATE TABLE dbo.Src (Id INT, Val {src});
             INSERT INTO dbo.Src VALUES (1, {lit});
             CREATE TABLE dbo.T_Values (Id INT, Val {dst});
             CREATE TABLE dbo.T_Select (Id INT, Val {dst});
             CREATE TABLE dbo.T_Exec   (Id INT, Val {dst});
             CREATE TABLE dbo.T_Update (Id INT, Val {dst});
             CREATE TABLE dbo.T_MergeI (Id INT, Val {dst});
             CREATE TABLE dbo.T_MergeU (Id INT, Val {dst});",
            src = case.src_type,
            dst = case.dst_type,
            lit = case.literal,
        );
        exec_batch(&engine, &setup_tables)
            .unwrap_or_else(|e| panic!("case '{}' setup tables failed: {e}", case.label));
        exec_batch(
            &engine,
            "CREATE PROCEDURE dbo.usp_EmitSrc AS BEGIN SELECT 1 AS Id, Val FROM dbo.Src; END;",
        )
        .unwrap_or_else(|e| panic!("case '{}' setup proc failed: {e}", case.label));
        let dml = format!(
            "INSERT INTO dbo.T_Values VALUES (1, {lit});
             INSERT INTO dbo.T_Select SELECT Id, Val FROM dbo.Src;
             INSERT INTO dbo.T_Exec EXEC dbo.usp_EmitSrc;

             INSERT INTO dbo.T_Update VALUES (1, NULL);
             UPDATE t SET t.Val = s.Val FROM dbo.T_Update t JOIN dbo.Src s ON s.Id = t.Id;

             MERGE INTO dbo.T_MergeI AS tgt
             USING dbo.Src AS src ON tgt.Id = src.Id
             WHEN NOT MATCHED THEN INSERT (Id, Val) VALUES (src.Id, src.Val);

             INSERT INTO dbo.T_MergeU VALUES (1, NULL);
             MERGE INTO dbo.T_MergeU AS tgt
             USING dbo.Src AS src ON tgt.Id = src.Id
             WHEN MATCHED THEN UPDATE SET tgt.Val = src.Val;",
            lit = case.literal,
        );
        exec_batch(&engine, &dml)
            .unwrap_or_else(|e| panic!("case '{}' dml failed: {e}", case.label));

        let read_val = |tbl: &str| -> Value {
            let qr = engine
                .query(&format!("SELECT Val FROM dbo.{tbl} WHERE Id = 1;"))
                .unwrap();
            qr.rows[0][0].clone()
        };

        let v_values = read_val("T_Values");
        let v_select = read_val("T_Select");
        let v_exec = read_val("T_Exec");
        let v_update = read_val("T_Update");
        let v_merge_i = read_val("T_MergeI");
        let v_merge_u = read_val("T_MergeU");

        assert_eq!(
            v_select, v_values,
            "case '{}': INSERT..SELECT ({v_select:?}) != INSERT..VALUES ({v_values:?})",
            case.label
        );
        assert_eq!(
            v_exec, v_values,
            "case '{}': INSERT..EXEC ({v_exec:?}) != INSERT..VALUES ({v_values:?})",
            case.label
        );
        assert_eq!(
            v_update, v_values,
            "case '{}': UPDATE..FROM ({v_update:?}) != INSERT..VALUES ({v_values:?})",
            case.label
        );
        assert_eq!(
            v_merge_i, v_values,
            "case '{}': MERGE INSERT ({v_merge_i:?}) != INSERT..VALUES ({v_values:?})",
            case.label
        );
        assert_eq!(
            v_merge_u, v_values,
            "case '{}': MERGE UPDATE ({v_merge_u:?}) != INSERT..VALUES ({v_values:?})",
            case.label
        );
    }
}

// ============================================================================
// Matrix 3: Constraint Cardinality Matrix (1-col, 2-col, 3-col)
// ============================================================================

#[test]
fn matrix_03_constraint_cardinality_1_2_3_columns() {
    for arity in 1usize..=3 {
        let cols_ddl: Vec<String> = (1..=arity).map(|i| format!("K{i} INT NOT NULL")).collect();
        let key_list: Vec<String> = (1..=arity).map(|i| format!("K{i}")).collect();
        let key_csv = key_list.join(", ");

        // Build rows that repeat every individual column when arity >= 2, while every tuple is distinct.
        let mut distinct_tuples: Vec<Vec<i32>> = vec![vec![1; arity]];
        for pos in 0..arity {
            let mut row = vec![1; arity];
            row[pos] = 2;
            distinct_tuples.push(row);
        }
        let format_row = |id: usize, vals: &[i32]| -> String {
            let mut parts = vec![id.to_string()];
            parts.extend(vals.iter().map(ToString::to_string));
            format!("({})", parts.join(", "))
        };
        let insert_values: Vec<String> = distinct_tuples
            .iter()
            .enumerate()
            .map(|(i, t)| format_row(i + 1, t))
            .collect();
        let dup_row = format_row(99, &distinct_tuples[0]);

        // --- 3a. PRIMARY KEY ---
        {
            let engine = Engine::new();
            let ddl = format!(
                "CREATE TABLE dbo.T_Pk (RowId INT NOT NULL, {cols}, CONSTRAINT PK_T PRIMARY KEY ({key_csv}));",
                cols = cols_ddl.join(", "),
            );
            exec_batch(&engine, &ddl).unwrap();
            exec_batch(
                &engine,
                &format!(
                    "INSERT INTO dbo.T_Pk (RowId, {key_csv}) VALUES {};",
                    insert_values.join(", ")
                ),
            )
            .unwrap_or_else(|e| panic!("PK arity={arity} distinct insert failed: {e}"));

            assert!(
                exec_batch(
                    &engine,
                    &format!("INSERT INTO dbo.T_Pk (RowId, {key_csv}) VALUES {dup_row};")
                )
                .is_err(),
                "PK arity={arity} duplicate INSERT must fail"
            );
            if arity >= 2 {
                assert!(
                    exec_batch(&engine, "UPDATE dbo.T_Pk SET K1 = 1 WHERE RowId = 2;").is_err(),
                    "PK arity={arity} duplicate UPDATE must fail"
                );
            }
        }

        // --- 3b. UNIQUE constraint (nullable cols when arity >= 2 to test NULL tuple rules too) ---
        {
            let engine = Engine::new();
            let nullable_cols: Vec<String> =
                (1..=arity).map(|i| format!("K{i} INT NULL")).collect();
            let ddl = format!(
                "CREATE TABLE dbo.T_Uq (RowId INT PRIMARY KEY, {cols}, CONSTRAINT UQ_T UNIQUE ({key_csv}));",
                cols = nullable_cols.join(", "),
            );
            exec_batch(&engine, &ddl).unwrap();
            exec_batch(
                &engine,
                &format!(
                    "INSERT INTO dbo.T_Uq (RowId, {key_csv}) VALUES {};",
                    insert_values.join(", ")
                ),
            )
            .unwrap_or_else(|e| panic!("UNIQUE arity={arity} distinct insert failed: {e}"));

            assert!(
                exec_batch(
                    &engine,
                    &format!("INSERT INTO dbo.T_Uq (RowId, {key_csv}) VALUES {dup_row};")
                )
                .is_err(),
                "UNIQUE arity={arity} duplicate INSERT must fail"
            );
            if arity >= 2 {
                assert!(
                    exec_batch(&engine, "UPDATE dbo.T_Uq SET K1 = 1 WHERE RowId = 2;").is_err(),
                    "UNIQUE arity={arity} duplicate UPDATE must fail"
                );
                // SQL Server UNIQUE with NULL: (10, NULL, ...) and (20, NULL, ...) are distinct;
                // a second (10, NULL, ...) collides.
                let nulls = vec!["NULL"; arity - 1].join(", ");
                exec_batch(
                    &engine,
                    &format!(
                        "INSERT INTO dbo.T_Uq (RowId, {key_csv}) VALUES (50, 10, {nulls}), (51, 20, {nulls});"
                    ),
                )
                .unwrap_or_else(|e| panic!("UNIQUE arity={arity} distinct NULL tuples failed: {e}"));
                assert!(
                    exec_batch(
                        &engine,
                        &format!(
                            "INSERT INTO dbo.T_Uq (RowId, {key_csv}) VALUES (52, 10, {nulls});"
                        )
                    )
                    .is_err(),
                    "UNIQUE arity={arity} duplicate (10, NULL...) tuple must fail"
                );

                // DROP CONSTRAINT lifts composite UNIQUE enforcement.
                exec_batch(&engine, "ALTER TABLE dbo.T_Uq DROP CONSTRAINT UQ_T;").unwrap();
                exec_batch(
                    &engine,
                    &format!("INSERT INTO dbo.T_Uq (RowId, {key_csv}) VALUES {dup_row};"),
                )
                .unwrap_or_else(|e| panic!("UNIQUE arity={arity} post-drop insert failed: {e}"));
            }
        }

        // --- 3c. UNIQUE INDEX ---
        {
            let engine = Engine::new();
            let ddl = format!(
                "CREATE TABLE dbo.T_Ux (RowId INT PRIMARY KEY, {cols});
                 CREATE UNIQUE INDEX UX_T ON dbo.T_Ux ({key_csv});",
                cols = cols_ddl.join(", "),
            );
            exec_batch(&engine, &ddl).unwrap();
            exec_batch(
                &engine,
                &format!(
                    "INSERT INTO dbo.T_Ux (RowId, {key_csv}) VALUES {};",
                    insert_values.join(", ")
                ),
            )
            .unwrap_or_else(|e| panic!("UNIQUE INDEX arity={arity} distinct insert failed: {e}"));
            assert!(
                exec_batch(
                    &engine,
                    &format!("INSERT INTO dbo.T_Ux (RowId, {key_csv}) VALUES {dup_row};")
                )
                .is_err(),
                "UNIQUE INDEX arity={arity} duplicate INSERT must fail"
            );
        }

        // --- 3d. FOREIGN KEY ---
        {
            let engine = Engine::new();
            let parent_ddl = format!(
                "CREATE TABLE dbo.T_Parent ({cols}, CONSTRAINT PK_Parent PRIMARY KEY ({key_csv}));",
                cols = cols_ddl.join(", "),
            );
            let child_cols: Vec<String> = (1..=arity).map(|i| format!("K{i} INT NULL")).collect();
            let child_ddl = format!(
                "CREATE TABLE dbo.T_Child (
                     RowId INT PRIMARY KEY,
                     {cols},
                     CONSTRAINT FK_Child FOREIGN KEY ({key_csv}) REFERENCES dbo.T_Parent ({key_csv})
                 );",
                cols = child_cols.join(", "),
            );
            exec_batch(&engine, &parent_ddl).unwrap();
            exec_batch(&engine, &child_ddl).unwrap();

            let p_vals = distinct_tuples[0]
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            exec_batch(
                &engine,
                &format!("INSERT INTO dbo.T_Parent ({key_csv}) VALUES ({p_vals});"),
            )
            .unwrap();
            // Valid parent match succeeds.
            exec_batch(
                &engine,
                &format!("INSERT INTO dbo.T_Child (RowId, {key_csv}) VALUES (1, {p_vals});"),
            )
            .unwrap();
            // Orphan child fails.
            let orphan = vec!["999"; arity].join(", ");
            assert!(
                exec_batch(
                    &engine,
                    &format!("INSERT INTO dbo.T_Child (RowId, {key_csv}) VALUES (2, {orphan});")
                )
                .is_err(),
                "FK arity={arity} orphan INSERT must fail"
            );
            // SQL Server MATCH SIMPLE: any NULL in composite FK bypasses check.
            if arity >= 2 {
                let partial_null = format!("999, {}", vec!["NULL"; arity - 1].join(", "));
                exec_batch(
                    &engine,
                    &format!(
                        "INSERT INTO dbo.T_Child (RowId, {key_csv}) VALUES (3, {partial_null});"
                    ),
                )
                .unwrap_or_else(|e| panic!("FK arity={arity} partial-NULL insert failed: {e}"));
            }
            // DROP CONSTRAINT lifts FK check.
            exec_batch(&engine, "ALTER TABLE dbo.T_Child DROP CONSTRAINT FK_Child;").unwrap();
            exec_batch(
                &engine,
                &format!("INSERT INTO dbo.T_Child (RowId, {key_csv}) VALUES (4, {orphan});"),
            )
            .unwrap();
        }
    }
}

// ============================================================================
// Matrix 4: Serialization & Canonicalization Matrix
// ============================================================================

#[test]
fn matrix_04_serialization_and_canonicalization() {
    let engine = Engine::new();

    let xml_cases = [
        ("<Empty />", "<Empty/>"),
        ("<Empty   />", "<Empty/>"),
        (
            "<Item  id = '7'  flag = \"true\"  />",
            "<Item id=\"7\" flag=\"true\"/>",
        ),
        (
            "<Root><Child>text</Child></Root >",
            "<Root><Child>text</Child></Root>",
        ),
        (
            "<Tag msg='say \"hi\"' />",
            "<Tag msg=\"say &quot;hi&quot;\"/>",
        ),
        ("<A /><B />", "<A/><B/>"),
        ("<!-- note --><X />", "<!-- note --><X/>"),
        (
            "<Raw><![CDATA[<not a tag />]]></Raw>",
            "<Raw><![CDATA[<not a tag />]]></Raw>",
        ),
    ];

    for (raw, expected) in xml_cases {
        let escaped = raw.replace('\'', "''");
        let sql = format!("SELECT CAST(CAST('{escaped}' AS XML) AS NVARCHAR(MAX));");
        let qr = exec_batch(&engine, &sql).unwrap().unwrap();
        assert_eq!(
            qr.rows[0][0],
            Value::NVarChar(expected.to_string()),
            "XML canonicalization mismatch for input: {raw}"
        );
    }

    // VARBINARY -> XML round-trip canonicalization
    let qr = exec_batch(
        &engine,
        "SELECT CAST(CAST(CAST('<Bin />' AS VARBINARY(MAX)) AS XML) AS NVARCHAR(MAX));",
    )
    .unwrap()
    .unwrap();
    assert_eq!(qr.rows[0][0], Value::NVarChar("<Bin/>".into()));

    // DECIMAL formatting preserves declared scale
    let qr = exec_batch(
        &engine,
        "SELECT
             CAST(CAST(5000 AS DECIMAL(18, 6)) AS VARCHAR(30)) AS d6,
             CAST(CAST(12.3456 AS DECIMAL(10, 2)) AS VARCHAR(30)) AS d2,
             CAST(CAST('2025-07-19' AS DATE) AS VARCHAR(30)) AS dt;",
    )
    .unwrap()
    .unwrap();
    assert_eq!(qr.rows[0][0], Value::VarChar("5000.000000".into()));
    assert_eq!(qr.rows[0][1], Value::VarChar("12.35".into()));
    assert_eq!(qr.rows[0][2], Value::VarChar("2025-07-19".into()));
}

// ============================================================================
// Matrix 5: Routine Composition Matrix (Scalar, Inline TVF, Inline-CTE TVF, MSTVF)
// ============================================================================

#[test]
fn matrix_05_routine_composition_default_apply_and_multi_rs() {
    let engine = Engine::new();

    exec_batch(
        &engine,
        "CREATE TABLE dbo.Groups (GroupId INT, Tag VARCHAR(10));
         CREATE TABLE dbo.Entries (GroupId INT, Amount INT);
         INSERT INTO dbo.Groups VALUES (1, 'A'), (2, 'B'), (3, 'C');
         INSERT INTO dbo.Entries VALUES (1, 10), (1, 20), (2, 5);

         CREATE FUNCTION dbo.Fn_Scale(@v INT, @mult INT = 2)
         RETURNS INT
         AS BEGIN RETURN @v * @mult; END;

         CREATE FUNCTION dbo.Fn_Inline(@gid INT, @minAmt INT = 1)
         RETURNS TABLE
         AS RETURN (
             SELECT GroupId, Amount FROM dbo.Entries
             WHERE GroupId = @gid AND Amount >= @minAmt
         );

         CREATE FUNCTION dbo.Fn_InlineCte(@gid INT, @minAmt INT = 1)
         RETURNS TABLE
         AS RETURN (
             WITH Scoped AS (
                 SELECT GroupId, Amount FROM dbo.Entries
                 WHERE GroupId = @gid AND Amount >= @minAmt
             )
             SELECT GroupId, SUM(Amount) AS SumAmt
             FROM Scoped
             GROUP BY GroupId
         );

         CREATE FUNCTION dbo.Fn_MultiStmt(@gid INT, @prefix VARCHAR(10) = 'G')
         RETURNS @out TABLE (GroupId INT, Code VARCHAR(20))
         AS
         BEGIN
             IF EXISTS (SELECT 1 FROM dbo.Entries WHERE GroupId = @gid)
                 INSERT INTO @out VALUES (@gid, @prefix + '-' + CAST(@gid AS VARCHAR(10)));
             RETURN;
         END;

         CREATE PROCEDURE dbo.usp_MatrixBundle
         AS
         BEGIN
             SET NOCOUNT ON;
             -- RS1: scalar UDF with DEFAULT arg
             SELECT g.GroupId, dbo.Fn_Scale(g.GroupId, DEFAULT) AS Scaled
             FROM dbo.Groups g
             ORDER BY g.GroupId;

             -- RS2: CROSS APPLY onto Inline-CTE TVF with DEFAULT arg
             SELECT g.GroupId, c.SumAmt
             FROM dbo.Groups g
             CROSS APPLY (
                 SELECT SumAmt FROM dbo.Fn_InlineCte(g.GroupId, DEFAULT)
             ) c
             ORDER BY g.GroupId;

             -- RS3: OUTER APPLY onto Multi-Statement TVF with DEFAULT arg (group 3 has NULL Code)
             SELECT g.GroupId, m.Code
             FROM dbo.Groups g
             OUTER APPLY (
                 SELECT Code FROM dbo.Fn_MultiStmt(g.GroupId, DEFAULT)
             ) m
             ORDER BY g.GroupId;
         END;",
    )
    .unwrap();

    // Direct calls with DEFAULT vs explicit args for all 4 routine kinds.
    let qr = engine
        .query("SELECT dbo.Fn_Scale(7, DEFAULT), dbo.Fn_Scale(7, 3);")
        .unwrap();
    assert_eq!(qr.rows[0][0].to_integer_i64(), Some(14));
    assert_eq!(qr.rows[0][1].to_integer_i64(), Some(21));

    let qr = engine
        .query("SELECT Amount FROM dbo.Fn_Inline(1, DEFAULT) ORDER BY Amount;")
        .unwrap();
    assert_eq!(qr.rows, vec![vec![Value::Int(10)], vec![Value::Int(20)]]);

    let qr = engine
        .query("SELECT SumAmt FROM dbo.Fn_InlineCte(1, DEFAULT);")
        .unwrap();
    assert_eq!(qr.rows[0][0].to_integer_i64(), Some(30));

    let qr = engine
        .query("SELECT Code FROM dbo.Fn_MultiStmt(2, DEFAULT);")
        .unwrap();
    assert_eq!(qr.rows, vec![vec![Value::VarChar("G-2".into())]]);

    // Multi-RS stored procedure with CROSS APPLY + OUTER APPLY onto CTE & Multi-Stmt TVFs.
    let stmts = parse_batch("EXEC dbo.usp_MatrixBundle;").unwrap();
    let results = engine.execute_batch_all_results(stmts).unwrap();
    assert_eq!(
        results.len(),
        3,
        "expected exactly 3 result sets (no TVF sink leakage), got {}",
        results.len()
    );

    assert_eq!(results[0].rows.len(), 3);
    assert_eq!(results[0].rows[2][1].to_integer_i64(), Some(6));

    assert_eq!(results[1].rows.len(), 2);
    assert_eq!(results[1].rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(results[1].rows[0][1].to_integer_i64(), Some(30));
    assert_eq!(results[1].rows[1][0].to_integer_i64(), Some(2));
    assert_eq!(results[1].rows[1][1].to_integer_i64(), Some(5));

    assert_eq!(results[2].rows.len(), 3);
    assert_eq!(results[2].rows[0][1], Value::VarChar("G-1".into()));
    assert_eq!(results[2].rows[1][1], Value::VarChar("G-2".into()));
    assert!(results[2].rows[2][1].is_null());
}
