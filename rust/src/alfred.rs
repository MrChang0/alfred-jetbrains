//! Alfred Script Filter JSON.

use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::{config::Config, discovery::ProjectEntry, hash, projects::basename};

const GENERIC_ICON: &str =
    "/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/public.generic-pc.icns";
const ERROR_ICON: &str =
    "/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/AlertStopIcon.icns";

#[derive(Debug, Serialize)]
pub struct AlfredResponse {
    pub cache: Cache,
    pub items: Vec<Item>,
}

#[derive(Debug, Serialize)]
pub struct Cache {
    pub seconds: u64,
    pub loosereload: bool,
}

#[derive(Debug, Serialize)]
pub struct Item {
    pub uid: String,
    pub title: String,
    #[serde(rename = "match")]
    pub match_: String,
    pub subtitle: String,
    pub arg: String,
    pub autocomplete: String,
    pub text: ItemText,
    pub icon: ItemIcon,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variables: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mods: Option<Map<String, Value>>,
}

#[derive(Debug, Serialize)]
pub struct ItemText {
    pub copy: String,
    pub largetype: String,
}

#[derive(Debug, Serialize)]
pub struct ItemIcon {
    pub path: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
}

pub fn response(config: &Config, entries: &[ProjectEntry]) -> AlfredResponse {
    let items = entries.iter().map(Item::from_entry).collect();
    AlfredResponse {
        cache: Cache {
            seconds: config.cache_seconds,
            loosereload: true,
        },
        items,
    }
}

pub fn info_response(config: &Config, title: &str, subtitle: &str) -> AlfredResponse {
    AlfredResponse {
        cache: Cache {
            seconds: config.cache_seconds,
            loosereload: true,
        },
        items: vec![Item {
            uid: "info".to_string(),
            title: title.to_string(),
            match_: title.to_string(),
            subtitle: subtitle.to_string(),
            arg: String::new(),
            autocomplete: title.to_string(),
            text: ItemText {
                copy: subtitle.to_string(),
                largetype: title.to_string(),
            },
            icon: ItemIcon {
                path: GENERIC_ICON.to_string(),
                type_: None,
            },
            variables: None,
            mods: None,
        }],
    }
}

pub fn error_response(cache_seconds: u64, message: &str) -> AlfredResponse {
    AlfredResponse {
        cache: Cache {
            seconds: cache_seconds,
            loosereload: true,
        },
        items: vec![Item {
            uid: "error".to_string(),
            title: message.to_string(),
            match_: message.to_string(),
            subtitle: "Run `jb --debug search` for details".to_string(),
            arg: String::new(),
            autocomplete: message.to_string(),
            text: ItemText {
                copy: message.to_string(),
                largetype: message.to_string(),
            },
            icon: ItemIcon {
                path: ERROR_ICON.to_string(),
                type_: None,
            },
            variables: None,
            mods: None,
        }],
    }
}

impl Item {
    fn from_entry(entry: &ProjectEntry) -> Self {
        let path = entry.path.to_string_lossy().to_string();
        let mut subtitle_parts = vec![match &entry.version {
            Some(version) => format!("{} {version}", entry.product.name),
            None => entry.product.name.clone(),
        }];
        if entry.stale {
            subtitle_parts.push("missing".to_string());
        }
        if entry.pinned {
            subtitle_parts.push("pinned".to_string());
        }
        if entry.is_worktree {
            subtitle_parts.push(match &entry.branch {
                Some(branch) => format!("worktree: {branch}"),
                None => "worktree".to_string(),
            });
        } else if let Some(branch) = &entry.branch {
            subtitle_parts.push(format!("branch: {branch}"));
        }
        if entry.app.is_none() && entry.bin.is_none() {
            subtitle_parts.push("IDE not found".to_string());
        }
        subtitle_parts.push(path.clone());
        let subtitle = subtitle_parts.join(" · ");

        let title = if entry.pinned {
            format!("★ {}", entry.name)
        } else {
            entry.name.clone()
        };

        let icon = match &entry.app {
            Some(app) => ItemIcon {
                path: app.to_string_lossy().to_string(),
                type_: Some("fileicon".to_string()),
            },
            None => ItemIcon {
                path: GENERIC_ICON.to_string(),
                type_: None,
            },
        };

        let mut variables = Map::new();
        variables.insert("jb_path".to_string(), Value::String(path.clone()));
        variables.insert(
            "jb_product".to_string(),
            Value::String(entry.product.id.clone()),
        );
        variables.insert(
            "jb_product_name".to_string(),
            Value::String(entry.product.name.clone()),
        );
        variables.insert(
            "jb_version".to_string(),
            Value::String(entry.version.clone().unwrap_or_default()),
        );
        variables.insert(
            "jb_build".to_string(),
            Value::String(entry.build.clone().unwrap_or_default()),
        );
        variables.insert(
            "jb_activation".to_string(),
            Value::String(
                entry
                    .activation_timestamp
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
            ),
        );
        variables.insert(
            "jb_production_code".to_string(),
            Value::String(entry.production_code.clone().unwrap_or_default()),
        );
        variables.insert(
            "jb_app".to_string(),
            Value::String(entry.app.as_deref().map(path_to_string).unwrap_or_default()),
        );
        variables.insert(
            "jb_bin".to_string(),
            Value::String(entry.bin.as_deref().map(path_to_string).unwrap_or_default()),
        );
        variables.insert(
            "jb_pinned".to_string(),
            Value::String(if entry.pinned { "1" } else { "0" }.to_string()),
        );
        variables.insert(
            "jb_stale".to_string(),
            Value::String(if entry.stale { "1" } else { "0" }.to_string()),
        );
        variables.insert(
            "jb_worktree".to_string(),
            Value::String(if entry.is_worktree { "1" } else { "0" }.to_string()),
        );
        variables.insert(
            "jb_branch".to_string(),
            Value::String(entry.branch.clone().unwrap_or_default()),
        );

        let mut mods = Map::new();
        mods.insert("cmd".to_string(), modifier("Pin/Unpin", "pin"));
        mods.insert("alt".to_string(), modifier("Reveal in Finder", "reveal"));
        mods.insert("ctrl".to_string(), modifier("Copy path", "copy"));
        mods.insert(
            "shift".to_string(),
            modifier("Open in Terminal", "terminal"),
        );
        mods.insert(
            "cmd+shift".to_string(),
            modifier("Forget project", "forget"),
        );

        Self {
            uid: format!("{:016x}", hash::fnv1a(&entry.key)),
            title,
            match_: format!(
                "{} {} {}",
                entry.name,
                basename(&entry.path),
                entry.product.name
            ),
            subtitle,
            arg: path.clone(),
            autocomplete: entry.name.clone(),
            text: ItemText {
                copy: path,
                largetype: entry.name.clone(),
            },
            icon,
            variables: Some(variables),
            mods: Some(mods),
        }
    }
}

