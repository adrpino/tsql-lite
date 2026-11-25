use crate::tds::packet::PacketReader;
use std::io;

use super::temporal::{
    decode_date_bytes, decode_datetime2_bytes, decode_datetime_bytes, decode_datetimeoffset_bytes,
    decode_money_bytes, decode_time_bytes,
};
use super::types::DecodedValue;

#[derive(Debug, Clone)]
struct TvpColMeta {
    type_id: u8,
    max_len: u16,
    precision: u8,
    scale: u8,
}

pub(super) fn read_typed_value(r: &mut PacketReader, type_id: u8) -> io::Result<DecodedValue> {
    match type_id {
        // INTNTYPE
        0x26 => {
            let max_len = r.read_u8()?;
            let actual_len = r.read_u8()?;
            if actual_len == 0 || r.remaining() < actual_len as usize {
                return Ok(scalar(int_type_name(max_len), "NULL".into()));
            }
            let bytes = r.read_bytes(actual_len as usize)?;
            Ok(scalar(
                int_type_name(max_len),
                read_le_int(bytes).to_string(),
            ))
        }
        // BITNTYPE
        0x68 => {
            let _max = r.read_u8()?;
            let actual = r.read_u8()?;
            if actual == 0 {
                return Ok(scalar("BIT".into(), "NULL".into()));
            }
            let b = r.read_u8()?;
            Ok(scalar(
                "BIT".into(),
                if b != 0 { "1".into() } else { "0".into() },
            ))
        }
        // FLTNTYPE
        0x6D => {
            let _max_len = r.read_u8()?;
            let actual_len = r.read_u8()?;
            if actual_len == 0 || r.remaining() < actual_len as usize {
                return Ok(scalar("FLOAT".into(), "NULL".into()));
            }
            let bytes = r.read_bytes(actual_len as usize)?;
            let val = if actual_len == 4 {
                f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64
            } else {
                f64::from_le_bytes(bytes.try_into().unwrap_or([0u8; 8]))
            };
            Ok(scalar("FLOAT".into(), format!("{}", val)))
        }
        // NVARCHARTYPE / NCHARTYPE
        0xE7 | 0xEF => {
            if r.remaining() < 7 {
                return Ok(scalar("NVARCHAR(MAX)".into(), "NULL".into()));
            }
            let max_len = r.read_u16_le()?;
            r.skip(5)?;
            if max_len == 0xFFFF {
                return match read_plp_bytes(r)? {
                    Some(bytes) => {
                        let s = decode_utf16le(&bytes);
                        Ok(scalar(
                            "NVARCHAR(MAX)".into(),
                            format!("N'{}'", s.replace('\'', "''")),
                        ))
                    }
                    None => Ok(scalar("NVARCHAR(MAX)".into(), "NULL".into())),
                };
            }
            if r.remaining() < 2 {
                return Ok(scalar("NVARCHAR(MAX)".into(), "NULL".into()));
            }
            let actual_len = r.read_u16_le()? as usize;
            let ty = if type_id == 0xEF {
                format!("NCHAR({})", (max_len / 2).max(1))
            } else {
                format!("NVARCHAR({})", (max_len / 2).max(1))
            };
            if actual_len == 0xFFFF || r.remaining() < actual_len {
                return Ok(scalar(ty, "NULL".into()));
            }
            let bytes = r.read_bytes(actual_len)?;
            let s = decode_utf16le(bytes);
            Ok(scalar(ty, format!("N'{}'", s.replace('\'', "''"))))
        }
        // BIGVARCHARTYPE / BIGCHARTYPE
        0xA7 | 0xAF => {
            if r.remaining() < 7 {
                return Ok(scalar("VARCHAR(MAX)".into(), "NULL".into()));
            }
            let max_len = r.read_u16_le()?;
            r.skip(5)?;
            if max_len == 0xFFFF {
                return match read_plp_bytes(r)? {
                    Some(bytes) => {
                        let s = String::from_utf8_lossy(&bytes).to_string();
                        Ok(scalar(
                            "VARCHAR(MAX)".into(),
                            format!("'{}'", s.replace('\'', "''")),
                        ))
                    }
                    None => Ok(scalar("VARCHAR(MAX)".into(), "NULL".into())),
                };
            }
            if r.remaining() < 2 {
                return Ok(scalar("VARCHAR(MAX)".into(), "NULL".into()));
            }
            let actual_len = r.read_u16_le()? as usize;
            let ty = if type_id == 0xAF {
                format!("CHAR({})", max_len.max(1))
            } else {
                format!("VARCHAR({})", max_len.max(1))
            };
            if actual_len == 0xFFFF || r.remaining() < actual_len {
                return Ok(scalar(ty, "NULL".into()));
            }
            let bytes = r.read_bytes(actual_len)?;
            let s = String::from_utf8_lossy(bytes).to_string();
            Ok(scalar(ty, format!("'{}'", s.replace('\'', "''"))))
        }
        // BIGVARBINARYTYPE / BIGBINARYTYPE
        0xA5 | 0xAD => {
            if r.remaining() < 2 {
                return Ok(scalar("VARBINARY(MAX)".into(), "NULL".into()));
            }
            let max_len = r.read_u16_le()?;
            if max_len == 0xFFFF {
                return match read_plp_bytes(r)? {
                    Some(bytes) => Ok(scalar("VARBINARY(MAX)".into(), format_hex_literal(&bytes))),
                    None => Ok(scalar("VARBINARY(MAX)".into(), "NULL".into())),
                };
            }
            if r.remaining() < 2 {
                return Ok(scalar("VARBINARY(MAX)".into(), "NULL".into()));
            }
            let actual_len = r.read_u16_le()? as usize;
            let ty = if type_id == 0xAD {
                format!("BINARY({})", max_len.max(1))
            } else {
                format!("VARBINARY({})", max_len.max(1))
            };
            if actual_len == 0xFFFF || r.remaining() < actual_len {
                return Ok(scalar(ty, "NULL".into()));
            }
            let bytes = r.read_bytes(actual_len)?;
            Ok(scalar(ty, format_hex_literal(bytes)))
        }
        // NTEXTTYPE
        0x63 => {
            if r.remaining() < 13 {
                return Ok(scalar("NTEXT".into(), "NULL".into()));
            }
            let _max_len = r.read_u32_le()?;
            r.skip(5)?;
            let actual_len = r.read_u32_le()? as usize;
            if r.remaining() < actual_len {
                return Ok(scalar("NTEXT".into(), "NULL".into()));
            }
            let bytes = r.read_bytes(actual_len)?;
            let s = decode_utf16le(bytes);
            Ok(scalar(
                "NTEXT".into(),
                format!("N'{}'", s.replace('\'', "''")),
            ))
        }
        // TEXTTYPE
        0x23 => {
            if r.remaining() < 13 {
                return Ok(scalar("TEXT".into(), "NULL".into()));
            }
            let _max_len = r.read_u32_le()?;
            r.skip(5)?;
            let actual_len = r.read_u32_le()? as usize;
            if r.remaining() < actual_len {
                return Ok(scalar("TEXT".into(), "NULL".into()));
            }
            let bytes = r.read_bytes(actual_len)?;
            let s = String::from_utf8_lossy(bytes).to_string();
            Ok(scalar(
                "TEXT".into(),
                format!("'{}'", s.replace('\'', "''")),
            ))
        }
        // DECIMALNTYPE / NUMERICNTYPE
        0x6A | 0x6C => {
            let _max_len = r.read_u8()?;
            let precision = r.read_u8()?;
            let scale = r.read_u8()?;
            let actual_len = r.read_u8()?;
            let ty = format!("DECIMAL({},{})", precision, scale);
            if actual_len == 0 || r.remaining() < actual_len as usize {
                return Ok(scalar(ty, "NULL".into()));
            }
            let bytes = r.read_bytes(actual_len as usize)?;
            let sign = bytes[0];
            let mut val: i128 = 0;
            for i in (1..bytes.len()).rev() {
                val = (val << 8) | bytes[i] as i128;
            }
            if sign == 0 {
                val = -val;
            }
            Ok(scalar(ty, format_decimal(val, scale)))
        }
        // GUIDTYPE: TYPE_INFO has max_len (u8=16), followed by actual_len (u8=0|16) + 16 bytes.
        0x24 => {
            let _max_len = r.read_u8()?;
            let actual_len = r.read_u8()?;
            if actual_len == 0 || r.remaining() < 16 {
                return Ok(scalar("UNIQUEIDENTIFIER".into(), "NULL".into()));
            }
            let b = r.read_bytes(16)?;
            Ok(scalar(
                "UNIQUEIDENTIFIER".into(),
                format!("'{}'", format_guid_bytes(b)),
            ))
        }
        // DATENTYPE (DATE: 0-byte TYPE_INFO, 1-byte len = 0|3, 3-byte days since 0001-01-01)
        0x28 => {
            let actual_len = r.read_u8()? as usize;
            if actual_len == 0 || r.remaining() < actual_len {
                return Ok(scalar("DATE".into(), "NULL".into()));
            }
            let b = r.read_bytes(actual_len)?;
            match decode_date_bytes(b) {
                Some(s) => Ok(scalar("DATE".into(), format!("'{}'", s))),
                None => Ok(scalar("DATE".into(), "NULL".into())),
            }
        }
        // TIMENTYPE (TIME(scale): 1-byte scale, 1-byte len = 0|3|4|5)
        0x29 => {
            let scale = r.read_u8()?;
            let ty = format!("TIME({})", scale);
            let actual_len = r.read_u8()? as usize;
            if actual_len == 0 || r.remaining() < actual_len {
                return Ok(scalar(ty, "NULL".into()));
            }
            let b = r.read_bytes(actual_len)?;
            match decode_time_bytes(b, scale) {
                Some(s) => Ok(scalar(ty, format!("'{}'", s))),
                None => Ok(scalar(ty, "NULL".into())),
            }
        }
        // DATETIME2NTYPE (DATETIME2(scale): 1-byte scale, 1-byte len = 0|6|7|8)
        0x2A => {
            let scale = r.read_u8()?;
            let ty = format!("DATETIME2({})", scale);
            let actual_len = r.read_u8()? as usize;
            if actual_len == 0 || r.remaining() < actual_len {
                return Ok(scalar(ty, "NULL".into()));
            }
            let b = r.read_bytes(actual_len)?;
            match decode_datetime2_bytes(b, scale) {
                Some(s) => Ok(scalar(ty, format!("'{}'", s))),
                None => Ok(scalar(ty, "NULL".into())),
            }
        }
        // DATETIMEOFFSETNTYPE (DATETIMEOFFSET(scale): 1-byte scale, 1-byte len = 0|8|9|10)
        0x2B => {
            let scale = r.read_u8()?;
            let ty = format!("DATETIMEOFFSET({})", scale);
            let actual_len = r.read_u8()? as usize;
            if actual_len == 0 || r.remaining() < actual_len {
                return Ok(scalar(ty, "NULL".into()));
            }
            let b = r.read_bytes(actual_len)?;
            match decode_datetimeoffset_bytes(b, scale) {
                Some(s) => Ok(scalar(ty, format!("'{}'", s))),
                None => Ok(scalar(ty, "NULL".into())),
            }
        }
        // MONEYNTYPE (MONEY / SMALLMONEY: 1-byte max_len = 4|8, 1-byte actual_len = 0|4|8)
        0x6E => {
            let max_len = r.read_u8()?;
            let ty = if max_len == 4 {
                "SMALLMONEY".into()
            } else {
                "MONEY".into()
            };
            let actual_len = r.read_u8()? as usize;
            if actual_len == 0 || r.remaining() < actual_len {
                return Ok(scalar(ty, "NULL".into()));
            }
            let b = r.read_bytes(actual_len)?;
            match decode_money_bytes(b) {
                Some(scaled) => Ok(scalar(ty, format_decimal(scaled, 4))),
                None => Ok(scalar(ty, "NULL".into())),
            }
        }
        // DATETIMNTYPE (DATETIME / SMALLDATETIME: 1-byte max_len = 4|8, 1-byte actual_len = 0|4|8)
        0x6F => {
            let max_len = r.read_u8()?;
            let ty = if max_len == 4 {
                "SMALLDATETIME".into()
            } else {
                "DATETIME".into()
            };
            let actual_len = r.read_u8()? as usize;
            if actual_len == 0 || r.remaining() < actual_len {
                return Ok(scalar(ty, "NULL".into()));
            }
            let b = r.read_bytes(actual_len)?;
            match decode_datetime_bytes(b) {
                Some(s) => Ok(scalar(ty, format!("'{}'", s))),
                None => Ok(scalar(ty, "NULL".into())),
            }
        }
        // XMLTYPE (1-byte schema_present [+ optional 3 B_VARCHARs], then PLP UTF-16LE payload)
        0xF1 => {
            skip_xml_schema_info(r)?;
            match read_plp_bytes(r)? {
                Some(bytes) => {
                    let s = decode_utf16le(&bytes);
                    let trimmed = s.strip_prefix('\u{FEFF}').unwrap_or(&s);
                    Ok(scalar(
                        "XML".into(),
                        format!("N'{}'", trimmed.replace('\'', "''")),
                    ))
                }
                None => Ok(scalar("XML".into(), "NULL".into())),
            }
        }
        // Fixed-length integer/bit/float types
        0x30 => {
            let v = r.read_u8()?;
            Ok(scalar("TINYINT".into(), v.to_string()))
        }
        0x32 => {
            let v = r.read_u8()?;
            Ok(scalar(
                "BIT".into(),
                if v != 0 { "1".into() } else { "0".into() },
            ))
        }
        0x34 => {
            let b = r.read_bytes(2)?;
            Ok(scalar(
                "SMALLINT".into(),
                i16::from_le_bytes([b[0], b[1]]).to_string(),
            ))
        }
        0x38 => {
            let b = r.read_bytes(4)?;
            Ok(scalar(
                "INT".into(),
                i32::from_le_bytes([b[0], b[1], b[2], b[3]]).to_string(),
            ))
        }
        0x7F => {
            let b = r.read_bytes(8)?;
            Ok(scalar(
                "BIGINT".into(),
                i64::from_le_bytes(b.try_into().unwrap_or([0u8; 8])).to_string(),
            ))
        }
        // TVPTYPE
        0xF3 => read_tvp_value(r),
        _ => Ok(scalar("NVARCHAR(MAX)".into(), "NULL".into())),
    }
}

