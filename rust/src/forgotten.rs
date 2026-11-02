//! Projects hidden with `jb forget`, stored like pins in a JSON array.

use std::{collections::HashSet, path::Path};

use anyhow::Result;

use crate::pins;

pub fn load_keys(path: &Path) -> Result<HashSet<String>> {
    pins::load_keys(path)
}

pub fn forget(path: &Path, project: &Path) -> Result<bool> {
    pins::pin(path, project)
}

pub fn remember(path: &Path, project: &Path) -> Result<bool> {
    pins::unpin(path, project)
}

pub fn clear(path: &Path) -> Result<()> {
    pins::save(path, &[])
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn forget_remember_and_clear() {
        let temp = tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let file = temp.path().join("config/forgotten.json");

        assert!(forget(&file, &project).unwrap());
        assert!(!forget(&file, &project).unwrap());
        assert_eq!(pins::load(&file).unwrap().len(), 1);

        assert!(remember(&file, &project).unwrap());
        assert_eq!(pins::load(&file).unwrap().len(), 0);
        assert!(!remember(&file, &project).unwrap());

        forget(&file, &project).unwrap();
        clear(&file).unwrap();
        assert!(pins::load(&file).unwrap().is_empty());
    }
}
