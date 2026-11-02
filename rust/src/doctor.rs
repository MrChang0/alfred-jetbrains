//! `jb doctor`: print what the discovery engine sees.

use std::{collections::HashSet, fmt::Write as _, fs, path::Path};

use anyhow::Result;

use crate::{config, config::Config, discovery, forgotten, pins};

pub fn run(config: &Config) -> Result<String> {
    let mut out = String::new();
    writeln!(out, "jb {}", env!("CARGO_PKG_VERSION"))?;
    writeln!(out, "config: {}", config.config_path.display())?;

    writeln!(out, "settings roots:")?;
    for root in &config.settings_roots {
        writeln!(
            out,
            "  {}  ({})",
            root.display(),
            if root.is_dir() { "ok" } else { "missing" }
        )?;
    }

    let settings = discovery::settings_dirs(config);
    writeln!(out, "settings dirs: {}", settings.len())?;
    for (product, path, version) in &settings {
        writeln!(
            out,
            "  {} {}  {}",
            product.name,
            version.as_deref().unwrap_or(""),
            path.display()
        )?;
    }

    let apps = discovery::product_apps(config);
    writeln!(out, "apps: {}", apps.len())?;
    for (name, path) in &apps {
        writeln!(out, "  {name}: {}", path.display())?;
    }

    let entries = discovery::discover(config);
    let unique = entries
        .iter()
        .map(|entry| &entry.key)
        .collect::<HashSet<_>>()
        .len();
    let stale = entries.iter().filter(|entry| entry.stale).count();
    let worktrees = discovery::worktree_entries(&entries).len();
    let pin_keys = pins::load_keys(&config.pins_file).unwrap_or_default();
    writeln!(
        out,
        "projects: {unique} unique, {} entries, {stale} stale, {worktrees} worktrees",
        entries.len()
    )?;
    writeln!(
        out,
        "pins: {} ({})",
        config.pins_file.display(),
        pin_keys.len()
    )?;
    let forgotten_keys = forgotten::load_keys(&config.forgotten_file).unwrap_or_default();
    writeln!(out, "ignore patterns: {}", config.ignore.len())?;
    writeln!(
        out,
        "forgotten: {} ({})",
        config.forgotten_file.display(),
        forgotten_keys.len()
    )?;
    writeln!(out, "default product: {}", config.default_product)?;
    writeln!(out, "sort: {}", config.sort)?;
    writeln!(out, "terminal: {}", config.terminal)?;
    writeln!(out, "hide worktrees: {}", config.hide_worktrees)?;

    let home = config::home_dir()?;
    let prefs_path = home.join("Library/Application Support/Alfred/prefs.json");
    if prefs_path.is_file() {
        let text = fs::read_to_string(&prefs_path)?;
        let prefs: serde_json::Value = serde_json::from_str(&text)?;
        if let Some(current) = prefs.get("current").and_then(|value| value.as_str()) {
            let workflow = Path::new(current)
                .join("workflows")
                .join(&config.workflow_name);
            let installed = workflow.join("info.plist").is_file();
            let binary = workflow.join("bin/jb");
            writeln!(
                out,
                "workflow: {} ({})",
                workflow.display(),
                if installed {
                    "installed"
                } else {
                    "not installed"
                }
            )?;
            writeln!(
                out,
                "workflow binary: {} ({})",
                binary.display(),
                if binary.is_file() {
                    "present"
                } else {
                    "missing"
                }
            )?;
        } else {
            writeln!(out, "workflow: Alfred prefs has no current path")?;
        }
    } else {
        writeln!(out, "workflow: Alfred prefs not found")?;
    }

    Ok(out)
}
