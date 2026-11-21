use iridium_core::{Engine, QueryResult};

fn exec(engine: &mut Engine, sql: &str) {
    let stmts = iridium_core::parse_batch(sql)
        .unwrap_or_else(|e| panic!("parse failed: {sql}\nerror: {e}"));
    engine
        .execute_batch(stmts)
        .unwrap_or_else(|e| panic!("SQL failed: {sql}\nerror: {e}"));
}

fn query(engine: &mut Engine, sql: &str) -> QueryResult {
    let stmts = iridium_core::parse_batch(sql)
        .unwrap_or_else(|e| panic!("parse failed: {sql}\nerror: {e}"));
    engine
        .execute_batch(stmts)
        .unwrap_or_else(|e| panic!("SQL failed: {sql}\nerror: {e}"))
        .into_iter()
        .last()
        .unwrap_or_default()
}

#[test]
fn for_json_path_and_auto() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE TABLE dbo.items (id INT, label NVARCHAR(50), price DECIMAL(10, 2), active BIT, note NVARCHAR(50) NULL);
         INSERT INTO dbo.items VALUES (1, N'Alpha', 12.50, 1, NULL), (2, N'Beta', 7.00, 0, N'ok');",
    );

    // 1. FOR JSON PATH default (omits NULLs, wraps in array)
    let r1 = query(
        &mut e,
        "SELECT id, label, active, note FROM dbo.items ORDER BY id FOR JSON PATH",
    );
    let parsed1: serde_json::Value =
        serde_json::from_str(&r1.rows[0][0].to_string_value()).expect("valid JSON");
    assert_eq!(
        parsed1,
        serde_json::json!([
            {"id": 1, "label": "Alpha", "active": true},
            {"id": 2, "label": "Beta", "active": false, "note": "ok"}
        ])
    );

    // 2. FOR JSON PATH with INCLUDE_NULL_VALUES and ROOT('items')
    let r2 = query(
        &mut e,
        "SELECT id, note FROM dbo.items ORDER BY id FOR JSON PATH, ROOT('items'), INCLUDE_NULL_VALUES",
    );
    let parsed2: serde_json::Value =
        serde_json::from_str(&r2.rows[0][0].to_string_value()).expect("valid JSON");
    assert_eq!(
        parsed2,
        serde_json::json!({
            "items": [
                {"id": 1, "note": null},
                {"id": 2, "note": "ok"}
            ]
        })
    );

    // 3. FOR JSON PATH with dotted nested aliases + WITHOUT_ARRAY_WRAPPER
    let r3 = query(
        &mut e,
        "SELECT id, label AS [meta.label], active AS [meta.active]
         FROM dbo.items WHERE id = 1
         FOR JSON PATH, WITHOUT_ARRAY_WRAPPER",
    );
    let parsed3: serde_json::Value =
        serde_json::from_str(&r3.rows[0][0].to_string_value()).expect("valid JSON");
    assert_eq!(
        parsed3,
        serde_json::json!({"id": 1, "meta": {"label": "Alpha", "active": true}})
    );

    // 4. FOR JSON AUTO + scalar subquery usage
    let r4 = query(
        &mut e,
        "SELECT (SELECT id, label FROM dbo.items WHERE id = 2 FOR JSON AUTO, WITHOUT_ARRAY_WRAPPER) AS payload",
    );
    let parsed4: serde_json::Value =
        serde_json::from_str(&r4.rows[0][0].to_string_value()).expect("valid JSON");
    assert_eq!(parsed4, serde_json::json!({"id": 2, "label": "Beta"}));

    // 5. Empty result set: 0 rows without ROOT, empty array with ROOT
    let r5_empty = query(&mut e, "SELECT id FROM dbo.items WHERE id = 999 FOR JSON PATH");
    assert!(r5_empty.rows.is_empty());

    let r5_root = query(
        &mut e,
        "SELECT id FROM dbo.items WHERE id = 999 FOR JSON PATH, ROOT('items')",
    );
    let parsed5: serde_json::Value =
        serde_json::from_str(&r5_root.rows[0][0].to_string_value()).expect("valid JSON");
    assert_eq!(parsed5, serde_json::json!({"items": []}));
}

