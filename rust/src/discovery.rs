//! Discover JetBrains projects across every product and every version.

use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use regex::Regex;
use walkdir::WalkDir;

use crate::{
    config::{Config, ProductConfig},
    logging,
    projects::{dedupe_key, expand_project_path, is_internal_key, project_name},
    worktree,
    xml::{self, RawEntry},
};

#[derive(Debug, Clone)]
pub struct ProjectEntry {
    pub key: String,
    pub path: PathBuf,
    pub name: String,
    pub product: ProductConfig,
    pub version: Option<String>,
    pub open_timestamp: Option<i64>,
    pub activation_timestamp: Option<i64>,
    pub build: Option<String>,
    pub production_code: Option<String>,
    pub app: Option<PathBuf>,
    pub bin: Option<PathBuf>,
    pub pinned: bool,
    pub stale: bool,
    pub is_worktree: bool,
    pub branch: Option<String>,
}

pub struct AppIndex {
    bundles: Vec<AppBundle>,
}

#[derive(Debug, Clone)]
struct AppBundle {
    path: PathBuf,
    stem: String,
    version: Option<String>,
}

pub struct BinIndex {
    by_name: HashMap<String, Vec<PathBuf>>,
}

type AppCacheKey = (String, Option<String>);
type AppCacheValue = (Option<PathBuf>, Option<PathBuf>);
type EntryParser = fn(&Path) -> Result<Vec<RawEntry>>;

pub fn discover(config: &Config) -> Vec<ProjectEntry> {
    let version_re =
        Regex::new(r"^(?:\d{4}\.\d(?:\.\d)?|\d\.\d(?:\.\d)?)$").expect("valid version regex");
    let app_index = build_app_index(config);
    let bin_index = build_bin_index();
    let mut app_cache: HashMap<AppCacheKey, AppCacheValue> = HashMap::new();
    let mut entries = Vec::new();

    for product in &config.products {
        for (settings_dir, version) in find_settings_dirs(config, product, &version_re) {
            logging::debug(format!(
                "settings: {} ({:?})",
                settings_dir.display(),
                version
            ));

            for raw in read_settings_entries(&settings_dir, product) {
                if is_internal_key(&raw.key) {
                    continue;
                }

                let path = expand_project_path(&raw.key);
                let key = dedupe_key(&path);
                let cache_key = (product.id.clone(), version.clone());
                let (app, bin) = app_cache
                    .entry(cache_key)
                    .or_insert_with(|| {
                        let app = app_index
                            .find(product, version.as_deref())
                            .map(Path::to_path_buf);
                        let bin = bin_index.find(product, app.as_deref());
                        (app, bin)
                    })
                    .clone();

                let is_worktree = worktree::is_linked_worktree(&path);
                entries.push(ProjectEntry {
                    key,
                    name: project_name(&path),
                    stale: !path.exists(),
                    path: path.clone(),
                    product: product.clone(),
                    version: version.clone(),
                    open_timestamp: raw.open_timestamp,
                    activation_timestamp: raw.activation_timestamp,
                    build: raw.build,
                    production_code: raw.production_code,
                    app,
                    bin,
                    pinned: false,
                    is_worktree,
                    branch: is_worktree.then(|| worktree::branch_for(&path)).flatten(),
                });
            }
        }
    }

    entries
}

pub fn dedupe(entries: Vec<ProjectEntry>) -> Vec<ProjectEntry> {
    let mut by_key: HashMap<String, ProjectEntry> = HashMap::new();
    for entry in entries {
        match by_key.get(&entry.key) {
            Some(existing)
                if (existing.open_timestamp.unwrap_or(0), !existing.stale)
                    >= (entry.open_timestamp.unwrap_or(0), !entry.stale) => {}
            _ => {
                by_key.insert(entry.key.clone(), entry);
            }
        }
    }
    by_key.into_values().collect()
}

