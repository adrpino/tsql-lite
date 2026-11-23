use std::sync::atomic::{AtomicU64, Ordering};

use iridium_core::executor::result::QueryResult;
use iridium_core::types::Value;
use iridium_core::{parse_batch, Engine};
use tiberius::{Client, ColumnData, Config};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

#[derive(Debug, Clone)]
pub enum DiffStep {
    /// Execute statement(s) on both engines; both must succeed.
    ExecOk {
        label: &'static str,
        sql: &'static str,
    },
    /// Execute statement(s) on both engines; both must return an error.
    ExpectErr {
        label: &'static str,
        sql: &'static str,
    },
    /// Execute query on both engines; both must succeed with matching rows.
    Query {
        label: &'static str,
        sql: &'static str,
        ordered: bool,
    },
}

#[derive(Debug, Clone)]
pub struct DiffSuite {
    pub name: &'static str,
    pub category: &'static str,
    /// Minimum SQL Server major version required (15 = 2019, 16 = 2022, 17 = 2025).
    pub min_sql_version: u32,
    pub setup_batches: &'static [&'static str],
    pub steps: &'static [DiffStep],
}

#[derive(Debug, Clone)]
pub enum DiffValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Decimal(String),
    Str(String),
    Bytes(Vec<u8>),
    Date(String),
    Time(String),
    DateTime(String),
    Uuid(String),
}

impl DiffValue {
    fn normalize_decimal_str(raw: &str) -> String {
        if let Some((int_p, frac_p)) = raw.split_once('.') {
            let trimmed = frac_p.trim_end_matches('0');
            if trimmed.is_empty() {
                int_p.to_string()
            } else {
                format!("{}.{}", int_p, trimmed)
            }
        } else {
            raw.to_string()
        }
    }

    fn normalize_time_str(raw: &str) -> String {
        Self::normalize_decimal_str(raw.trim())
    }

    fn normalize_datetime_str(raw: &str) -> String {
        let s = raw.trim().replace('T', " ");
        if let Some((prefix, rest)) = s.split_once('.') {
            // Preserve timezone suffix if present after fractional seconds
            if let Some(space_idx) = rest.find(' ') {
                let frac = rest[..space_idx].trim_end_matches('0');
                let tz = &rest[space_idx..];
                if frac.is_empty() {
                    format!("{}{}", prefix, tz)
                } else {
                    format!("{}.{}{}", prefix, frac, tz)
                }
            } else {
                let frac = rest.trim_end_matches('0');
                if frac.is_empty() {
                    prefix.to_string()
                } else {
                    format!("{}.{}", prefix, frac)
                }
            }
        } else {
            s
        }
    }

    pub fn from_iridium(val: &Value) -> Self {
        match val {
            Value::Null => DiffValue::Null,
            Value::Bit(b) => DiffValue::Bool(*b),
            Value::TinyInt(n) => DiffValue::Int(*n as i64),
            Value::SmallInt(n) => DiffValue::Int(*n as i64),
            Value::Int(n) => DiffValue::Int(*n as i64),
            Value::BigInt(n) => DiffValue::Int(*n),
            Value::Float(bits) => DiffValue::Float(f64::from_bits(*bits)),
            Value::Decimal(raw, scale) => DiffValue::Decimal(Self::normalize_decimal_str(
                &iridium_core::types::format_decimal(*raw, *scale),
            )),
            Value::Money(raw) => DiffValue::Decimal(Self::normalize_decimal_str(
                &iridium_core::types::format_decimal(*raw, 4),
            )),
            Value::SmallMoney(raw) => DiffValue::Decimal(Self::normalize_decimal_str(
                &iridium_core::types::format_decimal(*raw as i128, 4),
            )),
            Value::Char(s) | Value::VarChar(s) | Value::NChar(s) | Value::NVarChar(s) => {
                DiffValue::Str(s.clone())
            }
            Value::Binary(b) | Value::VarBinary(b) => DiffValue::Bytes(b.clone()),
            Value::Date(d) => DiffValue::Date(d.format("%Y-%m-%d").to_string()),
            Value::Time(t) => DiffValue::Time(Self::normalize_time_str(
                &t.format("%H:%M:%S%.f").to_string(),
            )),
            Value::DateTime(dt) | Value::DateTime2(dt) | Value::SmallDateTime(dt) => {
                DiffValue::DateTime(Self::normalize_datetime_str(
                    &dt.format("%Y-%m-%d %H:%M:%S%.f").to_string(),
                ))
            }
            Value::DateTimeOffset(s) => DiffValue::DateTime(Self::normalize_datetime_str(s)),
            Value::UniqueIdentifier(u) => DiffValue::Uuid(u.to_string().to_ascii_lowercase()),
            Value::SqlVariant(inner) => Self::from_iridium(inner),
            other => DiffValue::Str(other.to_string_value()),
        }
    }

