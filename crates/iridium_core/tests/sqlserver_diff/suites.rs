use super::harness::{DiffStep, DiffSuite};

pub fn all_suites() -> Vec<DiffSuite> {
    vec![
        classic_queries_and_joins(),
        ddl_constraints_and_filtered_indexes(),
        dml_merge_top_pivot_grouping(),
        aggregates_and_window_functions_sql2019(),
        window_functions_sql2022(),
        scalar_strings_math_dates_crypto_sql2019(),
        scalar_sql2022_dates_bitwise_logic(),
        json_and_tvfs_sql2019(),
        json_and_tvfs_sql2022(),
        programmability_triggers_and_transactions(),
        temporal_tables(),
        security_and_permissions(),
        xml_and_openxml(),
        catalog_and_metadata_parity(),
        enterprise_result_sets_schemabinding_ddl_triggers_applock_collation(),
        security_governance_and_query_fidelity(),
    ]
}

fn classic_queries_and_joins() -> DiffSuite {
    DiffSuite {
        name: "classic_queries_and_joins",
        category: "Core SQL & Joins",
        min_sql_version: 15,
        setup_batches: &[
            "CREATE TABLE dbo.Customers (
                CustomerId INT IDENTITY(1,1) PRIMARY KEY,
                FirstName NVARCHAR(50) NOT NULL,
                LastName NVARCHAR(50) NOT NULL,
                Email VARCHAR(100) UNIQUE,
                Phone VARCHAR(20) NULL,
                IsActive BIT NOT NULL DEFAULT 1
            );
            CREATE TABLE dbo.Products (
                ProductId INT PRIMARY KEY,
                Name NVARCHAR(100) NOT NULL,
                Price DECIMAL(10,2) NOT NULL,
                Stock INT NOT NULL DEFAULT 0,
                Category NVARCHAR(50)
            );
            CREATE TABLE dbo.Orders (
                OrderId INT PRIMARY KEY,
                CustomerId INT NOT NULL REFERENCES dbo.Customers(CustomerId),
                TotalAmount DECIMAL(10,2) NOT NULL
            );
            CREATE TABLE dbo.OrderItems (
                OrderId INT NOT NULL REFERENCES dbo.Orders(OrderId),
                ProductId INT NOT NULL REFERENCES dbo.Products(ProductId),
                Quantity INT NOT NULL,
                PRIMARY KEY (OrderId, ProductId)
            );
            CREATE TABLE dbo.Employees (
                EmployeeId INT PRIMARY KEY,
                FirstName NVARCHAR(50) NOT NULL,
                LastName NVARCHAR(50) NOT NULL,
                ManagerId INT NULL,
                Salary DECIMAL(10,2) NOT NULL
            );",
            "SET IDENTITY_INSERT dbo.Customers ON;
            INSERT INTO dbo.Customers (CustomerId, FirstName, LastName, Email, Phone, IsActive) VALUES
                (1, N'Alice', N'Smith', 'alice@example.com', '555-0101', 1),
                (2, N'Bob', N'Jones', 'bob@example.com', NULL, 1),
                (3, N'Carol', N'Lee', 'carol@example.com', '555-0103', 0);
            SET IDENTITY_INSERT dbo.Customers OFF;
            INSERT INTO dbo.Products VALUES
                (1, N'Laptop Pro', 1200.00, 10, N'Electronics'),
                (2, N'Mouse', 25.50, 50, N'Electronics'),
                (3, N'Desk Chair', 300.00, 5, N'Furniture');
            INSERT INTO dbo.Orders VALUES
                (100, 1, 1225.50),
                (101, 2, 300.00);
            INSERT INTO dbo.OrderItems VALUES
                (100, 1, 1),
                (100, 2, 1),
                (101, 3, 1);
            INSERT INTO dbo.Employees VALUES
                (1, N'Dana', N'Boss', NULL, 150000.00),
                (2, N'Evan', N'Dev', 1, 110000.00);",
            "CREATE VIEW dbo.vCustomerOrders AS
            SELECT c.CustomerId, c.FirstName, COUNT(o.OrderId) AS OrderCount, COALESCE(SUM(o.TotalAmount), 0) AS TotalSpent
            FROM dbo.Customers c
            LEFT JOIN dbo.Orders o ON c.CustomerId = o.CustomerId
            GROUP BY c.CustomerId, c.FirstName;",
        ],
        steps: &[
            DiffStep::Query {
                label: "literals_and_arithmetic",
                sql: "SELECT 1 AS n, 'hello' AS s, 1 + 2 * 3 / 2 AS r, NULL AS nil, CAST('123' AS INT) AS ci;",
                ordered: true,
            },
            DiffStep::Query {
                label: "basic_aggregates_and_distinct",
                sql: "SELECT COUNT(*) AS c, COUNT(DISTINCT Category) AS dc, MIN(Price) AS minp, MAX(Price) AS maxp, SUM(Price) AS sump FROM dbo.Products;",
                ordered: true,
            },
            DiffStep::Query {
                label: "group_by_and_having",
                sql: "SELECT Category, COUNT(*) AS cnt FROM dbo.Products GROUP BY Category HAVING COUNT(*) >= 1 ORDER BY Category;",
                ordered: true,
            },
            DiffStep::Query {
                label: "inner_left_self_joins",
                sql: "SELECT e.FirstName, m.FirstName AS Manager FROM dbo.Employees e LEFT JOIN dbo.Employees m ON e.ManagerId = m.EmployeeId ORDER BY e.EmployeeId;",
                ordered: true,
            },
            DiffStep::Query {
                label: "view_query",
                sql: "SELECT CustomerId, FirstName, OrderCount, TotalSpent FROM dbo.vCustomerOrders ORDER BY CustomerId;",
                ordered: true,
            },
            DiffStep::Query {
                label: "correlated_subquery_and_exists",
                sql: "SELECT p.Name, (SELECT COALESCE(SUM(Quantity), 0) FROM dbo.OrderItems oi WHERE oi.ProductId = p.ProductId) AS QtySold
                      FROM dbo.Products p
                      WHERE EXISTS (SELECT 1 FROM dbo.OrderItems oi WHERE oi.ProductId = p.ProductId)
                      ORDER BY p.ProductId;",
                ordered: true,
            },
            DiffStep::Query {
                label: "case_coalesce_isnull_like_offset",
                sql: "SELECT Name,
                             CASE WHEN Price > 500 THEN 'Pro' WHEN Price > 100 THEN 'Mid' ELSE 'Budget' END AS Tier
                      FROM dbo.Products
                      WHERE Name LIKE 'Laptop%' OR Price < 400
                      ORDER BY ProductId
                      OFFSET 1 ROWS FETCH NEXT 2 ROWS ONLY;",
                ordered: true,
            },
            DiffStep::ExpectErr {
                label: "err_missing_table",
                sql: "SELECT * FROM dbo.DoesNotExist;",
            },
            DiffStep::ExpectErr {
                label: "err_missing_column",
                sql: "SELECT FakeCol FROM dbo.Customers;",
            },
            DiffStep::ExpectErr {
                label: "err_divide_by_zero",
                sql: "SELECT 1 / 0;",
            },
            DiffStep::ExpectErr {
                label: "err_invalid_cast",
                sql: "SELECT CAST('not_a_number' AS INT);",
            },
            DiffStep::ExpectErr {
                label: "err_syntax",
                sql: "SELECT FROM WHERE;",
            },
        ],
    }
}

