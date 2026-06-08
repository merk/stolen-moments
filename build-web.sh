#!/usr/bin/env bash
# Build the wasm bundle into ./dist, ready to serve or deploy to GitHub Pages.
set -euo pipefail

NAME="game-time"
OUT="dist"

echo "==> Compiling release wasm"
cargo build --release --target wasm32-unknown-unknown

echo "==> Generating JS bindings"
rm -rf "$OUT"
mkdir -p "$OUT"
wasm-bindgen \
  --no-typescript \
  --target web \
  --out-dir "$OUT" \
  --out-name "$NAME" \
  "target/wasm32-unknown-unknown/release/${NAME}.wasm"

# Shrink the wasm if wasm-opt is available (install via `brew install binaryen`).
if command -v wasm-opt >/dev/null 2>&1; then
  echo "==> Optimising wasm (wasm-opt -Oz)"
  # Current rustc/LLVM emit post-MVP ops (bulk-memory, trunc_sat, sign-ext, …)
  # by default; wasm-opt rejects the input unless those features are allowed.
  wasm-opt -Oz \
    --enable-bulk-memory \
    --enable-nontrapping-float-to-int \
    --enable-sign-ext \
    --enable-mutable-globals \
    --enable-reference-types \
    --enable-multivalue \
    -o "$OUT/${NAME}_bg.wasm" "$OUT/${NAME}_bg.wasm"
else
  echo "==> Skipping wasm-opt (not installed)"
fi

echo "==> Copying shell + assets"
cp web/index.html "$OUT/index.html"
# Disable Jekyll so files/dirs are served verbatim (e.g. underscores, spaces).
touch "$OUT/.nojekyll"
# Ship only what the game loads at runtime (GLBs embed their textures).
mkdir -p "$OUT/assets/Models"
cp -R "assets/Models/GLB format" "$OUT/assets/Models/"

echo "==> Done. Bundle in ./$OUT"
du -sh "$OUT"
echo "Test locally with:  python3 -m http.server -d $OUT 8080"
