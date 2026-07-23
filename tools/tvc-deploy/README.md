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

   `tvc-deploy list-invitations --org <alias>` shows every invitation in the
   org with its status (created/accepted/revoked) -- useful for checking
   whether someone's accepted yet, since Turnkey doesn't email you that.

   `tvc-deploy list-activities --org <alias>` lists activities newest-first,
   with `--status`/`--activity-type` filters and `--limit`. Its default
   (non-`--json`) output is a table (id/type/status/created_at/summary) that
   wraps to your terminal width, and decodes each activity's `intent` into a
   short summary -- e.g. `updates tag 'releaser' (adds <user's display
   name>)` instead of just `ACTIVITY_TYPE_UPDATE_USER_TAG` -- resolving
   tag/user ids to their display names, so you don't have to cross-reference
   `list-tags`/`list-users` by hand. Add `--json` to dump the full raw activity (intent, votes,
   fingerprint) instead -- handy for comparing two activities that look like
   duplicates, since the dashboard doesn't make that easy either. A
   duplicate is usually the same intent submitted twice (e.g. a deploy
   retried before the first one finished consensus) -- the fingerprints will
   differ because they include the submission timestamp, but the `intent`
   payloads will be identical.

   `tvc-deploy view-activity --activity-id <id> --org <alias>` shows the
   same decoded summary for one activity, plus its full status/fingerprint
   and who's voted so far -- useful for digging into a specific activity
   flagged by `list-activities` without the surrounding noise of a full
   list.

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

   Before sending, it fetches the org's current members *and* pending
   invitations and skips any invitee whose **canonical** email (lowercased,
   `+suffix` stripped) matches either -- so `alice+dev1@co.com` is treated as
   the same person as `alice@co.com`, and inviting the same email twice
   before they accept is caught and skipped too. To bypass this for
   specific invitees (e.g. you deliberately want a `+dev` test account for
   someone who's already a real member), set `"allowExisting": true` on that
   invitee (or at the file's top level, for the whole batch) -- this lives in
   the file you already curated, not as a repeated command-line flag:

   ```json
   {"userName": "Alice", "email": "alice+dev1@co.com", "allowExisting": true}
   ```

   Or disable the check entirely with `--include-existing`.

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
via a `--vars` file before submission. `--dry-run` renders and prints
without submitting, useful for checking a template against a new org's tags
before it actually creates anything:

```
tvc-deploy list-tags --org prod                                  # look up prod tag ids
tvc-deploy create-policies --file templates/releaser-initiator-policies.json \
  --vars prod-vars.json --dry-run --org prod                     # verify rendering
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

## Deploying

`tvc-deploy deploy` refuses to run if the target `--app-id` already has a
`create_tvc_deployment` activity awaiting consensus, since Turnkey has no
dedup for this -- submitting the same deploy twice (e.g. a retry before the
first finished consensus) creates a second, independent activity rather than
reusing the pending one:

```
error: app <app-id> already has 1 deployment activity(ies) awaiting consensus: <activity-id>
approve or reject the existing one first (tvc-deploy approve-activity / reject-activity --activity-id <id>), or pass --force to submit anyway
```

Resolve the existing activity (approve or reject it) and re-run, or pass
`--force` to submit anyway. The check uses the active org by default; pass
`--org <alias>` on `deploy` if the deployment's org differs from it.

## Pruning deployments

Deployments accumulate: every `deploy` creates a new one, and old ones are not
cleaned up automatically. Two subcommands remove them. The delete itself goes
straight to the Turnkey API (a delete carries no manifest, so unlike `deploy` it
doesn't shell out to `tvc` to submit it); `prune` additionally shells out to
`tvc app status` to enumerate an app's deployments before deleting. Each surfaces
the activity id + the `approve-activity` follow-up when the org needs consensus,
exactly like invite/policy.

`delete-deployment` is the primitive, one deployment by id:

```
tvc-deploy delete-deployment --deploy-id <id> --org <alias>
```

`prune` is the convenience wrapper: keep the live deployment plus the `--keep`
newest (default 2), delete the rest.

```
# eyeball the plan first, nothing is deleted
tvc-deploy prune --app-id <app-id> --keep 2 --org <alias> --dry-run

# then for real (prompts before deleting; --yes bypasses for automation)
tvc-deploy prune --app-id <app-id> --keep 2 --org <alias>
```

`prune` lists deployments from `tvc app status`, orders them by the `created_at`
of their `create_tvc_deployment` activity (newest first), and prints a
`KEEP`/`DELETE` plan before doing anything:

```
prune plan for app <app-id> (keep newest 2 + live):
  KEEP   deploy-d  created_at=1737000400 (live)
  KEEP   deploy-c  created_at=1737000300
  DELETE deploy-b  created_at=1737000200
  DELETE deploy-a  created_at=1737000100
```

Guards:

- The live deployment (the `Targeted Deployment:` in `tvc app status`) is never
  deleted. Turnkey also refuses this server-side; to remove the live one, set a
  different deployment live first, or delete the whole app.
- `--keep` must be `>= 1`.
- A deployment with no matching `create_tvc_deployment` activity (so it can't be
  dated) is protected and flagged, not deleted.

Consensus works the same as everywhere else: `prune`/`delete-deployment` only
surface the activity id, and a quorum member runs `tvc-deploy approve-activity
--activity-id <id> --org <alias>` authenticated as themselves.
