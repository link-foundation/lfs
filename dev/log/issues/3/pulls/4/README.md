# Issue #3 / PR #4: false positives, false negatives, warnings and errors in CI/CD

This folder holds the raw evidence collected while solving
[issue #3](https://github.com/link-foundation/lfs/issues/3) in
[PR #4](https://github.com/link-foundation/lfs/pull/4). The files are kept
verbatim. `.pre-commit-config.yaml` and the lychee run in `links.yml` both
exclude `dev/log`.

| Path | Contents |
| --- | --- |
| `issue-3.json`, `pr-4.json` | Issue and pull request as returned by `gh` |
| `ci-logs/runs.json` | Recent runs on `main` |
| `ci-logs/run-<id>.log`, `run-<id>.json` | Full logs and metadata of the `main` runs listed in the issue (`e9e1354`, `34c010d`) |
| `ci-logs/annotations-main.tsv` | Every annotation (error, warning, notice) on those runs |
| `ci-logs/pr-run-*.log`, `annotations-pr2.tsv`, `pr2-run-*.json` | The last runs of PR #2 and the first run of this PR, for comparison |
| `ci-logs/release-<id>-failed.log` | Failed jobs of this PR's CI/CD Pipeline iterations |
| `templates/*.log` | Runs of the Rust, JS and Python pipeline templates, for comparing warnings |

## Timeline (2026-10-22, UTC)

| Time | Event |
| --- | --- |
| 07:59 | `main` at `e9e1354` (merge of PR #2). CI/CD Pipeline, Workflows and Broken Link Checker pass. Security fails in its `dependency-graph` job: *The dependency graph of link-foundation/lfs is disabled, so dependency review cannot run. A repository administrator has to enable "Dependency graph" at https://github.com/link-foundation/lfs/settings/security_analysis.* |
| 08:08 | The release creates `34c010d` (v0.21.0). Desktop Release passes. |
| 09:41 | Issue #3 is opened. |
| 09:42 | PR #4 is opened (`1375d3f`). Its CI passes. |
| 09:57 | `ba60cdb`: the main fixes. The CI/CD Pipeline fails on clippy `map_unwrap_or` in the new test, fixed by `6367200`. |
| 09:59 | `6367200`: Windows unit tests fail on imports that are unused off Unix (`Path`, `Command`, `Output`), fixed by `6bd45e6`. |
| 09:59–10:09 | `e2a6f04`, `8c6f6d4`, `3c1219c`, `9ec94d0`: files split under the 900-line warning threshold and the legacy caps removed. |
| 10:06, 10:09 | Runs 39760622993 and 39760967706 fail in integration tests this PR never touched: `nfs::stopping_the_server_commits_a_volume_that_reopens_clean` (Ubuntu, macOS, fresh merge) and `cli::a_volume_in_use_refuses_other_processes` (Ubuntu). Both fail with `Busy("… is in use by another process …")`. Fixed by `2959f4a`. |
| 10:22 | `1ea58ec`: every workflow passes. `ci-logs/annotations-pr4-1ea58ec.tsv` holds the remaining annotations: the intended dependency-graph warning, which needs an admin, the link checker's summary notice and GitHub's macOS capacity notice. No DEP0040, Git hint, offline zizmor or file-size output is left. The Pages deploy runs only on `main`, so the DEP0040 fix shows there first. |

## Requirements

1. Find every false positive, false negative, warning and error in CI/CD and fix it. Security was failing on `main`.
2. Compare the full file tree of the Rust, JS and Python pipeline templates and reuse their best practices.
3. If a template has the same problem, report an issue there too.
4. Follow [CI-CD-BEST-PRACTICES.md](https://github.com/link-assistant/hive-mind/blob/main/docs/CI-CD-BEST-PRACTICES.md).
5. Do everything in this single pull request.
6. From the task: collect the evidence here and reconstruct the timeline. Find each root cause, propose solutions and look for existing tools. Add verbose output that is off by default where a cause stays unclear. Report upstream issues with reproductions, workarounds and suggested fixes.

## Findings, root causes and fixes

### 1. Security fails on `main`: dependency graph disabled (an error, and a latent false positive)

- **Evidence.** In `run-39744445376.log` the `compare` API answers `403 Forbidden`. The repository's `security_and_analysis` has no `dependency_graph` entry, and `GET /dependency-graph/sbom` returns 404.
- **Root cause.** The dependency graph is a repository setting. Only an administrator can enable it, so no workflow change can turn it on. The failure on `main` is a **true positive**, by design (best practices principle 16: needed human actions fail loudly on the default branch).
- **Latent false positive.** The old probe counted *any* failed `gh api` call as "disabled". That includes a secondary rate limit, which also comes back as HTTP 403, and also 5xx and network errors. A transient GitHub hiccup would have printed the wrong instruction.
- **Fix.** `scripts/check-dependency-graph.sh` changes:
  - Only a plain `(HTTP 403)` without "rate limit" counts as disabled.
  - Everything else is retried (`DEPENDENCY_GRAPH_ATTEMPTS=3`, `DEPENDENCY_GRAPH_RETRY_SECONDS=10`) and then reported as `state=unknown`, never as disabled (principle 17).
  - `DEPENDENCY_GRAPH_DEBUG=1`, off by default, prints every raw answer.
  - Pull requests get a warning and skip the review; pushes to `main` fail with the settings URL.
- **Tests.** Four tests in `tests/unit/ci-cd/lfs_issue_3.rs` drive the script through a fake `gh`: enabled, disabled, retried transients, and unknown.
- **Blocked.** An administrator still has to enable *Dependency graph* at https://github.com/link-foundation/lfs/settings/security_analysis. Until then, Security on `main` keeps failing with that explicit message, as intended.

### 2. zizmor pedantic pass runs offline (a false negative)

- **Evidence.** `run-39744445425.log` contains "zizmor is running in offline mode by default".
- **Root cause.** The `pipx run zizmor` step had no `GH_TOKEN`, so it silently skipped the online audits (for example `impostor-commit` and `ref-confusion`).
- **Fix.** Give the step `GH_TOKEN: ${{ github.token }}`. The JS template already does this (#224).

### 3. `actions/deploy-pages` prints DEP0040 (a warning)

- **Evidence.** `[DEP0040] DeprecationWarning: The punycode module is deprecated` in the Pages deploy step.
- **Root cause.** A dependency bundled inside `actions/deploy-pages` uses `punycode`. The upstream issues are still open: actions/deploy-pages#434 and actions/deploy-pages#413.
- **Fix.** `NODE_OPTIONS: --disable-warning=DEP0040` on that step only. Other deprecations stay visible (see `experiments/issue-3-dep0040-node-options.sh`).

### 4. Git default-branch hint in every checkout (a warning)

- **Evidence.** "Using 'master' as the name for the initial branch" appears 10 times in the `main` logs.
- **Root cause.** `actions/checkout` runs `git init` before any step can configure Git.
- **Fix.** Workflow-level `GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_0` and `GIT_CONFIG_VALUE_0` set `init.defaultBranch=main` in every workflow. A test checks every workflow file.

### 5. Standalone shell scripts were never linted (a false negative)

- **Root cause.** actionlint shellchecks only inline `run:` blocks.
- **Fix.**
  - A ShellCheck job in `workflows.yml`, pinned through `shellcheck-py==0.11.0.1`.
  - `'**.sh'` added to its path filters.
  - The same pinned pre-commit hook.
  - The findings it reported are fixed.

### 6. File-size warnings on every run (a warning)

- **Evidence.** `annotations-main.tsv` shows three warnings: `workflow_release.rs` (1048 lines, legacy cap 1048), `src/links/mod.rs` (922) and `src/fs/tests.rs` (943). The legacy cap also hid `scripts/version-and-commit.rs`.
- **Fix.**
  - Split the files: `src/links/slots.rs`, `src/fs/tests/robustness.rs`, `tests/unit/ci-cd/workflow_release_gates.rs`, and `scripts/version-and-commit/{tests,crates-io}.rs`.
  - Delete `LEGACY_SOURCE_LIMITS`, so every source file has the regular 900/1000 budget.

### 7. Flaky "in use by another process" integration tests (a false positive)

- **Evidence.**
  - `release-39760622993-failed.log` and `release-39760967706-failed.log` fail right after the lock holder is released: `drop(held)` in `cli.rs`, `served.stop()` in `nfs.rs`.
  - The failure was not seen on `main` or in PR #2's runs. It is a latent race, and this PR touched neither the tests nor `src/device.rs`.
- **Root cause.**
  - `FileDevice` locks the image with `flock` (`File::try_lock`). A `flock` lock belongs to the *open file description*.
  - The integration test binary runs tests in parallel threads, and many of them spawn `lfs`. Between fork and exec, a spawned child holds a copy of every descriptor, `O_CLOEXEC` or not.
  - If another thread forks just before the volume is closed, the close only drops one reference. The lock survives until that child execs, so a reopen right then sees the volume as busy.
  - The same could hit any embedder that spawns processes while unmounting.
- **Reproduction.** `experiments/issue-3-flock-fork-inheritance.rs` locks, closes and relocks a file 20,000 times while other threads spawn `true`. With close only, 18 (1 spawning thread) and 182 (4 threads) reopens found the file still locked. With unlock-then-close, none did.
- **Fix.** `impl Drop for FileDevice` calls `file.unlock()` before the close. `LOCK_UN` releases the lock no matter how many descriptors share the description.
- **Test.** `device::tests::dropping_a_device_unlocks_it_while_its_descriptor_is_shared` holds a cloned descriptor, which is exactly what the half-spawned child holds. It fails without the fix with the same `Busy` error.

### 8. Windows build of the new test (an error this PR introduced, fixed)

- **Evidence.** `release-39759630131-failed.log` shows unused imports under `-D warnings` on Windows.
- **Fix.** The bash-driven tests now live in a `#[cfg(unix)]` module.

### Notices that need no action

- *Skipping Codecov upload because CODECOV_TOKEN is not configured*: an optional secret. The step reports it once and continues (principle 17).
- *Due to capacity constraints, jobs targeting macOS arm64 runners may experience longer queue times*: printed by GitHub, outside the repository's control.

## Template comparison

| Problem | lfs (this PR) | rust template | js template | python template |
| --- | --- | --- | --- | --- |
| Dependency-graph probe classifies 403 / rate limit / 5xx | fixed | missing, dependency review fails when the graph is off ([#207](https://github.com/link-foundation/rust-ai-driven-development-pipeline-template/issues/207)) | missing ([#249](https://github.com/link-foundation/js-ai-driven-development-pipeline-template/issues/249)) | dependency review not yet added ([#112](https://github.com/link-foundation/python-ai-driven-development-pipeline-template/issues/112), [comment](https://github.com/link-foundation/python-ai-driven-development-pipeline-template/issues/112#issuecomment-6338289549)) |
| DEP0040 from deploy-pages | fixed | present ([#208](https://github.com/link-foundation/rust-ai-driven-development-pipeline-template/issues/208)) | present ([#250](https://github.com/link-foundation/js-ai-driven-development-pipeline-template/issues/250)) | present ([#114](https://github.com/link-foundation/python-ai-driven-development-pipeline-template/issues/114)) |
| zizmor pedantic pass offline | fixed | present ([#209](https://github.com/link-foundation/rust-ai-driven-development-pipeline-template/issues/209)) | fixed upstream (#224) | zizmor not yet added (#110) |
| Git default-branch hint | fixed everywhere | only release/links fixed ([#210](https://github.com/link-foundation/rust-ai-driven-development-pipeline-template/issues/210)) | fixed | fixed (#94) |
| Legacy file-size caps | removed | present ([#211](https://github.com/link-foundation/rust-ai-driven-development-pipeline-template/issues/211)) | n/a | n/a |
| Standalone scripts not shellchecked | fixed | 10 findings ([#212](https://github.com/link-foundation/rust-ai-driven-development-pipeline-template/issues/212)) | 3 findings ([#251](https://github.com/link-foundation/js-ai-driven-development-pipeline-template/issues/251)) | `append_summary` drops the verdict ([#115](https://github.com/link-foundation/python-ai-driven-development-pipeline-template/issues/115)) |

Every upstream report includes a reproduction, a workaround and a suggested code fix.

## Best-practices mapping

- **Principle 2 (fail loudly, no silent skips):** zizmor online, ShellCheck on scripts.
- **Principle 14 (no permanent warnings):** file-size caps, DEP0040, the Git hint.
- **Principle 16 (human actions fail on the default branch with instructions, pass on pull requests):** the dependency graph.
- **Principle 17 (retry transients, report refusals once, report unknown instead of guessing):** the dependency-graph probe and the Codecov notice.

## Existing tools considered

- `actions/dependency-review-action` has no "skip when the graph is disabled" mode, so the probe stays.
- [ShellCheck](https://github.com/koalaman/shellcheck) is used through [shellcheck-py](https://github.com/shellcheck-py/shellcheck-py), for pinning and pre-commit support.
- [zizmor](https://github.com/zizmorcore/zizmor) is already in use.
- The flock fix uses only `std::fs::File::unlock`, with no new dependency.
