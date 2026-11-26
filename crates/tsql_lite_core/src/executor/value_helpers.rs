use crate::error::DbError;
use crate::types::Value;

pub(crate) fn is_string_type(v: &Value) -> bool {
    matches!(
        v,
        Value::Char(_) | Value::VarChar(_) | Value::NChar(_) | Value::NVarChar(_)
    )
}

pub(crate) fn to_i64(v: &Value) -> Result<i64, DbError> {
    v.to_integer_i64()
        .ok_or_else(|| DbError::Execution(format!("cannot convert {:?} to integer", v.data_type())))
}

pub(crate) fn to_decimal_parts(v: &Value) -> (i128, u8) {
    v.to_decimal_parts()
}

pub(crate) fn value_to_f64(v: &Value) -> Result<f64, DbError> {
    if let Some(f) = v.to_f64() {
        return Ok(f);
    }
    match v {
        Value::DateTime(dt) | Value::DateTime2(dt) | Value::SmallDateTime(dt) => {
            let epoch = chrono::NaiveDate::from_ymd_opt(1900, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap();
            Ok((*dt - epoch).num_milliseconds() as f64 / 86_400_000.0)
        }
        Value::Date(d) => {
            let epoch = chrono::NaiveDate::from_ymd_opt(1900, 1, 1).unwrap();
            Ok((*d - epoch).num_days() as f64)
        }
        _ => Err(DbError::Execution(format!(
            "cannot convert {:?} to float",
            v.data_type()
        ))),
    }
}

pub(crate) fn rescale_raw(raw: i128, from_scale: u8, to_scale: u8) -> i128 {
    if from_scale == to_scale {
        return raw;
    }
    if to_scale > from_scale {
        raw * 10i128.pow((to_scale - from_scale) as u32)
    } else {
        let divisor = 10i128.pow((from_scale - to_scale) as u32);
        let half = divisor / 2;
        if raw >= 0 {
            (raw + half) / divisor
        } else {
            (raw - half) / divisor
        }
    }
}

pub(crate) fn pad_right(s: &str, len: usize) -> String {
    if s.len() >= len {
        s[..len].to_string()
    } else {
        format!("{:width$}", s, width = len)
    }
}

pub(crate) fn pad_binary_right(data: &[u8], len: usize) -> Vec<u8> {
    if data.len() >= len {
        data[..len].to_vec()
    } else {
        let mut v = data.to_vec();
        v.resize(len, 0);
        v
    }
}
