mod support;
use serde_json::{Value, json};
use std::{fs, path::Path};
use support::*;
use xerrion_dev_tooling_installer::plan::{AGGREGATE_END, AGGREGATE_START, END, FRAGMENT, START};

#[test]
fn test_default_preview_does_not_write() {
    let f = Fixture::new();
    python_project(&f.project, true);
    let before = snapshot(&f.root);
    let output = f.succeed(&["--tools", "python"]);
    assert!(output.contains("tooling.toml"));
    assert_eq!(snapshot(&f.root), before);
}
#[test]
fn test_explicit_dry_run_does_not_write() {
    let f = Fixture::new();
    python_project(&f.project, true);
    let before = snapshot(&f.root);
    f.succeed(&["--tools", "python", "--dry-run"]);
    assert_eq!(snapshot(&f.root), before);
}

#[test]
fn shellcheck_profile_respects_directory_selection_and_preserves_scripts() {
    for (selection, directory, extra) in [
        ("shell", ".", vec![]),
        ("shell", "scripts", vec!["--shell-dir", "scripts"]),
        (
            "shellcheck",
            "release scripts",
            vec!["--dir", "shell=release scripts"],
        ),
    ] {
        let f = Fixture::new();
        let relative = Path::new(directory).join("check script.sh");
        let script = "#!/bin/sh\nprintf '%s\\n' 'Checked'\n";
        let config = "# Project ShellCheck settings\nshell=sh\n";
        f.write(&relative, script);
        f.write(".shellcheckrc", config);
        assert!(f.succeed(&["--help"]).contains("--shell-dir"));
        assert!(f.succeed(&["--list-tools"]).contains("shell: Shell"));

        let args = [vec!["--tools", selection], extra].concat();
        let before = snapshot(&f.root);
        let preview =
            f.succeed(&[args.clone(), vec!["--install-tools", "--install-deps"]].concat());
        assert!(preview.contains("mise install"));
        assert_eq!(snapshot(&f.root), before);

        let apply = [args, vec!["--apply"]].concat();
        f.succeed(&apply);
        let data = generated(&f.project);
        assert_eq!(data["vars"]["shell_dir"], directory);
        assert_eq!(data["vars"]["shell_files"], "*.sh");
        assert!(data["tools"].get("shellcheck").is_some());
        assert_eq!(data["tasks"]["check:shell"]["dir"], "{{ vars.shell_dir }}");
        assert_central_dependencies(&f.project, &["shell"], &[]);
        let central = central_tasks(&f.project);
        for name in ["format", "test", "deps"] {
            assert!(central.get(name).is_none());
        }
        assert!(read(&f.project, "AGENTS.md").contains("mise run check:shell"));
        assert_eq!(read(&f.project, &relative), script);
        assert_eq!(read(&f.project, ".shellcheckrc"), config);
        assert!(backups(&f.project, ".shellcheckrc").is_empty());
        let before = snapshot(&f.root);
        f.succeed(&apply);
        assert_eq!(snapshot(&f.root), before);
    }
}

#[test]
fn docker_profile_respects_directory_selection_and_preserves_dockerfiles() {
    for (selection, directory, extra) in [
        ("docker", ".", vec![]),
        ("docker", "docker", vec!["--docker-dir", "docker"]),
        (
            "hadolint",
            "container files",
            vec!["--dir", "hadolint=container files"],
        ),
    ] {
        let f = Fixture::new();
        let relative = Path::new(directory).join("Dockerfile");
        let dockerfile = "FROM alpine:3.22\nCMD [\"/bin/sh\"]\n";
        let config_path = Path::new(directory).join(".hadolint.yaml");
        let config = "ignored:\n  - DL3007\n";
        f.write(&relative, dockerfile);
        f.write(&config_path, config);
        assert!(f.succeed(&["--help"]).contains("--docker-dir"));
        assert!(f.succeed(&["--list-tools"]).contains("docker: Docker"));

        let args = [vec!["--tools", selection], extra].concat();
        let before = snapshot(&f.root);
        let preview =
            f.succeed(&[args.clone(), vec!["--install-tools", "--install-deps"]].concat());
        assert!(preview.contains("mise install"));
        assert!(!preview.contains("mise run deps:docker"));
        assert_eq!(snapshot(&f.root), before);

        let apply = [args, vec!["--apply"]].concat();
        f.succeed(&apply);
        let data = generated(&f.project);
        assert_eq!(data["vars"]["docker_dir"], directory);
        assert_eq!(data["vars"]["docker_files"], "Dockerfile");
        assert!(data["tools"].get("hadolint").is_some());
        assert_eq!(
            data["tasks"]["check:docker"]["dir"],
            "{{ vars.docker_dir }}"
        );
        assert_central_dependencies(&f.project, &["docker"], &[]);
        let central = central_tasks(&f.project);
        for name in ["format", "test", "deps"] {
            assert!(central.get(name).is_none());
        }
        assert!(read(&f.project, "AGENTS.md").contains("mise run check:docker"));
        assert_eq!(read(&f.project, &relative), dockerfile);
        assert_eq!(read(&f.project, &config_path), config);
        assert!(backups(&f.project, &config_path).is_empty());
        let before = snapshot(&f.root);
        f.succeed(&apply);
        assert_eq!(snapshot(&f.root), before);
    }
}

#[test]
fn docker_combines_with_other_tools_and_preserves_mise_overrides() {
    let f = Fixture::new();
    f.write("containers/Dockerfile", "FROM scratch\n");
    let original = "[tools]\nhadolint = '2.12.0'\n[vars]\ndocker_dir = 'containers'\ndocker_files = \"Dockerfile 'api service/Dockerfile'\"\n";
    f.write("mise.toml", original);
    let args = ["--tools", "docker", "shell", "markdown", "--apply"];
    f.succeed(&args);
    assert_root_preserved(&root_mise(&f.project), original.as_bytes());
    let root = toml_file(root_mise(&f.project));
    assert_eq!(root["tools"]["hadolint"], "2.12.0");
    assert_eq!(root["vars"]["docker_dir"], "containers");
    assert_eq!(
        root["vars"]["docker_files"],
        "Dockerfile 'api service/Dockerfile'"
    );
    assert!(generated(&f.project)["vars"].get("docker_dir").is_none());
    assert_central_dependencies(&f.project, &["docker", "shell", "markdown"], &[]);
    let before = snapshot(&f.root);
    f.succeed(&args);
    assert_eq!(snapshot(&f.root), before);
}

