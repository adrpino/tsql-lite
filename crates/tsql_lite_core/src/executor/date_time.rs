use crate::error::DbError;
use chrono::{Datelike, NaiveDate, NaiveDateTime};

/// Typed enum for date parts, replacing string-based dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatePart {
    Year,
    Quarter,
    Month,
    Week,
    Day,
    Hour,
    Minute,
    Second,
    Millisecond,
}

impl DatePart {
    /// Parse a date part string (case-insensitive) into a typed enum.
    pub fn from_str(s: &str) -> Result<Self, DbError> {
        match s.to_lowercase().as_str() {
            "year" | "yy" | "yyyy" => Ok(DatePart::Year),
            "quarter" | "qq" | "q" => Ok(DatePart::Quarter),
            "month" | "mm" | "m" => Ok(DatePart::Month),
            "week" | "wk" | "ww" => Ok(DatePart::Week),
            "day" | "dd" | "d" | "dayofyear" | "dy" | "y" | "weekday" | "dw" | "w" => {
                Ok(DatePart::Day)
            }
            "hour" | "hh" => Ok(DatePart::Hour),
            "minute" | "mi" | "n" => Ok(DatePart::Minute),
            "second" | "ss" | "s" => Ok(DatePart::Second),
            "millisecond" | "ms" => Ok(DatePart::Millisecond),
            _ => Err(DbError::Execution(format!("unknown datepart '{}'", s))),
        }
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|d| d.pred_opt())
        .map(|d| d.day())
        .unwrap_or(28)
}

fn add_months_clamped(dt: NaiveDateTime, months: i64) -> Option<NaiveDateTime> {
    let total_months = dt.year() as i64 * 12 + (dt.month() as i64 - 1) + months;
    let new_year = total_months.div_euclid(12) as i32;
    let new_month = (total_months.rem_euclid(12) + 1) as u32;
    let clamped_day = dt.day().min(days_in_month(new_year, new_month));
    NaiveDate::from_ymd_opt(new_year, new_month, clamped_day).map(|d| d.and_time(dt.time()))
}

pub(crate) fn parse_datetime_parts(
    s: &str,
    dateformat: &str,
) -> Result<(i32, i32, i32, i32, i32, i32), DbError> {
    let s = s.trim();
    let (date_part, time_part) = split_date_time(s);
    let (y, m, d) = parse_date_part(date_part, dateformat)?;
    let (h, mi, sec) = parse_time_part(time_part)?;
    Ok((y, m, d, h, mi, sec))
}

