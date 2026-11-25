use tsql_lite_core::{parse_sql, Engine};

fn engine_exec(engine: &mut Engine, sql: &str) -> Option<tsql_lite_core::QueryResult> {
    let stmt = parse_sql(sql).unwrap_or_else(|_| panic!("Parser falhou: {}", sql));
    engine
        .execute(stmt)
        .unwrap_or_else(|_| panic!("Engine falhou: {}", sql))
}

// ─── DATEADD ────────────────────────────────────────────────────────────

#[test]
fn test_dateadd_compare() {
    let mut engine = Engine::new();

    let sql = "SELECT DATEADD(day, 5, '2024-01-01')";
    let _engine_result = engine_exec(&mut engine, sql).unwrap();
}

// ─── DATEDIFF ───────────────────────────────────────────────────────────

#[test]
fn test_datediff_compare() {
    let mut engine = Engine::new();

    let sql = "SELECT DATEDIFF(day, '2024-01-01', '2024-01-10')";
    let _engine_result = engine_exec(&mut engine, sql).unwrap();
}

// ─── YEAR/MONTH/DAY ─────────────────────────────────────────────────────

#[test]
fn test_year_month_day_compare() {
    let mut engine = Engine::new();

    let sql = "SELECT YEAR('2024-06-15'), MONTH('2024-06-15'), DAY('2024-06-15')";
    let _engine_result = engine_exec(&mut engine, sql).unwrap();

    assert!(!_engine_result.rows.is_empty());
}

#[test]
fn test_datetime_casting_with_hm_fractional_and_basic_iso() {
    let mut engine = Engine::new();
    let r = engine_exec(
        &mut engine,
        "SELECT CAST('2026-04-01 08:00' AS DATETIME), CAST('2026-04-01 08:00:00.123' AS DATETIME2), CAST('2026-04-01T08:30' AS DATETIME), CAST('20260401' AS DATE)",
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0].len(), 4);
}

#[test]
fn test_temporal_and_float_type_precision_parameters() {
    let mut engine = Engine::new();
    let r = engine_exec(
        &mut engine,
        "BEGIN
            DECLARE @dt2 DATETIME2(0) = '2026-04-01T12:30:00';
            DECLARE @t TIME(3) = '12:30:00';
            DECLARE @dto DATETIMEOFFSET(7) = '2026-04-01 12:30:00';
            DECLARE @sdt SMALLDATETIME = '2026-04-01 12:30:00';
            DECLARE @f FLOAT(53) = 1.5;
            SELECT CAST(@dt2 AS DATETIME2(3)), @t, @dto, @sdt, @f;
        END",
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0].len(), 5);
}

#[test]
fn test_sqlserver_epoch_zero_datetime_and_binary_to_uuid() {
    let mut engine = Engine::new();
    let r = engine_exec(
        &mut engine,
        "SELECT DATEADD(day, DATEDIFF(day, 0, '2026-04-15 14:35:20'), 0), CAST(0 AS DATETIME), CAST(0x0 AS UNIQUEIDENTIFIER)",
    )
    .unwrap();
    assert_eq!(r.rows[0][0].to_string_value(), "2026-04-15 00:00:00");
    assert_eq!(r.rows[0][1].to_string_value(), "1900-01-01 00:00:00");
    assert_eq!(
        r.rows[0][2].to_string_value().to_ascii_lowercase(),
        "00000000-0000-0000-0000-000000000000"
    );
}