pub(super) fn read_plp_bytes(r: &mut PacketReader) -> io::Result<Option<Vec<u8>>> {
    if r.remaining() < 8 {
        return Ok(None);
    }
    let total_len = r.read_u64_le()?;
    if total_len == u64::MAX {
        return Ok(None);
    }
    let mut out = Vec::new();
    loop {
        if r.remaining() < 4 {
            break;
        }
        let chunk_len = r.read_u32_le()? as usize;
        if chunk_len == 0 {
            break;
        }
        if r.remaining() < chunk_len {
            break;
        }
        out.extend_from_slice(r.read_bytes(chunk_len)?);
    }
    Ok(Some(out))
}

fn format_guid_bytes(b: &[u8]) -> String {
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        b[3], b[2], b[1], b[0], b[5], b[4], b[7], b[6], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

fn format_hex_literal(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push_str("0x");
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(&mut s, "{:02X}", byte);
    }
    s
}

fn scalar(type_name: String, value_sql: String) -> DecodedValue {
    DecodedValue {
        type_name,
        value_sql,
        tvp_rows: None,
    }
}

fn read_tvp_value(r: &mut PacketReader) -> io::Result<DecodedValue> {
    let _db = read_b_varchar_utf16(r).unwrap_or_default();
    let schema = read_b_varchar_utf16(r).unwrap_or_else(|| "dbo".to_string());
    let type_name = read_b_varchar_utf16(r).unwrap_or_else(|| "TVP".to_string());
    let full_type = format!("{}.{}", schema, type_name);

    if r.remaining() < 2 {
        return Ok(DecodedValue {
            type_name: full_type,
            value_sql: "NULL".into(),
            tvp_rows: Some(vec![]),
        });
    }

    let col_count = r.read_u16_le()?;
    if col_count == 0xFFFF {
        return Ok(DecodedValue {
            type_name: full_type,
            value_sql: "NULL".into(),
            tvp_rows: Some(vec![]),
        });
    }

    let mut cols = Vec::new();
    for _ in 0..col_count {
        if r.remaining() < 7 {
            return Ok(DecodedValue {
                type_name: full_type,
                value_sql: "NULL".into(),
                tvp_rows: Some(vec![]),
            });
        }
        let _user_type = r.read_u32_le()?;
        let _flags = r.read_u16_le()?;
        let type_id = r.read_u8()?;
        cols.push(read_tvp_col_meta(r, type_id)?);

        if r.remaining() < 1 {
            return Ok(DecodedValue {
                type_name: full_type,
                value_sql: "NULL".into(),
                tvp_rows: Some(vec![]),
            });
        }
        let col_name_len = r.read_u8()? as usize;
        if col_name_len > 0 {
            if r.remaining() < col_name_len * 2 {
                return Ok(DecodedValue {
                    type_name: full_type,
                    value_sql: "NULL".into(),
                    tvp_rows: Some(vec![]),
                });
            }
            r.skip(col_name_len * 2)?;
        }
    }

    // Phase 1: consume optional TVP metadata tokens (TVP_ORDER_UNIQUE 0x10,
    // TVP_COLUMN_DEFAULT 0x12) until the first TVP_END_TOKEN (0x00) — or if
    // the wire stream jumps straight to TVP_ROW (0x01), start reading rows.
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut in_rows_phase = false;
    loop {
        if r.remaining() < 1 {
            break;
        }
        let token = r.read_u8()?;
        match token {
            0x00 => {
                if in_rows_phase {
                    break; // Final TVP_END_TOKEN after rows
                }
                // First TVP_END_TOKEN terminates optional metadata; rows follow
                // (or an immediate second 0x00 terminates an empty TVP).
                in_rows_phase = true;
            }
            0x01 => {
                // TVP_ROW
                in_rows_phase = true;
                let mut row = Vec::with_capacity(cols.len());
                for col in &cols {
                    row.push(read_tvp_cell_value(r, col)?);
                }
                rows.push(row);
            }
            0x10 => {
                // TVP_ORDER_UNIQUE: count(u16) * (col_num(u16) + flags(u8))
                if r.remaining() < 2 {
                    break;
                }
                let cnt = r.read_u16_le()? as usize;
                let skip = cnt * 3;
                if r.remaining() < skip {
                    break;
                }
                r.skip(skip)?;
            }
            0x12 => {
                // TVP_COLUMN_DEFAULT: count(u16) * col_num(u16)
                if r.remaining() < 2 {
                    break;
                }
                let cnt = r.read_u16_le()? as usize;
                let skip = cnt * 2;
                if r.remaining() < skip {
                    break;
                }
                r.skip(skip)?;
            }
            _ => break,
        }
    }

    Ok(DecodedValue {
        type_name: full_type,
        value_sql: "NULL".into(),
        tvp_rows: Some(rows),
    })
}

