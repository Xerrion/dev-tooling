#[allow(dead_code)]
mod support;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use support::*;
use xerrion_dev_tooling_installer::{catalog::Catalog, detection::detect};

#[test]
fn detects_nested_stacks_in_stable_order_without_writes() {
    let f = Fixture::new();
    f.write("web/package.json", "{}");
    f.write("engine/Cargo.toml", "[workspace]\nmembers = []\n");
    f.write(
        "engine/crates/example/Cargo.toml",
        "[package]\nname = 'example'\n",
    );
    f.write(
        "api/pyproject.toml",
        "[project]\nname = 'api'\nversion = '1'\n",
    );
    f.write("docs/intro.md", "# Intro\n");
    f.write("addon/main.lua", "return {}\n");
    f.write("scripts/deploy.sh", "#!/bin/sh\n");
    f.write("containers/Dockerfile.dev", "FROM scratch\n");
    let before = snapshot(&f.root);
    let catalog = Catalog::load(&templates()).unwrap();
    let first = detect(&f.project, &catalog).unwrap();
    assert_eq!(first, detect(&f.project, &catalog).unwrap());
    assert_eq!(
        first.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "docker",
            "javascript",
            "lua",
            "markdown",
            "python",
            "rust",
            "shell"
        ]
    );
    for (id, directory) in [
        ("docker", "containers"),
        ("javascript", "web"),
        ("lua", "addon"),
        ("markdown", "docs"),
        ("python", "api"),
        ("rust", "engine"),
        ("shell", "scripts"),
    ] {
        assert_eq!(first[id].directories, [Path::new(directory)]);
    }
    let output = f.succeed(&["--detect"]);
    assert_eq!(output, f.succeed(&["--detect"]));
    assert!(output.contains("DETECTED docker: containers/Dockerfile.dev"));
    assert_eq!(snapshot(&f.root), before);
    f.succeed(&["--detect", "--apply"]);
    let data = generated(&f.project);
    assert_eq!(data["vars"]["docker_file"], "Dockerfile.dev");
    assert_eq!(data["vars"]["docker_files"], "Dockerfile.dev");
    assert_eq!(data["vars"]["javascript_dir"], "web");
    assert_eq!(data["vars"]["rust_dir"], "engine");
    let before = snapshot(&f.root);
    f.succeed(&["--detect", "--apply"]);
    assert_eq!(snapshot(&f.root), before);
}

#[test]
fn scan_skips_dependencies_build_output_hidden_directories_and_symlinks() {
    let f = Fixture::new();
    for directory in [
        "node_modules/pkg",
        "target",
        "vendor",
        "dist",
        "build",
        "coverage",
        "__pycache__",
        "venv",
        ".venv",
        ".git",
        ".tooling-backups",
    ] {
        f.write(format!("{directory}/package.json"), "{}");
        f.write(format!("{directory}/file.py"), "pass\n");
    }
    #[cfg(unix)]
    {
        write(&f.outside, "Cargo.toml", "[workspace]\n");
        std::os::unix::fs::symlink(&f.outside, f.project.join("linked")).unwrap();
        std::os::unix::fs::symlink(f.outside.join("Cargo.toml"), f.project.join("Cargo.toml"))
            .unwrap();
    }
    assert!(
        detect(&f.project, &Catalog::load(&templates()).unwrap())
            .unwrap()
            .is_empty()
    );
    f.refuse(&["--detect", "--apply"]);
}

#[test]
fn scan_ignores_projects_below_two_nested_directory_levels() {
    let f = Fixture::new();
    f.write("apps/web/package.json", "{}");
    f.write("apps/team/mobile/package.json", "{}");

    let stack = detect(&f.project, &Catalog::load(&templates()).unwrap()).unwrap();

    assert_eq!(stack["javascript"].directories, [Path::new("apps/web")]);
}

#[test]
fn non_utf8_markdown_remains_filename_evidence() {
    let f = Fixture::new();
    f.write("docs/example.md", [0xff, 0xfe]);
    let stack = detect(&f.project, &Catalog::load(&templates()).unwrap()).unwrap();
    assert_eq!(stack["markdown"].directories, [Path::new("docs")]);
}

