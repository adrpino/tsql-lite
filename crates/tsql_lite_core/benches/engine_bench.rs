use divan::black_box;
use tsql_lite_core::{parse_batch, Database, SessionId, SessionManager, StatementExecutor};

fn main() {
    divan::main();
}

fn new_db_and_session() -> (Database, SessionId) {
    let db = Database::new();
    let sid = db.create_session();
    (db, sid)
}

fn exec_sql(db: &Database, sid: SessionId, sql: &str) {
    let stmts = parse_batch(sql).expect("parse batch");
    db.executor()
        .execute_session_batch(sid, stmts)
        .expect("exec batch");
}

fn query_rows(db: &Database, sid: SessionId, sql: &str) -> usize {
    let stmts = parse_batch(sql).expect("parse query");
    db.executor()
        .execute_session_batch(sid, stmts)
        .expect("exec query")
        .map_or(0, |r| r.rows.len())
}

fn seed_orders_db(customers: usize, orders: usize) -> (Database, SessionId) {
    let (db, sid) = new_db_and_session();
    exec_sql(
        &db,
        sid,
        "CREATE TABLE dbo.Customers (
            Id INT PRIMARY KEY,
            Region NVARCHAR(20) NOT NULL,
            Name NVARCHAR(100) NOT NULL
        );
        CREATE TABLE dbo.Orders (
            Id INT PRIMARY KEY,
            CustomerId INT NOT NULL,
            Amount DECIMAL(12, 2) NOT NULL,
            Status NVARCHAR(20) NOT NULL
        );",
    );

    let mut sql = String::with_capacity(customers * 56 + orders * 64);
    for c in 1..=customers {
        let region = match c % 4 {
            0 => "NORTH",
            1 => "SOUTH",
            2 => "EAST",
            _ => "WEST",
        };
        sql.push_str(&format!(
            "INSERT INTO dbo.Customers (Id, Region, Name) VALUES ({c}, N'{region}', N'Customer_{c}');\n"
        ));
    }
    for o in 1..=orders {
        let cid = (o % customers) + 1;
        let amount = (o % 250) + 10;
        let status = if o % 5 == 0 { "CANCELLED" } else { "PAID" };
        sql.push_str(&format!(
            "INSERT INTO dbo.Orders (Id, CustomerId, Amount, Status) VALUES ({o}, {cid}, {amount}.50, N'{status}');\n"
        ));
    }
    exec_sql(&db, sid, &sql);
    (db, sid)
}

#[divan::bench]
fn bench_01_cold_db_new_and_select_1() {
    let (db, sid) = new_db_and_session();
    black_box(query_rows(&db, sid, "SELECT 1 AS n;"));
}

#[divan::bench]
fn bench_02_parse_complex_procedure() {
    let sql = "
        CREATE PROCEDURE dbo.usp_OrderSummary
            @Region NVARCHAR(20),
            @MinAmount DECIMAL(12, 2) = 0
        AS
        BEGIN
            SET NOCOUNT ON;
            WITH ActiveOrders AS (
                SELECT o.Id, o.CustomerId, o.Amount,
                       ROW_NUMBER() OVER (PARTITION BY o.CustomerId ORDER BY o.Amount DESC) AS rn
                FROM dbo.Orders o
                WHERE o.Status = N'PAID' AND o.Amount >= @MinAmount
            )
            SELECT c.Region, c.Name, COUNT(*) AS OrderCount, SUM(a.Amount) AS TotalAmount
            FROM dbo.Customers c
            INNER JOIN ActiveOrders a ON a.CustomerId = c.Id
            WHERE c.Region = @Region AND a.rn <= 5
            GROUP BY c.Region, c.Name
            ORDER BY TotalAmount DESC, c.Name ASC;
        END;
    ";
    let stmts = parse_batch(black_box(sql)).expect("parse");
    black_box(stmts.len());
}

