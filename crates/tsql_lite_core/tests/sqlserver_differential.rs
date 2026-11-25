mod sqlserver_diff;

use tsql_lite_core::Engine;
use sqlserver_diff::harness::{
    connect_sqlserver, detect_sqlserver_major_version, run_differential_suite, run_tsql_lite_batch,
    run_tsql_lite_batch_detailed, run_tsql_lite_session_batch_detailed,
    run_multi_session_differential_suite, DiffStep, LiveSqlServerConfig, MultiSessionStep,
    SessionSlot,
};
use sqlserver_diff::suites::{all_multi_session_suites, all_suites};

/// Runs every differential suite offline against `tsql_lite_core` alone (no Docker required),
/// guaranteeing all scenarios stay valid and regression-free during normal `cargo test`.
#[test]
fn differential_suites_smoke_on_tsql_lite() {
    let mut total_steps = 0usize;
    for suite in all_suites() {
        let engine = Engine::new();
        for (i, batch) in suite.setup_batches.iter().enumerate() {
            run_tsql_lite_batch(&engine, batch).unwrap_or_else(|e| {
                panic!(
                    "suite '{}' setup[{}] failed on tsql-lite: {}",
                    suite.name, i, e
                )
            });
        }
        for step in suite.steps {
            total_steps += 1;
            match step {
                DiffStep::ExecOk { label, sql } => {
                    run_tsql_lite_batch(&engine, sql).unwrap_or_else(|e| {
                        panic!("suite '{}' [{}] ExecOk failed: {}", suite.name, label, e)
                    });
                }
                DiffStep::ExpectErr { label, sql } => {
                    assert!(
                        run_tsql_lite_batch(&engine, sql).is_err(),
                        "suite '{}' [{}] ExpectErr unexpectedly succeeded",
                        suite.name,
                        label
                    );
                }
                DiffStep::ExpectErrCode {
                    label,
                    sql,
                    expected_code,
                } => {
                    let err = run_tsql_lite_batch_detailed(&engine, sql).expect_err(&format!(
                        "suite '{}' [{}] ExpectErrCode unexpectedly succeeded",
                        suite.name, label
                    ));
                    if let Some(exp) = expected_code {
                        assert_eq!(
                            err.code,
                            Some(*exp),
                            "suite '{}' [{}] error code mismatch: got {:?}",
                            suite.name,
                            label,
                            err
                        );
                    }
                }
                DiffStep::Query { label, sql, .. } => {
                    run_tsql_lite_batch(&engine, sql).unwrap_or_else(|e| {
                        panic!("suite '{}' [{}] Query failed: {}", suite.name, label, e)
                    });
                }
            }
        }
    }
    assert!(total_steps >= 40, "expected at least 40 differential steps");
}

#[test]
fn multi_session_suites_smoke_on_tsql_lite() {
    for suite in all_multi_session_suites() {
        let engine = Engine::new();
        let sid_a = engine.create_session();
        let sid_b = engine.create_session();
        let db_name = format!("TsqlLiteSmokeMS_{}", suite.name);
        run_tsql_lite_session_batch_detailed(&engine, sid_a, &format!("CREATE DATABASE [{}];", db_name))
            .unwrap_or_else(|e| panic!("suite '{}' CREATE DATABASE failed: {}", suite.name, e));
        for (i, raw_opt) in suite.db_options_batches.iter().enumerate() {
            let opt_sql = raw_opt.replace("{db}", &db_name);
            run_tsql_lite_session_batch_detailed(&engine, sid_a, &opt_sql).unwrap_or_else(|e| {
                panic!("suite '{}' db_options[{}] failed: {}", suite.name, i, e)
            });
        }
        let use_sql = format!("USE [{}];", db_name);
        run_tsql_lite_session_batch_detailed(&engine, sid_a, &use_sql).unwrap();
        run_tsql_lite_session_batch_detailed(&engine, sid_b, &use_sql).unwrap();
        for (i, batch) in suite.setup_batches.iter().enumerate() {
            run_tsql_lite_session_batch_detailed(&engine, sid_a, batch).unwrap_or_else(|e| {
                panic!("suite '{}' setup[{}] failed: {}", suite.name, i, e)
            });
        }
        for step in suite.steps {
            match step {
                MultiSessionStep::ExecOk {
                    session,
                    label,
                    sql,
                } => {
                    let sid = if *session == SessionSlot::A { sid_a } else { sid_b };
                    run_tsql_lite_session_batch_detailed(&engine, sid, sql).unwrap_or_else(|e| {
                        panic!("suite '{}' [{}] ExecOk failed: {}", suite.name, label, e)
                    });
                }
                MultiSessionStep::ExpectErr {
                    session,
                    label,
                    sql,
                } => {
                    let sid = if *session == SessionSlot::A { sid_a } else { sid_b };
                    assert!(
                        run_tsql_lite_session_batch_detailed(&engine, sid, sql).is_err(),
                        "suite '{}' [{}] ExpectErr unexpectedly succeeded",
                        suite.name,
                        label
                    );
                }
                MultiSessionStep::ExpectErrCode {
                    session,
                    label,
                    sql,
                    expected_code,
                } => {
                    let sid = if *session == SessionSlot::A { sid_a } else { sid_b };
                    let err = run_tsql_lite_session_batch_detailed(&engine, sid, sql).expect_err(
                        &format!(
                            "suite '{}' [{}] ExpectErrCode unexpectedly succeeded",
                            suite.name, label
                        ),
                    );
                    if let Some(exp) = expected_code {
                        assert_eq!(
                            err.code,
                            Some(*exp),
                            "suite '{}' [{}] error code mismatch: got {:?}",
                            suite.name,
                            label,
                            err
                        );
                    }
                }
                MultiSessionStep::Query {
                    session,
                    label,
                    sql,
                    ..
                } => {
                    let sid = if *session == SessionSlot::A { sid_a } else { sid_b };
                    run_tsql_lite_session_batch_detailed(&engine, sid, sql).unwrap_or_else(|e| {
                        panic!("suite '{}' [{}] Query failed: {}", suite.name, label, e)
                    });
                }
            }
        }
        let _ = engine.close_session(sid_a);
        let _ = engine.close_session(sid_b);
    }
}

/// Live differential oracle comparing `tsql_lite_core` against a real SQL Server instance.
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
        "\n=== tsql-lite Differential Oracle vs Live SQL Server (major={}) ===",
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

    for suite in all_multi_session_suites() {
        let report = run_multi_session_differential_suite(&cfg, major, &suite).await;
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
