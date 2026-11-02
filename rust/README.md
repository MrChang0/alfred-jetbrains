# jb — Alfred JetBrains CLI (Rust)

A from-scratch Rust implementation of the Alfred workflow CLI. It scans every
JetBrains product and every installed version, merges recent projects, hides
projects whose directories no longer exist, and supports pinned projects.

## Build

```shell
./tool/build_jb.sh
```

This builds `bin/jb` for the host architecture (Apple Silicon). For a universal
binary:

```shell
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin   # from rust/
cargo build --release --target x86_64-apple-darwin
lipo -create \
  target/aarch64-apple-darwin/release/jb \
  target/x86_64-apple-darwin/release/jb \
  -output ../bin/jb
```

## Test

```shell
cd rust
cargo test
```

The suite covers XML extraction, multi-version discovery, stale filtering,
pinning, product scoping, Alfred JSON, and workflow installation.

## Install

```shell
./bin/jb install
```

This writes a new Alfred workflow named **JetBrains Projects (Rust)**,
`com.mrchang0.alfred-jetbrains-rust`, into Alfred's current preferences
directory and copies the running binary into its `bin/`.

The workflow registers `jb` plus one keyword per product, using the first entry
in each product's `keywords` list (`idea`, `pycharm`, `pstorm`, `wstorm`,
`studio`, `clion`, `goland`, `datagrip`, `dataspell`, `rider`, `rubymine`,
`rustrover`, `fleet`, `aqua`, `appcode`, `writerside`, `nova`, `air`,
`ideac`, `pycharmc`). Change the first keyword in `[products.<id>]` and rerun
`jb install` to customize it.

## Commands

```
jb search [--product <id>] [--filter <text>] [--show-stale]
jb open <path> [--product <id>] [--app <app>] [--bin <bin>]
jb pin <path>
jb unpin <path>
jb toggle-pin <path>
jb pins
jb config
jb install [--workflow-name <name>] [--retain]
jb version
```

A leading product token in `--filter` is consumed, so `jb idea pangolin` scopes
to IntelliJ IDEA. `jb config` prints the effective configuration.

## Configuration

The config file is `~/.config/alfred-jetbrains/config.toml` (override with
`JB_CONFIG`):

```toml
settings_roots = [
  "~/Library/Application Support/JetBrains",
  "~/Library/Application Support/Google",
  "~/Library/Preferences",
]
app_roots = [
  "/Applications",
  "~/Applications",
  "~/Applications/JetBrains Toolbox",
]
show_stale = false
cache_seconds = 60
pins_file = "~/.config/alfred-jetbrains/pins.json"
workflow_name = "alfred-jetbrains-rust"

[products.idea]
preference_prefix = "IntelliJIdea"
application_names = ["IntelliJ IDEA"]
binaries = ["idea"]
keywords = ["idea", "ij"]
```

Pins are stored as a JSON array of absolute paths in `pins_file`.

## How discovery works

1. Every configured settings root is scanned for product-version directories
   (`IntelliJIdea2026.2`, `PyCharm2026.2`, `AndroidStudio2024.3`, `Fleet`, …).
2. `recentProjects.xml`, `recentProjectDirectories.xml`, `recentSolutions.xml`,
   and Fleet `backend/**/trusted-paths.xml` are merged per settings directory.
3. Entries carry `projectOpenTimestamp`/`activationTimestamp`, so the same
   project opened in multiple products or versions keeps its most recent IDE.
4. Missing project directories are hidden unless `--show-stale` (or
   `show_stale = true`) is used; pinned projects are always shown.
5. The IDE application is matched by product name and, when present, by the
   settings version. The app's `Contents/MacOS` binary is preferred, then
   `JB_BINARIES`/`PATH`.
