# Shared policies

Org-wide rules via arai:extends: trust list, content pinning, signatures, and private policy sources.

## Shared policies — `arai:extends`

Instruction files can inherit rules from a trusted upstream URL. This
is the "org-wide CLAUDE.md" pattern without a policy service — just
another markdown file hosted wherever you like.

Declare the upstream in your CLAUDE.md:

```markdown
<!-- arai:extends https://example.com/standards/rust-backend.md -->

# My project rules
- Never publish artifacts before tag push
```

Then trust the URL:

```bash
arai trust --add https://example.com/standards/rust-backend.md
arai trust                  # List trusted URLs
arai trust --remove <url>   # Revoke
```

Ārai never fetches a URL that isn't explicitly trusted. HTTPS only,
512 KB size cap, 24-hour cache with stale-while-error fallback, and
extends are not recursive — the fetched file can't pull in further
URLs. On `arai init`, trusted upstream content is inlined ahead of the
local rules before the parser runs, so the rest of the pipeline sees
one merged file.

### Private policy sources (authenticated extends)

If your org policy file lives behind auth (an internal service, a
private GitHub raw URL, an artifact store), give the trust entry the
*name* of an environment variable holding a bearer token:

```bash
arai trust --add https://policy.internal.example/org-rules.md \
           --bearer-env ARAI_EXTENDS_TOKEN
export ARAI_EXTENDS_TOKEN="<token>"   # e.g. from your secret manager
arai scan                             # fetches with Authorization: Bearer <token>
```

Secret handling, by construction:

- Only the variable **name** is stored (in `trusted_extends.toml`); the
  token itself never touches disk, the audit log, telemetry, or error
  messages.
- The header is sent **only** to the exact trusted URL (and its
  `<url>.sig` signature sidecar on the same origin). Redirects are
  disabled on the fetch path, so a 30x can never carry the token to
  another host. HTTPS only, as always.
- If the variable is unset or empty, the fetch proceeds
  unauthenticated with a warning — same behavior as before the
  credential was configured.
- Content pinning (`@sha256:`), ed25519 signatures, tiers, caching,
  and the size cap all work unchanged on authenticated responses.