#[test]
fn test_python_project_preserves_embedded_configs_and_manifests() {
    let f = Fixture::new();
    let originals = python_project(&f.project, true);
    f.succeed(&["--tools", "python", "--apply"]);
    let data = generated(&f.project);
    assert_eq!(python_targets(&data), ["src", "tests"]);
    assert_eq!(data["vars"]["python_dir"], ".");
    assert!(data["tasks"].get("deps:python").is_some());
    assert!(
        data["tasks"]["test:python"]["run"]
            .as_str()
            .unwrap()
            .contains("pytest")
    );
    assert!(data["tasks"].get("check:python").is_some());
    for name in ["ruff.toml", ".ruff.toml", "ty.toml"] {
        assert!(!f.project.join(name).exists());
    }
    assert_preserved(&f.project, &originals);
    assert_central_dependencies(&f.project, &["python"], &[]);
}
#[test]
fn test_apply_preserves_agent_surroundings_and_is_idempotent() {
    let f = Fixture::new();
    let prefix = "# Project guidance\n\nKeep these local instructions exactly.\n\n";
    let suffix = "\n\n## Local workflow\nDo not change this trailing text.\n";
    let original = format!("{prefix}{START}\nobsolete generated guidance\n{END}{suffix}");
    f.write("AGENTS.md", &original);
    f.succeed(&["--tools", "lua", "--apply"]);
    let text = read(&f.project, "AGENTS.md");
    assert!(text.starts_with(prefix));
    assert!(text.ends_with(suffix));
    assert_eq!(text.matches(START).count(), 1);
    assert_eq!(text.matches(END).count(), 1);
    assert!(!text.contains("obsolete generated guidance"));
    assert_backup(&f.project, "AGENTS.md", original);
    let before = snapshot(&f.root);
    f.succeed(&["--tools", "lua", "--apply"]);
    assert_eq!(snapshot(&f.root), before);
}
#[test]
fn test_agent_file_default_selection_and_preservation() {
    for existing in [
        vec![],
        vec!["AGENS.md"],
        vec!["AGENTS.md"],
        vec!["AGENS.md", "AGENTS.md"],
    ] {
        let f = Fixture::new();
        for name in &existing {
            f.write(name, format!("# Original {name}\n"));
        }
        f.succeed(&["--tools", "lua", "--apply"]);
        let text = read(&f.project, "AGENTS.md");
        assert_eq!(text.matches(START).count(), 1);
        assert_eq!(text.matches(END).count(), 1);
        if existing.contains(&"AGENTS.md") {
            assert!(text.starts_with("# Original AGENTS.md\n"));
        }
        if existing.contains(&"AGENS.md") {
            assert_eq!(read(&f.project, "AGENS.md"), "# Original AGENS.md\n");
        } else {
            assert!(!f.project.join("AGENS.md").exists());
        }
    }
}
#[test]
fn test_explicit_agent_file() {
    for relative in ["notes/TEAM.md", "AGENS.md"] {
        let f = Fixture::new();
        f.write(relative, "# Team-specific instructions\n");
        f.succeed(&["--tools", "lua", "--agents-file", relative, "--apply"]);
        let text = read(&f.project, relative);
        assert!(text.starts_with("# Team-specific instructions\n"));
        assert_eq!(text.matches(START).count(), 1);
        assert_eq!(text.matches(END).count(), 1);
        assert!(!f.project.join("AGENTS.md").exists());
        if relative != "AGENS.md" {
            assert!(!f.project.join("AGENS.md").exists());
        }
    }
}
#[test]
fn test_malformed_or_duplicate_markers_fail_before_any_writes() {
    for markers in [
        vec![START],
        vec![END],
        vec![END, START],
        vec![START, START, END],
        vec![START, END, END],
        vec![START, END, START, END],
        vec![START, START, END, END],
    ] {
        let f = Fixture::new();
        f.write(
            "AGENTS.md",
            format!("# Local instructions\n{}\n", markers.join("\n")),
        );
        f.write("stylua.toml", "syntax = 'Lua54'\n");
        f.refuse(&["--tools", "lua", "--replace-configs", "--apply"]);
    }
}
#[test]
fn test_replace_configs_backs_up_changed_files_at_relative_paths() {
    let f = Fixture::new();
    python_project(&f.project.join("backend"), false);
    let originals = files(
        &f.project,
        &[
            ("backend/ruff.toml", "line-length = 79\n"),
            (
                "backend/ty.toml",
                "[environment]\npython-version = '3.10'\n",
            ),
        ],
    );
    let args = [
        "--tools",
        "python",
        "--python-dir",
        "backend",
        "--python-version",
        "3.13",
        "--replace-configs",
        "--apply",
    ];
    f.succeed(&args);
    for (rel, original) in originals {
        assert_ne!(fs::read(f.project.join(&rel)).unwrap(), original);
        toml_file(f.project.join(&rel));
        assert_backup(&f.project, rel, original);
    }
    let before = snapshot(&f.root);
    f.succeed(&args);
    assert_eq!(snapshot(&f.root), before);
}
#[test]
fn test_replace_configs_allows_standalone_replacement_of_embedded_settings() {
    let f = Fixture::new();
    let originals = python_project(&f.project, true);
    f.succeed(&["--tools", "python", "--replace-configs", "--apply"]);
    assert!(f.project.join("ruff.toml").is_file());
    assert!(f.project.join("ty.toml").is_file());
    assert_preserved(&f.project, &originals);
}
#[test]
fn test_changed_generated_fragment_is_backed_up() {
    let f = Fixture::new();
    f.succeed(&["--tools", "lua", "--apply"]);
    let changed = read(&f.project, FRAGMENT) + "\n# Locally edited generated file\n";
    f.write(FRAGMENT, &changed);
    f.succeed(&["--tools", "lua", "--apply"]);
    assert!(!read(&f.project, FRAGMENT).contains("Locally edited generated file"));
    assert_backup(&f.project, FRAGMENT, changed);
}
#[test]
fn test_existing_standalone_and_alternate_configs_are_not_shadowed() {
    let cases = [
        ("python", "ruff.toml", "ruff.toml", "line-length = 79\n"),
        ("python", ".ruff.toml", "ruff.toml", "line-length = 79\n"),
        (
            "python",
            "ty.toml",
            "ty.toml",
            "[environment]\npython-version = '3.12'\n",
        ),
        (
            "javascript",
            "biome.json",
            "biome.json",
            "{\"formatter\":{\"lineWidth\":100}}\n",
        ),
        (
            "javascript",
            "biome.jsonc",
            "biome.json",
            "{\n// local config\n\"root\":true\n}\n",
        ),
        ("lua", "stylua.toml", "stylua.toml", "syntax = 'Lua54'\n"),
        ("lua", ".stylua.toml", "stylua.toml", "syntax = 'Lua54'\n"),
        ("rust", "rustfmt.toml", "rustfmt.toml", "max_width = 80\n"),
        ("rust", ".rustfmt.toml", "rustfmt.toml", "max_width = 80\n"),
        (
            "rust",
            "clippy.toml",
            "clippy.toml",
            "too-many-arguments-threshold = 9\n",
        ),
        (
            "rust",
            ".clippy.toml",
            "clippy.toml",
            "too-many-arguments-threshold = 9\n",
        ),
        (
            "markdown",
            ".markdownlint.yaml",
            ".markdownlint-cli2.jsonc",
            "MD013: false\n",
        ),
        (
            "markdown",
            ".prettierrc",
            ".prettierrc.json",
            "{\"proseWrap\":\"always\"}\n",
        ),
        (
            "markdown",
            "prettier.config.js",
            ".prettierrc.json",
            "export default {};\n",
        ),
    ];
    for (tool, existing, canonical, text) in cases {
        let f = Fixture::new();
        match tool {
            "python" => {
                python_project(&f.project, false);
            }
            "javascript" => {
                f.write("package.json", "{}\n");
            }
            "rust" => {
                f.write(
                    "Cargo.toml",
                    "[package]\nname = 'sample'\nversion = '0.1.0'\nedition = '2021'\n",
                );
                f.write("src/lib.rs", "pub fn answer() -> u32 { 42 }\n");
            }
            _ => (),
        }
        f.write(existing, text);
        f.succeed(&["--tools", tool, "--apply"]);
        assert_eq!(read(&f.project, existing), text);
        assert!(backups(&f.project, existing).is_empty());
        if existing != canonical {
            assert!(
                !f.project.join(canonical).exists(),
                "{tool}: {existing} was shadowed"
            );
        }
    }
}

