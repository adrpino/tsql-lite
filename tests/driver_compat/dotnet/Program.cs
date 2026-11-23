using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;

namespace DriverCompat;

internal static class Program
{
    public static int Main(string[] args)
    {
        string? oracleConn = null;
        string? targetConn = null;
        string? filter = null;

        for (int i = 0; i < args.Length; i++)
        {
            if (args[i] == "--oracle" && i + 1 < args.Length)
            {
                oracleConn = args[++i];
            }
            else if (args[i] == "--target" && i + 1 < args.Length)
            {
                targetConn = args[++i];
            }
            else if (args[i] == "--filter" && i + 1 < args.Length)
            {
                filter = args[++i];
            }
        }

        if (string.IsNullOrWhiteSpace(oracleConn) || string.IsNullOrWhiteSpace(targetConn))
        {
            Console.Error.WriteLine("Usage: DriverCompat --oracle <sqlserver-connstr> --target <iridium-connstr> [--filter <substring>]");
            return 2;
        }

        List<(string Name, Func<string, string> Run)> scenarios = Scenarios.All();
        int passed = 0;
        int failed = 0;

        Console.WriteLine("========================================================================");
        Console.WriteLine("  .NET Microsoft.Data.SqlClient + Dapper Differential Oracle Suite");
        Console.WriteLine("========================================================================");

        foreach ((string name, Func<string, string> run) in scenarios)
        {
            if (!string.IsNullOrEmpty(filter)
                && name.IndexOf(filter, StringComparison.OrdinalIgnoreCase) < 0)
            {
                continue;
            }

            ScenarioOutcome oracleOutcome = ExecuteScenario(name, run, oracleConn);
            ScenarioOutcome targetOutcome = ExecuteScenario(name, run, targetConn);

            bool matched = oracleOutcome.Ok
                && targetOutcome.Ok
                && string.Equals(oracleOutcome.Payload, targetOutcome.Payload, StringComparison.Ordinal);

            if (matched)
            {
                passed++;
                Console.WriteLine(string.Format(
                    CultureInfo.InvariantCulture,
                    "[PASS] {0}  (sqlserver={1}ms, iridium={2}ms) => {3}",
                    name,
                    oracleOutcome.ElapsedMs,
                    targetOutcome.ElapsedMs,
                    targetOutcome.Payload));
            }
            else
            {
                failed++;
                Console.WriteLine(string.Format(
                    CultureInfo.InvariantCulture,
                    "[DIFF] {0}  (sqlserver={1}ms, iridium={2}ms)",
                    name,
                    oracleOutcome.ElapsedMs,
                    targetOutcome.ElapsedMs));
                Console.WriteLine(string.Format(
                    CultureInfo.InvariantCulture,
                    "       SQL Server 2022 [{0}]: {1}",
                    oracleOutcome.Ok ? "OK" : "ERR",
                    oracleOutcome.Payload));
                Console.WriteLine(string.Format(
                    CultureInfo.InvariantCulture,
                    "       Iridium Server  [{0}]: {1}",
                    targetOutcome.Ok ? "OK" : "ERR",
                    targetOutcome.Payload));
            }
        }

        Console.WriteLine("========================================================================");
        Console.WriteLine(string.Format(
            CultureInfo.InvariantCulture,
            "Summary: {0} passed, {1} failed ({2} total)",
            passed,
            failed,
            passed + failed));
        Console.WriteLine("========================================================================");

        return failed == 0 ? 0 : 1;
    }

    private static ScenarioOutcome ExecuteScenario(string name, Func<string, string> run, string connStr)
    {
        Stopwatch sw = Stopwatch.StartNew();
        try
        {
            string payload = run(connStr);
            sw.Stop();
            return new ScenarioOutcome
            {
                Name = name,
                Ok = true,
                Payload = payload,
                ElapsedMs = sw.ElapsedMilliseconds,
            };
        }
        catch (Exception ex)
        {
            sw.Stop();
            string message = ex.Message.Replace('\r', ' ').Replace('\n', ' ').Trim();
            return new ScenarioOutcome
            {
                Name = name,
                Ok = false,
                Payload = string.Format(CultureInfo.InvariantCulture, "{0}: {1}", ex.GetType().Name, message),
                ElapsedMs = sw.ElapsedMilliseconds,
            };
        }
    }
}
