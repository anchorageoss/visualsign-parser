# Contributing

For complete contribution guidelines, including PR workflow, code standards, and build instructions, see our documentation:

**[Contributing Overview](https://visualsign.dev/contributing)**

## Quick links

- [Contributing a Visualization](https://visualsign.dev/contributor-guides/contributing-visualization) — For DApp and protocol developers
- [Adding a New Chain](https://visualsign.dev/adding-new-chain) — For blockchain developers
- [Best Practices](https://visualsign.dev/contributor-guides/best-practices) — Code standards and testing guidelines

## Questions?

Reach out through the [issue tracker](https://github.com/anchorageoss/visualsign-parser/issues).

## Changes that break wallet integration

Label a PR `breaking-wallet-integration` when integrators must upgrade before it
can deploy: it changes `proto/` (`parser.proto`, the `ChainMetadata` messages),
a chain's `*_INTERMEDIATE_SCHEMA_VERSION` or intermediate-output types, the bytes
hashed into `metadata_digest` or the signed payload, or a golden fixture that
pins those bytes. The "Wallet integration" check
(`tools/wallet-integration/src/rules.rs`) fails a PR that makes such a change
without the label. Use the label too for breaks the rules can't see. Name
golden-byte constants `*_GOLDEN` so the check sees changes to them.

A labeled PR needs this section in its description. The check parses it, and
`tvc-deploy deploy` prints it to the operator, who must acknowledge the PR by
number before it deploys:

```markdown
## Wallet integration

- requires: <integrator> >= <minimum version>
- deploy-order: <who ships first, and when the parser may deploy>
```

Add one `requires` line per integrator, plus at least one `deploy-order` line.
Other prose in the section is fine. Lines inside HTML comments and code blocks
are ignored, and `<placeholders>` are rejected. Run the check locally with:

```bash
cargo run --manifest-path tools/wallet-integration/Cargo.toml -- \
  check-pr --base origin/main --head HEAD --labeled true --body-file body.md
```

## Agent setup (Claude Code & pi)

Shared agent configuration lives in tracked files and applies to both Claude Code and pi:

- `CLAUDE.md` (also symlinked as `AGENTS.md`) — project instructions both Claude Code and pi load at startup.
- `.claude/` — Claude Code tracked config: shared `settings.json`, hooks (`hooks/format-on-edit.sh` runs `rustfmt` after edits), and skills.
- `.pi/extensions/` — pi extensions: `protect-build-artifacts.ts` (blocks edits to generated paths) and `format-on-edit.ts` (mirrors the Claude rustfmt hook for pi).

Per-user scratch (`.claude/settings.local.json`, `.claude/worktrees/`, `.pi/mega-compact/`, etc.) is gitignored and never committed. To trust pi's project-local extensions for the first time, run `/trust`.
