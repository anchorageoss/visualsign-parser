# gRPC-Gateway for VisualSign Parser

This directory contains a gRPC-gateway implementation that exposes the Parser gRPC service as a REST API.

## Endpoints

- `POST /visualsign/api/v1/parse` - Parse unsigned transaction payloads

## Request Format

```bash
curl -X POST http://localhost:8080/visualsign/api/v1/parse \
  -H "Content-Type: application/json" \
  -d '{
    "request": {
      "unsigned_payload": "AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACAAQAIC016v0+XIik022SYSjfhzXdkN/4vJgwcW8aXpN5K2euuhpO9ls3hvuKK/xDxyXlsN5w2oJfNNLVeayaHlvZRVmeJNu1fQBgblAqJIMLXSbRxozp+uYRW6SmMRY6TsX/Q7AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwZGb+UhFzL/7K26csOb57yM5bvF9xJrLEObOkAAAAAEedVb8jHAbu50xW7OaBUH/bGy3qP0jlECsc2iVrwTjwbd9uHXZaGT2cvhRs7reawctIXtX1s3kTqM9YV+/wCpjJclj04kifG7PRApFI4NgwtaE5na/xCEBI572Nvp+Fm0P/on9df2SnTAmx8pWHneSwmrNt/J3VFLMhqns4zl6Mb6evO+2606PWXzaqvJdDGxu+TC0vbg5HymAgNFL11h8VaSsjI6qPaf8zTQcj7i/1CTIHnWI0ILbfgArfIP1bqf97H/URFIcr9HfiBhYzkPoK2byhlqUfrB0W80icm52QYEAAkDY5UXAAAAAAAHBgACAA4DBgEBAwIAAgwCAAAAQEIPAAAAAAAGAQIBEQUTBgACAQUJBQgFEAAMAgELDQoPBiXlF8uXeuOtKgEAAABWAP9kAAFAQg8AAAAAAACLAwAAAAAAMgAABgMCAAABCQGzKzGQ8mM7q6/oQp+WI7675rEffxHX3YWFCEp+Tg80bAPMy88DA8nO",
      "chain": "CHAIN_SOLANA"
    },
    "organization_id": "dummy-org-id"
  }'
```

## Development

Generate protobuf code:
```bash
go generate ./...
```

Run locally:
```bash
go run . --grpc-server-endpoint localhost:44020 --http-port 8080
```

## Docker

The gateway is automatically included in the `parser_unified` Docker container and starts on port 8080.