#[test]
fn test_monorepo_merges_profiles_preserves_pins_and_delegates_root_tasks() {
    let f = Fixture::new();
    let originals = monorepo(&f.project);
    let args = [
        "--tools",
        "python",
        "typescript",
        "rust",
        "lua",
        "markdown",
        "--python-dir",
        "scripts",
        "--javascript-dir",
        "web",
        "--rust-dir",
        "engine",
        "--lua-dir",
        "lua",
        "--markdown-dir",
        "docs",
        "--apply",
    ];
    f.succeed(&args);
    let data = generated(&f.project);
    for (language, directory) in [
        ("python", "scripts"),
        ("javascript", "web"),
        ("rust", "engine"),
        ("lua", "lua"),
        ("markdown", "docs"),
    ] {
        assert_eq!(data["vars"][format!("{language}_dir")], directory);
        assert!(data["tasks"].get(format!("check:{language}")).is_some());
        assert!(data["tasks"].get(format!("format:{language}")).is_some());
    }
    for name in ["typecheck", "test"] {
        let task = &data["tasks"][format!("{name}:javascript")];
        assert_eq!(task["depends"], json!([name]));
        assert!(task.get("run").is_none());
    }
    assert!(data["tasks"].get("deps:python").is_none());
    assert!(data["tasks"].get("test:python").is_none());
    assert_preserved(&f.project, &originals);
    for rel in [
        "scripts/ruff.toml",
        "scripts/ty.toml",
        "web/biome.json",
        "engine/rustfmt.toml",
        "engine/clippy.toml",
        "lua/stylua.toml",
        "lua/.styluaignore",
        "docs/.markdownlint-cli2.jsonc",
        "docs/.prettierrc.json",
        "docs/.prettierignore",
    ] {
        assert!(f.project.join(rel).is_file(), "{rel}");
    }
    for rel in [
        "ruff.toml",
        "biome.json",
        "rustfmt.toml",
        "stylua.toml",
        ".markdownlint-cli2.jsonc",
    ] {
        assert!(!f.project.join(rel).exists());
    }
    assert_backup(&f.project, "mise.toml", &originals[Path::new("mise.toml")]);
    assert!(backups(&f.project, "engine/rust-toolchain.toml").is_empty());
    let before = snapshot(&f.root);
    f.succeed(&args);
    assert_eq!(snapshot(&f.root), before);
}
#[test]
fn test_javascript_missing_package_scripts_and_root_tasks_omits_tasks_with_warning() {
    let f = Fixture::new();
    f.write("web/package.json", "{}\n");
    let output = f.succeed(&[
        "--tools",
        "javascript",
        "--javascript-dir",
        "web",
        "--apply",
    ]);
    let data = generated(&f.project);
    for name in ["typecheck", "test"] {
        assert!(data["tasks"].get(format!("{name}:javascript")).is_none());
        assert!(output.contains(name));
    }
    assert!(output.to_lowercase().contains("warn"));
    assert_eq!(read(&f.project, "web/package.json"), "{}\n");
}
#[test]
fn test_javascript_existing_package_scripts_are_used() {
    let f = Fixture::new();
    let manifest = "{\"scripts\":{\"typecheck\":\"tsc --noEmit\",\"test\":\"node --test\"}}\n";
    f.write("package.json", manifest);
    f.succeed(&["--tools", "javascript", "--apply"]);
    let data = generated(&f.project);
    for name in ["typecheck", "test"] {
        assert!(
            data["tasks"][format!("{name}:javascript")]["run"]
                .as_str()
                .unwrap()
                .contains(name)
        );
    }
    assert_eq!(read(&f.project, "package.json"), manifest);
}
#[test]
fn test_typescript_is_a_javascript_selection_alias() {
    let mut profiles = vec![];
    for tool in ["javascript", "typescript"] {
        let f = Fixture::new();
        f.write("package.json", "{}\n");
        f.succeed(&["--tools", tool, "--apply"]);
        let data = generated(&f.project);
        assert!(f.project.join("biome.json").is_file());
        assert!(data["tasks"].get("check:javascript").is_some());
        assert!(data["tasks"].get("check:typescript").is_none());
        assert!(data["tasks"].get("check:python").is_none());
        profiles.push(data);
    }
    assert_eq!(profiles[0], profiles[1]);
}
#[test]
fn test_only_selected_tool_configs_and_tasks_are_installed() {
    let f = Fixture::new();
    f.succeed(&["--tools", "lua", "--apply"]);
    let data = generated(&f.project);
    for name in ["stylua.toml", ".styluaignore"] {
        assert!(f.project.join(name).is_file());
    }
    for language in ["python", "javascript", "rust", "markdown"] {
        assert!(data["tasks"].get(format!("check:{language}")).is_none());
    }
    for name in [
        "ruff.toml",
        "ty.toml",
        "biome.json",
        "rust-toolchain.toml",
        ".markdownlint-cli2.jsonc",
    ] {
        assert!(!f.project.join(name).exists());
    }
}
#[test]
fn test_python_auto_mode_uses_scripts_without_pyproject() {
    let f = Fixture::new();
    f.write("scripts/report.py", "print('report')\n");
    f.succeed(&["--tools", "python", "--python-dir", "scripts", "--apply"]);
    let data = generated(&f.project);
    assert!(data["tasks"].get("deps:python").is_none());
    assert!(data["tasks"].get("test:python").is_none());
    assert_eq!(python_targets(&data), ["."]);
    assert!(f.project.join("scripts/ruff.toml").is_file());
    assert!(f.project.join("scripts/ty.toml").is_file());
    assert!(
        !snapshot(&f.project)
            .keys()
            .any(|path| path.ends_with("pyproject.toml"))
    );
}
#[test]
fn test_explicit_python_scripts_mode_omits_project_tasks() {
    for has_manifest in [false, true] {
        let f = Fixture::new();
        let originals = if has_manifest {
            python_project(&f.project, true)
        } else {
            Files::new()
        };
        f.write("report.py", "print('report')\n");
        f.succeed(&["--tools", "python", "--python-mode", "scripts", "--apply"]);
        let data = generated(&f.project);
        assert!(data["tasks"].get("deps:python").is_none());
        assert!(data["tasks"].get("test:python").is_none());
        assert!(data["tasks"].get("typecheck:python").is_some());
        assert_preserved(&f.project, &originals);
        assert_eq!(f.project.join("pyproject.toml").exists(), has_manifest);
    }
}
#[test]
fn test_python_file_autodetection() {
    for dirs in [vec!["src", "tests"], vec!["src"], vec!["tests"], vec![]] {
        let f = Fixture::new();
        for dir in &dirs {
            f.write(format!("{dir}/sample.py"), "pass\n");
        }
        f.succeed(&["--tools", "python", "--python-mode", "scripts", "--apply"]);
        assert_eq!(
            python_targets(&generated(&f.project)),
            if dirs.is_empty() { vec!["."] } else { dirs }
        );
    }
}
#[test]
fn test_explicit_python_files_version_and_project_mode() {
    let f = Fixture::new();
    let backend = f.project.join("backend");
    let originals = python_project(&backend, false);
    f.write("backend/cli.py", "pass\n");
    f.write("backend/jobs.py", "pass\n");
    f.succeed(&[
        "--tools",
        "python",
        "--python-dir",
        "backend",
        "--python-mode",
        "project",
        "--python-files",
        "cli.py",
        "jobs.py",
        "--python-version",
        "3.13",
        "--apply",
    ]);
    let data = generated(&f.project);
    assert_eq!(python_targets(&data), ["cli.py", "jobs.py"]);
    assert_eq!(data["vars"]["python_version"], "3.13");
    assert_eq!(data["tools"]["python"], "3.15");
    for name in ["deps:python", "test:python"] {
        assert!(data["tasks"].get(name).is_some());
    }
    assert_eq!(
        toml_file(backend.join("ruff.toml"))["target-version"],
        "py313"
    );
    assert_eq!(
        toml_file(backend.join("ty.toml"))["environment"]["python-version"],
        "3.13"
    );
    assert_preserved(&backend, &originals);
}
#[test]
fn test_shared_node_conflict_requires_resolution_before_writes() {
    let f = Fixture::new();
    f.write("package.json", "{}\n");
    let catalog = f.root.join("templates");
    copy_tree(&templates(), &catalog);
    let path = catalog.join("mise/profiles/markdown.toml");
    let original = fs::read_to_string(&path).unwrap();
    fs::write(path, original.replace("node = \"26\"", "node = \"24\"")).unwrap();
    f.refuse(&[
        "--templates",
        catalog.to_str().unwrap(),
        "--tools",
        "javascript",
        "markdown",
        "--apply",
    ]);
}
#[test]
fn test_explicit_node_version_resolves_shared_conflict() {
    let f = Fixture::new();
    f.write("package.json", "{}\n");
    f.succeed(&[
        "--tools",
        "javascript",
        "markdown",
        "--node-version",
        "24.15.0",
        "--apply",
    ]);
    let data = generated(&f.project);
    assert_eq!(data["tools"]["node"], "24.15.0");
    assert!(data["tasks"].get("check:javascript").is_some());
    assert!(data["tasks"].get("check:markdown").is_some());
}
#[test]
fn test_existing_pinned_rust_toolchain_is_not_replaced_by_config_overwrite() {
    let f = Fixture::new();
    let originals = monorepo(&f.project);
    for flag in ["--replace-configs", "--overwrite"] {
        f.succeed(&["--tools", "rust", "--rust-dir", "engine", flag, "--apply"]);
        assert_preserved(&f.project, &originals);
        assert!(backups(&f.project, "engine/rust-toolchain.toml").is_empty());
    }
}
#[test]
fn test_relative_language_directory_traversal_is_rejected() {
    let f = Fixture::new();
    for tool in ["python", "javascript", "rust", "lua", "markdown"] {
        f.refuse(&[
            "--tools",
            tool,
            &format!("--{tool}-dir"),
            "../outside",
            "--apply",
        ]);
    }
}
#[test]
fn test_absolute_outside_directory_is_rejected() {
    let f = Fixture::new();
    f.refuse(&[
        "--tools",
        "lua",
        "--lua-dir",
        f.outside.to_str().unwrap(),
        "--apply",
    ]);
}
#[test]
fn test_agent_file_traversal_is_rejected() {
    let f = Fixture::new();
    write(&f.outside, "AGENS.md", "# Outside instructions\n");
    f.refuse(&[
        "--tools",
        "lua",
        "--agents-file",
        "../outside/AGENS.md",
        "--apply",
    ]);
}
#[test]
fn test_python_file_traversal_is_rejected() {
    let f = Fixture::new();
    write(&f.outside, "report.py", "pass\n");
    f.refuse(&[
        "--tools",
        "python",
        "--python-files",
        "../outside/report.py",
        "--apply",
    ]);
}
#[cfg(unix)]
#[test]
fn test_symlinked_language_directories_outside_project_are_rejected() {
    let f = Fixture::new();
    std::os::unix::fs::symlink(&f.outside, f.project.join("linked")).unwrap();
    for tool in ["python", "javascript", "rust", "lua", "markdown"] {
        f.refuse(&[
            "--tools",
            tool,
            &format!("--{tool}-dir"),
            "linked",
            "--apply",
        ]);
    }
}
#[cfg(unix)]
#[test]
fn test_symlinked_python_file_outside_project_is_rejected() {
    let f = Fixture::new();
    let target = write(&f.outside, "report.py", "pass\n");
    std::os::unix::fs::symlink(target, f.project.join("report.py")).unwrap();
    f.refuse(&[
        "--tools",
        "python",
        "--python-files",
        "report.py",
        "--apply",
    ]);
}
#[cfg(unix)]
#[test]
fn test_symlinked_config_agent_fragment_and_backup_paths_are_rejected() {
    for (tool, relative, directory) in [
        ("lua", ".mise", true),
        ("lua", ".mise/conf.d", true),
        ("lua", FRAGMENT, false),
        ("lua", "mise.toml", false),
        ("lua", ".mise.toml", false),
        ("lua", "stylua.toml", false),
        ("lua", ".stylua.toml", false),
        ("lua", ".styluaignore", false),
        ("lua", "AGENS.md", false),
        ("lua", "AGENTS.md", false),
        ("lua", ".tooling-backups", true),
        ("python", ".ruff.toml", false),
    ] {
        let f = Fixture::new();
        let target = f.outside.join("target");
        if directory {
            fs::create_dir(&target).unwrap();
        } else {
            fs::write(&target, "# Outside file must remain untouched\n").unwrap();
        }
        let link = f.project.join(relative);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(target, link).unwrap();
        if !["AGENS.md", "AGENTS.md"].contains(&relative) {
            f.write("AGENTS.md", "# Existing instructions requiring a backup\n");
        }
        let mut args = vec!["--tools", tool, "--apply"];
        if relative == "AGENS.md" {
            args.extend(["--agents-file", "AGENS.md"]);
        }
        f.refuse(&args);
    }
}
#[cfg(unix)]
#[test]
fn test_explicit_symlinked_agent_file_is_rejected() {
    let f = Fixture::new();
    let target = write(&f.outside, "TEAM.md", "# Outside team guidance\n");
    std::os::unix::fs::symlink(target, f.project.join("TEAM.md")).unwrap();
    f.refuse(&["--tools", "lua", "--agents-file", "TEAM.md", "--apply"]);
}
#[test]
fn test_optional_install_commands_are_planned_but_never_run_in_preview_or_dry_run() {
    let f = Fixture::new();
    python_project(&f.project, true);
    for mode in [vec![], vec!["--dry-run"]] {
        for flags in [
            vec!["--install-tools"],
            vec!["--install-deps"],
            vec!["--install-tools", "--install-deps"],
        ] {
            let args = [vec!["--tools", "python"], mode.clone(), flags].concat();
            let before = snapshot(&f.root);
            let output = f.succeed(&args);
            assert!(output.contains("PLAN RUN"));
            assert!(output.contains("mise"));
            assert_eq!(snapshot(&f.root), before);
        }
    }
}
#[test]
fn test_optional_install_commands_execute_only_on_apply() {
    for flags in [
        vec!["--install-tools"],
        vec!["--install-deps"],
        vec!["--install-tools", "--install-deps"],
    ] {
        let f = Fixture::new();
        let originals = python_project(&f.project, true);
        let mut observed = vec![];
        let args = [vec!["--tools", "python", "--apply"], flags.clone()].concat();
        let plan = f.apply_recorded(&args, |command| {
            assert!(f.project.join(FRAGMENT).is_file());
            observed.push(command.argv.clone());
            Ok(())
        });
        assert!(!observed.is_empty());
        assert_eq!(
            observed,
            plan.commands
                .iter()
                .map(|c| c.argv.clone())
                .collect::<Vec<_>>()
        );
        if flags.contains(&"--install-tools") {
            assert!(observed.iter().any(|c| c == &["mise", "install"]));
        }
        if flags.contains(&"--install-deps") {
            assert!(
                observed
                    .iter()
                    .any(|c| resolved(c).starts_with(&["uv".into(), "sync".into()]))
            );
        }
        assert_preserved(&f.project, &originals);
    }
}
#[test]
fn test_external_command_oserror_is_reported_as_failure() {
    let f = Fixture::new();
    let (status, output) = f.invoke(&["--tools", "lua", "--install-tools", "--apply"]);
    assert_eq!(status, 1, "{output}");
    assert!(output.contains("Cannot run mise"), "{output}");
    assert!(f.project.join(FRAGMENT).is_file());
}
#[test]
fn test_missing_project_is_reported_without_writes() {
    let f = Fixture::new();
    let before = snapshot(&f.root);
    let (status, output) = f.invoke_at(&f.root.join("missing"), &["--tools", "lua", "--apply"]);
    assert_eq!(status, 1, "{output}");
    assert!(output.contains("Project directory does not exist"));
    assert_eq!(snapshot(&f.root), before);
}
#[test]
fn test_config_overwrite_converts_prettier_alternatives_with_deletion_backups() {
    for flag in ["--replace-configs", "--overwrite"] {
        for (relative, text) in [
            ("prettier.config.js", "export default { printWidth: 79 };\n"),
            (
                "prettier.config.cjs",
                "module.exports = { printWidth: 79 };\n",
            ),
            (".prettierrc.js", "export default { printWidth: 79 };\n"),
            (".prettierrc.cjs", "module.exports = { printWidth: 79 };\n"),
            (".prettierrc.toml", "printWidth = 79\n"),
        ] {
            let f = Fixture::new();
            let manifest = "{\"private\":true}\n";
            f.write("package.json", manifest);
            f.write(relative, text);
            let args = ["--tools", "markdown", flag, "--apply"];
            f.succeed(&args);
            assert!(!f.project.join(relative).exists());
            let data: Value = serde_json::from_str(&read(&f.project, ".prettierrc.json")).unwrap();
            assert!(data.is_object());
            assert_backup(&f.project, relative, text);
            assert_eq!(read(&f.project, "package.json"), manifest);
            let before = snapshot(&f.root);
            f.succeed(&args);
            assert_eq!(snapshot(&f.root), before);
        }
    }
}
#[test]
fn test_replace_configs_accepts_compatible_prettier_formats() {
    for (relative, text) in [
        (".prettierrc", "{\"printWidth\":79}\n"),
        (".prettierrc.json", "{\"printWidth\":79}\n"),
        (
            ".prettierrc.jsonc",
            "{\n// Existing formatting preferences\n\"printWidth\":79,\n}\n",
        ),
        (".prettierrc.yaml", "printWidth: 79\n"),
        (".prettierrc.yml", "printWidth: 79\n"),
    ] {
        let f = Fixture::new();
        f.write("package.json", "{\"private\":true}\n");
        f.write(relative, text);
        f.succeed(&["--tools", "markdown", "--replace-configs", "--apply"]);
        let existing = f.project.join(relative);
        let canonical = f.project.join(".prettierrc.json");
        assert!(existing.is_file() || canonical.is_file());
        if !existing.exists() || fs::read(&existing).unwrap() != text.as_bytes() {
            assert_backup(&f.project, relative, text);
        } else {
            assert_ne!(existing, canonical);
            assert!(canonical.is_file());
            assert_ne!(fs::read(canonical).unwrap(), text.as_bytes());
        }
        assert_eq!(read(&f.project, "package.json"), "{\"private\":true}\n");
    }
}
#[test]
fn test_root_aggregate_cycles_omit_unsafe_javascript_aliases_with_warning() {
    for dependency in ["*", "javascript"] {
        let f = Fixture::new();
        let original = format!(
            "[tasks.typecheck]\ndepends = ['typecheck:{dependency}']\n[tasks.test]\ndepends = ['test:{dependency}']\n"
        );
        let originals = files(
            &f.project,
            &[("web/package.json", "{}\n"), ("mise.toml", &original)],
        );
        let output = f.succeed(&[
            "--tools",
            "javascript",
            "--javascript-dir",
            "web",
            "--apply",
        ]);
        let data = generated(&f.project);
        assert!(data["tasks"].get("typecheck:javascript").is_none());
        assert!(data["tasks"].get("test:javascript").is_none());
        assert!(data["tasks"].get("check:javascript").is_some());
        assert!(output.to_lowercase().contains("warn"));
        assert!(output.contains("cycle"));
        assert_preserved(&f.project, &originals);
    }
}

