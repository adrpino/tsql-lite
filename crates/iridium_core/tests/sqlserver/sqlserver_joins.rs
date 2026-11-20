use iridium_core::types::Value;
use iridium_core::{parse_sql, Engine};







fn engine_exec(engine: &mut Engine, sql: &str) -> Option<iridium_core::QueryResult> {
    let stmt = parse_sql(sql).unwrap_or_else(|_| panic!("Parser falhou: {}", sql));
    engine.execute(stmt).unwrap_or_else(|_| panic!("Engine falhou: {}", sql))
}

// ─── LEFT OUTER JOIN ─────────────────────────────────────────────────────

#[test]
fn test_left_outer_join_compare() {
    let mut engine = Engine::new();
    engine_exec(&mut engine, "CREATE TABLE t_customers (id INT PRIMARY KEY, name VARCHAR(50))");
    engine_exec(&mut engine, "CREATE TABLE t_orders (id INT PRIMARY KEY, customer_id INT, amount INT)");

    engine_exec(&mut engine, "INSERT INTO t_customers VALUES (1, 'Alice'), (2, 'Bob')");
    engine_exec(&mut engine, "INSERT INTO t_orders VALUES (100, 1, 500)");

    let join_sql = "SELECT c.name, o.amount FROM t_customers c LEFT JOIN t_orders o ON c.id = o.customer_id ORDER BY c.id";
    let _engine_result = engine_exec(&mut engine, join_sql).unwrap();
    assert!(!_engine_result.rows.is_empty());

}

#[test]
fn test_hash_join_int_equals_string_split_value() {
    let mut engine = Engine::new();
    engine_exec(&mut engine, "CREATE TABLE t_items (id INT, label VARCHAR(20))");
    engine_exec(&mut engine, "INSERT INTO t_items VALUES (10, 'Alpha'), (20, 'Beta'), (30, 'Gamma')");

    let res = engine_exec(
        &mut engine,
        "SELECT i.id, i.label FROM t_items i JOIN STRING_SPLIT('10,30', ',') s ON i.id = s.value ORDER BY i.id",
    )
    .unwrap();
    assert_eq!(res.rows.len(), 2);
    assert_eq!(res.rows[0][0], Value::Int(10));
    assert_eq!(res.rows[1][0], Value::Int(30));
}

#[test]
fn test_hash_join_cross_width_numeric_and_case_insensitive_strings() {
    let mut engine = Engine::new();
    engine_exec(&mut engine, "CREATE TABLE t_left (id INT, code VARCHAR(20))");
    engine_exec(&mut engine, "CREATE TABLE t_right (id BIGINT, code NVARCHAR(20), tag VARCHAR(20))");
    engine_exec(&mut engine, "INSERT INTO t_left VALUES (1, 'ABC  '), (2, 'xyz')");
    engine_exec(&mut engine, "INSERT INTO t_right VALUES (1, N'abc', 'first'), (2, N'XYZ ', 'second')");

    let res = engine_exec(
        &mut engine,
        "SELECT l.id, r.tag FROM t_left l JOIN t_right r ON l.id = r.id AND l.code = r.code ORDER BY l.id",
    )
    .unwrap();
    assert_eq!(res.rows.len(), 2);
    assert_eq!(res.rows[0][1], Value::VarChar("first".into()));
    assert_eq!(res.rows[1][1], Value::VarChar("second".into()));
}

#[test]
fn test_group_by_case_insensitive_and_trimmed_strings() {
    let mut engine = Engine::new();
    engine_exec(&mut engine, "CREATE TABLE t_codes (code VARCHAR(20), qty INT)");
    engine_exec(&mut engine, "INSERT INTO t_codes VALUES ('Alpha', 2), ('alpha  ', 3), ('ALPHA', 5)");

    let res = engine_exec(
        &mut engine,
        "SELECT COUNT(*), SUM(qty) FROM t_codes GROUP BY code",
    )
    .unwrap();
    assert_eq!(res.rows.len(), 1);
    assert_eq!(res.rows[0][0], Value::BigInt(3));
    assert_eq!(res.rows[0][1], Value::BigInt(10));
}

#[test]
fn test_compare_numeric_with_string_order() {
    let mut engine = Engine::new();
    let res = engine_exec(
        &mut engine,
        "SELECT CASE WHEN 10 > '2' THEN 1 ELSE 0 END, CASE WHEN '2' < 10 THEN 1 ELSE 0 END",
    )
    .unwrap();
    assert_eq!(res.rows[0][0], Value::Int(1));
    assert_eq!(res.rows[0][1], Value::Int(1));
}


