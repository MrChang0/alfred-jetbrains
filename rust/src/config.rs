//! Configuration loading and the merged product table.

use std::{collections::HashMap, env, fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::product::{PRODUCTS, Product};

#[derive(Debug, Clone, Serialize)]
pub struct ProductConfig {
    pub id: String,
    pub name: String,
    pub preference_prefix: String,
    pub application_names: Vec<String>,
    pub binaries: Vec<String>,
    pub keywords: Vec<String>,
    pub fleet: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub config_path: PathBuf,
    pub settings_roots: Vec<PathBuf>,
    pub app_roots: Vec<PathBuf>,
    pub show_stale: bool,
    pub hide_worktrees: bool,
    pub cache_seconds: u64,
    pub pins_file: PathBuf,
    pub workflow_name: String,
    pub default_product: String,
    pub terminal: String,
    pub ignore: Vec<String>,
    pub forgotten_file: PathBuf,
    pub products: Vec<ProductConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    settings_roots: Option<Vec<String>>,
    app_roots: Option<Vec<String>>,
    show_stale: Option<bool>,
    hide_worktrees: Option<bool>,
    cache_seconds: Option<u64>,
    pins_file: Option<String>,
    workflow_name: Option<String>,
    default_product: Option<String>,
    terminal: Option<String>,
    ignore: Option<Vec<String>>,
    forgotten_file: Option<String>,
    #[serde(default)]
    products: HashMap<String, ProductOverride>,
}

#[derive(Debug, Default, Deserialize, Clone)]
struct ProductOverride {
    application_names: Option<Vec<String>>,
    preference_prefix: Option<String>,
    binaries: Option<Vec<String>>,
    keywords: Option<Vec<String>>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let home = home_dir()?;
        let config_path = env::var_os("JB_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config/alfred-jetbrains/config.toml"));

        let file = if config_path.exists() {
            let text = fs::read_to_string(&config_path)
                .with_context(|| format!("read config {}", config_path.display()))?;
            toml::from_str::<ConfigFile>(&text)
                .with_context(|| format!("parse config {}", config_path.display()))?
        } else {
            ConfigFile::default()
        };

        let settings_roots = file
            .settings_roots
            .unwrap_or_else(default_settings_roots)
            .into_iter()
            .map(|path| expand_path(&path))
            .collect();
        let app_roots = file
            .app_roots
            .unwrap_or_else(default_app_roots)
            .into_iter()
            .map(|path| expand_path(&path))
            .collect();

        let pins_file = expand_path(
            file.pins_file
                .as_deref()
                .unwrap_or("~/.config/alfred-jetbrains/pins.json"),
        );

        let products = PRODUCTS
            .iter()
            .map(|product| merge_product(product, file.products.get(product.id)))
            .collect();

        Ok(Self {
            config_path,
            settings_roots,
            app_roots,
            show_stale: file.show_stale.unwrap_or(false),
            hide_worktrees: file.hide_worktrees.unwrap_or(true),
            cache_seconds: file.cache_seconds.unwrap_or(60),
            pins_file,
            workflow_name: file
                .workflow_name
                .unwrap_or_else(|| "alfred-jetbrains-rust".to_string()),
            default_product: file.default_product.unwrap_or_else(|| "idea".to_string()),
            terminal: file.terminal.unwrap_or_else(|| "Terminal".to_string()),
            ignore: file.ignore.unwrap_or_default(),
            forgotten_file: expand_path(
                file.forgotten_file
                    .as_deref()
                    .unwrap_or("~/.config/alfred-jetbrains/forgotten.json"),
            ),
            products,
        })
    }

    pub fn product_by_keyword(&self, value: &str) -> Option<&ProductConfig> {
        self.products
            .iter()
            .find(|product| product_keyword_matches(product, value))
    }

    pub fn product_by_id(&self, value: &str) -> Option<&ProductConfig> {
        self.products
            .iter()
            .find(|product| product.id.eq_ignore_ascii_case(value))
    }

    pub fn effective_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

fn product_keyword_matches(product: &ProductConfig, value: &str) -> bool {
    product.id.eq_ignore_ascii_case(value)
        || product
            .keywords
            .iter()
            .any(|keyword| keyword.eq_ignore_ascii_case(value))
}

#[cfg(test)]
pub(crate) fn default_products() -> Vec<ProductConfig> {
    PRODUCTS
        .iter()
        .map(|product| merge_product(product, None))
        .collect()
}

fn merge_product(product: &Product, overrides: Option<&ProductOverride>) -> ProductConfig {
    let default_strings = |values: &'static [&'static str]| {
        values
            .iter()
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>()
    };

    let overrides = overrides.cloned().unwrap_or_default();
    ProductConfig {
        id: product.id.to_string(),
        name: product.name.to_string(),
        preference_prefix: overrides
            .preference_prefix
            .unwrap_or_else(|| product.preference_prefix.to_string()),
        application_names: overrides
            .application_names
            .unwrap_or_else(|| default_strings(product.application_names)),
        binaries: overrides
            .binaries
            .unwrap_or_else(|| default_strings(product.binaries)),
        keywords: overrides
            .keywords
            .unwrap_or_else(|| default_strings(product.keywords)),
        fleet: product.fleet,
    }
}

pub fn home_dir() -> Result<PathBuf> {
    match env::var_os("HOME") {
        Some(home) if !home.is_empty() => Ok(PathBuf::from(home)),
        _ => bail!("HOME is not set"),
    }
}

pub fn expand_path(value: &str) -> PathBuf {
    let home = env::var("HOME").unwrap_or_default();
    let expanded = if let Some(rest) = value.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else if value == "~" {
        home
    } else {
        value.replace("$HOME", &home)
    };
    PathBuf::from(expanded)
}

fn default_settings_roots() -> Vec<String> {
    vec![
        "~/Library/Application Support/JetBrains".to_string(),
        "~/Library/Application Support/Google".to_string(),
        "~/Library/Preferences".to_string(),
    ]
}

fn default_app_roots() -> Vec<String> {
    vec![
        "/Applications".to_string(),
        "~/Applications".to_string(),
        "~/Applications/JetBrains Toolbox".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_product_override() {
        let product = PRODUCTS
            .iter()
            .find(|product| product.id == "idea")
            .unwrap();
        let mut products = HashMap::new();
        products.insert(
            "idea".to_string(),
            ProductOverride {
                preference_prefix: Some("CustomIdea".to_string()),
                ..Default::default()
            },
        );
        let merged = merge_product(product, products.get("idea"));
        assert_eq!(merged.preference_prefix, "CustomIdea");
        assert!(
            merged
                .application_names
                .contains(&"IntelliJ IDEA".to_string())
        );
    }

    #[test]
    fn expands_home_prefix() {
        let home = env::var("HOME").unwrap();
        assert_eq!(expand_path("~/x"), PathBuf::from(format!("{home}/x")));
    }
}
