# Post-migration behaviour: print hint and exit

**Kind:** user_intervention
**Triggered by:** 01KSF4GQ3ER56WH6XWHQBBKECQ (user_input_received event, 2026-05-25)
**Affects:** contract/legacy-path-migration (v1, to be derived)

## Decision

After a successful migration (user accepted, ~/.arai/ moved to ~/.taniwha/arai/),
`arai init` prints a short confirmation message and exits with success (exit code 0).
It does NOT re-resolve Config::load within the same invocation.

Chosen option: **Print hint and exit**

> Emit a short message such as "Migration complete. Run 'arai init' again
> to complete initialisation." and exit with success (exit code 0). No
> re-resolution or second Config::load needed. The user runs "arai init"
> once more and everything operates against the new path.

## Rationale

This was the design-doc-recommended option. The design doc (v2, "Open questions")
notes that the rest of the init flow may already have made decisions against the
legacy path before the prompt fired, making re-resolution inside the same invocation
a source of subtle ordering bugs. The "print hint and exit" path is the
minimal-surprise reading and does not require threading re-resolution logic through
the post-migration caller site.

Confirmed by the user (matt@mustard.co.nz) on 2026-05-25.

## Implications for contract derivation

The `legacy-path-migration` module's `MigrationOutcome::prompted-accepted` variant
is purely informational to the `cmd_init` entry point. On receiving this outcome,
`cmd_init` prints the confirmation hint and exits. No second call to Config::load
is made. The contract for the module does not need to express any re-resolution
behaviour; the entry-point change in `src/main.rs` is the mechanical point where
this decision is encoded.
