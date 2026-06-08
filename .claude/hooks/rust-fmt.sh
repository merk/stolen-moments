#!/usr/bin/env bash
# PostToolUse hook: format Rust after an edit. Runs after EVERY edit, so it must
# stay cheap and must NEVER block — during a multi-file change the crate is
# routinely in a half-finished, non-compiling state, and that's fine here.
#
# rustfmt works per-file: a broken caller in another file doesn't stop the file
# we just touched from being formatted, and any file it can't parse is simply
# skipped. The heavyweight lint/type-check lives in the Stop gate (rust-gate.sh),
# which only runs once the whole change is coherent.
set -uo pipefail

payload=$(cat)
file=$(printf '%s' "$payload" | jq -r '.tool_input.file_path // empty')

case "$file" in
  *.rs) ;;
  *) exit 0 ;;
esac

cd "${CLAUDE_PROJECT_DIR:-.}" || exit 0

# Format the crate; ignore parse errors from mid-refactor files. Never blocks.
cargo fmt --quiet 2>/dev/null
exit 0