#[test]
fn xml_exist_query_and_nodes_shredding() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE TABLE dbo.xml_docs (doc_id INT, payload XML);
         INSERT INTO dbo.xml_docs VALUES
           (1, N'<order><item id=\"10\"><name>Bolt</name><qty>3</qty></item><item id=\"20\"><name>Nut</name><qty>7</qty></item></order>'),
           (2, N'<order></order>'),
           (3, NULL);",
    );

    // 1. Scalar .exist() on column and variable
    let r_exist = query(
        &mut e,
        "SELECT doc_id,
                payload.exist('/order/item') AS has_item,
                payload.exist('/order/item[@id=\"20\"]') AS has_20,
                payload.exist('/order/item[@id=\"99\"]') AS has_99
         FROM dbo.xml_docs ORDER BY doc_id",
    );
    assert_eq!(r_exist.rows[0][1].to_bool(), Some(true));
    assert_eq!(r_exist.rows[0][2].to_bool(), Some(true));
    assert_eq!(r_exist.rows[0][3].to_bool(), Some(false));
    assert_eq!(r_exist.rows[1][1].to_bool(), Some(false));
    assert!(r_exist.rows[2][1].is_null());

    // 2. Scalar .query() returning XML fragment
    let r_query = query(
        &mut e,
        "SELECT payload.query('/order/item[1]/name') AS first_name_xml
         FROM dbo.xml_docs WHERE doc_id = 1",
    );
    assert_eq!(r_query.rows[0][0].to_string_value(), "<name>Bolt</name>");

    // 3. CROSS APPLY .nodes() shredding column into rows with .value() and .exist()
    let r_nodes = query(
        &mut e,
        "SELECT d.doc_id,
                x.c.value('@id', 'INT') AS item_id,
                x.c.value('(name/text())[1]', 'NVARCHAR(50)') AS item_name,
                x.c.value('(qty/text())[1]', 'INT') AS qty
         FROM dbo.xml_docs d
         CROSS APPLY d.payload.nodes('/order/item') AS x(c)
         ORDER BY d.doc_id, item_id",
    );
    assert_eq!(r_nodes.rows.len(), 2);
    assert_eq!(r_nodes.rows[0][0].to_integer_i64(), Some(1));
    assert_eq!(r_nodes.rows[0][1].to_integer_i64(), Some(10));
    assert_eq!(r_nodes.rows[0][2].to_string_value(), "Bolt");
    assert_eq!(r_nodes.rows[0][3].to_integer_i64(), Some(3));
    assert_eq!(r_nodes.rows[1][1].to_integer_i64(), Some(20));
    assert_eq!(r_nodes.rows[1][2].to_string_value(), "Nut");
    assert_eq!(r_nodes.rows[1][3].to_integer_i64(), Some(7));

    // 4. OUTER APPLY .nodes() preserves unmatched rows with NULL
    let r_outer = query(
        &mut e,
        "SELECT d.doc_id, x.c.value('@id', 'INT') AS item_id
         FROM dbo.xml_docs d
         OUTER APPLY d.payload.nodes('/order/item') AS x(c)
         ORDER BY d.doc_id, item_id",
    );
    assert_eq!(r_outer.rows.len(), 4);
    assert_eq!(r_outer.rows[2][0].to_integer_i64(), Some(2));
    assert!(r_outer.rows[2][1].is_null());
    assert_eq!(r_outer.rows[3][0].to_integer_i64(), Some(3));
    assert!(r_outer.rows[3][1].is_null());

    // 5. Variable @doc.nodes(...) in FROM / CROSS APPLY
    let r_var = query(
        &mut e,
        "DECLARE @x XML = N'<root><v>100</v><v>200</v></root>';
         SELECT SUM(t.n.value('.', 'INT')) AS total
         FROM @x.nodes('/root/v') AS t(n);",
    );
    assert_eq!(r_var.rows[0][0].to_integer_i64(), Some(300));
}

#[test]
fn goto_labels_forward_backward_and_nested() {
    let mut e = Engine::new();

    // 1. Forward jump in a top-level batch (skips middle assignment)
    let r1 = query(
        &mut e,
        "DECLARE @v INT = 1;
         GOTO Done;
         SET @v = 999;
         Done:
         SELECT @v AS v;",
    );
    assert_eq!(r1.rows[0][0].to_integer_i64(), Some(1));

    // 2. Backward jump loop + jumping out of nested IF/WHILE in a stored procedure
    exec(
        &mut e,
        "CREATE PROCEDURE dbo.usp_goto_demo @limit INT
         AS
         BEGIN
             DECLARE @i INT = 0, @sum INT = 0;
             LoopStart:
             SET @i = @i + 1;
             IF @i > @limit
             BEGIN
                 GOTO ExitProc;
             END
             SET @sum = @sum + @i;
             GOTO LoopStart;
             SET @sum = -1;
             ExitProc:
             SELECT @sum AS total;
         END;",
    );
    let r2 = query(&mut e, "EXEC dbo.usp_goto_demo @limit = 5;");
    assert_eq!(r2.rows[0][0].to_integer_i64(), Some(15));

    // 3. Undefined label produces an error
    let stmts = iridium_core::parse_batch("GOTO MissingLabel;").expect("should parse");
    let err = e.execute_batch(stmts).expect_err("undefined label must fail");
    assert!(
        err.to_string().to_lowercase().contains("missinglabel")
            || err.to_string().to_lowercase().contains("label"),
        "unexpected error: {err}"
    );
}