#[test]
fn test_other_mise_fragment_supplies_directories_node_pin_and_existing_tasks() {
    let f = Fixture::new();
    let fragment = "[tools]\nnode = '22.14.0'\n[vars]\npython_dir = 'scripts'\njavascript_dir = 'web'\nmarkdown_dir = 'docs'\n[tasks.typecheck]\nrun = 'echo project typecheck'\n[tasks.test]\nrun = 'echo project test'\n[tasks.'check:javascript']\nrun = 'echo custom javascript check'\n[tasks.'typecheck:javascript']\nrun = 'echo custom javascript types'\n[tasks.'format:markdown']\nrun = 'echo custom markdown format'\n";
    let originals = files(
        &f.project,
        &[
            (
                "mise.toml",
                "# Preserve root configuration.\n[tools]\nruff = '0.11.13'\n",
            ),
            (".mise/conf.d/10-project.toml", fragment),
            ("scripts/report.py", "print('report')\n"),
            ("web/package.json", "{}\n"),
            ("docs/package.json", "{\"private\":true}\n"),
            ("docs/README.md", "# Project docs\n"),
        ],
    );
    f.succeed(&["--tools", "python", "javascript", "markdown", "--apply"]);
    let data = generated(&f.project);
    let inherited: Value = toml::from_str(fragment).unwrap();
    assert_eq!(
        data["tools"]
            .get("node")
            .unwrap_or(&inherited["tools"]["node"]),
        "22.14.0"
    );
    for section in ["vars", "tasks"] {
        for (name, value) in inherited[section].as_object().unwrap() {
            assert_eq!(data[section].get(name).unwrap_or(value), value, "{name}");
        }
    }
    assert_eq!(data["tasks"]["test:javascript"]["depends"], json!(["test"]));
    assert!(data["tasks"]["test:javascript"].get("run").is_none());
    for rel in [
        "scripts/ruff.toml",
        "scripts/ty.toml",
        "web/biome.json",
        "docs/.markdownlint-cli2.jsonc",
    ] {
        assert!(f.project.join(rel).is_file());
    }
    for rel in [
        "ruff.toml",
        "ty.toml",
        "biome.json",
        ".markdownlint-cli2.jsonc",
    ] {
        assert!(!f.project.join(rel).exists());
    }
    assert_preserved(&f.project, &originals);
    assert_backup(&f.project, "mise.toml", &originals[Path::new("mise.toml")]);
    assert!(backups(&f.project, ".mise/conf.d/10-project.toml").is_empty());
}
#[test]
fn test_markdown_uses_mise_tools_without_package_dependency_installation() {
    for (manager, version, lockfile, lock) in [
        (
            "npm",
            "10.9.2",
            "package-lock.json",
            "{\"lockfileVersion\":3,\"packages\":{}}\n",
        ),
        (
            "pnpm",
            "10.5.2",
            "pnpm-lock.yaml",
            "lockfileVersion: '9.0'\n",
        ),
        ("yarn", "4.9.1", "yarn.lock", "__metadata:\n  version: 8\n"),
    ] {
        let f = Fixture::new();
        let manifest =
            json!({"private":true,"packageManager":format!("{manager}@{version}")}).to_string();
        let originals = files(
            &f.project,
            &[
                ("docs/package.json", &manifest),
                (&format!("docs/{lockfile}"), lock),
                ("docs/README.md", "# Documentation\n"),
                ("docs/.gitignore", "node_modules/\n"),
            ],
        );
        f.apply_recorded(
            &[
                "--tools",
                "markdown",
                "--markdown-dir",
                "docs",
                "--install-deps",
                "--apply",
            ],
            |command| {
                assert_eq!(command.argv[..2], ["mise", "trust"]);
                Ok(())
            },
        );
        let data = generated(&f.project);
        for name in ["check:markdown", "format:markdown"] {
            let commands = task_commands(&data["tasks"][name]);
            assert!(!commands.is_empty());
            for command in commands {
                assert!(
                    ["markdownlint-cli2", "prettier"].contains(&resolved(&command)[0].as_str())
                );
            }
        }
        for name in ["markdownlint-cli2", "prettier"] {
            assert!(data["tools"].get(name).is_some());
        }
        assert!(data["tools"].get("bun").is_none());
        for task in data["tasks"].as_object().unwrap().values() {
            for command in task_commands(task) {
                assert!(
                    !command
                        .iter()
                        .any(|s| ["bun", "bunx"].contains(&s.as_str()))
                );
            }
        }
        assert!(
            !snapshot(&f.project)
                .keys()
                .any(|p| p.ends_with("bun.lock") || p.ends_with("bun.lockb"))
        );
        assert_preserved(&f.project, &originals);
    }
}
#[test]
fn test_workspace_root_package_manager_is_inherited_by_nested_web_and_docs() {
    for manager in ["npm", "pnpm", "yarn"] {
        let f = Fixture::new();
        let (originals, _) = workspace_project(&f.project, manager);
        let args = [
            "--tools",
            "javascript",
            "markdown",
            "--javascript-dir",
            "web",
            "--markdown-dir",
            "docs",
            "--apply",
        ];
        f.succeed(&args);
        let data = generated(&f.project);
        assert_eq!(data["vars"]["javascript_dir"], "web");
        assert_eq!(data["vars"]["markdown_dir"], "docs");
        for name in ["deps:javascript", "typecheck:javascript", "test:javascript"] {
            let commands = task_commands(&data["tasks"][name]);
            assert!(!commands.is_empty());
            for command in commands {
                assert_eq!(resolved(&command)[0], manager);
            }
        }
        for name in ["check:markdown", "format:markdown"] {
            let commands = task_commands(&data["tasks"][name]);
            assert!(!commands.is_empty());
            for command in commands {
                assert!(
                    ["markdownlint-cli2", "prettier"].contains(&resolved(&command)[0].as_str())
                );
            }
        }
        for task in data["tasks"].as_object().unwrap().values() {
            for command in task_commands(task) {
                assert!(
                    !command
                        .iter()
                        .any(|s| ["bun", "bunx"].contains(&s.as_str()))
                );
            }
        }
        assert!(
            !snapshot(&f.project)
                .keys()
                .any(|p| p.ends_with("bun.lock") || p.ends_with("bun.lockb"))
        );
        assert_central_dependencies(&f.project, &["javascript", "markdown"], &[]);
        assert_preserved(&f.project, &originals);
        let before = snapshot(&f.root);
        f.succeed(&args);
        assert_eq!(snapshot(&f.root), before);
    }
}
#[test]
fn test_workspace_shared_dependency_backups_precede_commands_and_keep_original_lock() {
    for manager in ["npm", "pnpm", "yarn"] {
        let f = Fixture::new();
        let (originals, lockfile) = workspace_project(&f.project, manager);
        let protected = [
            "package.json",
            lockfile,
            "web/package.json",
            "docs/package.json",
        ];
        let mut observed = vec![];
        f.apply_recorded(
            &[
                "--tools",
                "javascript",
                "markdown",
                "--javascript-dir",
                "web",
                "--markdown-dir",
                "docs",
                "--install-deps",
                "--apply",
            ],
            |command| {
                let parsed = resolved(&command.argv);
                if parsed.starts_with(&["mise".into(), "trust".into()]) {
                    return Ok(());
                }
                let commands = if parsed.starts_with(&["mise".into(), "run".into()]) {
                    let data = generated(&f.project);
                    parsed[2..]
                        .iter()
                        .flat_map(|name| task_commands(&data["tasks"][name]))
                        .map(|c| resolved(&c).to_vec())
                        .collect::<Vec<_>>()
                } else {
                    vec![parsed.to_vec()]
                };
                assert!(!commands.is_empty());
                for argv in &commands {
                    assert_eq!(argv[0], manager);
                    assert!(
                        argv[1..]
                            .iter()
                            .any(|s| ["ci", "install", "add"].contains(&s.as_str()))
                    );
                }
                for rel in protected {
                    assert_backup(&f.project, rel, &originals[Path::new(rel)]);
                }
                assert!(command.directory.starts_with(&f.project));
                observed.extend(commands);
                if manager == "npm" {
                    let mut contents: Value =
                        serde_json::from_str(&read(&f.project, lockfile)).unwrap();
                    contents["fixtureRevision"] = json!(observed.len());
                    f.write(lockfile, contents.to_string());
                } else {
                    f.write(
                        lockfile,
                        read(&f.project, lockfile) + "\n# Mock dependency operation\n",
                    );
                }
                Ok(())
            },
        );
        assert_eq!(observed.len(), 1);
        assert_ne!(
            fs::read(f.project.join(lockfile)).unwrap(),
            originals[Path::new(lockfile)]
        );
        for rel in protected {
            assert_backup(&f.project, rel, &originals[Path::new(rel)]);
        }
        for dir in ["web", "docs"] {
            for name in [
                "package-lock.json",
                "pnpm-lock.yaml",
                "yarn.lock",
                "bun.lock",
                "bun.lockb",
            ] {
                assert!(!f.project.join(dir).join(name).exists());
            }
        }
        assert_preserved(
            &f.project,
            &originals
                .into_iter()
                .filter(|(rel, _)| rel != Path::new(lockfile))
                .collect(),
        );
    }
}
#[test]
fn test_tool_only_pyproject_uses_scripts_auto_mode_without_dependency_commands() {
    let f = Fixture::new();
    let manifest = "[tool.ruff]\nline-length = 88\ntarget-version = 'py312'\n";
    f.write("pyproject.toml", manifest);
    f.write("report.py", "print('report')\n");
    let plan = f.apply_recorded(&["--tools", "python", "--install-deps", "--apply"], |_| {
        Ok(())
    });
    let data = generated(&f.project);
    assert!(data["tasks"].get("deps:python").is_none());
    assert!(data["tasks"].get("test:python").is_none());
    assert!(data["tasks"].get("typecheck:python").is_some());
    assert_eq!(python_targets(&data), ["."]);
    for task in data["tasks"].as_object().unwrap().values() {
        for command in task_commands(task) {
            assert!(!command.contains(&"pytest".into()));
            assert!(!command.starts_with(&["uv".into(), "add".into()]));
            assert!(!command.starts_with(&["uv".into(), "sync".into()]));
        }
    }
    assert_eq!(read(&f.project, "pyproject.toml"), manifest);
    assert!(!f.project.join("ruff.toml").exists());
    assert!(!f.project.join("uv.lock").exists());
    for command in plan.commands {
        let argv = resolved(&command.argv);
        let program = argv[0].as_str();
        assert!(
            !(program == "uv"
                && argv
                    .get(1)
                    .is_some_and(|s| ["add", "sync"].contains(&s.as_str())))
        );
        assert!(
            ![
                "npm", "npx", "pnpm", "yarn", "bun", "bunx", "pip", "pip3", "poetry", "pdm"
            ]
            .contains(&program)
        );
        assert!(
            !(program.starts_with("python")
                && argv.get(1..3) == Some(&["-m".into(), "pip".into()]))
        );
        assert!(!argv.contains(&"deps:python".into()));
        assert!(!argv.contains(&"pytest".into()));
    }
}
#[test]
fn test_rust_install_tools_uses_inherited_root_toolchain_pin() {
    let pin = "1.85.1";
    for (name, text) in [
        (
            "rust-toolchain.toml",
            format!("[toolchain]\nchannel = '{pin}'\ncomponents = ['rustfmt', 'clippy']\n"),
        ),
        ("rust-toolchain", format!("{pin}\n")),
    ] {
        let f = Fixture::new();
        let originals = files(
            &f.project,
            &[
                (name, &text),
                (
                    "engine/Cargo.toml",
                    "[package]\nname = 'engine'\nversion = '0.1.0'\nedition = '2021'\n",
                ),
                ("engine/Cargo.lock", "version = 4\n"),
                ("engine/src/lib.rs", "pub fn answer() -> u32 { 42 }\n"),
            ],
        );
        let plan = f.apply_recorded(
            &[
                "--tools",
                "rust",
                "--rust-dir",
                "engine",
                "--install-tools",
                "--apply",
            ],
            |_| Ok(()),
        );
        let commands: Vec<_> = plan
            .commands
            .iter()
            .filter(|c| c.argv[0] == "rustup")
            .collect();
        assert!(!commands.is_empty());
        assert!(commands.iter().any(|c| {
            c.argv
                .iter()
                .any(|s| s == pin || s == &format!("--toolchain={pin}"))
        }));
        assert!(!commands.iter().any(|c| {
            c.argv
                .iter()
                .any(|s| s == "stable" || s == "--toolchain=stable")
        }));
        if f.project.join("engine/rust-toolchain.toml").exists() {
            assert_eq!(
                toml_file(f.project.join("engine/rust-toolchain.toml"))["toolchain"]["channel"],
                pin
            );
        }
        assert_preserved(&f.project, &originals);
        assert!(backups(&f.project, name).is_empty());
    }
}
#[test]
fn test_markdown_preserves_existing_cli2_configuration() {
    for (relative, text) in [
        (
            ".markdownlint-cli2.jsonc",
            "{\n// CLI2-specific configuration\n\"config\":{\"MD013\":false}\n}\n",
        ),
        (".markdownlint-cli2.yaml", "config:\n  MD013: false\n"),
        (
            ".markdownlint-cli2.cjs",
            "module.exports = { config: { MD013: false } };\n",
        ),
    ] {
        let f = Fixture::new();
        f.write("package.json", "{\"private\":true}\n");
        f.write(relative, text);
        let output = f.succeed(&["--tools", "markdown", "--apply"]);
        assert_eq!(
            f.project.join(".markdownlint-cli2.jsonc").exists(),
            relative == ".markdownlint-cli2.jsonc"
        );
        assert_eq!(read(&f.project, relative), text);
        assert!(backups(&f.project, relative).is_empty());
        assert!(output.contains("KEEP"));
    }
}
#[test]
fn test_existing_python_files_variable_is_respected_by_default() {
    for source in ["mise.toml", ".mise/conf.d/10-project.toml"] {
        let f = Fixture::new();
        let mut originals = python_project(&f.project, true);
        f.write("custom/report.py", "pass\n");
        f.write("checks/test_report.py", "pass\n");
        originals.extend(files(
            &f.project,
            &[(source, "[vars]\npython_files = 'custom checks'\n")],
        ));
        f.succeed(&["--tools", "python", "--apply"]);
        let mut data = generated(&f.project);
        if data["vars"].get("python_files").is_none() {
            data["vars"]["python_files"] = json!("custom checks");
        }
        assert_eq!(python_targets(&data), ["custom", "checks"]);
        assert_preserved(&f.project, &originals);
    }
}
#[test]
fn test_conflicting_explicit_python_file_or_version_vars_fail_before_writes() {
    for source in ["mise.toml", ".mise/conf.d/10-project.toml"] {
        for (flag, value) in [("--python-files", "src"), ("--python-version", "3.13")] {
            let f = Fixture::new();
            python_project(&f.project, false);
            f.write("custom/report.py", "pass\n");
            f.write("checks/test_report.py", "pass\n");
            f.write(
                source,
                "[vars]\npython_files = 'custom checks'\npython_version = '3.12'\n",
            );
            f.write("AGENTS.md", "# Existing instructions must not change\n");
            f.refuse(&["--tools", "python", flag, value, "--apply"]);
        }
    }
}
#[test]
fn test_matching_explicit_python_file_and_version_vars_are_allowed() {
    for source in ["mise.toml", ".mise/conf.d/10-project.toml"] {
        let f = Fixture::new();
        let mut originals = python_project(&f.project, false);
        f.write("custom/report.py", "pass\n");
        f.write("checks/test_report.py", "pass\n");
        originals.extend(files(
            &f.project,
            &[(
                source,
                "[vars]\npython_files = 'custom checks'\npython_version = '3.12'\n",
            )],
        ));
        f.succeed(&[
            "--tools",
            "python",
            "--python-files",
            "custom",
            "checks",
            "--python-version",
            "3.12",
            "--apply",
        ]);
        let mut data = generated(&f.project);
        for (key, value) in [
            ("python_files", "custom checks"),
            ("python_version", "3.12"),
        ] {
            if data["vars"].get(key).is_none() {
                data["vars"][key] = json!(value);
            }
        }
        assert_eq!(python_targets(&data), ["custom", "checks"]);
        assert_eq!(data["vars"]["python_version"], "3.12");
        assert_preserved(&f.project, &originals);
    }
}

