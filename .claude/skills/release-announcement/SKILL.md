---
name: release-announcement
description: Draft a release announcement for visualsign-parser - changes grouped by chain, which chains the release enables (and which are new), integrator notes and contributor credit - using the release-notes tool (src/tools/release-notes). Use when asked to announce a release, write release notes, list changes per chain between two tags, or post a deploy update to a channel. Produces a public version safe for shared channels and an internal version; private deploy details are added at run time and never stored in the repo.
user-invocable: true
---

# Release announcement

`src/tools/release-notes` builds the announcement from git and public PR metadata. It
reads, it doesn't post. You review, edit, then post (or save as a draft) yourself.

## Run it

From `src/`, with `gh` logged in:

```bash
cargo run -q -p release-notes -- --repo-dir ..                       # newest tag vs the one before, public, Slack
cargo run -q -p release-notes -- --repo-dir .. --from v0.1033.0 --to v0.1046.0
cargo run -q -p release-notes -- --repo-dir .. --audience internal --format markdown
cargo run -q -p release-notes -- --repo-dir .. --format json          # data only, for your own templates
```

`--to` defaults to the newest tag at HEAD and `--from` to the tag before it, so run
`git fetch --tags` first.

What it produces:

- **Chains enabled**: chains whose feature is in `parser_app`'s `default` set and in the
  `parser_app` image's `CHAIN_FEATURES` at `--to`. A chain whose crate didn't exist at
  `--from` is marked **(new)**. Crates that exist but aren't in the default build are listed
  as **Opt-in**.
- **One section per chain**, new chains first: the PRs that touched that chain's crate. A
  PR that touches no chain crate goes under the chain in its conventional-commit scope
  (`fix(sui): ...`), then under **Core**. CI, build, test, docs and CLA PRs go under
  **Tooling/infra**, which only the internal audience sees.
- **Integrator notes**: PRs marked breaking or carrying an integrator-note section.
- **Contributors**: PR authors and `Co-authored-by` trailers, with bots and AI co-authors
  removed and emails resolved to GitHub logins.

## Audiences

| `--audience` | Contains | Where |
|---|---|---|
| `public` (default) | chains, core, integrator notes, thanks | shared channels with external collaborators, release pages |
| `internal` | public + tooling/infra + per-line co-authors | the team's own channel |

The public output contains only public GitHub links and handles.

## Private details (deploys)

The tool knows nothing about where a release is deployed, and nothing about it belongs in
this public repo. For an internal post, write the deploy block (environments, operation or
deployment IDs, smoke result, rollback target) to a file **outside the repo**, e.g. your
scratch directory, and pass it in:

```bash
cargo run -q -p release-notes -- --repo-dir .. --audience internal --footer-file /path/outside/repo/deploy.md
```

The file is appended verbatim. Take the values from your own deploy runbook and the deploy
you just ran. Never add them to a public announcement, and never commit the file.

## Make the output better at the source

The tool uses each PR's title, minus the `type(scope):` prefix, unless the PR body says
otherwise. To get a better line, a PR author can add:

- `## Release note`: the first paragraph replaces the title. Write it for a signer or
  integrator: what they now see or must do.
- `## Integrator note`: listed under Integrator notes. Use this for anything a caller must
  change, such as a new error or a changed input format.
- A `!` in the title (`fix(ethereum)!: ...`), a `breaking` label or a `BREAKING CHANGE`
  trailer marks the PR breaking even without a note.

When you edit the generated text by hand, keep the PR links and credits it produced.

## Before posting

1. Read the whole output. Fix titles that read as internal jargon. Merge or split lines if
   a PR spans chains oddly.
2. Check every enabled-chain claim that matters to the audience, ideally by parsing a
   fixture through the deployed service. "Enabled in the build" isn't the same as
   "reachable through a given gateway": a new `CHAIN_*` proto value also needs the gateway
   rebuilt from the new proto.
3. For the public version, confirm it has no internal hostnames, IDs, environment names or
   private channel names. The tool emits none, so anything like that came from an edit or
   a footer.
4. Prefer saving as a draft in the target channel so a human sends it.
