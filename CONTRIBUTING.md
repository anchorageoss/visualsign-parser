# Contributing

For complete contribution guidelines, including PR workflow, code standards, and build instructions, see our documentation:

**[Contributing Overview](https://visualsign.dev/contributing)**

## Quick links

- [Contributing a Visualization](https://visualsign.dev/contributor-guides/contributing-visualization) — For DApp and protocol developers
- [Adding a New Chain](https://visualsign.dev/adding-new-chain) — For blockchain developers
- [Best Practices](https://visualsign.dev/contributor-guides/best-practices) — Code standards and testing guidelines

## Questions?

Reach out through the [issue tracker](https://github.com/anchorageoss/visualsign-parser/issues).

## Agent setup (Claude Code & pi)

Shared agent configuration lives in tracked files and applies to both Claude Code and pi:

- `CLAUDE.md` (also symlinked as `AGENTS.md`) — project instructions both Claude Code and pi load at startup.
- `.claude/` — Claude Code tracked config: shared `settings.json`, hooks (`hooks/format-on-edit.sh` runs `rustfmt` after edits), and skills.
- `.pi/extensions/` — pi extensions: `protect-build-artifacts.ts` (blocks edits to generated paths) and `format-on-edit.ts` (mirrors the Claude rustfmt hook for pi).

Per-user scratch (`.claude/settings.local.json`, `.claude/worktrees/`, `.pi/mega-compact/`, etc.) is gitignored and never committed. To trust pi's project-local extensions for the first time, run `/trust`.
