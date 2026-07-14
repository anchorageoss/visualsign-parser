# tvc-deploy

Standalone deploy + Turnkey org-management helper for `parser_app`. See `src/main.rs`
for the full subcommand list (`--help` prints it too).

## Inviting a batch of users

To invite a whole team in one activity (and therefore one consensus approval, if
the org requires it):

1. **Authenticate.** Either run `tvc login` once (writes
   `~/.config/turnkey/tvc.config.toml`), or set
   `TVC_ORG_ID` / `TVC_API_KEY_PUBLIC` / `TVC_API_KEY_PRIVATE` (all three, or
   none) in the environment. `invite` picks whichever is available; use
   `--org <alias>` to pick a specific org from the config file instead of the
   active one.

2. **(Optional) Look up user-tag ids**, if you want to assign tags on invite.
   Tags are referenced by **UUID, not name**:

   ```
   tvc-deploy list-tags --org <alias>
   ```

3. **Write an `invitees.json`** listing everyone to invite:

   ```json
   {
     "accessType": "all",
     "invitees": [
       {"userName": "Alice", "email": "alice@co.com", "tags": ["<tag-uuid>"]},
       {"userName": "Bob", "email": "bob@co.com", "accessType": "web"}
     ]
   }
   ```

   - `accessType` is one of `web` | `api` | `all`. The top-level value is the
     default for every invitee; a per-invitee `accessType` overrides it.
   - `tags` is optional per invitee (defaults to none).

4. **Send the batch:**

   ```
   tvc-deploy invite --file invitees.json --org <alias>
   ```

   This resolves your own user id via `whoami` (used as `senderUserId`) unless
   `--sender-user-id <id>` is given, then submits all invitees as a single
   `create_invitations` activity.

5. **Approve, if needed.** If the org's policies require consensus, the command
   prints the activity id and the exact follow-up command instead of erroring:

   ```
   tvc-deploy approve-activity --activity-id <id> --org <alias>
   ```

   Approval must come from a quorum member — run it authenticated as that
   person (their own `tvc login` / env vars), not necessarily the same
   credentials that sent the invite.

A single invite (no file) works the same way with flags instead of a file:

```
tvc-deploy invite --user-name Alice --email alice@co.com --tags <tag-uuid> --org <alias>
```

## Batch-creating policies from a template

See `templates/releaser-initiator-policies.json` for an example: a template
with `{{PLACEHOLDER}}` tokens for tag ids that differ per environment, rendered
via a `--vars` file before submission. `--dry-run true` renders and prints
without submitting, useful for checking a template against a new org's tags
before it actually creates anything:

```
tvc-deploy list-tags --org prod                                  # look up prod tag ids
tvc-deploy create-policies --file templates/releaser-initiator-policies.json \
  --vars prod-vars.json --dry-run true --org prod                # verify rendering
tvc-deploy create-policies --file templates/releaser-initiator-policies.json \
  --vars prod-vars.json --org prod                                # actually create
```

### Basic / read-only access

`accessType` on an invite only controls *how* a user authenticates (web
dashboard vs. API key) -- it is not a permission level, and there is no
built-in "basic" tier. Permission scoping is entirely policy-driven, so a
read-only user is one who's been tagged and given an explicit deny.

`templates/readonly-policy.json` is a single-policy template for exactly
that: `EFFECT_DENY` on every activity type (`condition: "true"`), scoped via
`consensus` to only apply to whoever holds the given tag. Because
`EFFECT_DENY` always wins over any conflicting `EFFECT_ALLOW`, this acts as a
hard guardrail even if the tagged user later picks up other allow policies.
Queries (`list_*`/`get_*`) aren't gated by policies at all, so a read-only
user can still browse the org, wallets, policies, etc. -- this only blocks
activities that change state:

```
tvc-deploy create-policies --file templates/readonly-policy.json \
  --vars readonly-vars.json --org <alias>
```
