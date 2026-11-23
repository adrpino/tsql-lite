# T-SQL Function & System Surface Parity Reference

This document is the **code-verified** function-level parity reference for `iridium_core`.

Every `✅` / `❌` row in the function tables below is verified automatically by
`crates/iridium_core/tests/tsql_function_parity_doc.rs` against the live engine
registries (`builtin_registry`, `AggregateFn`, `WindowFunc`, built-in TVFs, and
`system_vars`). If a function is added or removed without updating this file,
`cargo test -p iridium_core --test tsql_function_parity_doc` fails.

For the high-level subsystem scoreboard (DDL, DML, Transactions, Metadata, TDS,
Security), see [`docs/compatibility-matrix.md`](compatibility-matrix.md).

---

## Aggregate Functions

| Function | Status | Notes |
|---|---|---|
| `COUNT` | ✅ | Supports `COUNT(*)`, `COUNT(expr)`, `COUNT(DISTINCT expr)`, and `OVER (...)` |
| `COUNT_BIG` | ✅ | Returns `BIGINT`; supports `*`, `DISTINCT`, and `OVER (...)` |
| `SUM` | ✅ | Supports `ALL`, `DISTINCT`, and `OVER (...)` |
| `AVG` | ✅ | Supports `ALL`, `DISTINCT`, and `OVER (...)` |
| `MIN` | ✅ | Supports `GROUP BY` and `OVER (...)` |
| `MAX` | ✅ | Supports `GROUP BY` and `OVER (...)` |
| `STDEV` | ✅ | Sample standard deviation (`FLOAT`); supports `DISTINCT` and `OVER (...)` |
| `STDEVP` | ✅ | Population standard deviation (`FLOAT`); supports `DISTINCT` and `OVER (...)` |
| `VAR` | ✅ | Sample variance (`FLOAT`); supports `DISTINCT` and `OVER (...)` |
| `VARP` | ✅ | Population variance (`FLOAT`); supports `DISTINCT` and `OVER (...)` |
| `CHECKSUM_AGG` | ✅ | XOR checksum over `INT` group values; supports `ALL`/`DISTINCT` |
| `STRING_AGG` | ✅ | Supports `WITHIN GROUP (ORDER BY ...)` |
| `JSON_OBJECTAGG` | ✅ | Supports `NULL ON NULL` / `ABSENT ON NULL` |
| `JSON_ARRAYAGG` | ✅ | Supports `ORDER BY` and `ABSENT ON NULL` / `NULL ON NULL` |
| `GROUPING` | ✅ | Supported with `ROLLUP`, `CUBE`, and `GROUPING SETS` |
| `GROUPING_ID` | ✅ | Supported with `ROLLUP`, `CUBE`, and `GROUPING SETS` |
| `APPROX_COUNT_DISTINCT` | ❌ | Not yet implemented |
| `APPROX_PERCENTILE_CONT` | ❌ | Not yet implemented |
| `APPROX_PERCENTILE_DISC` | ❌ | Not yet implemented |

## Window Functions

| Function | Status | Notes |
|---|---|---|
| `ROW_NUMBER` | ✅ | Full `PARTITION BY` / `ORDER BY` |
| `RANK` | ✅ | Full peer ranking |
| `DENSE_RANK` | ✅ | Full dense peer ranking |
| `NTILE` | ✅ | Bucket distribution |
| `LAG` | ✅ | Supports offset, default, and `IGNORE NULLS` / `RESPECT NULLS` |
| `LEAD` | ✅ | Supports offset, default, and `IGNORE NULLS` / `RESPECT NULLS` |
| `FIRST_VALUE` | ✅ | Supports window frame and `IGNORE NULLS` / `RESPECT NULLS` |
| `LAST_VALUE` | ✅ | Supports window frame and `IGNORE NULLS` / `RESPECT NULLS` |
| `PERCENT_RANK` | ✅ | Relative rank within partition |
| `CUME_DIST` | ✅ | Peer-aware cumulative distribution |
| `PERCENTILE_CONT` | ✅ | Continuous interpolated percentile |
| `PERCENTILE_DISC` | ✅ | Discrete percentile |

## String Functions

