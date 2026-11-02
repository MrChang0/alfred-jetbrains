//! Pinned projects, stored as a JSON array of absolute paths.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::projects::dedupe_key;

pub fn load(path: &Path) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

pub fn load_keys(path: &Path) -> Result<HashSet<String>> {
    Ok(load(path)?
        .iter()
        .map(|path| dedupe_key(Path::new(path)))
        .collect())
}

pub fn save(path: &Path, projects: &[String]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(projects)?;
    fs::write(path, format!("{text}\n")).with_context(|| format!("write {}", path.display()))
}

pub fn pin(path: &Path, project: &Path) -> Result<bool> {
    let mut projects = load(path)?;
    let target = absolute(project);
    let key = dedupe_key(Path::new(&target));
    if projects
        .iter()
        .any(|stored| dedupe_key(Path::new(stored)) == key)
    {
        return Ok(false);
    }
    projects.push(target);
    save(path, &projects)?;
    Ok(true)
}

pub fn unpin(path: &Path, project: &Path) -> Result<bool> {
    let projects = load(path)?;
    let target = absolute(project);
    let key = dedupe_key(Path::new(&target));
    let original_len = projects.len();
    let filtered = projects
        .into_iter()
        .filter(|stored| dedupe_key(Path::new(stored)) != key)
        .collect::<Vec<_>>();
    let changed = filtered.len() != original_len;
    if changed {
        save(path, &filtered)?;
    }
    Ok(changed)
}

pub fn toggle(path: &Path, project: &Path) -> Result<bool> {
    let target = absolute(project);
    let key = dedupe_key(Path::new(&target));
    let projects = load(path)?;
    let pinned = projects
        .iter()
        .any(|stored| dedupe_key(Path::new(stored)) == key);
    if pinned {
        unpin(path, project)?;
        Ok(false)
    } else {
        pin(path, project)?;
        Ok(true)
    }
}

pub fn absolute(project: &Path) -> String {
    let path: PathBuf = fs::canonicalize(project).unwrap_or_else(|_| {
        if project.is_absolute() {
            project.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(project)
        }
    });
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn pin_and_unpin_round_trip() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let pins = dir.path().join("config/pins.json");

        assert!(pin(&pins, &project).unwrap());
        assert!(!pin(&pins, &project).unwrap());
        assert_eq!(load(&pins).unwrap().len(), 1);

        assert!(unpin(&pins, &project).unwrap());
        assert_eq!(load(&pins).unwrap().len(), 0);
    }

    #[test]
    fn unpin_reports_not_pinned() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let pins = dir.path().join("pins.json");

        assert!(!unpin(&pins, &project).unwrap());
        assert!(!pins.exists());
    }

    #[test]
    fn toggle_reports_state() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let pins = dir.path().join("pins.json");

        assert!(toggle(&pins, &project).unwrap());
        assert!(!toggle(&pins, &project).unwrap());
    }
}
