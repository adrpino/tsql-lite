using System;
using System.Collections.Generic;
using System.Data;
using System.Globalization;
using System.Linq;
using Microsoft.Data.SqlClient;
using Microsoft.EntityFrameworkCore;

namespace DriverCompat;

internal sealed class CompatCustomer
{
    public int Id { get; set; }
    public string Name { get; set; } = string.Empty;
    public bool IsActive { get; set; }
    public List<CompatOrder> Orders { get; set; } = new List<CompatOrder>();
}

internal sealed class CompatOrder
{
    public int Id { get; set; }
    public int CustomerId { get; set; }
    public CompatCustomer Customer { get; set; } = null!;
    public decimal TotalAmount { get; set; }
    public string Code { get; set; } = string.Empty;
}

internal sealed class CompatDbContext : DbContext
{
    private readonly string _connectionString;

    public CompatDbContext(string connectionString)
    {
        _connectionString = connectionString;
    }

    public DbSet<CompatCustomer> Customers => Set<CompatCustomer>();
    public DbSet<CompatOrder> Orders => Set<CompatOrder>();

    protected override void OnConfiguring(DbContextOptionsBuilder optionsBuilder)
    {
        optionsBuilder.UseSqlServer(_connectionString);
    }

    protected override void OnModelCreating(ModelBuilder modelBuilder)
    {
        modelBuilder.Entity<CompatCustomer>(entity =>
        {
            entity.ToTable("EfCompatCustomers", "dbo");
            entity.HasKey(e => e.Id);
            entity.Property(e => e.Name).HasMaxLength(100).IsRequired();
        });

        modelBuilder.Entity<CompatOrder>(entity =>
        {
            entity.ToTable("EfCompatOrders", "dbo");
            entity.HasKey(e => e.Id);
            entity.Property(e => e.TotalAmount).HasPrecision(12, 2);
            entity.Property(e => e.Code).HasMaxLength(40).IsRequired();
            entity.HasOne(e => e.Customer)
                .WithMany(c => c.Orders)
                .HasForeignKey(e => e.CustomerId)
                .OnDelete(DeleteBehavior.Cascade);
        });
    }
}

internal static class Tier1Scenarios
{
    private const int DefaultTimeoutSeconds = 8;

    public static List<(string Name, Func<string, string> Run)> All()
    {
        return new List<(string Name, Func<string, string> Run)>
        {
            ("11_pooling_status_reset_isolation", RunPoolingStatusReset),
            ("12_prepared_command_prepare_execute_unprepare", RunPreparedCommand),
            ("13_print_and_raiserror_info_message", RunPrintInfoMessage),
            ("14_sql_exception_numbers_parity", RunSqlExceptionNumbers),
            ("15_sql_bulk_copy_multi_types", RunSqlBulkCopy),
            ("16_ef_core_8_ensure_created_crud_linq_batch", RunEfCore8Crud),
        };
    }

    private static SqlConnection OpenConnection(string connectionString, bool pooling = false)
    {
        SqlConnectionStringBuilder builder = new SqlConnectionStringBuilder(connectionString);
        builder.Pooling = pooling;
        builder.ConnectTimeout = 10;
        if (pooling)
        {
            builder.MinPoolSize = 1;
            builder.MaxPoolSize = 1;
            builder.ApplicationName = "DrvPoolTest_" + Guid.NewGuid().ToString("N");
        }

        SqlConnection conn = new SqlConnection(builder.ConnectionString);
        conn.Open();
        return conn;
    }

    private static void ExecNonQuery(SqlConnection conn, string sql)
    {
        using SqlCommand cmd = new SqlCommand(sql, conn);
        cmd.CommandTimeout = DefaultTimeoutSeconds;
        cmd.ExecuteNonQuery();
    }