/// Derive linked worktrees from the repositories of the given entries. This
/// reads `.git/worktrees/*/gitdir`, so it does not run `git` or scan unknown
/// directories.
pub fn worktree_entries(entries: &[ProjectEntry]) -> Vec<ProjectEntry> {
    let mut main_roots = Vec::new();
    let mut seen_roots = HashSet::new();
    for entry in entries {
        if let Some(info) = worktree::repo_info(&entry.path)
            && seen_roots.insert(info.main_root.clone())
        {
            main_roots.push(info.main_root);
        }
    }

    let mut result: HashMap<String, ProjectEntry> = HashMap::new();
    for main_root in main_roots {
        let base = entries
            .iter()
            .filter(|entry| {
                worktree::repo_info(&entry.path).is_some_and(|info| info.main_root == main_root)
            })
            .max_by_key(|entry| entry.open_timestamp.unwrap_or(i64::MIN));
        let Some(base) = base else {
            continue;
        };

        let base_name = if main_root.exists() {
            project_name(&main_root)
        } else {
            base.name.clone()
        };

        for path in worktree::linked_worktrees(&main_root) {
            let branch = worktree::branch_for(&path);
            let name = match &branch {
                Some(branch) => format!("{base_name} ({branch})"),
                None => format!("{base_name} (worktree)"),
            };
            let key = dedupe_key(&path);
            let entry = ProjectEntry {
                key: key.clone(),
                path: path.clone(),
                name,
                product: base.product.clone(),
                version: base.version.clone(),
                open_timestamp: base.open_timestamp,
                activation_timestamp: base.activation_timestamp,
                build: base.build.clone(),
                production_code: base.production_code.clone(),
                app: base.app.clone(),
                bin: base.bin.clone(),
                pinned: false,
                stale: !path.exists(),
                is_worktree: true,
                branch,
            };

            match result.get(&key) {
                Some(existing)
                    if existing.open_timestamp.unwrap_or(0)
                        >= entry.open_timestamp.unwrap_or(0) => {}
                _ => {
                    result.insert(key, entry);
                }
            }
        }
    }

    result.into_values().collect()
}

fn find_settings_dirs(
    config: &Config,
    product: &ProductConfig,
    version_re: &Regex,
) -> Vec<(PathBuf, Option<String>)> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();

    for root in &config.settings_roots {
        let Ok(read_dir) = fs::read_dir(root) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some(suffix) = name.strip_prefix(&product.preference_prefix) else {
                continue;
            };

            let version = if suffix.is_empty() {
                if product.fleet {
                    None
                } else {
                    continue;
                }
            } else if version_re.is_match(suffix) {
                Some(suffix.to_string())
            } else {
                continue;
            };

            if seen.insert(path.clone()) {
                dirs.push((path, version));
            }
        }
    }

    dirs.sort_by(|left, right| {
        version_key(&right.1)
            .cmp(&version_key(&left.1))
            .then_with(|| left.0.cmp(&right.0))
    });
    dirs
}

fn read_settings_entries(settings_dir: &Path, product: &ProductConfig) -> Vec<RawEntry> {
    let options = settings_dir.join("options");
    let parsers: [(&str, EntryParser); 3] = [
        ("recentProjects.xml", xml::parse_recent_projects),
        (
            "recentProjectDirectories.xml",
            xml::parse_recent_directories,
        ),
        ("recentSolutions.xml", xml::parse_recent_solutions),
    ];

    let mut merged: HashMap<String, RawEntry> = HashMap::new();
    for (file_name, parser) in parsers {
        let path = options.join(file_name);
        if !path.is_file() {
            continue;
        }
        match parser(&path) {
            Ok(entries) => {
                for entry in entries {
                    merge_raw(&mut merged, entry);
                }
            }
            Err(error) => logging::debug(format!("skip {}: {error:#}", path.display())),
        }
    }

    if product.fleet {
        let backend = settings_dir.join("backend");
        if backend.is_dir() {
            for entry in WalkDir::new(&backend).max_depth(5).into_iter().flatten() {
                if entry.file_name() != "trusted-paths.xml" {
                    continue;
                }
                match xml::parse_trusted_paths(entry.path()) {
                    Ok(entries) => {
                        for entry in entries {
                            merge_raw(&mut merged, entry);
                        }
                    }
                    Err(error) => {
                        logging::debug(format!("skip {}: {error:#}", entry.path().display()))
                    }
                }
            }
        }
    }

    let mut entries = merged.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.key.cmp(&right.key));
    entries
}

