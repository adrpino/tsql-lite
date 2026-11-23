using System;
using System.Collections.Generic;
using System.Data;
using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Threading;
using Dapper;
using Microsoft.Data.SqlClient;

namespace DriverCompat;

internal sealed class ScenarioOutcome
{
    public string Name { get; set; } = string.Empty;
    public bool Ok { get; set; }
    public string Payload { get; set; } = string.Empty;
    public long ElapsedMs { get; set; }
}

internal static class Scenarios
{
    private const int DefaultTimeoutSeconds = 6;

    public static List<(string Name, Func<string, string> Run)> All()
    {
        return new List<(string Name, Func<string, string> Run)>
        {
            ("01_tx_begin_commit_rollback_save", RunTransactionManager),
            ("02_rpc_output_inout_return_sqlclient", RunRpcOutputSqlClient),
            ("03_rpc_output_inout_return_dapper", RunRpcOutputDapper),
            ("04_rpc_scalar_types_temporal_money_xml_guid", RunRpcScalarTypes),
            ("05_rpc_tvp_rich_columns", RunRpcTvpRichColumns),
            ("06_nocount_on_vs_off_execute_non_query", RunNocountExecuteNonQuery),
            ("07_dapper_query_multiple", RunDapperQueryMultiple),
            ("08_mars_concurrent_readers", RunMarsConcurrentReaders),
            ("09_command_cancel_attention", RunCommandCancelAttention),
            ("10_multidb_use_and_three_part_query", RunMultiDbThreePartQuery),
        };
    }

    private static SqlConnection OpenConnection(string connectionString, bool enableMars = false)
    {
        SqlConnectionStringBuilder builder = new SqlConnectionStringBuilder(connectionString);
        builder.Pooling = false;
        builder.ConnectTimeout = 10;
        builder.MultipleActiveResultSets = enableMars;
        SqlConnection conn = new SqlConnection(builder.ConnectionString);
        conn.Open();
        return conn;
    }

    private static void ExecNonQuery(SqlConnection conn, string sql, SqlTransaction? tx = null)
    {
        using SqlCommand cmd = new SqlCommand(sql, conn, tx);
        cmd.CommandTimeout = DefaultTimeoutSeconds;
        cmd.ExecuteNonQuery();
    }

    private static string RunTransactionManager(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        ExecNonQuery(conn, "IF OBJECT_ID('dbo.DrvTxItems', 'U') IS NOT NULL DROP TABLE dbo.DrvTxItems;");
        ExecNonQuery(conn, "CREATE TABLE dbo.DrvTxItems (Id INT PRIMARY KEY, Label NVARCHAR(50));");

        // Transaction 1: Commit with Savepoint rollback
        using (SqlTransaction tx1 = conn.BeginTransaction(IsolationLevel.ReadCommitted))
        {
            ExecNonQuery(conn, "INSERT INTO dbo.DrvTxItems (Id, Label) VALUES (1, N'kept');", tx1);
            tx1.Save("sp_after_one");
            ExecNonQuery(conn, "INSERT INTO dbo.DrvTxItems (Id, Label) VALUES (2, N'rolled_to_sp');", tx1);
            tx1.Rollback("sp_after_one");
            ExecNonQuery(conn, "INSERT INTO dbo.DrvTxItems (Id, Label) VALUES (3, N'after_sp');", tx1);
            tx1.Commit();
        }

        // Transaction 2: Full Rollback
        using (SqlTransaction tx2 = conn.BeginTransaction())
        {
            ExecNonQuery(conn, "INSERT INTO dbo.DrvTxItems (Id, Label) VALUES (4, N'discarded');", tx2);
            tx2.Rollback();
        }

        // Query committed rows after both transactions
        List<string> rows = new List<string>();
        using (SqlCommand q = new SqlCommand("SELECT Id, Label FROM dbo.DrvTxItems ORDER BY Id;", conn))
        {
            q.CommandTimeout = DefaultTimeoutSeconds;
            using SqlDataReader reader = q.ExecuteReader();
            while (reader.Read())
            {
                rows.Add(string.Format(CultureInfo.InvariantCulture, "{0}:{1}", reader.GetInt32(0), reader.GetString(1)));
            }
        }

        ExecNonQuery(conn, "DROP TABLE dbo.DrvTxItems;");
        return string.Join("|", rows);
    }