#[test]
fn test_central_aggregates_include_only_selected_available_language_tasks() {
    let cases: &[(&str, &[&str], &[&str])] = &[
        ("python-project", &["python"], &[]),
        ("python-scripts", &["python"], &["--python-mode", "scripts"]),
        ("javascript", &["javascript"], &[]),
        ("rust", &["rust"], &[]),
        ("lua", &["lua"], &[]),
        ("markdown", &["markdown"], &[]),
        ("lua-rust", &["lua", "rust"], &[]),
    ];
    for &(label, languages, extra) in cases {
        let f = Fixture::new();
        let mut originals = Files::new();
        if label == "python-project" {
            originals.extend(python_project(&f.project, true));
        } else if languages.contains(&"python") {
            f.write("report.py", "pass\n");
        }
        if languages.contains(&"javascript") || languages.contains(&"markdown") {
            originals.extend(files(&f.project, &[("package.json", "{}\n")]));
        }
        if languages.contains(&"rust") {
            originals.extend(files(
                &f.project,
                &[
                    (
                        "Cargo.toml",
                        "[package]\nname = 'sample'\nversion = '0.1.0'\nedition = '2021'\n",
                    ),
                    ("src/lib.rs", "pub fn answer() -> u32 { 42 }\n"),
                ],
            ));
        }
        let unselected = if languages.contains(&"rust") {
            "python"
        } else {
            "rust"
        };
        let original = format!(
            "# Preserve unrelated project tasks.\n[tasks.build]\nrun = 'echo custom build'\n[tasks.'check:{unselected}']\nrun = 'echo unselected check'\n[tasks.'test:{unselected}']\nrun = 'echo unselected tests'\n"
        );
        originals.extend(files(&f.project, &[("mise.toml", &original)]));
        let args = [
            vec!["--tools"],
            languages.to_vec(),
            extra.to_vec(),
            vec!["--apply"],
        ]
        .concat();
        f.succeed(&args);
        assert_central_dependencies(&f.project, languages, &[]);
        let central = central_tasks(&f.project);
        for language in languages {
            for task in ["check", "format"] {
                assert!(strings(&central[task]["depends"]).contains(&format!("{task}:{language}")));
            }
        }
        if ["python-scripts", "javascript"].contains(&label) {
            assert!(central.get("test").is_none());
        }
        if label == "python-scripts" {
            assert!(central.get("deps").is_none());
        }
        assert_preserved(&f.project, &originals);
        assert_backup(&f.project, "mise.toml", &original);
        let agent = read(&f.project, "AGENTS.md");
        let block = agent
            .split_once(START)
            .unwrap()
            .1
            .split_once(END)
            .unwrap()
            .0;
        for name in central.as_object().unwrap().keys() {
            assert!(
                block.contains(&format!("`mise run {name}`")),
                "Missing guidance for {name}"
            );
        }
        let before = snapshot(&f.root);
        f.succeed(&args);
        assert_eq!(snapshot(&f.root), before);
    }
}
#[test]
fn test_user_aggregate_collisions_and_existing_bare_tasks_are_preserved() {
    for root_name in ["mise.toml", ".mise.toml"] {
        for name in ["check", "format", "test", "deps"] {
            let f = Fixture::new();
            let mut original = "# Keep project-owned entry points.\n[tasks.build]\nrun = 'echo custom build'\n[tasks.test]\nrun = 'echo custom test'\ndepends = ['build']\n".to_owned();
            if name == "test" {
                original.push_str("description = 'User owned'\n");
            }
            original.push_str("[tasks.typecheck]\nrun = 'echo custom typecheck'\n");
            if name != "test" {
                original.push_str(&format!("[tasks.{name}]\nrun = 'echo user aggregate'\ndescription = 'User owned'\ndepends = ['build']\n"));
            }
            f.write(root_name, &original);
            let before = toml_file(f.project.join(root_name));
            f.succeed(&["--tools", "lua", "--apply"]);
            let root = root_mise(&f.project);
            assert_eq!(root.file_name().unwrap(), root_name);
            let current = toml_file(&root);
            let central = central_tasks(&f.project);
            for preserved in ["build", "test", "typecheck", name] {
                assert_eq!(current["tasks"][preserved], before["tasks"][preserved]);
                assert!(central.get(preserved).is_none());
            }
            assert_central_dependencies(&f.project, &["lua"], &[name, "test"]);
            assert_root_preserved(&root, original.as_bytes());
            let before = snapshot(&f.root);
            f.succeed(&["--tools", "lua", "--apply"]);
            assert_eq!(snapshot(&f.root), before);
        }
    }
}
#[test]
fn test_managed_aggregates_compose_with_explicit_tasks_parent_and_inline_children() {
    for source in [
        "[tasks]\nbuild = 'echo existing build'\n",
        "[tasks]\nbuild = { run = 'echo existing build', description = 'User owned' }\n",
        "[tasks]\n[tasks.build]\nrun = 'echo existing build'\n",
        "[tasks]\nbuild = 'echo existing build'\ncheck = { run = 'echo user check', depends = ['build'] }\n",
        "[tasks]\nbuild = { run = 'echo existing build' }\ncheck = 'echo user check'\n",
    ] {
        let f = Fixture::new();
        let original = format!("# Preserve the explicit tasks parent and user syntax.\n{source}");
        let root = f.write("mise.toml", &original);
        let old = toml_file(&root);
        f.succeed(&["--tools", "lua", "--apply"]);
        let current = toml_file(&root);
        for (name, task) in old["tasks"].as_object().unwrap() {
            assert_eq!(&current["tasks"][name], task, "{name}");
        }
        let text = fs::read_to_string(&root).unwrap();
        let block = text
            .split_once(AGGREGATE_START)
            .unwrap()
            .1
            .split_once(AGGREGATE_END)
            .unwrap()
            .0;
        assert!(
            !block
                .lines()
                .any(|line| line.split('#').next().unwrap().trim() == "[tasks]")
        );
        let preserved = if old["tasks"].get("check").is_some() {
            vec!["check"]
        } else {
            vec![]
        };
        assert_central_dependencies(&f.project, &["lua"], &preserved);
        assert_root_preserved(&root, original.as_bytes());
        assert_backup(&f.project, "mise.toml", &original);
        let before = snapshot(&f.root);
        f.succeed(&["--tools", "lua", "--apply"]);
        assert_eq!(snapshot(&f.root), before);
    }
}
#[test]
fn test_central_block_replacement_preserves_surroundings_and_updates_selection() {
    let f = Fixture::new();
    let prefix = "# Exact prefix stays.\n\n[tasks.before]\nrun = 'echo before'\n\n";
    let suffix = "\n\n# Exact suffix stays.\n[tasks.after]\nrun = 'echo after'\n";
    let stale = "[tasks.check]\ndepends = ['check:rust']\n[tasks.format]\ndepends = ['format:rust']\n[tasks.test]\ndepends = ['test:rust']\n[tasks.deps]\ndepends = ['deps:python']\n";
    let original = format!("{prefix}{AGGREGATE_START}\n{stale}{AGGREGATE_END}{suffix}");
    let root = f.write("mise.toml", &original);
    f.write("report.py", "pass\n");
    f.succeed(&["--tools", "lua", "--apply"]);
    assert_central_dependencies(&f.project, &["lua"], &[]);
    assert_root_preserved(&root, original.as_bytes());
    assert_backup(&f.project, "mise.toml", &original);
    let lua_root = fs::read(&root).unwrap();
    let before = snapshot(&f.root);
    f.succeed(&["--tools", "lua", "--apply"]);
    assert_eq!(snapshot(&f.root), before);
    let args = ["--tools", "python", "--python-mode", "scripts", "--apply"];
    f.succeed(&args);
    assert_central_dependencies(&f.project, &["python"], &[]);
    let central = central_tasks(&f.project);
    assert!(strings(&central["check"]["depends"]).contains(&"check:python".into()));
    assert!(strings(&central["format"]["depends"]).contains(&"format:python".into()));
    assert!(central.get("test").is_none());
    assert!(central.get("deps").is_none());
    assert_root_preserved(&root, original.as_bytes());
    assert!(
        backups(&f.project, "mise.toml")
            .iter()
            .any(|p| fs::read(p).unwrap() == lua_root)
    );
    let before = snapshot(&f.root);
    f.succeed(&args);
    assert_eq!(snapshot(&f.root), before);
}
#[test]
fn test_malformed_root_toml_or_aggregate_markers_fail_before_any_writes() {
    let (start, end) = (AGGREGATE_START, AGGREGATE_END);
    let invalid = [
        format!("{start}\n"),
        format!("{end}\n"),
        format!("{end}\n{start}\n"),
        format!("{start}\n{start}\n{end}\n"),
        format!("{start}\n{end}\n{end}\n"),
        format!("{start}\n{end}\n{start}\n{end}\n"),
        format!("{start}\n[tasks.\n{end}\n"),
        "[tools]\npython = [\n".into(),
    ];
    for root_name in ["mise.toml", ".mise.toml"] {
        for contents in &invalid {
            let f = Fixture::new();
            f.write(root_name, contents);
            f.write("AGENTS.md", "# Keep local instructions\n");
            f.write("stylua.toml", "syntax = 'Lua54'\n");
            f.refuse(&["--tools", "lua", "--overwrite", "--apply"]);
        }
    }
}
#[test]
fn test_overwrite_preview_never_replaces_or_deletes_existing_configs() {
    let f = Fixture::new();
    files(
        &f.project,
        &[
            (
                "package.json",
                "{\"private\":true,\"packageManager\":\"npm@10.9.2\"}\n",
            ),
            (
                "mise.toml",
                "# Preserve existing root settings.\n[vars]\ncustom = 'keep'\n",
            ),
            ("prettier.config.js", "export default { printWidth: 79 };\n"),
            (".prettierrc.json", "{\"printWidth\":81}\n"),
            ("AGENTS.md", "# Preserve local instructions\n"),
        ],
    );
    for extra in [vec![], vec!["--dry-run"]] {
        let before = snapshot(&f.root);
        f.succeed(
            &[
                vec!["--tools", "markdown", "--overwrite", "--install-deps"],
                extra,
            ]
            .concat(),
        );
        assert_eq!(snapshot(&f.root), before);
    }
}
#[cfg(unix)]
#[test]
fn test_overwrite_rejects_outside_symlinks_before_config_replacement_or_deletion() {
    for (tool, relative, contents) in [
        ("markdown", "prettier.config.js", "export default {};\n"),
        ("markdown", ".prettierrc.cjs", "module.exports = {};\n"),
        ("markdown", ".prettierrc.toml", "printWidth = 79\n"),
        (
            "rust",
            "rust-toolchain.toml",
            "[toolchain]\nchannel = '1.85.1'\n",
        ),
    ] {
        let f = Fixture::new();
        let target = write(&f.outside, "config", contents);
        std::os::unix::fs::symlink(target, f.project.join(relative)).unwrap();
        let mut args = vec!["--tools", tool, "--overwrite", "--apply"];
        if tool == "rust" {
            files(
                &f.project,
                &[
                    (
                        "Cargo.toml",
                        "[package]\nname = 'sample'\nversion = '0.1.0'\n",
                    ),
                    ("src/lib.rs", "pub fn answer() -> u32 { 42 }\n"),
                ],
            );
            args.push("--overwrite-rust-toolchain");
        } else {
            files(
                &f.project,
                &[
                    ("package.json", "{}\n"),
                    (".prettierrc.json", "{\"printWidth\":81}\n"),
                ],
            );
        }
        f.write("AGENTS.md", "# Existing local instructions\n");
        f.refuse(&args);
    }
}
#[test]
fn test_overwrite_replaces_canonical_configs_and_cleans_up_alternates_with_backups() {
    for (tool, canonical, alternate, original, old_alternate) in [
        (
            "python",
            "ruff.toml",
            ".ruff.toml",
            "line-length = 79\n",
            "line-length = 81\n",
        ),
        (
            "javascript",
            "biome.json",
            "biome.jsonc",
            "{\"formatter\":{\"lineWidth\":79}}\n",
            "{\n// Alternate config\n\"formatter\":{\"lineWidth\":81}\n}\n",
        ),
        (
            "lua",
            "stylua.toml",
            ".stylua.toml",
            "indent_width = 2\n",
            "indent_width = 3\n",
        ),
    ] {
        let f = Fixture::new();
        let manifests = match tool {
            "python" => python_project(&f.project, false),
            "javascript" => files(&f.project, &[("package.json", "{}\n")]),
            _ => Files::new(),
        };
        f.write(canonical, original);
        f.write(alternate, old_alternate);
        let args = ["--tools", tool, "--overwrite", "--apply"];
        f.succeed(&args);
        let contents = read(&f.project, canonical);
        assert_ne!(contents, original);
        if canonical.ends_with(".json") {
            assert!(
                serde_json::from_str::<Value>(&contents)
                    .unwrap()
                    .is_object()
            );
        } else {
            toml_file(f.project.join(canonical));
        }
        assert!(!f.project.join(alternate).exists());
        assert_backup(&f.project, canonical, original);
        assert_backup(&f.project, alternate, old_alternate);
        assert_preserved(&f.project, &manifests);
        let before = snapshot(&f.root);
        f.succeed(&args);
        assert_eq!(snapshot(&f.root), before);
    }
}
#[test]
fn test_explicit_rust_toolchain_overwrite_opt_in_replaces_pin_with_backup() {
    let expected =
        toml_file(templates().join("rust/rust-toolchain.toml"))["toolchain"]["channel"].clone();
    for (relative, original) in [
        (
            "engine/rust-toolchain.toml",
            "[toolchain]\nchannel = '1.85.1'\ncomponents = ['rustfmt']\n",
        ),
        ("engine/rust-toolchain", "1.85.1\n"),
    ] {
        let f = Fixture::new();
        let originals = files(
            &f.project,
            &[
                (
                    "engine/Cargo.toml",
                    "[package]\nname = 'engine'\nversion = '0.1.0'\nedition = '2021'\n",
                ),
                ("engine/Cargo.lock", "version = 4\n"),
                ("engine/src/lib.rs", "pub fn answer() -> u32 { 42 }\n"),
            ],
        );
        f.write(relative, original);
        let args = [
            "--tools",
            "rust",
            "--rust-dir",
            "engine",
            "--overwrite",
            "--overwrite-rust-toolchain",
            "--apply",
        ];
        f.succeed(&args);
        assert_eq!(
            toml_file(f.project.join("engine/rust-toolchain.toml"))["toolchain"]["channel"],
            expected
        );
        assert!(!f.project.join("engine/rust-toolchain").exists());
        assert_ne!(read(&f.project, "engine/rust-toolchain.toml"), original);
        assert_backup(&f.project, relative, original);
        assert_preserved(&f.project, &originals);
        let before = snapshot(&f.root);
        f.succeed(&args);
        assert_eq!(snapshot(&f.root), before);
    }
}
#[test]
fn test_tools_are_required_and_unknown_choices_are_rejected() {
    let f = Fixture::new();
    for args in [vec![], vec!["--tools", "unknown-tool"]] {
        let before = snapshot(&f.root);
        let (code, output) = f.invoke(&args);
        assert_ne!(code, 0);
        assert!(!output.trim().is_empty());
        assert_eq!(snapshot(&f.root), before);
    }
}

