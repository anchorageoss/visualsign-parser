#!/usr/bin/env bash
# Repoint a TVC app at a freshly-built parser_http_server image.
#
# Reads the current targeted deploy ID from `tvc app status`, pulls the
# image for the current git HEAD SHA, recomputes the binary sha256, edits
# the deploy config in place (pivotContainerImageUrl + expectedPivotDigest),
# deletes the existing deploy, and creates a new one.
#
# Assumptions:
#   - `tvc login` is already done (or TVC_ORG_ID/TVC_API_KEY_* env set)
#   - `gh auth status` is logged in (used for ghcr.io docker login)
#   - The deploy config JSON has the appId, qosVersion, pivotArgs etc. set
#     already; this script only rewrites the two build-derived fields.
#
# Usage: scripts/redeploy-parser-http-server.sh [path/to/deploy.json]

set -euo pipefail

DEPLOY_JSON="${1:-deploy-2026-05-16-202301.json}"
REPO="ghcr.io/anchorageoss/parser_http_server"

if [[ ! -f "${DEPLOY_JSON}" ]]; then
  echo "deploy config not found: ${DEPLOY_JSON}" >&2
  exit 1
fi

APP_ID="$(jq -r .appId "${DEPLOY_JSON}")"
if [[ "${APP_ID}" == "<FILL_IN_APP_ID>" || -z "${APP_ID}" ]]; then
  echo "appId not set in ${DEPLOY_JSON}" >&2
  exit 1
fi

# Resolve the canonical PR-build tag. stagex.yml tags every PR image with
# both `pr-<N>` and the runner's `git rev-parse HEAD` — but for PR events
# `actions/checkout` defaults to `refs/pull/<N>/merge`, so the SHA tag is
# the *merge* commit, not the branch tip. `pr-<N>` is the stable handle.
PR_NUMBER="$(gh pr view --json number -q .number 2>/dev/null || true)"
if [[ -z "${PR_NUMBER}" ]]; then
  echo "no PR found for current branch; can't resolve pr-<N> tag" >&2
  exit 1
fi
PR_TAG="pr-${PR_NUMBER}"

echo "app:      ${APP_ID}"
echo "pr tag:   ${PR_TAG}"

echo "logging into ghcr…"
gh auth token | docker login ghcr.io -u "$(gh api user -q .login)" --password-stdin >/dev/null

echo "pulling ${REPO}:${PR_TAG}…"
docker pull -q "${REPO}:${PR_TAG}" >/dev/null

MANIFEST_DIGEST="$(docker inspect --format='{{index .RepoDigests 0}}' "${REPO}:${PR_TAG}" | cut -d@ -f2)"
echo "manifest: ${MANIFEST_DIGEST}"

CID="$(docker create "${REPO}:${PR_TAG}" /bin/true)"
trap 'docker rm "${CID}" >/dev/null 2>&1 || true' EXIT
BIN="/tmp/parser_http_server.${PR_TAG}"
docker cp "${CID}:/parser_http_server" "${BIN}"
BIN_DIGEST="$(sha256sum "${BIN}" | awk '{print $1}')"
echo "binary:   ${BIN_DIGEST}  (size $(stat -c%s "${BIN}") bytes)"

# Pin by digest for the deploy URL — the tag is just informational since
# the server stores and verifies the manifest digest. Including the
# `pr-<N>` tag in the URL gives operators a human-readable handle.
NEW_URL="${REPO}:${PR_TAG}@${MANIFEST_DIGEST}"
tmp="$(mktemp)"
jq --arg url "${NEW_URL}" --arg digest "${BIN_DIGEST}" \
  '.pivotContainerImageUrl = $url | .expectedPivotDigest = $digest' \
  "${DEPLOY_JSON}" > "${tmp}"
mv "${tmp}" "${DEPLOY_JSON}"
echo "updated ${DEPLOY_JSON}:"
jq '{pivotContainerImageUrl, expectedPivotDigest, pivotArgs}' "${DEPLOY_JSON}"

OLD_DEPLOY_ID="$(tvc app status --app-id "${APP_ID}" 2>/dev/null \
  | awk '/^Targeted Deployment:/ {print $3; exit}')"
if [[ -n "${OLD_DEPLOY_ID}" ]]; then
  echo "current live deploy: ${OLD_DEPLOY_ID} (will remain live until the new"
  echo "one is approved and \`tvc app set-live-deploy\`'d; then it can be deleted)"
fi

echo "creating new deploy…"
tvc deploy create --config-file "${DEPLOY_JSON}" | tee /tmp/tvc_create.out
NEW_DEPLOY_ID="$(awk '/^Deployment ID:/ {print $3; exit}' /tmp/tvc_create.out)"
echo ""
echo "next steps:"
echo "  tvc deploy approve --deploy-id ${NEW_DEPLOY_ID} --operator-id <YOUR_OPERATOR_ID>"
echo "  tvc app set-live-deploy --app-id ${APP_ID} --deploy-id ${NEW_DEPLOY_ID}"
if [[ -n "${OLD_DEPLOY_ID}" ]]; then
  echo "  tvc deploy delete --deploy-id ${OLD_DEPLOY_ID}"
fi