fn merge_raw(merged: &mut HashMap<String, RawEntry>, raw: RawEntry) {
    match merged.get_mut(&raw.key) {
        Some(existing) => {
            existing.open_timestamp = max_opt(existing.open_timestamp, raw.open_timestamp);
            existing.activation_timestamp =
                max_opt(existing.activation_timestamp, raw.activation_timestamp);
            if existing.build.is_none() {
                existing.build = raw.build;
            }
            if existing.production_code.is_none() {
                existing.production_code = raw.production_code;
            }
        }
        None => {
            merged.insert(raw.key.clone(), raw);
        }
    }
}

fn max_opt(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn version_key(version: &Option<String>) -> (u32, u32, u32) {
    let Some(version) = version else {
        return (0, 0, 0);
    };
    let mut parts = version
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

pub fn build_app_index(config: &Config) -> AppIndex {
    let version_re = Regex::new(r"(\d{4}\.\d(?:\.\d)?)").expect("valid app version regex");
    let mut bundles = Vec::new();
    let mut seen = HashSet::new();

    for root in &config.app_roots {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root)
            .max_depth(4)
            .follow_links(false)
            .into_iter()
            .flatten()
        {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("app") {
                continue;
            }
            if !seen.insert(path.to_path_buf()) {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string();
            let version = version_re
                .captures(&stem)
                .and_then(|captures| captures.get(1))
                .map(|value| value.as_str().to_string());
            bundles.push(AppBundle {
                path: path.to_path_buf(),
                stem,
                version,
            });
        }
    }

    bundles.sort_by(|left, right| left.path.cmp(&right.path));
    AppIndex { bundles }
}

impl AppIndex {
    pub fn find(&self, product: &ProductConfig, version: Option<&str>) -> Option<&Path> {
        let mut candidates = self
            .bundles
            .iter()
            .filter(|bundle| {
                product
                    .application_names
                    .iter()
                    .any(|name| bundle.matches(name))
            })
            .collect::<Vec<_>>();

        candidates.sort_by(|left, right| {
            let left_match = version.is_some_and(|version| left.stem.contains(version));
            let right_match = version.is_some_and(|version| right.stem.contains(version));
            right_match
                .cmp(&left_match)
                .then_with(|| version_key(&right.version).cmp(&version_key(&left.version)))
                .then_with(|| left.stem.cmp(&right.stem))
                .then_with(|| left.path.cmp(&right.path))
        });

        candidates.first().map(|bundle| bundle.path.as_path())
    }
}

impl AppBundle {
    fn matches(&self, name: &str) -> bool {
        self.stem == name || self.stem.starts_with(&format!("{name} "))
    }
}

pub fn build_bin_index() -> BinIndex {
    let mut dirs = Vec::new();
    let jb_binaries = env::var_os("JB_BINARIES").filter(|value| !value.is_empty());
    if let Some(value) = jb_binaries {
        dirs.extend(env::split_paths(&value));
    } else if let Some(path) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&path));
    }

    let mut by_name: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for dir in dirs {
        let Ok(read_dir) = fs::read_dir(dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                by_name.entry(name.to_string()).or_default().push(path);
            }
        }
    }

    for paths in by_name.values_mut() {
        paths.sort();
    }
    BinIndex { by_name }
}

impl BinIndex {
    pub fn find(&self, product: &ProductConfig, app: Option<&Path>) -> Option<PathBuf> {
        if let Some(app) = app {
            let macos = app.join("Contents/MacOS");
            if let Ok(read_dir) = fs::read_dir(macos) {
                for entry in read_dir.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if product.binaries.iter().any(|binary| binary == &name) {
                        return Some(entry.path());
                    }
                }
            }
        }

