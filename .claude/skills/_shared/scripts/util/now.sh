#!/usr/bin/env bash
# now.sh — emit the current UTC time in one of two formats.
#
# Default format: ISO 8601 with Z suffix, e.g. 2026-05-01T22:08:33Z
# --filename:     packed for use in event filenames, e.g. 20260501T220833Z
#
# Used throughout Taniwha state. The two formats must agree on the same
# instant — call this script once per logical event and use both forms from
# its output if you need both.
#
# Usage:
#   bash _shared/scripts/util/now.sh              # ISO 8601 with Z
#   bash _shared/scripts/util/now.sh --filename   # filename form
#   bash _shared/scripts/util/now.sh --both       # both, space-separated:
#                                                 # "2026-05-01T22:08:33Z 20260501T220833Z"

set -euo pipefail

mode="${1:-iso}"

case "$mode" in
    iso|"")
        date -u +"%Y-%m-%dT%H:%M:%SZ"
        ;;
    --filename|filename)
        date -u +"%Y%m%dT%H%M%SZ"
        ;;
    --both|both)
        # Single date call so both forms reflect the same instant.
        # date -u doesn't take multiple format args portably, so we capture once.
        epoch=$(date -u +%s)
        iso=$(date -u -d "@$epoch" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -r "$epoch" +"%Y-%m-%dT%H:%M:%SZ")
        fname=$(date -u -d "@$epoch" +"%Y%m%dT%H%M%SZ" 2>/dev/null || date -u -r "$epoch" +"%Y%m%dT%H%M%SZ")
        echo "$iso $fname"
        ;;
    *)
        echo "usage: $0 [--filename|--both]" >&2
        exit 2
        ;;
esac
