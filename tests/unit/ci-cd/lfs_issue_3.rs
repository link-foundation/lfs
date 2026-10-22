//! Regression tests for LFS issue #3: false positives, false negatives,
//! warnings and errors in CI/CD.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workflow(name: &str) -> String {
    fs::read_to_string(root().join(".github/workflows").join(name))
        .unwrap_or_else(|error| panic!("read {name}: {error}"))
        .replace("\r\n", "\n")
}

/// The step of `workflow` whose `name:` or `uses:` line contains `needle`.
fn step<'a>(workflow: &'a str, needle: &str) -> &'a str {
    workflow
        .split("\n      - ")
        .find(|step| {
            step.lines()
                .next()
                .is_some_and(|line| line.contains(needle))
        })
        .unwrap_or_else(|| panic!("no step starts with {needle}"))
}

#[test]
fn deploy_pages_does_not_print_the_punycode_deprecation() {
    let release = workflow("release.yml");
    let deploy = step(&release, "name: Deploy to GitHub Pages");
    assert!(deploy.contains("uses: actions/deploy-pages@"), "{deploy}");
    assert!(
        deploy.contains("env:\n          NODE_OPTIONS: --disable-warning=DEP0040"),
        "deploy-pages must silence only DEP0040: {deploy}"
    );
    assert!(!deploy.contains("--no-warnings"), "{deploy}");
}

#[test]
fn the_pedantic_zizmor_pass_runs_online_audits() {
    let workflows = workflow("workflows.yml");
    let audit = step(
        &workflows,
        "name: Audit for pedantic-only high-severity findings",
    );
    assert!(audit.contains("pipx run zizmor=="), "{audit}");
    assert!(
        audit.contains("env:\n          GH_TOKEN: ${{ github.token }}"),
        "without a token zizmor runs offline and skips online audits: {audit}"
    );
}

#[test]
fn every_tracked_shell_script_is_shellchecked() {
    let workflows = workflow("workflows.yml");
    let job = step(&workflows, "name: Shellcheck every tracked shell script");
    assert!(
        job.contains("git ls-files -z '*.sh'")
            && job.contains("pipx run --spec shellcheck-py==")
            && job.contains(" shellcheck"),
        "{job}"
    );
    assert_eq!(
        workflows
            .matches("paths: ['.github/**', 'scripts/check-workflow-tools.rs', '**.sh']")
            .count(),
        2,
        "a change to any shell script must trigger the shellcheck job"
    );
}

#[test]
fn every_workflow_suppresses_the_git_default_branch_hint() {
    for entry in fs::read_dir(root().join(".github/workflows")).unwrap() {
        let path = entry.unwrap().path();
        let text = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
        assert!(
            text.contains(
                "\n  GIT_CONFIG_COUNT: '1'\n  GIT_CONFIG_KEY_0: init.defaultBranch\n  GIT_CONFIG_VALUE_0: main\n"
            ),
            "{} must set init.defaultBranch for actions/checkout",
            path.display()
        );
    }
}

/// Runs scripts/check-dependency-graph.sh with a fake `gh` that answers each
/// call with the next of `answers`: `ok` or an error text printed to stderr.
#[cfg(unix)]
fn dependency_graph(answers: &[&str], mode: &str) -> (Output, String, usize) {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CASE: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "lfs-issue-3-graph-{}-{}",
        std::process::id(),
        CASE.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (index, answer) in answers.iter().enumerate() {
        fs::write(dir.join(format!("answer-{}", index + 1)), answer).unwrap();
    }
    let gh = dir.join("gh");
    fs::write(
        &gh,
        r#"#!/usr/bin/env bash
dir="$(dirname "$0")"
calls=$(( $(cat "$dir/calls" 2>/dev/null || echo 0) + 1 ))
echo "$calls" > "$dir/calls"
answer="$(cat "$dir/answer-$calls")"
if [ "$answer" = ok ]; then echo '{"changes":[]}'; exit 0; fi
echo "$answer" >&2
exit 1
"#,
    )
    .unwrap();
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    let output_file = dir.join("github-output");
    let mut command = Command::new("bash");
    command
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/check-dependency-graph.sh"))
        .args(["base", "head"]);
    if !mode.is_empty() {
        command.arg(mode);
    }
    let output = command
        .env(
            "PATH",
            format!("{}:{}", dir.display(), std::env::var("PATH").unwrap()),
        )
        .env("REPO", "owner/repo")
        .env("GITHUB_OUTPUT", &output_file)
        .env("DEPENDENCY_GRAPH_RETRY_SECONDS", "0")
        .output()
        .unwrap();
    let github_output = fs::read_to_string(&output_file).unwrap_or_default();
    let calls =
        fs::read_to_string(dir.join("calls")).map_or(0, |text| text.trim().parse().unwrap());
    let _ = fs::remove_dir_all(&dir);
    (output, github_output, calls)
}

#[cfg(unix)]
const FORBIDDEN: &str = r#"{"message":"Forbidden","status":"403"}gh: Forbidden (HTTP 403)"#;
#[cfg(unix)]
const RATE_LIMITED: &str = r#"{"message":"You have exceeded a secondary rate limit"}gh: You have exceeded a secondary rate limit (HTTP 403)"#;
#[cfg(unix)]
const SERVER_ERROR: &str = r#"{"message":"Server Error"}gh: Server Error (HTTP 502)"#;

#[cfg(unix)]
fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[cfg(unix)]
#[test]
fn an_enabled_dependency_graph_passes_quietly() {
    let (output, github_output, calls) = dependency_graph(&["ok"], "--require");
    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(github_output, "enabled=true\nstate=enabled\n");
    assert_eq!(calls, 1);
    assert!(!stdout(&output).contains("::"), "{}", stdout(&output));
}

#[cfg(unix)]
#[test]
fn a_disabled_dependency_graph_warns_on_pull_requests_and_fails_on_main() {
    let (output, github_output, calls) = dependency_graph(&[FORBIDDEN], "");
    assert!(output.status.success());
    assert_eq!(github_output, "enabled=false\nstate=disabled\n");
    assert_eq!(calls, 1, "a definite 403 must not be retried");
    assert!(stdout(&output).contains("::warning::The dependency graph of owner/repo is disabled"));
    assert!(stdout(&output).contains("https://github.com/owner/repo/settings/security_analysis"));

    let (output, _, _) = dependency_graph(&[FORBIDDEN], "--require");
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("::error::The dependency graph of owner/repo is disabled"));
}

#[cfg(unix)]
#[test]
fn transient_api_failures_are_retried_instead_of_reported_as_disabled() {
    let (output, github_output, calls) =
        dependency_graph(&[RATE_LIMITED, SERVER_ERROR, "ok"], "--require");
    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(github_output, "enabled=true\nstate=enabled\n");
    assert_eq!(calls, 3);
    assert!(!stdout(&output).contains("disabled"), "{}", stdout(&output));
}

#[cfg(unix)]
#[test]
fn an_unanswered_dependency_graph_check_is_reported_as_unknown() {
    let answers = [SERVER_ERROR, RATE_LIMITED, "gh: connection refused"];
    let (output, github_output, calls) = dependency_graph(&answers, "");
    assert!(output.status.success());
    assert_eq!(github_output, "enabled=false\nstate=unknown\n");
    assert_eq!(calls, 3);
    let text = stdout(&output);
    assert!(text.contains("::warning::Could not tell whether"), "{text}");
    assert!(!text.contains("is disabled"), "{text}");

    let (output, _, _) = dependency_graph(&answers, "--require");
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("::error::Could not tell whether"));
}
