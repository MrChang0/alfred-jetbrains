//! Open a project in the IDE that last used it.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result};

use crate::{config::Config, discovery, projects::dedupe_key};

pub struct OpenOptions {
    pub path: PathBuf,
    pub product: Option<String>,
    pub app: Option<PathBuf>,
    pub bin: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Target {
    pub app: Option<PathBuf>,
    pub bin: Option<PathBuf>,
}

pub fn open(config: &Config, options: OpenOptions) -> Result<()> {
    let path = fs::canonicalize(&options.path).unwrap_or_else(|_| options.path.clone());
    let target = resolve_target(
        config,
        &path,
        options.product.as_deref(),
        options.app,
        options.bin,
    )?;

    if let Some(bin) = target.bin {
        Command::new(&bin)
            .arg(&path)
            .spawn()
            .with_context(|| format!("run {}", bin.display()))?;
        return Ok(());
    }
    if let Some(app) = target.app {
        Command::new("/usr/bin/open")
            .arg("-a")
            .arg(&app)
            .arg(&path)
            .spawn()
            .with_context(|| format!("open with {}", app.display()))?;
        return Ok(());
    }

    Command::new("/usr/bin/open")
        .arg(&path)
        .spawn()
        .with_context(|| format!("open {}", path.display()))?;
    Ok(())
}

pub fn open_terminal(config: &Config, path: &Path) -> Result<()> {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    Command::new("/usr/bin/open")
        .arg("-a")
        .arg(&config.terminal)
        .arg(&path)
        .spawn()
        .with_context(|| format!("open terminal '{}' at {}", config.terminal, path.display()))?;
    Ok(())
}

pub fn resolve_target(
    config: &Config,
    path: &Path,
    product: Option<&str>,
    app: Option<PathBuf>,
    bin: Option<PathBuf>,
) -> Result<Target> {
    if app.is_some() || bin.is_some() {
        return Ok(Target { app, bin });
    }

    let explicit_product = match product {
        Some(value) => Some(resolve_product_id(config, value)?),
        None => None,
    };

    let key = dedupe_key(path);
    let entries = discovery::discover(config);
    let entry = entries
        .into_iter()
        .filter(|entry| entry.key == key)
        .filter(|entry| {
            explicit_product
                .as_deref()
                .is_none_or(|product| entry.product.id == product)
        })
        .max_by_key(|entry| entry.open_timestamp.unwrap_or(i64::MIN));

    if let Some(entry) = entry
        && (entry.app.is_some() || entry.bin.is_some())
    {
        return Ok(Target {
            app: entry.app,
            bin: entry.bin,
        });
    }

    let inference_root = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    // The configured default accepts both ids and keywords (`ij`), and an
    // unknown default must not silently fall back to the first product in
    // the table (Android Studio).
    let default_product = config
        .product_by_keyword(&config.default_product)
        .or_else(|| config.product_by_id(&config.default_product))
        .map(|product| product.id.clone());
    let product_id = explicit_product
        .or_else(|| infer_product(inference_root).map(ToString::to_string))
        .or(default_product);

    if let Some(product_id) = product_id
        && let Some(product) = config
            .products
            .iter()
            .find(|product| product.id == product_id)
    {
        let app_index = discovery::build_app_index(config);
        let bin_index = discovery::build_bin_index();
        let app = app_index.find(product, None).map(Path::to_path_buf);
        let bin = bin_index.find(product, app.as_deref());
        return Ok(Target { app, bin });
    }

    Ok(Target::default())
}

fn resolve_product_id(config: &Config, value: &str) -> Result<String> {
    Ok(config
        .product_by_keyword(value)
        .or_else(|| config.product_by_id(value))
        .ok_or_else(|| anyhow::anyhow!("unknown product: {value}"))?
        .id
        .clone())
}

/// Infer the IDE from files commonly found in a freshly cloned project.
pub fn infer_product(path: &Path) -> Option<&'static str> {
    let has = |name: &str| path.join(name).exists();

