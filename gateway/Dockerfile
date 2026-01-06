FROM golang:1.21-alpine AS builder

# Install protoc and protoc plugins
RUN apk add --no-cache protobuf-dev git

# Install Go protoc plugins
RUN go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.31.0 && \
    go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.3.0 && \
    go install github.com/grpc-ecosystem/grpc-gateway/v2/protoc-gen-grpc-gateway@v2.18.1

WORKDIR /build

# Copy proto files and gateway code
COPY proto /build/proto
COPY gateway /build/gateway

# Generate Go code from protos
WORKDIR /build/gateway
RUN ./gen.sh

# Download Go dependencies
RUN go mod download

# Build the gateway binary
RUN CGO_ENABLED=0 GOOS=linux go build -o /gateway main.go

# Final stage
FROM alpine:latest
RUN apk --no-cache add ca-certificates
WORKDIR /root/

COPY --from=builder /gateway .

EXPOSE 8080

ENTRYPOINT ["./gateway"]
