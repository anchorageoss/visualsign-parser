#!/bin/sh
set -e

# Default values
PARSER_HOST="${PARSER_HOST:-0.0.0.0}"
PARSER_PORT="${PARSER_PORT:-44020}"
PARSER_METRICS_PORT="${PARSER_METRICS_PORT:-44021}"
PARSER_OUTER_SOCKET_PATH="${PARSER_OUTER_SOCKET_PATH:-/tmp/outer_parser.sock}"
PARSER_INNER_SOCKET_PATH="${PARSER_INNER_SOCKET_PATH:-/tmp/inner_parser.sock}"
EPHEMERAL_FILE="${EPHEMERAL_FILE:-${ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || echo .)}/src/integration/fixtures/ephemeral.secret}"
GATEWAY_PORT="${GATEWAY_PORT:-8080}"

# Binary locations (use cargo run in dev, direct binaries in container)
PARSER_APP_BIN="${PARSER_APP_BIN:-cargo run --bin parser_app --}"
SIMULATOR_ENCLAVE_BIN="${SIMULATOR_ENCLAVE_BIN:-cargo run --bin simulator_enclave --}"
PARSER_HOST_BIN="${PARSER_HOST_BIN:-cargo run --bin parser_host --}"
GATEWAY_BIN="${GATEWAY_BIN:-/usr/local/bin/gateway}"

# Timeout for waiting (in seconds)
TIMEOUT="${TIMEOUT:-30}"

# Clean up any existing sockets
rm -f "${PARSER_OUTER_SOCKET_PATH}" "${PARSER_INNER_SOCKET_PATH}"

# Trap to clean up background processes on exit
cleanup() {
    echo "Shutting down..."
    [ -n "$APP_PID" ] && kill $APP_PID 2>/dev/null || true
    [ -n "$ENCLAVE_PID" ] && kill $ENCLAVE_PID 2>/dev/null || true
    [ -n "$GATEWAY_PID" ] && kill $GATEWAY_PID 2>/dev/null || true
    rm -f "${PARSER_OUTER_SOCKET_PATH}" "${PARSER_INNER_SOCKET_PATH}"
}
trap cleanup EXIT INT TERM

# Function to wait for a socket to exist
wait_for_socket() {
    local socket_path="$1"
    local component_name="$2"
    local elapsed=0

    echo "Waiting for ${component_name} socket at ${socket_path}..."
    while [ ! -S "${socket_path}" ]; do
        if [ $elapsed -ge $TIMEOUT ]; then
            echo "ERROR: Timeout waiting for ${component_name} socket"
            return 1
        fi
        sleep 0.1
        elapsed=$((elapsed + 1))
    done
    echo "${component_name} socket ready"
}

# Start parser_app in background
echo "Starting parser_app..."
$PARSER_APP_BIN \
    --usock "${PARSER_INNER_SOCKET_PATH}" \
    --ephemeral-file "${EPHEMERAL_FILE}" &
APP_PID=$!

# Wait for parser_app socket to be created
wait_for_socket "${PARSER_INNER_SOCKET_PATH}" "parser_app"

# Start simulator_enclave in background
echo "Starting simulator_enclave..."
$SIMULATOR_ENCLAVE_BIN \
    "${PARSER_OUTER_SOCKET_PATH}" \
    "${PARSER_INNER_SOCKET_PATH}" &
ENCLAVE_PID=$!

# Wait for simulator_enclave socket to be created
wait_for_socket "${PARSER_OUTER_SOCKET_PATH}" "simulator_enclave"

# Start parser_host in background
echo "Starting parser_host on ${PARSER_HOST}:${PARSER_PORT}..."
$PARSER_HOST_BIN \
    --host-ip "${PARSER_HOST}" \
    --host-port "${PARSER_PORT}" \
    --metrics \
    --metrics-port "${PARSER_METRICS_PORT}" \
    --usock "${PARSER_OUTER_SOCKET_PATH}" &
PARSER_HOST_PID=$!

# Wait a moment for parser_host to start
sleep 2

# Start grpc-gateway if binary exists
if [ -x "${GATEWAY_BIN}" ]; then
    echo "Starting grpc-gateway on port ${GATEWAY_PORT}..."
    $GATEWAY_BIN \
        --grpc-server-endpoint "localhost:${PARSER_PORT}" \
        --http-port "${GATEWAY_PORT}" &
    GATEWAY_PID=$!
    echo "Gateway ready at http://localhost:${GATEWAY_PORT}"
fi

echo "Parser is ready to accept requests"
echo "  gRPC: ${PARSER_HOST}:${PARSER_PORT}"
echo "  REST: http://localhost:${GATEWAY_PORT}/visualsign/api/v1/parse"

# Wait for parser_host process
wait $PARSER_HOST_PID