fn ddl_constraints_and_filtered_indexes() -> DiffSuite {
    DiffSuite {
        name: "ddl_constraints_and_filtered_indexes",
        category: "DDL, Constraints & Filtered Indexes",
        min_sql_version: 15,
        setup_batches: &[
            "CREATE TYPE dbo.AccountCode FROM VARCHAR(20) NOT NULL;
            CREATE TABLE dbo.Parents (Id INT PRIMARY KEY);
            INSERT INTO dbo.Parents VALUES (1), (2);
            CREATE TABLE dbo.Accounts (
                Id INT PRIMARY KEY,
                ParentId INT NULL,
                ExtCode VARCHAR(20) NULL,
                IsDeleted BIT NOT NULL,
                Amount INT NOT NULL,
                RowVer ROWVERSION,
                CONSTRAINT CK_Accounts_Amount CHECK (Amount > 0),
                CONSTRAINT FK_Accounts_Parent FOREIGN KEY (ParentId) REFERENCES dbo.Parents(Id) ON DELETE SET NULL
            );
            CREATE UNIQUE INDEX IX_Accounts_ActiveExt
                ON dbo.Accounts (ExtCode)
                WHERE ExtCode IS NOT NULL AND IsDeleted = 0;",
        ],
        steps: &[
            DiffStep::ExecOk {
                label: "filtered_index_allows_nulls_and_deleted_dups",
                sql: "INSERT INTO dbo.Accounts (Id, ParentId, ExtCode, IsDeleted, Amount) VALUES
                        (1, 1, NULL, 0, 10),
                        (2, 1, NULL, 0, 20),
                        (3, 2, 'EXT1', 1, 30),
                        (4, 2, 'EXT1', 1, 40),
                        (5, 1, 'EXT1', 0, 50);",
            },
            DiffStep::ExpectErr {
                label: "filtered_index_rejects_active_dup_insert",
                sql: "INSERT INTO dbo.Accounts (Id, ParentId, ExtCode, IsDeleted, Amount) VALUES (6, 1, 'EXT1', 0, 60);",
            },
            DiffStep::ExpectErr {
                label: "filtered_index_rejects_active_dup_update",
                sql: "UPDATE dbo.Accounts SET IsDeleted = 0 WHERE Id = 3;",
            },
            DiffStep::ExecOk {
                label: "fk_on_delete_set_null",
                sql: "DELETE FROM dbo.Parents WHERE Id = 2;",
            },
            DiffStep::Query {
                label: "verify_fk_set_null_applied",
                sql: "SELECT Id, ParentId FROM dbo.Accounts WHERE Id IN (3, 4) ORDER BY Id;",
                ordered: true,
            },
            DiffStep::ExecOk {
                label: "nocheck_allows_violating_row",
                sql: "ALTER TABLE dbo.Accounts NOCHECK CONSTRAINT CK_Accounts_Amount;
                      INSERT INTO dbo.Accounts (Id, ParentId, ExtCode, IsDeleted, Amount) VALUES (7, 1, 'EXT2', 0, -99);",
            },
            DiffStep::ExpectErr {
                label: "with_check_rejects_existing_violation",
                sql: "ALTER TABLE dbo.Accounts WITH CHECK CHECK CONSTRAINT CK_Accounts_Amount;",
            },
            DiffStep::ExecOk {
                label: "fix_row_then_with_check_succeeds",
                sql: "UPDATE dbo.Accounts SET Amount = 99 WHERE Id = 7;
                      ALTER TABLE dbo.Accounts WITH CHECK CHECK CONSTRAINT CK_Accounts_Amount;",
            },
            DiffStep::ExpectErr {
                label: "rowversion_rejects_explicit_update",
                sql: "UPDATE dbo.Accounts SET RowVer = 0x0000000000000001 WHERE Id = 1;",
            },
            DiffStep::Query {
                label: "scalar_udt_variable_and_table_column",
                sql: "DECLARE @code dbo.AccountCode = 'ACC-01';
                      DECLARE @t TABLE (C dbo.AccountCode);
                      INSERT INTO @t VALUES (@code);
                      SELECT @code, C FROM @t;",
                ordered: true,
            },
        ],
    }
}

fn dml_merge_top_pivot_grouping() -> DiffSuite {
    DiffSuite {
        name: "dml_merge_top_pivot_grouping",
        category: "DML, MERGE, TOP, PIVOT & GROUPING SETS",
        min_sql_version: 15,
        setup_batches: &[
            "CREATE TABLE dbo.Target (Id INT PRIMARY KEY, Val VARCHAR(20));
            INSERT INTO dbo.Target VALUES (1, 'old1'), (2, 'old2'), (3, 'remove_me');
            CREATE TABLE dbo.Source (Id INT PRIMARY KEY, Val VARCHAR(20));
            INSERT INTO dbo.Source VALUES (1, 'new1'), (4, 'ins4');
            CREATE TABLE dbo.Scores (Id INT PRIMARY KEY, Dept VARCHAR(10), Qtr VARCHAR(2), Amt INT);
            INSERT INTO dbo.Scores VALUES
                (1, 'A', 'Q1', 100), (2, 'A', 'Q2', 200),
                (3, 'B', 'Q1', 200), (4, 'B', 'Q2', 50);",
        ],
        steps: &[
            DiffStep::Query {
                label: "merge_with_output_action",
                sql: "MERGE dbo.Target AS t
                      USING dbo.Source AS s ON t.Id = s.Id
                      WHEN MATCHED THEN UPDATE SET t.Val = s.Val
                      WHEN NOT MATCHED BY TARGET THEN INSERT (Id, Val) VALUES (s.Id, s.Val)
                      WHEN NOT MATCHED BY SOURCE AND t.Id = 3 THEN DELETE
                      OUTPUT $action AS Act, COALESCE(inserted.Id, deleted.Id) AS RowId;",
                ordered: false,
            },
            DiffStep::Query {
                label: "top_with_ties",
                sql: "SELECT TOP (1) WITH TIES Id, Amt FROM dbo.Scores ORDER BY Amt DESC;",
                ordered: false,
            },
            DiffStep::Query {
                label: "top_percent",
                sql: "SELECT TOP (50) PERCENT Id FROM dbo.Scores ORDER BY Id;",
                ordered: true,
            },
            DiffStep::Query {
                label: "pivot_and_unpivot",
                sql: "SELECT Dept, Q1, Q2
                      FROM (SELECT Dept, Qtr, Amt FROM dbo.Scores) src
                      PIVOT (SUM(Amt) FOR Qtr IN ([Q1], [Q2])) p
                      ORDER BY Dept;",
                ordered: true,
            },
            DiffStep::Query {
                label: "grouping_sets_rollup",
                sql:
                    "SELECT Dept, Qtr, SUM(Amt) AS Total, GROUPING(Dept) AS gD, GROUPING(Qtr) AS gQ
                      FROM dbo.Scores
                      GROUP BY ROLLUP (Dept, Qtr);",
                ordered: false,
            },
            DiffStep::Query {
                label: "recursive_cte",
                sql: "WITH Nums(n) AS (
                          SELECT 1
                          UNION ALL
                          SELECT n + 1 FROM Nums WHERE n < 5
                      )
                      SELECT n FROM Nums ORDER BY n;",
                ordered: true,
            },
        ],
    }
}

fn aggregates_and_window_functions_sql2019() -> DiffSuite {
    DiffSuite {
        name: "aggregates_and_window_functions_sql2019",
        category: "Aggregates & Window Functions (SQL 2019+)",
        min_sql_version: 15,
        setup_batches: &[
            "CREATE TABLE dbo.Nums (Grp VARCHAR(1), Ord INT, Val INT NULL);
            INSERT INTO dbo.Nums VALUES
                ('A', 1, 2), ('A', 2, 4), ('A', 3, 4), ('A', 4, 4),
                ('A', 5, 5), ('A', 6, 5), ('A', 7, 7), ('A', 8, 9), ('A', 9, NULL),
                ('B', 1, 10), ('B', 2, 20);",
        ],
        steps: &[
            DiffStep::Query {
                label: "count_big_stats_checksum_distinct",
                sql: "SELECT
                        Grp,
                        COUNT_BIG(*) AS c_all,
                        COUNT_BIG(DISTINCT Val) AS c_dist,
                        SUM(DISTINCT Val) AS s_dist,
                        AVG(DISTINCT Val) AS a_dist,
                        STDEV(Val) AS sd,
                        STDEVP(Val) AS sdp,
                        VAR(Val) AS v,
                        VARP(Val) AS vp,
                        CHECKSUM_AGG(Val) AS chk
                      FROM dbo.Nums
                      GROUP BY Grp
                      ORDER BY Grp;",
                ordered: true,
            },
            DiffStep::Query {
                label: "string_agg_within_group",
                sql: "SELECT Grp, STRING_AGG(CAST(Val AS VARCHAR(10)), ',') WITHIN GROUP (ORDER BY Ord) AS Joined
                      FROM dbo.Nums
                      WHERE Val IS NOT NULL
                      GROUP BY Grp
                      ORDER BY Grp;",
                ordered: true,
            },
            DiffStep::Query {
                label: "window_ranking_and_cume_dist",
                sql: "SELECT
                        Ord,
                        ROW_NUMBER() OVER (ORDER BY Val) AS rn,
                        RANK() OVER (ORDER BY Val) AS rnk,
                        DENSE_RANK() OVER (ORDER BY Val) AS drnk,
                        PERCENT_RANK() OVER (ORDER BY Val) AS pr,
                        CUME_DIST() OVER (ORDER BY Val) AS cd,
                        LAG(Val, 1, -1) OVER (ORDER BY Ord) AS prev_v,
                        LEAD(Val, 1, -1) OVER (ORDER BY Ord) AS next_v
                      FROM dbo.Nums
                      WHERE Grp = 'A' AND Val IS NOT NULL
                      ORDER BY Ord;",
                ordered: true,
            },
        ],
    }
}

fn window_functions_sql2022() -> DiffSuite {
    DiffSuite {
        name: "window_functions_sql2022",
        category: "Window Functions (SQL 2022)",
        min_sql_version: 16,
        setup_batches: &[
            "CREATE TABLE dbo.Series (Grp VARCHAR(1), Ord INT, Val INT NULL);
            INSERT INTO dbo.Series VALUES
                ('A', 1, 10), ('A', 2, NULL), ('A', 3, NULL), ('A', 4, 40),
                ('B', 1, NULL), ('B', 2, 20), ('B', 3, 30);",
        ],
        steps: &[
            DiffStep::Query {
                label: "ignore_nulls_and_respect_nulls",
                sql: "SELECT
                        Grp,
                        Ord,
                        LAG(Val) IGNORE NULLS OVER (PARTITION BY Grp ORDER BY Ord) AS lag_ign,
                        LEAD(Val) IGNORE NULLS OVER (PARTITION BY Grp ORDER BY Ord) AS lead_ign,
                        FIRST_VALUE(Val) IGNORE NULLS OVER (PARTITION BY Grp ORDER BY Ord ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING) AS fv_ign,
                        LAST_VALUE(Val) IGNORE NULLS OVER (PARTITION BY Grp ORDER BY Ord ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING) AS lv_ign
                      FROM dbo.Series
                      ORDER BY Grp, Ord;",
                ordered: true,
            },
            DiffStep::Query {
                label: "named_window_clause",
                sql: "SELECT
                        Grp,
                        Ord,
                        SUM(COALESCE(Val, 0)) OVER w_grp AS grp_sum,
                        SUM(COALESCE(Val, 0)) OVER w_run AS run_sum,
                        ROW_NUMBER() OVER (w_grp ORDER BY Ord DESC) AS rev_rn
                      FROM dbo.Series
                      WINDOW
                        w_grp AS (PARTITION BY Grp),
                        w_run AS (w_grp ORDER BY Ord ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)
                      ORDER BY Grp, Ord;",
                ordered: true,
            },
        ],
    }
}