    if has("pom.xml")
        || has("build.gradle")
        || has("build.gradle.kts")
        || has("settings.gradle")
        || has("settings.gradle.kts")
    {
        return Some("idea");
    }
    if has("pyproject.toml") || has("requirements.txt") || has("setup.py") || has("Pipfile") {
        return Some("pycharm");
    }
    if has("package.json") {
        return Some("webstorm");
    }
    if has("go.mod") {
        return Some("goland");
    }
    if has("Cargo.toml") {
        return Some("rustrover");
    }
    if has("composer.json") {
        return Some("phpstorm");
    }
    if has("Gemfile") {
        return Some("rubymine");
    }
    if has("CMakeLists.txt") {
        return Some("clion");
    }
    if path.extension().and_then(|ext| ext.to_str()) == Some("sln") {
        return Some("rider");
    }
    if let Ok(read_dir) = fs::read_dir(path)
        && read_dir.flatten().any(|entry| {
            matches!(
                entry.path().extension().and_then(|ext| ext.to_str()),
                Some("sln" | "csproj")
            )
        })
    {
        return Some("rider");
    }
    if let Ok(misc) = fs::read_to_string(path.join(".idea/misc.xml")) {
        let misc = misc.to_lowercase();
        if misc.contains("python") {
            return Some("pycharm");
        }
        if misc.contains("golang") || misc.contains("go sdk") {
            return Some("goland");
        }
        if misc.contains("rust") {
            return Some("rustrover");
        }
        if misc.contains("php") {
            return Some("phpstorm");
        }
        if misc.contains("ruby") {
            return Some("rubymine");
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use tempfile::tempdir;

    use super::*;
    use crate::config::{Config, default_products};

    fn write_recent(dir: &Path, project: &Path) {
        let options = dir.join("options");
        fs::create_dir_all(&options).unwrap();
        fs::write(
            options.join("recentProjects.xml"),
            format!(
                r#"<application><component name="RecentProjectsManager"><option name="additionalInfo"><map>
                <entry key="{}"><value><RecentProjectMetaInfo><option name="projectOpenTimestamp" value="10" /></RecentProjectMetaInfo></value></entry>
                </map></option></component></application>"#,
                project.display()
            ),
        )
        .unwrap();
    }

    #[test]
    fn resolves_the_ide_from_recent_settings() {
        let temp = tempdir().unwrap();
        let settings = temp.path().join("JetBrains");
        let apps = temp.path().join("Applications");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        write_recent(&settings.join("IntelliJIdea2026.2"), &project);
        let macos = apps.join("IntelliJ IDEA.app/Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("idea"), b"#!/bin/sh\n").unwrap();

        let config = Config {
            config_path: PathBuf::from("/dev/null"),
            settings_roots: vec![settings],
            app_roots: vec![apps],
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
        };

        let target = resolve_target(&config, &project, None, None, None).unwrap();
        assert!(target.app.unwrap().ends_with("IntelliJ IDEA.app"));
        assert!(target.bin.unwrap().ends_with("Contents/MacOS/idea"));
    }

    #[test]
    fn infers_products_from_build_files() {
        let temp = tempdir().unwrap();
        let cases = [
            ("java", "pom.xml", "idea"),
            ("python", "pyproject.toml", "pycharm"),
            ("web", "package.json", "webstorm"),
            ("go", "go.mod", "goland"),
            ("rust", "Cargo.toml", "rustrover"),
            ("php", "composer.json", "phpstorm"),
            ("ruby", "Gemfile", "rubymine"),
            ("cpp", "CMakeLists.txt", "clion"),
            ("dotnet", "app.sln", "rider"),
        ];
        for (name, marker, expected) in cases {
            let dir = temp.path().join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(marker), "").unwrap();
            assert_eq!(infer_product(&dir), Some(expected), "marker {marker}");
        }
    }

    #[test]
    fn unopened_project_uses_inferred_ide() {
        let temp = tempdir().unwrap();
        let apps = temp.path().join("Applications");
        let project = temp.path().join("fresh-clone");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("pom.xml"), "").unwrap();
        let macos = apps.join("IntelliJ IDEA.app/Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("idea"), b"#!/bin/sh\n").unwrap();

        let config = Config {
            config_path: PathBuf::from("/dev/null"),
            settings_roots: vec![],
            app_roots: vec![apps],
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
        };

        let target = resolve_target(&config, &project, None, None, None).unwrap();
        assert!(target.app.unwrap().ends_with("IntelliJ IDEA.app"));
        assert!(target.bin.unwrap().ends_with("Contents/MacOS/idea"));
    }

    #[test]
    fn default_product_is_used_without_markers() {
        let temp = tempdir().unwrap();
        let apps = temp.path().join("Applications");
        let project = temp.path().join("plain");
        fs::create_dir_all(&project).unwrap();
        let macos = apps.join("PyCharm.app/Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("pycharm"), b"#!/bin/sh\n").unwrap();

        let config = Config {
            config_path: PathBuf::from("/dev/null"),
            settings_roots: vec![],
            app_roots: vec![apps],
            show_stale: false,
            hide_worktrees: true,
            sort: "recent".to_string(),
            cache_seconds: 60,
            pins_file: PathBuf::from("/dev/null"),
            workflow_name: "test".to_string(),
            default_product: "pycharm".to_string(),
            terminal: "Terminal".to_string(),
            ignore: vec![],
            forgotten_file: PathBuf::from("/dev/null"),
            products: default_products(),
        };

        let target = resolve_target(&config, &project, None, None, None).unwrap();
        assert!(target.app.unwrap().ends_with("PyCharm.app"));
        assert!(target.bin.unwrap().ends_with("Contents/MacOS/pycharm"));
    }

    fn test_config(apps: PathBuf, default_product: &str) -> Config {
        Config {
            config_path: PathBuf::from("/dev/null"),
            settings_roots: vec![],
            app_roots: vec![apps],
            show_stale: false,
            hide_worktrees: true,
            sort: "recent".to_string(),
            cache_seconds: 60,
            pins_file: PathBuf::from("/dev/null"),
            workflow_name: "test".to_string(),
            default_product: default_product.to_string(),
            terminal: "Terminal".to_string(),
            ignore: vec![],
            forgotten_file: PathBuf::from("/dev/null"),
            products: default_products(),
        }
    }

    #[test]
    fn default_product_accepts_a_keyword() {
        let temp = tempdir().unwrap();
        let apps = temp.path().join("Applications");
        let macos = apps.join("IntelliJ IDEA.app/Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("idea"), b"#!/bin/sh\n").unwrap();
        let project = temp.path().join("plain");
        fs::create_dir_all(&project).unwrap();

        let config = test_config(apps, "ij");
        let target = resolve_target(&config, &project, None, None, None).unwrap();
        assert!(target.app.unwrap().ends_with("IntelliJ IDEA.app"));
    }

    #[test]
    fn unknown_default_product_does_not_pick_the_first_product() {
        let temp = tempdir().unwrap();
        let apps = temp.path().join("Applications");
        let macos = apps.join("Android Studio.app/Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("studio"), b"#!/bin/sh\n").unwrap();
        let project = temp.path().join("plain");
        fs::create_dir_all(&project).unwrap();

        let config = test_config(apps, "typo");
        let target = resolve_target(&config, &project, None, None, None).unwrap();
        assert_eq!(target, Target::default());
    }

    #[test]
    fn explicit_bin_wins() {
        let temp = tempdir().unwrap();
        let bin = temp.path().join("bin");
        fs::write(&bin, b"#!/bin/sh\n").unwrap();
        let config = Config {
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
        };
        let target = resolve_target(&config, temp.path(), None, None, Some(bin.clone())).unwrap();
        assert_eq!(target.bin, Some(bin));
    }

    #[test]
    fn unknown_product_errors() {
        let config = Config {
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
        };
        let error =
            resolve_target(&config, Path::new("/tmp"), Some("nope"), None, None).unwrap_err();
        assert!(error.to_string().contains("unknown product"));
    }
}
