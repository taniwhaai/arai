#!/usr/bin/env bash
# Called only after a successful enrich build. Reject stale lean artifacts too.
set -euo pipefail

source_binary=$1
lean_binary=$2
full_binary=$3
smoke_test=${4:-false}

if [ ! -s "$source_binary" ] || [ ! -s "$lean_binary" ]; then
  echo "::error::Expected nonempty full and lean build outputs before staging."
  exit 1
fi

if cmp -s "$source_binary" "$lean_binary"; then
  echo "::error::Full build is identical to lean; refusing to publish a mislabeled artifact."
  exit 1
fi

# Cross-compiled binaries cannot run on this runner. Native binaries must start.
if [ "$smoke_test" = true ]; then
  "$source_binary" --version
fi

cp "$source_binary" "$full_binary"
chmod +x "$full_binary"
