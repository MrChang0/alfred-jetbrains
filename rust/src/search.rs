//! Project search, filtering, and Alfred response assembly.

use anyhow::{Result, anyhow};
use glob::Pattern;

use crate::{
    alfred::{self, AlfredResponse},
    config::Config,
    discovery::{self, ProjectEntry},
    forgotten, logging, pins,
};

pub struct SearchOptions {
    pub product: Option<String>,
    pub filter: String,
    pub show_stale: bool,
    pub worktrees: bool,
}

pub fn search(config: &Config, options: SearchOptions) -> Result<AlfredResponse> {
    let (product_id, filter) = resolve_product(config, options.product, options.filter)?;
    let pin_keys = match pins::load_keys(&config.pins_file) {
        Ok(keys) => keys,
        Err(error) => {
            logging::debug(format!("pins: {error:#}"));
            Default::default()
        }
    };

    let mut entries = discovery::discover(config);
    for entry in &mut entries {
        entry.pinned = pin_keys.contains(&entry.key);
    }

    if options.worktrees {
        entries = discovery::worktree_entries(&entries);
        for entry in &mut entries {
            entry.pinned = pin_keys.contains(&entry.key);
        }
    } else if config.hide_worktrees {
        entries.retain(|entry| !entry.is_worktree);
    }

    let forgotten_keys = match forgotten::load_keys(&config.forgotten_file) {
        Ok(keys) => keys,
        Err(error) => {
            logging::debug(format!("forgotten: {error:#}"));
            Default::default()
        }
    };
    let ignore_patterns = compile_ignore(&config.ignore);
    entries.retain(|entry| {
        !forgotten_keys.contains(&entry.key) && !is_ignored(entry, &ignore_patterns)
    });

    if let Some(product_id) = &product_id {
        entries.retain(|entry| &entry.product.id == product_id);
    }

    let show_stale = options.show_stale || config.show_stale;
    entries.retain(|entry| show_stale || !entry.stale || entry.pinned);

    let mut entries = discovery::dedupe(entries);

    let filter = filter.trim().to_lowercase();
    if !filter.is_empty() {
        entries.retain(|entry| matches_filter(entry, &filter));
    }

    entries.sort_by(|left, right| {
        right
            .pinned
            .cmp(&left.pinned)
            .then_with(|| {
                right
                    .open_timestamp
                    .unwrap_or(i64::MIN)
                    .cmp(&left.open_timestamp.unwrap_or(i64::MIN))
            })
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.path.cmp(&right.path))
    });

    if entries.is_empty() && filter.is_empty() {
        if options.worktrees {
            return Ok(alfred::info_response(
                config,
                "No git worktrees found",
                "Worktrees are discovered from your recent project repositories",
            ));
        }
        let subtitle = match &product_id {
            Some(product) => format!("No recent projects found for {product}"),
            None => format!(
                "Searched {}",
                config
                    .settings_roots
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        return Ok(alfred::info_response(
            config,
            "No JetBrains projects found",
            &subtitle,
        ));
    }

    Ok(alfred::response(config, &entries))
}

fn compile_ignore(patterns: &[String]) -> Vec<Pattern> {
    let home = std::env::var("HOME").unwrap_or_default();
    patterns
        .iter()
        .filter_map(|pattern| {
            let expanded = if let Some(rest) = pattern.strip_prefix("~/") {
                format!("{home}/{rest}")
            } else {
                pattern.clone()
            };
            match Pattern::new(&expanded.to_lowercase()) {
                Ok(pattern) => Some(pattern),
                Err(error) => {
                    logging::debug(format!("ignore pattern {pattern:?}: {error}"));
                    None
                }
            }
        })
        .collect()
}

fn is_ignored(entry: &ProjectEntry, patterns: &[Pattern]) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let path = entry.path.to_string_lossy().to_lowercase();
    let name = entry.name.to_lowercase();
    patterns
        .iter()
        .any(|pattern| pattern.matches(&path) || pattern.matches(&name))
}

fn resolve_product(
    config: &Config,
    product: Option<String>,
    filter: String,
) -> Result<(Option<String>, String)> {
    if let Some(value) = product {
        let product = config
            .product_by_keyword(&value)
            .or_else(|| config.product_by_id(&value))
            .ok_or_else(|| anyhow!("unknown product: {value}"))?;
        return Ok((Some(product.id.clone()), filter));
    }

    let mut parts = filter.splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or_default();
    let rest = parts.next().unwrap_or_default().trim_start();
    if let Some(product) = config.product_by_keyword(first) {
        return Ok((Some(product.id.clone()), rest.to_string()));
    }

    Ok((None, filter))
}

