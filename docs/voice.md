# Voice spec — Arai

## VoiceSpec — restrained-declarative register rules

> **committed — do not paraphrase**

1. **Declarative, calm sentences.** State what is or what happened. No exclamation marks, no "Done!"/"Done.", no cute or marketing flourish, no cheerleading.
2. **Specific over template.** An error names the thing and the consequence, not a bare "Failed to X". Form: `Could not <verb> <named thing>: {e}` (or an equally specific declarative). The underlying `{e}` detail is retained.
3. **Adequate at the deny.** A deny quotes the rule and its `source:line` — enough for the reader to act — with no lecture and no padding. The existing rule/source/line payload is kept; only the framing words change.
4. **No anthropomorphism.** Arai does not "watch your rules", "keep an eye on", "want", or "think". It enforces, records, blocks, allows. ("watching your rules" → "enforcing this project's rules" / "watching this project".)
5. **Sentence case; periods on full sentences.** Headers and labels use sentence case (not Title Case, not ALL CAPS). Full sentences end with a period. Fragments/labels (e.g. a status field) need not.
6. **Preserve #83 colour and #84 glyph behaviour.** This slice changes WORDS, not the colour bytes (#83) or the outcome-glyph prefix (#84). Where a line is coloured or glyph-prefixed today, it stays coloured/prefixed; only the words change.

## SelfReferenceGlossary — one term per concept, held consistently

> **committed — do not paraphrase**

| Concept | Committed term | Notes |
|---|---|---|
| the tool itself | **Arai** | Proper noun. Never "we", never "I". |
| a single constraint | **rule** (plural **rules**) | The unit a user authors / that fires. |
| the system / the command surface | **guardrails** | The `arai guardrails` command keeps its name; "guardrails" is the collective system, "rule" is the unit. |
| the AI actor | **the model** | Not "the agent", "the assistant", "Claude". |
| the human reader | **you** | Second person. Not "the user" in user-facing prose. |