| Function | Status | Notes |
|---|---|---|
| `ASCII` | ✅ | |
| `CHAR` | ✅ | |
| `CHARINDEX` | ✅ | |
| `CONCAT` | ✅ | |
| `CONCAT_WS` | ✅ | |
| `DATALENGTH` | ✅ | |
| `DIFFERENCE` | ✅ | |
| `FORMAT` | ✅ | |
| `FORMATMESSAGE` | ✅ | |
| `LEFT` | ✅ | |
| `LEN` | ✅ | |
| `LOWER` | ✅ | |
| `LTRIM` | ✅ | Supports optional trim characters |
| `NCHAR` | ✅ | |
| `PATINDEX` | ✅ | |
| `QUOTENAME` | ✅ | |
| `REPLACE` | ✅ | |
| `REPLICATE` | ✅ | |
| `REVERSE` | ✅ | |
| `RIGHT` | ✅ | |
| `RTRIM` | ✅ | Supports optional trim characters |
| `SOUNDEX` | ✅ | |
| `SPACE` | ✅ | |
| `STR` | ✅ | |
| `STRING_ESCAPE` | ✅ | |
| `STUFF` | ✅ | |
| `SUBSTRING` | ✅ | |
| `TRANSLATE` | ✅ | |
| `TRIM` | ✅ | Supports `LEADING`/`TRAILING`/`BOTH` and character list |
| `UNICODE` | ✅ | |
| `UNISTR` | ✅ | SQL Server 2025 Unicode escape decoding |
| `UPPER` | ✅ | |

## Date and Time Functions

| Function | Status | Notes |
|---|---|---|
| `CURRENT_TIMESTAMP` | ✅ | |
| `CURRENT_DATE` | ✅ | |
| `CURRENT_TIME` | ✅ | |
| `CURRENT_TIMEZONE` | ✅ | |
| `CURRENT_TIMEZONE_ID` | ✅ | |
| `DATEADD` | ✅ | |
| `DATEDIFF` | ✅ | |
| `DATEDIFF_BIG` | ✅ | |
| `DATEFROMPARTS` | ✅ | |
| `DATENAME` | ✅ | |
| `DATEPART` | ✅ | |
| `DATETRUNC` | ✅ | SQL Server 2022 truncation across all dateparts |
| `DATE_BUCKET` | ✅ | SQL Server 2022 bucketing with optional origin |
| `DATETIMEFROMPARTS` | ✅ | |
| `DATETIME2FROMPARTS` | ✅ | |
| `DATETIMEOFFSETFROMPARTS` | ✅ | |
| `DAY` | ✅ | |
| `EOMONTH` | ✅ | |
| `GETDATE` | ✅ | |
| `GETUTCDATE` | ✅ | |
| `ISDATE` | ✅ | |
| `MONTH` | ✅ | |
| `SMALLDATETIMEFROMPARTS` | ✅ | |
| `SWITCHOFFSET` | ✅ | |
| `SYSDATETIME` | ✅ | |
| `SYSDATETIMEOFFSET` | ✅ | |
| `SYSUTCDATETIME` | ✅ | |
| `TIMEFROMPARTS` | ✅ | |
| `TODATETIMEOFFSET` | ✅ | |
| `YEAR` | ✅ | |

## Mathematical & Bitwise Functions

| Function | Status | Notes |
|---|---|---|
| `ABS` | ✅ | |
| `ACOS` | ✅ | |
| `ASIN` | ✅ | |
| `ATAN` | ✅ | |
| `ATN2` | ✅ | |
| `BIT_COUNT` | ✅ | SQL Server 2022 |
| `CEILING` | ✅ | |
| `CHECKSUM` | ✅ | |
| `COS` | ✅ | |
| `COT` | ✅ | |
| `DEGREES` | ✅ | |
| `EXP` | ✅ | |
| `FLOOR` | ✅ | |
| `GET_BIT` | ✅ | SQL Server 2022 |
| `LEFT_SHIFT` | ✅ | SQL Server 2022 |
| `LOG` | ✅ | |
| `LOG10` | ✅ | |
| `PI` | ✅ | |
| `POWER` | ✅ | |
| `RADIANS` | ✅ | |
| `RAND` | ✅ | |
| `RIGHT_SHIFT` | ✅ | SQL Server 2022 |
| `ROUND` | ✅ | |
| `SET_BIT` | ✅ | SQL Server 2022 |
| `SIGN` | ✅ | |
| `SIN` | ✅ | |
| `SQRT` | ✅ | |
| `SQUARE` | ✅ | |
| `TAN` | ✅ | |
| `VECTOR_DISTANCE` | ✅ | SQL Server 2025 |
| `VECTOR_NORM` | ❌ | SQL Server 2025 |
| `VECTOR_NORMALIZE` | ❌ | SQL Server 2025 |
| `VECTORPROPERTY` | ❌ | SQL Server 2025 |