fn scalar_strings_math_dates_crypto_sql2019() -> DiffSuite {
    DiffSuite {
        name: "scalar_strings_math_dates_crypto_sql2019",
        category: "Scalar Built-ins (SQL 2019+)",
        min_sql_version: 15,
        setup_batches: &[],
        steps: &[
            DiffStep::Query {
                label: "string_functions",
                sql: "SELECT
                        LEN('hello   '),
                        DATALENGTH('hello   '),
                        SUBSTRING('abcdef', 2, 3),
                        CHARINDEX('cd', 'abcdef'),
                        STUFF('abcdef', 2, 3, 'XY'),
                        TRANSLATE('2*[3+4]/{7-2}', '[]{}', '()()'),
                        CONCAT_WS('-', 'a', NULL, 'b', 'c'),
                        QUOTENAME('my]col'),
                        PARSENAME('srv.db.sch.obj', 1),
                        PARSENAME('srv.db.sch.obj', 2),
                        SOUNDEX('Smith'),
                        DIFFERENCE('Smith', 'Smythe');",
                ordered: true,
            },
            DiffStep::Query {
                label: "math_functions",
                sql: "SELECT
                        ABS(-42),
                        CEILING(4.2),
                        FLOOR(4.8),
                        ROUND(123.456, 2),
                        SIGN(-10),
                        POWER(2, 10),
                        SQRT(81),
                        SQUARE(9);",
                ordered: true,
            },
            DiffStep::Query {
                label: "datetime_constructors_and_arithmetic",
                sql: "SELECT
                        DATEFROMPARTS(2025, 6, 15),
                        TIMEFROMPARTS(14, 30, 15, 500, 3),
                        SMALLDATETIMEFROMPARTS(2025, 6, 15, 9, 45),
                        DATETIME2FROMPARTS(2025, 6, 15, 14, 30, 15, 5, 1),
                        DATETIMEOFFSETFROMPARTS(2025, 6, 15, 14, 30, 15, 0, -5, 0, 0),
                        EOMONTH('2024-02-10'),
                        DATEDIFF(day, '2025-01-01', '2025-01-11'),
                        DATEDIFF_BIG(second, '2025-01-01 00:00:00', '2025-01-01 00:01:30');",
                ordered: true,
            },
            DiffStep::Query {
                label: "compress_decompress_and_hashbytes",
                sql: "SELECT
                        CAST(DECOMPRESS(COMPRESS('hello gzip roundtrip')) AS VARCHAR(MAX)),
                        HASHBYTES('MD5', 'hello'),
                        HASHBYTES('SHA1', 'hello'),
                        HASHBYTES('SHA2_256', 'hello'),
                        HASHBYTES('SHA2_512', 'hello'),
                        HASHBYTES('SHA2_256', N'hello');",
                ordered: true,
            },
            DiffStep::Query {
                label: "logic_choose_iif_nullif",
                sql: "SELECT
                        CHOOSE(2, 'alpha', 'beta', 'gamma'),
                        IIF(10 > 5, 'yes', 'no'),
                        NULLIF(5, 5),
                        NULLIF(5, 6),
                        ISNUMERIC('123.45'),
                        ISNUMERIC('abc');",
                ordered: true,
            },
        ],
    }
}

fn scalar_sql2022_dates_bitwise_logic() -> DiffSuite {
    DiffSuite {
        name: "scalar_sql2022_dates_bitwise_logic",
        category: "Scalar Built-ins (SQL 2022)",
        min_sql_version: 16,
        setup_batches: &[],
        steps: &[
            DiffStep::Query {
                label: "datetrunc_and_date_bucket",
                sql: "SELECT
                        DATETRUNC(year, CAST('2025-06-15 14:35:22' AS DATETIME2)),
                        DATETRUNC(quarter, CAST('2025-06-15 14:35:22' AS DATETIME2)),
                        DATETRUNC(month, CAST('2025-06-15 14:35:22' AS DATETIME2)),
                        DATETRUNC(day, CAST('2025-06-15 14:35:22' AS DATETIME2)),
                        DATETRUNC(hour, CAST('2025-06-15 14:35:22' AS DATETIME2)),
                        DATE_BUCKET(day, 7, CAST('2025-01-15' AS DATE), CAST('2025-01-01' AS DATE)),
                        DATE_BUCKET(hour, 4, CAST('2025-01-01 11:20:00' AS DATETIME2), CAST('2025-01-01 00:00:00' AS DATETIME2));",
                ordered: true,
            },
            DiffStep::Query {
                label: "bitwise_sql2022",
                sql: "SELECT
                        LEFT_SHIFT(1, 4),
                        RIGHT_SHIFT(32, 3),
                        BIT_COUNT(CAST(21 AS INT)),
                        GET_BIT(CAST(21 AS INT), 0),
                        GET_BIT(CAST(21 AS INT), 1),
                        GET_BIT(CAST(21 AS INT), 2),
                        SET_BIT(CAST(0 AS INT), 3),
                        SET_BIT(CAST(15 AS INT), 1, 0);",
                ordered: true,
            },
            DiffStep::Query {
                label: "is_distinct_from_greatest_least",
                sql: "SELECT
                        CASE WHEN NULL IS DISTINCT FROM NULL THEN 1 ELSE 0 END,
                        CASE WHEN NULL IS NOT DISTINCT FROM NULL THEN 1 ELSE 0 END,
                        CASE WHEN 1 IS DISTINCT FROM NULL THEN 1 ELSE 0 END,
                        CASE WHEN 5 IS NOT DISTINCT FROM 5 THEN 1 ELSE 0 END,
                        GREATEST(10, 30, 20),
                        LEAST(10, 30, 20);",
                ordered: true,
            },
        ],
    }
}

fn json_and_tvfs_sql2019() -> DiffSuite {
    DiffSuite {
        name: "json_and_tvfs_sql2019",
        category: "JSON & Built-in TVFs (SQL 2019+)",
        min_sql_version: 15,
        setup_batches: &[],
        steps: &[
            DiffStep::Query {
                label: "json_value_query_modify_isjson",
                sql: "SELECT
                        JSON_VALUE('{\"a\":1,\"b\":{\"c\":\"hi\"}}', '$.b.c'),
                        JSON_QUERY('{\"arr\":[1,2,3]}', '$.arr'),
                        JSON_MODIFY('{\"x\":1}', '$.x', 2),
                        ISJSON('{\"a\":1}'),
                        ISJSON('not json');",
                ordered: true,
            },
            DiffStep::Query {
                label: "openjson_with_schema",
                sql: "SELECT Id, Name
                      FROM OPENJSON('[{\"id\":1,\"name\":\"A\"},{\"id\":2,\"name\":\"B\"}]')
                      WITH (Id INT '$.id', Name VARCHAR(20) '$.name')
                      ORDER BY Id;",
                ordered: true,
            },
            DiffStep::Query {
                label: "string_split_basic",
                sql: "SELECT value FROM STRING_SPLIT('red,green,blue', ',') ORDER BY value;",
                ordered: true,
            },
        ],
    }
}

fn json_and_tvfs_sql2022() -> DiffSuite {
    DiffSuite {
        name: "json_and_tvfs_sql2022",
        category: "JSON & Built-in TVFs (SQL 2022)",
        min_sql_version: 16,
        setup_batches: &[],
        steps: &[
            DiffStep::Query {
                label: "json_object_array_path_exists_isjson2",
                sql: "SELECT
                        JSON_OBJECT('a': 1, 'b': NULL NULL ON NULL),
                        JSON_OBJECT('a': 1, 'b': NULL ABSENT ON NULL),
                        JSON_ARRAY(1, NULL, 'x' ABSENT ON NULL),
                        JSON_ARRAY(1, NULL, 'x' NULL ON NULL),
                        JSON_PATH_EXISTS('{\"a\":{\"b\":1}}', '$.a.b'),
                        JSON_PATH_EXISTS('{\"a\":{\"b\":1}}', '$.a.z'),
                        ISJSON('{\"a\":1}', OBJECT),
                        ISJSON('[1,2]', ARRAY),
                        ISJSON('42', SCALAR);",
                ordered: true,
            },
            DiffStep::Query {
                label: "generate_series_and_string_split_ordinal",
                sql: "SELECT value FROM GENERATE_SERIES(1, 9, 2) ORDER BY value;",
                ordered: true,
            },
            DiffStep::Query {
                label: "string_split_with_ordinal",
                sql: "SELECT value, ordinal FROM STRING_SPLIT('x,y,z', ',', 1) ORDER BY ordinal;",
                ordered: true,
            },
        ],
    }
}

