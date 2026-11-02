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
`pycharmc`). Change the first keyword in `[products.<id>]` and rerun
`jb install` to customize it.

## Commands

```
jb search [--product <id>] [--filter <text>] [--show-stale] [--worktrees]
          [--sort recent|oldest|name|name-desc|path]
jb open [path] [--product <id>] [--app <app>] [--bin <bin>]
jb <path>                     # shorthand for `jb open <path>`
jb terminal [path]            # open a directory in the configured terminal
jb doctor                     # print discovery diagnostics
jb forget <path>              # hide a project
jb forget --clear             # clear the forgotten list
jb remember <path>            # restore a forgotten project
jb forgotten                  # list forgotten projects
jb pin <path>
jb unpin <path>
jb toggle-pin <path>
jb pins
jb config
jb install [--workflow-name <name>] [--retain]
jb version
```

A leading product token in `--filter` is consumed when a query follows it, so
`jb idea pangolin` scopes to IntelliJ IDEA; a bare keyword such as `go` is
treated as a normal search term. `jb config` prints the effective configuration.

`jb open` defaults to the current directory, so after cloning a repository:

```shell
cd my-new-repo
jb .
```

The IDE is inferred from project files:

| Marker | Product |
|---|---|
| `pom.xml`, `build.gradle[.kts]`, `settings.gradle[.kts]` | IntelliJ IDEA |
| `pyproject.toml`, `requirements.txt`, `setup.py`, `Pipfile` | PyCharm |
| `package.json` | WebStorm |
| `go.mod` | GoLand |
| `Cargo.toml` | RustRover |
| `composer.json` | PhpStorm |
| `Gemfile` | RubyMine |
| `CMakeLists.txt` | CLion |
| `*.sln`, `*.csproj` | Rider |

If nothing matches, `default_product` (default `idea`) is used. Override with
`--product`. The same inference backs the workflow's **Open in JetBrains** File
Action, so you can select any folder in Alfred or Finder.

## Configuration

### Alfred configuration page

Alfred Preferences → Workflows → JetBrains Projects (Rust) → Configure
Workflow exposes `jb_terminal`, `jb_default_product`, `jb_show_stale`,
`jb_hide_worktrees`, `jb_sort`, `jb_ignore`, `jb_settings_roots`, and
`jb_app_roots`. Alfred passes these as environment variables to the CLI;

- `jb_terminal`, `jb_default_product`, `jb_settings_roots`, `jb_app_roots`
  override the TOML values,
- `jb_show_stale` / `jb_hide_worktrees` are parsed as booleans,
- `jb_ignore` lines are appended to the TOML `ignore` list.

### TOML config

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
sort = "recent"
cache_seconds = 0   # 0 disables Alfred result caching
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

Pins are stored as a JSON array of absolute paths in `pins_file`.

## Modifiers

- `⌘` pin/unpin
- `⌥` reveal in Finder
- `⌃` copy the project path
- `⇧` open the project in `terminal`
- `⌘⇧` forget the project

## Ignoring and forgetting

- `ignore` config globs are matched case-insensitively against the project name
  and full path, e.g. `["~/Downloads/*", "*-scratch"]`.
- `jb forget <path>` hides one project; `jb remember <path>` restores it, and
  `jb forget --clear` empties the forgotten list.
- Ignored and forgotten projects stay hidden even when pinned.

The Alfred item exposes `⌘⇧` for `jb forget`; run `jb forgotten` or
`jb remember` from the terminal to restore.

## Diagnostics

```shell
jb doctor
```

Prints settings roots and product/version directories, matched application
bundles, project/worktree/stale/pin counts, the terminal setting, and the
installed workflow status.

## Search

Filtering uses a case-insensitive subsequence match, so `pngln` finds
`pangolin`. Whitespace-separated terms must all match, so `fory java` finds
`fory-java` and `github agentscope` can match path components. Version queries
such as `2026.2` match the IDE version, and `~/Code` matches a home-relative
path. When a query is present, matches are ordered by score; otherwise the
configured `sort` order applies:

- `recent` (default) — most recently opened first
- `oldest` — least recently opened first
- `name` / `name-desc` — project name A–Z / Z–A
- `path` — full path

Pinned projects always come first. Set the order with `--sort`, `sort` in the
TOML config, or the **Sort order** field on the Alfred configuration page.

## Git worktrees

`jb search --worktrees` lists linked worktrees instead of recent projects.
The workflow exposes this as `jb~` and `<product>~` (`idea~`, `pycharm~`, …).
Each result shows the worktree branch in its subtitle.

Worktrees are derived from the repositories of recent projects by reading
`<repo>/.git/worktrees/*/gitdir`; this does not run `git` or scan unrelated
directories. The normal project list hides linked worktrees by default; set
`hide_worktrees = false` to keep them in the main list as well.

## How discovery works

1. Every configured settings root is scanned for product-version directories
   (`IntelliJIdea2026.2`, `PyCharm2026.2`, `AndroidStudio2026.2`, `Fleet`, …).
2. `recentProjects.xml`, `recentProjectDirectories.xml`, `recentSolutions.xml`,
   and Fleet `backend/**/trusted-paths.xml` are merged per settings directory.
3. Entries carry `projectOpenTimestamp`/`activationTimestamp`, so the same
   project opened in multiple products or versions keeps its most recent IDE.
4. Missing project directories are hidden unless `--show-stale` (or
   `show_stale = true`) is used; pinned projects are always shown. The current
   git branch is shown for projects and worktrees. Projects matching `ignore`
   globs or in the forgotten list are hidden.
5. The IDE application is matched by product name and, when present, by the
   settings version. The app's `Contents/MacOS` binary is preferred, then
   `JB_BINARIES`/`PATH`.
