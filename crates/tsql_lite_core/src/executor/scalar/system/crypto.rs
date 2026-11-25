use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

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
    let input_bytes: Vec<u8> = match &data_val {
        Value::Binary(b) | Value::VarBinary(b) => b.clone(),
        Value::NVarChar(s) | Value::NChar(s) => {
            s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
        }
        other => other.to_string_value().into_bytes(),
    };

    let hash_bytes = match algo.as_str() {
        "MD5" => Md5::digest(&input_bytes).to_vec(),
        "SHA" | "SHA1" | "SHA_1" => Sha1::digest(&input_bytes).to_vec(),
        "SHA2_256" | "SHA256" => Sha256::digest(&input_bytes).to_vec(),
        "SHA2_512" | "SHA512" => Sha512::digest(&input_bytes).to_vec(),
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

const PASSPHRASE_ENVELOPE_MAGIC: &[u8; 4] = b"IRPB";
const PASSPHRASE_NONCE_LEN: usize = 12;

fn derive_passphrase_key(passphrase: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"tsql-lite-passphrase-v1:");
    for u in passphrase.encode_utf16() {
        hasher.update(u.to_le_bytes());
    }
    hasher.finalize().into()
}

fn value_to_crypto_bytes(val: &Value) -> Vec<u8> {
    match val {
        Value::Binary(b) | Value::VarBinary(b) => b.clone(),
        other => other.to_string_value().into_bytes(),
    }
}

fn build_authenticator_aad(add_auth: bool, auth_bytes: Option<&[u8]>) -> Vec<u8> {
    let mut aad = Vec::new();
    if add_auth {
        aad.push(1);
        if let Some(b) = auth_bytes {
            aad.extend_from_slice(b);
        }
    } else {
        aad.push(0);
    }
    aad
}

pub(crate) fn eval_crypt_gen_random(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.is_empty() || args.len() > 2 {
        return Err(DbError::Execution(
            "CRYPT_GEN_RANDOM expects 1 or 2 arguments".into(),
        ));
    }
    let len_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if len_val.is_null() {
        return Ok(Value::Null);
    }
    let Some(len_i64) = len_val.to_integer_i64() else {
        return Err(DbError::Execution(
            "CRYPT_GEN_RANDOM length must be an integer".into(),
        ));
    };
    if !(1..=8000).contains(&len_i64) {
        return Ok(Value::Null);
    }
    let seed_bytes = if let Some(seed_arg) = args.get(1) {
        let sv = eval_expr(seed_arg, row, ctx, catalog, storage, clock)?;
        if sv.is_null() {
            Vec::new()
        } else {
            value_to_crypto_bytes(&sv)
        }
    } else {
        Vec::new()
    };

    let target_len = len_i64 as usize;
    let mut out = Vec::with_capacity(target_len);
    let mut counter: u64 = 0;
    while out.len() < target_len {
        let entropy = uuid::Uuid::new_v4();
        let mut hasher = Sha256::new();
        hasher.update(entropy.as_bytes());
        hasher.update(&seed_bytes);
        hasher.update(counter.to_le_bytes());
        let block = hasher.finalize();
        let take = (target_len - out.len()).min(block.len());
        out.extend_from_slice(&block[..take]);
        counter += 1;
    }
    Ok(Value::VarBinary(out))
}

pub(crate) fn eval_encryptbypassphrase(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() < 2 || args.len() > 4 {
        return Err(DbError::Execution(
            "ENCRYPTBYPASSPHRASE expects 2 to 4 arguments".into(),
        ));
    }
    let pass_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let plain_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if pass_val.is_null() || plain_val.is_null() {
        return Ok(Value::Null);
    }

    let add_auth = if let Some(flag_arg) = args.get(2) {
        let fv = eval_expr(flag_arg, row, ctx, catalog, storage, clock)?;
        if fv.is_null() {
            false
        } else {
            fv.to_integer_i64().unwrap_or(0) != 0
        }
    } else {
        false
    };
    let auth_bytes = if let Some(auth_arg) = args.get(3) {
        let av = eval_expr(auth_arg, row, ctx, catalog, storage, clock)?;
        if av.is_null() {
            None
        } else {
            Some(value_to_crypto_bytes(&av))
        }
    } else {
        None
    };

    let key = derive_passphrase_key(&pass_val.to_string_value());
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| DbError::Execution(format!("ENCRYPTBYPASSPHRASE key error: {e}")))?;
    let entropy = uuid::Uuid::new_v4();
    let nonce_bytes: [u8; PASSPHRASE_NONCE_LEN] = entropy.as_bytes()[..PASSPHRASE_NONCE_LEN]
        .try_into()
        .expect("12-byte nonce");
    let nonce = Nonce::from(nonce_bytes);
    let plaintext = value_to_crypto_bytes(&plain_val);
    let aad = build_authenticator_aad(add_auth, auth_bytes.as_deref());

    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| DbError::Execution("ENCRYPTBYPASSPHRASE encryption failed".into()))?;

    let mut out = Vec::with_capacity(
        PASSPHRASE_ENVELOPE_MAGIC.len() + PASSPHRASE_NONCE_LEN + ciphertext.len(),
    );
    out.extend_from_slice(PASSPHRASE_ENVELOPE_MAGIC);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(Value::VarBinary(out))
}

pub(crate) fn eval_decryptbypassphrase(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() < 2 || args.len() > 4 {
        return Err(DbError::Execution(
            "DECRYPTBYPASSPHRASE expects 2 to 4 arguments".into(),
        ));
    }
    let pass_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let cipher_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if pass_val.is_null() || cipher_val.is_null() {
        return Ok(Value::Null);
    }

    let add_auth = if let Some(flag_arg) = args.get(2) {
        let fv = eval_expr(flag_arg, row, ctx, catalog, storage, clock)?;
        if fv.is_null() {
            false
        } else {
            fv.to_integer_i64().unwrap_or(0) != 0
        }
    } else {
        false
    };
    let auth_bytes = if let Some(auth_arg) = args.get(3) {
        let av = eval_expr(auth_arg, row, ctx, catalog, storage, clock)?;
        if av.is_null() {
            None
        } else {
            Some(value_to_crypto_bytes(&av))
        }
    } else {
        None
    };

    let data = match cipher_val {
        Value::Binary(b) | Value::VarBinary(b) => b,
        _ => return Ok(Value::Null),
    };
    let header_len = PASSPHRASE_ENVELOPE_MAGIC.len() + PASSPHRASE_NONCE_LEN;
    if data.len() < header_len || &data[..4] != PASSPHRASE_ENVELOPE_MAGIC {
        return Ok(Value::Null);
    }
    let Ok(nonce) = Nonce::try_from(&data[4..header_len]) else {
        return Ok(Value::Null);
    };
    let ciphertext = &data[header_len..];

    let key = derive_passphrase_key(&pass_val.to_string_value());
    let Ok(cipher) = Aes256Gcm::new_from_slice(&key) else {
        return Ok(Value::Null);
    };
    let aad = build_authenticator_aad(add_auth, auth_bytes.as_deref());
    match cipher.decrypt(
        &nonce,
        Payload {
            msg: ciphertext,
            aad: &aad,
        },
    ) {
        Ok(plain) => Ok(Value::VarBinary(plain)),
        Err(_) => Ok(Value::Null),
    }
}