#[cfg(unix)]
#[test]
fn noninteractive_detection_sanitizes_filename_evidence() {
    let f = Fixture::new();
    f.write("docs/\x1b]0;spoofed\x07.md", "# Document\n");

    let output = f.succeed(&["--detect"]);

    assert!(!output.contains('\x1b'));
    assert!(!output.contains('\x07'));
    assert!(output.contains("DETECTED markdown: docs/]0;spoofed.md"));

    let f = Fixture::new();
    f.write("apps/\x1b]0;spoofed\x07/package.json", "{}");
    f.write("apps/other/package.json", "{}");

    let (_, output) = f.invoke(&["--detect"]);

    assert!(!output.contains('\x1b'));
    assert!(!output.contains('\x07'));
    assert!(output.contains("Multiple javascript project directories detected"));
    assert!(output.contains("apps/]0;spoofed, apps/other"));
}

#[test]
fn independent_project_directories_require_an_explicit_choice() {
    let f = Fixture::new();
    f.write("apps/one/package.json", "{}");
    f.write("apps/two/package.json", "{}");
    assert!(
        f.refuse(&["--detect", "--apply"])
            .contains("Multiple javascript project directories")
    );
    f.succeed(&["--detect", "--javascript-dir", "apps/two", "--apply"]);
    assert_eq!(generated(&f.project)["vars"]["javascript_dir"], "apps/two");
}

#[test]
fn root_manifest_wins_over_nested_manifests() {
    let f = Fixture::new();
    f.write("package.json", "{}");
    f.write("apps/web/package.json", "{}");
    f.write("src/main.ts", "export {};\n");
    let stack = detect(&f.project, &Catalog::load(&templates()).unwrap()).unwrap();
    assert_eq!(stack["javascript"].directories, [Path::new(".")]);
    f.succeed(&["--detect", "--apply"]);
    assert_eq!(generated(&f.project)["vars"]["javascript_dir"], ".");
}

#[test]
fn source_fallback_and_nested_shell_files_use_all_detected_paths() {
    let f = Fixture::new();
    f.write("src/report.py", "pass\n");
    f.write("tests/test_report.py", "pass\n");
    f.write("scripts/deploy.sh", "#!/bin/sh\n");
    f.write("scripts/lib/check file.bash", "#!/bin/bash\n");
    f.succeed(&["--detect", "--apply"]);
    let data = generated(&f.project);
    assert_eq!(data["vars"]["python_dir"], ".");
    assert_eq!(data["vars"]["shell_dir"], "scripts");
    assert_eq!(
        shlex::split(data["vars"]["shell_files"].as_str().unwrap()).unwrap(),
        ["deploy.sh", "lib/check file.bash"]
    );
    assert!(data["tasks"].get("test:python").is_none());
}

#[test]
fn existing_mise_directory_and_file_settings_take_precedence() {
    let f = Fixture::new();
    f.write("containers/Dockerfile", "FROM scratch\n");
    f.write("other/Dockerfile.dev", "FROM scratch\n");
    f.write("mise.toml", "[vars]\ndocker_dir = 'other'\ndocker_file = 'Dockerfile.dev'\ndocker_files = 'Dockerfile.dev'\n");
    f.succeed(&["--detect", "--apply"]);
    assert!(generated(&f.project)["vars"].get("docker_dir").is_none());
    assert!(generated(&f.project)["vars"].get("docker_file").is_none());
}

fn interactive(f: &Fixture, args: &[&str], input: &str) -> (bool, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_installer"))
        .arg(&f.project)
        .arg("--templates")
        .arg(templates())
        .arg("--interactive")
        .args(args)
        .env("PATH", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    (
        result.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ),
    )
}

#[test]
fn interactive_accepts_detection_and_applies_only_after_the_plan() {
    let f = Fixture::new();
    f.write("Dockerfile", "FROM scratch\n");
    let (success, output) = interactive(&f, &[], "\n\nn\ny\n");
    assert!(success, "{output}");
    assert!(output.find("Files:").unwrap() < output.find("Apply this setup?").unwrap());
    assert!(f.project.join("mise.toml").exists());
    assert!(generated(&f.project)["tasks"].get("run:docker").is_some());
}

