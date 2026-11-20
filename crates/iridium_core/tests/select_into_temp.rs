use iridium_core::{types::Value, Engine};

#[test]
fn select_into_temp_table_parses_and_executes_without_alias() {
    let e = Engine::new();
    e.exec("CREATE TABLE dbo.items (id INT, label NVARCHAR(50))")
        .unwrap();
    e.exec("INSERT INTO dbo.items VALUES (1, N'alpha'), (2, N'beta')")
        .unwrap();

    e.exec("SELECT id, label INTO #copy FROM dbo.items WHERE id >= 1")
        .unwrap();
    let r = e.query("SELECT id, label FROM #copy ORDER BY id").unwrap();
    assert_eq!(r.rows.len(), 2);
    assert_eq!(r.rows[0][0], Value::Int(1));
    assert_eq!(r.rows[1][0], Value::Int(2));

    e.exec("DROP TABLE IF EXISTS #copy").unwrap();
}

#[test]
fn select_into_temp_table_inside_stored_procedure() {
    let e = Engine::new();
    e.exec("CREATE TABLE dbo.orders (id INT, amount INT)")
        .unwrap();
    e.exec("INSERT INTO dbo.orders VALUES (1, 100), (2, 250)")
        .unwrap();
    e.exec(
        "CREATE PROCEDURE dbo.summarize_orders AS BEGIN SELECT id, amount INTO #high FROM dbo.orders WHERE amount >= 200; SELECT COUNT(*) AS cnt, SUM(amount) AS total FROM #high; END",
    )
    .unwrap();

    let r = e.query("EXEC dbo.summarize_orders").unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], Value::BigInt(1));
    assert_eq!(r.rows[0][1], Value::BigInt(250));
}
