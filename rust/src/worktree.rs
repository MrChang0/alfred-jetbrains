//! Read Git's own worktree metadata without invoking `git` or scanning
//! arbitrary directories.

use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    pub main_root: PathBuf,
    pub common_git_dir: PathBuf,
}

/// Project entries may be directories or files (for example a Rider `.sln`
/// solution); repository metadata lives in the containing directory.
fn repo_root(path: &Path) -> &Path {
    if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    }
}

pub fn repo_info(path: &Path) -> Option<RepoInfo> {
    let root = repo_root(path);
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(RepoInfo {
            main_root: root.to_path_buf(),
            common_git_dir: dot_git,
        });
    }
    if !dot_git.is_file() {
        return None;
    }

    let git_dir = resolve_git_dir(root)?;
    let worktrees_dir = git_dir.parent()?;
    if worktrees_dir.file_name()?.to_str()? != "worktrees" {
        return None;
    }
    let common_git_dir = worktrees_dir.parent()?.to_path_buf();
    let main_root = common_git_dir.parent()?.to_path_buf();

    Some(RepoInfo {
        main_root,
        common_git_dir,
    })
}

/// A linked worktree has a `.git` file pointing at
/// `<main>/.git/worktrees/<name>`; submodules point at `.git/modules/...`.
pub fn is_linked_worktree(path: &Path) -> bool {
    let root = repo_root(path);
    if !root.join(".git").is_file() {
        return false;
    }
    resolve_git_dir(root)
        .and_then(|git_dir| {
            git_dir
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .map(|name| name == "worktrees")
        })
        .unwrap_or(false)
}

/// Every linked worktree of the repository containing `path`, including the
/// main worktree only if it is itself linked elsewhere. The main checkout is
/// not included.
pub fn linked_worktrees(path: &Path) -> Vec<PathBuf> {
    let Some(info) = repo_info(path) else {
        return Vec::new();
    };
    let worktrees_dir = info.common_git_dir.join("worktrees");
    let Ok(read_dir) = fs::read_dir(worktrees_dir) else {
        return Vec::new();
    };

    let mut worktrees = Vec::new();
    for entry in read_dir.flatten() {
        let gitdir_file = entry.path().join("gitdir");
        let Ok(value) = fs::read_to_string(gitdir_file) else {
            continue;
        };
        let git_file = PathBuf::from(value.trim());
        let Some(root) = git_file.parent() else {
            continue;
        };
        if !root.exists() {
            continue;
        }
        let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        if !worktrees.contains(&root) {
            worktrees.push(root);
        }
    }
    worktrees.sort();
    worktrees
}

/// Current branch (or short detached commit) of a repository/worktree.
pub fn branch_for(path: &Path) -> Option<String> {
    let root = repo_root(path);
    let git_dir = if root.join(".git").is_dir() {
        root.join(".git")
    } else {
        resolve_git_dir(root)?
    };
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(branch) = head.strip_prefix("ref: refs/heads/") {
        return Some(branch.to_string());
    }
    head.get(..7).map(ToString::to_string)
}

fn resolve_git_dir(path: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(path.join(".git")).ok()?;
    let value = text.trim().strip_prefix("gitdir:")?.trim();
    let git_dir = PathBuf::from(value);
    if git_dir.is_absolute() {
        Some(git_dir)
    } else {
        Some(path.join(git_dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fake_repo() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempdir().unwrap();
        let main_root = temp.path().join("main");
        let common = main_root.join(".git");
        let worktree_git = common.join("worktrees/feature");
        fs::create_dir_all(&worktree_git).unwrap();
        fs::write(common.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let worktree = temp.path().join("main-feature");
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

        (temp, main_root, worktree)
    }

    #[test]
    fn detects_linked_worktree() {
        let (_temp, main, worktree) = fake_repo();
        assert!(!is_linked_worktree(&main));
        assert!(is_linked_worktree(&worktree));
        assert_eq!(repo_info(&worktree).unwrap().main_root, main);
    }

    #[test]
    fn lists_linked_worktrees_and_branches() {
        let (_temp, main, worktree) = fake_repo();
        let worktrees = linked_worktrees(&main);
        assert_eq!(worktrees, vec![fs::canonicalize(&worktree).unwrap()]);
        assert_eq!(branch_for(&main).as_deref(), Some("main"));
        assert_eq!(branch_for(&worktree).as_deref(), Some("feature"));
    }

    #[test]
    fn file_entries_use_the_parent_repository() {
        let (_temp, main, worktree) = fake_repo();
        let solution = main.join("Solution.sln");
        fs::write(&solution, "").unwrap();
        assert_eq!(repo_info(&solution).unwrap().main_root, main);
        assert_eq!(branch_for(&solution).as_deref(), Some("main"));

        let worktree_solution = worktree.join("Solution.sln");
        fs::write(&worktree_solution, "").unwrap();
        assert!(is_linked_worktree(&worktree_solution));
        assert_eq!(branch_for(&worktree_solution).as_deref(), Some("feature"));
    }
}
