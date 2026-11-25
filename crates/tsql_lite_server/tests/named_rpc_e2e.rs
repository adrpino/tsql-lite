use tsql_lite_core::Database;
use tsql_lite_server::{ServerConfig, TdsServer};
use tiberius::{Client, Config, Row};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;

async fn start_server() -> u16 {
    let _ = env_logger::builder().is_test(true).try_init();

    let config = ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        auth: None,
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

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    port
}

async fn connect_tiberius(port: u16) -> Client<tokio_util::compat::Compat<TcpStream>> {
    let mut config = Config::new();
    config.host("127.0.0.1");
    config.port(port);
    config.trust_cert();
    config.encryption(tiberius::EncryptionLevel::NotSupported);

    let tcp = TcpStream::connect(config.get_addr()).await.unwrap();
    tcp.set_nodelay(true).unwrap();
    Client::connect(config, tcp.compat_write()).await.unwrap()
}

fn wrap_tds_packet(pkt_type: u8, payload: &[u8]) -> Vec<u8> {
    let total_len = (8 + payload.len()) as u16;
    let mut pkt = Vec::with_capacity(total_len as usize);
    pkt.push(pkt_type);
    pkt.push(0x01); // EOM
    pkt.extend_from_slice(&total_len.to_be_bytes());
    pkt.extend_from_slice(&0u16.to_be_bytes()); // spid
    pkt.push(1); // packet id
    pkt.push(0); // window
    pkt.extend_from_slice(payload);
    pkt
}

async fn read_tds_message(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut payload = Vec::new();
    let pkt_type;
    loop {
        let mut hdr = [0u8; 8];
        stream.read_exact(&mut hdr).await.unwrap();
        let cur_type = hdr[0];
        let status = hdr[1];
        let len = u16::from_be_bytes([hdr[2], hdr[3]]) as usize;
        let body_len = len.saturating_sub(8);
        let mut body = vec![0u8; body_len];
        stream.read_exact(&mut body).await.unwrap();
        payload.extend_from_slice(&body);
        if (status & 0x01) != 0 {
            pkt_type = cur_type;
            break;
        }
    }
    (pkt_type, payload)
}

async fn raw_tds_login(port: u16) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.set_nodelay(true).unwrap();

    // PRELOGIN with VERSION + ENCRYPTION=NOT_SUP (0x02)
    let prelogin_payload: [u8; 18] = [
        0x00, 0x00, 0x0B, 0x00, 0x06, // VERSION at offset 11, len 6
        0x01, 0x00, 0x11, 0x00, 0x01, // ENCRYPTION at offset 17, len 1
        0xFF, // TERMINATOR
        16, 0, 0, 0, 0, 0,    // version bytes
        0x02, // ENCRYPT_NOT_SUP
    ];
    stream
        .write_all(&wrap_tds_packet(0x12, &prelogin_payload))
        .await
        .unwrap();
    let (resp_type, _) = read_tds_message(&mut stream).await;
    assert_eq!(resp_type, 0x04);

    // Minimal TDS 7.4 LOGIN7 packet (94-byte fixed header, empty variable fields)
    let mut login = vec![0u8; 94];
    let len = login.len() as u32;
    login[0..4].copy_from_slice(&len.to_le_bytes());
    login[4..8].copy_from_slice(&0x74000004u32.to_le_bytes()); // TDS 7.4
    login[8..12].copy_from_slice(&4096u32.to_le_bytes()); // packet size
    stream
        .write_all(&wrap_tds_packet(0x10, &login))
        .await
        .unwrap();
    let (login_resp_type, login_resp) = read_tds_message(&mut stream).await;
    assert_eq!(login_resp_type, 0x04);
    assert!(
        login_resp.contains(&0xAD),
        "expected LOGINACK token (0xAD) in login response"
    );
    stream
}

fn push_utf16le(buf: &mut Vec<u8>, s: &str) {
    for u in s.encode_utf16() {
        buf.extend_from_slice(&u.to_le_bytes());
    }
}

fn encode_param_name(buf: &mut Vec<u8>, name: &str) {
    let units: Vec<u16> = name.encode_utf16().collect();
    buf.push(units.len() as u8);
    for u in units {
        buf.extend_from_slice(&u.to_le_bytes());
    }
}

fn contains_utf16le_ignore_ascii_case(haystack: &[u8], needle: &str) -> bool {
    let needle_u16: Vec<u16> = needle
        .encode_utf16()
        .map(|u| {
            if (b'A' as u16..=b'Z' as u16).contains(&u) {
                u + 32
            } else {
                u
            }
        })
        .collect();
    let needle_bytes = needle_u16.len() * 2;
    if haystack.len() < needle_bytes {
        return false;
    }
    (0..=haystack.len() - needle_bytes).any(|off| {
        needle_u16.iter().enumerate().all(|(i, &nu)| {
            let mut hu = u16::from_le_bytes([haystack[off + i * 2], haystack[off + i * 2 + 1]]);
            if (b'A' as u16..=b'Z' as u16).contains(&hu) {
                hu += 32;
            }
            hu == nu
        })
    })
}

