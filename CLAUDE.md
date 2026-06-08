# game-time

A small Bevy game: a procedurally-generated noise dungeon with a time-loop
mechanic. The player explores, collects coins, and is hunted by adversaries;
each loop reset replays the player's past runs as "ghosts". Targets native
desktop and the web (WebGL2 / wasm), deployed to GitHub Pages.

- **Engine:** Bevy `0.18` (features: `webgl2`)
- **Edition:** Rust 2024
- **Toolchain:** nightly (rustfmt + clippy from the same nightly)
- **Other deps:** `noise` (dungeon generation), `rand` 0.8 (`small_rng`)

## Architecture

`src/main.rs` builds the `App`, sets up `DefaultPlugins` (nearest-neighbour
sampling for the pixel-art look), global ambient + a key directional light, and
registers one plugin per gameplay system. Each module owns a `Plugin` and its
components/systems:

| Module | Plugin | Responsibility |
|---|---|---|
| `dungeon.rs` | `DungeonPlugin` | Noise-based map gen, `DungeonMap` + tile↔world helpers, `SpawnPoint` |
| `camera.rs` | `IsoCameraPlugin` | Isometric orthographic camera that follows the player |
| `player.rs` | `PlayerPlugin` | Player spawn + movement |
| `props.rs` | `PropsPlugin` | Scatters barrels/rocks/chests/coins onto floor tiles |
| `coins.rs` | `CoinsPlugin` | Coin pickup, `CoinScore`, HUD |
| `time_loop.rs` | `TimeLoopPlugin` | Records runs, replays them as `Ghost`s, emits `LoopReset` |
| `adversary.rs` | `AdversaryPlugin` | Vision-cone patrol/chase AI, grid pathfinding |
| `wasm_compat.rs` | — | Web-only shims |

Plugins that depend on the generated map run their setup in `PostStartup` so
`DungeonMap`/`SpawnPoint` already exist. `LoopReset` is an event/observer that
plugins hook to reset their state on each loop.

## Commands

```bash
cargo run                 # native dev build (fast compile, opt-level 1)
cargo check               # quick type-check
cargo clippy --all-targets -- -D warnings   # lint (the Stop hook runs this)
cargo fmt                 # format (the edit hook runs this for you)
./build-web.sh            # release wasm → ./dist (wasm-bindgen, optional wasm-opt)
./deploy-web.sh           # publish ./dist to GitHub Pages
```

Web requires `wasm32-unknown-unknown`, `wasm-bindgen-cli`, and optionally
`binaryen` (`wasm-opt`) for size. `.cargo/config.toml` sets `wasm-server-runner`
as the wasm runner, so `cargo run --target wasm32-unknown-unknown` serves in a
browser.

The repo is also checked out on a `gh-pages`/deploy branch that contains only
the built bundle (no `Cargo.toml`/`src`). Source work happens on `main`.

## Automated formatting & linting (hooks)

`.claude/settings.json` wires two hooks so changes stay green without manual
steps:

- **PostToolUse** (`rust-fmt.sh`) — after every edit to a `.rs` file, runs
  `cargo fmt`. Cheap, per-file, and **never blocks**: during a multi-file
  refactor the crate is often temporarily non-compiling, and that's expected.
- **Stop** (`rust-gate.sh`) — when a turn finishes (the change is now coherent),
  runs `cargo clippy --all-targets -- -D warnings`. If it fails it blocks the
  stop and feeds the diagnostics back to be fixed. This is the real validation
  gate; keep the tree warning-clean.

If you add code that legitimately trips a pedantic lint, prefer a scoped
`#[allow(...)]` with a comment over leaving a warning. Bevy's query types trip
`clippy::type_complexity`, which is why it's allowed crate-wide in `main.rs`.

## Committing

After completing a coherent round of changes — and once the clippy Stop-gate
passes (warning-clean) — create a commit with a concise, descriptive message
summarising the change. A "round" is a self-contained unit of work, not every
turn; don't commit a half-finished or non-compiling tree. Committing directly to
`main` is fine. Never commit to the `gh-pages` deploy branch.

## Bevy 0.18 + edition 2024 gotchas

These cost build cycles before — check installed Bevy source under
`~/.cargo/registry/src/**` (allow-listed for reading) when an API is uncertain
rather than guessing from older docs.

- `ScalingMode` lives at `bevy::camera::ScalingMode` (not `bevy::render::camera`).
- Global ambient light is the **resource** `GlobalAmbientLight`
  (`insert_resource`). `AmbientLight` is now a per-camera **Component**, not a
  resource. Both are in the prelude.
- Edition 2024 reserves `gen` as a keyword, so rand 0.8's bare `.gen()` won't
  parse — use `rand::random()` or `r#gen()`. (`gen_range` is fine.)
- Orthographic 3D camera:
  `Projection::from(OrthographicProjection { scaling_mode: ScalingMode::FixedVertical { viewport_height }, ..OrthographicProjection::default_3d() })`.
- GLTF scene load:
  `asset_server.load(GltfAssetLabel::Scene(0).from_asset("path.glb"))` paired
  with `SceneRoot(handle)`.

## Assets (Kenney mini-dungeon)

Models live under `assets/Models/GLB format/` (Kenney mini-dungeon pack).

- Each `.glb` gets its colour from a shared atlas `Textures/colormap.png`
  referenced by **external URI** (textures are not embedded); Bevy resolves it
  relative to the `.glb`. Models rendering white/untextured means the URI didn't
  resolve. (The web build copies the GLB folder; confirm the atlas ships too.)
- Each surface samples a **single texel** of the atlas → flat single-colour
  faces. The "basic polygons" look is intentional, not a bug.
- Tile footprint is 1 unit. `floor.glb` is a flat 1×1 quad at y=0; `wall.glb`
  fills the tile (−0.5..0.5 X/Z), is 1.1 tall with base at y=0.
- `character-human.glb` is a static mesh (no rig/animation) — movement
  translates/rotates it; there's no walk cycle. Its **front faces +Z**, so face
  a movement direction with `looking_to(-move_dir, Vec3::Y)` (Bevy's
  `looking_to` aligns −Z).
