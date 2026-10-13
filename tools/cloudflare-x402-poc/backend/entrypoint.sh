#!/bin/sh
# Non-attested dev backend: gRPC parser on 127.0.0.1:44020, HTTP gateway in front.
parser_grpc_server --accept-unsigned-abis &
exec parser_gateway