fn programmability_triggers_and_transactions() -> DiffSuite {
    DiffSuite {
        name: "programmability_triggers_and_transactions",
        category: "Programmability, Triggers & Transactions",
        min_sql_version: 15,
        setup_batches: &[
            "CREATE TYPE dbo.IntList AS TABLE (Val INT);
            CREATE SEQUENCE dbo.SeqOrder AS INT START WITH 100 INCREMENT BY 5;
            CREATE TABLE dbo.AuditEvents (Note VARCHAR(50), Nest INT, UpdatedMask VARBINARY(4));
            CREATE TABLE dbo.Items (Id INT PRIMARY KEY, Status VARCHAR(20), Qty INT);",
            "CREATE TRIGGER dbo.trItemsAfterUpdate ON dbo.Items AFTER UPDATE AS
            BEGIN
                SET NOCOUNT ON;
                IF UPDATE(Status)
                    INSERT INTO dbo.AuditEvents VALUES ('status_changed', TRIGGER_NESTLEVEL(), COLUMNS_UPDATED());
                ELSE
                    INSERT INTO dbo.AuditEvents VALUES ('other_changed', TRIGGER_NESTLEVEL(), COLUMNS_UPDATED());
            END;",
            "CREATE PROCEDURE dbo.usp_SumList @list dbo.IntList READONLY, @total INT OUTPUT AS
            BEGIN
                SELECT @total = SUM(Val) FROM @list;
            END;",
            "CREATE FUNCTION dbo.ufn_Double(@x INT) RETURNS INT AS
            BEGIN
                RETURN @x * 2;
            END;",
        ],
        steps: &[
            DiffStep::ExecOk {
                label: "trigger_update_and_columns_updated",
                sql: "INSERT INTO dbo.Items VALUES (1, 'NEW', 10);
                      UPDATE dbo.Items SET Status = 'PAID' WHERE Id = 1;
                      UPDATE dbo.Items SET Qty = 20 WHERE Id = 1;",
            },
            DiffStep::Query {
                label: "verify_trigger_audit_log",
                sql: "SELECT Note, Nest, UpdatedMask FROM dbo.AuditEvents ORDER BY Note;",
                ordered: true,
            },
            DiffStep::Query {
                label: "tvp_output_and_scalar_udf_and_sequence",
                sql: "DECLARE @t dbo.IntList;
                      INSERT INTO @t VALUES (10), (20), (30);
                      DECLARE @sum INT;
                      EXEC dbo.usp_SumList @list = @t, @total = @sum OUTPUT;
                      DECLARE @s1 INT = NEXT VALUE FOR dbo.SeqOrder;
                      DECLARE @s2 INT = NEXT VALUE FOR dbo.SeqOrder;
                      SELECT @sum AS TvpSum, dbo.ufn_Double(21) AS Doubled, @s1 AS S1, @s2 AS S2;",
                ordered: true,
            },
            DiffStep::Query {
                label: "xact_abort_doomed_tx_in_try_catch",
                sql: "SET XACT_ABORT ON;
                      DECLARE @xs INT = 0;
                      BEGIN TRY
                          BEGIN TRAN;
                          INSERT INTO dbo.Items VALUES (1, 'DUP', 1);
                          COMMIT TRAN;
                      END TRY
                      BEGIN CATCH
                          SET @xs = XACT_STATE();
                          IF XACT_STATE() <> 0 ROLLBACK TRAN;
                      END CATCH;
                      SET XACT_ABORT OFF;
                      SELECT @xs AS DoomedXactState, @@TRANCOUNT AS FinalTranCount;",
                ordered: true,
            },
            DiffStep::Query {
                label: "table_var_rollback_immunity",
                sql: "DECLARE @tv TABLE (Id INT);
                      BEGIN TRAN;
                      INSERT INTO @tv VALUES (42);
                      ROLLBACK TRAN;
                      SELECT Id FROM @tv;",
                ordered: true,
            },
        ],
    }
}

fn temporal_tables() -> DiffSuite {
    DiffSuite {
        name: "temporal_tables",
        category: "System-Versioned Temporal Tables",
        min_sql_version: 15,
        setup_batches: &[
            "CREATE TABLE dbo.Contracts (
                ContractId INT PRIMARY KEY,
                Rate INT NOT NULL,
                ValidFrom DATETIME2 GENERATED ALWAYS AS ROW START NOT NULL,
                ValidTo DATETIME2 GENERATED ALWAYS AS ROW END NOT NULL,
                PERIOD FOR SYSTEM_TIME (ValidFrom, ValidTo)
            ) WITH (SYSTEM_VERSIONING = ON (HISTORY_TABLE = dbo.ContractsHistory));",
        ],
        steps: &[
            DiffStep::ExecOk {
                label: "insert_temporal",
                sql: "INSERT INTO dbo.Contracts (ContractId, Rate) VALUES (1, 100);",
            },
            DiffStep::ExecOk {
                label: "update_temporal",
                sql: "WAITFOR DELAY '00:00:00.020'; UPDATE dbo.Contracts SET Rate = 125 WHERE ContractId = 1;",
            },
            DiffStep::ExecOk {
                label: "delete_temporal",
                sql: "WAITFOR DELAY '00:00:00.020'; DELETE FROM dbo.Contracts WHERE ContractId = 1;",
            },
            DiffStep::Query {
                label: "for_system_time_all_returns_both_versions",
                sql: "SELECT ContractId, Rate FROM dbo.Contracts FOR SYSTEM_TIME ALL ORDER BY Rate;",
                ordered: true,
            },
            DiffStep::Query {
                label: "sys_periods_exposed",
                sql: "SELECT name, start_column_id, end_column_id FROM sys.periods;",
                ordered: true,
            },
        ],
    }
}

fn security_and_permissions() -> DiffSuite {
    DiffSuite {
        name: "security_and_permissions",
        category: "Security, Roles & EXECUTE AS",
        min_sql_version: 15,
        setup_batches: &[
            "CREATE TABLE dbo.Vault (Id INT PRIMARY KEY, Secret VARCHAR(20));
            INSERT INTO dbo.Vault VALUES (1, 'alpha');
            CREATE USER app_reader WITHOUT LOGIN;
            CREATE USER app_blocked WITHOUT LOGIN;
            CREATE ROLE r_read;
            ALTER ROLE r_read ADD MEMBER app_reader;
            ALTER ROLE r_read ADD MEMBER app_blocked;
            GRANT SELECT ON dbo.Vault TO r_read;
            DENY SELECT ON dbo.Vault TO app_blocked;",
        ],
        steps: &[
            DiffStep::Query {
                label: "execute_as_reader_allowed_select",
                sql: "EXECUTE AS USER = 'app_reader';
                      DECLARE @u NVARCHAR(128) = USER_NAME();
                      DECLARE @s VARCHAR(20);
                      SELECT @s = Secret FROM dbo.Vault WHERE Id = 1;
                      REVERT;
                      SELECT @u, @s, USER_NAME();",
                ordered: true,
            },
            DiffStep::ExpectErr {
                label: "execute_as_reader_denied_insert",
                sql: "EXECUTE AS USER = 'app_reader';
                      INSERT INTO dbo.Vault VALUES (2, 'nope');",
            },
            DiffStep::ExecOk {
                label: "revert_after_error_if_needed",
                sql: "REVERT;",
            },
            DiffStep::ExpectErr {
                label: "execute_as_blocked_deny_overrides_role_grant",
                sql: "EXECUTE AS USER = 'app_blocked';
                      SELECT Secret FROM dbo.Vault;",
            },
            DiffStep::ExecOk {
                label: "revert_blocked",
                sql: "REVERT;",
            },
        ],
    }
}

fn xml_and_openxml() -> DiffSuite {
    DiffSuite {
        name: "xml_and_openxml",
        category: "XML & Legacy OPENXML",
        min_sql_version: 15,
        setup_batches: &[],
        steps: &[
            DiffStep::Query {
                label: "openxml_attr_and_parent_xpath",
                sql: "DECLARE @hdoc INT;
                      DECLARE @xml NVARCHAR(MAX) = N'<ROOT><Cust Id=\"C1\" Name=\"Alice\"><Ord No=\"101\" Amt=\"50\"/><Ord No=\"102\" Amt=\"75\"/></Cust><Cust Id=\"C2\" Name=\"Bob\"><Ord No=\"201\" Amt=\"99\"/></Cust></ROOT>';
                      EXEC sp_xml_preparedocument @hdoc OUTPUT, @xml;
                      SELECT CustId, Name, OrdNo, Amt
                      FROM OPENXML(@hdoc, '/ROOT/Cust/Ord', 1)
                      WITH (
                          CustId VARCHAR(10) '../@Id',
                          Name VARCHAR(20) '../@Name',
                          OrdNo INT '@No',
                          Amt INT '@Amt'
                      )
                      ORDER BY OrdNo;",
                ordered: true,
            },
            DiffStep::Query {
                label: "openxml_element_centric",
                sql: "DECLARE @h INT;
                      EXEC sp_xml_preparedocument @h OUTPUT, N'<Items><Item><Code>A1</Code><Qty>4</Qty></Item><Item><Code>B2</Code><Qty>9</Qty></Item></Items>';
                      SELECT Code, Qty FROM OPENXML(@h, '/Items/Item', 2) WITH (Code VARCHAR(10), Qty INT) ORDER BY Code;",
                ordered: true,
            },
        ],
    }
}

