mod sqlserver_diff;

use iridium_core::Engine;
use sqlserver_diff::harness::{
    connect_sqlserver, detect_sqlserver_major_version, run_differential_suite, run_iridium_batch,
    DiffStep, LiveSqlServerConfig,
};
use sqlserver_diff::suites::all_suites;

/// Runs every differential suite offline against `iridium_core` alone (no Docker required),
/// guaranteeing all scenarios stay valid and regression-free during normal `cargo test`.
#[test]
fn differential_suites_smoke_on_iridium() {
    let mut total_steps = 0usize;
    for suite in all_suites() {
        let engine = Engine::new();
        for (i, batch) in suite.setup_batches.iter().enumerate() {
            run_iridium_batch(&engine, batch).unwrap_or_else(|e| {
                panic!(
                    "suite '{}' setup[{}] failed on Iridium: {}",
                    suite.name, i, e
                )
            });
        }
        for step in suite.steps {
            total_steps += 1;
            match step {
                DiffStep::ExecOk { label, sql } => {
                    run_iridium_batch(&engine, sql).unwrap_or_else(|e| {
                        panic!("suite '{}' [{}] ExecOk failed: {}", suite.name, label, e)
                    });
                }
                DiffStep::ExpectErr { label, sql } => {
                    assert!(
                        run_iridium_batch(&engine, sql).is_err(),
                        "suite '{}' [{}] ExpectErr unexpectedly succeeded",
                        suite.name,
                        label
                    );
                }
                DiffStep::Query { label, sql, .. } => {
                    run_iridium_batch(&engine, sql).unwrap_or_else(|e| {
                        panic!("suite '{}' [{}] Query failed: {}", suite.name, label, e)
                    });
                }
            }
        }
    }
    assert!(total_steps >= 40, "expected at least 40 differential steps");
}

/// Live differential oracle comparing `iridium_core` against a real SQL Server instance.
///
/// Configure via env vars (`SQLSERVER_HOST`, `SQLSERVER_PORT`, `SQLSERVER_USER`,
/// `SQLSERVER_PASSWORD`) or launch via `./scripts/test-compat.sh` / `./scripts/test-compat.ps1`.
#[tokio::test]
#[ignore = "requires live SQL Server (run via ./scripts/test-compat.sh or --ignored)"]
async fn differential_parity_against_live_sqlserver() {
    let cfg = LiveSqlServerConfig::from_env();
    let mut probe = connect_sqlserver(&cfg)
        .await
        .expect("failed to connect to live SQL Server oracle");
    let major = detect_sqlserver_major_version(&mut probe)
        .await
        .expect("failed to query SQL Server ProductMajorVersion");
    drop(probe);

    println!(
        "\n=== Iridium SQL Differential Oracle vs Live SQL Server (major={}) ===",
        major
    );

    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let mut total_skipped = 0usize;
    let mut all_failures = Vec::new();

    for suite in all_suites() {
        let report = run_differential_suite(&cfg, major, &suite).await;
        if let Some(reason) = report.skipped_reason {
            total_skipped += 1;
            println!("  [SKIP] {:<42} ({})", report.suite_name, reason);
            continue;
        }
        total_passed += report.passed;
        total_failed += report.failures.len();
        if report.failures.is_empty() {
            println!(
                "  [PASS] {:<42} ({} checks) [{}]",
                report.suite_name, report.passed, report.category
            );
        } else {
            println!(
                "  [FAIL] {:<42} ({} passed, {} failed) [{}]",
                report.suite_name,
                report.passed,
                report.failures.len(),
                report.category
            );
            for f in &report.failures {
                println!("         - {}", f);
                all_failures.push(format!("{}: {}", report.suite_name, f));
            }
        }
    }

    println!(
        "\nSummary: {} passed, {} failed, {} suites skipped (SQL Server major={})",
        total_passed, total_failed, total_skipped, major
    );
    assert!(
        all_failures.is_empty(),
        "Differential parity mismatches found ({}):\n{}",
        all_failures.len(),
        all_failures.join("\n")
    );
}
