//! Install the Alfred workflow: write info.plist, icon, and the binary.

use std::{
    collections::HashSet,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use plist::{Dictionary, Value};

use crate::{
    config::{self, Config, ProductConfig},
    hash,
};

const ACTION_SCRIPT: &str = r#"project="${1:-$jb_path}"
action="${jb_action:-open}"
case "$action" in
  pin) ./bin/jb toggle-pin "$project" ;;
  forget) ./bin/jb forget "$project" ;;
  terminal) ./bin/jb terminal "$project" ;;
  reveal) /usr/bin/open -R "$project" ;;
  copy) printf '%s' "$project" | /usr/bin/pbcopy ;;
  *)
    if [ -n "${jb_bin:-}" ] && [ -x "$jb_bin" ]; then
      "$jb_bin" "$project"
    elif [ -n "${jb_app:-}" ] && [ -d "$jb_app" ]; then
      /usr/bin/open -a "$jb_app" "$project"
    else
      ./bin/jb open "$project"
    fi
    ;;
esac
"#;

pub struct InstallOptions {
    pub workflow_name: Option<String>,
    pub retain: bool,
}

pub fn install(config: &Config, options: InstallOptions) -> Result<PathBuf> {
    let home = config::home_dir()?;
    install_in(&home, config, options)
}

fn install_in(home: &Path, config: &Config, options: InstallOptions) -> Result<PathBuf> {
    let prefs_path = home.join("Library/Application Support/Alfred/prefs.json");
    if !prefs_path.is_file() {
        bail!(
            "Alfred preferences not found at {}; is Alfred installed?",
            prefs_path.display()
        );
    }

    let prefs_text = fs::read_to_string(&prefs_path)
        .with_context(|| format!("read {}", prefs_path.display()))?;
    let prefs: serde_json::Value = serde_json::from_str(&prefs_text)
        .with_context(|| format!("parse {}", prefs_path.display()))?;
    let current = prefs
        .get("current")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("{} has no 'current' path", prefs_path.display()))?;

    let name = options
        .workflow_name
        .unwrap_or_else(|| config.workflow_name.clone());
    let workflow = Path::new(current).join("workflows").join(&name);
    fs::create_dir_all(&workflow).with_context(|| format!("create {}", workflow.display()))?;

    let info_plist = workflow.join("info.plist");
    plist::to_file_xml(&info_plist, &workflow_value(&config.products))
        .with_context(|| format!("write {}", info_plist.display()))?;

    let icon = workflow.join("icon.png");
    fs::write(&icon, include_bytes!("../assets/icon.png"))
        .with_context(|| format!("write {}", icon.display()))?;

    if !options.retain {
        let executable = std::env::current_exe().context("locate current executable")?;
        let destination = workflow.join("bin/jb");
        fs::create_dir_all(destination.parent().expect("destination has a parent"))
            .with_context(|| format!("create {}", destination.display()))?;
        let same_file = fs::canonicalize(&executable).ok() == fs::canonicalize(&destination).ok();
        if !same_file {
            fs::copy(&executable, &destination).with_context(|| {
                format!("copy {} to {}", executable.display(), destination.display())
            })?;
            fs::set_permissions(&destination, fs::Permissions::from_mode(0o755))?;
        }
    }

    Ok(workflow)
}

