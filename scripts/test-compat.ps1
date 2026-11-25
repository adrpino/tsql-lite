#!/usr/bin/env pwsh
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $RepoRoot

$Image = if ($env:SQLSERVER_IMAGE) { $env:SQLSERVER_IMAGE } else { "mcr.microsoft.com/mssql/server:2022-latest" }
$Port = if ($env:SQLSERVER_PORT) { $env:SQLSERVER_PORT } else { "11433" }
$Password = if ($env:SQLSERVER_PASSWORD) { $env:SQLSERVER_PASSWORD } else { "TsqlLiteTest!2026" }
$ContainerName = "tsql_lite_diff_sqlserver_$PID"
$ManagedContainer = $false
$Runtime = $null

try {
    if (-not $env:SQLSERVER_HOST) {
        if (Get-Command docker -ErrorAction SilentlyContinue) {
            $Runtime = "docker"
        } elseif (Get-Command podman -ErrorAction SilentlyContinue) {
            $Runtime = "podman"
        } else {
            throw "Neither docker nor podman found (or set SQLSERVER_HOST/SQLSERVER_PORT/SQLSERVER_PASSWORD to use an existing SQL Server)."
        }

        Write-Host "Starting disposable SQL Server container ($Image) via $Runtime on 127.0.0.1:$Port..."
        & $Runtime run -d --name $ContainerName `
            -e ACCEPT_EULA=Y `
            -e "MSSQL_SA_PASSWORD=$Password" `
            -p "127.0.0.1:${Port}:1433" `
            $Image | Out-Null
        $ManagedContainer = $true

        Write-Host "Waiting for SQL Server readiness on 127.0.0.1:$Port..."
        $Ready = $false
        for ($attempt = 1; $attempt -le 60; $attempt++) {
            & $Runtime exec $ContainerName /opt/mssql-tools18/bin/sqlcmd -S localhost -U sa -P $Password -C -Q "SELECT 1" *> $null
            if ($LASTEXITCODE -eq 0) {
                $Ready = $true
                Write-Host "SQL Server ready (attempt $attempt)."
                break
            }
            & $Runtime exec $ContainerName /opt/mssql-tools/bin/sqlcmd -S localhost -U sa -P $Password -Q "SELECT 1" *> $null
            if ($LASTEXITCODE -eq 0) {
                $Ready = $true
                Write-Host "SQL Server ready (attempt $attempt)."
                break
            }
            Start-Sleep -Seconds 2
        }

        if (-not $Ready) {
            throw "SQL Server did not become ready in time."
        }
    }

    $env:SQLSERVER_HOST = if ($env:SQLSERVER_HOST) { $env:SQLSERVER_HOST } else { "127.0.0.1" }
    $env:SQLSERVER_PORT = $Port
    $env:SQLSERVER_USER = if ($env:SQLSERVER_USER) { $env:SQLSERVER_USER } else { "sa" }
    $env:SQLSERVER_PASSWORD = $Password

    & cargo test -p tsql_lite_core --test sqlserver_differential -- --ignored --nocapture @args
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
} finally {
    if ($ManagedContainer -and $Runtime) {
        Write-Host "Cleaning up disposable container $ContainerName..."
        & $Runtime rm -f $ContainerName *> $null
    }
}
