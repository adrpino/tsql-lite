# tsql-lite

An embedded **SQL Server-compatible T-SQL engine** and **TDS 7.4 wire server** written in pure Rust — designed for millisecond local development, CI integration testing, and embedded/WASM workflows without Docker or a full SQL Server installation.

- **163 / 163** live SQL Server 2022 differential parity checks passing (`./scripts/test-compat.sh`)
- **19 / 19** live `.NET` `Microsoft.Data.SqlClient` + Dapper + EF Core 8 differential checks passing (`./scripts/test-drivers.sh`)
- **Zero-copy CoW database cloning & isolated snapshots** (`fork_isolated()`, `CREATE DATABASE ... AS CLONE OF` / `AS SNAPSHOT OF`)
- **Persistent (`redb` + WAL) or pure in-memory (`--memory`) execution**

---

## Highlights

- **T-SQL Language Surface:** `CREATE/ALTER/DROP` tables, views, stored procedures, scalar/inline/multi-statement TVFs, `AFTER` & `INSTEAD OF` DML triggers, database-level DDL triggers (`EVENTDATA()`), `MERGE` (with error `8672`), `PIVOT`/`UNPIVOT`, recursive CTEs (`MAXRECURSION`), `GROUPING SETS`/`ROLLUP`/`CUBE`, window functions (including SQL Server 2022 named `WINDOW` clauses), `FOR JSON`, `FOR XML` + `.value()`/`.exist()`/`.query()`/`.nodes()`/`.modify()`, `OPENJSON`, `OPENXML`, `GENERATE_SERIES`, temporal tables (`FOR SYSTEM_TIME`), and native scalar/table UDTs.
- **Strict SQL Server Parity:** Exact error-code parity (`103`, `257`, `515`, `529`, `547`, `1054`, `1205`, `1222`, `1505`, `2601`, `2627`, `2628`, `3702`, `3729`, `3906`, `3952`, `3960`, `4512`, `5074`, `8672`, `9400`/`9436`), verbatim `sys.sql_modules` / `OBJECT_DEFINITION()` / `sp_helptext`, `WITH SCHEMABINDING` creation + drop/alter guards, 128-char/116-char identifier limits, and strict XML well-formedness & implicit-conversion enforcement.
- **Concurrency, Isolation & Lifecycle:** Row-locking table hints (`UPDLOCK`, `ROWLOCK`, `READPAST`, `HOLDLOCK`, `NOLOCK`, `NOWAIT`, `READCOMMITTEDLOCK`, `XLOCK`, `TABLOCK`/`TABLOCKX`), `READ_COMMITTED_SNAPSHOT` (RCSI) & `SNAPSHOT` MVCC isolation with 3-way row merge & error `3960` conflict abort, deadlock detection (`1205`), `ALTER DATABASE ... SET SINGLE_USER WITH ROLLBACK IMMEDIATE` session eviction, and `sp_MSforeachtable`.
- **Enterprise Governance:** Multi-database isolation, Row-Level Security (`CREATE SECURITY POLICY` `FILTER` & `BLOCK` predicates), Dynamic Data Masking (`MASKED WITH` + `UNMASK`), and `GRANT`/`DENY`/`REVOKE` + `EXECUTE AS`.
- **TDS 7.4 Wire Server (`tsql-lite-server`):** TLS 1.3/1.2, MARS (`SMP 0x53`), Transaction Manager (`0x0E`), RPC (`sp_executesql`, `sp_prepare`/`sp_prepexec`/`sp_execute`, cursors, named stored procedures with `OUTPUT` & TVPs), `SqlBulkCopy`, `ATTENTION` cancellation, and SSMS Object Explorer probes.

---

## Quick Start

### Embedded Rust (`tsql-lite-core`)

```rust
use tsql_lite_core::Engine;

let engine = Engine::new();
engine.exec("CREATE TABLE dbo.Users (Id INT IDENTITY(1,1) PRIMARY KEY, Name NVARCHAR(100) NOT NULL);")?;
engine.exec("INSERT INTO dbo.Users (Name) VALUES (N'Ada'), (N'Linus');")?;

let result = engine.query("SELECT Id, Name FROM dbo.Users ORDER BY Id;")?;
assert_eq!(result.rows.len(), 2);

// Instant copy-on-write fork for parallel isolated tests:
let test_db = engine.fork_isolated();
```

### Standalone TDS Server (`tsql-lite-server`)

```bash
# Persistent storage on disk (default)
cargo run --release -p tsql-lite-server --bin tsql-lite-server -- --tls-gen

# Ephemeral in-memory mode (for local dev & CI)
cargo run --release -p tsql-lite-server --bin tsql-lite-server -- --memory --tls-gen
```

Connect with `Microsoft.Data.SqlClient`, Dapper, EF Core 8, `tiberius`, `sqlcmd`, or SSMS on `localhost:1433`.

### WebAssembly (`tsql-lite-wasm`)

```bash
wasm-pack build crates/tsql_lite_wasm --target web --out-dir crates/tsql_lite_wasm/pkg
```

### Differential Verification Against Live SQL Server 2022

```bash
./scripts/test-compat.sh    # 163/163 T-SQL & concurrency checks vs live SQL Server 2022
./scripts/test-drivers.sh   # 19/19 .NET SqlClient + Dapper + EF Core 8 checks
```

---

## Workspace Crates

| Crate | Path | Purpose |
|---|---|---|
| `tsql-lite-core` | `crates/tsql_lite_core` | T-SQL lexer, parser, binder, executor, MVCC/locking, `redb` persistence, catalog |
| `tsql-lite-server` | `crates/tsql_lite_server` | TDS 7.4 wire server (`tsql-lite-server`) |
| `tsql-lite-wasm` | `crates/tsql_lite_wasm` | WebAssembly bindings |

---

## Documentation

- [Compatibility Matrix](docs/compatibility-matrix.md)
- [T-SQL Function Parity Reference](docs/tsql-function-parity.md)
- [SQL Server Error Parity](docs/error-parity.md)
- [Docker Guide](docs/docker.md)
- [FAQ](docs/faq.md)

---

## Origin & Attribution

`tsql-lite` was originally forked from [celsowm/iridium-sql](https://github.com/celsowm/iridium-sql) (MIT License) at `v0.1.13` and independently evolved with multi-database isolation, row-locking hints, RCSI/SNAPSHOT MVCC, enterprise security (RLS, DDM, DDL triggers), verbatim `sys.sql_modules`, strict SQL Server error/XML/identifier/SCHEMABINDING parity, and live SQL Server 2022 + `.NET` differential verification.

Full commit history and original copyright notices are preserved in this repository and in [`LICENSE`](LICENSE).

## Disclaimer

`tsql-lite` is an independent clean-room Rust implementation and does not contain any proprietary Microsoft SQL Server code. Microsoft and SQL Server are trademarks of Microsoft Corporation and are referenced solely for compatibility and interoperability description.
