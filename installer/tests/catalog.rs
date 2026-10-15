use std::{fs, path::Path, process::Command};
use tempfile::TempDir;
use xerrion_dev_tooling_installer::{
    catalog::Catalog,
    plan::{Plan, inside, update_managed},
};

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn fixture() -> TempDir {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "templates/mise/mise.toml",
        "[tasks.check]\ndescription = 'Check selected tools'\n[tasks.format]\ndescription = 'Format selected tools'\n[tasks.deps]\ndescription = 'Sync selected dependencies'\n",
    );
    write(
        root.path(),
        "templates/catalog/demo.toml",
        r#"
schema_version = 1
title = "Demo tooling"
profile = "mise/profiles/demo.toml"
aliases = ["example"]
install_dependencies = ["deps:demo"]
[aggregates]
check = ["check:demo"]
deps = ["deps:demo"]
[[configs]]
source = "demo/demo.json"
alternatives = ["demo.json", ".demo.json"]
[[guidance]]
task = "check:demo"
when = "After demo edits, run its checks."
"#,
    );
    write(
        root.path(),
        "templates/mise/profiles/demo.toml",
        "[tools]\ndemo = '1'\n[tasks.'check:demo']\nrun = 'demo check'\ndir = '{{ vars.demo_dir }}'\n[tasks.'deps:demo']\nrun = 'demo sync'\n",
    );
    write(
        root.path(),
        "templates/demo/demo.json",
        "{\"enabled\": true}\n",
    );
    fs::create_dir_all(root.path().join("project/subdir")).unwrap();
    root
}
fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_installer"))
        .arg(root.join("project"))
        .arg("--templates")
        .arg(root.join("templates"))
        .args(args)
        .env("PATH", "")
        .output()
        .unwrap()
}
#[test]
fn new_tool_is_loaded_by_the_same_binary_without_recompilation() {
    let root = fixture();
    let preview = run(
        root.path(),
        &[
            "--tools",
            "example",
            "--dir",
            "demo=subdir",
            "--install-deps",
        ],
    );
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(!root.path().join("project/mise.toml").exists());
    assert!(!root.path().join("project/subdir/demo.json").exists());
    assert!(String::from_utf8_lossy(&preview.stdout).contains("mise run deps:demo"));
    let applied = run(
        root.path(),
        &["--tools", "example", "--demo-dir", "subdir", "--apply"],
    );
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert!(root.path().join("project/subdir/demo.json").is_file());
    let root_mise = fs::read_to_string(root.path().join("project/mise.toml")).unwrap();
    let data: toml::Value = toml::from_str(&root_mise).unwrap();
    assert_eq!(
        data["tasks"]["check"]["depends"][0].as_str(),
        Some("check:demo")
    );
    let agents = fs::read_to_string(root.path().join("project/AGENTS.md")).unwrap();
    assert!(agents.contains("After demo edits, run its checks. Use `mise run check:demo`."));
    let rerun = run(
        root.path(),
        &["--tools", "example", "--demo-dir", "subdir", "--apply"],
    );
    assert!(rerun.status.success());
    assert!(String::from_utf8_lossy(&rerun.stdout).contains("Applied 0 file changes."));
    assert!(!root.path().join("project/.tooling-backups").exists());
}
#[test]
fn malformed_catalog_fails_without_writing_target() {
    for replacement in [
        ("schema_version = 1", "schema_version = 2"),
        (
            "title = \"Demo tooling\"",
            "unknown_option = true\ntitle = \"Demo tooling\"",
        ),
        ("demo/demo.json", "../outside.json"),
        (
            "title = \"Demo tooling\"",
            "adapter = 'python'\ntitle = \"Demo tooling\"",
        ),
        ("check = [\"check:demo\"]", "check = [\"missing:demo\"]"),
    ] {
        let root = fixture();
        let path = root.path().join("templates/catalog/demo.toml");
        let text = fs::read_to_string(&path).unwrap();
        fs::write(path, text.replace(replacement.0, replacement.1)).unwrap();
        let result = run(root.path(), &["--tools", "demo", "--apply"]);
        assert!(!result.status.success());
        assert!(!root.path().join("project/mise.toml").exists());
        assert_eq!(
            fs::read_dir(root.path().join("project")).unwrap().count(),
            1
        );
    }
}

