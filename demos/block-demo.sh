#!/usr/bin/env bash
# Arai demo: blocking a forbidden command at the PreToolUse hook.
# Replayable: `asciinema rec demos/block.cast -c "bash demos/block-demo.sh"`

set -u

GREEN=$'\033[1;32m'
CYAN=$'\033[1;36m'
RED=$'\033[1;31m'
DIM=$'\033[2m'
BOLD=$'\033[1m'
RESET=$'\033[0m'

type_cmd() {
    local cmd="$1"
    printf "%s\$%s " "$GREEN" "$RESET"
    local i
    for ((i = 0; i < ${#cmd}; i++)); do
        printf "%s" "${cmd:$i:1}"
        sleep 0.025
    done
    printf "\n"
}

clear
sleep 0.5

printf "%s# Arai — blocking a command the model was told not to run%s\n\n" "$BOLD" "$RESET"
sleep 1.0

printf "%s# 1. The rule, as parsed from CLAUDE.md:%s\n" "$DIM" "$RESET"
type_cmd 'arai guardrails | grep -i "git push"'
arai guardrails | grep -i "git push"
sleep 1.6

printf "\n%s# 2. Claude Code is about to run a forbidden command.%s\n"   "$DIM" "$RESET"
printf "%s#    Its PreToolUse hook pipes the tool call to arai:%s\n"     "$DIM" "$RESET"
PAYLOAD='{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git push --force origin main"}}'
type_cmd "echo '$PAYLOAD' \\"
printf "    %s|%s arai guardrails --match-stdin\n" "$CYAN" "$RESET"
sleep 0.7

printf "\n"
echo "$PAYLOAD" | arai guardrails --match-stdin | jq -C .
printf "\n"
sleep 2.5

printf "%s#    permissionDecision: \"deny\" → Claude Code refuses to run the tool.%s\n\n" "$DIM" "$RESET"
sleep 1.5

printf "%s# 3. The firing is recorded in the local audit log:%s\n" "$DIM" "$RESET"
type_cmd 'arai audit | tail -3'
arai audit 2>/dev/null | tail -3
sleep 2.5

printf "\n%s# Rule enforced. No prompt-engineering, no LLM in the loop —%s\n" "$DIM" "$RESET"
printf "%s# just a deterministic hook reading your CLAUDE.md.%s\n" "$DIM" "$RESET"
sleep 2.0
