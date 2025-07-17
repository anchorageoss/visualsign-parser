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

You can also manually exercise health checks. This is what Kubernetes will use to gauge whether the host is healthy or not:

```
grpcurl -plaintext -d '{"service":""}' localhost:44020 grpc.health.v1.Health/Check
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

## Manual Testing Notes

## Example Solana tx

### CLI

```
cargo run --bin parser_cli -- --chain solana -t 'AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAGDpVgWUMU7MEPPORo0ORMinVaO1ktDjHe3//f1qqIwJ2XYaz02Vuj7xyKHc5e6LXN5WxDxzUGN72irt3XVidnPQdbX1g0C8G9eZLm2AYo6hVEwP0bql0mb8fZLQW6g3h/XIjx/6Oi3+YXvcTjVzJRoyLj/K6B5aRXOQ5kdRwApGXinqdo/t9kTIqum44hiK3Qa8VQ+/cWyCK5zmPHeD2VLh8J5qP+7PmQMuHB32uXItyzY057jjRAk2vDSwzByOtSH/zRQemDLK8QrZF0lcoPJxtbKTzUcCfqc3AH7UDrOaC9BIo+CMO0lb4X9FQn2JvsW4DH4mlcGGTXZ0PbOb7TRtYAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAFTlniTAUaihSnsel5fw4Szfp0kCFmUxlaalEqbxZmArjJclj04kifG7PRApFI4NgwtaE5na/xCEBI572Nvp+FkDBkZv5SEXMv/srbpyw5vnvIzlu8X3EmssQ5s6QAAAAAaBTtTK9ooXRnL9rIYDGmPoTqFe+h1EtyKT9tvbABZQBt324ddloZPZy+FGzut5rBy0he1fWzeROoz1hX7/AKk7YUJFOy/o1K3RALVqqztUypoKMpR8OCcCt0Rr0FUhSAYIAgABDAIAAAAA5AtUAgAAAAoGAAIABggNAQEMCgcJBAECBQIGCA0JDgDkC1QCAAAACwAFAoAaBgALAAkDUMMAAAAAAAAIAgADDAIAAAAQJwAAAAAAAA=='
```

### gRPC

```
grpcurl -plaintext -d '{"unsigned_payload": "AgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAGDpVgWUMU7MEPPORo0ORMinVaO1ktDjHe3//f1qqIwJ2XYaz02Vuj7xyKHc5e6LXN5WxDxzUGN72irt3XVidnPQdbX1g0C8G9eZLm2AYo6hVEwP0bql0mb8fZLQW6g3h/XIjx/6Oi3+YXvcTjVzJRoyLj/K6B5aRXOQ5kdRwApGXinqdo/t9kTIqum44hiK3Qa8VQ+/cWyCK5zmPHeD2VLh8J5qP+7PmQMuHB32uXItyzY057jjRAk2vDSwzByOtSH/zRQemDLK8QrZF0lcoPJxtbKTzUcCfqc3AH7UDrOaC9BIo+CMO0lb4X9FQn2JvsW4DH4mlcGGTXZ0PbOb7TRtYAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAFTlniTAUaihSnsel5fw4Szfp0kCFmUxlaalEqbxZmArjJclj04kifG7PRApFI4NgwtaE5na/xCEBI572Nvp+FkDBkZv5SEXMv/srbpyw5vnvIzlu8X3EmssQ5s6QAAAAAaBTtTK9ooXRnL9rIYDGmPoTqFe+h1EtyKT9tvbABZQBt324ddloZPZy+FGzut5rBy0he1fWzeROoz1hX7/AKk7YUJFOy/o1K3RALVqqztUypoKMpR8OCcCt0Rr0FUhSAYIAgABDAIAAAAA5AtUAgAAAAoGAAIABggNAQEMCgcJBAECBQIGCA0JDgDkC1QCAAAACwAFAoAaBgALAAkDUMMAAAAAAAAIAgADDAIAAAAQJwAAAAAAAA==", "chain":"CHAIN_SOLANA"}'  localhost:44020 parser.ParserService/Parse
```
