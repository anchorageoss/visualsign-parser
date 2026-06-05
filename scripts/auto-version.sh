#!/usr/bin/env bash
set -euo pipefail

# Resolve the raw branch name before sanitization so comparisons against
# "main"/"master" can't be fooled by branches like "main." that collapse
# to "main-" after character sanitization.
#
# Precedence:
#   1. GITHUB_HEAD_REF -- set on pull_request events
#   2. GITHUB_REF_NAME -- set on push/workflow_dispatch events
#   3. git symbolic-ref --short HEAD -- local developer runs
RAW_BRANCH="${GITHUB_HEAD_REF:-}"
if [ -z "$RAW_BRANCH" ]; then
  RAW_BRANCH="${GITHUB_REF_NAME:-}"
fi
if [ -z "$RAW_BRANCH" ]; then
  if git symbolic-ref --short HEAD > /dev/null 2>&1; then
    RAW_BRANCH="$(git symbolic-ref --short HEAD)"
  fi
fi

SHORT_HASH=$(git rev-parse --short=12 HEAD)

# A shallow clone makes the commit-distance counts below wrong (git only sees
# the truncated history). On CI, deepen to full history and fail if we can't;
# locally, warn loudly so a bogus version is obvious rather than silent.
if [ "$(git rev-parse --is-shallow-repository 2>/dev/null)" = "true" ]; then
  if [ "${GITHUB_ACTIONS:-}" = "true" ]; then
    echo "Repository is shallow; fetching full history for an accurate version..." >&2
    git fetch --unshallow 2>/dev/null || git fetch --depth=2147483647 2>/dev/null || true
    if [ "$(git rev-parse --is-shallow-repository 2>/dev/null)" = "true" ]; then
      echo "ERROR: repository is still shallow after fetch; refusing to compute a version from truncated history." >&2
      exit 1
    fi
  else
    echo "WARNING: shallow clone detected -- the computed version will be wrong." >&2
    echo "         Run 'git fetch --unshallow' for an accurate commit distance." >&2
  fi
fi

# Sanitized branch for semver build metadata (allowed: [0-9A-Za-z-]).
# Trailing "-" is a separator before SHORT_HASH; omitted when branch is empty.
if [ -n "$RAW_BRANCH" ]; then
  # shellcheck disable=SC2001
  BRANCH_META="$(echo "$RAW_BRANCH" | sed 's/[^a-zA-Z0-9-]/-/g')-"
else
  BRANCH_META=
fi

# Treat the build as a default-branch build only when we know the ref is
# actually the canonical default branch. `pull_request` events always
# represent a non-default ref by definition, so even a fork PR opened from
# a branch literally named `main`/`master` falls through to the merge-base
# path below.
if { [ "$RAW_BRANCH" = "main" ] || [ "$RAW_BRANCH" = "master" ]; } \
   && [ "${GITHUB_EVENT_NAME:-}" != "pull_request" ]; then
  HEIGHT=$(git rev-list --count HEAD)
  echo "0.$HEIGHT.0+${BRANCH_META}$SHORT_HASH"
  exit 0
fi

# Find the remote that points at the canonical upstream repo. Prefer
# $GITHUB_REPOSITORY (set in Actions) so forks/renames work without editing this
# script; fall back to the known upstream slug, then to "origin". Override with
# AUTO_VERSION_REMOTE if your remote layout differs.
EXPECTED_REPO="${AUTO_VERSION_REPO:-${GITHUB_REPOSITORY:-anchorageoss/visualsign-parser}}"
REMOTE="${AUTO_VERSION_REMOTE:-}"
if [ -z "$REMOTE" ]; then
  REMOTE=$(git remote -v | awk -v repo="$EXPECTED_REPO" '$0 ~ "[[:space:]]\\(fetch\\)" && index($0, repo) {print $1; exit}')
fi
if [ -z "$REMOTE" ]; then
  REMOTE="origin"
fi

# Resolve the default branch. Prefer main, then master. If neither
# remote-tracking ref is present (e.g. a CI checkout that fetched only the
# current ref), try to fetch both before giving up, so the merge-base below
# doesn't fail cryptically under `set -e`.
if git rev-parse --verify "$REMOTE/main" > /dev/null 2>&1; then
  DEFAULT_BRANCH="main"
elif git rev-parse --verify "$REMOTE/master" > /dev/null 2>&1; then
  DEFAULT_BRANCH="master"
else
  git fetch "$REMOTE" main > /dev/null 2>&1 || true
  git fetch "$REMOTE" master > /dev/null 2>&1 || true
  if git rev-parse --verify "$REMOTE/main" > /dev/null 2>&1; then
    DEFAULT_BRANCH="main"
  elif git rev-parse --verify "$REMOTE/master" > /dev/null 2>&1; then
    DEFAULT_BRANCH="master"
  else
    echo "ERROR: cannot resolve $REMOTE/main or $REMOTE/master to compute a merge base." >&2
    exit 1
  fi
fi

MERGE_BASE=$(git merge-base "$REMOTE/$DEFAULT_BRANCH" HEAD)
if [ "$MERGE_BASE" = "$(git rev-parse "$REMOTE/$DEFAULT_BRANCH")" ] && [ "${GITHUB_ACTIONS:-}" = "true" ]; then
  # On CI, the remote-tracking ref may be stale (shallow clone) -- fetch to get the real merge base.
  # Skipped on local builds; run `git fetch` manually if the version number looks wrong.
  echo "Fetching $REMOTE..." >&2
  git fetch "$REMOTE" >&2
  MERGE_BASE=$(git merge-base "$REMOTE/$DEFAULT_BRANCH" HEAD)
fi
MERGE_HEIGHT=$(git rev-list --count "$MERGE_BASE")
HEIGHT=$(git rev-list --count HEAD)
MERGE_DIFF=$((HEIGHT - MERGE_HEIGHT))
echo "0.$MERGE_HEIGHT.$MERGE_DIFF+${BRANCH_META}$SHORT_HASH"
