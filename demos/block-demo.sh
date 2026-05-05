#!/usr/bin/env bash
# Arai demo: a forbidden tool call as it appears mid-Claude-Code-session.
# Replayable: `asciinema rec demos/block.cast -c "bash demos/block-demo.sh"`

set -u

RESET=$'\033[0m'
DIM=$'\033[2m'
BOLD=$'\033[1m'
RED=$'\033[1;31m'
GREEN=$'\033[1;32m'
YELLOW=$'\033[1;33m'
ORANGE=$'\033[38;5;208m'
WHITE=$'\033[1;37m'
GREY=$'\033[38;5;245m'

type_slow() {
    local text="$1"
    local i
    for ((i = 0; i < ${#text}; i++)); do
        printf "%s" "${text:$i:1}"
        sleep 0.022
    done
}

clear
sleep 0.4

# ── User prompt ────────────────────────────────────────────────────────
printf "%s>%s " "$GREY" "$RESET"
type_slow "ship the auth fixes to main"
printf "\n\n"
sleep 0.7

# ── Claude turn 1: acknowledgment (typed, like the model streaming) ────
printf "%s●%s " "$ORANGE" "$RESET"
type_slow "Pushing the changes to main now."
printf "\n\n"
sleep 0.5

# ── Claude turn 2: tool call (renders instantly in Claude Code) ────────
printf "%s●%s %sBash%s(git push --force origin main)\n" \
       "$ORANGE" "$RESET" "$BOLD" "$RESET"
sleep 0.4
printf "  %s⎿%s  %sRunning…%s\n" "$GREY" "$RESET" "$DIM" "$RESET"
sleep 0.9

# ── The block ─ overwrite "Running…" with the deny ─────────────────────
printf "\033[1A\033[2K"
printf "  %s⎿%s  %sError%s (PreToolUse:Bash hook):\n" \
       "$GREY" "$RESET" "$RED" "$RESET"
printf "     %sArai: \"Git never: git push to main without a PR\"%s\n" \
       "$RED" "$RESET"
printf "     %s[from CLAUDE.md:12 — \"never\" → block]%s\n\n" \
       "$DIM" "$RESET"
sleep 2.0

# ── Claude recovers ────────────────────────────────────────────────────
printf "%s●%s " "$ORANGE" "$RESET"
type_slow "Right — Arai blocked that. CLAUDE.md says never push to main"
printf "\n  "
type_slow "without a PR. Let me branch off and open one instead."
printf "\n\n"
sleep 0.6

printf "%s●%s %sBash%s(git checkout -b auth-fixes && git push -u origin auth-fixes)\n" \
       "$ORANGE" "$RESET" "$BOLD" "$RESET"
sleep 0.5
printf "  %s⎿%s  %sBranch 'auth-fixes' set up to track origin/auth-fixes.%s\n" \
       "$GREY" "$RESET" "$DIM" "$RESET"
printf "     %sremote: Create a pull request for 'auth-fixes':%s\n" \
       "$DIM" "$RESET"
printf "     %s  https://github.com/you/repo/compare/auth-fixes%s\n\n" \
       "$DIM" "$RESET"
sleep 3.2
