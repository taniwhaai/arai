# Rule enrichment

The three classification tiers and the per-rule (noenrich) opt-out.

## Enrichment

Three tiers of rule understanding, each more accurate:

```bash
arai scan                  # Tier 1: Built-in verb taxonomy (free, instant)
arai scan --enrich         # Tier 2: Sentence transformer model (local, ~80MB download)
arai scan --enrich-llm     # Tier 3a: LLM classification via CLI
arai scan --enrich-api     # Tier 3b: LLM classification via API (no CLI needed)
```

Configure your LLM:
```bash
# Via CLI tool (shell-out)
ARAI_LLM_CMD="claude -p" arai scan --enrich-llm
ARAI_LLM_CMD="ollama run llama3" arai scan --enrich-llm

# Via API (OpenAI-compatible endpoints)
ARAI_API_KEY=sk-... arai scan --enrich-api                    # OpenAI (default)
ARAI_API_URL=http://localhost:11434/v1 arai scan --enrich-api  # Ollama (auto-detected)
ARAI_API_URL=https://api.groq.com/openai/v1 ARAI_API_KEY=gsk-... ARAI_API_MODEL=llama-3.3-70b-versatile arai scan --enrich-api

# Or in ~/.taniwha/arai/config.toml
[enrich]
llm_command = "llm -m gpt-4o-mini"       # for --enrich-llm
api_url = "https://api.openai.com/v1"     # for --enrich-api
api_key_env = "OPENAI_API_KEY"
model = "gpt-4o-mini"
```


## Per-rule enrichment opt-out — `(noenrich)`

`arai scan --enrich-llm` and `--enrich-api` send the full text of every
guardrail to whatever LLM you've configured (`ARAI_LLM_CMD` /
`ARAI_API_URL`). For most rules that's fine — they're already in
`CLAUDE.md`. But if a single rule mentions an internal codename you'd
rather not ship to a third-party endpoint, append `(noenrich)`:

```markdown
- Never deploy to internal-codename-cluster (noenrich)
```

The annotation is stripped from the rule body at parse time and stored
separately; the enrichment paths filter the rule out before building the
prompt. `(noenrich)` and `(expires …)` can appear together in either
order. To opt out globally, just don't pass `--enrich-llm` /
`--enrich-api` — neither runs by default.

Before each enrichment run Arai prints a one-line notice with the
resolved destination and a locality verdict (`local` / `REMOTE` /
`unknown locality`), plus the count of rules excluded via `(noenrich)`,
so you can see at a glance whether rule text is about to leave the
host.
