use crate::ast::Expr;
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::storage::Storage;
use crate::types::Value;

use super::super::super::clock::Clock;
use super::super::super::context::ExecutionContext;
use super::super::super::evaluator::eval_expr;
use super::super::super::model::ContextTable;

pub(crate) fn eval_hashbytes(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution("HASHBYTES expects 2 arguments".into()));
    }
    let algo_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let data_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;

    if algo_val.is_null() || data_val.is_null() {
        return Ok(Value::Null);
    }

    let algo = algo_val.to_string_value().to_uppercase();
    let data = data_val.to_string_value();

    let hash_bytes = match algo.as_str() {
        "MD5" => {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            data.hash(&mut hasher);
            let h = hasher.finish();
            let mut bytes = h.to_be_bytes().to_vec();
            bytes.resize(16, 0);
            bytes
        }
        "SHA1" | "SHA_1" => {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher1 = DefaultHasher::new();
            let mut hasher2 = DefaultHasher::new();
            data.hash(&mut hasher1);
            data.len().hash(&mut hasher2);
            let mut bytes = Vec::with_capacity(20);
            bytes.extend_from_slice(&hasher1.finish().to_be_bytes());
            bytes.extend_from_slice(&hasher2.finish().to_be_bytes());
            bytes.extend_from_slice(&[0u8; 4]);
            bytes
        }
        "SHA2_256" | "SHA256" => {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut bytes = Vec::with_capacity(32);
            for i in 0..4 {
                let mut hasher = DefaultHasher::new();
                data.hash(&mut hasher);
                i.hash(&mut hasher);
                bytes.extend_from_slice(&hasher.finish().to_be_bytes());
            }
            bytes
        }
        "SHA2_512" | "SHA512" => {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut bytes = Vec::with_capacity(64);
            for i in 0..8 {
                let mut hasher = DefaultHasher::new();
                data.hash(&mut hasher);
                i.hash(&mut hasher);
                bytes.extend_from_slice(&hasher.finish().to_be_bytes());
            }
            bytes
        }
        _ => {
            return Err(DbError::Execution(format!(
                "Unsupported hash algorithm '{}'. Supported: MD5, SHA1, SHA2_256, SHA2_512",
                algo
            )))
        }
    };

    Ok(Value::VarBinary(hash_bytes))
}

pub(crate) fn eval_compress(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    if args.len() != 1 {
        return Err(DbError::Execution("COMPRESS expects 1 argument".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let raw_bytes: Vec<u8> = match val {
        Value::Binary(b) | Value::VarBinary(b) => b,
        other => other.to_string_value().into_bytes(),
    };
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&raw_bytes)
        .map_err(|e| DbError::Execution(format!("COMPRESS failed: {}", e)))?;
    let compressed = encoder
        .finish()
        .map_err(|e| DbError::Execution(format!("COMPRESS failed: {}", e)))?;
    Ok(Value::VarBinary(compressed))
}

pub(crate) fn eval_decompress(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    if args.len() != 1 {
        return Err(DbError::Execution("DECOMPRESS expects 1 argument".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let compressed = match val {
        Value::Binary(b) | Value::VarBinary(b) => b,
        _ => {
            return Err(DbError::Execution(
                "DECOMPRESS expects a VARBINARY/BINARY argument".into(),
            ))
        }
    };
    let mut decoder = GzDecoder::new(compressed.as_slice());
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| DbError::Execution(format!("DECOMPRESS failed: {}", e)))?;
    Ok(Value::VarBinary(out))
}

fn fnv1a_32_update(mut h: u32, bytes: &[u8]) -> u32 {
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

fn hash_value_binary_checksum(h: u32, val: &Value) -> u32 {
    let tag: u8 = match val {
        Value::Null => 0,
        Value::Bit(_) => 1,
        Value::TinyInt(_) => 2,
        Value::SmallInt(_) => 3,
        Value::Int(_) => 4,
        Value::BigInt(_) => 5,
        Value::Float(_) => 6,
        Value::Decimal(_, _) => 7,
        Value::Money(_) => 8,
        Value::SmallMoney(_) => 9,
        Value::Char(_) | Value::VarChar(_) => 10,
        Value::NChar(_) | Value::NVarChar(_) => 11,
        Value::Binary(_) | Value::VarBinary(_) => 12,
        _ => 13,
    };
    let h = fnv1a_32_update(h, &[tag ^ 0xA5]);
    match val {
        Value::Null => fnv1a_32_update(h, &[0]),
        Value::Bit(b) => fnv1a_32_update(h, &[u8::from(*b)]),
        Value::TinyInt(n) => fnv1a_32_update(h, &[*n]),
        Value::SmallInt(n) => fnv1a_32_update(h, &n.to_le_bytes()),
        Value::Int(n) => fnv1a_32_update(h, &n.to_le_bytes()),
        Value::BigInt(n) => fnv1a_32_update(h, &n.to_le_bytes()),
        Value::Float(bits) => fnv1a_32_update(h, &bits.to_le_bytes()),
        Value::Decimal(u, s) => {
            let h = fnv1a_32_update(h, &u.to_le_bytes());
            fnv1a_32_update(h, &[*s])
        }
        Value::Money(u) => fnv1a_32_update(h, &u.to_le_bytes()),
        Value::SmallMoney(u) => fnv1a_32_update(h, &u.to_le_bytes()),
        Value::Char(s) | Value::VarChar(s) | Value::NChar(s) | Value::NVarChar(s) => {
            // Case-sensitive byte representation (unlike CHECKSUM)
            let h = fnv1a_32_update(h, &(s.len() as u32).to_le_bytes());
            fnv1a_32_update(h, s.as_bytes())
        }
        Value::Binary(b) | Value::VarBinary(b) => {
            let h = fnv1a_32_update(h, &(b.len() as u32).to_le_bytes());
            fnv1a_32_update(h, b)
        }
        other => {
            let s = other.to_string_value();
            fnv1a_32_update(h, s.as_bytes())
        }
    }
}

pub(crate) fn eval_binary_checksum(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.is_empty() {
        return Err(DbError::Execution(
            "BINARY_CHECKSUM expects at least 1 argument".into(),
        ));
    }
    let mut h: u32 = 2166136261;
    if args.len() == 1 && matches!(args[0], Expr::Wildcard) {
        for tbl in row {
            if let Some(r) = &tbl.row {
                for val in &r.values {
                    h = hash_value_binary_checksum(h, val);
                }
            }
        }
    } else {
        for arg in args {
            let val = eval_expr(arg, row, ctx, catalog, storage, clock)?;
            h = hash_value_binary_checksum(h, &val);
        }
    }
    Ok(Value::Int(h as i32))
}