fn skip_xml_schema_info(r: &mut PacketReader) -> io::Result<()> {
    if r.remaining() < 1 {
        return Ok(());
    }
    let schema_present = r.read_u8()?;
    if schema_present != 0 {
        let _ = read_b_varchar_utf16(r);
        let _ = read_b_varchar_utf16(r);
        if r.remaining() >= 2 {
            let len = r.read_u16_le()? as usize;
            if r.remaining() >= len * 2 {
                r.skip(len * 2)?;
            }
        }
    }
    Ok(())
}

fn read_tvp_col_meta(r: &mut PacketReader, type_id: u8) -> io::Result<TvpColMeta> {
    let mut meta = TvpColMeta {
        type_id,
        max_len: 0,
        precision: 0,
        scale: 0,
    };

    match type_id {
        0x24 | 0x26 | 0x68 | 0x6D | 0x6E | 0x6F => {
            if r.remaining() >= 1 {
                meta.max_len = r.read_u8()? as u16;
            }
        }
        0x28 => {} // DATE has no extra TYPE_INFO bytes
        0x29..=0x2B => {
            if r.remaining() >= 1 {
                meta.scale = r.read_u8()?;
            }
        }
        0xA7 | 0xAF | 0xE7 | 0xEF => {
            if r.remaining() >= 2 {
                meta.max_len = r.read_u16_le()?;
            }
            if r.remaining() >= 5 {
                r.skip(5)?;
            }
        }
        0xA5 | 0xAD => {
            if r.remaining() >= 2 {
                meta.max_len = r.read_u16_le()?;
            }
        }
        0x6A | 0x6C => {
            if r.remaining() >= 1 {
                meta.max_len = r.read_u8()? as u16;
            }
            if r.remaining() >= 1 {
                meta.precision = r.read_u8()?;
            }
            if r.remaining() >= 1 {
                meta.scale = r.read_u8()?;
            }
        }
        0xF1 => {
            skip_xml_schema_info(r)?;
        }
        _ => {}
    }

    Ok(meta)
}