pub(crate) fn date_to_days(y: i32, m: i32, d: i32) -> i64 {
    let (y_adj, m_adj) = if m <= 2 { (y - 1, m + 12) } else { (y, m) };
    let era = y_adj as i64 / 400;
    let yoe = y_adj as i64 - era * 400;
    let doy = (153 * (m_adj as i64 - 3) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub(crate) fn day_of_week_from_date(y: i32, m: i32, d: i32) -> i32 {
    let days = date_to_days(y, m, d);
    (((days + 719471) % 7 + 7) % 7) as i32
}

pub(crate) fn apply_dateadd(
    part: &str,
    num: i64,
    date_str: &str,
    dateformat: &str,
) -> Result<NaiveDateTime, DbError> {
    let date_part = DatePart::from_str(part)?;
    let (y, m, d, h, mi, s) = parse_datetime_parts(date_str, dateformat)?;
    let date = NaiveDate::from_ymd_opt(y, m as u32, d as u32)
        .ok_or_else(|| DbError::Execution(format!("invalid datetime format: '{}'", date_str)))?;
    let dt = date
        .and_hms_opt(h as u32, mi as u32, s as u32)
        .ok_or_else(|| DbError::Execution(format!("invalid datetime format: '{}'", date_str)))?;

    let result = match date_part {
        DatePart::Year => add_months_clamped(dt, num * 12)
            .ok_or_else(|| DbError::Execution("invalid year after DATEADD".into()))?,
        DatePart::Quarter => add_months_clamped(dt, num * 3)
            .ok_or_else(|| DbError::Execution("invalid quarter after DATEADD".into()))?,
        DatePart::Month => add_months_clamped(dt, num)
            .ok_or_else(|| DbError::Execution("invalid month after DATEADD".into()))?,
        DatePart::Week => dt + chrono::Duration::weeks(num),
        DatePart::Day => dt + chrono::Duration::days(num),
        DatePart::Hour => dt + chrono::Duration::hours(num),
        DatePart::Minute => dt + chrono::Duration::minutes(num),
        DatePart::Second => dt + chrono::Duration::seconds(num),
        DatePart::Millisecond => dt + chrono::Duration::milliseconds(num),
    };

    Ok(result)
}

fn split_date_time(s: &str) -> (&str, &str) {
    if let Some(pos) = s.find('T') {
        (&s[..pos], s[pos + 1..].trim())
    } else if let Some(pos) = s.find(' ') {
        (&s[..pos], s[pos + 1..].trim())
    } else {
        (s, "")
    }
}

fn parse_date_part(date_part: &str, dateformat: &str) -> Result<(i32, i32, i32), DbError> {
    let cleaned = date_part.trim();
    // SQL Server converts empty/whitespace strings to the 1900-01-01 DATETIME epoch.
    if cleaned.is_empty() {
        return Ok((1900, 1, 1));
    }
    // SQL Server allows integer day offsets relative to 1900-01-01 (e.g., DATEADD/DATEDIFF with 0).
    if let Ok(days) = cleaned.parse::<i64>() {
        if let Some(epoch) = NaiveDate::from_ymd_opt(1900, 1, 1) {
            let shifted = epoch + chrono::Duration::days(days);
            return Ok((shifted.year(), shifted.month() as i32, shifted.day() as i32));
        }
    }
    let segments: Vec<&str> = cleaned
        .split(['-', '/', '.'])
        .filter(|s| !s.is_empty())
        .collect();
    if segments.len() < 3 {
        return Err(DbError::Execution(format!(
            "invalid datetime format: '{}'",
            date_part
        )));
    }

    if segments[0].len() == 4 {
        let y = parse_i32(segments[0], "year", date_part)?;
        let m = parse_i32(segments[1], "month", date_part)?;
        let d = parse_i32(segments[2], "day", date_part)?;
        return Ok((y, m, d));
    }

    let fmt = dateformat.to_ascii_lowercase();
    let (y, m, d) = match fmt.as_str() {
        "dmy" => (
            parse_i32(segments[2], "year", date_part)?,
            parse_i32(segments[1], "month", date_part)?,
            parse_i32(segments[0], "day", date_part)?,
        ),
        "ydm" => (
            parse_i32(segments[0], "year", date_part)?,
            parse_i32(segments[2], "month", date_part)?,
            parse_i32(segments[1], "day", date_part)?,
        ),
        "myd" => (
            parse_i32(segments[2], "year", date_part)?,
            parse_i32(segments[0], "month", date_part)?,
            parse_i32(segments[1], "day", date_part)?,
        ),
        "dym" => (
            parse_i32(segments[1], "year", date_part)?,
            parse_i32(segments[2], "month", date_part)?,
            parse_i32(segments[0], "day", date_part)?,
        ),
        _ => (
            parse_i32(segments[2], "year", date_part)?,
            parse_i32(segments[0], "month", date_part)?,
            parse_i32(segments[1], "day", date_part)?,
        ),
    };
    Ok((y, m, d))
}

fn parse_time_part(time_part: &str) -> Result<(i32, i32, i32), DbError> {
    if time_part.is_empty() {
        return Ok((0, 0, 0));
    }
    let time_segments: Vec<&str> = time_part.split(':').collect();
    let h = time_segments
        .first()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mi = time_segments
        .get(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let sec = time_segments
        .get(2)
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0) as i32;
    Ok((h, mi, sec))
}

fn parse_i32(segment: &str, label: &str, source: &str) -> Result<i32, DbError> {
    segment
        .parse::<i32>()
        .map_err(|_| DbError::Execution(format!("invalid {} in '{}'", label, source)))
}
