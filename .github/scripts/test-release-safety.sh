#!/usr/bin/env bash
# Exercise failure paths without a GitHub token, network, or compiled binaries.
set -euo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
test_dir=$(mktemp -d)
trap 'rm -f "$test_dir/gh" "$test_dir/lean" "$test_dir/full" "$test_dir/staged" "$test_dir/output" "$test_dir/calls"; rmdir "$test_dir"' EXIT

cat > "$test_dir/gh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$GH_TEST_CALLS"
if [ "$GH_TEST_MODE" = invalid ]; then
  printf 'untrusted diagnostic containing %s\n' "$GH_TOKEN" >&2
  exit 1
fi
if [ "$GH_TEST_MODE" = no-repo ] && [[ "$*" == *repos/* ]]; then
  exit 1
fi
SH
chmod +x "$test_dir/gh"
export PATH="$test_dir:$PATH" GITHUB_REPOSITORY=example/arai
export GH_TEST_CALLS="$test_dir/calls" GH_TEST_MODE=valid

expect_failure() {
  if "$@" > "$test_dir/output" 2>&1; then
    echo "Expected command to fail: $*" >&2
    exit 1
  fi
}

export GH_TOKEN=''
expect_failure bash "$script_dir/check-release-token.sh"
test ! -e "$test_dir/calls"

export GH_TOKEN=regression-test-secret GH_TEST_MODE=invalid
expect_failure bash "$script_dir/check-release-token.sh"
if grep -q "$GH_TOKEN" "$test_dir/output"; then
  echo "Credential leaked into preflight output" >&2
  exit 1
fi

export GH_TEST_MODE=no-repo
expect_failure bash "$script_dir/check-release-token.sh"

export GH_TEST_MODE=valid
bash "$script_dir/check-release-token.sh"

printf 'lean binary\n' > "$test_dir/lean"
cp "$test_dir/lean" "$test_dir/full"
expect_failure bash "$script_dir/stage-full-binary.sh" "$test_dir/full" "$test_dir/lean" "$test_dir/staged"
test ! -e "$test_dir/staged"

printf '#!/usr/bin/env bash\nexit 1\n' > "$test_dir/full"
chmod +x "$test_dir/full"
expect_failure bash "$script_dir/stage-full-binary.sh" "$test_dir/full" "$test_dir/lean" "$test_dir/staged" true
test ! -e "$test_dir/staged"

printf '#!/usr/bin/env bash\nprintf "arai test-full\\n"\n' > "$test_dir/full"
bash "$script_dir/stage-full-binary.sh" "$test_dir/full" "$test_dir/lean" "$test_dir/staged" true
cmp "$test_dir/full" "$test_dir/staged"
echo "Release safety regression checks passed."
