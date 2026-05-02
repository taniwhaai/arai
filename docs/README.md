# docs/

Public-facing documents committed to the repo and linked from the
marketing site (`site/index.html`) + main README.

## Files

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
