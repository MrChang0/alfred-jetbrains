# JetBrains: Open project (Rust)

An Alfred 5 workflow that opens your recent JetBrains projects across every
installed IDE and every installed version.

## Features

- **All products, all versions** — scans every JetBrains settings directory and
  merges `recentProjects.xml`, `recentProjectDirectories.xml`,
  `recentSolutions.xml`, and Fleet `trusted-paths.xml`
- **Recency order** — uses the project-open timestamp, so the same project
  opened in several IDEs or versions resolves to the most recent one
- **Stale filtering** — hides projects whose directories no longer exist
  (`--show-stale` to include them); pinned projects are always shown
- **Pins** — favorite projects stay at the top with `★`
- **Correct IDE/version** — opens the project in the IDE that last used it
- **Product scoping** — `jb idea pangolin`, `jb pycharm foo`, `jb webstorm bar`
- **Open anything** — `jb .` after `cd` infers the IDE from project files; the
  Alfred **Open in JetBrains** File Action works on any folder
- **Git worktrees** — `jb~` / `idea~` list linked worktrees with their branches;
  they are hidden from the normal project list
- **Git branch** — shown in the subtitle for every git checkout
- **Ignore / forget** — hide projects with config globs or `jb forget`;
  `jb remember` restores them
- **Terminal & diagnostics** — `⇧` opens a terminal; `jb doctor` prints what
  the CLI sees
- **Alfred modifiers** — `⌘` pin/unpin, `⌥` reveal in Finder, `⌃` copy path,
  `⇧` open in Terminal, `⌘⇧` forget

## Requirements

- macOS
- [Alfred 5](https://www.alfredapp.com) with the Powerpack
- One or more JetBrains IDEs
- Rust toolchain (to build from source)

## Build and install

```shell
./tool/build_jb.sh    # builds bin/jb
./bin/jb install      # installs the "JetBrains Projects (Rust)" workflow
```

The workflow registers the `jb` keyword.

## Usage

Type `jb` in Alfred to search all recent projects, or use a product keyword
directly:

- `idea <name>` — IntelliJ IDEA (Ultimate)
- `pycharm <name>` — PyCharm (Professional)
- `pstorm <name>` — PhpStorm
- `wstorm <name>` — WebStorm
- `studio <name>` — Android Studio
- `clion`, `goland`, `datagrip`, `dataspell`, `rider`, `rubymine`,
  `rustrover`, `fleet`, `aqua`, `appcode`, `writerside`
- Community editions use `ideac` and `pycharmc`

`jb idea <name>` also works if you prefer the global keyword.

Add `~` to list linked git worktrees instead of recent projects:

- `jb~` — all worktrees
- `idea~`, `pycharm~`, … — worktrees of one product

Worktrees are discovered from your recent project repositories by reading
Git's own `.git/worktrees/*/gitdir` files; no directory scanning and no `git`
process. The normal list hides linked worktrees (set `hide_worktrees = false`
to include them).

### Open a fresh clone

```shell
git clone <repo>
cd <repo>
jb .
```

`jb .` / `jb open .` infers the IDE from project files: `pom.xml` or Gradle →
IntelliJ IDEA, `pyproject.toml` → PyCharm, `package.json` → WebStorm, `go.mod`
→ GoLand, `Cargo.toml` → RustRover, `*.sln`/`*.csproj` → Rider,
`composer.json` → PhpStorm, `Gemfile` → RubyMine, `CMakeLists.txt` → CLion.
Use `--product` to override, or set `default_product` in the config.

In Alfred or Finder, select any folder and run the **Open in JetBrains** File
Action for the same behavior.

Hold `⌘` to pin/unpin, `⌥` to reveal in Finder, `⌃` to copy the path, `⇧` to
open the project in your terminal, and `⌘⇧` to forget it.

### Hide projects

Use config globs for broad rules:

```toml
ignore = ["~/Downloads/*", "*-scratch"]
```

Or forget a single project from the CLI or Alfred (`⌘⇧`):

```shell
jb forget <path>
jb forgotten
jb remember <path>
jb forget --clear
```

Ignored and forgotten projects are hidden even when pinned.

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
hide_worktrees = true
cache_seconds = 60
pins_file = "~/.config/alfred-jetbrains/pins.json"
workflow_name = "alfred-jetbrains-rust"
default_product = "idea"
terminal = "Terminal"
ignore = ["~/Downloads/*", "*-scratch"]
forgotten_file = "~/.config/alfred-jetbrains/forgotten.json"

[products.idea]
preference_prefix = "IntelliJIdea"
application_names = ["IntelliJ IDEA"]
binaries = ["idea"]
keywords = ["idea", "ij"]
```

`jb config` prints the effective configuration. See
[`rust/README.md`](rust/README.md) for discovery details and the full command
list.

## Diagnostics

```shell
jb doctor
```

Prints the settings roots and version directories it found, the installed
JetBrains apps it matched, project/worktree/stale/pin counts, the terminal
setting, and whether the Alfred workflow is installed.

## Development

```shell
cd rust
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

## Universal binary

Rust binaries merge cleanly with `lipo`:

```shell
rustup target add aarch64-apple-darwin x86_64-apple-darwin
(cd rust && cargo build --release --target aarch64-apple-darwin)
(cd rust && cargo build --release --target x86_64-apple-darwin)
lipo -create \
  rust/target/aarch64-apple-darwin/release/jb \
  rust/target/x86_64-apple-darwin/release/jb \
  -output bin/jb
./bin/jb install
```

## License

MIT © [bchatard](https://github.com/bchatard); Rust implementation © MrChang0.
