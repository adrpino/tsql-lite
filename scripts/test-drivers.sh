#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# Portable containerized differential driver compatibility suite.
# Runs real Microsoft.Data.SqlClient + Dapper inside mcr.microsoft.com/dotnet/sdk:8.0
# against BOTH a live SQL Server 2022 container (ground-truth oracle) and iridium-server.
# Requires only Docker or Podman + Rust (no local .NET SDK needed).

SQLSERVER_IMAGE="${SQLSERVER_IMAGE:-mcr.microsoft.com/mssql/server:2022-latest}"
DOTNET_IMAGE="${DOTNET_IMAGE:-mcr.microsoft.com/dotnet/sdk:8.0}"
PASSWORD="${SQLSERVER_PASSWORD:-IridiumTest!2026}"
IRIDIUM_PORT="${IRIDIUM_DRIVER_TEST_PORT:-14433}"
NET_NAME="iridium_drv_net_$$"
SQL_CONTAINER="iridium_drv_sqlserver_$$"
NUGET_VOL="iridium_drv_nuget_cache"
IRIDIUM_PID=""
IRIDIUM_LOG="/tmp/iridium_drv_server_$$.log"
RUNTIME=""

if command -v docker >/dev/null 2>&1; then
    RUNTIME="docker"
elif command -v podman >/dev/null 2>&1; then
    RUNTIME="podman"
else
    echo "Error: neither docker nor podman found." >&2
    exit 1
fi

cleanup() {
    if [ -n "$IRIDIUM_PID" ]; then
        kill "$IRIDIUM_PID" 2>/dev/null || true
        wait "$IRIDIUM_PID" 2>/dev/null || true
    fi
    "$RUNTIME" rm -f "$SQL_CONTAINER" >/dev/null 2>&1 || true
    "$RUNTIME" network rm "$NET_NAME" >/dev/null 2>&1 || true
    rm -f "$IRIDIUM_LOG"
}
trap cleanup EXIT

echo "1/4 Building tsql-lite-server (release)..."
cargo build --release -p tsql-lite-server --bin tsql-lite-server

echo "2/4 Starting tsql-lite-server on 0.0.0.0:${IRIDIUM_PORT}..."
RUST_LOG="${RUST_LOG:-warn}" "$REPO_ROOT/target/release/tsql-lite-server" \
    --memory --tls-gen --host 0.0.0.0 --port "$IRIDIUM_PORT" \
    -u sa -P "$PASSWORD" -d master >"$IRIDIUM_LOG" 2>&1 &
IRIDIUM_PID=$!

for attempt in $(seq 1 30); do
    if nc -z 127.0.0.1 "$IRIDIUM_PORT" >/dev/null 2>&1; then
        break
    fi
    sleep 0.2
done

echo "3/4 Starting disposable SQL Server 2022 oracle container ($SQLSERVER_IMAGE)..."
"$RUNTIME" network create "$NET_NAME" >/dev/null
PLATFORM_ARG=()
if [ "$(uname -m)" = "arm64" ] || [ "$(uname -m)" = "aarch64" ]; then
    PLATFORM_ARG=(--platform linux/amd64)
fi

"$RUNTIME" run -d --name "$SQL_CONTAINER" \
    ${PLATFORM_ARG[@]+"${PLATFORM_ARG[@]}"} \
    --network "$NET_NAME" --network-alias sqlserver \
    -e ACCEPT_EULA=Y \
    -e MSSQL_SA_PASSWORD="$PASSWORD" \
    "$SQLSERVER_IMAGE" >/dev/null

READY=false
for attempt in $(seq 1 60); do
    if "$RUNTIME" exec "$SQL_CONTAINER" /opt/mssql-tools18/bin/sqlcmd \
        -S localhost -U sa -P "$PASSWORD" -C -Q "SELECT 1" >/dev/null 2>&1 \
       || "$RUNTIME" exec "$SQL_CONTAINER" /opt/mssql-tools/bin/sqlcmd \
        -S localhost -U sa -P "$PASSWORD" -Q "SELECT 1" >/dev/null 2>&1; then
        READY=true
        echo "    SQL Server 2022 oracle ready (attempt $attempt)."
        break
    fi
    sleep 2
done

if [ "$READY" != true ]; then
    echo "Error: SQL Server oracle did not become ready in time." >&2
    "$RUNTIME" logs "$SQL_CONTAINER" | tail -n 30 >&2 || true
    exit 1
fi

HOST_GATEWAY_ARG=()
if [ "$(uname -s)" = "Linux" ]; then
    HOST_GATEWAY_ARG=(--add-host=host.docker.internal:host-gateway)
fi

ORACLE_CONN="Server=sqlserver,1433;Database=master;User Id=sa;Password=${PASSWORD};Encrypt=True;TrustServerCertificate=True;"
IRIDIUM_CONN="Server=host.docker.internal,${IRIDIUM_PORT};Database=master;User Id=sa;Password=${PASSWORD};Encrypt=True;TrustServerCertificate=True;"

echo "4/4 Running containerized .NET Microsoft.Data.SqlClient + Dapper suite..."
"$RUNTIME" run --rm \
    --network "$NET_NAME" \
    ${HOST_GATEWAY_ARG[@]+"${HOST_GATEWAY_ARG[@]}"} \
    -v "$REPO_ROOT/tests/driver_compat/dotnet:/src:ro" \
    -v "${NUGET_VOL}:/root/.nuget/packages" \
    -w /work \
    "$DOTNET_IMAGE" \
    bash -c 'cp -r /src/. /work/ && dotnet run --configuration Release -- --oracle "$1" --target "$2" "${@:3}"' \
    _ "$ORACLE_CONN" "$IRIDIUM_CONN" "$@"
