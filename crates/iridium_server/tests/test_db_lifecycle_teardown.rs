mod common;
use common::*;

async fn exec_expect_err_code(
    client: &mut tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>,
    sql: &str,
) -> Option<u32> {
    let err = match client.simple_query(sql).await {
        Ok(stream) => stream.into_results().await.err()?,
        Err(e) => e,
    };
    match err {
        tiberius::error::Error::Server(tok) => Some(tok.code()),
        _ => None,
    }
}

#[tokio::test]
async fn test_tds_pooled_single_user_rollback_immediate_and_error_codes() {
    let port = start_server().await;
    let mut conn_a = connect(port).await;
    let mut conn_b = connect(port).await;

    // Create throwaway DB and populate a test schema
    exec_sql(&mut conn_a, "CREATE DATABASE [TdsPoolDb];").await;
    exec_sql(
        &mut conn_a,
        "USE [TdsPoolDb];
         CREATE TABLE dbo.Users (
             Id INT PRIMARY KEY,
             Code VARCHAR(4) NOT NULL,
             Email VARCHAR(50) NULL
         );
         CREATE UNIQUE INDEX UX_Users_Email ON dbo.Users(Email) WHERE Email IS NOT NULL;
         INSERT INTO dbo.Users VALUES (1, 'A1', 'a@example.com');",
    )
    .await;

    // Verify exact TDS wire error numbers (2627 PK, 2601 unique index, 2628 truncation)
    assert_eq!(
        exec_expect_err_code(
            &mut conn_a,
            "INSERT INTO dbo.Users VALUES (1, 'B1', 'b@example.com');"
        )
        .await,
        Some(2627)
    );
    assert_eq!(
        exec_expect_err_code(
            &mut conn_a,
            "INSERT INTO dbo.Users VALUES (2, 'B2', 'a@example.com');"
        )
        .await,
        Some(2601)
    );
    assert_eq!(
        exec_expect_err_code(
            &mut conn_a,
            "INSERT INTO dbo.Users VALUES (3, 'TOOLONG', 'c@example.com');"
        )
        .await,
        Some(2628)
    );

    // Park conn_b inside TdsPoolDb with an open uncommitted transaction (simulating pooled connection)
    exec_sql(
        &mut conn_b,
        "USE [TdsPoolDb]; BEGIN TRAN; INSERT INTO dbo.Users VALUES (9, 'X9', 'x@example.com');",
    )
    .await;

    // conn_a switches to master; raw DROP DATABASE fails with 3702 while conn_b is attached
    exec_sql(&mut conn_a, "USE master;").await;
    assert_eq!(
        exec_expect_err_code(&mut conn_a, "DROP DATABASE [TdsPoolDb];").await,
        Some(3702)
    );

    // Teardown via SINGLE_USER WITH ROLLBACK IMMEDIATE evicts conn_b and drops cleanly
    exec_sql(
        &mut conn_a,
        "ALTER DATABASE [TdsPoolDb] SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE [TdsPoolDb];",
    )
    .await;

    let (_, rows) = query_sql(
        &mut conn_a,
        "SELECT CASE WHEN DB_ID('TdsPoolDb') IS NULL THEN 1 ELSE 0 END AS Dropped",
    )
    .await;
    assert_eq!(rows, vec![vec!["1".to_string()]]);

    let (_, b_rows) = query_sql(&mut conn_b, "SELECT DB_NAME(), @@TRANCOUNT").await;
    assert_eq!(b_rows, vec![vec!["master".to_string(), "0".to_string()]]);
}