#[test]
fn interactive_decline_eof_and_invalid_answers_never_write() {
    for input in ["\n\nn\nn\n", "", "\n\nn\n", "unknown\n", "\n\nn\nmaybe\n"] {
        let f = Fixture::new();
        f.write("README.md", "# Project\n");
        let before = snapshot(&f.root);
        let (_, output) = interactive(&f, &[], input);
        assert!(!output.contains("Applied "));
        assert_eq!(snapshot(&f.root), before);
    }
}

#[test]
fn interactive_manual_alias_selection_and_dry_run() {
    let f = Fixture::new();
    fs::create_dir(f.project.join("containers")).unwrap();
    let before = snapshot(&f.root);
    let (success, output) = interactive(&f, &["--dry-run"], "hadolint\ncontainers\nn\n");
    assert!(success, "{output}");
    assert!(output.contains("check:docker") || output.contains("tooling.toml"));
    assert!(!output.contains("Apply this setup?"));
    assert_eq!(snapshot(&f.root), before);
}

#[test]
fn interactive_recovers_from_selection_directory_and_confirmation_typos() {
    let f = Fixture::new();
    f.write("Dockerfile", "FROM scratch\n");
    let before = snapshot(&f.root);
    let (success, output) = interactive(&f, &[], "typo\n1\nmissing\n.\nmaybe\nn\ndetails\nn\n");
    assert!(success, "{output}");
    assert!(output.contains("Unknown selection: typo"));
    assert!(output.contains("That directory does not exist"));
    assert!(output.contains("Choose y or n"));
    assert!(output.contains("Plan details"));
    assert!(!output.contains('\u{1b}'));
    assert_eq!(snapshot(&f.root), before);
}

#[test]
fn interactive_numbered_selection_applies_only_the_chosen_tools() {
    let f = Fixture::new();
    f.write("Dockerfile", "FROM scratch\n");
    let (success, output) = interactive(&f, &[], "1,7\n.\n.\nn\ny\n");
    assert!(success, "{output}");
    let data = generated(&f.project);
    assert!(data["tasks"].get("check:docker").is_some());
    assert!(data["tasks"].get("check:shell").is_some());
    assert!(data["tasks"].get("check:markdown").is_none());
    assert!(output.contains("Setup complete"));
}

#[test]
fn interactive_cancellation_and_unsafe_directory_retry_do_not_write() {
    for input in [
        "q\n",
        "\nq\n",
        "\n.\nq\n",
        "\n.\nn\nq\n",
        "\n../outside\n.\nn\nn\n",
    ] {
        let f = Fixture::new();
        f.write("Dockerfile", "FROM scratch\n");
        let before = snapshot(&f.root);
        let (success, output) = interactive(&f, &[], input);
        assert!(success, "{output}");
        assert!(!output.contains("Applied "));
        assert_eq!(snapshot(&f.root), before);
    }
}

#[test]
fn interactive_explicit_installation_in_dry_run_only_displays_commands() {
    let f = Fixture::new();
    f.write("Dockerfile", "FROM scratch\n");
    let before = snapshot(&f.root);
    let (success, output) = interactive(&f, &["--install-tools", "--dry-run"], "\n\n");
    assert!(success, "{output}");
    assert!(output.contains("mise install"));
    assert!(!output.contains("Apply this setup?"));
    assert_eq!(snapshot(&f.root), before);
}

#[test]
fn noninteractive_missing_selection_and_conflicting_modes_fail_without_writes() {
    let f = Fixture::new();
    f.write("README.md", "# Project\n");
    assert!(f.refuse(&["--apply"]).contains("Use --detect"));
    for args in [
        vec!["--detect", "--tools", "markdown"],
        vec!["--detect", "--interactive"],
    ] {
        let before = snapshot(&f.root);
        let (code, _) = f.invoke(&args);
        assert_ne!(code, 0);
        assert_eq!(snapshot(&f.root), before);
    }
}