    pub fn from_tiberius(col: &ColumnData<'_>) -> Self {
        match col {
            ColumnData::U8(Some(n)) => DiffValue::Int(*n as i64),
            ColumnData::I16(Some(n)) => DiffValue::Int(*n as i64),
            ColumnData::I32(Some(n)) => DiffValue::Int(*n as i64),
            ColumnData::I64(Some(n)) => DiffValue::Int(*n),
            ColumnData::F32(Some(n)) => DiffValue::Float(*n as f64),
            ColumnData::F64(Some(n)) => DiffValue::Float(*n),
            ColumnData::Bit(Some(b)) => DiffValue::Bool(*b),
            ColumnData::String(Some(s)) => DiffValue::Str(s.to_string()),
            ColumnData::Guid(Some(u)) => DiffValue::Uuid(u.to_string().to_ascii_lowercase()),
            ColumnData::Binary(Some(b)) => DiffValue::Bytes(b.to_vec()),
            ColumnData::Numeric(Some(num)) => {
                DiffValue::Decimal(Self::normalize_decimal_str(&num.to_string()))
            }
            ColumnData::Xml(Some(xml)) => DiffValue::Str(xml.as_ref().to_string()),
            ColumnData::Date(Some(d)) => {
                let days = d.days() as i64;
                let base = chrono::NaiveDate::from_ymd_opt(1, 1, 1).unwrap();
                let nd = base + chrono::Duration::days(days);
                DiffValue::Date(nd.format("%Y-%m-%d").to_string())
            }
            ColumnData::Time(Some(t)) => {
                let scale = t.scale() as u32;
                let inc = t.increments();
                let nanos_per_unit = 10_u64.pow(9u32.saturating_sub(scale));
                let total_nanos = inc.saturating_mul(nanos_per_unit);
                let secs = (total_nanos / 1_000_000_000) as u32;
                let nanos = (total_nanos % 1_000_000_000) as u32;
                let nt = chrono::NaiveTime::from_num_seconds_from_midnight_opt(secs, nanos)
                    .unwrap_or_else(|| chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap());
                DiffValue::Time(Self::normalize_time_str(
                    &nt.format("%H:%M:%S%.f").to_string(),
                ))
            }
            ColumnData::DateTime(Some(dt)) => {
                let base = chrono::NaiveDate::from_ymd_opt(1900, 1, 1).unwrap();
                let d = base + chrono::Duration::days(dt.days() as i64);
                let ms = ((dt.seconds_fragments() as u64) * 1000 + 150) / 300;
                let ndt =
                    d.and_hms_opt(0, 0, 0).unwrap() + chrono::Duration::milliseconds(ms as i64);
                DiffValue::DateTime(Self::normalize_datetime_str(
                    &ndt.format("%Y-%m-%d %H:%M:%S%.f").to_string(),
                ))
            }
            ColumnData::SmallDateTime(Some(dt)) => {
                let base = chrono::NaiveDate::from_ymd_opt(1900, 1, 1).unwrap();
                let d = base + chrono::Duration::days(dt.days() as i64);
                let ndt = d.and_hms_opt(0, 0, 0).unwrap()
                    + chrono::Duration::minutes(dt.seconds_fragments() as i64);
                DiffValue::DateTime(Self::normalize_datetime_str(
                    &ndt.format("%Y-%m-%d %H:%M:%S%.f").to_string(),
                ))
            }
            ColumnData::DateTime2(Some(dt2)) => {
                let base = chrono::NaiveDate::from_ymd_opt(1, 1, 1).unwrap();
                let d = base + chrono::Duration::days(dt2.date().days() as i64);
                let t = dt2.time();
                let scale = t.scale() as u32;
                let nanos_per_unit = 10_u64.pow(9u32.saturating_sub(scale));
                let total_nanos = t.increments().saturating_mul(nanos_per_unit);
                let secs = (total_nanos / 1_000_000_000) as u32;
                let nanos = (total_nanos % 1_000_000_000) as u32;
                let nt = chrono::NaiveTime::from_num_seconds_from_midnight_opt(secs, nanos)
                    .unwrap_or_else(|| chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap());
                let ndt = d.and_time(nt);
                DiffValue::DateTime(Self::normalize_datetime_str(
                    &ndt.format("%Y-%m-%d %H:%M:%S%.f").to_string(),
                ))
            }
            ColumnData::DateTimeOffset(Some(dto)) => {
                let dt2 = dto.datetime2();
                let base = chrono::NaiveDate::from_ymd_opt(1, 1, 1).unwrap();
                let d = base + chrono::Duration::days(dt2.date().days() as i64);
                let t = dt2.time();
                let scale = t.scale() as u32;
                let nanos_per_unit = 10_u64.pow(9u32.saturating_sub(scale));
                let total_nanos = t.increments().saturating_mul(nanos_per_unit);
                let secs = (total_nanos / 1_000_000_000) as u32;
                let nanos = (total_nanos % 1_000_000_000) as u32;
                let nt = chrono::NaiveTime::from_num_seconds_from_midnight_opt(secs, nanos)
                    .unwrap_or_else(|| chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap());
                let ndt = d.and_time(nt) + chrono::Duration::minutes(dto.offset() as i64);
                let off = dto.offset();
                let sign = if off < 0 { '-' } else { '+' };
                let oh = off.abs() / 60;
                let om = off.abs() % 60;
                let formatted = format!(
                    "{} {}{:02}:{:02}",
                    ndt.format("%Y-%m-%d %H:%M:%S%.f"),
                    sign,
                    oh,
                    om
                );
                DiffValue::DateTime(Self::normalize_datetime_str(&formatted))
            }
            _ => DiffValue::Null,
        }
    }

