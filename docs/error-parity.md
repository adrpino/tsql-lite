# Error Parity and Custom Errors

## Purpose

This document records the current state of error handling in tsql-lite and how it compares to SQL Server.

The short version:

- custom errors are supported
- some common SQL Server error numbers are matched
- total SQL Server error parity is not implemented

## Custom Errors

tsql-lite supports explicit custom errors through [`DbError::Custom`](../crates/tsql_lite_core/src/error.rs).

That variant carries:

- `class`
- `number`
- `message`

This is what backs supported `THROW` and `RAISERROR` flows when the engine raises a user-defined error.

## Current Parity Matrix

| Area | Current behavior | Parity level | Notes |
|---|---|---|---|
| Parse errors | Mapped to SQL Server-style syntax error codes | Partial | `DbError::Parse` currently maps to `102`, but not all parser wording and states match SQL Server exactly. |
| Missing object / schema / trigger / view | Mapped to `208` | Good | Verified 1:1 against SQL Server 2022 (`ExpectErrCode`). |
| Missing column | Mapped to `207` | Good | Both qualified and unqualified column misses verified 1:1 against SQL Server 2022. |
| Database missing | Mapped to `911` | Good | Useful for common connection / catalog checks. |
| Duplicate table / trigger / view / type / schema | Mapped to `2714` | Good | Verified 1:1 against SQL Server 2022. |
| Duplicate column | Mapped to `2705` | Good | Common table definition error. |
| Primary key / unique constraint violation | Mapped to `2627` | Good | Verified 1:1 against SQL Server 2022 (`SqlException.Number` & `ERROR_NUMBER()`). |
| Unique index violation | Mapped to `2601` | Good | Distinguished from `2627` (PK/UQ constraints); verified 1:1 against SQL Server 2022 & over TDS. |
| `CREATE UNIQUE INDEX` duplicate key | Mapped to `1505` | Good | Matches SQL Server index creation duplicate error. |
| Foreign key / `CHECK` constraint violation | Mapped to `547` | Good | Verified 1:1 against SQL Server 2022. |
| `NOT NULL` constraint violation | Mapped to `515` | Good | Verified 1:1 against SQL Server 2022. |
| String / binary truncation (SQL Server 2019+) | Mapped to `2628` | Good | Matches SQL Server 2019+/2022 compat 150/160 truncation error; verified 1:1 against SQL Server 2022 & over TDS. |
| Lock timeout / `NOWAIT` | Mapped to `1222` | Good | Emitted on `SET LOCK_TIMEOUT` expiry and `WITH (NOWAIT)` without dooming active tx when `XACT_ABORT OFF`; verified in 2-session SQL Server 2022 oracle. |
| Deadlock victim | Mapped to `1205` | Good | Matches the SQL Server deadlock victim code. |
| Snapshot isolation update conflict | Mapped to `3960` | Good | Immediately aborts and rolls back snapshot transaction; verified in 2-session SQL Server 2022 oracle. |
| Snapshot isolation not allowed in database | Mapped to `3952` | Good | Verified in live SQL Server 2022 oracle when `ALLOW_SNAPSHOT_ISOLATION` is `OFF`. |
| Read-only database write (incl. snapshots) | Mapped to `3906` | Good | Enforced on `READ_ONLY` databases and `CREATE DATABASE ... AS SNAPSHOT OF`. |
| Database currently in use on `DROP DATABASE` | Mapped to `3702` | Good | Enforced when sibling sessions remain attached without `SINGLE_USER WITH ROLLBACK IMMEDIATE`; verified over TDS. |
| Arithmetic overflow | Mapped to `220` | Good | Common arithmetic overflow parity. |
| Divide by zero | Mapped to `8134` | Good | Verified 1:1 against SQL Server 2022. |
| Conversion failure | Mapped to `245` | Good | Verified 1:1 against SQL Server 2022. |
| Cursor not declared | Mapped to `16916` | Good | Cursor-specific error parity exists for the supported path. |
| Generic semantic errors | Collapsed to `50000` | Partial | Many distinct SQL Server semantic errors are not modeled individually. |
| Generic execution errors | Collapsed to `50000` | Partial | Broad execution failures share one bucket. |
| Generic storage errors | Collapsed to `50001` | Partial | Useful for the engine, but not SQL Server-complete. |
| `THROW` | Implemented and tested | Partial | Works for the supported subset, not a full SQL Server error system. |
| `RAISERROR` | Implemented and tested | Partial | Severity and message handling exist, but not the full formatting surface. |

## What Is Still Missing

- a complete SQL Server error catalog
- exact mapping for every runtime, semantic, and metadata failure
- full message text parity for each error number
- exact severity and state behavior across all error sources
- broader parity for `RAISERROR` formatting and edge cases

## Practical Guidance

When adding new failures, choose the narrowest existing SQL Server-style number that fits the behavior.

If a failure does not have a good SQL Server match yet:

1. add a specific `DbError` variant if the engine needs to distinguish it
2. map it to the closest SQL Server number
3. add a parity test if the case is important

## Related Files

- [`crates/tsql_lite_core/src/error.rs`](../crates/tsql_lite_core/src/error.rs)
- [`crates/tsql_lite_core/tests/sqlserver_differential.rs`](../crates/tsql_lite_core/tests/sqlserver_differential.rs) (`error_number_parity_sql2019`, `table_locking_hints_multi_session_parity`, `snapshot_isolation_mvcc_and_3960_parity`, `snapshot_isolation_disallowed_3952_parity`)
- [`crates/tsql_lite_server/tests/test_db_lifecycle_teardown.rs`](../crates/tsql_lite_server/tests/test_db_lifecycle_teardown.rs)
- [`crates/tsql_lite_core/tests/try_catch_test.rs`](../crates/tsql_lite_core/tests/try_catch_test.rs)
- [`crates/tsql_lite_core/tests/procedural_edge_cases.rs`](../crates/tsql_lite_core/tests/procedural_edge_cases.rs)
- [`crates/tsql_lite_core/tests/sqlserver_comparison.rs`](../crates/tsql_lite_core/tests/sqlserver_comparison.rs)

## Status

Current state is intentionally pragmatic:

- enough parity to support common clients and tests
- not enough parity to claim full SQL Server error compatibility

