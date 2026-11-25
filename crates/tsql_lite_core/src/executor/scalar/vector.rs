use crate::ast::Expr;
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::storage::Storage;
use crate::types::{vector_to_f32, Value};

use super::super::clock::Clock;
use super::super::context::ExecutionContext;
use super::super::evaluator::eval_expr;
use super::super::model::ContextTable;

pub(crate) fn eval_vector_distance(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 3 {
        return Err(DbError::Execution(
            "VECTOR_DISTANCE expects 3 arguments".into(),
        ));
    }

    let metric_value = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let left_value = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    let right_value = eval_expr(&args[2], row, ctx, catalog, storage, clock)?;

    if metric_value.is_null() || left_value.is_null() || right_value.is_null() {
        return Ok(Value::Null);
    }

    let metric = metric_value.to_string_value().to_ascii_lowercase();
    let left_bits = match left_value {
        Value::Vector(bits) => bits,
        other => {
            return Err(DbError::Execution(format!(
                "Argument data type {:?} is invalid for argument 2 of vector_distance function",
                other.data_type()
            )))
        }
    };
    let right_bits = match right_value {
        Value::Vector(bits) => bits,
        other => {
            return Err(DbError::Execution(format!(
                "Argument data type {:?} is invalid for argument 3 of vector_distance function",
                other.data_type()
            )))
        }
    };

    if left_bits.len() != right_bits.len() {
        return Err(DbError::Execution(
            "vector dimension mismatch in VECTOR_DISTANCE".into(),
        ));
    }

    let left = vector_to_f32(&left_bits);
    let right = vector_to_f32(&right_bits);
    let result = match metric.as_str() {
        "euclidean" => euclidean_distance(&left, &right),
        "cosine" => cosine_distance(&left, &right),
        "dot" => negative_dot_product(&left, &right),
        _ => {
            return Err(DbError::Execution(
                "invalid distance metric for VECTOR_DISTANCE".into(),
            ))
        }
    };

    Ok(Value::Float(result.to_bits()))
}

fn euclidean_distance(left: &[f32], right: &[f32]) -> f64 {
    let sum = left
        .iter()
        .zip(right.iter())
        .map(|(a, b)| {
            let diff = f64::from(*a) - f64::from(*b);
            diff * diff
        })
        .sum::<f64>();
    sum.sqrt()
}

fn cosine_distance(left: &[f32], right: &[f32]) -> f64 {
    let (dot, left_norm, right_norm) =
        left.iter()
            .zip(right.iter())
            .fold((0.0f64, 0.0f64, 0.0f64), |(dot, ln, rn), (a, b)| {
                let af = f64::from(*a);
                let bf = f64::from(*b);
                (dot + af * bf, ln + af * af, rn + bf * bf)
            });

    let denom = left_norm.sqrt() * right_norm.sqrt();
    if denom == 0.0 {
        if left == right {
            0.0
        } else {
            1.0
        }
    } else {
        1.0 - (dot / denom)
    }
}

fn negative_dot_product(left: &[f32], right: &[f32]) -> f64 {
    -left
        .iter()
        .zip(right.iter())
        .map(|(a, b)| f64::from(*a) * f64::from(*b))
        .sum::<f64>()
}

fn compute_vector_norm(values: &[f32], norm_type: &str) -> Result<f64, DbError> {
    match norm_type {
        "norm1" => Ok(values.iter().map(|v| f64::from(*v).abs()).sum()),
        "norm2" => Ok(values
            .iter()
            .map(|v| {
                let d = f64::from(*v);
                d * d
            })
            .sum::<f64>()
            .sqrt()),
        "norminf" => Ok(values
            .iter()
            .map(|v| f64::from(*v).abs())
            .fold(0.0f64, f64::max)),
        _ => Err(DbError::Execution(format!(
            "Invalid norm type '{}'. Supported: norm1, norm2, norminf",
            norm_type
        ))),
    }
}

pub(crate) fn eval_vector_norm(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution("VECTOR_NORM expects 2 arguments".into()));
    }
    let vec_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let norm_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if vec_val.is_null() || norm_val.is_null() {
        return Ok(Value::Null);
    }
    let bits = match vec_val {
        Value::Vector(b) => b,
        other => {
            return Err(DbError::Execution(format!(
                "Argument data type {:?} is invalid for argument 1 of vector_norm function",
                other.data_type()
            )))
        }
    };
    let vals = vector_to_f32(&bits);
    let norm_type = norm_val.to_string_value().trim().to_ascii_lowercase();
    let res = compute_vector_norm(&vals, &norm_type)?;
    Ok(Value::Float(res.to_bits()))
}

pub(crate) fn eval_vector_normalize(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution(
            "VECTOR_NORMALIZE expects 2 arguments".into(),
        ));
    }
    let vec_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let norm_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if vec_val.is_null() || norm_val.is_null() {
        return Ok(Value::Null);
    }
    let bits = match vec_val {
        Value::Vector(b) => b,
        other => {
            return Err(DbError::Execution(format!(
                "Argument data type {:?} is invalid for argument 1 of vector_normalize function",
                other.data_type()
            )))
        }
    };
    let vals = vector_to_f32(&bits);
    let norm_type = norm_val.to_string_value().trim().to_ascii_lowercase();
    let norm = compute_vector_norm(&vals, &norm_type)?;
    if norm == 0.0 {
        return Ok(Value::Vector(bits));
    }
    let normalized: Vec<u32> = vals
        .iter()
        .map(|v| ((f64::from(*v) / norm) as f32).to_bits())
        .collect();
    Ok(Value::Vector(normalized))
}

pub(crate) fn eval_vector_property(
    args: &[Expr],
    row: &[ContextTable],
    ctx: &mut ExecutionContext,
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
) -> Result<Value, DbError> {
    if args.len() != 2 {
        return Err(DbError::Execution(
            "VECTORPROPERTY expects 2 arguments".into(),
        ));
    }
    let vec_val = eval_expr(&args[0], row, ctx, catalog, storage, clock)?;
    let prop_val = eval_expr(&args[1], row, ctx, catalog, storage, clock)?;
    if vec_val.is_null() || prop_val.is_null() {
        return Ok(Value::Null);
    }
    let bits = match vec_val {
        Value::Vector(b) => b,
        other => {
            return Err(DbError::Execution(format!(
                "Argument data type {:?} is invalid for argument 1 of vectorproperty function",
                other.data_type()
            )))
        }
    };
    match prop_val
        .to_string_value()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "dimensions" => Ok(Value::Int(bits.len() as i32)),
        "basetype" => Ok(Value::NVarChar("float".to_string())),
        _ => Ok(Value::Null),
    }
}
