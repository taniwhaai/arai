#!/usr/bin/env bash
# new_ulid.sh — emit a single ULID-shaped identifier on stdout.
#
# A ULID is a 26-character Crockford Base32 string: 10 chars of millisecond
# timestamp, 16 chars of randomness. Sortable by creation time. Used throughout
# Taniwha state for event ids, decision ids, re-raise ids, and handoff ids.
#
# Usage: bash _shared/scripts/util/new_ulid.sh
# Output: a single 26-char ULID, no trailing newline beyond the standard one.

set -euo pipefail

python3 -c '
import secrets
import time

CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"

ts_ms = int(time.time() * 1000)

ts_chars = []
n = ts_ms
for _ in range(10):
    ts_chars.append(CROCKFORD[n & 0x1F])
    n >>= 5
ts_part = "".join(reversed(ts_chars))

rand_part = "".join(CROCKFORD[secrets.randbelow(32)] for _ in range(16))

print(ts_part + rand_part)
'
