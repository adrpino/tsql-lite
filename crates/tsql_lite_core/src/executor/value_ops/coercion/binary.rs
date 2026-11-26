use super::numeric::coerce_int;
use super::xml::{canonicalize_xml_fragment, xml_conversion_not_allowed};
use crate::error::DbError;
use crate::executor::value_helpers::pad_binary_right;
use crate::types::{DataType, Value};
use uuid::Uuid;

pub(crate) fn coerce_binary(data: &[u8], ty: &DataType) -> Result<Value, DbError> {
    match ty {
        DataType::Bit
        | DataType::TinyInt
        | DataType::SmallInt
        | DataType::Int
        | DataType::BigInt
        | DataType::Float
        | DataType::Decimal { .. }
        | DataType::Money
        | DataType::SmallMoney => {
            let i = parse_binary_to_i64(data)?;
            coerce_int(i, ty)
        }
        DataType::Binary { len } => {
            let padded = pad_binary_right(data, *len as usize);
            Ok(Value::Binary(padded))
        }
        DataType::VarBinary { .. } => Ok(Value::VarBinary(data.to_vec())),
        DataType::Char { .. } | DataType::VarChar { .. } => {
            Ok(Value::VarChar(String::from_utf8_lossy(data).into_owned()))
        }
        DataType::NChar { .. } | DataType::NVarChar { .. } => {
            Ok(Value::NVarChar(String::from_utf8_lossy(data).into_owned()))
        }
        DataType::SqlVariant => Ok(Value::SqlVariant(Box::new(Value::Binary(data.to_vec())))),
        DataType::Xml => {
            let text = if data.starts_with(&[0xFF, 0xFE]) && data.len().is_multiple_of(2) {
                let units: Vec<u16> = data[2..]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&c| u16::from_le_bytes(c))
                    .collect();
                String::from_utf16(&units)
                    .map_err(|_| DbError::Execution("invalid UTF-16 XML binary".into()))?
            } else {
                let bytes = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data);
                String::from_utf8(bytes.to_vec())
                    .map_err(|_| DbError::Execution("invalid UTF-8 XML binary".into()))?
            };
            let canonical = canonicalize_xml_fragment(&text)?;
            Ok(Value::VarChar(canonical))
        }
        DataType::UniqueIdentifier => {
            let padded = pad_binary_right(data, 16);
            let arr: [u8; 16] = padded[..16].try_into().map_err(|_| {
                DbError::Execution("cannot convert BINARY to UNIQUEIDENTIFIER".into())
            })?;
            let uuid = uuid::Uuid::from_bytes_le(arr);
            Ok(Value::UniqueIdentifier(uuid))
        }
        DataType::DateTime
        | DataType::DateTime2
        | DataType::SmallDateTime
        | DataType::DateTimeOffset
        | DataType::Date
        | DataType::Time => Err(DbError::Execution(format!(
            "cannot convert BINARY to {:?}",
            ty
        ))),
        _ => Err(DbError::Execution(format!(
            "cannot convert BINARY to {:?}",
            ty
        ))),
    }
}

pub(crate) fn parse_binary_to_i64(data: &[u8]) -> Result<i64, DbError> {
    if data.is_empty() {
        return Ok(0);
    }
    if data.len() > 8 {
        return Err(DbError::Execution(
            "cannot convert BINARY longer than 8 bytes to integer".into(),
        ));
    }

    let mut n: u64 = 0;
    for b in data {
        n = (n << 8) | (*b as u64);
    }

    // Sign-extend according to payload width (SQL Server-style binary-to-int casts).
    let bit_width = (data.len() * 8) as u32;
    if bit_width < 64 && (n & (1u64 << (bit_width - 1))) != 0 {
        let mask = (!0u64) << bit_width;
        n |= mask;
    }

    Ok(n as i64)
}

pub(crate) fn coerce_uuid_value(v: Uuid, ty: &DataType) -> Result<Value, DbError> {
    match ty {
        DataType::Char { .. } | DataType::VarChar { .. } => Ok(Value::VarChar(v.to_string())),
        DataType::NChar { .. } | DataType::NVarChar { .. } => Ok(Value::NVarChar(v.to_string())),
        DataType::UniqueIdentifier => Ok(Value::UniqueIdentifier(v)),
        DataType::SqlVariant => Ok(Value::SqlVariant(Box::new(Value::UniqueIdentifier(v)))),
        DataType::Xml => Err(xml_conversion_not_allowed("uniqueidentifier")),
        _ => Err(DbError::Execution(format!(
            "cannot convert UNIQUEIDENTIFIER to {:?}",
            ty
        ))),
    }
}