## Logic, Conversion & JSON Functions

| Function | Status | Notes |
|---|---|---|
| `CAST` | ✅ | Special syntax |
| `CONVERT` | ✅ | Special syntax with style codes |
| `TRY_CAST` | ✅ | Special syntax |
| `TRY_CONVERT` | ✅ | Special syntax |
| `PARSE` | ✅ | Special syntax |
| `TRY_PARSE` | ✅ | Special syntax |
| `CHOOSE` | ✅ | |
| `COALESCE` | ✅ | |
| `GREATEST` | ✅ | SQL Server 2022 |
| `IIF` | ✅ | |
| `ISNULL` | ✅ | |
| `ISNUMERIC` | ✅ | |
| `LEAST` | ✅ | SQL Server 2022 |
| `NULLIF` | ✅ | |
| `ISJSON` | ✅ | Supports 1-arg and 2-arg (`VALUE`/`ARRAY`/`OBJECT`/`SCALAR`) |
| `JSON_ARRAY` | ✅ | Supports `NULL ON NULL` / `ABSENT ON NULL` |
| `JSON_ARRAY_LENGTH` | ✅ | Extension helper |
| `JSON_KEYS` | ✅ | Extension helper |
| `JSON_MODIFY` | ✅ | |
| `JSON_OBJECT` | ✅ | Supports `NULL ON NULL` / `ABSENT ON NULL` |
| `JSON_PATH_EXISTS` | ✅ | SQL Server 2022 |
| `JSON_QUERY` | ✅ | |
| `JSON_VALUE` | ✅ | |
| `JSON_CONTAINS` | ❌ | SQL Server 2025 |

## Metadata, System, Security, Crypto & Trigger Functions

