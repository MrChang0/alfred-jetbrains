use std::{fs, path::Path, process::Command};

use tempfile::TempDir;

fn write_fixture(home: &Path) {
    let settings = home.join("Library/Application Support/JetBrains/IntelliJIdea2026.2/options");
    fs::create_dir_all(&settings).unwrap();

    let alpha = home.join("projects/alpha");
    let beta = home.join("projects/beta");
    let missing = home.join("projects/missing");
    fs::create_dir_all(alpha.join(".idea")).unwrap();
    fs::write(alpha.join(".idea/.name"), "Alpha").unwrap();
    fs::create_dir_all(&beta).unwrap();

    fs::write(
        settings.join("recentProjects.xml"),
        format!(
            r#"<application><component name="RecentProjectsManager"><option name="additionalInfo"><map>
            <entry key="{}"><value><RecentProjectMetaInfo><option name="projectOpenTimestamp" value="100" /></RecentProjectMetaInfo></value></entry>
            <entry key="{}"><value><RecentProjectMetaInfo><option name="projectOpenTimestamp" value="200" /></RecentProjectMetaInfo></value></entry>
            <entry key="{}"><value><RecentProjectMetaInfo><option name="projectOpenTimestamp" value="300" /></RecentProjectMetaInfo></value></entry>
            </map></option></component></application>"#,
            alpha.display(),
            beta.display(),
            missing.display()
        ),
    )
    .unwrap();

    let macos = home.join("Applications/IntelliJ IDEA.app/Contents/MacOS");
    fs::create_dir_all(&macos).unwrap();
    fs::write(macos.join("idea"), b"#!/bin/sh\n").unwrap();
}

fn run(home: &Path, config: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_jb"))
        .env("HOME", home)
        .env("JB_CONFIG", config)
        .env("JB_BINARIES", home.join("empty-bin"))
        .args(args)
        .output()
        .unwrap()
}

fn search(home: &Path, config: &Path, filter: &str, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["search", "--filter", filter];
    args.extend_from_slice(extra);
    let output = run(home, config, &args);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn search_hides_stale_and_sorts_by_recency() {
    let temp = TempDir::new().unwrap();
    let home = temp.path();
    write_fixture(home);
    let config = home.join("config.toml");

    let json = search(home, &config, "", &[]);
    let titles = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["title"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(titles, vec!["beta", "Alpha"]);
}

#[test]
fn show_stale_includes_missing_projects() {
    let temp = TempDir::new().unwrap();
    let home = temp.path();
    write_fixture(home);
    let config = home.join("config.toml");

    let json = search(home, &config, "", &["--show-stale"]);
    assert_eq!(json["items"].as_array().unwrap().len(), 3);
    assert!(
        json["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["title"] == "missing")
    );
}

#[test]
fn pin_moves_project_to_the_top() {
    let temp = TempDir::new().unwrap();
    let home = temp.path();
    write_fixture(home);
    let config = home.join("config.toml");
    let alpha = home.join("projects/alpha");

    let output = run(home, &config, &["pin", alpha.to_str().unwrap()]);
    assert!(output.status.success());

    let json = search(home, &config, "", &[]);
    assert_eq!(json["items"][0]["title"], "★ Alpha");
    assert_eq!(json["items"][0]["variables"]["jb_pinned"], "1");
}

#[test]
fn product_prefix_scopes_results() {
    let temp = TempDir::new().unwrap();
    let home = temp.path();
    write_fixture(home);
    let config = home.join("config.toml");

    let json = search(home, &config, "idea alpha", &[]);
    assert_eq!(json["items"].as_array().unwrap().len(), 1);
    assert_eq!(json["items"][0]["title"], "Alpha");
}

#[test]
fn config_prints_effective_settings() {
    let temp = TempDir::new().unwrap();
    let home = temp.path();
    write_fixture(home);
    let config = home.join("config.toml");

    let output = run(home, &config, &["config"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["show_stale"], false);
    assert!(
        json["settings_roots"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path.as_str().unwrap().ends_with("JetBrains"))
    );
}

#[test]
fn install_writes_workflow_files() {
    let temp = TempDir::new().unwrap();
    let home = temp.path();
    write_fixture(home);
    let config = home.join("config.toml");

    let alfred = home.join("Alfred.alfredpreferences");
    fs::create_dir_all(&alfred).unwrap();
    fs::create_dir_all(home.join("Library/Application Support/Alfred")).unwrap();
    fs::write(
        home.join("Library/Application Support/Alfred/prefs.json"),
        format!(r#"{{"current":"{}"}}"#, alfred.display()),
    )
    .unwrap();

    let output = run(
        home,
        &config,
        &["install", "--workflow-name", "test-workflow", "--retain"],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let workflow = alfred.join("workflows/test-workflow");
    assert!(workflow.join("info.plist").is_file());
    assert!(workflow.join("icon.png").is_file());
    let value = plist::Value::from_file(workflow.join("info.plist")).unwrap();
    let objects = value.as_dictionary().unwrap()["objects"]
        .as_array()
        .unwrap();
    assert!(objects.len() > 2);
    assert!(objects.iter().any(|object| {
        object.as_dictionary().unwrap()["config"]
            .as_dictionary()
            .unwrap()
            .get("keyword")
            .and_then(|value| value.as_string())
            == Some("idea")
    }));
}

#[test]
fn open_spawns_the_configured_binary() {
    let temp = TempDir::new().unwrap();
    let home = temp.path();
    write_fixture(home);
    let config = home.join("config.toml");
    let project = home.join("projects/alpha");
    let marker = home.join("opened.txt");
    let fake = home.join("fake-idea");
    fs::write(
        &fake,
        format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", marker.display()),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let output = run(
        home,
        &config,
        &[
            "open",
            project.to_str().unwrap(),
            "--bin",
            fake.to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    for _ in 0..100 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        fs::read_to_string(&marker).unwrap(),
        fs::canonicalize(&project).unwrap().to_string_lossy()
    );
}

#[test]
fn install_copies_the_binary_by_default() {
    let temp = TempDir::new().unwrap();
    let home = temp.path();
    write_fixture(home);
    let config = home.join("config.toml");

    let alfred = home.join("Alfred.alfredpreferences");
    fs::create_dir_all(&alfred).unwrap();
    fs::create_dir_all(home.join("Library/Application Support/Alfred")).unwrap();
    fs::write(
        home.join("Library/Application Support/Alfred/prefs.json"),
        format!(r#"{{"current":"{}"}}"#, alfred.display()),
    )
    .unwrap();

    let output = run(
        home,
        &config,
        &["install", "--workflow-name", "test-workflow"],
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let binary = alfred.join("workflows/test-workflow/bin/jb");
    assert!(binary.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(
            fs::metadata(&binary).unwrap().permissions().mode() & 0o111,
            0
        );
    }
}