#[divan::bench]
fn bench_03_insert_500_rows(bencher: divan::Bencher) {
    let mut insert_sql = String::with_capacity(500 * 64);
    for i in 1..=500 {
        insert_sql.push_str(&format!(
            "INSERT INTO dbo.Items (Id, Code, Price) VALUES ({i}, N'ITEM_{i}', {i}.25);\n"
        ));
    }
    bencher
        .with_inputs(|| {
            let (db, sid) = new_db_and_session();
            exec_sql(
                &db,
                sid,
                "CREATE TABLE dbo.Items (Id INT PRIMARY KEY, Code NVARCHAR(32), Price DECIMAL(10, 2));",
            );
            (db, sid)
        })
        .bench_local_values(|(db, sid)| {
            exec_sql(&db, sid, black_box(&insert_sql));
            black_box(db);
        });
}

#[divan::bench]
fn bench_04_pk_point_lookup(bencher: divan::Bencher) {
    let (db, sid) = seed_orders_db(200, 1000);
    bencher.bench_local(|| {
        black_box(query_rows(
            &db,
            sid,
            "SELECT Id, CustomerId, Amount, Status FROM dbo.Orders WHERE Id = 777;",
        ))
    });
}

#[divan::bench]
fn bench_05_join_group_by_order_by(bencher: divan::Bencher) {
    let (db, sid) = seed_orders_db(200, 1000);
    bencher.bench_local(|| {
        black_box(query_rows(
            &db,
            sid,
            "SELECT c.Region, COUNT(*) AS OrderCount, SUM(o.Amount) AS TotalAmount
             FROM dbo.Orders o
             INNER JOIN dbo.Customers c ON c.Id = o.CustomerId
             WHERE o.Status = N'PAID'
             GROUP BY c.Region
             ORDER BY TotalAmount DESC;",
        ))
    });
}

#[divan::bench]
fn bench_06_hash_in_subquery(bencher: divan::Bencher) {
    let (db, sid) = seed_orders_db(200, 1000);
    bencher.bench_local(|| {
        black_box(query_rows(
            &db,
            sid,
            "SELECT COUNT(*) AS Cnt
             FROM dbo.Orders
             WHERE CustomerId IN (SELECT Id FROM dbo.Customers WHERE Region IN (N'NORTH', N'EAST'));",
        ))
    });
}

#[divan::bench]
fn bench_07_procedural_while_loop(bencher: divan::Bencher) {
    let (db, sid) = new_db_and_session();
    bencher.bench_local(|| {
        black_box(query_rows(
            &db,
            sid,
            "DECLARE @i INT = 0, @acc BIGINT = 0;
             WHILE @i < 200
             BEGIN
                 SET @i = @i + 1;
                 SET @acc = @acc + (@i * 3 - 1);
             END
             SELECT @acc AS Total;",
        ))
    });
}

#[divan::bench]
fn bench_08_for_json_path(bencher: divan::Bencher) {
    let (db, sid) = seed_orders_db(50, 200);
    bencher.bench_local(|| {
        black_box(query_rows(
            &db,
            sid,
            "SELECT TOP 50 o.Id, o.CustomerId, o.Amount, o.Status
             FROM dbo.Orders o
             ORDER BY o.Id
             FOR JSON PATH;",
        ))
    });
}

#[divan::bench]
fn bench_09_read_committed_select_in_tx(bencher: divan::Bencher) {
    let (db, sid) = new_db_and_session();
    let mut ddl = String::new();
    for i in 0..200 {
        ddl.push_str(&format!(
            "CREATE TABLE dbo.CatalogTable_{i} (Id INT PRIMARY KEY, Val INT);\n"
        ));
    }
    ddl.push_str("INSERT INTO dbo.CatalogTable_0 VALUES (1, 42);");
    exec_sql(&db, sid, &ddl);
    exec_sql(&db, sid, "BEGIN TRANSACTION;");
    bencher.bench_local(|| {
        black_box(query_rows(
            &db,
            sid,
            "SELECT Val FROM dbo.CatalogTable_0 WHERE Id = 1;",
        ))
    });
    exec_sql(&db, sid, "ROLLBACK TRANSACTION;");
}