    private static string RunPoolingStatusReset(string connStr)
    {
        SqlConnectionStringBuilder builder = new SqlConnectionStringBuilder(connStr);
        builder.Pooling = true;
        builder.MinPoolSize = 1;
        builder.MaxPoolSize = 1;
        builder.ConnectTimeout = 10;
        builder.ApplicationName = "DrvPoolIso_" + Guid.NewGuid().ToString("N");
        string pooledConnStr = builder.ConnectionString;

        // Setup target table on an unpooled connection
        using (SqlConnection setup = OpenConnection(connStr))
        {
            ExecNonQuery(setup, "IF OBJECT_ID('dbo.DrvPoolProbe', 'U') IS NOT NULL DROP TABLE dbo.DrvPoolProbe;");
            ExecNonQuery(setup, "CREATE TABLE dbo.DrvPoolProbe (Id INT PRIMARY KEY);");
        }

        try
        {
            // Checkout 1: dirty the session state (SET NOCOUNT ON + #temp table)
            using (SqlConnection c1 = new SqlConnection(pooledConnStr))
            {
                c1.Open();
                ExecNonQuery(c1, "SET NOCOUNT ON; CREATE TABLE #PoolDirtyTemp (Id INT); INSERT INTO #PoolDirtyTemp VALUES (99);");
            }

            // Checkout 2: reuses the exact same physical socket with TDS STATUS_RESET (0x08)
            int tempGone;
            int rowsAffectedAfterReset;
            int tranCount;
            using (SqlConnection c2 = new SqlConnection(pooledConnStr))
            {
                c2.Open();
                using (SqlCommand qTemp = new SqlCommand("SELECT CASE WHEN OBJECT_ID('tempdb..#PoolDirtyTemp') IS NULL THEN 1 ELSE 0 END;", c2))
                {
                    qTemp.CommandTimeout = DefaultTimeoutSeconds;
                    tempGone = Convert.ToInt32(qTemp.ExecuteScalar(), CultureInfo.InvariantCulture);
                }

                // If SET NOCOUNT was properly reset to OFF, ExecuteNonQuery returns 2 (not -1)
                using (SqlCommand ins = new SqlCommand("INSERT INTO dbo.DrvPoolProbe VALUES (1), (2);", c2))
                {
                    ins.CommandTimeout = DefaultTimeoutSeconds;
                    rowsAffectedAfterReset = ins.ExecuteNonQuery();
                }

                using (SqlCommand qTx = new SqlCommand("SELECT @@TRANCOUNT;", c2))
                {
                    qTx.CommandTimeout = DefaultTimeoutSeconds;
                    tranCount = Convert.ToInt32(qTx.ExecuteScalar(), CultureInfo.InvariantCulture);
                }
            }

            SqlConnection.ClearAllPools();
            return string.Format(
                CultureInfo.InvariantCulture,
                "tempGone={0};rows={1};trancount={2}",
                tempGone,
                rowsAffectedAfterReset,
                tranCount);
        }
        finally
        {
            SqlConnection.ClearAllPools();
            using SqlConnection cleanup = OpenConnection(connStr);
            ExecNonQuery(cleanup, "IF OBJECT_ID('dbo.DrvPoolProbe', 'U') IS NOT NULL DROP TABLE dbo.DrvPoolProbe;");
        }
    }

    private static string RunPreparedCommand(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        ExecNonQuery(conn, "IF OBJECT_ID('dbo.DrvPrepItems', 'U') IS NOT NULL DROP TABLE dbo.DrvPrepItems;");
        ExecNonQuery(conn, "CREATE TABLE dbo.DrvPrepItems (Id INT PRIMARY KEY, Tag NVARCHAR(40), Price DECIMAL(10,2));");

        try
        {
            using (SqlCommand cmd = new SqlCommand("INSERT INTO dbo.DrvPrepItems (Id, Tag, Price) VALUES (@Id, @Tag, @Price);", conn))
            {
                cmd.CommandTimeout = DefaultTimeoutSeconds;
                SqlParameter pId = cmd.Parameters.Add("@Id", SqlDbType.Int);
                SqlParameter pTag = cmd.Parameters.Add("@Tag", SqlDbType.NVarChar, 40);
                SqlParameter pPrice = cmd.Parameters.Add("@Price", SqlDbType.Decimal);
                pPrice.Precision = 10;
                pPrice.Scale = 2;

                cmd.Prepare();

                pId.Value = 1;
                pTag.Value = "first";
                pPrice.Value = 12.50m;
                cmd.ExecuteNonQuery();

                pId.Value = 2;
                pTag.Value = "second";
                pPrice.Value = 99.75m;
                cmd.ExecuteNonQuery();
            }

            List<string> rows = new List<string>();
            using (SqlCommand sel = new SqlCommand("SELECT Id, Tag, Price FROM dbo.DrvPrepItems WHERE Id >= @MinId ORDER BY Id;", conn))
            {
                sel.CommandTimeout = DefaultTimeoutSeconds;
                SqlParameter pMin = sel.Parameters.Add("@MinId", SqlDbType.Int);
                sel.Prepare();

                pMin.Value = 1;
                using SqlDataReader r = sel.ExecuteReader();
                while (r.Read())
                {
                    rows.Add(string.Format(
                        CultureInfo.InvariantCulture,
                        "{0}:{1}:{2}",
                        r.GetInt32(0),
                        r.GetString(1),
                        r.GetDecimal(2).ToString("0.00", CultureInfo.InvariantCulture)));
                }
            }

            return string.Join("|", rows);
        }
        finally
        {
            ExecNonQuery(conn, "IF OBJECT_ID('dbo.DrvPrepItems', 'U') IS NOT NULL DROP TABLE dbo.DrvPrepItems;");
        }
    }