#[test]
fn grouping_sets_rollup_cube_and_grouping_functions() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE TABLE dbo.sales (region NVARCHAR(20), dept NVARCHAR(20), amount INT);
         INSERT INTO dbo.sales VALUES
           (N'East', N'A', 10),
           (N'East', N'B', 20),
           (N'West', N'A', 30);",
    );

    // 1. ROLLUP(region, dept) -> detail (3) + region subtotals (2) + grand total (1) = 6 rows
    let r_rollup = query(
        &mut e,
        "SELECT region, dept, SUM(amount) AS total,
                GROUPING(region) AS g_reg,
                GROUPING(dept) AS g_dept,
                GROUPING_ID(region, dept) AS gid
         FROM dbo.sales
         GROUP BY ROLLUP(region, dept)
         ORDER BY gid, region, dept",
    );
    assert_eq!(r_rollup.rows.len(), 6);
    // Detail row East/A: gid = 0
    assert_eq!(r_rollup.rows[0][0].to_string_value(), "East");
    assert_eq!(r_rollup.rows[0][1].to_string_value(), "A");
    assert_eq!(r_rollup.rows[0][2].to_integer_i64(), Some(10));
    assert_eq!(r_rollup.rows[0][5].to_integer_i64(), Some(0));
    // Region subtotal East/NULL: gid = 1 (g_reg=0, g_dept=1)
    assert_eq!(r_rollup.rows[3][0].to_string_value(), "East");
    assert!(r_rollup.rows[3][1].is_null());
    assert_eq!(r_rollup.rows[3][2].to_integer_i64(), Some(30));
    assert_eq!(r_rollup.rows[3][3].to_integer_i64(), Some(0));
    assert_eq!(r_rollup.rows[3][4].to_integer_i64(), Some(1));
    assert_eq!(r_rollup.rows[3][5].to_integer_i64(), Some(1));
    // Grand total NULL/NULL: gid = 3 (g_reg=1, g_dept=1)
    assert!(r_rollup.rows[5][0].is_null());
    assert!(r_rollup.rows[5][1].is_null());
    assert_eq!(r_rollup.rows[5][2].to_integer_i64(), Some(60));
    assert_eq!(r_rollup.rows[5][5].to_integer_i64(), Some(3));

    // 2. CUBE(region, dept) -> detail (3) + by region (2) + by dept (2) + grand total (1) = 8 rows
    let r_cube = query(
        &mut e,
        "SELECT region, dept, SUM(amount) AS total, GROUPING_ID(region, dept) AS gid
         FROM dbo.sales
         GROUP BY CUBE(region, dept)",
    );
    assert_eq!(r_cube.rows.len(), 8);

    // 3. Explicit GROUPING SETS ((region), (dept), ()) -> 2 + 2 + 1 = 5 rows
    let r_sets = query(
        &mut e,
        "SELECT region, dept, SUM(amount) AS total
         FROM dbo.sales
         GROUP BY GROUPING SETS ((region), (dept), ())",
    );
    assert_eq!(r_sets.rows.len(), 5);
}

#[test]
fn at_time_zone_dst_switchoffset_and_todatetimeoffset() {
    let mut e = Engine::new();

    // 1. Winter vs Summer DST for Eastern Standard Time (UTC-5 winter, UTC-4 summer)
    let r_winter = query(
        &mut e,
        "SELECT CAST('2025-01-15 17:00:00' AS DATETIME2) AT TIME ZONE 'UTC' AT TIME ZONE 'Eastern Standard Time'",
    );
    assert_eq!(
        r_winter.rows[0][0].to_string_value(),
        "2025-01-15 12:00:00 -05:00"
    );

    let r_summer = query(
        &mut e,
        "SELECT CAST('2025-07-15 16:00:00' AS DATETIME2) AT TIME ZONE 'UTC' AT TIME ZONE 'Eastern Standard Time'",
    );
    assert_eq!(
        r_summer.rows[0][0].to_string_value(),
        "2025-07-15 12:00:00 -04:00"
    );

    // 2. EU / UK DST (GMT Standard Time -> +01:00 in July, +00:00 in Jan) & Asia/Tokyo (+09:00 no DST)
    let r_london_summer = query(
        &mut e,
        "SELECT CAST('2025-07-15 12:00:00' AS DATETIME2) AT TIME ZONE 'GMT Standard Time'",
    );
    assert_eq!(
        r_london_summer.rows[0][0].to_string_value(),
        "2025-07-15 12:00:00 +01:00"
    );

    let r_tokyo = query(
        &mut e,
        "SELECT CAST('2025-01-15 00:00:00' AS DATETIME2) AT TIME ZONE 'UTC' AT TIME ZONE 'Tokyo Standard Time'",
    );
    assert_eq!(
        r_tokyo.rows[0][0].to_string_value(),
        "2025-01-15 09:00:00 +09:00"
    );

    // 3. Unknown timezone name raises an error
    let bad = iridium_core::parse_batch(
        "SELECT CAST('2025-01-15 12:00:00' AS DATETIME2) AT TIME ZONE 'Mars/Olympus'",
    )
    .expect("should parse");
    assert!(e.execute_batch(bad).is_err());

    // 4. TODATETIMEOFFSET and SWITCHOFFSET
    let r_to = query(
        &mut e,
        "SELECT TODATETIMEOFFSET(CAST('2025-03-01 10:30:00' AS DATETIME2), '-05:00'),
                TODATETIMEOFFSET(CAST('2025-03-01 10:30:00' AS DATETIME2), 120)",
    );
    assert_eq!(r_to.rows[0][0].to_string_value(), "2025-03-01 10:30:00 -05:00");
    assert_eq!(r_to.rows[0][1].to_string_value(), "2025-03-01 10:30:00 +02:00");

    let r_sw = query(
        &mut e,
        "SELECT SWITCHOFFSET(TODATETIMEOFFSET(CAST('2025-03-01 10:30:00' AS DATETIME2), '-05:00'), '+02:00')",
    );
    assert_eq!(r_sw.rows[0][0].to_string_value(), "2025-03-01 17:30:00 +02:00");
}