fn catalog_and_metadata_parity() -> DiffSuite {
    DiffSuite {
        name: "catalog_and_metadata_parity",
        category: "Catalog & System Metadata",
        min_sql_version: 15,
        setup_batches: &[
            "CREATE TABLE dbo.OrdersMeta (
                Id INT PRIMARY KEY,
                Code VARCHAR(20) NULL,
                Amount INT NOT NULL,
                CONSTRAINT CK_OrdersMeta_Amt CHECK (Amount >= 0)
            );
            CREATE UNIQUE INDEX IX_OrdersMeta_Code ON dbo.OrdersMeta (Code) WHERE Code IS NOT NULL;",
            "CREATE VIEW dbo.vOrdersMeta AS SELECT Id, Code FROM dbo.OrdersMeta;",
            "EXEC sp_addextendedproperty @name = N'MS_Description', @value = N'Order table', @level0type = N'SCHEMA', @level0name = N'dbo', @level1type = N'TABLE', @level1name = N'OrdersMeta';",
        ],
        steps: &[
            DiffStep::Query {
                label: "sys_indexes_has_filter",
                sql: "SELECT name, is_unique, has_filter FROM sys.indexes WHERE name = 'IX_OrdersMeta_Code';",
                ordered: true,
            },
            DiffStep::Query {
                label: "sys_check_constraints_disabled_and_trusted",
                sql: "ALTER TABLE dbo.OrdersMeta NOCHECK CONSTRAINT CK_OrdersMeta_Amt;
                      SELECT name, is_disabled, is_not_trusted FROM sys.check_constraints WHERE name = 'CK_OrdersMeta_Amt';",
                ordered: true,
            },
            DiffStep::Query {
                label: "sys_extended_properties",
                sql: "SELECT name, CAST(value AS NVARCHAR(100)) FROM sys.extended_properties WHERE name = 'MS_Description';",
                ordered: true,
            },
            DiffStep::Query {
                label: "sys_sql_expression_dependencies_view",
                sql: "SELECT DISTINCT referenced_schema_name = COALESCE(referenced_schema_name, 'dbo'), referenced_entity_name
                      FROM sys.sql_expression_dependencies
                      WHERE referencing_id = OBJECT_ID('dbo.vOrdersMeta');",
                ordered: true,
            },
        ],
    }
}

fn enterprise_result_sets_schemabinding_ddl_triggers_applock_collation() -> DiffSuite {
    DiffSuite {
        name: "enterprise_result_sets_schemabinding_ddl_triggers_applock_collation",
        category: "Result Sets, Schemabinding, DDL Triggers, AppLock & Collation",
        min_sql_version: 15,
        setup_batches: &[
            "CREATE TABLE dbo.BoundBase (Id INT PRIMARY KEY, Amount INT NOT NULL, ExtraNote NVARCHAR(50) NULL);
             CREATE TABLE dbo.DdlLog (EvType NVARCHAR(100), SchName NVARCHAR(128), ObjName NVARCHAR(128), ObjType NVARCHAR(100));
             CREATE TABLE dbo.AccentWords (Id INT PRIMARY KEY, Word NVARCHAR(40) NOT NULL);
             INSERT INTO dbo.AccentWords VALUES (1, N'cafe'), (2, N'café'), (3, N'CAFÉ'), (4, N'résumé'), (5, N'resume');",
            "CREATE VIEW dbo.vBoundBase WITH SCHEMABINDING AS
             SELECT Id, Amount FROM dbo.BoundBase;",
            "CREATE PROCEDURE dbo.usp_ReshapeSample AS
             BEGIN
                 SELECT 42 AS raw_a, N'abcdef' AS raw_b;
             END;",
            "CREATE PROCEDURE dbo.usp_EmptySample AS
             BEGIN
                 DECLARE @x INT = 1;
             END;",
            "CREATE TRIGGER tr_LogTableDdl ON DATABASE
             FOR DDL_TABLE_EVENTS
             AS
             BEGIN
                 SET NOCOUNT ON;
                 INSERT INTO dbo.DdlLog (EvType, SchName, ObjName, ObjType)
                 VALUES (
                     EVENTDATA().value('(/EVENT_INSTANCE/EventType)[1]', 'NVARCHAR(100)'),
                     EVENTDATA().value('(/EVENT_INSTANCE/SchemaName)[1]', 'NVARCHAR(128)'),
                     EVENTDATA().value('(/EVENT_INSTANCE/ObjectName)[1]', 'NVARCHAR(128)'),
                     EVENTDATA().value('(/EVENT_INSTANCE/ObjectType)[1]', 'NVARCHAR(100)')
                 );
             END;",
        ],
        steps: &[
            DiffStep::Query {
                label: "exec_with_result_sets_reshape",
                sql: "EXEC dbo.usp_ReshapeSample WITH RESULT SETS ((ItemId BIGINT NOT NULL, Tag NVARCHAR(3)));",
                ordered: true,
            },
            DiffStep::ExecOk {
                label: "exec_with_result_sets_none_ok",
                sql: "EXEC dbo.usp_EmptySample WITH RESULT SETS NONE;",
            },
            DiffStep::ExpectErr {
                label: "exec_with_result_sets_none_rejects_rows",
                sql: "EXEC dbo.usp_ReshapeSample WITH RESULT SETS NONE;",
            },
            DiffStep::ExpectErr {
                label: "exec_with_result_sets_column_count_mismatch",
                sql: "EXEC dbo.usp_ReshapeSample WITH RESULT SETS ((OnlyOne INT));",
            },
            DiffStep::ExpectErr {
                label: "schemabinding_blocks_drop_table",
                sql: "DROP TABLE dbo.BoundBase;",
            },
            DiffStep::ExpectErr {
                label: "schemabinding_blocks_drop_referenced_column",
                sql: "ALTER TABLE dbo.BoundBase DROP COLUMN Amount;",
            },
            DiffStep::ExecOk {
                label: "schemabinding_allows_drop_unreferenced_column",
                sql: "ALTER TABLE dbo.BoundBase DROP COLUMN ExtraNote;",
            },
            DiffStep::Query {
                label: "sys_sql_modules_is_schema_bound",
                sql: "SELECT is_schema_bound FROM sys.sql_modules WHERE object_id = OBJECT_ID('dbo.vBoundBase');",
                ordered: true,
            },
            DiffStep::Query {
                label: "ddl_trigger_eventdata_log",
                sql: "CREATE TABLE dbo.DdlProbe (K INT);
                      ALTER TABLE dbo.DdlProbe ADD V INT;
                      DROP TABLE dbo.DdlProbe;
                      SELECT EvType, SchName, ObjName, ObjType FROM dbo.DdlLog ORDER BY EvType, ObjName;",
                ordered: true,
            },
            DiffStep::ExpectErr {
                label: "applock_test_transaction_outside_tx_errors",
                sql: "SELECT APPLOCK_TEST('public', 'diff_lock', 'Shared', 'Transaction');",
            },
            DiffStep::Query {
                label: "applock_mode_and_test_lifecycle",
                sql: "DECLARE @rc1 INT, @rc2 INT;
                      DECLARE @m0 NVARCHAR(32) = APPLOCK_MODE('public', 'diff_lock', 'Session');
                      EXEC @rc1 = sp_getapplock @Resource = 'diff_lock', @LockMode = 'Shared', @LockOwner = 'Session';
                      DECLARE @m1 NVARCHAR(32) = APPLOCK_MODE('public', 'diff_lock', 'Session');
                      DECLARE @t_same SMALLINT = APPLOCK_TEST('public', 'diff_lock', 'Exclusive', 'Session');
                      DECLARE @t_other_s SMALLINT, @t_other_x SMALLINT;
                      BEGIN TRANSACTION;
                          SET @t_other_s = APPLOCK_TEST('public', 'diff_lock', 'Shared', 'Transaction');
                          SET @t_other_x = APPLOCK_TEST('public', 'diff_lock', 'Exclusive', 'Transaction');
                      COMMIT TRANSACTION;
                      EXEC @rc2 = sp_releaseapplock @Resource = 'diff_lock', @LockOwner = 'Session';
                      DECLARE @m2 NVARCHAR(32) = APPLOCK_MODE('public', 'diff_lock', 'Session');
                      SELECT @rc1, @rc2, @m0, @m1, @t_same, @t_other_s, @t_other_x, @m2;",
                ordered: true,
            },
            DiffStep::Query {
                label: "collate_accent_sensitivity_ai_vs_as",
                sql: "SELECT
                        CASE WHEN N'cafe' COLLATE Latin1_General_CI_AI = N'café' THEN 1 ELSE 0 END,
                        CASE WHEN N'cafe' COLLATE Latin1_General_CI_AS = N'café' THEN 1 ELSE 0 END,
                        CASE WHEN N'Cafe' COLLATE Latin1_General_CS_AI = N'Café' THEN 1 ELSE 0 END,
                        CASE WHEN N'cafe' COLLATE Latin1_General_CS_AI = N'Café' THEN 1 ELSE 0 END,
                        (SELECT COUNT(*) FROM dbo.AccentWords WHERE Word COLLATE Latin1_General_CI_AI LIKE N'resume'),
                        (SELECT COUNT(*) FROM dbo.AccentWords WHERE Word COLLATE Latin1_General_CI_AS LIKE N'resume'),
                        (SELECT COUNT(*) FROM (SELECT Word COLLATE Latin1_General_CI_AI AS W FROM dbo.AccentWords GROUP BY Word COLLATE Latin1_General_CI_AI) g1),
                        (SELECT COUNT(*) FROM (SELECT Word COLLATE Latin1_General_CI_AS AS W FROM dbo.AccentWords GROUP BY Word COLLATE Latin1_General_CI_AS) g2);",
                ordered: true,
            },
            DiffStep::Query {
                label: "approx_aggregates_sql2022",
                sql: "SELECT
                        APPROX_COUNT_DISTINCT(v),
                        APPROX_PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY v),
                        APPROX_PERCENTILE_DISC(0.5) WITHIN GROUP (ORDER BY v)
                      FROM (VALUES (10), (20), (30), (40), (20), (NULL)) AS t(v);",
                ordered: true,
            },
            DiffStep::Query {
                label: "filegroup_and_file_metadata",
                sql: "SELECT
                        FILEGROUP_ID('PRIMARY'),
                        FILEGROUP_NAME(1),
                        FILEGROUPPROPERTY('PRIMARY', 'IsDefault'),
                        FILEGROUPPROPERTY('PRIMARY', 'IsUserDefinedFG'),
                        FILEGROUPPROPERTY('PRIMARY', 'IsReadOnly'),
                        FILE_ID(FILE_NAME(1)),
                        FILEPROPERTY(FILE_NAME(1), 'IsPrimaryFile'),
                        FILEPROPERTY(FILE_NAME(1), 'IsLogFile'),
                        FILEPROPERTY(FILE_NAME(2), 'IsLogFile');",
                ordered: true,
            },
            DiffStep::Query {
                label: "crypt_gen_random_and_passphrase_roundtrip",
                sql: "DECLARE @r VARBINARY(16) = CRYPT_GEN_RANDOM(16);
                      DECLARE @c1 VARBINARY(8000) = ENCRYPTBYPASSPHRASE('test_pass', 'hello_crypto');
                      DECLARE @c2 VARBINARY(8000) = ENCRYPTBYPASSPHRASE('test_pass', 'bound_payload', 1, 'ctx1');
                      SELECT
                        DATALENGTH(@r),
                        CAST(DECRYPTBYPASSPHRASE('test_pass', @c1) AS VARCHAR(50)),
                        DECRYPTBYPASSPHRASE('wrong_pass', @c1),
                        CAST(DECRYPTBYPASSPHRASE('test_pass', @c2, 1, 'ctx1') AS VARCHAR(50)),
                        DECRYPTBYPASSPHRASE('test_pass', @c2, 1, 'ctx2');",
                ordered: true,
            },
        ],
    }
}

