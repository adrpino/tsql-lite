use iridium_core::{types::Value, Engine};

#[test]
fn create_view_with_explicit_column_list_renames_output_columns() {
    let e = Engine::new();
    e.exec("CREATE TABLE dbo.items (id INT, price INT)")
        .unwrap();
    e.exec("INSERT INTO dbo.items VALUES (1, 50), (2, 120)")
        .unwrap();

    e.exec("CREATE VIEW dbo.v_items (item_id, unit_price) AS SELECT id, price FROM dbo.items")
        .unwrap();

    let r = e
        .query("SELECT item_id, unit_price FROM dbo.v_items ORDER BY item_id")
        .unwrap();
    assert_eq!(r.columns, vec!["item_id", "unit_price"]);
    assert_eq!(r.rows.len(), 2);
    assert_eq!(r.rows[0], vec![Value::Int(1), Value::Int(50)]);
    assert_eq!(r.rows[1], vec![Value::Int(2), Value::Int(120)]);
}

#[test]
fn create_view_with_schemabinding_and_column_list() {
    let e = Engine::new();
    e.exec("CREATE TABLE dbo.orders (id INT, qty INT)").unwrap();
    e.exec("INSERT INTO dbo.orders VALUES (10, 4)").unwrap();

    e.exec(
        "CREATE VIEW dbo.v_orders (order_id, doubled_qty) WITH SCHEMABINDING AS SELECT id, qty * 2 FROM dbo.orders",
    )
    .unwrap();

    let r = e
        .query("SELECT order_id, doubled_qty FROM dbo.v_orders")
        .unwrap();
    assert_eq!(r.rows[0], vec![Value::Int(10), Value::BigInt(8)]);
}

#[test]
fn create_view_rejects_mismatched_column_count() {
    let e = Engine::new();
    e.exec("CREATE TABLE dbo.t (a INT, b INT)").unwrap();
    assert!(e
        .exec("CREATE VIEW dbo.v_bad (only_one) AS SELECT a, b FROM dbo.t")
        .is_err());
}

#[test]
fn create_view_with_parenthesized_select_body() {
    let e = Engine::new();
    e.exec("CREATE TABLE dbo.items (id INT, price INT)")
        .unwrap();
    e.exec("INSERT INTO dbo.items VALUES (1, 25), (2, 75)")
        .unwrap();

    e.exec("CREATE VIEW dbo.v_paren AS (SELECT id, price FROM dbo.items WHERE price > 30)")
        .unwrap();

    let r = e.query("SELECT id, price FROM dbo.v_paren").unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0], vec![Value::Int(2), Value::Int(75)]);
}
