use std::time::Duration;

use tsql_lite_core::Database;
use tsql_lite_server::tds::packet::{self, TABULAR_RESULT, TDS7_PRELOGIN};
use tsql_lite_server::tds::prelogin::{
    build_prelogin_response, parse_prelogin, ENCRYPT_NOT_SUP, ENCRYPT_OFF,
};
use tsql_lite_server::{Credentials, ServerConfig, TdsServer};
use tiberius::{Client, Config, EncryptionLevel};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;

mod common;
use common::*;

#[tokio::test]
async fn test_auth_reject() {
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        auth: Some(Credentials {
            user: "admin".to_string(),
            password: "secret".to_string(),
        }),
        database: "master".to_string(),
        packet_size: 4096,
        tls_enabled: false,
        tls_cert_path: None,
        tls_key_path: None,
        pool_min_size: 1,
        pool_max_size: 50,
        pool_idle_timeout_secs: 300,
        data_dir: None,
    };

    let mut server = TdsServer::new_with_database(Database::new(), config);
    let addr = server.bind().await.unwrap();
    let port = addr.port();

    tokio::spawn(async move {
        server.run().await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut tds_config = Config::new();
    tds_config.host("127.0.0.1");
    tds_config.port(port);
    tds_config.trust_cert();
    tds_config.encryption(tiberius::EncryptionLevel::NotSupported);
    tds_config.authentication(tiberius::AuthMethod::sql_server("wrong", "creds"));

    let tcp = TcpStream::connect(tds_config.get_addr())
        .await
        .expect("Failed to connect");
    tcp.set_nodelay(true).unwrap();

    let result = Client::connect(tds_config, tcp.compat_write()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_auth_accept() {
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        auth: Some(Credentials {
            user: "admin".to_string(),
            password: "secret".to_string(),
        }),
        database: "master".to_string(),
        packet_size: 4096,
        tls_enabled: false,
        tls_cert_path: None,
        tls_key_path: None,
        pool_min_size: 1,
        pool_max_size: 50,
        pool_idle_timeout_secs: 300,
        data_dir: None,
    };

    let mut server = TdsServer::new_with_database(Database::new(), config);
    let addr = server.bind().await.unwrap();
    let port = addr.port();

    tokio::spawn(async move {
        server.run().await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut tds_config = Config::new();
    tds_config.host("127.0.0.1");
    tds_config.port(port);
    tds_config.trust_cert();
    tds_config.encryption(tiberius::EncryptionLevel::NotSupported);
    tds_config.authentication(tiberius::AuthMethod::sql_server("admin", "secret"));

    let tcp = TcpStream::connect(tds_config.get_addr())
        .await
        .expect("Failed to connect");
    tcp.set_nodelay(true).unwrap();

    let mut client = Client::connect(tds_config, tcp.compat_write())
        .await
        .expect("Should connect with correct creds");

    let (_, rows) = query_sql(&mut client, "SELECT 99 as val").await;
    assert_eq!(rows[0][0], "99");
}

fn temp_cert_paths(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir();
    let pid = std::process::id();
    let cert = dir.join(format!("tsql_lite_test_{}_{}.crt", tag, pid));
    let key = dir.join(format!("tsql_lite_test_{}_{}.key", tag, pid));
    tsql_lite_server::tls::generate_self_signed_cert(cert.to_str().unwrap(), key.to_str().unwrap())
        .expect("generate self-signed cert");
    (cert, key)
}

#[tokio::test]
async fn test_no_tls_replies_encrypt_not_sup_for_client_encrypt_off() {
    let port = start_server().await;

    let mut tcp = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("Failed to connect");
    tcp.set_nodelay(true).unwrap();

    // Send a standard MS-TDS PRELOGIN packet advertising ENCRYPT_OFF (0x00).
    let req = build_prelogin_response(ENCRYPT_OFF);
    packet::write_packet(&mut tcp, TDS7_PRELOGIN, &req)
        .await
        .expect("Failed to write PRELOGIN");
    tcp.flush().await.expect("Failed to flush PRELOGIN");

    let (header, resp) =
        tokio::time::timeout(Duration::from_secs(5), packet::read_message(&mut tcp))
            .await
            .expect("Timed out reading PRELOGIN response")
            .expect("Failed to read PRELOGIN response");

    assert_eq!(
        header.packet_type, TABULAR_RESULT,
        "PRELOGIN response packet type should be TABULAR_RESULT (0x04)"
    );
    let parsed = parse_prelogin(&resp).expect("Failed to parse PRELOGIN response");
    assert_eq!(
        parsed.encryption, ENCRYPT_NOT_SUP,
        "no-TLS server must reply ENCRYPT_NOT_SUP (0x02) when client sends ENCRYPT_OFF (0x00)"
    );
}

#[tokio::test]
async fn test_no_tls_rejects_encryption_required() {
    let port = start_server().await;

    let mut tds_config = Config::new();
    tds_config.host("127.0.0.1");
    tds_config.port(port);
    tds_config.trust_cert();
    tds_config.encryption(EncryptionLevel::Required);

    let tcp = TcpStream::connect(tds_config.get_addr())
        .await
        .expect("Failed to connect");
    tcp.set_nodelay(true).unwrap();

    let res = tokio::time::timeout(
        Duration::from_secs(5),
        Client::connect(tds_config, tcp.compat_write()),
    )
    .await
    .expect("Handshake should fail fast, not time out");
    assert!(
        res.is_err(),
        "no-TLS server must reject EncryptionLevel::Required"
    );
}

#[tokio::test]
async fn test_tls_required_handshake_and_query() {
    let (cert, key) = temp_cert_paths("tls_req");
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        auth: None,
        database: "master".to_string(),
        packet_size: 4096,
        tls_enabled: true,
        tls_cert_path: Some(cert.to_string_lossy().to_string()),
        tls_key_path: Some(key.to_string_lossy().to_string()),
        pool_min_size: 1,
        pool_max_size: 50,
        pool_idle_timeout_secs: 300,
        data_dir: None,
    };

    let port = start_server_with_config(config).await;

    let mut tds_config = Config::new();
    tds_config.host("127.0.0.1");
    tds_config.port(port);
    tds_config.trust_cert();
    tds_config.encryption(EncryptionLevel::Required);

    let tcp = TcpStream::connect(tds_config.get_addr())
        .await
        .expect("Failed to connect");
    tcp.set_nodelay(true).unwrap();

    let connect_res = tokio::time::timeout(
        Duration::from_secs(5),
        Client::connect(tds_config, tcp.compat_write()),
    )
    .await;

    let _ = std::fs::remove_file(&cert);
    let _ = std::fs::remove_file(&key);

    let mut client = connect_res
        .expect("TLS handshake timed out for EncryptionLevel::Required")
        .expect("TLS server should complete handshake for EncryptionLevel::Required");

    let (_, rows) = query_sql(&mut client, "SELECT 42 AS val").await;
    assert_eq!(rows[0][0], "42");
}

#[tokio::test]
async fn test_tls_enabled_accepts_encryption_off() {
    let (cert, key) = temp_cert_paths("tls_off");
    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        auth: None,
        database: "master".to_string(),
        packet_size: 4096,
        tls_enabled: true,
        tls_cert_path: Some(cert.to_string_lossy().to_string()),
        tls_key_path: Some(key.to_string_lossy().to_string()),
        pool_min_size: 1,
        pool_max_size: 50,
        pool_idle_timeout_secs: 300,
        data_dir: None,
    };

    let port = start_server_with_config(config).await;

    let mut tds_config = Config::new();
    tds_config.host("127.0.0.1");
    tds_config.port(port);
    tds_config.trust_cert();
    tds_config.encryption(EncryptionLevel::Off);

    let tcp = TcpStream::connect(tds_config.get_addr())
        .await
        .expect("Failed to connect");
    tcp.set_nodelay(true).unwrap();

    let connect_res = tokio::time::timeout(
        Duration::from_secs(5),
        Client::connect(tds_config, tcp.compat_write()),
    )
    .await;

    let _ = std::fs::remove_file(&cert);
    let _ = std::fs::remove_file(&key);

    let mut client = connect_res
        .expect("TLS handshake timed out for EncryptionLevel::Off")
        .expect("TLS server should negotiate encryption for EncryptionLevel::Off");

    let (_, rows) = query_sql(&mut client, "SELECT 42 AS val").await;
    assert_eq!(rows[0][0], "42");
}

#[tokio::test]
async fn test_large_nvarchar_max_for_json_streams_without_truncation() {
    let port = start_server().await;
    let mut client = connect(port).await;

    exec_sql(
        &mut client,
        "CREATE TABLE dbo.LargePayload (id INT NOT NULL PRIMARY KEY, body NVARCHAR(200));",
    )
    .await;

    // Insert 200 rows via SQL_BATCH so FOR JSON PATH exceeds 15,000 characters
    // (far above legacy 2,048 chars / 4,096 bytes TEXTSIZE).
    let mut insert = String::from("INSERT INTO dbo.LargePayload (id, body) VALUES ");
    for i in 1..=200 {
        if i > 1 {
            insert.push(',');
        }
        insert.push_str(&format!(
            "({i}, N'payload_item_{i:04}_abcdefghijklmnopqrstuvwxyz_0123456789')"
        ));
    }
    client
        .simple_query(&insert)
        .await
        .expect("insert batch failed")
        .into_results()
        .await
        .expect("insert results failed");

    let rows = client
        .simple_query("SELECT id, body FROM dbo.LargePayload ORDER BY id FOR JSON PATH")
        .await
        .expect("FOR JSON query failed")
        .into_first_result()
        .await
        .expect("FOR JSON result failed");
    assert_eq!(rows.len(), 1);
    let json_text = rows[0].get::<&str, _>(0).expect("JSON string column");
    assert!(
        json_text.len() > 15_000,
        "expected > 15,000 chars JSON payload, got {} chars",
        json_text.len()
    );
    let parsed: serde_json::Value = serde_json::from_str(json_text)
        .expect("large FOR JSON payload must parse without truncation");
    let arr = parsed.as_array().expect("JSON array");
    assert_eq!(arr.len(), 200);
    assert_eq!(arr[0]["id"], 1);
    assert_eq!(arr[199]["id"], 200);
}
