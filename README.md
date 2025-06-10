# visualsign-parser

This repo contains an enclave application to parse unsigned transactions and return VisualSign output.

## Running tests

```
make -C src test
```

## Running locally

The following will start 3 sub-processes: an enclave simulator, a host, and an inner app:

```
make -C src parser
```

The parser will expose a gRPC interface on port `44020` by default.

Once it's up and running, make requests with:

```
grpcurl -plaintext -d '{"unsigned_payload": "0xabcdef"}'  localhost:44020 parser.ParserService/Parse
```

If everything works you'll get a response like this:

```
{
  "parsedTransaction": {
    "payload": {
      "transactionMetadata": [
        ...
      ]
    }
  }
}
```

## Building parser OCI containers

This repository uses [StageX](https://stagex.tools) to build OCI containers. To build these locally, you'll need Docker > 26 and `containerd` for OCI compatibility:

- If you are using Docker Desktop, go to Dashboard > Settings > "Use containerd for pulling and storing images"
- If you are using a Linux-based system, add the following to `/etc/docker/daemon.json`:
  ```
  {
    "features": {
      "containerd-snapshotter": true
    },
    "registry-mirrors": ["https://ghcr.io/anchorageoss"]
  }
  ```

Then build the OCI containers with the `Makefile` targets:

```sh
# Builds the parser app container
make out/parser_app/index.json

# Builds the parser host container
make out/parser_host/index.json
```

Note: you can also build non-OCI versions with `make non-oci-docker-images`.
