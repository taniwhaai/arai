#!/usr/bin/env bash
# Read-only preflight. Never print credentials or GitHub's response body.
set -euo pipefail

if [ -z "${GH_TOKEN:-}" ]; then
  echo "::error::RELEASE_TOKEN is required. Configure a dedicated PAT or GitHub App token; the job token cannot trigger tag-driven publishing."
  exit 1
fi

if [ -z "${GITHUB_REPOSITORY:-}" ]; then
  echo "::error::GITHUB_REPOSITORY is required for release authentication checks."
  exit 1
fi

if ! gh api graphql -f query='query { viewer { login } }' --silent >/dev/null 2>&1; then
  echo "::error::RELEASE_TOKEN authentication failed. Replace the invalid credential, then rerun the release job."
  exit 1
fi

if ! gh api "repos/${GITHUB_REPOSITORY}" --silent >/dev/null 2>&1; then
  echo "::error::RELEASE_TOKEN cannot read this repository. Check its repository access before rerunning."
  exit 1
fi

echo "Release authentication and repository access verified (read-only)."
