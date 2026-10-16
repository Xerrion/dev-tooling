pub mod adapters;
pub mod catalog;
pub mod cli;
pub mod data;
pub mod detection;
pub mod interactive;
pub mod plan;
pub mod project;
pub mod selector;
pub mod ui;

use anyhow::{Context, Result, bail, ensure};
use catalog::{Adapter, Catalog};
use cli::Options;
use data::*;
use plan::*;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

fn agent_block(catalog: &Catalog, selected: &[String], tasks: &Table, scripts: bool) -> String {
    let mut lines = vec![START.into(), "## Development tooling".into(), String::new(),
        "Run these mise tasks from the repository root. Existing project instructions and prerequisite tasks still apply.".into(),
        "Review formatting and safe-fix diffs; do not rewrite unrelated files or change tool versions as part of a routine check.".into(), String::new()];
    for (name, when) in [
        (
            "check",
            "Before finishing changes across stacks, run all configured lint, formatting, and type checks.",
        ),
        (
            "format",
            "To format every selected language, run the aggregate formatter and review the diff.",
        ),
        (
            "test",
            "After cross-stack behavior changes, run all configured test suites and their prerequisites.",
        ),
        (
            "deps",
            "When project dependencies changed or are missing, sync dependencies for selected projects.",
        ),
    ] {
        if tasks.contains_key(name) {
            lines.push(format!("- {when} Use `mise run {name}`."));
        }
    }
    for id in selected {
        let tool = &catalog.tools[id];
        lines.extend([String::new(), format!("### {}", tool.title), String::new()]);
        for item in &tool.guidance {
            if tasks.contains_key(&item.task) {
                lines.push(format!("- {} Use `mise run {}`.", item.when, item.task));
            }
        }
        if tool.adapter == Adapter::Python && scripts {
            lines.push("- This is script-only Python tooling. Do not create a uv project or pytest suite just to run these checks; retain the project's existing script setup and validation tasks.".into());
        }
    }
    lines.extend([String::new(), "Tool versions, variables, and tasks in higher-precedence mise files override the generated fragment. Check `mise config` and `mise tasks ls` when behavior differs.".into(),
        "Run installation or dependency-sync commands only when needed; they can download packages or execute dependency build scripts.".into(), END.into()]);
    lines.join("\n")
}