pub fn workflow_value(products: &[ProductConfig]) -> Value {
    let action_uid = hash::uuid_like("jb:action");
    let mut objects = Vec::new();
    let mut connected_uids = Vec::new();
    let mut seen = HashSet::new();

    let global_uid = hash::uuid_like("jb:search:all");
    objects.push(search_object(
        &global_uid,
        "jb",
        "./bin/jb search --filter \"$1\"",
        "JetBrains Projects",
        "Open JetBrains project",
        "searching projects",
    ));
    connected_uids.push(global_uid);

    let global_worktree_uid = hash::uuid_like("jb:search:worktrees:all");
    objects.push(search_object(
        &global_worktree_uid,
        "jb~",
        "./bin/jb search --worktrees --filter \"$1\"",
        "JetBrains Worktrees",
        "Open a git worktree",
        "searching worktrees",
    ));
    connected_uids.push(global_worktree_uid);

    for product in products {
        let Some(keyword) = product
            .keywords
            .first()
            .map(String::as_str)
            .map(str::trim)
            .filter(|keyword| !keyword.is_empty())
        else {
            continue;
        };
        if !seen.insert(keyword.to_ascii_lowercase()) {
            continue;
        }

        let uid = hash::uuid_like(&format!("jb:search:{keyword}"));
        let script = format!("./bin/jb search --product {} --filter \"$1\"", product.id);
        objects.push(search_object(
            &uid,
            keyword,
            &script,
            &product.name,
            &format!("Open {} project", product.name),
            &format!("searching {} projects", product.name),
        ));
        connected_uids.push(uid);

        let worktree_uid = hash::uuid_like(&format!("jb:search:{keyword}:worktrees"));
        let worktree_script = format!(
            "./bin/jb search --product {} --worktrees --filter \"$1\"",
            product.id
        );
        objects.push(search_object(
            &worktree_uid,
            &format!("{keyword}~"),
            &worktree_script,
            &format!("{} Worktrees", product.name),
            &format!("Open {} worktree", product.name),
            &format!("searching {} worktrees", product.name),
        ));
        connected_uids.push(worktree_uid);
    }

    objects.push(action_object(&action_uid));

    let file_action_uid = hash::uuid_like("jb:fileaction");
    objects.push(file_action_object(&file_action_uid));
    connected_uids.push(file_action_uid);

    let mut connections = Dictionary::new();
    for uid in &connected_uids {
        let mut destination = Dictionary::new();
        destination.insert(
            "destinationuid".to_string(),
            Value::String(action_uid.clone()),
        );
        destination.insert("modifiers".to_string(), Value::from(0));
        destination.insert("modifiersubtext".to_string(), Value::String(String::new()));
        destination.insert("vitoclose".to_string(), Value::from(false));
        connections.insert(
            uid.clone(),
            Value::Array(vec![Value::Dictionary(destination)]),
        );
    }

    let mut all_uids = connected_uids;
    all_uids.push(action_uid);
    let mut uidata = Dictionary::new();
    for (index, uid) in all_uids.iter().enumerate() {
        let mut position = Dictionary::new();
        position.insert("xpos".to_string(), Value::from(100.0_f64));
        position.insert("ypos".to_string(), Value::from(100.0 + index as f64 * 70.0));
        uidata.insert(uid.clone(), Value::Dictionary(position));
    }

    let mut root = Dictionary::new();
    root.insert(
        "bundleid".to_string(),
        Value::from("com.mrchang0.alfred-jetbrains-rust"),
    );
    root.insert("category".to_string(), Value::from("Productivity"));
    root.insert("createdby".to_string(), Value::from("MrChang0"));
    root.insert(
        "description".to_string(),
        Value::from("Open JetBrains projects from Alfred"),
    );
    root.insert("disabled".to_string(), Value::from(false));
    root.insert("name".to_string(), Value::from("JetBrains Projects (Rust)"));
    root.insert(
        "readme".to_string(),
        Value::from(
            "# JetBrains Projects\n\n\
             Type `jb` for all projects, or a product keyword such as `idea`, \
             `pycharm`, `pstorm`, `wstorm`, `studio`, `clion`, `goland`, \
             `datagrip`, `dataspell`, `rider`, `rubymine`, `rustrover`, \
             `fleet`, `aqua`, `appcode`, or `writerside`. Add `~` to see linked \
             git worktrees (`jb~`, `idea~`). Hold ⌘ to pin/unpin, ⌥ to reveal in \
             Finder, ⌃ to copy the path, ⇧ to open a terminal, and ⌘⇧ to forget. \
             You can also select any folder in Alfred or Finder and use Open in \
             JetBrains.",
        ),
    );
    root.insert(
        "version".to_string(),
        Value::from(env!("CARGO_PKG_VERSION")),
    );
    root.insert(
        "webaddress".to_string(),
        Value::from("https://github.com/MrChang0/alfred-jetbrains"),
    );
    root.insert("userconfigurationconfig".to_string(), user_configuration());
    root.insert("objects".to_string(), Value::Array(objects));
    root.insert("connections".to_string(), Value::Dictionary(connections));
    root.insert("uidata".to_string(), Value::Dictionary(uidata));
    Value::Dictionary(root)
}

