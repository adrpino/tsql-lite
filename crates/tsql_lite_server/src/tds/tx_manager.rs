use std::io;

use super::packet::PacketReader;

pub const TM_GET_DTC_ADDRESS: u16 = 0;
pub const TM_PROPAGATE_XACT: u16 = 1;
pub const TM_BEGIN_XACT: u16 = 5;
pub const TM_PROMOTE_XACT: u16 = 6;
pub const TM_COMMIT_XACT: u16 = 7;
pub const TM_ROLLBACK_XACT: u16 = 8;
pub const TM_SAVE_XACT: u16 = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxManagerRequest {
    Begin {
        isolation_level: u8,
        name: String,
    },
    Commit {
        name: String,
        chain_begin: Option<(u8, String)>,
    },
    Rollback {
        name: String,
        chain_begin: Option<(u8, String)>,
    },
    Save {
        name: String,
    },
    Other(u16),
}

pub fn isolation_level_sql(code: u8) -> Option<&'static str> {
    match code {
        0x01 => Some("SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED"),
        0x02 => Some("SET TRANSACTION ISOLATION LEVEL READ COMMITTED"),
        0x03 => Some("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ"),
        0x04 => Some("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE"),
        0x05 => Some("SET TRANSACTION ISOLATION LEVEL SNAPSHOT"),
        _ => None,
    }
}

pub fn parse_tx_manager_request(data: &[u8]) -> io::Result<TxManagerRequest> {
    let mut r = PacketReader::new(data);
    if r.remaining() < 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "TM request too short for ALL_HEADERS",
        ));
    }

    let total_headers = r.read_u32_le()? as usize;
    if total_headers >= 4 && total_headers - 4 <= r.remaining() {
        r.skip(total_headers - 4)?;
    }

    if r.remaining() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "TM request missing RequestType",
        ));
    }
    let req_type = r.read_u16_le()?;

    match req_type {
        TM_BEGIN_XACT => {
            let iso = if r.remaining() >= 1 { r.read_u8()? } else { 0 };
            let name = read_tm_name(&mut r)?;
            Ok(TxManagerRequest::Begin {
                isolation_level: iso,
                name,
            })
        }
        TM_COMMIT_XACT => {
            let name = read_tm_name(&mut r)?;
            let chain_begin = read_optional_chain_begin(&mut r)?;
            Ok(TxManagerRequest::Commit { name, chain_begin })
        }
        TM_ROLLBACK_XACT => {
            let name = read_tm_name(&mut r)?;
            let chain_begin = read_optional_chain_begin(&mut r)?;
            Ok(TxManagerRequest::Rollback { name, chain_begin })
        }
        TM_SAVE_XACT => {
            let name = read_tm_name(&mut r)?;
            Ok(TxManagerRequest::Save { name })
        }
        other => Ok(TxManagerRequest::Other(other)),
    }
}

fn read_optional_chain_begin(r: &mut PacketReader) -> io::Result<Option<(u8, String)>> {
    if r.remaining() < 1 {
        return Ok(None);
    }
    let flags = r.read_u8()?;
    if (flags & 0x01) != 0 && r.remaining() >= 1 {
        let iso = r.read_u8()?;
        let next_name = read_tm_name(r)?;
        return Ok(Some((iso, next_name)));
    }
    Ok(None)
}

fn read_tm_name(r: &mut PacketReader) -> io::Result<String> {
    if r.remaining() < 1 {
        return Ok(String::new());
    }
    let byte_len = r.read_u8()? as usize;
    if byte_len == 0 || r.remaining() < byte_len {
        return Ok(String::new());
    }
    let bytes = r.read_bytes(byte_len)?;
    let mut u16s = Vec::with_capacity(bytes.len() / 2);
    for &chunk in bytes.as_chunks::<2>().0 {
        u16s.push(u16::from_le_bytes(chunk));
    }
    Ok(String::from_utf16_lossy(&u16s))
}

pub fn quote_tsql_ident(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_tm_packet(payload: &[u8]) -> Vec<u8> {
        // Minimal ALL_HEADERS with 18-byte Transaction Descriptor header (TotalLen = 22)
        let mut buf = Vec::new();
        buf.extend_from_slice(&22u32.to_le_bytes()); // TotalLength
        buf.extend_from_slice(&18u32.to_le_bytes()); // HeaderLength
        buf.extend_from_slice(&2u16.to_le_bytes()); // HeaderType = TransactionDescriptor
        buf.extend_from_slice(&[0u8; 8]); // TxDescriptor
        buf.extend_from_slice(&1u32.to_le_bytes()); // OutstandingRequestCount
        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn parse_begin_save_rollback_commit() {
        let begin_pkt = build_tm_packet(&[5, 0, 2, 0]); // TM_BEGIN_XACT, iso=2, empty name
        assert_eq!(
            parse_tx_manager_request(&begin_pkt).unwrap(),
            TxManagerRequest::Begin {
                isolation_level: 2,
                name: String::new()
            }
        );

        let mut save_payload = vec![9, 0, 6]; // TM_SAVE_XACT, 6 bytes ("sp1" in UTF-16LE)
        for ch in "sp1".encode_utf16() {
            save_payload.extend_from_slice(&ch.to_le_bytes());
        }
        assert_eq!(
            parse_tx_manager_request(&build_tm_packet(&save_payload)).unwrap(),
            TxManagerRequest::Save {
                name: "sp1".to_string()
            }
        );

        let commit_pkt = build_tm_packet(&[7, 0, 0, 0]);
        assert_eq!(
            parse_tx_manager_request(&commit_pkt).unwrap(),
            TxManagerRequest::Commit {
                name: String::new(),
                chain_begin: None
            }
        );
    }
}
