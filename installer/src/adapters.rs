use crate::{data::*, plan::*, project::delegation_cycle};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const LOCKS: &[(&str, &str)] = &[
    ("bun", "bun.lock"),
    ("bun", "bun.lockb"),
    ("pnpm", "pnpm-lock.yaml"),
    ("npm", "package-lock.json"),
    ("yarn", "yarn.lock"),
];
#[derive(Debug)]
pub struct Package {
    pub manager: String,
    pub data: Value,
    pub owner: PathBuf,
}
fn workspace(project: &Path, location: &Path, data: &Value) -> Result<bool> {
    let has_workspaces = data.get("workspaces").is_some_and(|v| match v {
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        _ => false,
    });
    Ok(has_workspaces
        || inside(
            project,
            relative(project, location)?.join("pnpm-workspace.yaml"),
        )?
        .exists())
}
pub fn package_manager(directory: &Path, project: &Path) -> Result<Package> {
    let mut entries = vec![];
    let mut current = directory;
    loop {
        let path = inside(project, relative(project, current)?.join("package.json"))?;
        entries.push((current.to_path_buf(), load_json(&path)?));
        if current == project {
            break;
        }
        current = current.parent().context("Package search escaped project")?;
    }
    let mut manager = "bun".to_owned();
    let mut owner = directory.to_path_buf();
    let mut request = String::new();
    for (location, data) in &entries {
        let declaration = string(data, "packageManager", "")?;
        if !declaration.is_empty() {
            manager = declaration.split('@').next().unwrap_or_default().into();
            ensure!(
                ["bun", "npm", "pnpm", "yarn"].contains(&manager.as_str()),
                "Unsupported package manager: {declaration}"
            );
            owner = location.clone();
            request = declaration.into();
            break;
        }
        let mut found = false;
        for (name, lock) in LOCKS {
            if inside(project, relative(project, location)?.join(lock))?.exists() {
                manager = (*name).into();
                owner = location.clone();
                found = true;
                break;
            }
        }
        if found {
            break;
        }
    }
    let mut child = entries[0].1.clone();
    if !request.is_empty() && string(&child, "packageManager", "")?.is_empty() {
        child["packageManager"] = json!(request);
    }
    let child_has_package = directory.join("package.json").exists();
    for (location, data) in &entries {
        let is_workspace = workspace(project, location, data)?;
        let mut matching_lock = false;
        for (name, lock) in LOCKS {
            if *name == manager
                && inside(project, relative(project, location)?.join(lock))?.exists()
            {
                matching_lock = true;
            }
        }
        if (location == directory || is_workspace || !child_has_package)
            && (matching_lock || location == &owner)
        {
            owner = location.clone();
            break;
        }
    }
    if child_has_package && owner != directory {
        let data = &entries
            .iter()
            .find(|(location, _)| location == &owner)
            .context("Missing package owner")?
            .1;
        if !workspace(project, &owner, data)? {
            owner = directory.to_path_buf();
        }
    }
    Ok(Package {
        manager,
        data: child,
        owner,
    })
}
pub fn manager_tools(profile: &mut Value, package: &Package) -> Result<()> {
    if package.manager != "bun" {
        object_mut(&mut profile["tools"])?.remove("bun");
    }
    if ["pnpm", "yarn"].contains(&package.manager.as_str()) {
        let request = string(&package.data, "packageManager", "")?;
        let version = request
            .split_once('@')
            .map(|(_, version)| version.split('+').next().unwrap_or("latest"))
            .unwrap_or("latest");
        profile["tools"][&package.manager] = json!(version);
    }
    Ok(())
}
pub fn javascript(
    profile: &mut Value,
    directory: &Path,
    existing: &Value,
    plan: &mut Plan,
) -> Result<()> {
    let package = package_manager(directory, &plan.project)?;
    let manager = &package.manager;
    let locked = |name: &str| package.owner.join(name).exists();
    let install = match manager.as_str() {
        "bun" if locked("bun.lock") || locked("bun.lockb") => "bun install --frozen-lockfile",
        "bun" => "bun install",
        "npm" if locked("package-lock.json") => "npm ci",
        "npm" => "npm install",
        "pnpm" if locked("pnpm-lock.yaml") => "pnpm install --frozen-lockfile",
        "pnpm" => "pnpm install",
        _ => "yarn install",
    };
    profile["tasks"]["deps:javascript"]["run"] = json!(install);
    profile["tasks"]["deps:javascript"]["dir"] =
        json!(display_relative(&plan.project, &package.owner)?);
    profile["tasks"]["deps:javascript"]["description"] =
        json!(format!("Install JavaScript dependencies using {manager}"));
    manager_tools(profile, &package)?;
    for action in ["typecheck", "test"] {
        let name = format!("{action}:javascript");
        if existing["tasks"].get(&name).is_some() {
            continue;
        }
        if package.data["scripts"].get(action).is_some() {
            profile["tasks"][&name]["run"] = json!(format!("{manager} run {action}"));
        } else if existing["tasks"].get(action).is_some() {
            let mut graph = table(profile, "tasks")?;
            graph.extend(table(existing, "tasks")?);
            if delegation_cycle(action, &name, &graph) {
                object_mut(&mut profile["tasks"])?.remove(&name);
                plan.notes.push(format!("WARNING: omitted {name}: delegating to {action} would create a dependency cycle"));
            } else {
                profile["tasks"][&name] = json!({"description": format!("Run the project's {action} task and prerequisites"), "depends": [action]});
            }
        } else {
            object_mut(&mut profile["tasks"])?.remove(&name);
            plan.notes.push(format!(
                "WARNING: omitted {name}: no package script or existing project task named {action}"
            ));
        }
    }
    if !directory.join("package.json").exists() {
        plan.notes.push("WARNING: JavaScript has no package.json; create the application package before installing dependencies".into());
    }
    Ok(())
}
pub fn python_mode(
    args: &crate::cli::Options,
    existing: &Value,
    directory: &Path,
    project: &Path,
) -> Result<(bool, String, Value)> {
    let data = load_toml(&inside(
        project,
        relative(project, directory)?.join("pyproject.toml"),
    )?)?;
    let is_project = data.get("project").is_some() || data["tool"]["uv"].get("workspace").is_some();
    ensure!(
        args.python_mode != "project" || is_project,
        "Python project mode requires project or uv workspace metadata in pyproject.toml"
    );
    let scripts = args.python_mode == "scripts" || (args.python_mode == "auto" && !is_project);
    let mut version = args
        .python_version
        .clone()
        .unwrap_or(string(&existing["vars"], "python_version", "3.14")?.into());
    if args.python_version.is_none() && existing["vars"].get("python_version").is_none() {
        let pattern = regex::Regex::new(r"(?:>=|==|~=)\s*(3\.\d+)")?;
        if let Some(captures) = pattern.captures(string(&data["project"], "requires-python", "")?) {
            version = captures[1].into();
        }
    }
    ensure!(
        regex::Regex::new(r"^3\.\d+$")?.is_match(&version),
        "--python-version must be a major.minor version, such as 3.14"
    );
    Ok((scripts, version, data))
}
pub fn python(
    profile: &mut Value,
    args: &crate::cli::Options,
    existing: &Value,
    directory: &Path,
    plan: &mut Plan,
    state: &(bool, String, Value),
) -> Result<()> {
    let (scripts, version, pyproject) = state;
    let inherited = shlex::split(string(&existing["vars"], "python_files", "")?)
        .context("Invalid quoting in python_files")?;
    let detected: Vec<String> = ["src", "tests"]
        .iter()
        .filter(|name| directory.join(name).is_dir())
        .map(|s| (*s).into())
        .collect();
    let files = args.python_files.clone().unwrap_or_else(|| {
        if !inherited.is_empty() {
            inherited
        } else if !detected.is_empty() {
            detected
        } else {
            vec![".".into()]
        }
    });
    for name in &files {
        inside(
            &plan.project,
            relative(&plan.project, directory)?.join(name),
        )?;
    }
    profile["vars"]["python_files"] = json!(shell_join(&files)?);
    profile["vars"]["python_version"] = json!(version);
    if let Some(executable) = &args.python_executable {
        inside(
            &plan.project,
            relative(&plan.project, directory)?.join(executable),
        )?;
        profile["vars"]["python_executable"] = json!(executable);
    } else if cfg!(windows) {
        profile["vars"]["python_executable"] = json!(".venv/Scripts/python.exe");
    }
    if *scripts {
        let tasks = object_mut(&mut profile["tasks"])?;
        tasks.remove("deps:python");
        tasks.remove("test:python");
        if args.python_executable.is_none() {
            profile["tasks"]["typecheck:python"]["run"] = json!(
                "ty check --python-version {{ vars.python_version | quote }} {{ vars.python_files }}"
            );
        }
        plan.notes.push(
            "Python scripts mode: no uv project, dependency sync, or pytest task is created".into(),
        );
    } else if pyproject["dependency-groups"].get("dev").is_none() {
        profile["tasks"]["deps:python"]["run"] = json!("uv sync --locked");
    }
    Ok(())
}