fn search_object(
    uid: &str,
    keyword: &str,
    script: &str,
    title: &str,
    subtext: &str,
    running_subtext: &str,
) -> Value {
    let mut config = Dictionary::new();
    config.insert("alfredfiltersresults".to_string(), Value::from(false));
    config.insert("alfredfiltersresultsmatchmode".to_string(), Value::from(2));
    config.insert(
        "argumenttreatemptyqueryasnil".to_string(),
        Value::from(false),
    );
    config.insert("argumenttrimmode".to_string(), Value::from(0));
    config.insert("argumenttype".to_string(), Value::from(1));
    config.insert("escaping".to_string(), Value::from(102));
    config.insert("keyword".to_string(), Value::String(keyword.to_string()));
    config.insert("queuedelaycustom".to_string(), Value::from(3));
    config.insert(
        "queuedelayimmediatelyinitially".to_string(),
        Value::from(true),
    );
    config.insert("queuedelaymode".to_string(), Value::from(0));
    config.insert("queuemode".to_string(), Value::from(1));
    config.insert(
        "runningsubtext".to_string(),
        Value::String(running_subtext.to_string()),
    );
    config.insert("script".to_string(), Value::String(script.to_string()));
    config.insert("scriptargtype".to_string(), Value::from(1));
    config.insert("scriptfile".to_string(), Value::from(""));
    config.insert("subtext".to_string(), Value::String(subtext.to_string()));
    config.insert("title".to_string(), Value::String(title.to_string()));
    config.insert("type".to_string(), Value::from(5));
    config.insert("withspace".to_string(), Value::from(true));

    let mut object = Dictionary::new();
    object.insert("config".to_string(), Value::Dictionary(config));
    object.insert(
        "type".to_string(),
        Value::from("alfred.workflow.input.scriptfilter"),
    );
    object.insert("uid".to_string(), Value::String(uid.to_string()));
    object.insert("version".to_string(), Value::from(3));
    Value::Dictionary(object)
}

fn action_object(uid: &str) -> Value {
    let mut config = Dictionary::new();
    config.insert("concurrently".to_string(), Value::from(false));
    config.insert("escaping".to_string(), Value::from(102));
    config.insert("script".to_string(), Value::from(ACTION_SCRIPT));
    config.insert("scriptargtype".to_string(), Value::from(1));
    config.insert("scriptfile".to_string(), Value::from(""));
    config.insert("type".to_string(), Value::from(5));

    let mut object = Dictionary::new();
    object.insert("config".to_string(), Value::Dictionary(config));
    object.insert(
        "type".to_string(),
        Value::from("alfred.workflow.action.script"),
    );
    object.insert("uid".to_string(), Value::String(uid.to_string()));
    object.insert("version".to_string(), Value::from(2));
    Value::Dictionary(object)
}

fn file_action_object(uid: &str) -> Value {
    let mut config = Dictionary::new();
    config.insert("acceptsmulti".to_string(), Value::from(0));
    config.insert("filetypes".to_string(), Value::Array(Vec::new()));
    config.insert("name".to_string(), Value::from("Open in JetBrains"));

    let mut object = Dictionary::new();
    object.insert("config".to_string(), Value::Dictionary(config));
    object.insert(
        "type".to_string(),
        Value::from("alfred.workflow.trigger.action"),
    );
    object.insert("uid".to_string(), Value::String(uid.to_string()));
    object.insert("version".to_string(), Value::from(1));
    Value::Dictionary(object)
}

fn user_configuration() -> Value {
    Value::Array(vec![
        text_config(
            "jb_terminal",
            "Terminal Application",
            "Application opened by ⇧ and `jb terminal` (Ghostty, iTerm, Terminal, …)",
            "Ghostty",
        ),
        text_config(
            "jb_default_product",
            "Default IDE",
            "Product id used when `jb open` cannot infer one (idea, pycharm, …)",
            "idea",
        ),
        checkbox_config(
            "jb_show_stale",
            "Show stale projects",
            "Show projects whose directories no longer exist",
            false,
        ),
        checkbox_config(
            "jb_hide_worktrees",
            "Hide linked worktrees",
            "Hide worktrees from the normal list; they stay available under jb~",
            true,
        ),
        popup_config(
            "jb_sort",
            "Sort order",
            "Order of results before a query is typed",
            "recent",
            &[
                ("Most recent", "recent"),
                ("Least recent", "oldest"),
                ("Name A–Z", "name"),
                ("Name Z–A", "name-desc"),
                ("Path", "path"),
            ],
        ),
        textarea_config(
            "jb_ignore",
            "Ignore globs",
            "One glob per line, matched against project name and path (e.g. ~/Downloads/* or *-scratch)",
            "",
        ),
        text_config(
            "jb_settings_roots",
            "Settings roots",
            "Colon-separated JetBrains settings roots",
            "~/Library/Application Support/JetBrains:~/Library/Application Support/Google:~/Library/Preferences",
        ),
        text_config(
            "jb_app_roots",
            "Application roots",
            "Colon-separated folders scanned for JetBrains .app bundles",
            "/Applications:~/Applications:~/Applications/JetBrains Toolbox",
        ),
    ])
}

