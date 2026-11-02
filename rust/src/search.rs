//! Project search, filtering, and Alfred response assembly.

use anyhow::{Result, anyhow};

use crate::{
    alfred::{self, AlfredResponse},
    config::Config,
    discovery::{self, ProjectEntry},
    logging, pins,
};

pub struct SearchOptions {
    pub product: Option<String>,
    pub filter: String,
    pub show_stale: bool,
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
        pins,
    };

    fn test_config(settings: PathBuf, apps: PathBuf, pins_file: PathBuf) -> Config {
        Config {
            config_path: PathBuf::from("/dev/null"),
            settings_roots: vec![settings],
            app_roots: vec![apps],
            show_stale: false,
            cache_seconds: 60,
            pins_file,
            workflow_name: "test".to_string(),
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

    #[test]
    fn hides_stale_by_default_and_sorts_by_timestamp() {
        let (_temp, config) = fixture();
        let response = search(
            &config,
            SearchOptions {
                product: None,
                filter: String::new(),
                show_stale: false,
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
    fn shows_stale_when_requested() {
        let (_temp, config) = fixture();
        let response = search(
            &config,
            SearchOptions {
                product: None,
                filter: String::new(),
                show_stale: true,
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
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown product"));
    }
}