#[test]
fn collate_case_sensitive_like_in_order_by_and_group_by() {
    let mut e = Engine::new();
    exec(
        &mut e,
        "CREATE TABLE dbo.codes (code NVARCHAR(20));
         INSERT INTO dbo.codes VALUES (N'abc'), (N'ABC'), (N'Abc'), (N'abc');",
    );

    // 1. Case-sensitive LIKE vs default case-insensitive LIKE
    let r_like = query(
        &mut e,
        "SELECT
            CASE WHEN N'abc' COLLATE Latin1_General_CS_AS LIKE N'A%' THEN 1 ELSE 0 END AS cs_miss,
            CASE WHEN N'Abc' COLLATE Latin1_General_CS_AS LIKE N'A%' THEN 1 ELSE 0 END AS cs_hit,
            CASE WHEN N'abc' COLLATE Latin1_General_CI_AS LIKE N'A%' THEN 1 ELSE 0 END AS ci_hit",
    );
    assert_eq!(r_like.rows[0][0].to_integer_i64(), Some(0));
    assert_eq!(r_like.rows[0][1].to_integer_i64(), Some(1));
    assert_eq!(r_like.rows[0][2].to_integer_i64(), Some(1));

    // 2. Case-sensitive IN (...)
    let r_in = query(
        &mut e,
        "SELECT
            CASE WHEN N'abc' COLLATE Latin1_General_CS_AS IN (N'ABC', N'Abc') THEN 1 ELSE 0 END AS cs_miss,
            CASE WHEN N'abc' COLLATE Latin1_General_CS_AS IN (N'ABC', N'abc') THEN 1 ELSE 0 END AS cs_hit",
    );
    assert_eq!(r_in.rows[0][0].to_integer_i64(), Some(0));
    assert_eq!(r_in.rows[0][1].to_integer_i64(), Some(1));

    // 3. Case-sensitive ORDER BY (... COLLATE Latin1_General_BIN: ASCII 'A' (65) < 'a' (97))
    let r_ord = query(
        &mut e,
        "SELECT code FROM dbo.codes ORDER BY code COLLATE Latin1_General_BIN ASC",
    );
    let ordered: Vec<String> = r_ord.rows.iter().map(|r| r[0].to_string_value()).collect();
    assert_eq!(ordered, vec!["ABC", "Abc", "abc", "abc"]);

    // 4. Case-sensitive GROUP BY (... COLLATE Latin1_General_CS_AS separates 'abc', 'ABC', 'Abc')
    let r_grp = query(
        &mut e,
        "SELECT code COLLATE Latin1_General_CS_AS AS c, COUNT(*) AS cnt
         FROM dbo.codes
         GROUP BY code COLLATE Latin1_General_CS_AS
         ORDER BY c COLLATE Latin1_General_BIN ASC",
    );
    assert_eq!(r_grp.rows.len(), 3);
    assert_eq!(r_grp.rows[0][0].to_string_value(), "ABC");
    assert_eq!(r_grp.rows[0][1].to_integer_i64(), Some(1));
    assert_eq!(r_grp.rows[1][0].to_string_value(), "Abc");
    assert_eq!(r_grp.rows[1][1].to_integer_i64(), Some(1));
    assert_eq!(r_grp.rows[2][0].to_string_value(), "abc");
    assert_eq!(r_grp.rows[2][1].to_integer_i64(), Some(2));
}
