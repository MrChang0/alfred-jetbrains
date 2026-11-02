mod alfred;
mod config;
mod discovery;
mod doctor;
mod forgotten;
mod hash;
mod install;
mod logging;
mod open;
mod pins;
mod product;
mod projects;
mod search;
mod worktree;
mod xml;

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::{config::Config, install::InstallOptions, open::OpenOptions, search::SearchOptions};

#[derive(Parser)]
#[command(name = "jb", version, about = "Open JetBrains projects from Alfred")]
struct Cli {
    /// Enable debug logging on stderr.
    #[arg(long, global = true)]
    debug: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print Alfred Script Filter JSON for recent projects.
    Search {
        /// Scope to a product id or keyword, e.g. idea or pycharm.
        #[arg(long)]
        product: Option<String>,

        /// Filter text. A leading product keyword is consumed.
        #[arg(long, default_value = "")]
        filter: String,

        /// Include projects whose directory no longer exists.
        #[arg(long)]
        show_stale: bool,

        /// Show linked git worktrees instead of recent projects.
        #[arg(long)]
        worktrees: bool,

        /// Sort order: recent, oldest, name, name-desc, path.
        #[arg(long)]
        sort: Option<String>,
    },

    /// Open a project in the IDE that last used it.
    Open {
        /// Project directory (or file) to open; defaults to the current directory.
        #[arg(default_value = ".")]
        path: PathBuf,

        #[arg(long)]
        product: Option<String>,

        #[arg(long)]
        app: Option<PathBuf>,

        #[arg(long)]
        bin: Option<PathBuf>,
    },

    /// Open a project directory in the configured terminal.
    Terminal {
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Print diagnostics about settings, apps, and projects.
    Doctor,

    /// Hide a project from search results.
    Forget {
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,

        /// Clear the forgotten list.
        #[arg(long)]
        clear: bool,
    },

    /// Restore a forgotten project.
    Remember { path: PathBuf },

    /// List forgotten projects.
    Forgotten,

    /// Pin a project.
    Pin { path: PathBuf },

    /// Unpin a project.
    Unpin { path: PathBuf },

    /// Toggle a project's pin.
    TogglePin { path: PathBuf },

    /// List pinned projects.
    Pins,

    /// Print the effective configuration.
    Config,

    /// Install the Alfred workflow.
    Install {
        #[arg(long)]
        workflow_name: Option<String>,

        /// Do not copy the running binary into the workflow.
        #[arg(long)]
        retain: bool,
    },

    /// Print the version.
    Version,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("jb: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse_from(normalized_args());
    let debug =
        cli.debug || env::var_os("JB_DEBUG").is_some() || env::var_os("alfred_debug").is_some();
    logging::init(debug);

    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            if matches!(cli.command, Commands::Search { .. }) {
                let response = alfred::error_response(60, &format!("{error:#}"));
                println!("{}", serde_json::to_string(&response)?);
                return Ok(());
            }
            return Err(error);
        }
    };

    match cli.command {
        Commands::Search {
            product,
            filter,
            show_stale,
            worktrees,
            sort,
        } => {
            let options = SearchOptions {
                product,
                filter,
                show_stale,
                worktrees,
                sort,
            };
            let response = match search::search(&config, options) {
                Ok(response) => response,
                Err(error) => alfred::error_response(config.cache_seconds, &format!("{error:#}")),
            };
            println!("{}", serde_json::to_string(&response)?);
        }
        Commands::Open {
            path,
            product,
            app,
            bin,
        } => {
            open::open(
                &config,
                OpenOptions {
                    path,
                    product,
                    app,
                    bin,
                },
            )?;
        }
        Commands::Terminal { path } => {
            open::open_terminal(&config, &path)?;
        }
        Commands::Doctor => {
            println!("{}", doctor::run(&config)?);
        }
        Commands::Forget { path, clear } => {
            if clear {
                forgotten::clear(&config.forgotten_file)?;
                println!("forgotten projects cleared");
            } else if let Some(path) = path {
                let added = forgotten::forget(&config.forgotten_file, &path)?;
                println!(
                    "{}",
                    if added {
                        "forgotten"
                    } else {
                        "already forgotten"
                    }
                );
            } else {
                anyhow::bail!("provide a path or --clear");
            }
        }
        Commands::Remember { path } => {
            let removed = forgotten::remember(&config.forgotten_file, &path)?;
            println!(
                "{}",
                if removed {
                    "remembered"
                } else {
                    "not forgotten"
                }
            );
        }
        Commands::Forgotten => {
            for path in pins::load(&config.forgotten_file)? {
                println!("{path}");
            }
        }
        Commands::Pin { path } => {
            let added = pins::pin(&config.pins_file, &path)?;
            println!("{}", if added { "pinned" } else { "already pinned" });
        }
        Commands::Unpin { path } => {
            let removed = pins::unpin(&config.pins_file, &path)?;
            println!("{}", if removed { "unpinned" } else { "not pinned" });
        }
        Commands::TogglePin { path } => {
            let pinned = pins::toggle(&config.pins_file, &path)?;
            println!("{}", if pinned { "pinned" } else { "unpinned" });
        }
        Commands::Pins => {
            for path in pins::load(&config.pins_file)? {
                println!("{path}");
            }
        }
        Commands::Config => {
            println!("{}", config.effective_json()?);
        }
        Commands::Install {
            workflow_name,
            retain,
        } => {
            let workflow = install::install(
                &config,
                InstallOptions {
                    workflow_name,
                    retain,
                },
            )?;
            println!("Installed workflow: {}", workflow.display());
        }
        Commands::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
        }
    }

    Ok(())
}

/// Allow `jb .` / `jb /path/to/project` as a shortcut for `jb open ...`.
/// Global flags may precede the path (`jb --debug .`), and arguments are kept
/// as `OsString` so non-UTF-8 paths do not panic.
fn normalized_args() -> Vec<OsString> {
    let mut args: Vec<OsString> = env::args_os().collect();
    let Some(index) = args
        .iter()
        .skip(1)
        .position(|arg| !arg.to_string_lossy().starts_with('-'))
        .map(|position| position + 1)
    else {
        return args;
    };

    let candidate = &args[index];
    let subcommand = candidate.to_str().is_some_and(is_subcommand);
    if !subcommand && is_path_like(Path::new(candidate)) {
        args.insert(index, OsString::from("open"));
    }
    args
}

fn is_path_like(path: &Path) -> bool {
    let value = path.as_os_str().to_string_lossy();
    value == "."
        || value == ".."
        || value.starts_with('/')
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with("~/")
        || path.exists()
}

fn is_subcommand(value: &str) -> bool {
    matches!(
        value,
        "search"
            | "open"
            | "terminal"
            | "doctor"
            | "forget"
            | "remember"
            | "forgotten"
            | "pin"
            | "unpin"
            | "toggle-pin"
            | "pins"
            | "config"
            | "install"
            | "version"
            | "help"
    )
}
