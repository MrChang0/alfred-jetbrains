//! Project paths and display names.

use std::{
    fs,
    path::{Path, PathBuf},
};

use roxmltree::Document;

pub fn expand_project_path(key: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    let expanded = key.replace("$USER_HOME$", &home);
    if let Some(rest) = expanded.strip_prefix("~/") {
        PathBuf::from(&home).join(rest)
    } else {
        PathBuf::from(expanded)
    }
}

/// JetBrains stores pseudo-projects such as LightEdit under its application
/// config directory, and Maven-style keys under `$MAVEN_REPOSITORY$`. Only
/// `$USER_HOME$` refers to a real user project, so every other leading
/// macro is filtered out rather than enumerating known cases.
pub fn is_internal_key(key: &str) -> bool {
    let Some(rest) = key.strip_prefix('$') else {
        return false;
    };
    let Some(end) = rest.find('$') else {
        return true;
    };
    &rest[..end] != "USER_HOME"
}

/// A stable key used for deduplication and pin matching. Existing paths are
/// canonicalized so symlinks and `..` segments collapse; missing paths fall
/// back to a case-insensitive literal (APFS is case-insensitive by default).
pub fn dedupe_key(path: &Path) -> String {
    match fs::canonicalize(path) {
        Ok(canonical) => canonical.to_string_lossy().to_string(),
        Err(_) => path.to_string_lossy().to_lowercase(),
    }
}

pub fn basename(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

pub fn project_name(path: &Path) -> String {
    let idea = path.join(".idea");
    if idea.is_dir() {
        for file_name in ["name", ".name"] {
            if let Some(value) = read_trimmed(&idea.join(file_name)) {
                return value;
            }
        }

        if let Ok(read_dir) = fs::read_dir(&idea) {
            let mut modules = read_dir
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("iml"))
                .collect::<Vec<_>>();
            modules.sort();
            if let Some(module) = modules.first()
                && let Some(stem) = module.file_stem().and_then(|stem| stem.to_str())
            {
                return stem.to_string();
            }
        }

        if let Some(name) = workspace_name(&idea.join("workspace.xml")) {
            return name;
        }
    }

    if path.extension().and_then(|ext| ext.to_str()) == Some("sln")
        && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
    {
        return stem.to_string();
    }

    basename(path)
}

fn read_trimmed(path: &Path) -> Option<String> {
    let value = fs::read_to_string(path).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn workspace_name(path: &Path) -> Option<String> {
    let xml = fs::read_to_string(path).ok()?;
    let doc = Document::parse(&xml).ok()?;

    let project_view = doc.descendants().find(|node| {
        node.is_element()
            && node.tag_name().name() == "component"
            && node.attribute("name") == Some("ProjectView")
    })?;

    if let Some(path_element) = project_view
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "PATH_ELEMENT")
        && let Some(value) = path_element
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "option")
            .filter_map(|node| node.attribute("value"))
            .map(str::trim)
            .find(|value| !value.is_empty())
    {
        return Some(value.to_string());
    }

    project_view
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "item")
        .find(|node| {
            node.attribute("type")
                .is_some_and(|kind| kind.contains("ProjectViewProjectNode"))
        })
        .and_then(|node| node.attribute("name"))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn expands_user_home() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            expand_project_path("$USER_HOME$/Code/demo"),
            PathBuf::from(format!("{home}/Code/demo"))
        );
    }

    #[test]
    fn skips_internal_keys() {
        assert!(is_internal_key("$APPLICATION_CONFIG_DIR$/light-edit"));
        assert!(is_internal_key("$MAVEN_REPOSITORY$/com/example/demo"));
        assert!(!is_internal_key("$USER_HOME$/Code/demo"));
        assert!(!is_internal_key("/Users/me/Code/demo"));
    }

    #[test]
    fn reads_dot_name() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("my-project");
        fs::create_dir_all(project.join(".idea")).unwrap();
        fs::write(project.join(".idea/.name"), "Custom Name\n").unwrap();
        assert_eq!(project_name(&project), "Custom Name");
    }

    #[test]
    fn reads_iml_stem() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("my-project");
        fs::create_dir_all(project.join(".idea")).unwrap();
        fs::write(project.join(".idea/backend.iml"), "").unwrap();
        assert_eq!(project_name(&project), "backend");
    }

    #[test]
    fn falls_back_to_basename() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("plain-project");
        fs::create_dir_all(&project).unwrap();
        assert_eq!(project_name(&project), "plain-project");
    }

    #[test]
    fn reads_workspace_project_view_name() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("workspace-project");
        fs::create_dir_all(project.join(".idea")).unwrap();
        fs::write(
            project.join(".idea/workspace.xml"),
            r#"<project><component name="ProjectView"><panes><pane id="ProjectPane">
              <expand><path><item type="1:ProjectViewProjectNode" name="From Workspace" /></path></expand>
            </pane></panes></component></project>"#,
        )
        .unwrap();
        assert_eq!(project_name(&project), "From Workspace");
    }
}
