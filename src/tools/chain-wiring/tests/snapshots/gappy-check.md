
**Crate**

- [x] src/chain_parsers/visualsign-gappy exists
- [x] crate default = ["cli-plugin"]
- [x] crate [lints] workspace = true
- [x] workspace member

**Proto and core enum**

- [x] proto CHAIN_GAPPY slot
- [x] generated code regenerated
- [x] registry Chain::Gappy
- [x] registry FromStr "gappy"
- [x] chain_conversion proto->registry

**parser_app / http-server**

- [x] parser_app feature
- [x] parser_app registers converter
- [ ] parser_app registry test (src/parser/app/src/registry.rs)
- [x] http-server feature

**Default set: parser_app default == http-server default == every CHAIN_FEATURES**

- gappy is DEFAULT-ON in parser_app (state this in the PR)
- [x] http-server default matches parser_app default
- [x] images/parser_app/Containerfile CHAIN_FEATURES matches parser_app default
- [x] images/parser_http_server/Containerfile CHAIN_FEATURES matches parser_app default
- [ ] .oss-scanner/Dockerfile CHAIN_FEATURES differs from parser_app default [gappy ] vs [ ]

**parser_cli**

- [x] parser_cli feature
- [x] parser_cli main.rs plugin
- [x] cli-core chains.rs mapping
- [x] Makefile PARSER_CLI_CHAINS
- [x] cli_test.rs feature list
- [x] cli_test.rs disabled prefix
- [ ] parser_cli fixture (src/parser/cli/tests/fixtures/gappy-*.input)

**Repo metadata and docs**

- [x] labeler chain:gappy
- [x] CODEOWNERS entry
- [ ] docs/chains/gappy.mdx (docs/chains/gappy.mdx)
- [x] docs.json nav
- [x] chain-modules.mdx
- [x] introduction.mdx
- [x] parser-cli.mdx --chain list
- [x] README chain list
- [ ] changelog entry (docs/changelog/_drafts/<date>-gappy-*.md)
- [x] CLAUDE.md crate list
- no APPLE_NO_C_CRATES in src/Makefile yet; check C deps by hand (see SKILL.md)

29/34 touch points present, 5 missing