fn security_governance_and_query_fidelity() -> DiffSuite {
    DiffSuite {
        name: "security_governance_and_query_fidelity",
        category: "RLS, Masking, Updatable Views, Nested XML & MAXRECURSION",
        min_sql_version: 16,
        setup_batches: &[
            "CREATE TABLE dbo.AlterBackfill (
                Id INT PRIMARY KEY,
                Label NVARCHAR(40) NOT NULL
             );
             INSERT INTO dbo.AlterBackfill VALUES (1, N'One'), (2, N'Two');",
            "CREATE TABLE dbo.XmlGroups (GrpId INT PRIMARY KEY, GrpName NVARCHAR(40));
             CREATE TABLE dbo.XmlItems (ItemId INT PRIMARY KEY, GrpId INT, ItemName NVARCHAR(40));
             INSERT INTO dbo.XmlGroups VALUES (1, N'A&B'), (2, N'Clean');
             INSERT INTO dbo.XmlItems VALUES (10, 1, N'X < Y'), (11, 1, N'P & Q'), (20, 2, N'Solo');",
            "CREATE TABLE dbo.UpdBase (
                Id INT PRIMARY KEY,
                Category NVARCHAR(20) NOT NULL,
                Qty INT NOT NULL
             );
             INSERT INTO dbo.UpdBase VALUES (1, N'Books', 10), (2, N'Tools', 20);",
            "CREATE VIEW dbo.vBooksOnly AS
             SELECT Id, Category AS Cat, Qty FROM dbo.UpdBase WHERE Category = N'Books'
             WITH CHECK OPTION;",
            "CREATE TABLE dbo.MaskedLedger (
                Id INT PRIMARY KEY,
                Email VARCHAR(100) MASKED WITH (FUNCTION = 'email()') NULL,
                Code VARCHAR(20) MASKED WITH (FUNCTION = 'partial(2, \"XXX\", 2)') NULL,
                Amount INT MASKED WITH (FUNCTION = 'default()') NOT NULL,
                EventDate DATE MASKED WITH (FUNCTION = 'datetime(\"Y\")') NULL
             );
             INSERT INTO dbo.MaskedLedger VALUES
                (1, 'bob@example.com', 'AB1234CD', 4200, '2024-05-19'),
                (2, NULL, 'XY', 100, NULL);
             CREATE USER diff_mask_user WITHOUT LOGIN;
             GRANT SELECT ON dbo.MaskedLedger TO diff_mask_user;",
            "CREATE TABLE dbo.RlsOrders (
                OrderId INT PRIMARY KEY,
                TenantId INT NOT NULL,
                Note NVARCHAR(40) NOT NULL
             );
             INSERT INTO dbo.RlsOrders VALUES
                (1, 100, N'T100-1'),
                (2, 100, N'T100-2'),
                (3, 200, N'T200-1');",
            "CREATE FUNCTION dbo.fn_diff_rls(@TenantId INT)
             RETURNS TABLE
             WITH SCHEMABINDING
             AS
             RETURN SELECT 1 AS allow_row
                    WHERE @TenantId = CAST(SESSION_CONTEXT(N'DiffTenant') AS INT)
                       OR CAST(SESSION_CONTEXT(N'DiffBypass') AS INT) = 1;",
            "CREATE SECURITY POLICY dbo.DiffTenantPolicy
                ADD FILTER PREDICATE dbo.fn_diff_rls(TenantId) ON dbo.RlsOrders,
                ADD BLOCK PREDICATE dbo.fn_diff_rls(TenantId) ON dbo.RlsOrders AFTER INSERT,
                ADD BLOCK PREDICATE dbo.fn_diff_rls(TenantId) ON dbo.RlsOrders AFTER UPDATE
             WITH (STATE = ON);",
        ],
        steps: &[
            DiffStep::Query {
                label: "option_maxrecursion_explicit_default_zero_and_error_530",
                sql: "DECLARE @ok_cnt INT, @err_explicit INT = 0, @err_default INT = 0, @zero_cnt INT;
                      WITH c1 AS (SELECT 1 AS n UNION ALL SELECT n + 1 FROM c1 WHERE n < 4)
                      SELECT @ok_cnt = COUNT(*) FROM c1 OPTION (MAXRECURSION 3);
                      BEGIN TRY
                          WITH c2 AS (SELECT 1 AS n UNION ALL SELECT n + 1 FROM c2 WHERE n < 5)
                          SELECT COUNT(*) FROM c2 OPTION (MAXRECURSION 3);
                      END TRY
                      BEGIN CATCH
                          SET @err_explicit = ERROR_NUMBER();
                      END CATCH
                      BEGIN TRY
                          WITH c3 AS (SELECT 1 AS n UNION ALL SELECT n + 1 FROM c3 WHERE n <= 101)
                          SELECT COUNT(*) FROM c3;
                      END TRY
                      BEGIN CATCH
                          SET @err_default = ERROR_NUMBER();
                      END CATCH;
                      WITH c4 AS (SELECT 1 AS n UNION ALL SELECT n + 1 FROM c4 WHERE n < 120)
                      SELECT @zero_cnt = COUNT(*) FROM c4 OPTION (MAXRECURSION 0);
                      SELECT @ok_cnt, @err_explicit, @err_default, @zero_cnt;",
                ordered: true,
            },
            DiffStep::ExecOk {
                label: "alter_table_add_col_setup",
                sql: "ALTER TABLE dbo.AlterBackfill ADD NullDef INT NULL DEFAULT 7;
                      ALTER TABLE dbo.AlterBackfill ADD WithVals INT NULL DEFAULT 9 WITH VALUES;
                      ALTER TABLE dbo.AlterBackfill ADD NotNullDef NVARCHAR(10) NOT NULL DEFAULT N'ok';
                      INSERT INTO dbo.AlterBackfill (Id, Label) VALUES (3, N'Three');",
            },
            DiffStep::Query {
                label: "alter_table_add_col_default_not_null_and_with_values",
                sql: "DECLARE @err4901 INT = 0;
                      BEGIN TRY
                          ALTER TABLE dbo.AlterBackfill ADD BadNN INT NOT NULL;
                      END TRY
                      BEGIN CATCH
                          SET @err4901 = ERROR_NUMBER();
                      END CATCH
                      SELECT Id, NullDef, WithVals, NotNullDef, @err4901 AS Err4901
                      FROM dbo.AlterBackfill ORDER BY Id;",
                ordered: true,
            },
            DiffStep::Query {
                label: "nested_for_xml_type_composition_and_value",
                sql: "SELECT
                        (SELECT
                            g.GrpId AS '@id',
                            g.GrpName AS 'name',
                            (SELECT i.ItemName AS 'it'
                             FROM dbo.XmlItems i
                             WHERE i.GrpId = g.GrpId
                             ORDER BY i.ItemId
                             FOR XML PATH(''), TYPE) AS 'items'
                         FROM dbo.XmlGroups g
                         ORDER BY g.GrpId
                         FOR XML PATH('grp'), ROOT('all')),
                        STUFF((SELECT N', ' + ItemName
                               FROM dbo.XmlItems
                               ORDER BY ItemId
                               FOR XML PATH(''), TYPE).value('.', 'NVARCHAR(MAX)'), 1, 2, N''),
                        STUFF((SELECT CHAR(13) + CHAR(10) + ItemName
                               FROM dbo.XmlItems
                               WHERE ItemId IN (10, 20)
                               ORDER BY ItemId
                               FOR XML PATH(''), TYPE).value('.', 'NVARCHAR(MAX)'), 1, 2, N'');",
                ordered: true,
            },
            DiffStep::Query {
                label: "updatable_views_and_with_check_option_error_550",
                sql: "INSERT INTO dbo.vBooksOnly (Id, Cat, Qty) VALUES (3, N'Books', 30);
                      DECLARE @ins_err INT = 0, @upd_err INT = 0;
                      BEGIN TRY
                          INSERT INTO dbo.vBooksOnly (Id, Cat, Qty) VALUES (4, N'Tools', 40);
                      END TRY
                      BEGIN CATCH
                          SET @ins_err = ERROR_NUMBER();
                      END CATCH
                      UPDATE dbo.vBooksOnly SET Qty = Qty + 5;
                      BEGIN TRY
                          UPDATE dbo.vBooksOnly SET Cat = N'Other' WHERE Id = 1;
                      END TRY
                      BEGIN CATCH
                          SET @upd_err = ERROR_NUMBER();
                      END CATCH
                      DELETE FROM dbo.vBooksOnly WHERE Id IN (2, 3);
                      SELECT Id, Category, Qty, @ins_err AS InsErr, @upd_err AS UpdErr
                      FROM dbo.UpdBase ORDER BY Id;",
                ordered: true,
            },
            DiffStep::Query {
                label: "dynamic_data_masking_and_unmask_permissions",
                sql: "DECLARE @m_email VARCHAR(100), @m_code VARCHAR(20), @m_amt INT, @m_dt DATE, @m_null_email VARCHAR(100), @m_short_code VARCHAR(20);
                      DECLARE @u_email VARCHAR(100), @u_amt INT;
                      EXECUTE AS USER = 'diff_mask_user';
                      SELECT @m_email = Email, @m_code = Code, @m_amt = Amount, @m_dt = EventDate
                      FROM dbo.MaskedLedger WHERE Email = 'bob@example.com';
                      SELECT @m_null_email = Email, @m_short_code = Code
                      FROM dbo.MaskedLedger WHERE Id = 2;
                      REVERT;
                      GRANT UNMASK TO diff_mask_user;
                      EXECUTE AS USER = 'diff_mask_user';
                      SELECT @u_email = Email, @u_amt = Amount FROM dbo.MaskedLedger WHERE Id = 1;
                      REVERT;
                      REVOKE UNMASK FROM diff_mask_user;
                      SELECT @m_email, @m_code, @m_amt, @m_dt, @m_null_email, @m_short_code, @u_email, @u_amt,
                             (SELECT COUNT(*) FROM sys.masked_columns WHERE object_id = OBJECT_ID('dbo.MaskedLedger') AND is_masked = 1);",
                ordered: true,
            },
            DiffStep::Query {
                label: "row_level_security_filter_and_block_predicates",
                sql: "EXEC sp_set_session_context @key = N'DiffTenant', @value = 100;
                      EXEC sp_set_session_context @key = N'DiffBypass', @value = 0;
                      DECLARE @t100_cnt INT = (SELECT COUNT(*) FROM dbo.RlsOrders);
                      INSERT INTO dbo.RlsOrders VALUES (4, 100, N'T100-3');
                      DECLARE @blk_ins INT = 0, @blk_upd INT = 0;
                      BEGIN TRY
                          INSERT INTO dbo.RlsOrders VALUES (5, 200, N'Forbidden');
                      END TRY
                      BEGIN CATCH
                          SET @blk_ins = ERROR_NUMBER();
                      END CATCH
                      BEGIN TRY
                          UPDATE dbo.RlsOrders SET TenantId = 200 WHERE OrderId = 1;
                      END TRY
                      BEGIN CATCH
                          SET @blk_upd = ERROR_NUMBER();
                      END CATCH
                      DELETE FROM dbo.RlsOrders WHERE OrderId IN (2, 3);
                      ALTER SECURITY POLICY dbo.DiffTenantPolicy WITH (STATE = OFF);
                      DECLARE @all_cnt INT = (SELECT COUNT(*) FROM dbo.RlsOrders);
                      DECLARE @pol_cnt INT = (SELECT COUNT(*) FROM sys.security_policies WHERE name = 'DiffTenantPolicy');
                      DECLARE @pred_cnt INT = (SELECT COUNT(*) FROM sys.security_predicates WHERE object_id = OBJECT_ID('dbo.DiffTenantPolicy'));
                      SELECT @t100_cnt, @blk_ins, @blk_upd, @all_cnt, @pol_cnt, @pred_cnt;",
                ordered: true,
            },
            DiffStep::Query {
                label: "residual_hash_join_left_right_full_outer_parity",
                sql: "CREATE TABLE #JL (Id INT, Val INT);
                      CREATE TABLE #JR (Id INT, IsActive BIT, MinVal INT, Name VARCHAR(20));
                      INSERT INTO #JL VALUES (1, 100), (2, 10), (3, 50), (NULL, 99);
                      INSERT INTO #JR VALUES
                          (1, 1, 50, 'r1_ok'),
                          (1, 0, 50, 'r1_off'),
                          (2, 1, 50, 'r2_low'),
                          (4, 1, 10, 'r4_only'),
                          (NULL, 1, 0, 'r9_null');
                      SELECT 'L' AS Side, l.Id, r.Name
                      FROM #JL l
                      LEFT JOIN #JR r ON l.Id = r.Id AND r.IsActive = 1 AND l.Val >= r.MinVal
                      UNION ALL
                      SELECT 'R' AS Side, l.Id, r.Name
                      FROM #JL l
                      RIGHT JOIN #JR r ON l.Id = r.Id AND r.IsActive = 1 AND l.Val >= r.MinVal
                      UNION ALL
                      SELECT 'F' AS Side, l.Id, r.Name
                      FROM #JL l
                      FULL OUTER JOIN #JR r ON l.Id = r.Id AND r.IsActive = 1 AND l.Val >= r.MinVal
                      ORDER BY Side, Id, Name;",
                ordered: true,
            },
            DiffStep::Query {
                label: "set_op_trailing_order_by_offset_fetch_and_join_where_pushdown",
                sql: "SELECT Id AS ItemId, Val AS Score FROM #JL WHERE Id IS NOT NULL
                      UNION ALL
                      SELECT Id, MinVal FROM #JR WHERE Id IS NOT NULL
                      ORDER BY ItemId ASC, Score DESC
                      OFFSET 1 ROWS FETCH NEXT 4 ROWS ONLY;",
                ordered: true,
            },
            DiffStep::Query {
                label: "left_join_anti_join_where_not_pushed_to_right",
                sql: "SELECT l.Id, l.Val
                      FROM #JL l
                      LEFT JOIN #JR r ON l.Id = r.Id AND r.IsActive = 1
                      WHERE r.Id IS NULL AND l.Val >= 50
                      ORDER BY COALESCE(l.Id, 999);",
                ordered: true,
            },
            DiffStep::Query {
                label: "context_info_default_null_nstring_alias_and_checkpoint",
                sql: "DECLARE @init_null BIT = CASE WHEN CONTEXT_INFO() IS NULL THEN 1 ELSE 0 END;
                      SET CONTEXT_INFO 0xCAFE;
                      DECLARE @after_set VARBINARY(2) = SUBSTRING(CONTEXT_INFO(), 1, 2);
                      SET CONTEXT_INFO 0x;
                      DECLARE @after_clear BIT = CASE WHEN CONTEXT_INFO() IS NULL THEN 1 ELSE 0 END;
                      CHECKPOINT;
                      SELECT @init_null AS N'InitNull', @after_set AS N'AfterSet', @after_clear AS N'AfterClear';",
                ordered: true,
            },
            DiffStep::Query {
                label: "multi_table_join_where_pushdown_correlated_exists_and_outer_apply",
                sql: "CREATE TABLE #JM (Id INT, Tag VARCHAR(20), Weight INT);
                      INSERT INTO #JM VALUES (1, 'r1_ok', 10), (2, 'r2_low', 20), (3, 'none', 30);
                      SELECT l.Id, r.Name, m.Weight, ap.TopName
                      FROM #JL l
                      LEFT JOIN #JR r ON l.Id = r.Id AND r.IsActive = 1
                      INNER JOIN #JM m ON m.Id = l.Id
                      OUTER APPLY (
                          SELECT TOP 1 r2.Name AS TopName
                          FROM #JR r2
                          WHERE r2.Id = l.Id
                          ORDER BY r2.Name
                      ) ap
                      WHERE (r.Name = m.Tag OR r.Id IS NULL)
                        AND EXISTS (SELECT 1 FROM #JM m2 WHERE m2.Id = l.Id AND m2.Weight >= 10)
                      ORDER BY l.Id;",
                ordered: true,
            },
            DiffStep::Query {
                label: "catalog_metadata_constraints_permissions_and_config_parity",
                sql: "CREATE TABLE dbo.DiffMetaParent (Id INT PRIMARY KEY);
                      CREATE TABLE dbo.DiffMetaChild (
                          Id INT PRIMARY KEY,
                          ParentId INT NOT NULL,
                          Qty INT NOT NULL,
                          CONSTRAINT CK_DiffMetaChild_Qty CHECK (Qty >= 0),
                          CONSTRAINT FK_DiffMetaChild_Parent FOREIGN KEY (ParentId) REFERENCES dbo.DiffMetaParent(Id)
                      );
                      SELECT
                          (SELECT COUNT(*) FROM sys.check_constraints ck
                           LEFT JOIN sys.filetable_system_defined_objects ft
                             ON ck.parent_object_id = ft.parent_object_id AND ck.object_id = ft.object_id
                           WHERE ck.parent_object_id = OBJECT_ID('dbo.DiffMetaChild')
                             AND ck.is_disabled = 0 AND ck.is_not_trusted = 0
                             AND ck.is_ms_shipped = 0 AND ft.object_id IS NULL) AS CkCnt,
                          (SELECT COUNT(*) FROM sys.foreign_keys fk
                           LEFT JOIN sys.filetable_system_defined_objects ft
                             ON fk.parent_object_id = ft.parent_object_id AND fk.object_id = ft.object_id
                           WHERE fk.parent_object_id = OBJECT_ID('dbo.DiffMetaChild')
                             AND fk.is_disabled = 0 AND fk.is_not_trusted = 0
                             AND fk.is_ms_shipped = 0 AND ft.object_id IS NULL) AS FkCnt,
                          (SELECT CAST(value AS INT) FROM [master].[dbo].[sysconfigures] WHERE config = 1126) AS FullTextLang,
                          (SELECT COUNT(*) FROM [master].[dbo].[sysdatabases] WHERE name = N'master') AS MasterDbCnt,
                          (SELECT COUNT(*) FROM sys.database_scoped_configurations WHERE name = N'MAXDOP') AS MaxDopCfgCnt,
                          (SELECT COUNT(*) FROM sys.server_permissions WHERE state = N'D') AS DeniedSrvPerms,
                          (SELECT CASE WHEN COUNT(*) > 0 THEN 1 ELSE 0 END FROM sys.fn_builtin_permissions('')) AS HasBuiltinPerms;",
                ordered: true,
            },
            DiffStep::Query {
                label: "binary_substring_datalength_and_null_propagation_parity",
                sql: "DECLARE @bin VARBINARY(8) = 0xCAFEBABE;
                      SELECT
                          SUBSTRING(@bin, 1, 2) AS S1,
                          SUBSTRING(@bin, 0, 2) AS S0,
                          SUBSTRING(@bin, 3, 10) AS S3,
                          SUBSTRING(CAST(NULL AS VARBINARY(4)), 1, 2) AS SNull,
                          DATALENGTH(@bin) AS BinLen,
                          DATALENGTH(CAST(NULL AS VARBINARY(4))) AS NullLen,
                          LEFT(CAST(NULL AS VARCHAR(10)), 2) AS LeftNull,
                          RIGHT(CAST(NULL AS VARCHAR(10)), 2) AS RightNull;",
                ordered: true,
            },
            DiffStep::Query {
                label: "inline_column_constraints_fk_check_and_catalog_parity",
                sql: "CREATE TABLE dbo.DiffInlineParent (
                          Id INT NOT NULL CONSTRAINT PK_DiffInlineParent PRIMARY KEY
                      );
                      CREATE TABLE dbo.DiffInlineChild (
                          Id INT NOT NULL CONSTRAINT PK_DiffInlineChild PRIMARY KEY,
                          ParentId INT NOT NULL CONSTRAINT FK_DiffInlineChild_Parent REFERENCES dbo.DiffInlineParent(Id) ON DELETE CASCADE,
                          Qty INT NOT NULL CONSTRAINT CK_DiffInlineChild_Qty CHECK (Qty >= 0),
                          Flag BIT NOT NULL CONSTRAINT DF_DiffInlineChild_Flag DEFAULT (1)
                      );
                      INSERT INTO dbo.DiffInlineParent VALUES (1);
                      INSERT INTO dbo.DiffInlineChild (Id, ParentId, Qty) VALUES (10, 1, 5);
                      DECLARE @chk_err INT = 0, @fk_err INT = 0;
                      BEGIN TRY
                          INSERT INTO dbo.DiffInlineChild (Id, ParentId, Qty) VALUES (11, 1, -1);
                      END TRY
                      BEGIN CATCH SET @chk_err = 1; END CATCH
                      BEGIN TRY
                          INSERT INTO dbo.DiffInlineChild (Id, ParentId, Qty) VALUES (12, 999, 1);
                      END TRY
                      BEGIN CATCH SET @fk_err = 1; END CATCH
                      DELETE FROM dbo.DiffInlineParent WHERE Id = 1;
                      SELECT
                          @chk_err,
                          @fk_err,
                          (SELECT COUNT(*) FROM dbo.DiffInlineChild),
                          (SELECT name FROM sys.check_constraints WHERE parent_object_id = OBJECT_ID('dbo.DiffInlineChild')),
                          (SELECT name FROM sys.foreign_keys WHERE parent_object_id = OBJECT_ID('dbo.DiffInlineChild'));",
                ordered: true,
            },
            DiffStep::Query {
                label: "partition_function_dollar_partition_and_switch_parity",
                sql: "CREATE PARTITION FUNCTION pf_DiffLeft (INT) AS RANGE LEFT FOR VALUES (10, 20);
                      CREATE PARTITION FUNCTION pf_DiffRight (INT) AS RANGE RIGHT FOR VALUES (10, 20);
                      CREATE TABLE dbo.DiffSwitchSrc (Id INT NOT NULL, Val VARCHAR(10));
                      CREATE TABLE dbo.DiffSwitchDst (Id INT NOT NULL, Val VARCHAR(10));
                      INSERT INTO dbo.DiffSwitchSrc VALUES (1, 'a'), (2, 'b');
                      ALTER TABLE dbo.DiffSwitchSrc SWITCH TO dbo.DiffSwitchDst;
                      SELECT
                          $PARTITION.pf_DiffLeft(NULL),
                          $PARTITION.pf_DiffLeft(10),
                          $PARTITION.pf_DiffLeft(11),
                          $PARTITION.pf_DiffRight(9),
                          $PARTITION.pf_DiffRight(10),
                          $PARTITION.pf_DiffRight(20),
                          (SELECT COUNT(*) FROM dbo.DiffSwitchSrc),
                          (SELECT COUNT(*) FROM dbo.DiffSwitchDst);",
                ordered: true,
            },
            DiffStep::Query {
                label: "spatial_geometry_geography_and_hierarchyid_parity",
                sql: "DECLARE @g1 GEOMETRY = geometry::Point(0, 0, 0);
                      DECLARE @g2 GEOMETRY = geometry::STGeomFromText('POINT(3 4)', 0);
                      DECLARE @geo GEOGRAPHY = geography::Point(40.7128, -74.0060, 4326);
                      DECLARE @h HIERARCHYID = hierarchyid::Parse('/1/2/3/');
                      DECLARE @p HIERARCHYID = hierarchyid::Parse('/1/2/');
                      SELECT
                          CAST(@g1.STDistance(@g2) AS INT),
                          CAST(@g2.STX AS INT),
                          CAST(@g2.STY AS INT),
                          @geo.STSrid,
                          @h.GetLevel(),
                          @h.IsDescendantOf(@p),
                          @p.IsDescendantOf(@h),
                          @h.GetAncestor(1).ToString(),
                          @h.GetReparentedValue(@p, hierarchyid::Parse('/9/')).ToString();",
                ordered: true,
            },
            DiffStep::Query {
                label: "always_encrypted_cmk_cek_ddl_and_catalog_parity",
                sql: "CREATE COLUMN MASTER KEY [CMK_DiffTest]
                      WITH (
                          KEY_STORE_PROVIDER_NAME = N'MSSQL_CERTIFICATE_STORE',
                          KEY_PATH = N'CurrentUser/my/AABBCCDDEE00112233445566778899FF'
                      );
                      CREATE COLUMN ENCRYPTION KEY [CEK_DiffTest]
                      WITH VALUES (
                          COLUMN_MASTER_KEY = [CMK_DiffTest],
                          ALGORITHM = 'RSA_OAEP',
                          ENCRYPTED_VALUE = 0x016E00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF
                      );
                      DECLARE @cmk_name NVARCHAR(128) = (SELECT name FROM sys.column_master_keys WHERE name = 'CMK_DiffTest');
                      DECLARE @cek_name NVARCHAR(128) = (SELECT name FROM sys.column_encryption_keys WHERE name = 'CEK_DiffTest');
                      DECLARE @alg NVARCHAR(128) = (SELECT encryption_algorithm_name FROM sys.column_encryption_key_values WHERE column_encryption_key_id = (SELECT column_encryption_key_id FROM sys.column_encryption_keys WHERE name = 'CEK_DiffTest'));
                      DROP COLUMN ENCRYPTION KEY [CEK_DiffTest];
                      DROP COLUMN MASTER KEY [CMK_DiffTest];
                      SELECT @cmk_name, @cek_name, @alg, (SELECT COUNT(*) FROM sys.column_master_keys WHERE name = 'CMK_DiffTest');",
                ordered: true,
            },
        ],
    }
}
