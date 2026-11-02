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

    let product_id = match product {
        Some(value) => Some(
            config
                .product_by_keyword(value)
                .or_else(|| config.product_by_id(value))
                .ok_or_else(|| anyhow::anyhow!("unknown product: {value}"))?
                .id
                .clone(),
        ),
        None => None,
    };

    let key = dedupe_key(path);
    let entries = discovery::discover(config);
    let entry = entries
        .into_iter()
        .filter(|entry| entry.key == key)
        .filter(|entry| {
            product_id
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

    if let Some(product_id) = product_id {
        let product = config
            .products
            .iter()
            .find(|product| product.id == product_id)
            .ok_or_else(|| anyhow::anyhow!("unknown product: {product_id}"))?;
        let app_index = discovery::build_app_index(config);
        let bin_index = discovery::build_bin_index();
        let app = app_index.find(product, None).map(Path::to_path_buf);
        let bin = bin_index.find(product, app.as_deref());
        return Ok(Target { app, bin });
    }

    Ok(Target::default())
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
            cache_seconds: 60,
            pins_file: PathBuf::from("/dev/null"),
            workflow_name: "test".to_string(),
            products: default_products(),
        };

        let target = resolve_target(&config, &project, None, None, None).unwrap();
        assert!(target.app.unwrap().ends_with("IntelliJ IDEA.app"));
        assert!(target.bin.unwrap().ends_with("Contents/MacOS/idea"));
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
            cache_seconds: 60,
            pins_file: PathBuf::from("/dev/null"),
            workflow_name: "test".to_string(),
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
            cache_seconds: 60,
            pins_file: PathBuf::from("/dev/null"),
            workflow_name: "test".to_string(),
            products: default_products(),
        };
        let error =
            resolve_target(&config, Path::new("/tmp"), Some("nope"), None, None).unwrap_err();
        assert!(error.to_string().contains("unknown product"));
    }
}