pub fn build_plan(args: &Options, catalog: &Catalog) -> Result<Plan> {
    let project = args.project.canonicalize().with_context(|| {
        format!(
            "Project directory does not exist: {}",
            args.project.display()
        )
    })?;
    ensure!(
        project.is_dir(),
        "Project directory does not exist: {}",
        project.display()
    );
    let mut plan = Plan::new(project.clone());
    inside(&project, BACKUPS)?;
    let (existing, mise_paths) = project::existing_mise(&project)?;
    let mut directories = BTreeMap::new();
    for id in &args.tools {
        let var = format!("{id}_dir");
        let rel =
            args.dirs
                .get(id)
                .map(String::as_str)
                .unwrap_or(string(&existing["vars"], &var, ".")?);
        let directory = inside(&project, rel)?;
        ensure!(
            directory.is_dir(),
            "Tool working directory does not exist: {}",
            directory.display()
        );
        directories.insert(id.clone(), directory);
    }
    let mut generated = json!({"tools": {}, "vars": {}, "tasks": {}});
    let mut python_state = (
        false,
        args.python_version.clone().unwrap_or("3.14".into()),
        json!({}),
    );
    for id in &args.tools {
        if catalog.tools[id].adapter == Adapter::Python {
            python_state = adapters::python_mode(args, &existing, &directories[id], &project)?;
        }
    }
    for id in &args.tools {
        let mut profile = catalog.profile(id)?;
        for section in ["tools", "vars", "tasks"] {
            if profile.get(section).is_none() {
                profile[section] = json!({});
            }
        }
        let directory = &directories[id];
        profile["vars"][format!("{id}_dir")] = json!(display_relative(&project, directory)?);
        for (name, value) in &args.detected_vars {
            if name.starts_with(&format!("{id}_")) {
                profile["vars"][name] = json!(value);
            }
        }
        match catalog.tools[id].adapter {
            Adapter::Python => adapters::python(
                &mut profile,
                args,
                &existing,
                directory,
                &mut plan,
                &python_state,
            )?,
            Adapter::Javascript => {
                adapters::javascript(&mut profile, directory, &existing, &mut plan)?
            }
            _ => (),
        }
        for (name, version) in table(&profile, "tools")? {
            if let Some(pin) = existing["tools"].get(&name) {
                plan.notes.push(format!(
                    "KEEP mise tool {name}: existing project version {pin}"
                ));
                continue;
            }
            let request = if name == "node" {
                args.node_version
                    .as_ref()
                    .map(|s| json!(s))
                    .unwrap_or(version)
            } else {
                version
            };
            if let Some(old) = generated["tools"].get(&name)
                && old != &request
            {
                let hint = if name == "node" {
                    "pass --node-version for a shared Node version"
                } else {
                    "set a shared version in the profiles or the project's mise config"
                };
                bail!("Conflicting {name} versions in selected profiles; {hint}");
            }
            generated["tools"][name] = request;
        }
        for (name, value) in table(&profile, "vars")? {
            if let Some(old) = existing["vars"].get(&name) {
                let explicit = (name == format!("{id}_dir") && args.dirs.contains_key(id))
                    || (name == "python_files" && args.python_files.is_some())
                    || (name == "python_version" && args.python_version.is_some())
                    || (name == "python_executable" && args.python_executable.is_some());
                ensure!(
                    !explicit || old == &value,
                    "Existing mise variable {name} overrides this argument; update it in the project config first"
                );
                plan.notes.push(format!(
                    "KEEP mise variable {name}: existing project override"
                ));
                continue;
            }
            if let Some(old) = generated["vars"].get(&name) {
                ensure!(
                    old == &value,
                    "Conflicting variable {name} in selected profiles"
                );
            }
            generated["vars"][name] = value;
        }
        for (name, task) in table(&profile, "tasks")? {
            if existing["tasks"].get(&name).is_some() {
                plan.notes.push(format!(
                    "KEEP mise task {name}: existing project definition takes precedence"
                ));
                continue;
            }
            if let Some(old) = generated["tasks"].get(&name) {
                ensure!(old == &task, "Conflicting task {name} in selected profiles");
            }
            generated["tasks"][name] = task;
        }
        project::add_configs(&mut plan, catalog, id, directory, args, &python_state.1)?;
    }
    plan.stage(FRAGMENT, render_toml(&generated)?)?;
    let mut available = table(&generated, "tasks")?;
    available.extend(table(&existing, "tasks")?);
    let central = project::add_central(&mut plan, catalog, &args.tools, &available, &existing)?;
    for id in &args.tools {
        if let Some(ignore) = &catalog.tools[id].gitignore {
            let rel = relative(&project, &directories[id])?.join(".gitignore");
            if !inside(&project, &rel)?.exists() {
                plan.stage(rel, ignore.clone())?;
            }
        }
    }
    let mut ignore = plan.planned_text(".gitignore")?;
    if !ignore.lines().any(|line| line == "/.tooling-backups/") {
        if !ignore.is_empty() && !ignore.ends_with('\n') {
            ignore.push('\n');
        }
        ignore.push_str("/.tooling-backups/\n");
        plan.stage(".gitignore", ignore)?;
    }
    let agents = inside(&project, &args.agents_file)?;
    available.extend(central);
    let block = agent_block(catalog, &args.tools, &available, python_state.0);
    plan.stage(
        &args.agents_file,
        update_managed(&read_text(&agents)?, &block, START, END)?,
    )?;
    if args.install_tools || args.install_deps {
        plan.notes.push("Installation trusts project mise config. Hooks and package builds may execute project or third-party code; downloads can occur.".into());
        let mut paths = mise_paths;
        paths.push(project.join(project::central_filename(&project)?));
        paths.push(project.join(FRAGMENT));
        let mut seen = BTreeSet::new();
        for path in paths {
            if seen.insert(path.clone()) {
                plan.command(
                    vec!["mise".into(), "trust".into(), path.to_string_lossy().into()],
                    &project,
                );
            }
        }
    }
    if args.install_tools {
        plan.command(vec!["mise".into(), "install".into()], &project);
        for id in &args.tools {
            if catalog.tools[id].adapter == Adapter::Rust {
                adapters::rust_toolchain(&mut plan, &directories[id], &catalog.tools[id].configs)?;
            }
        }
    }
    if args.install_deps {
        for id in &args.tools {
            let directory = &directories[id];
            match catalog.tools[id].adapter {
                Adapter::Python if !python_state.0 => {
                    adapters::python_dependencies(&mut plan, directory, &python_state.2)?
                }
                Adapter::Javascript => plan.command(
                    vec!["mise".into(), "run".into(), "deps:javascript".into()],
                    &project,
                ),
                _ => (),
            }
            for task in &catalog.tools[id].install_dependencies {
                plan.command(vec!["mise".into(), "run".into(), task.clone()], &project);
            }
        }
        let mut backup_directories = BTreeSet::new();
        for directory in directories.values() {
            let mut current = directory.as_path();
            loop {
                backup_directories.insert(current.to_path_buf());
                if current == project {
                    break;
                }
                current = current.parent().context("Backup search escaped project")?;
            }
        }
        let dependency_files: BTreeSet<_> = args
            .tools
            .iter()
            .flat_map(|id| &catalog.tools[id].dependency_files)
            .collect();
        for directory in backup_directories {
            for filename in &dependency_files {
                let rel = relative(&project, &directory)?.join(filename);
                let path = inside(&project, &rel)?;
                if path.is_file() {
                    plan.dependency_backups.insert(rel, std::fs::read(path)?);
                }
            }
        }
    }
    plan.notes.push("Root/local mise files, parent configs, selected environments, and custom discovery settings can override this fragment. Verify with mise config and mise tasks ls.".into());
    Ok(plan)
}

pub fn templates_directory(args: &[std::ffi::OsString]) -> Result<PathBuf> {
    for (index, arg) in args.iter().enumerate() {
        if arg == "--templates" {
            return args
                .get(index + 1)
                .map(PathBuf::from)
                .context("--templates needs a path");
        }
        if let Some(path) = arg.to_str().and_then(|s| s.strip_prefix("--templates=")) {
            return Ok(PathBuf::from(path));
        }
    }
    let executable = std::env::current_exe()?;
    for ancestor in executable.ancestors().skip(1) {
        let path = ancestor.join("templates");
        if path.join("catalog").is_dir() {
            return Ok(path);
        }
    }
    bail!(
        "Cannot find templates beside the executable or its parent directories; pass --templates PATH"
    )
}
