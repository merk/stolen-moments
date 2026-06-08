#!/usr/bin/env bash
# Stop hook: the real validation gate. Runs when Claude finishes its turn, i.e.
# once any multi-file change is complete and the crate should compile as a whole.
#
# Runs clippy (which also type-checks) with warnings denied. On failure it exits
# 2, which blocks the stop and feeds the diagnostics back so Claude fixes them
# before handing control back to you. It loops until the tree is clean.
#
# Because it gates on Stop rather than on every edit, an in-progress refactor
# never gets interrupted by errors about callers that simply haven't been
# updated yet.
set -uo pipefail

cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0

# Only gate when there's actually a crate here (skip on the gh-pages deploy
# branch, which has no Cargo.toml).
[ -f Cargo.toml ] || exit 0

if ! diagnostics=$(cargo clippy --all-targets --message-format short -- -D warnings 2>&1); then
  {
    echo "cargo clippy is failing — resolve before finishing:"
    echo "$diagnostics" | grep -E '^(error|warning|src/)' | head -40
  } >&2
  exit 2
fi

exit 0
