# Wiring checklist

Do these in order; later steps fail to compile without earlier ones. Paths are from the
repo root. Snippets show the line to add for a chain `foo` (`Foo`, `CHAIN_FOO`); keep each
list in the order the file already uses. `cargo run -q -p chain-wiring -- check foo` (from
`src/`) verifies every step marked (c).

## Crate and workspace

1. (c) **Crate** `src/chain_parsers/visualsign-foo/Cargo.toml`
   ```toml
   [features]
   default    = ["cli-plugin"]
   cli-plugin = ["dep:clap", "dep:parser_cli_core", "dep:generated"]

   [lints]
   workspace = true
   ```
   `src/lib.rs`: `#[cfg(feature = "cli-plugin")] pub mod cli_plugin;` and
   `#[cfg(feature = "cli-plugin")] pub use cli_plugin::{FooArgs, FooPlugin};`.

2. (c) **Workspace** `src/Cargo.toml` `members`: `"chain_parsers/visualsign-foo",`

## Proto and core enum

3. (c) **Proto slot** `proto/parser/parser.proto`, `enum Chain`:
   ```proto
   CHAIN_FOO = 9;

   // Reserve space for future chains
   reserved 10 to 998;
   ```
   Slots 0-6 are on main. Open PRs claim more: #578 (Zcash) holds 7 and #579 (Cosmos) holds 8.
   Before picking, list the slots in flight:
   ```bash
   for n in $(gh pr list --repo anchorageoss/visualsign-parser --state open --limit 200 --json number,files \
       -q '.[] | select(any(.files[]; .path == "proto/parser/parser.proto")) | .number'); do
     echo "#$n"; gh pr diff "$n" --repo anchorageoss/visualsign-parser | grep -E '^\+ *CHAIN_'; done
   ```
   Take the next number nobody holds and name the PRs you checked in your PR body. Never
   renumber a slot that's on main. If another chain PR merges first, rebase: keep both
   enum lines, move `reserved` past the higher slot, and regenerate (step 4). Don't
   hand-merge `descriptor.bin`; take either side, then regenerate.

4. (c) **Codegen** `make -C src generated`, with protoc **21.4** (pinned in
   `.github/protoc.env`; install with `scripts/install-protoc.sh`). Check
   `protoc --version` first: output from another protoc version (e.g. 36.x) differs and CI's
   "working tree is clean" step fails. Commit `src/generated/src/generated/{parser.rs,descriptor.bin}`.

5. (c) **Registry enum** `src/visualsign/src/registry.rs`: the variant, its `as_str` arm,
   its `FromStr` arm, and both unit-test asserts.
   ```rust
   Foo,                                  // enum Chain
   Chain::Foo => "Foo",                  // as_str
   "foo" => Chain::Foo,                  // FromStr
   ```

6. (c) **Proto -> registry** `src/parser/app/src/chain_conversion.rs`: in
   `proto_to_registry` (no `_` arm, so a missing line fails to compile), in the test's
   `registry_to_proto`, and in the round-trip list.
   ```rust
   ProtoChain::Foo => RegistryChain::Foo,
   ```

## parser_app and http-server

7. (c) **parser_app feature** `src/parser/app/Cargo.toml`
   ```toml
   visualsign-foo = { path = "../../chain_parsers/visualsign-foo", optional = true }
   foo = ["dep:visualsign-foo"]
   # default-on only: add "foo" to `default`
   ```

8. (c) **parser_app registry + test** `src/parser/app/src/registry.rs`
   ```rust
   #[cfg(feature = "foo")]
   registry.register::<visualsign_foo::FooTransaction, _>(
       visualsign::registry::Chain::Foo,
       visualsign_foo::FooVisualSignConverter::default(),
   );
   ```
   and a test module that converts one pinned vector through the real registry:
   ```rust
   #[cfg(all(test, feature = "foo"))]
   mod foo_tests {
       #![allow(clippy::unwrap_used, clippy::expect_used)]
       use visualsign::registry::Chain;
       use visualsign::vsptrait::VisualSignOptions;

       #[test]
       fn registry_decodes_foo() {
           let registry = super::create_registry(&crate::config::ParserConfig::accept_unsigned());
           let result = registry
               .convert_transaction(&Chain::Foo, FOO_VECTOR, VisualSignOptions::default())
               .unwrap();
           assert_eq!(result.payload.payload_type, "FooTx");
       }
   }
   ```

9. (c) **http-server feature** `src/parser/http-server/Cargo.toml`
   ```toml
   foo = ["parser_app/foo"]
   # default-on only: add "foo" to `default`
   ```

10. (c) **Image feature sets**, default-on only. `CHAIN_FEATURES` in
    `images/parser_app/Containerfile`, `images/parser_http_server/Containerfile` and
    `.oss-scanner/Dockerfile` must equal parser_app's `default` (sorted):
    ```dockerfile
    ARG CHAIN_FEATURES="bitcoin ethereum foo near solana sui tron unspecified"
    ```
    `.oss-scanner/Dockerfile` is the one PRs forget (bitcoin is missing from it on main today).

## parser_cli

11. (c) **parser_cli feature** `src/parser/cli/Cargo.toml`
    ```toml
    foo = ["dep:visualsign-foo"]
    visualsign-foo = { path = "../../chain_parsers/visualsign-foo", optional = true }
    ```
    Add `"foo"` to parser_cli's `default` (the CLI isn't attested, so default-on is fine
    here even for an opt-in chain).