    pub fn approx_eq(&self, other: &DiffValue) -> bool {
        match (self, other) {
            (DiffValue::Null, DiffValue::Null) => true,
            (DiffValue::Bool(a), DiffValue::Bool(b)) => a == b,
            // SQL Server often returns INT (0/1) for predicate/ISJSON/bit functions where Iridium returns BIT
            (DiffValue::Bool(a), DiffValue::Int(b)) => (*a && *b == 1) || (!*a && *b == 0),
            (DiffValue::Int(a), DiffValue::Bool(b)) => (*b && *a == 1) || (!*b && *a == 0),
            (DiffValue::Int(a), DiffValue::Int(b)) => a == b,
            (DiffValue::Float(a), DiffValue::Float(b)) => {
                (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0)
            }
            (DiffValue::Int(a), DiffValue::Float(b)) | (DiffValue::Float(b), DiffValue::Int(a)) => {
                ((*a as f64) - b).abs() <= 1e-6
            }
            (DiffValue::Decimal(a), DiffValue::Decimal(b)) => a == b,
            (DiffValue::Decimal(a), DiffValue::Int(b))
            | (DiffValue::Int(b), DiffValue::Decimal(a)) => a == &b.to_string(),
            (DiffValue::Decimal(a), DiffValue::Float(b))
            | (DiffValue::Float(b), DiffValue::Decimal(a)) => a
                .parse::<f64>()
                .is_ok_and(|af| (af - b).abs() <= 1e-6 * af.abs().max(b.abs()).max(1.0)),
            (DiffValue::Str(a), DiffValue::Str(b)) => a == b,
            (DiffValue::Bytes(a), DiffValue::Bytes(b)) => a == b,
            (DiffValue::Date(a), DiffValue::Date(b)) => a == b,
            (DiffValue::Time(a), DiffValue::Time(b)) => a == b,
            (DiffValue::DateTime(a), DiffValue::DateTime(b)) => a == b,
            // Allow Date vs DateTime at midnight (e.g. DATETRUNC/DATE_BUCKET type nuances)
            (DiffValue::Date(d), DiffValue::DateTime(dt))
            | (DiffValue::DateTime(dt), DiffValue::Date(d)) => {
                dt.starts_with(d.as_str()) && dt.ends_with("00:00:00")
            }
            (DiffValue::Uuid(a), DiffValue::Uuid(b)) => a == b,
            _ => false,
        }
    }

