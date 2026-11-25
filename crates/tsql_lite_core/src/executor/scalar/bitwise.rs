use crate::ast::Expr;
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::storage::Storage;
use crate::types::Value;

use crate::executor::clock::Clock;
use crate::executor::context::ExecutionContext;
use crate::executor::evaluator::eval_expr;
use crate::executor::model::ContextTable;

fn eval_shift_binary(bytes: &[u8], shift: usize, left: bool) -> Vec<u8> {
    let total_bits = bytes.len() * 8;
    if shift >= total_bits {
        return vec![0u8; bytes.len()];
    }
    let byte_shift = shift / 8;
    let bit_shift = shift % 8;
    let mut out = vec![0u8; bytes.len()];
    if left {
        for (i, slot) in out.iter_mut().enumerate() {
            let src = i + byte_shift;
            if src < bytes.len() {
                let mut val = bytes[src] << bit_shift;
                if bit_shift > 0 && src + 1 < bytes.len() {
                    val |= bytes[src + 1] >> (8 - bit_shift);
                }
                *slot = val;
            }
        }
    } else {
        for (i, slot) in out.iter_mut().enumerate() {
            if i >= byte_shift {
                let src = i - byte_shift;
                let mut val = bytes[src] >> bit_shift;
                if bit_shift > 0 && src > 0 {
                    val |= bytes[src - 1] << (8 - bit_shift);
                }
                *slot = val;
            }
        }
    }
    out
}

pub(crate) fn eval_left_shift(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    eval_shift_common(args, true, row, ctx, catalog, storage, clock)
}

pub(crate) fn eval_right_shift(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    eval_shift_common(args, false, row, ctx, catalog, storage, clock)
}

