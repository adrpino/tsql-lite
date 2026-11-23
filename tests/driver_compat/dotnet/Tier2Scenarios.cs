using System;
using System.Collections.Generic;
using System.Data;
using System.Data.SqlTypes;
using System.Globalization;
using System.Linq;
using Microsoft.Data.SqlClient;
using Microsoft.EntityFrameworkCore;

namespace DriverCompat;

internal sealed class CompatLedgerEntry
{
    public int Id { get; set; }
    public Guid ExternalId { get; set; }
    public string Category { get; set; } = string.Empty;
    public decimal FeeMoney { get; set; }
    public DateTime CreatedUtc { get; set; }
    public DateTimeOffset RecordedAt { get; set; }
    public DateOnly BusinessDate { get; set; }
    public TimeOnly CutoffTime { get; set; }
}

internal sealed class RichTypesDbContext : DbContext
{
    private readonly string _connectionString;

    public RichTypesDbContext(string connectionString)
    {
        _connectionString = connectionString;
    }

    public DbSet<CompatLedgerEntry> Entries => Set<CompatLedgerEntry>();

    protected override void OnConfiguring(DbContextOptionsBuilder optionsBuilder)
    {
        optionsBuilder.UseSqlServer(_connectionString);
    }

    protected override void OnModelCreating(ModelBuilder modelBuilder)
    {
        modelBuilder.Entity<CompatLedgerEntry>(entity =>
        {
            entity.ToTable("EfRichLedger", "dbo");
            entity.HasKey(e => e.Id);
            entity.Property(e => e.Category).HasMaxLength(40).IsRequired();
            entity.Property(e => e.FeeMoney).HasColumnType("money");
            entity.Property(e => e.CreatedUtc).HasColumnType("datetime2(3)");
            entity.Property(e => e.RecordedAt).HasColumnType("datetimeoffset(3)");
        });
    }
}

internal static class Tier2Scenarios
{
    private const int DefaultTimeoutSeconds = 8;

    public static List<(string Name, Func<string, string> Run)> All()
    {
        return new List<(string Name, Func<string, string> Run)>
        {
            ("17_typed_wire_read_temporal_money_guid_xml_decimal", RunTypedWireRead),
            ("18_ef_core_8_rich_types_execute_update_delete_groupby", RunEfCoreRichTypes),
            ("19_multidb_catalog_isolation_and_cross_db_sp_executesql", RunMultiDbCatalogIsolation),
        };
    }

