#!/bin/sh
# Non-attested dev backend: gRPC parser on 127.0.0.1:44020, HTTP gateway in front.
# PAYMENT_MARKER_PUBKEY (SEC1 uncompressed P-256 hex) turns on proof-of-payment
# enforcement: every parse then needs a marker signed by that key.
if [ -n "$PAYMENT_MARKER_PUBKEY" ]; then
  parser_grpc_server --accept-unsigned-abis --require-payment-marker-pubkey "$PAYMENT_MARKER_PUBKEY" &
else
  parser_grpc_server --accept-unsigned-abis &
fi
exec parser_gateway