fn text_config(variable: &str, label: &str, description: &str, default: &str) -> Value {
    let mut config = Dictionary::new();
    config.insert("default".to_string(), Value::from(default));
    config.insert("placeholder".to_string(), Value::from(default));
    config.insert("required".to_string(), Value::from(false));
    config.insert("trim".to_string(), Value::from(true));
    user_config_entry(variable, "textfield", label, description, config)
}

fn checkbox_config(variable: &str, label: &str, description: &str, default: bool) -> Value {
    let mut config = Dictionary::new();
    config.insert("default".to_string(), Value::from(default));
    config.insert("required".to_string(), Value::from(false));
    config.insert("text".to_string(), Value::from(""));
    user_config_entry(variable, "checkbox", label, description, config)
}

fn popup_config(
    variable: &str,
    label: &str,
    description: &str,
    default: &str,
    pairs: &[(&str, &str)],
) -> Value {
    let mut config = Dictionary::new();
    config.insert("default".to_string(), Value::from(default));
    let pairs = pairs
        .iter()
        .map(|(label, value)| Value::Array(vec![Value::from(*label), Value::from(*value)]))
        .collect::<Vec<_>>();
    config.insert("pairs".to_string(), Value::Array(pairs));
    user_config_entry(variable, "popupbutton", label, description, config)
}

fn textarea_config(variable: &str, label: &str, description: &str, default: &str) -> Value {
    let mut config = Dictionary::new();
    config.insert("default".to_string(), Value::from(default));
    config.insert("required".to_string(), Value::from(false));
    config.insert("trim".to_string(), Value::from(true));
    config.insert("verticalsize".to_string(), Value::from(4));
    user_config_entry(variable, "textarea", label, description, config)
}

