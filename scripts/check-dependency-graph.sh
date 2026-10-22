#!/usr/bin/env bash
# Reports whether the repository's dependency graph is enabled, which
# actions/dependency-review-action needs. Enabling it is a repository setting
# that only an administrator can change, so pull requests skip the review
# with a warning while pushes to main fail with instructions.
#
# Usage: check-dependency-graph.sh <base-sha> <head-sha> [--require]
# Needs GH_TOKEN and REPO; writes `enabled=true|false` to GITHUB_OUTPUT.
set -euo pipefail

BASE="${1:?base commit is required}"
HEAD="${2:?head commit is required}"
MODE="${3:-}"
SETTINGS="https://github.com/${REPO:?REPO is required}/settings/security_analysis"

if response="$(gh api "repos/$REPO/dependency-graph/compare/$BASE...$HEAD" 2>&1)"; then
  enabled=true
else
  enabled=false
fi
echo "enabled=$enabled" >> "${GITHUB_OUTPUT:-/dev/null}"
[ "$enabled" = true ] && { echo "The dependency graph is enabled."; exit 0; }

message="The dependency graph of $REPO is disabled, so dependency review cannot run. \
A repository administrator has to enable \"Dependency graph\" at $SETTINGS."
echo "GitHub answered: ${response:0:300}"
if [ "$MODE" = "--require" ]; then
  echo "::error::$message"
  exit 1
fi
echo "::warning::$message Skipping dependency review for this pull request."
