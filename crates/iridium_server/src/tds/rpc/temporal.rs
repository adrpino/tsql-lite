//! Pure wire decoders for MS-TDS temporal (`DATE`, `TIME`, `DATETIME`, `SMALLDATETIME`,
//! `DATETIME2`, `DATETIMEOFFSET`) and `MONEY`/`SMALLMONEY` payloads.

/// Civil date from days since `0001-01-01` (Gregorian proleptic calendar, Howard Hinnant algorithm).
pub(super) fn days_since_0001_to_ymd(days_0001: i64) -> (i32, u32, u32) {
    // 0001-01-01 is day 0 here; 1970-01-01 is day 719_162.
    let z = days_0001 + 306; // shift epoch to 0000-03-01 (day 0 = 0001-01-01 -> 306 days after 0000-03-01)
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = y + if m <= 2 { 1 } else { 0 };
    (year as i32, m as u32, d as u32)
}

/// Days between `0001-01-01` and `1900-01-01`.
const DAYS_0001_TO_1900: i64 = 693_595;

pub(super) fn decode_date_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 3 {
        return None;
    }
    let days = (bytes[0] as i64) | ((bytes[1] as i64) << 8) | ((bytes[2] as i64) << 16);
    let (y, m, d) = days_since_0001_to_ymd(days);
    Some(format!("{:04}-{:02}-{:02}", y, m, d))
}

pub(super) fn read_le_u64_var(bytes: &[u8]) -> u64 {
    let mut v = 0u64;
    for (i, &b) in bytes.iter().enumerate() {
        v |= (b as u64) << (i * 8);
    }
    v
}

pub(super) fn format_time_ticks(ticks: u64, scale: u8) -> String {
    let scale = scale.min(7);
    let divisor = 10u64.pow(scale as u32);
    let total_secs = ticks / divisor;
    let frac = ticks % divisor;
    let hours = (total_secs / 3600) % 24;
    let mins = (total_secs / 60) % 60;
    let secs = total_secs % 60;
    if scale == 0 {
        format!("{:02}:{:02}:{:02}", hours, mins, secs)
    } else {
        format!(
            "{:02}:{:02}:{:02}.{:0>width$}",
            hours,
            mins,
            secs,
            frac,
            width = scale as usize
        )
    }
}

pub(super) fn decode_time_bytes(bytes: &[u8], scale: u8) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let ticks = read_le_u64_var(bytes);
    Some(format_time_ticks(ticks, scale))
}

pub(super) fn decode_datetime2_bytes(bytes: &[u8], scale: u8) -> Option<String> {
    if bytes.len() < 4 {
        return None;
    }
    let time_len = bytes.len() - 3;
    let time_str = decode_time_bytes(&bytes[..time_len], scale)?;
    let date_str = decode_date_bytes(&bytes[time_len..])?;
    Some(format!("{} {}", date_str, time_str))
}

pub(super) fn decode_datetimeoffset_bytes(bytes: &[u8], scale: u8) -> Option<String> {
    if bytes.len() < 6 {
        return None;
    }
    let time_len = bytes.len() - 5;
    let ticks_utc = read_le_u64_var(&bytes[..time_len]) as i128;
    let date_bytes = &bytes[time_len..time_len + 3];
    let days_utc =
        (date_bytes[0] as i64) | ((date_bytes[1] as i64) << 8) | ((date_bytes[2] as i64) << 16);
    let offset_min = i16::from_le_bytes([bytes[time_len + 3], bytes[time_len + 4]]);

    // MS-TDS stores DATETIMEOFFSET date+time in UTC; shift by offset_min to render
    // the local wall-clock timestamp matching the ±HH:MM suffix.
    let scale = scale.min(7);
    let divisor = 10i128.pow(scale as u32);
    let ticks_per_day = 86_400i128 * divisor;
    let total_local_ticks =
        (days_utc as i128) * ticks_per_day + ticks_utc + (offset_min as i128) * 60 * divisor;
    let local_days = total_local_ticks.div_euclid(ticks_per_day) as i64;
    let local_ticks = total_local_ticks.rem_euclid(ticks_per_day) as u64;

    let (y, m, d) = days_since_0001_to_ymd(local_days);
    let time_str = format_time_ticks(local_ticks, scale);
    let sign = if offset_min < 0 { '-' } else { '+' };
    let abs_min = offset_min.unsigned_abs();
    let off_h = abs_min / 60;
    let off_m = abs_min % 60;
    Some(format!(
        "{:04}-{:02}-{:02} {} {}{:02}:{:02}",
        y, m, d, time_str, sign, off_h, off_m
    ))
}

pub(super) fn decode_datetime_bytes(bytes: &[u8]) -> Option<String> {
    match bytes.len() {
        4 => {
            // SMALLDATETIME: u16 days since 1900-01-01, u16 minutes since midnight
            let days_1900 = u16::from_le_bytes([bytes[0], bytes[1]]) as i64;
            let mins = u16::from_le_bytes([bytes[2], bytes[3]]) as u32;
            let (y, m, d) = days_since_0001_to_ymd(DAYS_0001_TO_1900 + days_1900);
            Some(format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:00",
                y,
                m,
                d,
                mins / 60,
                mins % 60
            ))
        }
        8 => {
            // DATETIME: i32 days since 1900-01-01, u32 1/300th-second ticks since midnight
            let days_1900 = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64;
            let ticks_300 = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as u64;
            let total_ms = (ticks_300 * 1000 + 150) / 300;
            let total_secs = total_ms / 1000;
            let ms = total_ms % 1000;
            let hours = (total_secs / 3600) % 24;
            let mins = (total_secs / 60) % 60;
            let secs = total_secs % 60;
            let (y, m, d) = days_since_0001_to_ymd(DAYS_0001_TO_1900 + days_1900);
            Some(format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
                y, m, d, hours, mins, secs, ms
            ))
        }
        _ => None,
    }
}

pub(super) fn decode_money_bytes(bytes: &[u8]) -> Option<i128> {
    match bytes.len() {
        4 => Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i128),
        8 => {
            // MS-TDS MONEY stores high 32 bits first, then low 32 bits (both LE).
            let high = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64;
            let low = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as u64;
            let combined = ((high as i128) << 32) | (low as i128);
            Some(combined)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_and_sample_dates_match_gregorian() {
        assert_eq!(days_since_0001_to_ymd(0), (1, 1, 1));
        assert_eq!(days_since_0001_to_ymd(DAYS_0001_TO_1900), (1900, 1, 1));
        assert_eq!(days_since_0001_to_ymd(719_162), (1970, 1, 1));
    }

    #[test]
    fn money_8byte_high_low_order() {
        // 12345.67 => 123_456_700 = high 0, low 123_456_700
        let mut buf = [0u8; 8];
        buf[0..4].copy_from_slice(&0i32.to_le_bytes());
        buf[4..8].copy_from_slice(&123_456_700u32.to_le_bytes());
        assert_eq!(decode_money_bytes(&buf), Some(123_456_700));
    }
}
