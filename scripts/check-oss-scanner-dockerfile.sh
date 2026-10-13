#!/usr/bin/env bash
# Fail if stage 1 of .oss-scanner/Dockerfile drifts from the build stage of images/parser_app/Containerfile.
# Stage 1 must stay a line-for-line copy so the release parser_app in the scan image has the same sha256 as
# `make non-oci-docker-images`. Comments, blank lines, stage names and the heredoc delimiter are ignored.
set -euo pipefail

first_stage() {
    awk '/^FROM /{n++} n==1' "$1" | sed -e 's/[[:space:]]*#.*$//' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' \
        -e 's/ AS [A-Za-z_]*$//' -e 's/EOF2/EOF/g' | grep -v '^$'
}

cd "$(git rev-parse --show-toplevel)"
if ! diff -u <(first_stage images/parser_app/Containerfile) <(first_stage .oss-scanner/Dockerfile); then
    echo "error: stage 1 of .oss-scanner/Dockerfile no longer matches images/parser_app/Containerfile" >&2
    exit 1
fi
echo "ok: .oss-scanner/Dockerfile stage 1 matches images/parser_app/Containerfile"
