# docs/

Public-facing documents committed to the repo and linked from the
marketing site (`site/index.html`) + main README.

## Product guides

| File | What it covers |
|------|----------------|
| [enforcement.md](enforcement.md) | Deny mode, severity pins, disable/enable, `check-diff`, `why`, compliance, expiry |
| [audit.md](audit.md) | Local JSONL log, `--verify`, `status`, `stats` |
| [audit-ship.md](audit-ship.md) | `arai audit --ship` to your own collector |
| [rule-testing.md](rule-testing.md) | `diff`, `lint`, `test`, `record` |
| [extends.md](extends.md) | `arai:extends`, trust list, signatures, private sources |
| [mcp.md](mcp.md) | Stdio MCP tools (add / list / check_action / recent_decisions) |
| [codex.md](codex.md) | Codex native hooks, `/hooks` trust, `apply_patch` |
| [instruction-discovery.md](instruction-discovery.md) | Nested files, Claude `paths`, Cursor `globs`, legacy adoption |
| [rules-file-spec.md](rules-file-spec.md) | Canonical `arai.toml`; `canonicalize` / `sync` |
| [enrichment.md](enrichment.md) | Taxonomy / ONNX / LLM classification tiers |
| [repo-layer-scope.md](repo-layer-scope.md) | What `check-diff` does and does not match |
| [telemetry-payload.md](telemetry-payload.md) | Self-hosted telemetry event schema |
| [stack-integration.md](stack-integration.md) | Arai upstream of Kete |
| [releases.md](releases.md) | Release-plz, tokens, Homebrew |
| [upstream/grok-hooks-reverification-1.0.0.md](upstream/grok-hooks-reverification-1.0.0.md) | Grok Build 1.0.0 PreToolUse deny evidence |
| [voice.md](voice.md) | User-facing copy register (committed; do not paraphrase) |

Design notes (not user-facing how-tos): [design-http-hooks-kete-integration.md](design-http-hooks-kete-integration.md).

## Compliance feature inventory

- **`arai-compliance-features.pdf`** — the canonical artifact linked
  from the site and README.  GitHub previews PDFs inline, so an
  evaluator clicking the link sees the document in their browser
  without a download prompt.
- **`arai-compliance-features.docx`** — Word source for the same
  content.  Edit this when the compliance feature inventory needs an
  update; the PDF is regenerated from it.

## Regenerating the PDF after editing the docx

On Windows with Microsoft Word installed, this PowerShell snippet
drives Word headlessly to convert in place:

```powershell
$src = (Resolve-Path .\docs\arai-compliance-features.docx).Path
$dst = (Join-Path (Resolve-Path .\docs).Path 'arai-compliance-features.pdf')
$word = New-Object -ComObject Word.Application
$word.Visible = $false
$word.DisplayAlerts = 0
try {
    $doc = $word.Documents.Open($src, [ref]$false, [ref]$true)  # ReadOnly
    $doc.SaveAs([ref]$dst, [ref]17)  # 17 = wdFormatPDF
    $doc.Close([ref]$false)
} finally {
    $word.Quit()
    [System.Runtime.Interopservices.Marshal]::ReleaseComObject($word) | Out-Null
}
```

On Linux / macOS, `libreoffice --headless --convert-to pdf
docs/arai-compliance-features.docx --outdir docs/` produces an
equivalent file.

Both formats must be committed together — link consumers (site,
README, external partners with the URL) point at the PDF; the docx
is for the next editor.