fn read_tvp_cell_value(r: &mut PacketReader, col: &TvpColMeta) -> io::Result<String> {
    match col.type_id {
        0x24 => {
            if r.remaining() < 1 {
                return Ok("NULL".into());
            }
            let len = r.read_u8()? as usize;
            if len < 16 || r.remaining() < len {
                return Ok("NULL".into());
            }
            let b = r.read_bytes(len)?;
            Ok(format!("'{}'", format_guid_bytes(&b[..16])))
        }
        0x26 => {
            if r.remaining() < 1 {
                return Ok("NULL".into());
            }
            let len = r.read_u8()? as usize;
            if len == 0 || r.remaining() < len {
                return Ok("NULL".into());
            }
            let bytes = r.read_bytes(len)?;
            Ok(read_le_int(bytes).to_string())
        }
        0x68 => {
            if r.remaining() < 1 {
                return Ok("NULL".into());
            }
            let len = r.read_u8()? as usize;
            if len == 0 || r.remaining() < len {
                return Ok("NULL".into());
            }
            let b = r.read_u8()?;
            Ok(if b == 0 { "0".into() } else { "1".into() })
        }
        0x6D => {
            if r.remaining() < 1 {
                return Ok("NULL".into());
            }
            let len = r.read_u8()? as usize;
            if len == 0 || r.remaining() < len {
                return Ok("NULL".into());
            }
            let bytes = r.read_bytes(len)?;
            let val = if len == 4 {
                f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64
            } else {
                f64::from_le_bytes(bytes.try_into().unwrap_or([0u8; 8]))
            };
            Ok(format!("{}", val))
        }
        0x6E => {
            if r.remaining() < 1 {
                return Ok("NULL".into());
            }
            let len = r.read_u8()? as usize;
            if len == 0 || r.remaining() < len {
                return Ok("NULL".into());
            }
            let bytes = r.read_bytes(len)?;
            Ok(decode_money_bytes(bytes)
                .map(|v| format_decimal(v, 4))
                .unwrap_or_else(|| "NULL".into()))
        }
        0x6F => {
            if r.remaining() < 1 {
                return Ok("NULL".into());
            }
            let len = r.read_u8()? as usize;
            if len == 0 || r.remaining() < len {
                return Ok("NULL".into());
            }
            let bytes = r.read_bytes(len)?;
            Ok(decode_datetime_bytes(bytes)
                .map(|s| format!("'{}'", s))
                .unwrap_or_else(|| "NULL".into()))
        }
        0x28 => {
            if r.remaining() < 1 {
                return Ok("NULL".into());
            }
            let len = r.read_u8()? as usize;
            if len == 0 || r.remaining() < len {
                return Ok("NULL".into());
            }
            let bytes = r.read_bytes(len)?;
            Ok(decode_date_bytes(bytes)
                .map(|s| format!("'{}'", s))
                .unwrap_or_else(|| "NULL".into()))
        }
        0x29 => {
            if r.remaining() < 1 {
                return Ok("NULL".into());
            }
            let len = r.read_u8()? as usize;
            if len == 0 || r.remaining() < len {
                return Ok("NULL".into());
            }
            let bytes = r.read_bytes(len)?;
            Ok(decode_time_bytes(bytes, col.scale)
                .map(|s| format!("'{}'", s))
                .unwrap_or_else(|| "NULL".into()))
        }
        0x2A => {
            if r.remaining() < 1 {
                return Ok("NULL".into());
            }
            let len = r.read_u8()? as usize;
            if len == 0 || r.remaining() < len {
                return Ok("NULL".into());
            }
            let bytes = r.read_bytes(len)?;
            Ok(decode_datetime2_bytes(bytes, col.scale)
                .map(|s| format!("'{}'", s))
                .unwrap_or_else(|| "NULL".into()))
        }
        0x2B => {
            if r.remaining() < 1 {
                return Ok("NULL".into());
            }
            let len = r.read_u8()? as usize;
            if len == 0 || r.remaining() < len {
                return Ok("NULL".into());
            }
            let bytes = r.read_bytes(len)?;
            Ok(decode_datetimeoffset_bytes(bytes, col.scale)
                .map(|s| format!("'{}'", s))
                .unwrap_or_else(|| "NULL".into()))
        }
        0xA7 | 0xAF => {
            if col.max_len == 0xFFFF {
                return match read_plp_bytes(r)? {
                    Some(bytes) => {
                        let s = String::from_utf8_lossy(&bytes).to_string();
                        Ok(format!("'{}'", s.replace('\'', "''")))
                    }
                    None => Ok("NULL".into()),
                };
            }
            if r.remaining() < 2 {
                return Ok("NULL".into());
            }
            let len = r.read_u16_le()? as usize;
            if len == 0xFFFF || r.remaining() < len {
                return Ok("NULL".into());
            }
            let bytes = r.read_bytes(len)?;
            let s = String::from_utf8_lossy(bytes).to_string();
            Ok(format!("'{}'", s.replace('\'', "''")))
        }
        0xE7 | 0xEF => {
            if col.max_len == 0xFFFF {
                return match read_plp_bytes(r)? {
                    Some(bytes) => {
                        let s = decode_utf16le(&bytes);
                        Ok(format!("N'{}'", s.replace('\'', "''")))
                    }
                    None => Ok("NULL".into()),
                };
            }
            if r.remaining() < 2 {
                return Ok("NULL".into());
            }
            let len = r.read_u16_le()? as usize;
            if len == 0xFFFF || r.remaining() < len {
                return Ok("NULL".into());
            }
            let bytes = r.read_bytes(len)?;
            let s = decode_utf16le(bytes);
            Ok(format!("N'{}'", s.replace('\'', "''")))
        }
        0xA5 | 0xAD => {
            if col.max_len == 0xFFFF {
                return match read_plp_bytes(r)? {
                    Some(bytes) => Ok(format_hex_literal(&bytes)),
                    None => Ok("NULL".into()),
                };
            }
            if r.remaining() < 2 {
                return Ok("NULL".into());
            }
            let len = r.read_u16_le()? as usize;
            if len == 0xFFFF || r.remaining() < len {
                return Ok("NULL".into());
            }
            let bytes = r.read_bytes(len)?;
            Ok(format_hex_literal(bytes))
        }
        0xF1 => match read_plp_bytes(r)? {
            Some(bytes) => {
                let s = decode_utf16le(&bytes);
                let trimmed = s.strip_prefix('\u{FEFF}').unwrap_or(&s);
                Ok(format!("N'{}'", trimmed.replace('\'', "''")))
            }
            None => Ok("NULL".into()),
        },
        0x6A | 0x6C => {
            if r.remaining() < 1 {
                return Ok("NULL".into());
            }
            let len = r.read_u8()? as usize;
            if len == 0 || r.remaining() < len {
                return Ok("NULL".into());
            }
            let bytes = r.read_bytes(len)?;
            let sign = bytes[0];
            let mut val: i128 = 0;
            for i in (1..bytes.len()).rev() {
                val = (val << 8) | bytes[i] as i128;
            }
            if sign == 0 {
                val = -val;
            }
            Ok(format_decimal(val, col.scale))
        }
        0x30 => Ok(r.read_u8()?.to_string()),
        0x32 => Ok(if r.read_u8()? == 0 {
            "0".into()
        } else {
            "1".into()
        }),
        0x34 => {
            let b = r.read_bytes(2)?;
            Ok(i16::from_le_bytes([b[0], b[1]]).to_string())
        }
        0x38 => {
            let b = r.read_bytes(4)?;
            Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]).to_string())
        }
        0x7F => {
            let b = r.read_bytes(8)?;
            Ok(i64::from_le_bytes(b.try_into().unwrap_or([0u8; 8])).to_string())
        }
        _ => Ok("NULL".into()),
    }
}