        for binary in &product.binaries {
            if let Some(paths) = self.by_name.get(binary)
                && let Some(path) = paths.first()
            {
                return Some(path.clone());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::config::default_products;

    fn test_config(settings: &Path, apps: &Path) -> Config {
        Config {
            config_path: PathBuf::from("/dev/null"),
            settings_roots: vec![settings.to_path_buf()],
            app_roots: vec![apps.to_path_buf()],
            show_stale: false,
            hide_worktrees: true,
            cache_seconds: 60,
            pins_file: PathBuf::from("/dev/null"),
            workflow_name: "test".to_string(),
            default_product: "idea".to_string(),
            products: default_products(),
        }
    }

    fn write_recent(dir: &Path, entries: &[(&str, i64)]) {
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

    fn write_app(apps: &Path) {
        let macos = apps.join("IntelliJ IDEA.app/Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("idea"), b"#!/bin/sh\n").unwrap();
    }

    #[test]
    fn discovers_entries_and_app() {
        let temp = tempdir().unwrap();
        let settings = temp.path().join("JetBrains");
        let apps = temp.path().join("Applications");
        let project = temp.path().join("projects/alpha");
        fs::create_dir_all(project.join(".idea")).unwrap();
        fs::write(project.join(".idea/.name"), "Alpha").unwrap();
        write_recent(
            &settings.join("IntelliJIdea2026.2"),
            &[(project.to_str().unwrap(), 100)],
        );
        write_app(&apps);

        let entries = discover(&test_config(&settings, &apps));
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.name, "Alpha");
        assert_eq!(entry.product.id, "idea");
        assert_eq!(entry.version.as_deref(), Some("2026.2"));
        assert_eq!(entry.open_timestamp, Some(100));
        assert!(entry.app.as_ref().unwrap().ends_with("IntelliJ IDEA.app"));
        assert!(entry.bin.as_ref().unwrap().ends_with("Contents/MacOS/idea"));
        assert!(!entry.stale);
    }

    #[test]
    fn merges_all_versions_and_keeps_stale_flag() {
        let temp = tempdir().unwrap();
        let settings = temp.path().join("JetBrains");
        let apps = temp.path().join("Applications");
        let old_project = temp.path().join("projects/old");
        let new_project = temp.path().join("projects/new");
        fs::create_dir_all(&old_project).unwrap();
        fs::create_dir_all(&new_project).unwrap();
        write_recent(
            &settings.join("IntelliJIdea2025.3"),
            &[(old_project.to_str().unwrap(), 50)],
        );
        write_recent(
            &settings.join("IntelliJIdea2026.2"),
            &[(new_project.to_str().unwrap(), 100)],
        );
        write_app(&apps);

        let entries = discover(&test_config(&settings, &apps));
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| !entry.stale));

        let missing = temp.path().join("projects/missing");
        write_recent(
            &settings.join("IntelliJIdea2026.2"),
            &[(missing.to_str().unwrap(), 200)],
        );
        let entries = discover(&test_config(&settings, &apps));
        assert!(entries.iter().any(|entry| entry.stale));
    }

    #[test]
    fn fleet_reads_trusted_paths() {
        let temp = tempdir().unwrap();
        let settings = temp.path().join("JetBrains");
        let apps = temp.path().join("Applications");
        let project = temp.path().join("projects/fleet");
        fs::create_dir_all(&project).unwrap();
        let backend = settings.join("Fleet/backend/default");
        fs::create_dir_all(&backend).unwrap();
        fs::write(
            backend.join("trusted-paths.xml"),
            format!(
                r#"<application><component><option name="TRUSTED_PROJECT_PATHS"><map><entry key="{}" /></map></option></component></application>"#,
                project.display()
            ),
        )
        .unwrap();

        let entries = discover(&test_config(&settings, &apps));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].product.id, "fleet");
        assert_eq!(entries[0].path, project);
    }
}