fn modifier(subtitle: &str, action: &str) -> Value {
    let mut variables = Map::new();
    variables.insert("jb_action".to_string(), Value::String(action.to_string()));
    let mut value = Map::new();
    value.insert("subtitle".to_string(), Value::String(subtitle.to_string()));
    value.insert("variables".to_string(), Value::Object(variables));
    Value::Object(value)
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::{Config, default_products};

    fn config() -> Config {
        Config {
            config_path: PathBuf::from("/dev/null"),
            settings_roots: vec![],
            app_roots: vec![],
            show_stale: false,
            hide_worktrees: true,
            sort: "recent".to_string(),
            cache_seconds: 60,
            pins_file: PathBuf::from("/dev/null"),
            workflow_name: "test".to_string(),
            default_product: "idea".to_string(),
            terminal: "Terminal".to_string(),
            ignore: vec![],
            forgotten_file: PathBuf::from("/dev/null"),
            products: default_products(),
        }
    }

    fn entry(name: &str, pinned: bool, stale: bool) -> ProjectEntry {
        let product = default_products()
            .into_iter()
            .find(|product| product.id == "idea")
            .unwrap();
        ProjectEntry {
            key: "/tmp/alpha".to_string(),
            path: PathBuf::from("/tmp/alpha"),
            name: name.to_string(),
            product,
            version: Some("2026.2".to_string()),
            open_timestamp: Some(1),
            activation_timestamp: None,
            build: Some("IU-1".to_string()),
            production_code: Some("IU".to_string()),
            app: Some(PathBuf::from("/Applications/IntelliJ IDEA.app")),
            bin: Some(PathBuf::from(
                "/Applications/IntelliJ IDEA.app/Contents/MacOS/idea",
            )),
            pinned,
            stale,
            is_worktree: false,
            branch: None,
        }
    }

    #[test]
    fn normal_entry_shows_branch() {
        let mut entry = entry("alpha", false, false);
        entry.branch = Some("main".to_string());
        let item = Item::from_entry(&entry);
        assert!(item.subtitle.contains("branch: main"));
        assert_eq!(item.variables.unwrap()["jb_branch"], "main");
    }

    #[test]
    fn worktree_entry_shows_branch() {
        let mut entry = entry("alpha-feature", false, false);
        entry.is_worktree = true;
        entry.branch = Some("feature/x".to_string());
        let item = Item::from_entry(&entry);
        assert!(item.subtitle.contains("worktree: feature/x"));
        let variables = item.variables.unwrap();
        assert_eq!(variables["jb_worktree"], "1");
        assert_eq!(variables["jb_branch"], "feature/x");
    }

    #[test]
    fn builds_alfred_item() {
        let item = Item::from_entry(&entry("alpha", false, false));
        assert_eq!(item.title, "alpha");
        assert_eq!(item.arg, "/tmp/alpha");
        assert_eq!(item.icon.type_.as_deref(), Some("fileicon"));
        assert!(item.mods.as_ref().unwrap().contains_key("cmd+shift"));
        let variables = item.variables.unwrap();
        assert_eq!(variables["jb_product"], "idea");
        assert_eq!(variables["jb_version"], "2026.2");
        assert_eq!(variables["jb_stale"], "0");
    }

    #[test]
    fn pinned_entry_gets_star() {
        let item = Item::from_entry(&entry("alpha", true, false));
        assert_eq!(item.title, "★ alpha");
        assert!(item.subtitle.contains("pinned"));
    }

    #[test]
    fn stale_entry_is_marked() {
        let item = Item::from_entry(&entry("alpha", false, true));
        assert!(item.subtitle.contains("missing"));
        assert_eq!(item.variables.unwrap()["jb_stale"], "1");
    }

    #[test]
    fn uid_is_stable() {
        let first = Item::from_entry(&entry("alpha", false, false)).uid;
        let second = Item::from_entry(&entry("alpha", false, false)).uid;
        assert_eq!(first, second);
        assert_eq!(first.len(), 16);
    }

    #[test]
    fn response_has_cache() {
        let response = response(&config(), &[entry("alpha", false, false)]);
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["cache"]["seconds"], 60);
        assert_eq!(json["items"][0]["title"], "alpha");
    }
}