    private static string RunRpcOutputSqlClient(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        ExecNonQuery(conn, "IF OBJECT_ID('dbo.usp_DrvEchoOut', 'P') IS NOT NULL DROP PROCEDURE dbo.usp_DrvEchoOut;");
        ExecNonQuery(conn, @"
CREATE PROCEDURE dbo.usp_DrvEchoOut
    @InVal INT,
    @InOutVal INT OUTPUT,
    @OutTotal DECIMAL(10, 2) OUTPUT,
    @OutLabel NVARCHAR(100) OUTPUT
AS
BEGIN
    SET NOCOUNT ON;
    SET @InOutVal = @InOutVal + @InVal;
    SET @OutTotal = CAST(@InOutVal AS DECIMAL(10, 2)) * 2.50;
    SET @OutLabel = N'sum=' + CAST(@InOutVal AS NVARCHAR(20));
    SELECT @InVal AS InputCopy, @OutLabel AS LabelCopy;
    RETURN 77;
END");

        using SqlCommand cmd = new SqlCommand("dbo.usp_DrvEchoOut", conn);
        cmd.CommandType = CommandType.StoredProcedure;
        cmd.CommandTimeout = DefaultTimeoutSeconds;

        cmd.Parameters.Add("@InVal", SqlDbType.Int).Value = 15;

        SqlParameter pInOut = cmd.Parameters.Add("@InOutVal", SqlDbType.Int);
        pInOut.Direction = ParameterDirection.InputOutput;
        pInOut.Value = 5;

        SqlParameter pTotal = cmd.Parameters.Add("@OutTotal", SqlDbType.Decimal);
        pTotal.Direction = ParameterDirection.Output;
        pTotal.Precision = 10;
        pTotal.Scale = 2;

        SqlParameter pLabel = cmd.Parameters.Add("@OutLabel", SqlDbType.NVarChar, 100);
        pLabel.Direction = ParameterDirection.Output;

        SqlParameter pRet = cmd.Parameters.Add("@RetVal", SqlDbType.Int);
        pRet.Direction = ParameterDirection.ReturnValue;

        string rowText = string.Empty;
        using (SqlDataReader reader = cmd.ExecuteReader())
        {
            if (reader.Read())
            {
                rowText = string.Format(
                    CultureInfo.InvariantCulture,
                    "{0}/{1}",
                    reader.GetInt32(0),
                    reader.GetString(1));
            }
        }

        string result = string.Format(
            CultureInfo.InvariantCulture,
            "row={0};inout={1};total={2};label={3};ret={4}",
            rowText,
            pInOut.Value,
            Convert.ToDecimal(pTotal.Value, CultureInfo.InvariantCulture).ToString("0.00", CultureInfo.InvariantCulture),
            pLabel.Value,
            pRet.Value);

        ExecNonQuery(conn, "DROP PROCEDURE dbo.usp_DrvEchoOut;");
        return result;
    }

    private static string RunRpcOutputDapper(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        ExecNonQuery(conn, "IF OBJECT_ID('dbo.usp_DrvDapperOut', 'P') IS NOT NULL DROP PROCEDURE dbo.usp_DrvDapperOut;");
        ExecNonQuery(conn, @"
CREATE PROCEDURE dbo.usp_DrvDapperOut
    @Seed INT,
    @Doubled INT OUTPUT,
    @Tag NVARCHAR(64) OUTPUT
AS
BEGIN
    SET NOCOUNT ON;
    SET @Doubled = @Seed * 2;
    SET @Tag = N'tag-' + CAST(@Seed AS NVARCHAR(20));
    SELECT @Seed AS SeedVal;
    RETURN 42;
END");

        DynamicParameters dp = new DynamicParameters();
        dp.Add("@Seed", 21, DbType.Int32, ParameterDirection.Input);
        dp.Add("@Doubled", dbType: DbType.Int32, direction: ParameterDirection.Output);
        dp.Add("@Tag", dbType: DbType.String, direction: ParameterDirection.Output, size: 64);
        dp.Add("@Ret", dbType: DbType.Int32, direction: ParameterDirection.ReturnValue);

        int seedRow = conn.QuerySingle<int>(
            "dbo.usp_DrvDapperOut",
            dp,
            commandType: CommandType.StoredProcedure,
            commandTimeout: DefaultTimeoutSeconds);

        string result = string.Format(
            CultureInfo.InvariantCulture,
            "row={0};doubled={1};tag={2};ret={3}",
            seedRow,
            dp.Get<int>("@Doubled"),
            dp.Get<string>("@Tag"),
            dp.Get<int>("@Ret"));

        ExecNonQuery(conn, "DROP PROCEDURE dbo.usp_DrvDapperOut;");
        return result;
    }

    private static string RunRpcScalarTypes(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        Guid sampleGuid = new Guid("12345678-9ABC-DEF0-1234-56789ABCDEF0");
        DateTime sampleDate = new DateTime(2026, 7, 19);
        TimeSpan sampleTime = new TimeSpan(0, 14, 35, 42, 125);
        DateTime sampleDt = new DateTime(2026, 7, 19, 14, 35, 42);
        DateTime sampleDt2 = new DateTime(2026, 7, 19, 14, 35, 42, 500);
        DateTimeOffset sampleDto = new DateTimeOffset(2026, 7, 19, 14, 35, 42, TimeSpan.FromHours(-5));
        decimal sampleMoney = 12345.67m;
        double sampleFloat = 3.125;
        float sampleReal = 2.5f;
        decimal sampleDec = -9876.5432m;
        string sampleXml = "<item id=\"7\"><name>widget</name></item>";
        string sampleLongNvar = new string('A', 4500);
        byte[] sampleBin = new byte[] { 0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02 };

        using SqlCommand cmd = new SqlCommand(@"
SELECT
    CONVERT(VARCHAR(10), @pDate, 23) AS D,
    CONVERT(VARCHAR(12), @pTime, 114) AS T,
    CONVERT(VARCHAR(19), @pDt, 120) AS DT,
    CONVERT(VARCHAR(23), @pDt2, 121) AS DT2,
    DATEPART(TZOFFSET, @pDto) AS DTO_TZ,
    CONVERT(VARCHAR(19), @pDto, 120) AS DTO_DT,
    CAST(@pMoney AS DECIMAL(12, 2)) AS M,
    CAST(@pFloat AS DECIMAL(10, 3)) AS F,
    CAST(@pReal AS DECIMAL(10, 2)) AS R,
    @pDec AS DEC_VAL,
    UPPER(CAST(@pGuid AS VARCHAR(36))) AS G,
    @pXml.value('(/item/@id)[1]', 'INT') AS X_ID,
    LEN(@pNvarMax) AS NV_LEN,
    DATALENGTH(@pBinMax) AS BIN_LEN,
    CASE WHEN @pNullDate IS NULL THEN 1 ELSE 0 END AS NULL_D_OK,
    CASE WHEN @pNullMoney IS NULL THEN 1 ELSE 0 END AS NULL_M_OK;", conn);
        cmd.CommandTimeout = DefaultTimeoutSeconds;

        cmd.Parameters.Add("@pDate", SqlDbType.Date).Value = sampleDate;
        cmd.Parameters.Add("@pTime", SqlDbType.Time).Value = sampleTime;
        cmd.Parameters.Add("@pDt", SqlDbType.DateTime).Value = sampleDt;

        SqlParameter pDt2 = cmd.Parameters.Add("@pDt2", SqlDbType.DateTime2);
        pDt2.Scale = 3;
        pDt2.Value = sampleDt2;

        SqlParameter pDto = cmd.Parameters.Add("@pDto", SqlDbType.DateTimeOffset);
        pDto.Scale = 3;
        pDto.Value = sampleDto;

        cmd.Parameters.Add("@pMoney", SqlDbType.Money).Value = sampleMoney;
        cmd.Parameters.Add("@pFloat", SqlDbType.Float).Value = sampleFloat;
        cmd.Parameters.Add("@pReal", SqlDbType.Real).Value = sampleReal;

        SqlParameter pDec = cmd.Parameters.Add("@pDec", SqlDbType.Decimal);
        pDec.Precision = 12;
        pDec.Scale = 4;
        pDec.Value = sampleDec;

        cmd.Parameters.Add("@pGuid", SqlDbType.UniqueIdentifier).Value = sampleGuid;
        cmd.Parameters.Add("@pXml", SqlDbType.Xml).Value = sampleXml;
        cmd.Parameters.Add("@pNvarMax", SqlDbType.NVarChar, -1).Value = sampleLongNvar;
        cmd.Parameters.Add("@pBinMax", SqlDbType.VarBinary, -1).Value = sampleBin;
        cmd.Parameters.Add("@pNullDate", SqlDbType.Date).Value = DBNull.Value;
        cmd.Parameters.Add("@pNullMoney", SqlDbType.Money).Value = DBNull.Value;

        using SqlDataReader reader = cmd.ExecuteReader();
        if (!reader.Read())
        {
            throw new InvalidOperationException("No row returned from scalar type probe.");
        }

        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < reader.FieldCount; i++)
        {
            if (i > 0)
            {
                sb.Append(';');
            }

            object val = reader.GetValue(i);
            string formatted = val is decimal dec
                ? dec.ToString(CultureInfo.InvariantCulture)
                : Convert.ToString(val, CultureInfo.InvariantCulture) ?? "NULL";
            sb.Append(reader.GetName(i)).Append('=').Append(formatted);
        }

        return sb.ToString();
    }

    private static string RunRpcTvpRichColumns(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        ExecNonQuery(conn, "IF OBJECT_ID('dbo.usp_DrvConsumeTvp', 'P') IS NOT NULL DROP PROCEDURE dbo.usp_DrvConsumeTvp;");
        ExecNonQuery(conn, "IF TYPE_ID('dbo.DrvRichTvp') IS NOT NULL DROP TYPE dbo.DrvRichTvp;");
        ExecNonQuery(conn, @"
CREATE TYPE dbo.DrvRichTvp AS TABLE (
    Id INT NOT NULL,
    Score FLOAT NULL,
    Amount DECIMAL(10, 2) NULL,
    Price MONEY NULL,
    Active BIT NOT NULL,
    RefId UNIQUEIDENTIFIER NULL,
    EventDate DATE NULL,
    CreatedAt DATETIME2(3) NULL,
    Notes NVARCHAR(MAX) NULL,
    Payload VARBINARY(MAX) NULL
);");
        ExecNonQuery(conn, @"
CREATE PROCEDURE dbo.usp_DrvConsumeTvp
    @Rows dbo.DrvRichTvp READONLY
AS
BEGIN
    SET NOCOUNT ON;
    SELECT
        Id,
        CAST(Score AS DECIMAL(10, 3)) AS ScoreVal,
        Amount,
        CAST(Price AS DECIMAL(10, 2)) AS PriceVal,
        CAST(Active AS INT) AS ActiveVal,
        UPPER(CAST(RefId AS VARCHAR(36))) AS RefVal,
        CONVERT(VARCHAR(10), EventDate, 23) AS DateVal,
        CONVERT(VARCHAR(23), CreatedAt, 121) AS Dt2Val,
        Notes,
        DATALENGTH(Payload) AS PayloadLen
    FROM @Rows
    ORDER BY Id;
END");

        DataTable tvp = new DataTable();
        tvp.Columns.Add("Id", typeof(int));
        tvp.Columns.Add("Score", typeof(double));
        tvp.Columns.Add("Amount", typeof(decimal));
        tvp.Columns.Add("Price", typeof(decimal));
        tvp.Columns.Add("Active", typeof(bool));
        tvp.Columns.Add("RefId", typeof(Guid));
        tvp.Columns.Add("EventDate", typeof(DateTime));
        tvp.Columns.Add("CreatedAt", typeof(DateTime));
        tvp.Columns.Add("Notes", typeof(string));
        tvp.Columns.Add("Payload", typeof(byte[]));

        tvp.Rows.Add(
            1,
            1.75,
            42.50m,
            19.95m,
            true,
            new Guid("AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE"),
            new DateTime(2026, 3, 15),
            new DateTime(2026, 3, 15, 9, 30, 15, 250),
            "hello-plp-tvp",
            new byte[] { 0x01, 0x02, 0x03, 0x04 });
        tvp.Rows.Add(
            2,
            DBNull.Value,
            DBNull.Value,
            DBNull.Value,
            false,
            DBNull.Value,
            DBNull.Value,
            DBNull.Value,
            DBNull.Value,
            DBNull.Value);

        using SqlCommand cmd = new SqlCommand("dbo.usp_DrvConsumeTvp", conn);
        cmd.CommandType = CommandType.StoredProcedure;
        cmd.CommandTimeout = DefaultTimeoutSeconds;
        SqlParameter tvpParam = cmd.Parameters.AddWithValue("@Rows", tvp);
        tvpParam.SqlDbType = SqlDbType.Structured;
        tvpParam.TypeName = "dbo.DrvRichTvp";

        List<string> lines = new List<string>();
        using (SqlDataReader reader = cmd.ExecuteReader())
        {
            while (reader.Read())
            {
                List<string> cols = new List<string>();
                for (int i = 0; i < reader.FieldCount; i++)
                {
                    if (reader.IsDBNull(i))
                    {
                        cols.Add("NULL");
                    }
                    else
                    {
                        object v = reader.GetValue(i);
                        cols.Add(v is decimal d
                            ? d.ToString(CultureInfo.InvariantCulture)
                            : Convert.ToString(v, CultureInfo.InvariantCulture) ?? "NULL");
                    }
                }

                lines.Add(string.Join(",", cols));
            }
        }

        ExecNonQuery(conn, "DROP PROCEDURE dbo.usp_DrvConsumeTvp;");
        ExecNonQuery(conn, "DROP TYPE dbo.DrvRichTvp;");
        return string.Join("|", lines);
    }

    private static string RunNocountExecuteNonQuery(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        ExecNonQuery(conn, "IF OBJECT_ID('dbo.DrvNocountTbl', 'U') IS NOT NULL DROP TABLE dbo.DrvNocountTbl;");
        ExecNonQuery(conn, "CREATE TABLE dbo.DrvNocountTbl (Id INT PRIMARY KEY);");

        int withCountOff;
        using (SqlCommand cmd = new SqlCommand("SET NOCOUNT OFF; INSERT INTO dbo.DrvNocountTbl VALUES (1), (2);", conn))
        {
            cmd.CommandTimeout = DefaultTimeoutSeconds;
            withCountOff = cmd.ExecuteNonQuery();
        }

        int withCountOn;
        using (SqlCommand cmd = new SqlCommand("SET NOCOUNT ON; INSERT INTO dbo.DrvNocountTbl VALUES (3), (4), (5);", conn))
        {
            cmd.CommandTimeout = DefaultTimeoutSeconds;
            withCountOn = cmd.ExecuteNonQuery();
        }

        int withCountOffAgain;
        using (SqlCommand cmd = new SqlCommand("SET NOCOUNT OFF; DELETE FROM dbo.DrvNocountTbl WHERE Id <= 2;", conn))
        {
            cmd.CommandTimeout = DefaultTimeoutSeconds;
            withCountOffAgain = cmd.ExecuteNonQuery();
        }

        ExecNonQuery(conn, "DROP TABLE dbo.DrvNocountTbl;");
        return string.Format(
            CultureInfo.InvariantCulture,
            "off={0};on={1};off_del={2}",
            withCountOff,
            withCountOn,
            withCountOffAgain);
    }

    private static string RunDapperQueryMultiple(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        using SqlMapper.GridReader multi = conn.QueryMultiple(
            "SELECT 10 AS A; SELECT N'alpha' AS B UNION ALL SELECT N'beta'; SELECT CAST(3.50 AS DECIMAL(6,2)) AS C;",
            commandTimeout: DefaultTimeoutSeconds);

        int first = multi.ReadSingle<int>();
        List<string> second = multi.Read<string>().AsList();
        decimal third = multi.ReadSingle<decimal>();

        return string.Format(
            CultureInfo.InvariantCulture,
            "{0}|[{1}]|{2}",
            first,
            string.Join(",", second),
            third.ToString("0.00", CultureInfo.InvariantCulture));
    }

    private static string RunMarsConcurrentReaders(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr, enableMars: true);
        using SqlCommand cmd1 = new SqlCommand("SELECT 1 AS V UNION ALL SELECT 2;", conn);
        using SqlCommand cmd2 = new SqlCommand("SELECT N'x' AS T UNION ALL SELECT N'y';", conn);
        cmd1.CommandTimeout = DefaultTimeoutSeconds;
        cmd2.CommandTimeout = DefaultTimeoutSeconds;

        List<string> pairs = new List<string>();
        using (SqlDataReader r1 = cmd1.ExecuteReader())
        using (SqlDataReader r2 = cmd2.ExecuteReader())
        {
            while (r1.Read() && r2.Read())
            {
                pairs.Add(string.Format(CultureInfo.InvariantCulture, "{0}:{1}", r1.GetInt32(0), r2.GetString(0)));
            }
        }

        return string.Join("|", pairs);
    }

    private static string RunCommandCancelAttention(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        using SqlCommand cmd = new SqlCommand("WAITFOR DELAY '00:00:05'; SELECT 1;", conn);
        cmd.CommandTimeout = 10;

        Stopwatch sw = Stopwatch.StartNew();
        using Timer cancelTimer = new Timer(_ =>
        {
            try
            {
                cmd.Cancel();
            }
            catch
            {
                // Ignore race if command already completed.
            }
        }, null, 350, Timeout.Infinite);

        bool cancelled = false;
        try
        {
            cmd.ExecuteScalar();
        }
        catch (SqlException)
        {
            cancelled = true;
        }

        sw.Stop();
        bool fastEnough = sw.ElapsedMilliseconds < 2500;

        // Verify connection is healthy after cancellation
        using SqlCommand afterCmd = new SqlCommand("SELECT 42;", conn);
        afterCmd.CommandTimeout = DefaultTimeoutSeconds;
        int afterVal = Convert.ToInt32(afterCmd.ExecuteScalar(), CultureInfo.InvariantCulture);

        return string.Format(
            CultureInfo.InvariantCulture,
            "cancelled={0};fast={1};after={2}",
            cancelled,
            fastEnough,
            afterVal);
    }

    private static string RunMultiDbThreePartQuery(string connStr)
    {
        using SqlConnection conn = OpenConnection(connStr);
        ExecNonQuery(conn, "IF DB_ID('DrvCompatAlpha') IS NOT NULL DROP DATABASE DrvCompatAlpha;");
        ExecNonQuery(conn, "IF DB_ID('DrvCompatBeta') IS NOT NULL DROP DATABASE DrvCompatBeta;");
        ExecNonQuery(conn, "CREATE DATABASE DrvCompatAlpha;");
        ExecNonQuery(conn, "CREATE DATABASE DrvCompatBeta;");

        conn.ChangeDatabase("DrvCompatAlpha");
        ExecNonQuery(conn, "CREATE TABLE dbo.Ledger (Id INT PRIMARY KEY, Tag NVARCHAR(30));");
        ExecNonQuery(conn, "INSERT INTO dbo.Ledger VALUES (1, N'alpha-1'), (2, N'alpha-2');");

        conn.ChangeDatabase("DrvCompatBeta");
        ExecNonQuery(conn, "CREATE TABLE dbo.Ledger (Id INT PRIMARY KEY, Amount INT);");
        ExecNonQuery(conn, "INSERT INTO dbo.Ledger VALUES (1, 100), (2, 250);");

        List<string> rows = new List<string>();
        using (SqlCommand q = new SqlCommand(@"
SELECT a.Id, a.Tag, b.Amount
FROM DrvCompatAlpha.dbo.Ledger a
INNER JOIN DrvCompatBeta.dbo.Ledger b ON a.Id = b.Id
ORDER BY a.Id;", conn))
        {
            q.CommandTimeout = DefaultTimeoutSeconds;
            using SqlDataReader reader = q.ExecuteReader();
            while (reader.Read())
            {
                rows.Add(string.Format(
                    CultureInfo.InvariantCulture,
                    "{0}:{1}:{2}",
                    reader.GetInt32(0),
                    reader.GetString(1),
                    reader.GetInt32(2)));
            }
        }

        conn.ChangeDatabase("master");
        ExecNonQuery(conn, "DROP DATABASE DrvCompatAlpha;");
        ExecNonQuery(conn, "DROP DATABASE DrvCompatBeta;");
        return string.Join("|", rows);
    }
}