    pub fn sort_key(&self) -> String {
        match self {
            DiffValue::Null => "\0NULL".to_string(),
            DiffValue::Bool(b) => format!("B:{}", if *b { 1 } else { 0 }),
            DiffValue::Int(n) => format!("N:{:020}", n),
            DiffValue::Float(f) => format!("F:{:.6}", f),
            DiffValue::Decimal(d) => format!("D:{}", d),
            DiffValue::Str(s) => format!("S:{}", s),
            DiffValue::Bytes(b) => format!("X:{}", hex::encode(b)),
            DiffValue::Date(d) => format!("DT:{}", d),
            DiffValue::Time(t) => format!("TM:{}", t),
            DiffValue::DateTime(dt) => format!("DTM:{}", dt),
            DiffValue::Uuid(u) => format!("U:{}", u),
        }
    }
}

pub fn rows_match(
    sql_rows: &[Vec<DiffValue>],
    iridium_rows: &[Vec<DiffValue>],
    ordered: bool,
) -> bool {
    if sql_rows.len() != iridium_rows.len() {
        return false;
    }
    if ordered {
        sql_rows
            .iter()
            .zip(iridium_rows.iter())
            .all(|(a, b)| a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.approx_eq(y)))
    } else {
        let mut s_sorted: Vec<&Vec<DiffValue>> = sql_rows.iter().collect();
        let mut i_sorted: Vec<&Vec<DiffValue>> = iridium_rows.iter().collect();
        let row_key = |r: &&Vec<DiffValue>| {
            r.iter()
                .map(DiffValue::sort_key)
                .collect::<Vec<_>>()
                .join("|")
        };
        s_sorted.sort_by_key(row_key);
        i_sorted.sort_by_key(row_key);
        s_sorted
            .iter()
            .zip(i_sorted.iter())
            .all(|(a, b)| a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.approx_eq(y)))
    }
}

pub fn run_iridium_batch(engine: &Engine, sql: &str) -> Result<Vec<Vec<DiffValue>>, String> {
    let stmts = parse_batch(sql).map_err(|e| e.to_string())?;
    let res: Option<QueryResult> = engine.execute_batch(stmts).map_err(|e| e.to_string())?;
    Ok(res
        .map(|qr| {
            qr.rows
                .iter()
                .map(|row| row.iter().map(DiffValue::from_iridium).collect())
                .collect()
        })
        .unwrap_or_default())
}

pub struct LiveSqlServerConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
}