    private static string RunPrintInfoMessage(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        List<string> infoMessages = new List<string>();
        conn.InfoMessage += (_, args) =>
        {
            foreach (SqlError err in args.Errors)
            {
                if (!string.IsNullOrWhiteSpace(err.Message))
                {
                    infoMessages.Add(err.Message.Trim());
                }
            }
        };

        int scalarVal;
        using (SqlCommand cmd = new SqlCommand(@"
PRINT N'hello-info-1';
RAISERROR(N'low-sev-msg', 5, 1) WITH NOWAIT;
PRINT N'hello-info-2';
SELECT 99 AS Answer;", conn))
        {
            cmd.CommandTimeout = DefaultTimeoutSeconds;
            scalarVal = Convert.ToInt32(cmd.ExecuteScalar(), CultureInfo.InvariantCulture);
        }

        return string.Format(
            CultureInfo.InvariantCulture,
            "val={0};msgs=[{1}]",
            scalarVal,
            string.Join("|", infoMessages));
    }

    private static int CaptureSqlErrorNumber(SqlConnection conn, string sql)
    {
        try
        {
            using SqlCommand cmd = new SqlCommand(sql, conn);
            cmd.CommandTimeout = DefaultTimeoutSeconds;
            cmd.ExecuteNonQuery();
            return 0;
        }
        catch (SqlException ex)
        {
            return ex.Number;
        }
    }

    private static string RunSqlExceptionNumbers(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        ExecNonQuery(conn, "IF OBJECT_ID('dbo.DrvErrChild', 'U') IS NOT NULL DROP TABLE dbo.DrvErrChild;");
        ExecNonQuery(conn, "IF OBJECT_ID('dbo.DrvErrParent', 'U') IS NOT NULL DROP TABLE dbo.DrvErrParent;");
        ExecNonQuery(conn, "CREATE TABLE dbo.DrvErrParent (Id INT PRIMARY KEY, Name NVARCHAR(30) NOT NULL);");
        ExecNonQuery(conn, "CREATE TABLE dbo.DrvErrChild (Id INT PRIMARY KEY, ParentId INT NOT NULL REFERENCES dbo.DrvErrParent(Id));");
        ExecNonQuery(conn, "INSERT INTO dbo.DrvErrParent (Id, Name) VALUES (1, N'ok');");

        try
        {
            int pkErr = CaptureSqlErrorNumber(conn, "INSERT INTO dbo.DrvErrParent (Id, Name) VALUES (1, N'dup');");
            int fkErr = CaptureSqlErrorNumber(conn, "INSERT INTO dbo.DrvErrChild (Id, ParentId) VALUES (10, 999);");
            int nullErr = CaptureSqlErrorNumber(conn, "INSERT INTO dbo.DrvErrParent (Id, Name) VALUES (2, NULL);");
            int objErr = CaptureSqlErrorNumber(conn, "SELECT * FROM dbo.DrvMissingTable_999;");
            int divErr = CaptureSqlErrorNumber(conn, "SELECT 1 / 0;");
            int throwErr = CaptureSqlErrorNumber(conn, "THROW 51234, N'custom throw', 1;");
            int raiseErr = CaptureSqlErrorNumber(conn, "RAISERROR(N'custom raise', 16, 1);");

            return string.Format(
                CultureInfo.InvariantCulture,
                "pk={0};fk={1};null={2};obj={3};div={4};throw={5};raise={6}",
                pkErr,
                fkErr,
                nullErr,
                objErr,
                divErr,
                throwErr,
                raiseErr);
        }
        finally
        {
            ExecNonQuery(conn, "IF OBJECT_ID('dbo.DrvErrChild', 'U') IS NOT NULL DROP TABLE dbo.DrvErrChild;");
            ExecNonQuery(conn, "IF OBJECT_ID('dbo.DrvErrParent', 'U') IS NOT NULL DROP TABLE dbo.DrvErrParent;");
        }
    }

    private static string RunSqlBulkCopy(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        ExecNonQuery(conn, "IF OBJECT_ID('dbo.DrvBulkTarget', 'U') IS NOT NULL DROP TABLE dbo.DrvBulkTarget;");
        ExecNonQuery(conn, @"
CREATE TABLE dbo.DrvBulkTarget (
    Id INT NOT NULL PRIMARY KEY,
    Title NVARCHAR(100) NULL,
    Amount DECIMAL(12, 2) NULL,
    Active BIT NOT NULL,
    CreatedAt DATETIME2(3) NULL,
    RefKey UNIQUEIDENTIFIER NULL
);");

        try
        {
            DataTable dt = new DataTable();
            dt.Columns.Add("Id", typeof(int));
            dt.Columns.Add("Title", typeof(string));
            dt.Columns.Add("Amount", typeof(decimal));
            dt.Columns.Add("Active", typeof(bool));
            dt.Columns.Add("CreatedAt", typeof(DateTime));
            dt.Columns.Add("RefKey", typeof(Guid));

            dt.Rows.Add(1, "alpha", 10.25m, true, new DateTime(2026, 4, 1, 8, 15, 30, 500), new Guid("11111111-2222-3333-4444-555555555555"));
            dt.Rows.Add(2, "beta", 200.00m, false, new DateTime(2026, 4, 2, 12, 0, 0, 0), new Guid("66666666-7777-8888-9999-AAAAAAAAAAAA"));
            dt.Rows.Add(3, DBNull.Value, DBNull.Value, true, DBNull.Value, DBNull.Value);

            using (SqlBulkCopy bulk = new SqlBulkCopy(conn))
            {
                bulk.DestinationTableName = "dbo.DrvBulkTarget";
                bulk.BulkCopyTimeout = DefaultTimeoutSeconds;
                bulk.WriteToServer(dt);
            }

            List<string> lines = new List<string>();
            using (SqlCommand q = new SqlCommand(@"
SELECT
    Id,
    ISNULL(Title, N'NULL'),
    ISNULL(CAST(Amount AS VARCHAR(20)), 'NULL'),
    CAST(Active AS INT),
    ISNULL(CONVERT(VARCHAR(23), CreatedAt, 121), 'NULL'),
    ISNULL(UPPER(CAST(RefKey AS VARCHAR(36))), 'NULL')
FROM dbo.DrvBulkTarget
ORDER BY Id;", conn))
            {
                q.CommandTimeout = DefaultTimeoutSeconds;
                using SqlDataReader r = q.ExecuteReader();
                while (r.Read())
                {
                    lines.Add(string.Format(
                        CultureInfo.InvariantCulture,
                        "{0},{1},{2},{3},{4},{5}",
                        r.GetInt32(0),
                        r.GetString(1),
                        r.GetString(2),
                        r.GetInt32(3),
                        r.GetString(4),
                        r.GetString(5)));
                }
            }

            return string.Join("|", lines);
        }
        finally
        {
            ExecNonQuery(conn, "IF OBJECT_ID('dbo.DrvBulkTarget', 'U') IS NOT NULL DROP TABLE dbo.DrvBulkTarget;");
        }
    }

    private static string RunEfCore8Crud(string connStr)
    {
        const string dbName = "DrvEfCompatDb";
        using (SqlConnection admin = OpenConnection(connStr))
        {
            ExecNonQuery(admin, "IF DB_ID('" + dbName + "') IS NOT NULL DROP DATABASE " + dbName + ";");
        }

        SqlConnectionStringBuilder efBuilder = new SqlConnectionStringBuilder(connStr);
        efBuilder.InitialCatalog = dbName;
        efBuilder.Pooling = false;
        efBuilder.ConnectTimeout = 10;
        string efConnStr = efBuilder.ConnectionString;

        try
        {
            // 1. EnsureCreated creates DB + tables + PKs + FKs + indexes
            using (CompatDbContext ctx = new CompatDbContext(efConnStr))
            {
                ctx.Database.SetCommandTimeout(DefaultTimeoutSeconds);
                ctx.Database.EnsureCreated();
            }

            // 2. Insert parent + child graph via SaveChanges (identity propagation + batched inserts)
            int bobId;
            using (CompatDbContext ctx = new CompatDbContext(efConnStr))
            {
                ctx.Database.SetCommandTimeout(DefaultTimeoutSeconds);
                CompatCustomer alice = new CompatCustomer
                {
                    Name = "Alice",
                    IsActive = true,
                    Orders = new List<CompatOrder>
                    {
                        new CompatOrder { Code = "ORD-1", TotalAmount = 100.50m },
                        new CompatOrder { Code = "ORD-2", TotalAmount = 49.50m },
                    },
                };
                CompatCustomer bob = new CompatCustomer
                {
                    Name = "Bob",
                    IsActive = true,
                    Orders = new List<CompatOrder>
                    {
                        new CompatOrder { Code = "ORD-3", TotalAmount = 250.00m },
                    },
                };
                CompatCustomer carol = new CompatCustomer
                {
                    Name = "Carol",
                    IsActive = false,
                };

                ctx.Customers.AddRange(alice, bob, carol);
                ctx.SaveChanges();
                bobId = bob.Id;
            }

            // 3. Update & Delete via SaveChanges
            using (CompatDbContext ctx = new CompatDbContext(efConnStr))
            {
                ctx.Database.SetCommandTimeout(DefaultTimeoutSeconds);
                CompatCustomer? bob = ctx.Customers.Include(c => c.Orders).FirstOrDefault(c => c.Id == bobId);
                if (bob == null)
                {
                    throw new InvalidOperationException("Bob not found after SaveChanges.");
                }

                bob.Name = "Robert";
                bob.Orders.Add(new CompatOrder { Code = "ORD-4", TotalAmount = 75.25m });
                CompatCustomer? carol = ctx.Customers.FirstOrDefault(c => !c.IsActive);
                if (carol != null)
                {
                    ctx.Customers.Remove(carol);
                }

                ctx.SaveChanges();
            }

            // 4. LINQ Include + OrderBy + Skip/Take (OFFSET FETCH) + Aggregation
            using (CompatDbContext ctx = new CompatDbContext(efConnStr))
            {
                ctx.Database.SetCommandTimeout(DefaultTimeoutSeconds);
                List<CompatCustomer> list = ctx.Customers
                    .Include(c => c.Orders)
                    .Where(c => c.IsActive)
                    .OrderBy(c => c.Name)
                    .Skip(0)
                    .Take(5)
                    .ToList();

                List<string> summary = new List<string>();
                foreach (CompatCustomer c in list)
                {
                    List<string> orderParts = c.Orders
                        .OrderBy(o => o.Code)
                        .Select(o => string.Format(
                            CultureInfo.InvariantCulture,
                            "{0}:{1}",
                            o.Code,
                            o.TotalAmount.ToString("0.00", CultureInfo.InvariantCulture)))
                        .ToList();
                    summary.Add(string.Format(
                        CultureInfo.InvariantCulture,
                        "{0}[{1}]",
                        c.Name,
                        string.Join(",", orderParts)));
                }

                return string.Join("|", summary);
            }
        }
        finally
        {
            SqlConnection.ClearAllPools();
            using SqlConnection admin = OpenConnection(connStr);
            ExecNonQuery(admin, "IF DB_ID('" + dbName + "') IS NOT NULL DROP DATABASE " + dbName + ";");
        }
    }
}