fn read_b_varchar_utf16(r: &mut PacketReader) -> Option<String> {
    if r.remaining() < 1 {
        return None;
    }
    let len = r.read_u8().ok()? as usize;
    if len == 0 {
        return Some(String::new());
    }
    if r.remaining() < len * 2 {
        return None;
    }
    let bytes = r.read_bytes(len * 2).ok()?;
    Some(decode_utf16le(bytes))
}

fn int_type_name(max_len: u8) -> String {
    match max_len {
        1 => "TINYINT".into(),
        2 => "SMALLINT".into(),
        8 => "BIGINT".into(),
        _ => "INT".into(),
    }
}

fn read_le_int(bytes: &[u8]) -> i64 {
    match bytes.len() {
        1 => bytes[0] as i64,
        2 => i16::from_le_bytes([bytes[0], bytes[1]]) as i64,
        4 => i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64,
        8 => i64::from_le_bytes(bytes.try_into().unwrap_or([0u8; 8])),
        _ => 0,
    }
}

pub(super) fn decode_utf16le(bytes: &[u8]) -> String {
    let mut u16s = Vec::with_capacity(bytes.len() / 2);
    for &chunk in bytes.as_chunks::<2>().0 {
        u16s.push(u16::from_le_bytes(chunk));
    }
    String::from_utf16_lossy(&u16s).to_string()
}

fn format_decimal(val: i128, scale: u8) -> String {
    if scale == 0 {
        return val.to_string();
    }
    let divisor = 10i128.pow(scale as u32);
    let int_part = val / divisor;
    let frac_part = (val % divisor).abs();
    format!(
        "{}.{:0>width$}",
        int_part,
        frac_part,
        width = scale as usize
    )
}
