//! Project search, filtering, and Alfred response assembly.

use std::{cmp::Ordering, collections::HashMap};

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
    pub sort: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    Recent,
    Oldest,
    Name,
    NameDesc,
    Path,
}

impl SortOrder {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "recent" | "most-recent" | "newest" => Some(Self::Recent),
            "oldest" | "least-recent" => Some(Self::Oldest),
            "name" | "name-asc" | "a-z" => Some(Self::Name),
            "name-desc" | "z-a" => Some(Self::NameDesc),
            "path" => Some(Self::Path),
            _ => None,
        }
    }
}

pub fn search(config: &Config, options: SearchOptions) -> Result<AlfredResponse> {
    let (product_id, filter) = resolve_product(config, options.product, options.filter)?;
    let sort = options
        .sort
        .as_deref()
        .and_then(SortOrder::parse)
        .or_else(|| SortOrder::parse(&config.sort))
        .unwrap_or(SortOrder::Recent);
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
    let mut scores: HashMap<String, u32> = HashMap::new();
    if !filter.is_empty() {
        entries.retain(|entry| {
            if let Some(score) = fuzzy_score(entry, &filter) {
                scores.insert(entry.key.clone(), score);
                true
            } else {
                false
            }
        });
    }

    entries.sort_by(|left, right| {
        right
            .pinned
            .cmp(&left.pinned)
            .then_with(|| {
                if filter.is_empty() {
                    Ordering::Equal
                } else {
                    scores
                        .get(&right.key)
                        .copied()
                        .unwrap_or(0)
                        .cmp(&scores.get(&left.key).copied().unwrap_or(0))
                }
            })
            .then_with(|| compare_entries(left, right, sort))
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
    // Only consume a leading keyword when something follows it. Otherwise a
    // short keyword such as `go` would swallow a query for a project name.
    if !rest.is_empty()
        && let Some(product) = config.product_by_keyword(first)
    {
        return Ok((Some(product.id.clone()), rest.to_string()));
    }

    Ok((None, filter))
}

fn compare_entries(left: &ProjectEntry, right: &ProjectEntry, sort: SortOrder) -> Ordering {
    match sort {
        SortOrder::Recent => right
            .open_timestamp
            .unwrap_or(i64::MIN)
            .cmp(&left.open_timestamp.unwrap_or(i64::MIN))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase())),
        SortOrder::Oldest => left
            .open_timestamp
            .unwrap_or(i64::MAX)
            .cmp(&right.open_timestamp.unwrap_or(i64::MAX))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase())),
        SortOrder::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
        SortOrder::NameDesc => right.name.to_lowercase().cmp(&left.name.to_lowercase()),
        SortOrder::Path => left.path.cmp(&right.path),
    }
    .then_with(|| left.path.cmp(&right.path))
}

fn fuzzy_score(entry: &ProjectEntry, filter: &str) -> Option<u32> {
    let home = std::env::var("HOME").ok();
    fuzzy_score_with_home(entry, filter, home.as_deref())
}

fn fuzzy_score_with_home(entry: &ProjectEntry, filter: &str, home: Option<&str>) -> Option<u32> {
    let filter = filter.trim();
    if filter.is_empty() {
        return Some(0);
    }

    let expanded = expand_tilde(&filter.to_lowercase(), home);
    if filter.contains('/') || filter.contains('~') {
        let path = entry.path.to_string_lossy().to_lowercase();
        return fuzzy_match(&path, &expanded);
    }

    let name = entry.name.to_lowercase();
    let basename = entry
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_lowercase();
    let product_name = entry.product.name.to_lowercase();
    let product_id = entry.product.id.to_lowercase();
    let version = entry.version.as_deref().unwrap_or_default().to_lowercase();
    let build = entry.build.as_deref().unwrap_or_default().to_lowercase();
    let components = relative_components(&entry.path, home);

    // Every whitespace-separated term must match somewhere, so `fory java`
    // finds `fory-java` and `github agentscope` can span path components.
    let mut total = 0u32;
    for term in expanded.split_whitespace() {
        let best = [
            fuzzy_match(&name, term),
            // A directory basename is a weaker signal than the project name,
            // which may come from `.idea/.name` or a module file.
            fuzzy_match(&basename, term).map(|score| score.saturating_sub(100)),
            word_prefix_score(&product_name, term).or_else(|| word_prefix_score(&product_id, term)),
            contiguous_score(&version, term),
            contiguous_score(&build, term),
            components
                .iter()
                .filter_map(|component| component_score(component, term))
                .max(),
        ]
        .into_iter()
        .flatten()
        .max()?;
        total = total.saturating_add(best);
    }

    Some(total)
}