#[test]
fn failed_command_stops_dispatch_after_files_and_backups_are_applied() {
    let f = Fixture::new();
    f.write("config.txt", "before");
    let mut plan = xerrion_dev_tooling_installer::plan::Plan::new(f.project.clone());
    plan.stage("config.txt", "after").unwrap();
    plan.command(vec!["first".into()], &f.project);
    plan.command(vec!["second".into()], &f.project);
    let mut calls = 0;
    let error = plan
        .apply_with(|command| {
            calls += 1;
            assert_eq!(command.argv, ["first"]);
            assert_eq!(read(&f.project, "config.txt"), "after");
            assert_backup(&f.project, "config.txt", "before");
            anyhow::bail!("Simulated command failure")
        })
        .unwrap_err();
    assert_eq!(calls, 1);
    assert!(error.to_string().contains("Simulated command failure"));
    assert_eq!(read(&f.project, "config.txt"), "after");
}
#[test]
fn process_runner_reports_nonzero_exit_status() {
    let f = Fixture::new();
    let mut plan = xerrion_dev_tooling_installer::plan::Plan::new(f.project.clone());
    plan.command(
        vec![
            env!("CARGO_BIN_EXE_installer").into(),
            "--templates".into(),
            templates().to_str().unwrap().into(),
            "--unknown-option".into(),
        ],
        &f.project,
    );
    let before = snapshot(&f.root);
    let error = plan.apply().unwrap_err();
    assert!(error.to_string().contains("Command failed"));
    assert_eq!(snapshot(&f.root), before);
}