    private static SqlConnection OpenConnection(string connectionString)
    {
        SqlConnectionStringBuilder builder = new SqlConnectionStringBuilder(connectionString);
        builder.Pooling = false;
        builder.ConnectTimeout = 10;
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

    private static string RunTypedWireRead(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        ExecNonQuery(conn, "IF OBJECT_ID('dbo.DrvWireTypes', 'U') IS NOT NULL DROP TABLE dbo.DrvWireTypes;");
        ExecNonQuery(conn, @"
CREATE TABLE dbo.DrvWireTypes (
    Id INT PRIMARY KEY,
    ColDate DATE NULL,
    ColTime TIME(3) NULL,
    ColDt DATETIME NULL,
    ColSmallDt SMALLDATETIME NULL,
    ColDt2 DATETIME2(3) NULL,
    ColDto DATETIMEOFFSET(3) NULL,
    ColMoney MONEY NULL,
    ColNegMoney MONEY NULL,
    ColSmallMoney SMALLMONEY NULL,
    ColDec DECIMAL(12, 4) NULL,
    ColGuid UNIQUEIDENTIFIER NULL,
    ColXml XML NULL,
    ColNvarMax NVARCHAR(MAX) NULL,
    ColBinMax VARBINARY(MAX) NULL
);");

        try
        {
            ExecNonQuery(conn, @"
INSERT INTO dbo.DrvWireTypes VALUES (
    1,
    '2026-07-19',
    '14:35:42.125',
    '2026-07-19 14:35:42.500',
    '2026-07-19 14:35:00',
    '2026-07-19 14:35:42.875',
    '2026-07-19 14:35:42.500 -05:00',
    1234567.8901,
    -987654.3210,
    4321.5678,
    -12345.6789,
    '12345678-9ABC-DEF0-1234-56789ABCDEF0',
    N'<doc v=""1""><k>ok</k></doc>',
    REPLICATE(CAST(N'Z' AS NVARCHAR(MAX)), 5000),
    0xDEADBEEF01020304
);
INSERT INTO dbo.DrvWireTypes VALUES (
    2, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL
);");

            List<string> rows = new List<string>();
            using (SqlCommand cmd = new SqlCommand(@"
SELECT
    Id, ColDate, ColTime, ColDt, ColSmallDt, ColDt2, ColDto,
    ColMoney, ColNegMoney, ColSmallMoney, ColDec, ColGuid, ColXml, ColNvarMax, ColBinMax
FROM dbo.DrvWireTypes
ORDER BY Id;", conn))
            {
                cmd.CommandTimeout = DefaultTimeoutSeconds;
                using SqlDataReader r = cmd.ExecuteReader();
                while (r.Read())
                {
                    int id = r.GetInt32(0);
                    if (id == 1)
                    {
                        DateTime d = r.GetDateTime(1);
                        TimeSpan t = r.GetTimeSpan(2);
                        DateTime dt = r.GetDateTime(3);
                        DateTime sdt = r.GetDateTime(4);
                        DateTime dt2 = r.GetDateTime(5);
                        DateTimeOffset dto = r.GetDateTimeOffset(6);
                        decimal m = r.GetDecimal(7);
                        decimal nm = r.GetDecimal(8);
                        decimal sm = r.GetDecimal(9);
                        decimal dec = r.GetDecimal(10);
                        Guid g = r.GetGuid(11);
                        SqlXml xml = r.GetSqlXml(12);
                        string nv = r.GetString(13);
                        byte[] bin = r.GetSqlBinary(14).Value;

                        rows.Add(string.Format(
                            CultureInfo.InvariantCulture,
                            "1|d={0:yyyy-MM-dd}|t={1:hh\\:mm\\:ss\\.fff}|dt={2:yyyy-MM-dd HH:mm:ss.fff}|sdt={3:yyyy-MM-dd HH:mm}|dt2={4:yyyy-MM-dd HH:mm:ss.fff}|dto={5:yyyy-MM-dd HH:mm:ss.fff zzz}|m={6:0.0000}|nm={7:0.0000}|sm={8:0.0000}|dec={9:0.0000}|g={10}|xml={11}|nvlen={12}|bin={13}",
                            d,
                            t,
                            dt,
                            sdt,
                            dt2,
                            dto,
                            m,
                            nm,
                            sm,
                            dec,
                            g.ToString("D").ToUpperInvariant(),
                            xml.Value,
                            nv.Length,
                            Convert.ToHexString(bin)));
                    }
                    else
                    {
                        int nullCount = 0;
                        for (int c = 1; c < r.FieldCount; c++)
                        {
                            if (r.IsDBNull(c))
                            {
                                nullCount++;
                            }
                        }

                        rows.Add(string.Format(CultureInfo.InvariantCulture, "2|nulls={0}", nullCount));
                    }
                }
            }

            return string.Join("||", rows);
        }
        finally
        {
            if (conn.State == ConnectionState.Open)
            {
                ExecNonQuery(conn, "IF OBJECT_ID('dbo.DrvWireTypes', 'U') IS NOT NULL DROP TABLE dbo.DrvWireTypes;");
            }
        }
    }

    private static string RunEfCoreRichTypes(string connStr)
    {
        const string dbName = "DrvEfRichDb";
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
            using (RichTypesDbContext ctx = new RichTypesDbContext(efConnStr))
            {
                ctx.Database.SetCommandTimeout(DefaultTimeoutSeconds);
                ctx.Database.EnsureCreated();
            }

            Guid g1 = new Guid("11111111-1111-1111-1111-111111111111");
            Guid g2 = new Guid("22222222-2222-2222-2222-222222222222");
            Guid g3 = new Guid("33333333-3333-3333-3333-333333333333");

            using (RichTypesDbContext ctx = new RichTypesDbContext(efConnStr))
            {
                ctx.Database.SetCommandTimeout(DefaultTimeoutSeconds);
                ctx.Entries.AddRange(
                    new CompatLedgerEntry
                    {
                        ExternalId = g1,
                        Category = "A",
                        FeeMoney = 500000.25m,
                        CreatedUtc = new DateTime(2026, 6, 15, 10, 20, 30, 500),
                        RecordedAt = new DateTimeOffset(2026, 6, 15, 10, 20, 30, 500, TimeSpan.FromHours(2)),
                        BusinessDate = new DateOnly(2026, 6, 15),
                        CutoffTime = new TimeOnly(17, 30, 0),
                    },
                    new CompatLedgerEntry
                    {
                        ExternalId = g2,
                        Category = "A",
                        FeeMoney = 125.75m,
                        CreatedUtc = new DateTime(2026, 6, 16, 11, 0, 0, 0),
                        RecordedAt = new DateTimeOffset(2026, 6, 16, 11, 0, 0, 0, TimeSpan.FromHours(-4)),
                        BusinessDate = new DateOnly(2026, 6, 16),
                        CutoffTime = new TimeOnly(18, 0, 0),
                    },
                    new CompatLedgerEntry
                    {
                        ExternalId = g3,
                        Category = "TEMP",
                        FeeMoney = 10.00m,
                        CreatedUtc = new DateTime(2026, 6, 17, 12, 0, 0, 0),
                        RecordedAt = new DateTimeOffset(2026, 6, 17, 12, 0, 0, 0, TimeSpan.Zero),
                        BusinessDate = new DateOnly(2026, 6, 17),
                        CutoffTime = new TimeOnly(9, 15, 0),
                    });
                ctx.SaveChanges();
            }

            // EF Core 8 ExecuteUpdate + ExecuteDelete
            using (RichTypesDbContext ctx = new RichTypesDbContext(efConnStr))
            {
                ctx.Database.SetCommandTimeout(DefaultTimeoutSeconds);
                ctx.Entries
                    .Where(e => e.Category == "A")
                    .ExecuteUpdate(setters => setters.SetProperty(e => e.FeeMoney, e => e.FeeMoney + 10.00m));
                ctx.Entries
                    .Where(e => e.Category == "TEMP")
                    .ExecuteDelete();
            }

            // Read back typed entities + GroupBy aggregation
            using (RichTypesDbContext ctx = new RichTypesDbContext(efConnStr))
            {
                ctx.Database.SetCommandTimeout(DefaultTimeoutSeconds);
                List<CompatLedgerEntry> rows = ctx.Entries.OrderBy(e => e.BusinessDate).ToList();
                var grouped = ctx.Entries
                    .GroupBy(e => e.Category)
                    .Select(g => new { g.Key, Cnt = g.Count(), Total = g.Sum(x => x.FeeMoney) })
                    .OrderBy(x => x.Key)
                    .ToList();

                string rowPart = string.Join("|", rows.Select(r => string.Format(
                    CultureInfo.InvariantCulture,
                    "{0}:{1}:{2:0.00}:{3:yyyy-MM-dd HH:mm:ss.fff}:{4:yyyy-MM-dd HH:mm:ss.fff zzz}:{5:yyyy-MM-dd}:{6:HH\\:mm}",
                    r.ExternalId.ToString("D").ToUpperInvariant().Substring(0, 8),
                    r.Category,
                    r.FeeMoney,
                    r.CreatedUtc,
                    r.RecordedAt,
                    r.BusinessDate,
                    r.CutoffTime)));

                string grpPart = string.Join(",", grouped.Select(g => string.Format(
                    CultureInfo.InvariantCulture,
                    "{0}={1}/{2:0.00}",
                    g.Key,
                    g.Cnt,
                    g.Total)));

                return rowPart + "||GRP:" + grpPart;
            }
        }
        finally
        {
            SqlConnection.ClearAllPools();
            using SqlConnection admin = OpenConnection(connStr);
            ExecNonQuery(admin, "IF DB_ID('" + dbName + "') IS NOT NULL DROP DATABASE " + dbName + ";");
        }
    }

    private static string RunMultiDbCatalogIsolation(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        ExecNonQuery(conn, "IF DB_ID('DrvCatIsoA') IS NOT NULL DROP DATABASE DrvCatIsoA;");
        ExecNonQuery(conn, "IF DB_ID('DrvCatIsoB') IS NOT NULL DROP DATABASE DrvCatIsoB;");
        ExecNonQuery(conn, "CREATE DATABASE DrvCatIsoA;");
        ExecNonQuery(conn, "CREATE DATABASE DrvCatIsoB;");

        try
        {
            conn.ChangeDatabase("DrvCatIsoA");
            ExecNonQuery(conn, @"
CREATE TABLE dbo.ItemsA (
    Id INT PRIMARY KEY,
    Qty INT NOT NULL CONSTRAINT CK_ItemsA_Qty CHECK (Qty > 0)
);
CREATE INDEX IX_ItemsA_Qty ON dbo.ItemsA (Qty);
INSERT INTO dbo.ItemsA VALUES (1, 10), (2, 20);");
            ExecNonQuery(conn, "CREATE PROCEDURE dbo.usp_ProcA AS SELECT 1;");

            conn.ChangeDatabase("DrvCatIsoB");
            ExecNonQuery(conn, @"
CREATE TABLE dbo.ItemsB (
    Id INT PRIMARY KEY,
    Code NVARCHAR(20) NOT NULL
);
CREATE TABLE dbo.ItemsBChild (
    Id INT PRIMARY KEY,
    ParentId INT NOT NULL CONSTRAINT FK_ItemsBChild_Parent REFERENCES dbo.ItemsB(Id)
);");
            ExecNonQuery(conn, "CREATE PROCEDURE dbo.usp_ProcB AS SELECT 2;");

            // Verify DB B catalog views only see DB B objects (zero leakage from DB A)
            string catB;
            using (SqlCommand q = new SqlCommand(@"
SELECT
    (SELECT COUNT(*) FROM sys.tables WHERE name = 'ItemsA') AS LeakTbl,
    (SELECT COUNT(*) FROM sys.columns c INNER JOIN sys.tables t ON c.object_id = t.object_id WHERE t.name = 'ItemsA') AS LeakCol,
    (SELECT COUNT(*) FROM sys.indexes WHERE name = 'IX_ItemsA_Qty') AS LeakIdx,
    (SELECT COUNT(*) FROM sys.check_constraints WHERE name = 'CK_ItemsA_Qty') AS LeakCk,
    (SELECT COUNT(*) FROM sys.procedures WHERE name = 'usp_ProcA') AS LeakProc,
    (SELECT COUNT(*) FROM sys.foreign_keys WHERE name = 'FK_ItemsBChild_Parent') AS OwnFk,
    (SELECT COUNT(*) FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_NAME = 'ItemsA') AS LeakInfoTbl,
    (SELECT COUNT(*) FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = 'ItemsA') AS LeakInfoCol,
    (SELECT COUNT(*) FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_NAME = 'ItemsB') AS OwnInfoTbl;", conn))
            {
                q.CommandTimeout = DefaultTimeoutSeconds;
                using SqlDataReader r = q.ExecuteReader();
                r.Read();
                catB = string.Format(
                    CultureInfo.InvariantCulture,
                    "leak=({0},{1},{2},{3},{4},{5},{6})|own=({7},{8})",
                    r.GetInt32(0),
                    r.GetInt32(1),
                    r.GetInt32(2),
                    r.GetInt32(3),
                    r.GetInt32(4),
                    r.GetInt32(6),
                    r.GetInt32(7),
                    r.GetInt32(5),
                    r.GetInt32(8));
            }

            // Cross-database sp_executesql: while connected to DrvCatIsoB, execute dynamic SQL in DrvCatIsoA
            string crossExec;
            using (SqlCommand xq = new SqlCommand(
                "EXEC DrvCatIsoA.sys.sp_executesql N'SELECT DB_NAME() AS DbName, COUNT(*) AS Cnt FROM dbo.ItemsA';",
                conn))
            {
                xq.CommandTimeout = DefaultTimeoutSeconds;
                using SqlDataReader r = xq.ExecuteReader();
                r.Read();
                crossExec = string.Format(
                    CultureInfo.InvariantCulture,
                    "xexec={0}:{1}",
                    r.GetString(0),
                    r.GetInt32(1));
            }

            // Confirm caller DB context is still DrvCatIsoB after cross-DB sp_executesql
            string afterDb;
            using (SqlCommand aq = new SqlCommand("SELECT DB_NAME();", conn))
            {
                aq.CommandTimeout = DefaultTimeoutSeconds;
                afterDb = Convert.ToString(aq.ExecuteScalar(), CultureInfo.InvariantCulture) ?? string.Empty;
            }

            return catB + "|" + crossExec + "|after=" + afterDb;
        }
        finally
        {
            conn.ChangeDatabase("master");
            ExecNonQuery(conn, "IF DB_ID('DrvCatIsoA') IS NOT NULL DROP DATABASE DrvCatIsoA;");
            ExecNonQuery(conn, "IF DB_ID('DrvCatIsoB') IS NOT NULL DROP DATABASE DrvCatIsoB;");
        }
    }
}