#[tokio::test]
async fn named_stored_procedure_rpc_roundtrip_over_tcp() {
    let port = start_server().await;

    // 1. Seed schema + stored procedures via tiberius over the same live server
    let mut client = connect_tiberius(port).await;
    client
        .execute(
            "CREATE TYPE dbo.GuidTagList AS TABLE (TagId INT, ItemGuid UNIQUEIDENTIFIER);",
            &[],
        )
        .await
        .unwrap();
    client
        .execute(
            "CREATE PROCEDURE dbo.usp_InspectPayload
                @ItemGuid UNIQUEIDENTIFIER,
                @Note NVARCHAR(MAX),
                @Tag INT
             AS
             BEGIN
                SELECT CAST(@ItemGuid AS NVARCHAR(36)) AS GuidText,
                       LEN(@Note) AS NoteLen,
                       @Tag AS TagVal;
             END;",
            &[],
        )
        .await
        .unwrap();
    client
        .execute(
            "CREATE PROCEDURE dbo.usp_CountGuidTvp @Items dbo.GuidTagList READONLY AS
             BEGIN
                SELECT COUNT(*) AS RowCnt, MIN(CAST(ItemGuid AS NVARCHAR(36))) AS FirstGuid
                FROM @Items;
             END;",
            &[],
        )
        .await
        .unwrap();

    // Also verify parameterized NVARCHAR(MAX) PLP through tiberius sp_executesql
    let long_str = "x".repeat(5000);
    let rows: Vec<Row> = client
        .query("SELECT LEN(@P1) AS L", &[&long_str.as_str()])
        .await
        .unwrap()
        .into_first_result()
        .await
        .unwrap();
    let len_val: i32 = rows[0].get::<i32, _>(0).unwrap();
    assert_eq!(len_val, 5000);

    // 2. Open raw TDS connection and send real named-proc RPC frames (0x03)
    let mut raw = raw_tds_login(port).await;

    // 2a. Named RPC: xp_msver with duplicate @optname params
    let mut xp_frame = Vec::new();
    xp_frame.extend_from_slice(&4u32.to_le_bytes()); // ALL_HEADERS len = 4
    let xp_name = "xp_msver";
    xp_frame.extend_from_slice(&(xp_name.encode_utf16().count() as u16).to_le_bytes());
    push_utf16le(&mut xp_frame, xp_name);
    xp_frame.extend_from_slice(&0u16.to_le_bytes()); // option flags
    for opt in ["Platform", "ProductVersion"] {
        encode_param_name(&mut xp_frame, "@optname");
        xp_frame.push(0x00); // status
        xp_frame.push(0xE7); // NVARCHAR
        let byte_len = (opt.encode_utf16().count() * 2) as u16;
        xp_frame.extend_from_slice(&byte_len.to_le_bytes());
        xp_frame.extend_from_slice(&[0x09, 0x04, 0xD0, 0x00, 0x34]); // collation
        xp_frame.extend_from_slice(&byte_len.to_le_bytes());
        push_utf16le(&mut xp_frame, opt);
    }
    raw.write_all(&wrap_tds_packet(0x03, &xp_frame))
        .await
        .unwrap();
    let (resp_type, xp_resp) = read_tds_message(&mut raw).await;
    assert_eq!(resp_type, 0x04);
    assert!(
        !xp_resp.contains(&0xAA),
        "xp_msver RPC should not return TDS ERROR token"
    );
    assert!(
        contains_utf16le_ignore_ascii_case(&xp_resp, "ProductVersion"),
        "expected ProductVersion in xp_msver TDS response"
    );

    // 2b. Named user RPC: dbo.usp_InspectPayload with GUIDTYPE (0x24) + PLP NVARCHAR(MAX) + INTN
    let mut sp_frame = Vec::new();
    sp_frame.extend_from_slice(&4u32.to_le_bytes());
    let sp_name = "dbo.usp_InspectPayload";
    sp_frame.extend_from_slice(&(sp_name.encode_utf16().count() as u16).to_le_bytes());
    push_utf16le(&mut sp_frame, sp_name);
    sp_frame.extend_from_slice(&0u16.to_le_bytes());

    // @ItemGuid UNIQUEIDENTIFIER = 12345678-9ABC-DEF0-1122-334455667788
    encode_param_name(&mut sp_frame, "@ItemGuid");
    sp_frame.push(0x00);
    sp_frame.push(0x24); // GUIDTYPE
    sp_frame.push(0x10); // max_len
    sp_frame.push(0x10); // actual_len
    sp_frame.extend_from_slice(&[
        0x78, 0x56, 0x34, 0x12, 0xBC, 0x9A, 0xF0, 0xDE, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88,
    ]);

    // @Note NVARCHAR(MAX) PLP ("alpha-payload")
    encode_param_name(&mut sp_frame, "@Note");
    sp_frame.push(0x00);
    sp_frame.push(0xE7); // NVARCHARTYPE
    sp_frame.extend_from_slice(&0xFFFFu16.to_le_bytes()); // MAX -> PLP
    sp_frame.extend_from_slice(&[0x09, 0x04, 0xD0, 0x00, 0x34]);
    let mut note_bytes = Vec::new();
    push_utf16le(&mut note_bytes, "alpha-payload");
    sp_frame.extend_from_slice(&(note_bytes.len() as u64).to_le_bytes());
    sp_frame.extend_from_slice(&(note_bytes.len() as u32).to_le_bytes());
    sp_frame.extend_from_slice(&note_bytes);
    sp_frame.extend_from_slice(&0u32.to_le_bytes()); // PLP end

    // @Tag INT = 77
    encode_param_name(&mut sp_frame, "@Tag");
    sp_frame.push(0x00);
    sp_frame.push(0x26); // INTNTYPE
    sp_frame.push(0x04); // max_len
    sp_frame.push(0x04); // actual_len
    sp_frame.extend_from_slice(&77i32.to_le_bytes());

    raw.write_all(&wrap_tds_packet(0x03, &sp_frame))
        .await
        .unwrap();
    let (resp_type, sp_resp) = read_tds_message(&mut raw).await;
    assert_eq!(resp_type, 0x04);
    assert!(
        !sp_resp.contains(&0xAA),
        "dbo.usp_InspectPayload RPC should not return TDS ERROR token"
    );
    assert!(
        contains_utf16le_ignore_ascii_case(&sp_resp, "12345678-9ABC-DEF0-1122-334455667788"),
        "expected formatted GUID in response payload (resp={:02X?})",
        sp_resp
    );

    // 2c. Named user RPC with TVP (0xF3) containing INT + UNIQUEIDENTIFIER cells
    let mut tvp_frame = Vec::new();
    tvp_frame.extend_from_slice(&4u32.to_le_bytes());
    let tvp_proc = "dbo.usp_CountGuidTvp";
    tvp_frame.extend_from_slice(&(tvp_proc.encode_utf16().count() as u16).to_le_bytes());
    push_utf16le(&mut tvp_frame, tvp_proc);
    tvp_frame.extend_from_slice(&0u16.to_le_bytes());

    encode_param_name(&mut tvp_frame, "@Items");
    tvp_frame.push(0x00);
    tvp_frame.push(0xF3); // TVPTYPE
    encode_param_name(&mut tvp_frame, ""); // db
    encode_param_name(&mut tvp_frame, "dbo"); // schema
    encode_param_name(&mut tvp_frame, "GuidTagList"); // type name
    tvp_frame.extend_from_slice(&2u16.to_le_bytes()); // 2 columns
                                                      // Col 1: TagId INTN(4)
    tvp_frame.extend_from_slice(&0u32.to_le_bytes()); // user_type
    tvp_frame.extend_from_slice(&0u16.to_le_bytes()); // flags
    tvp_frame.push(0x26); // INTN
    tvp_frame.push(0x04); // max_len
    tvp_frame.push(0x00); // empty col name
                          // Col 2: ItemGuid GUIDTYPE(16)
    tvp_frame.extend_from_slice(&0u32.to_le_bytes());
    tvp_frame.extend_from_slice(&0u16.to_le_bytes());
    tvp_frame.push(0x24); // GUIDTYPE
    tvp_frame.push(0x10); // max_len
    tvp_frame.push(0x00); // empty col name
                          // TVP Row 1
    tvp_frame.push(0x01); // TVP_ROW_TOKEN
    tvp_frame.push(0x04); // int len
    tvp_frame.extend_from_slice(&10i32.to_le_bytes());
    tvp_frame.push(0x10); // guid len
    tvp_frame.extend_from_slice(&[
        0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB,
        0xCC,
    ]);
    tvp_frame.push(0x00); // TVP_END_TOKEN

    raw.write_all(&wrap_tds_packet(0x03, &tvp_frame))
        .await
        .unwrap();
    let (resp_type, tvp_resp) = read_tds_message(&mut raw).await;
    assert_eq!(resp_type, 0x04);
    assert!(
        !tvp_resp.contains(&0xAA),
        "dbo.usp_CountGuidTvp RPC should not return TDS ERROR token"
    );
    assert!(
        contains_utf16le_ignore_ascii_case(&tvp_resp, "DDCCBBAA-2211-4433-5566-778899AABBCC"),
        "expected TVP GUID cell round-trip in response payload (resp={:02X?})",
        tvp_resp
    );
}