fn expand_tilde(filter: &str, home: Option<&str>) -> String {
    let Some(home) = home else {
        return filter.to_string();
    };
    let home = home.to_lowercase();
    if filter == "~" {
        home
    } else if let Some(rest) = filter.strip_prefix("~/") {
        format!("{}/{}", home.trim_end_matches('/'), rest)
    } else {
        filter.to_string()
    }
}

/// Path components below `HOME`; the home directory itself is not a useful
/// search token and matched almost every project when scored.
fn relative_components(path: &std::path::Path, home: Option<&str>) -> Vec<String> {
    let mut components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
        .collect::<Vec<_>>();
    if let Some(home) = home {
        let home = home.to_lowercase();
        let home_components = std::path::Path::new(&home)
            .components()
            .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
            .collect::<Vec<_>>();
        if !home_components.is_empty()
            && components.len() >= home_components.len()
            && components[..home_components.len()] == home_components[..]
        {
            components.drain(..home_components.len());
        }
    }
    components
}

fn component_score(component: &str, term: &str) -> Option<u32> {
    let length = term.chars().count() as u32;
    if component.starts_with(term) {
        Some(300 + length * 10)
    } else if term.chars().count() >= 3 && component.contains(term) {
        Some(250 + length * 5)
    } else {
        None
    }
}

/// Product names match by word prefix only: `idea` matches "IntelliJ IDEA",
/// but the two-letter `li` does not match every JetBrains product.
fn word_prefix_score(haystack: &str, term: &str) -> Option<u32> {
    haystack
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .find(|word| word.starts_with(term))
        .map(|_| 500 + term.chars().count() as u32 * 10)
}

fn contiguous_score(haystack: &str, term: &str) -> Option<u32> {
    if haystack.is_empty() {
        return None;
    }
    haystack
        .find(term)
        .map(|position| 400 + term.chars().count() as u32 * 10 - position.min(100) as u32)
}