| Function | Status | Notes |
|---|---|---|
| `APP_NAME` | ✅ | |
| `BINARY_CHECKSUM` | ✅ | Case-sensitive 32-bit rolling checksum |
| `COLLATIONPROPERTY` | ✅ | |
| `COL_LENGTH` | ✅ | |
| `COL_NAME` | ✅ | |
| `COLUMNPROPERTY` | ✅ | |
| `COLUMNS_UPDATED` | ✅ | Bitmask of updated columns in DML triggers |
| `COMPRESS` | ✅ | RFC 1952 GZIP (pure Rust) |
| `CONNECTIONPROPERTY` | ✅ | |
| `CONTEXT_INFO` | ✅ | |
| `CURRENT_TRANSACTION_ID` | ✅ | |
| `CURRENT_USER` | ✅ | |
| `DATABASE_PRINCIPAL_ID` | ✅ | |
| `DATABASE_PRINCIPAL_NAME` | ✅ | |
| `DATABASEPROPERTYEX` | ✅ | |
| `DB_ID` | ✅ | |
| `DB_NAME` | ✅ | |
| `DECOMPRESS` | ✅ | RFC 1952 GZIP (pure Rust) |
| `ERROR_LINE` | ✅ | |
| `ERROR_MESSAGE` | ✅ | |
| `ERROR_NUMBER` | ✅ | |
| `ERROR_PROCEDURE` | ✅ | |
| `ERROR_SEVERITY` | ✅ | |
| `ERROR_STATE` | ✅ | |
| `FULLTEXTSERVICEPROPERTY` | ✅ | |
| `GETANSINULL` | ✅ | |
| `HAS_DBACCESS` | ✅ | |
| `HAS_PERMS_BY_NAME` | ✅ | |
| `HASHBYTES` | ✅ | MD5, SHA1, SHA2_256, SHA2_512 |
| `HOST_ID` | ✅ | |
| `HOST_NAME` | ✅ | |
| `IDENT_CURRENT` | ✅ | |
| `IDENT_INCR` | ✅ | |
| `IDENT_SEED` | ✅ | |
| `INDEXKEY_PROPERTY` | ✅ | |
| `INDEXPROPERTY` | ✅ | |
| `INDEX_COL` | ✅ | |
| `IS_MEMBER` | ✅ | |
| `IS_SRVROLEMEMBER` | ✅ | |
| `MIN_ACTIVE_ROWVERSION` | ✅ | 8-byte active rowversion watermark |
| `NEWID` | ✅ | |
| `NEWSEQUENTIALID` | ✅ | Monotonic sequential UUID |
| `OBJECT_DEFINITION` | ✅ | |
| `OBJECT_ID` | ✅ | |
| `OBJECT_NAME` | ✅ | |
| `OBJECT_SCHEMA_NAME` | ✅ | |
| `OBJECTPROPERTY` | ✅ | |
| `OBJECTPROPERTYEX` | ✅ | |
| `ORIGINAL_DB_NAME` | ✅ | |
| `ORIGINAL_LOGIN` | ✅ | |
| `PARSENAME` | ✅ | |
| `PERMISSIONS` | ✅ | |
| `PROCID` | ✅ | |
| `ROWCOUNT_BIG` | ✅ | `BIGINT` rowcount of last statement |
| `SCHEMA_ID` | ✅ | |
| `SCHEMA_NAME` | ✅ | |
| `SCOPE_IDENTITY` | ✅ | |
| `SERVERPROPERTY` | ✅ | |
| `SESSION_CONTEXT` | ✅ | |
| `SESSION_USER` | ✅ | |
| `SESSIONPROPERTY` | ✅ | |
| `SQL_VARIANT_PROPERTY` | ✅ | |
| `SUSER_ID` | ✅ | |
| `SUSER_NAME` | ✅ | |
| `SUSER_SID` | ✅ | |
| `SUSER_SNAME` | ✅ | |
| `SYSTEM_USER` | ✅ | |
| `TRIGGER_NESTLEVEL` | ✅ | Trigger nesting depth (global or by `object_id`) |
| `TYPE_ID` | ✅ | Built-ins, table types, and scalar alias UDTs |
| `TYPE_NAME` | ✅ | Built-ins, table types, and scalar alias UDTs |
| `TYPEPROPERTY` | ✅ | Built-ins and scalar alias UDTs |
| `UPDATE` | ✅ | Trigger column-updated predicate `UPDATE(col)` |
| `USER_ID` | ✅ | |
| `USER_NAME` | ✅ | |
| `USER_SID` | ✅ | |
| `XACT_STATE` | ✅ | Returns `1`, `0`, or `-1` (doomed under `XACT_ABORT ON`) |
| `XML_SCHEMA_NAMESPACE` | ✅ | Shim (returns `NULL`) |
| `APPLOCK_MODE` | ✅ | Current lock mode held by owner (`NoLock`, `Shared`, `Exclusive`, etc.) |
| `APPLOCK_TEST` | ✅ | Returns `1`/`0` for application lock grantability across lock owners |
| `CERT_ID` | ❌ | Server crypto/certificate surface |
| `CRYPT_GEN_RANDOM` | ❌ | Server crypto surface |
| `ENCRYPTBYPASSPHRASE` | ❌ | Server crypto surface |
| `DECRYPTBYPASSPHRASE` | ❌ | Server crypto surface |
| `EVENTDATA` | ✅ | Standard `<EVENT_INSTANCE>` XML payload inside DDL triggers (`NULL` outside) |
| `FILE_ID` | ❌ | Physical OS file metadata |
| `FILE_NAME` | ❌ | Physical OS file metadata |
| `FILEGROUP_ID` | ❌ | |
| `FILEGROUP_NAME` | ❌ | |
| `FILEPROPERTY` | ❌ | |
| `FILEGROUPPROPERTY` | ❌ | |

## Built-in Table-Valued Functions (TVFs)