#[test]
fn catalog_declares_dependency_files_to_back_up() {
    let root = fixture();
    let path = root.path().join("templates/catalog/demo.toml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("dependency_files = ['demo.lock']\n{text}")).unwrap();
    write(root.path(), "project/demo.lock", "original lock");
    let catalog = Catalog::load(&root.path().join("templates")).unwrap();
    let matches = xerrion_dev_tooling_installer::cli::command(&catalog)
        .try_get_matches_from([
            "installer",
            root.path().join("project").to_str().unwrap(),
            "--tools",
            "demo",
            "--install-deps",
        ])
        .unwrap();
    let options = xerrion_dev_tooling_installer::cli::options(&matches, &catalog).unwrap();
    let plan = xerrion_dev_tooling_installer::build_plan(&options, &catalog).unwrap();
    assert_eq!(
        plan.dependency_backups[Path::new("demo.lock")],
        b"original lock"
    );
    assert!(!root.path().join("project/.tooling-backups").exists());
}

#[test]
fn conflicting_config_templates_fail_before_writes() {
    let root = fixture();
    write(
        root.path(),
        "templates/catalog/second.toml",
        r#"
schema_version = 1
title = "Second"
profile = "mise/profiles/demo.toml"
[[configs]]
source = "second/demo.json"
"#,
    );
    write(
        root.path(),
        "templates/second/demo.json",
        "{\"enabled\": false}\n",
    );
    let result = run(root.path(), &["--tools", "demo", "second", "--apply"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Conflicting config"));
    assert!(!root.path().join("project/demo.json").exists());
}
#[test]
fn duplicate_alias_is_rejected() {
    let root = fixture();
    let text = fs::read_to_string(root.path().join("templates/catalog/demo.toml")).unwrap();
    write(root.path(), "templates/catalog/second.toml", &text);
    let result = Catalog::load(&root.path().join("templates"));
    assert!(result.err().unwrap().to_string().contains("alias"));
}
#[test]
fn plan_detects_concurrent_edits_before_any_write() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "existing.txt", "before");
    let mut plan = Plan::new(root.path().canonicalize().unwrap());
    plan.stage("a-new.txt", "new").unwrap();
    plan.stage("existing.txt", "after").unwrap();
    write(root.path(), "existing.txt", "concurrent edit");
    assert!(
        plan.apply()
            .unwrap_err()
            .to_string()
            .contains("changed during planning")
    );
    assert!(!root.path().join("a-new.txt").exists());
    assert!(!root.path().join(".tooling-backups").exists());
}
#[test]
fn changed_dependency_aborts_before_config_writes() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "package.json", "{}");
    let mut plan = Plan::new(root.path().canonicalize().unwrap());
    plan.stage("a-new.txt", "new").unwrap();
    plan.dependency_backups
        .insert("package.json".into(), b"{}".to_vec());
    write(root.path(), "package.json", "{\"changed\":true}");
    assert!(plan.apply().is_err());
    assert!(!root.path().join("a-new.txt").exists());
}
#[test]
fn parent_traversal_and_malformed_markers_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    assert!(inside(root.path(), "../outside").is_err());
    for text in ["start", "end start", "start end start end"] {
        assert!(update_managed(text, "replacement", "start", "end").is_err());
    }
}
#[cfg(unix)]
#[test]
fn symlink_added_after_planning_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let mut plan = Plan::new(root.path().canonicalize().unwrap());
    plan.stage("nested/config.txt", "content").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("nested")).unwrap();
    assert!(plan.apply().is_err());
    assert!(!outside.path().join("config.txt").exists());
}
#[test]
fn relocated_binary_accepts_external_templates_without_python() {
    let root = fixture();
    let binary = root.path().join("standalone-installer");
    fs::copy(env!("CARGO_BIN_EXE_installer"), &binary).unwrap();
    let output = Command::new(binary)
        .args(["--list-tools", "--templates"])
        .arg(root.path().join("templates"))
        .env("PATH", "")
        .current_dir(root.path().join("project"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("demo: Demo tooling"));
}
