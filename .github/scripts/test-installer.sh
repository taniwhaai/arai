#!/usr/bin/env bash
# Exercise the real installer with local release fixtures and no network.
set -euo pipefail
script_dir=$(cd "$(dirname "$0")" && pwd)
repo_dir=$(cd "$script_dir/../.." && pwd)
test_dir=$(mktemp -d -t arai-installer-test.XXXXXXXX)
trap 'rm -rf -- "$test_dir"' EXIT
mkdir -p "$test_dir/tools"
printf 'fixture executable\n' > "$test_dir/binary"
export INSTALL_TEST_ROOT="$test_dir"
export INSTALL_TEST_SHA
INSTALL_TEST_SHA=$(sha256sum "$test_dir/binary" | awk '{print $1}')

cat > "$test_dir/tools/uname" <<'SH'
#!/usr/bin/env bash
case "$1" in
  -s) printf '%s\n' "$INSTALL_TEST_OS" ;;
  -m) printf '%s\n' "$INSTALL_TEST_ARCH" ;;
  *) exit 1 ;;
esac
SH
cat > "$test_dir/tools/curl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$INSTALL_TEST_ROOT/calls"
output=''
url=${!#}
while [ "$#" -gt 0 ]; do
  if [ "$1" = -o ]; then output=$2; shift; fi
  shift
done
case "$url" in
  "https://github.com/taniwhaai/arai/releases/download/v9.9.9/checksums.txt")
    printf '%s  %s\n' "$INSTALL_TEST_SHA" "$INSTALL_TEST_ASSET" > "$output"
    ;;
  "https://github.com/taniwhaai/arai/releases/download/v9.9.9/$INSTALL_TEST_ASSET")
    cp "$INSTALL_TEST_ROOT/binary" "$output"
    printf 200
    ;;
  *) echo "Unexpected URL: $url" >&2; exit 1 ;;
esac
SH
chmod +x "$test_dir/tools/uname" "$test_dir/tools/curl"
export PATH="$test_dir/tools:$PATH" ARAI_VERSION=v9.9.9 ARAI_SKIP_CHECKSUM=0
export INSTALL_TEST_OS INSTALL_TEST_ARCH INSTALL_TEST_ASSET ARAI_FULL ARAI_INSTALL_DIR

for entry in 'Linux x86_64 linux-x86_64' 'Linux aarch64 linux-aarch64' \
             'Darwin x86_64 darwin-x86_64' 'Darwin arm64 darwin-aarch64' \
             'MINGW64_NT x86_64 windows-x86_64'; do
  read -r INSTALL_TEST_OS INSTALL_TEST_ARCH platform <<< "$entry"
  suffix=''
  if [[ "$platform" == windows-* ]]; then suffix=.exe; fi
  for ARAI_FULL in 0 1; do
    variant=arai
    if [ "$ARAI_FULL" = 1 ]; then variant=arai-full; fi
    INSTALL_TEST_ASSET="$variant-$platform$suffix"
    ARAI_INSTALL_DIR="$test_dir/install with spaces/$platform-$ARAI_FULL"
    sh "$repo_dir/install.sh" > "$test_dir/output" 2>&1
    cmp "$test_dir/binary" "$ARAI_INSTALL_DIR/arai$suffix"
    test -x "$ARAI_INSTALL_DIR/arai$suffix"
    grep -q 'Checksum verified' "$test_dir/output"
  done
done

# An old Windows extensionless payload must not continue shadowing arai.exe.
INSTALL_TEST_OS=MINGW64_NT
INSTALL_TEST_ARCH=x86_64
ARAI_FULL=0
INSTALL_TEST_ASSET=arai-windows-x86_64.exe
ARAI_INSTALL_DIR="$test_dir/windows upgrade with spaces"
mkdir -p "$ARAI_INSTALL_DIR"
printf 'previous extensionless payload\n' > "$ARAI_INSTALL_DIR/arai"
sh "$repo_dir/install.sh" > "$test_dir/output" 2>&1
test ! -e "$ARAI_INSTALL_DIR/arai"
cmp "$test_dir/binary" "$ARAI_INSTALL_DIR/arai.exe"
previous_files=("$ARAI_INSTALL_DIR"/.arai-previous.*/arai)
test "${#previous_files[@]}" = 1
grep -q '^previous extensionless payload$' "${previous_files[0]}"

INSTALL_TEST_OS=MSYS_NT
INSTALL_TEST_ARCH=arm64
rm "$test_dir/calls"
if sh "$repo_dir/install.sh" > "$test_dir/output" 2>&1; then
  echo 'Expected Windows ARM64 to fail before any download.' >&2
  exit 1
fi
test ! -e "$test_dir/calls"
grep -q 'No native Windows ARM64 release' "$test_dir/output"
echo 'Installer checks passed (10 platform/variant installs and a legacy Windows upgrade).'
