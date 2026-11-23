#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# Cross-platform differential compatibility runner against live SQL Server 2022.
# Uses an existing instance if SQLSERVER_HOST is set, otherwise spins up a
# disposable Docker/Podman container and tears it down automatically on exit.

IMAGE="${SQLSERVER_IMAGE:-mcr.microsoft.com/mssql/server:2022-latest}"
PORT="${SQLSERVER_PORT:-11433}"
PASSWORD="${SQLSERVER_PASSWORD:-IridiumTest!2026}"
CONTAINER_NAME="iridium_diff_sqlserver_$$"
MANAGED_CONTAINER=false
RUNTIME=""

cleanup() {
    if [ "$MANAGED_CONTAINER" = true ] && [ -n "$RUNTIME" ]; then
        echo "Cleaning up disposable container $CONTAINER_NAME..."
        "$RUNTIME" rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

if [ -z "${SQLSERVER_HOST:-}" ]; then
    if command -v docker >/dev/null 2>&1; then
        RUNTIME="docker"
    elif command -v podman >/dev/null 2>&1; then
        RUNTIME="podman"
    else
        echo "Error: neither docker nor podman found (or set SQLSERVER_HOST/SQLSERVER_PORT/SQLSERVER_PASSWORD to use an existing SQL Server)." >&2
        exit 1
    fi

    echo "Starting disposable SQL Server container ($IMAGE) via $RUNTIME on 127.0.0.1:$PORT..."
    PLATFORM_ARG=()
    if [ "$(uname -m)" = "arm64" ] || [ "$(uname -m)" = "aarch64" ]; then
        PLATFORM_ARG=(--platform linux/amd64)
    fi

    "$RUNTIME" run -d --name "$CONTAINER_NAME" \
        "${PLATFORM_ARG[@]}" \
        -e ACCEPT_EULA=Y \
        -e MSSQL_SA_PASSWORD="$PASSWORD" \
        -p "127.0.0.1:${PORT}:1433" \
        "$IMAGE" >/dev/null
    MANAGED_CONTAINER=true

    echo "Waiting for SQL Server readiness on 127.0.0.1:$PORT..."
    READY=false
    for attempt in $(seq 1 60); do
        if "$RUNTIME" exec "$CONTAINER_NAME" /opt/mssql-tools18/bin/sqlcmd \
            -S localhost -U sa -P "$PASSWORD" -C -Q "SELECT 1" >/dev/null 2>&1 \
           || "$RUNTIME" exec "$CONTAINER_NAME" /opt/mssql-tools/bin/sqlcmd \
            -S localhost -U sa -P "$PASSWORD" -Q "SELECT 1" >/dev/null 2>&1; then
            READY=true
            echo "SQL Server ready (attempt $attempt)."
            break
        fi
        sleep 2
    done

    if [ "$READY" != true ]; then
        echo "Error: SQL Server did not become ready in time." >&2
        "$RUNTIME" logs "$CONTAINER_NAME" | tail -n 30 >&2 || true
        exit 1
    fi
fi

SQLSERVER_HOST="${SQLSERVER_HOST:-127.0.0.1}" \
SQLSERVER_PORT="$PORT" \
SQLSERVER_USER="${SQLSERVER_USER:-sa}" \
SQLSERVER_PASSWORD="$PASSWORD" \
cargo test -p iridium_core --test sqlserver_differential -- --ignored --nocapture "$@"
