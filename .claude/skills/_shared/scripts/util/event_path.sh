#!/usr/bin/env bash
# event_path.sh — build the canonical event file path for a Taniwha event.
#
# Events live at .taniwha/kupu/events/<UTC-year>/<UTC-month>/<UTC-day>/<filename-ts>-<event-id>.yaml.
# This script takes an event id and (optionally) a timestamp, and emits the
# full repo-relative path. It does NOT create the file or its parent
# directories — callers are expected to mkdir -p the parent before writing.
#
# Usage:
#   bash _shared/scripts/util/event_path.sh <event-id>                # uses current UTC time
#   bash _shared/scripts/util/event_path.sh <event-id> <iso-timestamp> # uses given time
#
# Output: a single path string, e.g.
#   .taniwha/kupu/events/2026/05/01/20260501T220833Z-01KQGF40000USERINPUT001.yaml

set -euo pipefail

if [ "$#" -lt 1 ]; then
    echo "usage: $0 <event-id> [<iso-timestamp>]" >&2
    exit 2
fi

event_id="$1"
iso_ts="${2:-}"

if [ -z "$iso_ts" ]; then
    iso_ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
fi

# Parse YYYY-MM-DDTHH:MM:SSZ
year="${iso_ts:0:4}"
month="${iso_ts:5:2}"
day="${iso_ts:8:2}"
hour="${iso_ts:11:2}"
minute="${iso_ts:14:2}"
second="${iso_ts:17:2}"

filename_ts="${year}${month}${day}T${hour}${minute}${second}Z"

echo ".taniwha/kupu/events/${year}/${month}/${day}/${filename_ts}-${event_id}.yaml"