fn fuzzy_match(haystack: &str, needle: &str) -> Option<u32> {
    if needle.is_empty() {
        return Some(0);
    }
    if let Some(position) = haystack.find(needle) {
        let needle_len = needle.chars().count() as u32;
        return Some(1000 + needle_len * 10 - position.min(200) as u32);
    }

    let mut chars = haystack.chars().enumerate();
    let mut last_index: Option<usize> = None;
    let mut score = 100u32;
    for needle_char in needle.chars() {
        let mut found = None;
        for (index, ch) in chars.by_ref() {
            if ch == needle_char {
                found = Some(index);
                break;
            }
        }
        let index = found?;
        if let Some(last) = last_index {
            if index == last + 1 {
                score += 10;
            } else {
                score = score.saturating_sub((index - last - 1).min(20usize) as u32);
            }
        }
        last_index = Some(index);
    }
    Some(score)
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
            sort: "recent".to_string(),
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

    fn opts_filter(filter: &str) -> SearchOptions {
        SearchOptions {
            product: None,
            filter: filter.to_string(),
            show_stale: false,
            worktrees: false,
            sort: None,
        }
    }

    fn entry_for(path: &std::path::Path, name: &str) -> ProjectEntry {
        let product = default_products()
            .into_iter()
            .find(|product| product.id == "idea")
            .unwrap();
        ProjectEntry {
            key: path.to_string_lossy().to_string(),
            path: path.to_path_buf(),
            name: name.to_string(),
            product,
            version: Some("2026.2".to_string()),
            open_timestamp: Some(1),
            activation_timestamp: None,
            build: None,
            production_code: None,
            app: None,
            bin: None,
            pinned: false,
            stale: false,
            is_worktree: false,
            branch: None,
        }
    }

    fn custom_fixture(projects: &[(&str, &str, i64)]) -> (tempfile::TempDir, Config) {
        let temp = tempdir().unwrap();
        let settings = temp.path().join("JetBrains");
        let apps = temp.path().join("Applications");
        let mut recent = Vec::new();
        for (relative, name, timestamp) in projects {
            let path = temp.path().join(relative);
            fs::create_dir_all(path.join(".idea")).unwrap();
            fs::write(path.join(".idea/.name"), name).unwrap();
            recent.push((path, *timestamp));
        }
        let entries = recent
            .iter()
            .map(|(path, timestamp)| (path.to_str().unwrap().to_string(), *timestamp))
            .collect::<Vec<_>>();
        let refs = entries
            .iter()
            .map(|(path, timestamp)| (path.as_str(), *timestamp))
            .collect::<Vec<_>>();
        write_recent(&settings.join("IntelliJIdea2026.2"), &refs);
        let macos = apps.join("IntelliJ IDEA.app/Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("idea"), b"#!/bin/sh\n").unwrap();
        let config = test_config(settings, apps, temp.path().join("pins.json"));
        (temp, config)
    }

    #[test]
    fn multi_word_filter_matches_name_tokens() {
        let (_temp, config) = custom_fixture(&[("Code/github/fory/java", "fory-java", 100)]);
        let response = search(&config, opts_filter("fory java")).unwrap();
        assert_eq!(response.items.len(), 1);
        assert_eq!(response.items[0].title, "fory-java");
    }

    #[test]
    fn multi_word_filter_matches_path_components() {
        let (_temp, config) = custom_fixture(&[("Code/github/agentscope", "agentscope", 100)]);
        let response = search(&config, opts_filter("github agentscope")).unwrap();
        assert_eq!(response.items.len(), 1);
        assert_eq!(response.items[0].title, "agentscope");
    }

    #[test]
    fn filter_matches_version_and_product() {
        let (_temp, config) = custom_fixture(&[("Code/github/demo", "demo", 100)]);
        assert_eq!(
            search(&config, opts_filter("2026.2")).unwrap().items.len(),
            1
        );
        assert_eq!(search(&config, opts_filter("idea")).unwrap().items.len(), 1);
    }

    #[test]
    fn path_components_do_not_match_home_directory() {
        let temp = tempdir().unwrap();
        let home = temp.path().join("Users/lishuchang");
        let project = home.join("Code/project");
        fs::create_dir_all(&project).unwrap();
        let entry = entry_for(&project, "project");
        let home = home.to_str().unwrap();

        assert!(fuzzy_score_with_home(&entry, "project", Some(home)).is_some());
        assert!(fuzzy_score_with_home(&entry, "li", Some(home)).is_none());
    }

    #[test]
    fn tilde_filter_matches_home_relative_path() {
        let temp = tempdir().unwrap();
        let home = temp.path().join("home");
        let project = home.join("Code/github/demo");
        fs::create_dir_all(&project).unwrap();
        let entry = entry_for(&project, "demo");
        let home = home.to_str().unwrap();

        assert!(fuzzy_score_with_home(&entry, "~/Code/github", Some(home)).is_some());
        assert!(fuzzy_score_with_home(&entry, "~/Code/other", Some(home)).is_none());
    }

    #[test]
    fn name_matches_rank_above_basename_matches() {
        let temp = tempdir().unwrap();
        let name_match = entry_for(&temp.path().join("other"), "target");
        let basename_match = entry_for(&temp.path().join("target"), "other");

        let name_score = fuzzy_score_with_home(&name_match, "target", None).unwrap();
        let basename_score = fuzzy_score_with_home(&basename_match, "target", None).unwrap();
        assert!(name_score > basename_score);
    }

    #[test]
    fn two_letter_product_keyword_does_not_swallow_query() {
        let (_temp, config) = custom_fixture(&[("Code/github/gopher", "gopher", 100)]);
        let response = search(&config, opts_filter("go")).unwrap();
        assert_eq!(response.items.len(), 1);
        assert_eq!(response.items[0].title, "gopher");
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
                sort: None,
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
                sort: None,
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
                sort: None,
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
    fn fuzzy_matches_subsequence() {
        assert!(fuzzy_match("pangolin-search", "pngln").is_some());
        assert!(fuzzy_match("beta", "bt").is_some());
        assert!(fuzzy_match("pangolin", "xyz").is_none());
    }

    #[test]
    fn sorts_by_name_oldest_and_path() {
        let (_temp, config) = fixture();
        let order = |sort: &str| {
            let response = search(
                &config,
                SearchOptions {
                    product: None,
                    filter: String::new(),
                    show_stale: false,
                    worktrees: false,
                    sort: Some(sort.to_string()),
                },
            )
            .unwrap();
            response
                .items
                .iter()
                .map(|item| item.title.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(order("name"), vec!["Alpha", "beta"]);
        assert_eq!(order("oldest"), vec!["Alpha", "beta"]);
        assert_eq!(order("path"), vec!["Alpha", "beta"]);
        assert_eq!(order("name-desc"), vec!["beta", "Alpha"]);
    }

    #[test]
    fn fuzzy_filter_keeps_subsequence_matches() {
        let (_temp, config) = fixture();
        let response = search(
            &config,
            SearchOptions {
                product: None,
                filter: "bt".to_string(),
                show_stale: false,
                worktrees: false,
                sort: None,
            },
        )
        .unwrap();
        assert_eq!(response.items.len(), 1);
        assert_eq!(response.items[0].title, "beta");
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
                sort: None,
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
                sort: None,
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
                sort: None,
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
                sort: None,
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
                sort: None,
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
                sort: None,
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
                sort: None,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown product"));
    }
}