fn user_config_entry(
    variable: &str,
    type_: &str,
    label: &str,
    description: &str,
    config: Dictionary,
) -> Value {
    let mut entry = Dictionary::new();
    entry.insert("variable".to_string(), Value::from(variable));
    entry.insert("type".to_string(), Value::from(type_));
    entry.insert("label".to_string(), Value::from(label));
    entry.insert("description".to_string(), Value::from(description));
    entry.insert("config".to_string(), Value::Dictionary(config));
    Value::Dictionary(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_products() -> Vec<ProductConfig> {
        crate::config::default_products()
    }

    #[test]
    fn workflow_has_all_product_keywords() {
        let value = workflow_value(&default_products());
        let dict = value.as_dictionary().unwrap();
        let objects = dict["objects"].as_array().unwrap();
        assert!(objects.len() > 2);
        assert_eq!(
            objects.last().unwrap().as_dictionary().unwrap()["type"].as_string(),
            Some("alfred.workflow.trigger.action")
        );
        assert!(objects.iter().any(|object| {
            object.as_dictionary().unwrap()["type"].as_string()
                == Some("alfred.workflow.action.script")
        }));
        let file_action = objects.last().unwrap().as_dictionary().unwrap();
        assert_eq!(
            file_action["config"].as_dictionary().unwrap()["name"].as_string(),
            Some("Open in JetBrains")
        );
        assert_eq!(file_action["version"].as_signed_integer(), Some(1));

        let find = |keyword: &str| {
            objects
                .iter()
                .map(|object| object.as_dictionary().unwrap())
                .find(|object| {
                    object["config"]
                        .as_dictionary()
                        .unwrap()
                        .get("keyword")
                        .and_then(|value| value.as_string())
                        == Some(keyword)
                })
                .unwrap_or_else(|| panic!("missing keyword {keyword}"))
        };

        let idea = find("idea");
        assert_eq!(
            idea["config"].as_dictionary().unwrap()["script"].as_string(),
            Some("./bin/jb search --product idea --filter \"$1\"")
        );
        assert_eq!(
            idea["config"].as_dictionary().unwrap()["alfredfiltersresults"].as_boolean(),
            Some(false)
        );
        let jb_worktrees = find("jb~");
        assert_eq!(
            jb_worktrees["config"].as_dictionary().unwrap()["script"].as_string(),
            Some("./bin/jb search --worktrees --filter \"$1\"")
        );
        let idea_worktrees = find("idea~");
        assert_eq!(
            idea_worktrees["config"].as_dictionary().unwrap()["script"].as_string(),
            Some("./bin/jb search --product idea --worktrees --filter \"$1\"")
        );

        let action = objects
            .iter()
            .map(|object| object.as_dictionary().unwrap())
            .find(|object| object["type"].as_string() == Some("alfred.workflow.action.script"))
            .expect("action object");
        let action_script = action["config"].as_dictionary().unwrap()["script"]
            .as_string()
            .unwrap();
        assert!(action_script.contains("forget)"));
        assert!(action_script.contains("./bin/jb open \"$project\""));

        let connections = dict["connections"].as_dictionary().unwrap();
        assert_eq!(connections.len(), objects.len() - 1);
    }

    #[test]
    fn workflow_object_uids_are_unique() {
        let value = workflow_value(&default_products());
        let dict = value.as_dictionary().unwrap();
        let objects = dict["objects"].as_array().unwrap();
        let mut uids = std::collections::HashSet::new();
        for object in objects {
            let uid = object.as_dictionary().unwrap()["uid"]
                .as_string()
                .unwrap()
                .to_string();
            assert!(uids.insert(uid.clone()), "duplicate uid {uid}");
        }
        assert_eq!(uids.len(), objects.len());
    }

    #[test]
    fn workflow_has_user_configuration() {
        let value = workflow_value(&default_products());
        let dict = value.as_dictionary().unwrap();
        let items = dict["userconfigurationconfig"].as_array().unwrap();
        assert_eq!(items.len(), 8);

        let terminal = items
            .iter()
            .map(|item| item.as_dictionary().unwrap())
            .find(|item| item["variable"].as_string() == Some("jb_terminal"))
            .expect("jb_terminal");
        assert_eq!(terminal["type"].as_string(), Some("textfield"));
        assert_eq!(
            terminal["config"].as_dictionary().unwrap()["default"].as_string(),
            Some("Ghostty")
        );
        assert!(items.iter().any(|item| {
            item.as_dictionary().unwrap()["variable"].as_string() == Some("jb_show_stale")
        }));
        let sort = items
            .iter()
            .map(|item| item.as_dictionary().unwrap())
            .find(|item| item["variable"].as_string() == Some("jb_sort"))
            .expect("jb_sort");
        assert_eq!(sort["type"].as_string(), Some("popupbutton"));
        assert_eq!(
            sort["config"].as_dictionary().unwrap()["default"].as_string(),
            Some("recent")
        );
    }

    #[test]
    fn install_writes_plist_and_icon() {
        let temp = tempfile::tempdir().unwrap();
        let alfred = temp
            .path()
            .join("Library/Application Support/Alfred/Alfred.alfredpreferences");
        std::fs::create_dir_all(&alfred).unwrap();
        std::fs::write(
            temp.path()
                .join("Library/Application Support/Alfred/prefs.json"),
            format!(r#"{{"current":"{}"}}"#, alfred.display()),
        )
        .unwrap();

        let config = Config {
            config_path: PathBuf::from("/dev/null"),
            settings_roots: vec![],
            app_roots: vec![],
            show_stale: false,
            hide_worktrees: true,
            sort: "recent".to_string(),
            cache_seconds: 60,
            pins_file: PathBuf::from("/dev/null"),
            workflow_name: "alfred-jetbrains-rust".to_string(),
            default_product: "idea".to_string(),
            terminal: "Terminal".to_string(),
            ignore: vec![],
            forgotten_file: PathBuf::from("/dev/null"),
            products: default_products(),
        };

        let workflow = install_in(
            temp.path(),
            &config,
            InstallOptions {
                workflow_name: None,
                retain: true,
            },
        )
        .unwrap();

        assert!(workflow.join("info.plist").is_file());
        assert!(workflow.join("icon.png").is_file());
        let plist = plist::Value::from_file(workflow.join("info.plist")).unwrap();
        assert!(plist.as_dictionary().unwrap().contains_key("objects"));
    }
}