impl LiveSqlServerConfig {
    pub fn from_env() -> Self {
        Self {
            host: std::env::var("SQLSERVER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: std::env::var("SQLSERVER_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(11433),
            user: std::env::var("SQLSERVER_USER").unwrap_or_else(|_| "sa".to_string()),
            password: std::env::var("SQLSERVER_PASSWORD")
                .unwrap_or_else(|_| "IridiumTest!2026".to_string()),
        }
    }
}

pub async fn connect_sqlserver(
    cfg: &LiveSqlServerConfig,
) -> Result<Client<Compat<TcpStream>>, String> {
    let mut tcfg = Config::new();
    tcfg.host(&cfg.host);
    tcfg.port(cfg.port);
    tcfg.authentication(tiberius::AuthMethod::sql_server(&cfg.user, &cfg.password));
    tcfg.trust_cert();
    let tcp = TcpStream::connect(tcfg.get_addr())
        .await
        .map_err(|e| format!("TCP connect {}:{} failed: {}", cfg.host, cfg.port, e))?;
    tcp.set_nodelay(true).ok();
    Client::connect(tcfg, tcp.compat_write())
        .await
        .map_err(|e| format!("TDS login failed: {}", e))
}

pub async fn run_sqlserver_batch(
    client: &mut Client<Compat<TcpStream>>,
    sql: &str,
) -> Result<Vec<Vec<DiffValue>>, String> {
    let stream = client.simple_query(sql).await.map_err(|e| e.to_string())?;
    let all_sets = stream.into_results().await.map_err(|e| e.to_string())?;
    let last_set = all_sets.into_iter().last().unwrap_or_default();
    Ok(last_set
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|cd| DiffValue::from_tiberius(&cd))
                .collect()
        })
        .collect())
}

pub async fn detect_sqlserver_major_version(
    client: &mut Client<Compat<TcpStream>>,
) -> Result<u32, String> {
    let rows = run_sqlserver_batch(
        client,
        "SELECT CAST(SERVERPROPERTY('ProductMajorVersion') AS INT);",
    )
    .await?;
    match rows.first().and_then(|r| r.first()) {
        Some(DiffValue::Int(v)) => Ok(*v as u32),
        other => Err(format!("unexpected ProductMajorVersion: {:?}", other)),
    }
}

static DB_SEQ: AtomicU64 = AtomicU64::new(1);

pub struct SuiteRunReport {
    pub suite_name: &'static str,
    pub category: &'static str,
    pub skipped_reason: Option<String>,
    pub passed: usize,
    pub failures: Vec<String>,
}

