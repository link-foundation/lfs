---
bump: patch
---

### Fixed
- CI/CD no longer reports a transient GitHub API failure as a disabled dependency graph: only a plain `403 Forbidden` counts as disabled, other answers are retried and then reported as unknown, and `DEPENDENCY_GRAPH_DEBUG=1` prints every raw answer.
- The pedantic zizmor pass now runs with a token, so it no longer falls back to offline mode and skips the online audits.
- GitHub Pages deployment no longer prints the `punycode` DEP0040 deprecation warning from `actions/deploy-pages`.
- The Security, Workflows and Desktop Release workflows no longer print Git's default-branch hint in every checkout.
- Every file is back under the file-size warning threshold, so the check no longer warns on every change to `src/links/mod.rs`, `src/fs/tests.rs`, `scripts/version-and-commit.rs` and `tests/unit/ci-cd/workflow_release.rs`; the legacy caps that kept the last two permanently inside the warning band are gone.

### Added
- A pinned ShellCheck job (and pre-commit hook) lints every tracked shell script, which actionlint never covered; the findings it raised are fixed.
