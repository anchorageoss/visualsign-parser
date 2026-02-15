package main

//go:generate sh -c "rm -rf gen && mkdir -p gen && protoc -I ../proto -I ../proto/vendor -I /usr/include -I /usr/local/include -I /include --go_out=gen --go_opt=paths=source_relative --go-grpc_out=gen --go-grpc_opt=paths=source_relative --grpc-gateway_out=gen --grpc-gateway_opt=paths=source_relative --grpc-gateway_opt=generate_unbound_methods=true ../proto/parser/parser.proto ../proto/health/rpc.proto"