pub async fn run_differential_suite(
    cfg: &LiveSqlServerConfig,
    sql_major_version: u32,
    suite: &DiffSuite,
) -> SuiteRunReport {
    if sql_major_version < suite.min_sql_version {
        return SuiteRunReport {
            suite_name: suite.name,
            category: suite.category,
            skipped_reason: Some(format!(
                "requires SQL Server major >= {} (connected: {})",
                suite.min_sql_version, sql_major_version
            )),
            passed: 0,
            failures: Vec::new(),
        };
    }

    let mut client = match connect_sqlserver(cfg).await {
        Ok(c) => c,
        Err(e) => {
            return SuiteRunReport {
                suite_name: suite.name,
                category: suite.category,
                skipped_reason: None,
                passed: 0,
                failures: vec![format!("connect failed: {}", e)],
            }
        }
    };

    let db_name = format!(
        "IridiumDiff_{}_{}",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    if let Err(e) =
        run_sqlserver_batch(&mut client, &format!("CREATE DATABASE [{}];", db_name)).await
    {
        return SuiteRunReport {
            suite_name: suite.name,
            category: suite.category,
            skipped_reason: None,
            passed: 0,
            failures: vec![format!("CREATE DATABASE failed: {}", e)],
        };
    }

    let mut passed = 0usize;
    let mut failures = Vec::new();
    let engine = Engine::new();

    if let Err(e) = run_sqlserver_batch(&mut client, &format!("USE [{}];", db_name)).await {
        failures.push(format!("USE {} failed: {}", db_name, e));
    } else {
        let mut setup_ok = true;
        for (i, batch) in suite.setup_batches.iter().enumerate() {
            let s_res = run_sqlserver_batch(&mut client, batch).await;
            let i_res = run_iridium_batch(&engine, batch);
            if let Err(e) = s_res {
                failures.push(format!("setup[{}] SQL Server error: {}", i, e));
                setup_ok = false;
                break;
            }
            if let Err(e) = i_res {
                failures.push(format!("setup[{}] Iridium error: {}", i, e));
                setup_ok = false;
                break;
            }
        }

        if setup_ok {
            for step in suite.steps {
                match step {
                    DiffStep::ExecOk { label, sql } => {
                        let s_res = run_sqlserver_batch(&mut client, sql).await;
                        let i_res = run_iridium_batch(&engine, sql);
                        match (s_res, i_res) {
                            (Ok(_), Ok(_)) => passed += 1,
                            (Err(se), Ok(_)) => failures.push(format!(
                                "[{}] SQL Server FAILED ({}) while Iridium succeeded",
                                label, se
                            )),
                            (Ok(_), Err(ie)) => failures.push(format!(
                                "[{}] Iridium FAILED ({}) while SQL Server succeeded",
                                label, ie
                            )),
                            (Err(se), Err(ie)) => failures.push(format!(
                                "[{}] BOTH failed (expected OK) — SQL Server: {} | Iridium: {}",
                                label, se, ie
                            )),
                        }
                    }
                    DiffStep::ExpectErr { label, sql } => {
                        let s_res = run_sqlserver_batch(&mut client, sql).await;
                        let i_res = run_iridium_batch(&engine, sql);
                        match (s_res, i_res) {
                            (Err(_), Err(_)) => passed += 1,
                            (Ok(_), Err(_)) => failures.push(format!(
                                "[{}] SQL Server SUCCEEDED while Iridium rejected (expected both to error)",
                                label
                            )),
                            (Err(se), Ok(_)) => failures.push(format!(
                                "[{}] Iridium SUCCEEDED while SQL Server rejected ({})",
                                label, se
                            )),
                            (Ok(_), Ok(_)) => failures.push(format!(
                                "[{}] BOTH succeeded (expected both to error)",
                                label
                            )),
                        }
                    }
                    DiffStep::Query {
                        label,
                        sql,
                        ordered,
                    } => {
                        let s_res = run_sqlserver_batch(&mut client, sql).await;
                        let i_res = run_iridium_batch(&engine, sql);
                        match (s_res, i_res) {
                            (Ok(s_rows), Ok(i_rows)) => {
                                if rows_match(&s_rows, &i_rows, *ordered) {
                                    passed += 1;
                                } else {
                                    failures.push(format!(
                                        "[{}] RESULT MISMATCH:\n  SQL Server ({} rows): {:?}\n  Iridium    ({} rows): {:?}",
                                        label,
                                        s_rows.len(),
                                        s_rows,
                                        i_rows.len(),
                                        i_rows
                                    ));
                                }
                            }
                            (Err(se), Ok(i_rows)) => failures.push(format!(
                                "[{}] SQL Server ERROR ({}) while Iridium returned {:?}",
                                label, se, i_rows
                            )),
                            (Ok(s_rows), Err(ie)) => failures.push(format!(
                                "[{}] Iridium ERROR ({}) while SQL Server returned {:?}",
                                label, ie, s_rows
                            )),
                            (Err(se), Err(ie)) => failures.push(format!(
                                "[{}] BOTH errored on Query — SQL Server: {} | Iridium: {}",
                                label, se, ie
                            )),
                        }
                    }
                }
            }
        }
    }

    // Always clean up throwaway database
    let _ = run_sqlserver_batch(
        &mut client,
        &format!(
            "USE master; ALTER DATABASE [{}] SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE [{}];",
            db_name, db_name
        ),
    )
    .await;

    SuiteRunReport {
        suite_name: suite.name,
        category: suite.category,
        skipped_reason: None,
        passed,
        failures,
    }
}
