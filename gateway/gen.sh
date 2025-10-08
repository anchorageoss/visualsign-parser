#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROTO_DIR="${SCRIPT_DIR}/../proto"
GEN_DIR="${SCRIPT_DIR}/gen"

# Clean and create gen directory
rm -rf "${GEN_DIR}"
mkdir -p "${GEN_DIR}"

# Find protoc include directory
PROTOC_INCLUDE=$(protoc --version > /dev/null 2>&1 && dirname $(which protoc))/../include || echo "/usr/include"

# Generate Go code with grpc-gateway
protoc -I "${PROTO_DIR}" \
  -I "${PROTO_DIR}/vendor" \
  -I "${PROTOC_INCLUDE}" \
  --go_out="${GEN_DIR}" \
  --go_opt=paths=source_relative \
  --go-grpc_out="${GEN_DIR}" \
  --go-grpc_opt=paths=source_relative \
  --grpc-gateway_out="${GEN_DIR}" \
  --grpc-gateway_opt=paths=source_relative \
  --grpc-gateway_opt=generate_unbound_methods=true \
  "${PROTO_DIR}/parser/parser.proto" \
  "${PROTO_DIR}/health/rpc.proto"

echo "Proto generation complete in ${GEN_DIR}"
