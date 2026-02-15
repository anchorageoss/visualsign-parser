package main

import (
	"bytes"
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"

	"github.com/grpc-ecosystem/grpc-gateway/v2/runtime"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"

	gw "github.com/anchorageoss/visualsign-parser/gateway/gen/parser"
)

var (
	grpcServerEndpoint = flag.String("grpc-server-endpoint", "localhost:44020", "gRPC server endpoint")
	httpPort           = flag.String("http-port", "8080", "HTTP server port")
)

// turnkeyMiddleware unwraps Turnkey request format and wraps response
func turnkeyMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		// Read the body
		body, err := io.ReadAll(r.Body)
		if err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}
		r.Body.Close()

		// Try to parse as Turnkey format (with "request" field)
		var turnkeyReq map[string]interface{}
		needsWrapping := false
		if err := json.Unmarshal(body, &turnkeyReq); err == nil {
			if requestField, ok := turnkeyReq["request"]; ok {
				// Unwrap the request
				unwrapped, err := json.Marshal(requestField)
				if err != nil {
					http.Error(w, err.Error(), http.StatusInternalServerError)
					return
				}
				body = unwrapped
				needsWrapping = true
			}
		}

		r.Body = io.NopCloser(bytes.NewReader(body))

		if needsWrapping {
			// Wrap the response writer to wrap the response
			wrappedWriter := &responseWrapper{
				ResponseWriter: w,
				statusCode:     http.StatusOK,
			}

			next.ServeHTTP(wrappedWriter, r)
			wrappedWriter.finalize()
		} else {
			// Pass through as-is
			next.ServeHTTP(w, r)
		}
	})
}

type responseWrapper struct {
	http.ResponseWriter
	statusCode int
	body       bytes.Buffer
}

func (rw *responseWrapper) WriteHeader(code int) {
	rw.statusCode = code
	// Don't write header yet, wait for body
}

func (rw *responseWrapper) Write(b []byte) (int, error) {
	return rw.body.Write(b)
}

func (rw *responseWrapper) finalize() {
	// Wrap the response
	var response map[string]interface{}
	if err := json.Unmarshal(rw.body.Bytes(), &response); err == nil {
		wrapped := map[string]interface{}{
			"response": response,
		}
		wrappedBytes, _ := json.Marshal(wrapped)
		rw.ResponseWriter.WriteHeader(rw.statusCode)
		rw.ResponseWriter.Write(wrappedBytes)
	} else {
		// If not JSON, pass through
		rw.ResponseWriter.WriteHeader(rw.statusCode)
		rw.ResponseWriter.Write(rw.body.Bytes())
	}
}

func run() error {
	ctx := context.Background()
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	mux := runtime.NewServeMux()
	opts := []grpc.DialOption{grpc.WithTransportCredentials(insecure.NewCredentials())}
	err := gw.RegisterParserServiceHandlerFromEndpoint(ctx, mux, *grpcServerEndpoint, opts)
	if err != nil {
		return err
	}

	// Wrap with Turnkey middleware
	handler := turnkeyMiddleware(mux)

	addr := fmt.Sprintf(":%s", *httpPort)
	log.Printf("Starting HTTP server on %s, proxying to gRPC server at %s", addr, *grpcServerEndpoint)
	return http.ListenAndServe(addr, handler)
}

func main() {
	flag.Parse()

	if err := run(); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
}
