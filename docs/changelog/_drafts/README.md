# Changelog drafts

PR authors of user-facing changes drop a fragment here. A maintainer batches drafts into one or more `<Update>` blocks on `../changelog.mdx` when a meaningful release ships, then deletes the consumed drafts.

This directory is intentionally not listed in `docs.json` navigation, so Mintlify does not publish it.

## Fragment format

Name the file `YYYY-MM-DD-<pr-slug>.md`. Include YAML frontmatter and a short prose body.

Example (`2026-05-05-metadao-futarchy.md`):

```markdown
---
category: chain-parsers           # one of: chain-parsers | fundamentals
label: "Added MetaDAO Futarchy preset"
description: ""                   # filled in at curation time with the release version (e.g. "v0.646.0")
tags: ["Solana", "Wallet API"]
---
Solana governance transactions now decode conditional vault and market
interactions. See [Solana presets](/chains/solana).
```

The maintainer reads the frontmatter into the corresponding props on `<Update>`, lifts the body into the block, and writes it to whichever of `../chain-parsers.mdx` or `../fundamentals.mdx` matches `category`. `description` is filled in with the first release tag that contains the change — look up via `git tag --contains <sha> --sort=v:refname | head -1`.

## Category choice

- **`chain-parsers`** — anything under `src/chain_parsers/`: new chain, new preset, IDL handling, decoder fix, fixture coverage.
- **`fundamentals`** — everything else: core types in `src/visualsign`, field builders, the parser binaries (`src/parser/{cli,app,grpc-server}`), the HTTP gateway, attestation, policy, codegen, integration tests, build infrastructure.

If a change spans both (rare), drop two fragments.

## Controlled tag vocabulary

Use tags from the shared vocabulary so subscribers can filter:

- **Audience** — `Wallet API`, `Contributors`, `Security`, `Ecosystem`.
- **Chain** — `Ethereum`, `Solana`, `Sui`, `Tron`.
- **Theme** — `Policy`, `Architecture`, `Lints`, `Attestation`, `Performance`.

A single fragment usually has one audience tag, one chain tag (if applicable), and zero or one theme tag.

## What counts as user-facing

Drop a draft when the change is one of:

- New chain or preset coverage.
- A change to VisualSign payload shape, field names, or determinism guarantees.
- A new or changed CLI, gRPC, or HTTP surface.
- A security-relevant change (TEE, attestation, policy).
- A breaking change to wallet integration.

Skip drafts for internal refactors, test changes, dependency bumps, and CI plumbing.

## Why this workflow

Authoring the entry close to the change keeps facts accurate. Curating at release time keeps the published changelog readable. Maintainers do not have to mine `git log` for context that has already faded.