pub fn python_dependencies(plan: &mut Plan, directory: &Path, data: &Value) -> Result<()> {
    let dev = data["dependency-groups"]["dev"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let dependencies = data["project"]["dependencies"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let pattern = regex::Regex::new(r"(?i)^pytest(?:\s|[<>=!~;\[]|$)")?;
    let pytest_present = dev
        .iter()
        .chain(&dependencies)
        .any(|v| v.as_str().is_some_and(|v| pattern.is_match(v)));
    if !pytest_present {
        plan.command(
            vec!["mise", "exec", "--", "uv", "add", "--dev", "pytest"]
                .into_iter()
                .map(String::from)
                .collect(),
            directory,
        );
    }
    let mut command: Vec<String> = vec!["mise", "exec", "--", "uv", "sync"]
        .into_iter()
        .map(String::from)
        .collect();
    if directory.join("uv.lock").exists() || !pytest_present {
        command.push("--locked".into());
    }
    if !dev.is_empty() || !pytest_present {
        command.extend(["--group".into(), "dev".into()]);
    }
    plan.command(command, directory);
    Ok(())
}
pub fn rust_toolchain(
    plan: &mut Plan,
    directory: &Path,
    configs: &[crate::catalog::Config],
) -> Result<()> {
    let config = configs
        .iter()
        .find(|c| c.kind == "rust-toolchain")
        .context("Rust adapter needs a rust-toolchain config")?;
    let found = crate::project::existing_config(&plan.project, directory, config)?;
    let rel = relative(&plan.project, directory)?.join("rust-toolchain.toml");
    let toolchain = if let Some(change) = plan.changes.get(&rel)
        && let Some(bytes) = &change.after
    {
        parse_toml(std::str::from_utf8(bytes)?, "planned toolchain")?["toolchain"].clone()
    } else if let Some(path) = found {
        let text = std::fs::read_to_string(&path)?;
        if path.extension().is_some_and(|s| s == "toml") || text.trim().starts_with('[') {
            load_toml(&path)?["toolchain"].clone()
        } else {
            json!({"channel": text.trim()})
        }
    } else {
        json!({"channel": "stable"})
    };
    let mut argv: Vec<String> = vec![
        "rustup",
        "toolchain",
        "install",
        string(&toolchain, "channel", "stable")?,
        "--profile",
        string(&toolchain, "profile", "minimal")?,
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let mut components = strings(&toolchain["components"])?;
    for component in ["clippy", "rustfmt"] {
        if !components.iter().any(|s| s == component) {
            components.push(component.into());
        }
    }
    for component in components {
        argv.extend(["--component".into(), component]);
    }
    for target in strings(&toolchain["targets"])? {
        argv.extend(["--target".into(), target]);
    }
    plan.command(argv, directory);
    Ok(())
}
