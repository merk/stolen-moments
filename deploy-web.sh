#!/usr/bin/env bash
# Build, then publish ./dist to the `gh-pages` branch. No CI required.
set -euo pipefail

BRANCH="gh-pages"
OUT="dist"

./build-web.sh

echo "==> Publishing $OUT/ to '$BRANCH'"
# Use a throwaway worktree so your working tree / current branch stay untouched.
WORKTREE="$(mktemp -d)"
trap 'git worktree remove --force "$WORKTREE" 2>/dev/null || true' EXIT

if git show-ref --quiet "refs/heads/$BRANCH"; then
  git worktree add --force "$WORKTREE" "$BRANCH"
else
  git worktree add --force "$WORKTREE" --detach
  git -C "$WORKTREE" checkout --orphan "$BRANCH"
  git -C "$WORKTREE" rm -rf . >/dev/null 2>&1 || true
fi

# Replace contents with the fresh bundle.
find "$WORKTREE" -mindepth 1 -maxdepth 1 ! -name '.git' -exec rm -rf {} +
cp -R "$OUT"/. "$WORKTREE"/

git -C "$WORKTREE" add -A
if git -C "$WORKTREE" diff --cached --quiet; then
  echo "==> No changes to publish"
else
  git -C "$WORKTREE" commit -m "Deploy $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  git -C "$WORKTREE" push origin "$BRANCH"
  echo "==> Pushed. Set Pages source to branch '$BRANCH' (root) in repo settings."
fi
