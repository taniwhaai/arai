# Release operations

Pushes to `main` run tests and release-plz. A version release creates a tag,
whose CI run builds binaries, attaches checksums/signatures, publishes packages,
and then updates the Homebrew formula once the binary assets are available.

## Authentication

`RELEASE_TOKEN` must be a valid dedicated personal access token with repository
Contents and Pull requests write access, or a GitHub App installation token with
equivalent permissions supplied while it is valid. A fine-grained PAT can be
restricted to this repository. Configure any additional workflow-file permission
needed by the chosen credential and repository policies.

Release-plz uses this credential through the GitHub API. There is deliberately
no fallback to the workflow's `GITHUB_TOKEN`: tags and releases created by that
token do not trigger the tag-driven build/publish workflow. Checkout and artifact
uploads use the job token because those operations do not need to trigger another
workflow. Checkout does not persist credentials.

The release job first makes read-only authentication and repository-access checks.
They suppress API responses and never print the token. These checks establish
identity and read access; the actual release operations still enforce write
permissions and repository rules.

If this preflight fails, a repository owner must replace `RELEASE_TOKEN` in the
repository's Actions secrets, then rerun the failed job. Changing workflow YAML
alone cannot repair an invalid or revoked token. Keep the separate
`CARGO_REGISTRY_TOKEN` and `NPM_TOKEN` publishing credentials current as well.
`HOMEBREW_TAP_TOKEN` needs Contents write access to `taniwhaai/homebrew-tap`;
without it the workflow emits a warning and skips that optional channel.

## Binary variants and Homebrew

Lean builds are required for every supported release platform. Full builds
include the optional enrichment dependencies and may be unavailable on a
platform where those dependencies cannot build. Only a successful full build is
staged; a byte-identical lean/full pair is rejected. Native full binaries also
run `--version` before staging. A failed optional full build produces a warning
and no full asset for that platform; it never republishes the lean file as full.

The Homebrew workflow is called after `attach-binaries` succeeds, using the same
tag. It also supports manual dispatch for an existing stable `vX.Y.Z` release
(empty input selects latest). Manual dispatch requires the release's checksum
manifest and platform binaries to have been uploaded already.

For validation without publishing, run:

```sh
bash .github/scripts/test-release-safety.sh
```

The regression checks use a fake GitHub CLI and temporary fixtures. They cover
missing/invalid credentials, inaccessible repositories, suppressed credential
diagnostics, stale full artifacts, native startup failure, and successful staging.

References: [GitHub workflow trigger semantics](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow#triggering-a-workflow-from-a-workflow),
[release-plz tokens](https://release-plz.dev/docs/github/token), and
[release-plz checkout credentials](https://release-plz.dev/docs/github/persist-credentials).