12. (c) **parser_cli plugin** `src/parser/cli/src/main.rs`, in `ChainArgs` and in the plugin list:
    ```rust
    #[cfg(feature = "foo")]
    #[command(flatten)]
    pub(crate) foo: visualsign_foo::FooArgs,
    // ...
    #[cfg(feature = "foo")]
    plugins.push(Box::new(visualsign_foo::FooPlugin::new(self.foo.clone())));
    ```
    `FooPlugin` implements `parser_cli_core::ChainPlugin` (`chain`, `register`,
    `create_metadata`); copy `visualsign-bitcoin/src/cli_plugin.rs`. Prefix every flag
    with the chain name (`--foo-network`); flags share one namespace.

13. (c) **CLI chain name** `src/parser/cli-core/src/chains.rs`, `chain_string_mapping()`:
    ```rust
    mapping.insert("foo", Chain::Foo);
    ```
    Without it, `available_chains()` and help text omit the chain.

14. (c) **Narrow CLI build** `src/Makefile`: add `foo` to `PARSER_CLI_CHAINS` (a literal
    list). `PARSER_APP_CHAINS` is derived from the `chain_parsers/visualsign-*` directories
    and needs no edit.

15. (c) **CLI end-to-end fixture** `src/parser/cli/tests/`:
    - `cli_test.rs`: add `feature = "foo"` to the `cfg_attr(not(all(...)))` list (only if
      `foo` is in parser_cli `default`), and to `disabled_chain_prefixes`:
      ```rust
      #[cfg(not(feature = "foo"))]
      "foo",
      ```
    - `fixtures/foo-json.input`: one CLI argument per line (`decode`, `--chain`, `foo`,
      `-o`, `json`, `-t`, `<vector>`), and `fixtures/foo-json.display.expected` with the
      expected JSON (fields compared by membership). Add `foo-json.diagnostics.expected`
      if the chain emits diagnostics.
    Without a fixture, `parser-cli-narrow-build-check` builds the chain and tests nothing.

## Repo metadata

16. (c) **Labeler** `.github/labeler.yml`
    ```yaml
    "chain:foo":
      - changed-files:
          - any-glob-to-any-file:
              - 'src/chain_parsers/visualsign-foo/**'
    ```
17. (c) **CODEOWNERS** `.github/CODEOWNERS`: `/src/chain_parsers/visualsign-foo/ @<owner> @prasanna-anchorage`.
    Owners need write access or GitHub ignores the line.
18. **Apple / iOS** `src/Makefile` `APPLE_NO_C_CRATES` (once PR #587 is merged): add
    `visualsign-foo` if C-free. Reported as INFO.

## Docs

19. (c) **Docs**
    - `docs/chains/foo.mdx`: input formats, what's shown, what's refused, trust labels.
    - `docs/docs.json`: `"chains/foo"` in the Chains group.
    - `docs/chain-modules.mdx` and `docs/introduction.mdx`: a `- **[Foo](./chains/foo)** - ...` bullet.
    - `docs/parser-cli.mdx`: add `` `foo` `` to the `--chain` row.
    - `README.md`: `- Foo (...)` under Supported Chains.
    - `docs/changelog/_drafts/<YYYY-MM-DD>-foo-<slug>.md`: format in `docs/changelog/_drafts/README.md`.
      Use `category: core` until the chain has its own stream; say so in a comment.
20. (c) **Agent docs** `CLAUDE.md` (AGENTS.md is a symlink): add `foo` to the
    `chain_parsers/visualsign-{...}` list under Workspace Layout.

## Downstream: visualsign-turnkeyclient

`chain-wiring` can't see this repo, so this step is by hand. It belongs to M6 on the tracking issue.

21. **[visualsign-turnkeyclient](https://github.com/anchorageoss/visualsign-turnkeyclient)**,
    the Go client callers use to parse through Turnkey and verify the attestation. It passes
    `--chain` / `VerifyRequest.Chain` through as a string, so a chain with no metadata and no
    intermediate output works without client changes (Bitcoin did, at v0.1046.0). Open a client
    PR when the parser PR adds either of these:
    - **A `<Variant>Metadata` arm on `ChainMetadata`.** Mirror it in `api/types.go`
      (`RequestChainMetadata` and its oneof-shaped `MarshalJSON`/`UnmarshalJSON`) and
      `api/borsh.go` (`BorshBytes`, which `metadataDigest` is checked against), and add the
      chain-must-match check in `verify/service.go` next to the Ethereum/Solana/NEAR ones.
      Add a Borsh cross-check vector produced by this repo's encoder.
    - **Intermediate output.** Add a decoder and a `CHAIN_FOO` case to the
      intermediate-output switch in `verify/service.go`, mirroring the schema version.

    Either way, update the `--chain` usage in `cmd/verify.go`, the README and
    `WALLET_INTEGRATION_GUIDE.md`, and add a `CHAIN_FOO` request test. Once the chain is
    deployed, smoke it end to end:
    `visualsign-turnkeyclient verify --dev-path --chain CHAIN_FOO --unsigned-payload <fixture>`
    must return `valid`, `attestationValid` and `signatureValid` all true. A `CHAIN_*` value that
    isn't in the proto yet also needs Turnkey's gateway rebuilt from the new proto first.