fn matches_filter(entry: &ProjectEntry, filter: &str) -> bool {
    entry.name.to_lowercase().contains(filter)
        || entry.path.to_string_lossy().to_lowercase().contains(filter)
        || entry.product.name.to_lowercase().contains(filter)
        || entry.product.id.to_lowercase().contains(filter)
        || entry
            .version
            .as_deref()
            .is_some_and(|version| version.to_lowercase().contains(filter))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use tempfile::tempdir;

    use super::*;
    use crate::{
        config::{Config, default_products},
        forgotten, pins,
    };

    fn test_config(settings: PathBuf, apps: PathBuf, pins_file: PathBuf) -> Config {
        Config {
            config_path: PathBuf::from("/dev/null"),
            settings_roots: vec![settings],
            app_roots: vec![apps],
            show_stale: false,
            hide_worktrees: true,
            cache_seconds: 60,
            pins_file,
            workflow_name: "test".to_string(),
            default_product: "idea".to_string(),
            terminal: "Terminal".to_string(),
            ignore: vec![],
            forgotten_file: PathBuf::from("/dev/null"),
            products: default_products(),
        }
    }

    fn write_recent(dir: &std::path::Path, entries: &[(&str, i64)]) {
        let options = dir.join("options");
        fs::create_dir_all(&options).unwrap();
        let mut xml = String::from(
            r#"<application><component name="RecentProjectsManager"><option name="additionalInfo"><map>"#,
        );
        for (path, timestamp) in entries {
            xml.push_str(&format!(
                r#"<entry key="{path}"><value><RecentProjectMetaInfo><option name="projectOpenTimestamp" value="{timestamp}" /></RecentProjectMetaInfo></value></entry>"#
            ));
        }
        xml.push_str("</map></option></component></application>");
        fs::write(options.join("recentProjects.xml"), xml).unwrap();
    }

    fn fixture() -> (tempfile::TempDir, Config) {
        let temp = tempdir().unwrap();
        let settings = temp.path().join("JetBrains");
        let apps = temp.path().join("Applications");
        let alpha = temp.path().join("projects/alpha");
        let beta = temp.path().join("projects/beta");
        let missing = temp.path().join("projects/missing");
        fs::create_dir_all(alpha.join(".idea")).unwrap();
        fs::write(alpha.join(".idea/.name"), "Alpha").unwrap();
        fs::create_dir_all(&beta).unwrap();
        write_recent(
            &settings.join("IntelliJIdea2026.2"),
            &[
                (alpha.to_str().unwrap(), 100),
                (beta.to_str().unwrap(), 200),
                (missing.to_str().unwrap(), 300),
            ],
        );
        let macos = apps.join("IntelliJ IDEA.app/Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("idea"), b"#!/bin/sh\n").unwrap();

        let config = test_config(settings, apps, temp.path().join("pins.json"));
        (temp, config)
    }

    fn worktree_fixture() -> (tempfile::TempDir, Config) {
        let temp = tempdir().unwrap();
        let settings = temp.path().join("JetBrains");
        let apps = temp.path().join("Applications");
        let main = temp.path().join("projects/main");
        fs::create_dir_all(&main).unwrap();
        fs::write(main.join("pom.xml"), "").unwrap();
        let common = main.join(".git");
        let worktree_git = common.join("worktrees/feature");
        fs::create_dir_all(&worktree_git).unwrap();
        fs::write(common.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let worktree = temp.path().join("projects/main-feature");
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", worktree_git.display()),
        )
        .unwrap();
        fs::write(
            worktree_git.join("gitdir"),
            format!("{}\n", worktree.join(".git").display()),
        )
        .unwrap();
        fs::write(worktree_git.join("HEAD"), "ref: refs/heads/feature\n").unwrap();

        write_recent(
            &settings.join("IntelliJIdea2026.2"),
            &[
                (main.to_str().unwrap(), 200),
                (worktree.to_str().unwrap(), 100),
            ],
        );
        let macos = apps.join("IntelliJ IDEA.app/Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("idea"), b"#!/bin/sh\n").unwrap();

        let config = test_config(settings, apps, temp.path().join("pins.json"));
        (temp, config)
    }

    #[test]
    fn worktree_mode_lists_only_linked_worktrees() {
        let (_temp, config) = worktree_fixture();

        let normal = search(
            &config,
            SearchOptions {
                product: None,
                filter: String::new(),
                show_stale: false,
                worktrees: false,
            },
        )
        .unwrap();
        assert_eq!(normal.items.len(), 1);
        assert_eq!(normal.items[0].title, "main");

        let worktrees = search(
            &config,
            SearchOptions {
                product: None,
                filter: String::new(),
                show_stale: false,
                worktrees: true,
            },
        )
        .unwrap();
        assert_eq!(worktrees.items.len(), 1);
        assert!(worktrees.items[0].title.contains("feature"));
        assert_eq!(
            worktrees.items[0]
                .variables
                .as_ref()
                .unwrap()
                .get("jb_worktree")
                .and_then(|value| value.as_str()),
            Some("1")
        );
    }

    #[test]
    fn hides_stale_by_default_and_sorts_by_timestamp() {
        let (_temp, config) = fixture();
        let response = search(
            &config,
            SearchOptions {
                product: None,
                filter: String::new(),
                show_stale: false,
                worktrees: false,
            },
        )
        .unwrap();
        let titles = response
            .items
            .iter()
            .map(|item| item.title.clone())
            .collect::<Vec<_>>();
        assert_eq!(titles, vec!["beta", "Alpha"]);
    }

    #[test]
    fn ignores_and_forgets_projects() {
        let (temp, mut config) = fixture();
        config.ignore = vec!["*beta*".to_string()];
        config.forgotten_file = temp.path().join("forgotten.json");

        let response = search(
            &config,
            SearchOptions {
                product: None,
                filter: String::new(),
                show_stale: false,
                worktrees: false,
            },
        )
        .unwrap();
        let titles = response
            .items
            .iter()
            .map(|item| item.title.clone())
            .collect::<Vec<_>>();
        assert_eq!(titles, vec!["Alpha"]);

        let alpha = temp.path().join("projects/alpha");
        forgotten::forget(&config.forgotten_file, &alpha).unwrap();
        let response = search(
            &config,
            SearchOptions {
                product: None,
                filter: String::new(),
                show_stale: false,
                worktrees: false,
            },
        )
        .unwrap();
        assert!(!response.items.iter().any(|item| item.title == "Alpha"));

        forgotten::remember(&config.forgotten_file, &alpha).unwrap();
        let response = search(
            &config,
            SearchOptions {
                product: None,
                filter: String::new(),
                show_stale: false,
                worktrees: false,
            },
        )
        .unwrap();
        assert!(response.items.iter().any(|item| item.title == "Alpha"));
    }

    #[test]
    fn shows_stale_when_requested() {
        let (_temp, config) = fixture();
        let response = search(
            &config,
            SearchOptions {
                product: None,
                filter: String::new(),
                show_stale: true,
                worktrees: false,
            },
        )
        .unwrap();
        assert_eq!(response.items.len(), 3);
        assert!(response.items.iter().any(|item| item.title == "missing"));
    }

    #[test]
    fn pinned_projects_come_first() {
        let (temp, config) = fixture();
        let alpha = temp.path().join("projects/alpha");
        pins::pin(&config.pins_file, &alpha).unwrap();

        let response = search(
            &config,
            SearchOptions {
                product: None,
                filter: String::new(),
                show_stale: false,
                worktrees: false,
            },
        )
        .unwrap();
        assert_eq!(response.items[0].title, "★ Alpha");
    }

    #[test]
    fn product_prefix_is_consumed() {
        let (_temp, config) = fixture();
        let response = search(
            &config,
            SearchOptions {
                product: None,
                filter: "idea alpha".to_string(),
                show_stale: false,
                worktrees: false,
            },
        )
        .unwrap();
        assert_eq!(response.items.len(), 1);
        assert_eq!(response.items[0].title, "Alpha");
    }

    #[test]
    fn unknown_product_errors() {
        let (_temp, config) = fixture();
        let error = search(
            &config,
            SearchOptions {
                product: Some("nope".to_string()),
                filter: String::new(),
                show_stale: false,
                worktrees: false,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown product"));
    }
}