| Function | Status | Notes |
|---|---|---|
| `OPENJSON` | ✅ | Default schema and explicit `WITH (...)` schema |
| `STRING_SPLIT` | ✅ | Supports 2-arg and 3-arg `enable_ordinal` |
| `GENERATE_SERIES` | ✅ | SQL Server 2022 (`start, stop [, step]`) |
| `OPENXML` | ✅ | Legacy XML shredding with `sp_xml_preparedocument`/`sp_xml_removedocument` |
| `OPENROWSET` | ❌ | External data source TVF |
| `OPENQUERY` | ❌ | Linked-server TVF |
| `CHANGETABLE` | ❌ | Change Tracking TVF |
| `CONTAINSTABLE` | ❌ | Full-Text Search TVF |
| `FREETEXTTABLE` | ❌ | Full-Text Search TVF |

## System Variables (`@@`)

| Variable | Status | Notes |
|---|---|---|
| `@@DATEFIRST` | ✅ | |
| `@@ERROR` | ✅ | |
| `@@FETCH_STATUS` | ✅ | |
| `@@IDENTITY` | ✅ | |
| `@@LANGUAGE` | ✅ | |
| `@@MAX_CONNECTIONS` | ✅ | |
| `@@MAX_PRECISION` | ✅ | |
| `@@MICROSOFTVERSION` | ✅ | |
| `@@NESTLEVEL` | ✅ | |
| `@@OPTIONS` | ✅ | |
| `@@PROCID` | ✅ | |
| `@@REMSERVER` | ✅ | |
| `@@ROWCOUNT` | ✅ | |
| `@@SERVERNAME` | ✅ | |
| `@@SERVICENAME` | ✅ | |
| `@@SPID` | ✅ | |
| `@@TEXTSIZE` | ✅ | |
| `@@TRANCOUNT` | ✅ | |
| `@@VERSION` | ✅ | Reports SQL Server 2022 emulation string |

## SQL Server 2025 RegEx & Fuzzy Matching Functions

| Function | Status | Notes |
|---|---|---|
| `REGEXP_LIKE` | ✅ | |
| `REGEXP_REPLACE` | ✅ | |
| `REGEXP_SUBSTR` | ✅ | |
| `REGEXP_INSTR` | ✅ | |
| `REGEXP_COUNT` | ✅ | |
| `EDIT_DISTANCE` | ✅ | |
| `EDIT_DISTANCE_SIMILARITY` | ✅ | |
| `JARO_WINKLER_DISTANCE` | ✅ | |
| `JARO_WINKLER_SIMILARITY` | ✅ | |

---

## System Stored Procedures (`sp_*` / `xp_*`)

Implemented in `crates/iridium_core/src/executor/script/procedural/system_procedures/`:

- **Dynamic execution & metadata inspection:** `sp_executesql`, `sp_describe_first_result_set`, `sp_describe_undeclared_parameters`, `sp_help`, `sp_helptext`, `sp_columns`, `sp_tables`, `sp_helpindex`, `sp_helpconstraint`, `sp_rename`, `sp_refreshsqlmodule`, `sp_refreshview`, `sp_refresh_parameter_encryption`
- **Extended properties & table options (persisted in catalog):** `sp_addextendedproperty`, `sp_updateextendedproperty`, `sp_dropextendedproperty`, `sp_tableoption`
- **Session, XML & application locks:** `sp_set_session_context`, `sp_getapplock`, `sp_releaseapplock`, `sp_xml_preparedocument`, `sp_xml_removedocument`
- **Database, security & server probes:** `sp_who`, `sp_databases`, `sp_helpdb`, `sp_server_info`, `sp_monitor`, `sp_helpuser`, `sp_helprole`, `sp_helprolemember`, `sp_helpsrvrole`, `sp_helpsrvrolemember`, `sp_helpfile`, `sp_helpfilegroup`, `sp_msgetversion`, `sp_MSIsContainedAGSession`, `xp_instance_regread`, `xp_msver`, `xp_qv`
- **Cursor RPCs (`iridium_server` TDS layer):** `sp_prepexec`, `sp_cursoropen`, `sp_cursorfetch`, `sp_cursorclose`
- **Out of scope for embedded engine:** SQL Server Agent (`sp_add_job`, etc.), Replication (`sp_addpublication`, etc.), Database Mail, Service Broker, and Availability Groups (`sp_add_ag_*`).
