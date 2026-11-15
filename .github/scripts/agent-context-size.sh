#!/usr/bin/env bash
# Prints how many bytes of agent context a commit puts in front of every agent:
# each AGENTS.md and CLAUDE.md in full, and the description of each skill and
# review agent (their bodies load only when used).
#
#   .github/scripts/agent-context-size.sh [ref]   (default HEAD)
set -euo pipefail

ref=${1:-HEAD}
files=$(git ls-tree -r --name-only "$ref")

{
	grep -E '(^|/)(AGENTS|CLAUDE)\.md$' <<<"$files" | grep -v node_modules |
		while read -r f; do git show "$ref:$f"; done
	grep -E '(^|/)skills/[^/]+/SKILL\.md$|^\.claude/agents/[^/]+\.md$' <<<"$files" | grep -v node_modules |
		while read -r f; do git show "$ref:$f" | grep -m1 '^description:' || true; done
} | wc -c | tr -d ' '