#[allow(clippy::too_many_arguments)]
fn eval_shift_common(
    args: &[Expr],
    left: bool,
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    let fname = if left { "LEFT_SHIFT" } else { "RIGHT_SHIFT" };
    if args.len() != 2 {
        return Err(DbError::Execution(format!("{} expects 2 arguments", fname)));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let amt = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if val.is_null() || amt.is_null() {
        return Ok(Value::Null);
    }
    let shift = amt
        .to_integer_i64()
        .ok_or_else(|| DbError::Execution(format!("{} shift amount must be an integer", fname)))?;
    if shift < 0 {
        return Err(DbError::Execution(format!(
            "Invalid shift amount for {}: must be non-negative",
            fname
        )));
    }
    let s = shift as u32;
    match val {
        Value::TinyInt(v) => {
            let res = if s >= 8 {
                0
            } else if left {
                v.wrapping_shl(s)
            } else {
                v.wrapping_shr(s)
            };
            Ok(Value::TinyInt(res))
        }
        Value::SmallInt(v) => {
            let u = v as u16;
            let res = if s >= 16 {
                0
            } else if left {
                u.wrapping_shl(s)
            } else {
                u.wrapping_shr(s)
            };
            Ok(Value::SmallInt(res as i16))
        }
        Value::Int(v) => {
            let u = v as u32;
            let res = if s >= 32 {
                0
            } else if left {
                u.wrapping_shl(s)
            } else {
                u.wrapping_shr(s)
            };
            Ok(Value::Int(res as i32))
        }
        Value::BigInt(v) => {
            let u = v as u64;
            let res = if s >= 64 {
                0
            } else if left {
                u.wrapping_shl(s)
            } else {
                u.wrapping_shr(s)
            };
            Ok(Value::BigInt(res as i64))
        }
        Value::Binary(b) => Ok(Value::Binary(eval_shift_binary(&b, shift as usize, left))),
        Value::VarBinary(b) => Ok(Value::VarBinary(eval_shift_binary(
            &b,
            shift as usize,
            left,
        ))),
        _ => Err(DbError::Execution(format!(
            "{} requires an integer or binary expression",
            fname
        ))),
    }
}

pub(crate) fn eval_bit_count(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 1 {
        return Err(DbError::Execution("BIT_COUNT expects 1 argument".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    if val.is_null() {
        return Ok(Value::Null);
    }
    let count: u64 = match val {
        Value::Bit(b) => u64::from(b),
        Value::TinyInt(v) => v.count_ones() as u64,
        Value::SmallInt(v) => (v as u16).count_ones() as u64,
        Value::Int(v) => (v as u32).count_ones() as u64,
        Value::BigInt(v) => (v as u64).count_ones() as u64,
        Value::Binary(b) | Value::VarBinary(b) => {
            b.iter().map(|byte| byte.count_ones() as u64).sum()
        }
        _ => {
            return Err(DbError::Execution(
                "BIT_COUNT requires an integer, bit, or binary expression".into(),
            ))
        }
    };
    Ok(Value::BigInt(count as i64))
}

pub(crate) fn eval_get_bit(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution("GET_BIT expects 2 arguments".into()));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let offset_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if val.is_null() || offset_val.is_null() {
        return Ok(Value::Null);
    }
    let offset = offset_val
        .to_integer_i64()
        .ok_or_else(|| DbError::Execution("GET_BIT bit_offset must be an integer".into()))?;
    if offset < 0 {
        return Err(DbError::Execution(
            "Invalid bit_offset for GET_BIT: must be non-negative".into(),
        ));
    }
    let bit = match val {
        Value::Bit(b) => offset == 0 && b,
        Value::TinyInt(v) => offset < 8 && ((v >> offset) & 1) != 0,
        Value::SmallInt(v) => offset < 16 && (((v as u16) >> offset) & 1) != 0,
        Value::Int(v) => offset < 32 && (((v as u32) >> offset) & 1) != 0,
        Value::BigInt(v) => offset < 64 && (((v as u64) >> offset) & 1) != 0,
        Value::Binary(b) | Value::VarBinary(b) => {
            let byte_from_right = (offset / 8) as usize;
            let bit_in_byte = (offset % 8) as u32;
            if byte_from_right >= b.len() {
                false
            } else {
                let idx = b.len() - 1 - byte_from_right;
                ((b[idx] >> bit_in_byte) & 1) != 0
            }
        }
        _ => {
            return Err(DbError::Execution(
                "GET_BIT requires an integer, bit, or binary expression".into(),
            ))
        }
    };
    Ok(Value::Bit(bit))
}

pub(crate) fn eval_set_bit(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() < 2 || args.len() > 3 {
        return Err(DbError::Execution(
            "SET_BIT expects 2 or 3 arguments".into(),
        ));
    }
    let val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let offset_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    let bit_val = if args.len() == 3 {
        let bv = eval_expr(&args[2], row, ctx, catalog, storage, clock)?;
        if bv.is_null() {
            return Ok(Value::Null);
        }
        match bv {
            Value::Bit(b) => b,
            _ => bv.to_integer_i64().unwrap_or(0) != 0,
        }
    } else {
        true
    };
    if val.is_null() || offset_val.is_null() {
        return Ok(Value::Null);
    }
    let offset = offset_val
        .to_integer_i64()
        .ok_or_else(|| DbError::Execution("SET_BIT bit_offset must be an integer".into()))?;
    if offset < 0 {
        return Err(DbError::Execution(
            "Invalid bit_offset for SET_BIT: must be non-negative".into(),
        ));
    }
    match val {
        Value::TinyInt(v) => {
            if offset >= 8 {
                return Err(DbError::Execution(
                    "SET_BIT bit_offset out of range for tinyint".into(),
                ));
            }
            let mask = 1u8 << offset;
            Ok(Value::TinyInt(if bit_val { v | mask } else { v & !mask }))
        }
        Value::SmallInt(v) => {
            if offset >= 16 {
                return Err(DbError::Execution(
                    "SET_BIT bit_offset out of range for smallint".into(),
                ));
            }
            let mask = 1u16 << offset;
            let u = v as u16;
            Ok(Value::SmallInt(
                (if bit_val { u | mask } else { u & !mask }) as i16,
            ))
        }
        Value::Int(v) => {
            if offset >= 32 {
                return Err(DbError::Execution(
                    "SET_BIT bit_offset out of range for int".into(),
                ));
            }
            let mask = 1u32 << offset;
            let u = v as u32;
            Ok(Value::Int(
                (if bit_val { u | mask } else { u & !mask }) as i32,
            ))
        }
        Value::BigInt(v) => {
            if offset >= 64 {
                return Err(DbError::Execution(
                    "SET_BIT bit_offset out of range for bigint".into(),
                ));
            }
            let mask = 1u64 << offset;
            let u = v as u64;
            Ok(Value::BigInt(
                (if bit_val { u | mask } else { u & !mask }) as i64,
            ))
        }
        Value::Binary(mut b) => {
            let byte_from_right = (offset / 8) as usize;
            let bit_in_byte = (offset % 8) as u32;
            if byte_from_right >= b.len() {
                return Err(DbError::Execution(
                    "SET_BIT bit_offset out of range for binary".into(),
                ));
            }
            let idx = b.len() - 1 - byte_from_right;
            let mask = 1u8 << bit_in_byte;
            if bit_val {
                b[idx] |= mask;
            } else {
                b[idx] &= !mask;
            }
            Ok(Value::Binary(b))
        }
        Value::VarBinary(mut b) => {
            let byte_from_right = (offset / 8) as usize;
            let bit_in_byte = (offset % 8) as u32;
            if byte_from_right >= b.len() {
                return Err(DbError::Execution(
                    "SET_BIT bit_offset out of range for varbinary".into(),
                ));
            }
            let idx = b.len() - 1 - byte_from_right;
            let mask = 1u8 << bit_in_byte;
            if bit_val {
                b[idx] |= mask;
            } else {
                b[idx] &= !mask;
            }
            Ok(Value::VarBinary(b))
        }
        _ => Err(DbError::Execution(
            "SET_BIT requires an integer or binary expression".into(),
        )),
    }
}
