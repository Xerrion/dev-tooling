use crate::{
    catalog::{Catalog, Config},
    cli::Options,
    data::{Table, load_toml, render_toml, table},
    plan::*,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

pub fn central_filename(project: &Path) -> Result<&'static str> {
    Ok(if inside(project, ".mise.toml")?.exists() {
        ".mise.toml"
    } else {
        "mise.toml"
    })
}
pub fn existing_mise(project: &Path) -> Result<(Value, Vec<PathBuf>)> {
    let mut merged = json!({"tools": {}, "vars": {}, "tasks": {}});
    let mut candidates = Vec::new();
    for base in [".config/mise", ".mise", "mise"] {
        let fragments = inside(project, format!("{base}/conf.d"))?;
        if fragments.exists() {
            let mut entries = fs::read_dir(&fragments)?
                .map(|e| e.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()?;
            entries.sort();
            entries.retain(|path| {
                !path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .starts_with('.')
            });
            candidates.extend(
                entries
                    .iter()
                    .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                    .cloned(),
            );
            for folder in entries.iter().filter(|p| p.is_dir()) {
                for name in ["mise.toml", "mise.local.toml"] {
                    candidates.push(folder.join(name));
                }
            }
        }
        candidates.push(project.join(base).join("config.toml"));
        if base == ".config/mise" {
            candidates.push(project.join(base).join("mise.toml"));
            candidates.push(project.join(".config/mise.toml"));
        }
    }
    for name in [
        "mise.toml",
        ".mise.toml",
        ".config/mise/config.local.toml",
        ".config/mise.local.toml",
        ".mise/config.local.toml",
        "mise/config.local.toml",
        "mise.local.toml",
        ".mise.local.toml",
    ] {
        candidates.push(project.join(name));
    }
    let mut paths = vec![];
    for candidate in candidates {
        let rel = relative(project, &candidate)?;
        if rel == Path::new(FRAGMENT) {
            continue;
        }
        let path = inside(project, rel)?;
        if !path.exists() {
            continue;
        }
        let mut text = fs::read_to_string(&path)?;
        // Validate the original as well as the unmanaged remainder.
        crate::data::parse_toml(&text, &path.display().to_string())?;
        if path.parent() == Some(project)
            && ["mise.toml", ".mise.toml"]
                .iter()
                .any(|name| path.ends_with(name))
        {
            text = update_managed(&text, "", AGGREGATE_START, AGGREGATE_END)?;
        }
        let data = crate::data::parse_toml(&text, &path.display().to_string())?;
        for section in ["tools", "vars", "tasks"] {
            crate::data::object_mut(&mut merged[section])?.extend(table(&data, section)?);
        }
        paths.push(path);
    }
    Ok((merged, paths))
}

pub fn existing_config(
    project: &Path,
    directory: &Path,
    config: &Config,
) -> Result<Option<PathBuf>> {
    let filename = Path::new(&config.source)
        .file_name()
        .context("Template has no filename")?
        .to_string_lossy()
        .into_owned();
    let names = if config.alternatives.is_empty() {
        vec![filename]
    } else {
        config.alternatives.clone()
    };
    let mut current = directory;
    loop {
        for name in &names {
            let path = inside(project, relative(project, current)?.join(name))?;
            if path.exists() {
                return Ok(Some(path));
            }
        }
        if ["ruff", "ty"].contains(&config.kind.as_str()) {
            let path = inside(project, relative(project, current)?.join("pyproject.toml"))?;
            if load_toml(&path)?["tool"].get(&config.kind).is_some() {
                return Ok(Some(path));
            }
        }
        if config.kind == "prettier" {
            let path = inside(project, relative(project, current)?.join("package.json"))?;
            if crate::data::load_json(&path)?.get("prettier").is_some() {
                return Ok(Some(path));
            }
        }
        if current == project {
            return Ok(None);
        }
        current = current.parent().context("Config search escaped project")?;
    }
}

pub fn add_configs(
    plan: &mut Plan,
    catalog: &Catalog,
    id: &str,
    directory: &Path,
    args: &Options,
    version: &str,
) -> Result<()> {
    for config in &catalog.tools[id].configs {
        let template = inside(&catalog.root, &config.source)?;
        let destination = directory.join(template.file_name().context("Template has no filename")?);
        let rel = relative(&plan.project, &destination)?;
        inside(&plan.project, &rel)?;
        let found = existing_config(&plan.project, directory, config)?;
        let replace = if config.kind == "rust-toolchain" {
            args.overwrite_rust_toolchain
        } else {
            args.overwrite
        };
        if let Some(found) = &found
            && !replace
        {
            plan.notes.push(format!(
                "KEEP {}: existing {} configuration",
                relative(&plan.project, found)?.display(),
                config.kind
            ));
            continue;
        }
        if replace {
            for name in &config.alternatives {
                let alternate = inside(
                    &plan.project,
                    relative(&plan.project, directory)?.join(name),
                )?;
                if alternate != destination {
                    plan.remove(relative(&plan.project, &alternate)?)?;
                }
            }
            if config.kind == "prettier" {
                let rel = relative(&plan.project, directory)?.join("package.json");
                let original = plan.planned_text(&rel)?;
                if !original.is_empty() {
                    let mut data: Value = serde_json::from_str(&original)?;
                    if crate::data::object_mut(&mut data)?
                        .remove("prettier")
                        .is_some()
                    {
                        let indent: &[u8] = if original.contains("\n\t") {
                            b"\t"
                        } else {
                            b"  "
                        };
                        let mut bytes = vec![];
                        let mut serializer = serde_json::Serializer::with_formatter(
                            &mut bytes,
                            serde_json::ser::PrettyFormatter::with_indent(indent),
                        );
                        serde::Serialize::serialize(&data, &mut serializer)?;
                        plan.stage(rel, String::from_utf8(bytes)? + "\n")?;
                    }
                }
            }
        }
        if let Some(found) = &found
            && found != &destination
        {
            plan.notes.push(format!(
                "WARNING: {} will override {} settings in {}",
                rel.display(),
                config.kind,
                relative(&plan.project, found)?.display()
            ));
        }
        let mut text = fs::read_to_string(template)?;
        if catalog.tools[id].adapter == crate::catalog::Adapter::Python {
            let (setting, target) = if config.kind == "ruff" {
                ("target-version", format!("py{}", version.replace('.', "")))
            } else {
                ("python-version", version.to_owned())
            };
            let pattern = regex::Regex::new(&format!(r#"(?m)^{setting}\s*=\s*"[^"]*""#))?;
            text = pattern
                .replace_all(&text, format!("{setting} = \"{target}\""))
                .into_owned();
        }
        if let Some(change) = plan.changes.get(&rel)
            && let Some(after) = &change.after
        {
            ensure!(
                after == text.as_bytes(),
                "Conflicting config templates for {}; select compatible profiles",
                rel.display()
            );
        }
        plan.stage(rel, text)?;
    }
    Ok(())
}

pub fn delegation_cycle(action: &str, alias: &str, tasks: &Table) -> bool {
    fn visit(name: &str, alias: &str, tasks: &Table, visited: &mut BTreeSet<String>) -> bool {
        if name == alias {
            return true;
        }
        if !visited.insert(name.into()) {
            return false;
        }
        let Some(task) = tasks.get(name) else {
            return false;
        };
        for key in ["depends", "depends_post"] {
            for dependency in task[key].as_array().into_iter().flatten() {
                let pattern = dependency
                    .as_str()
                    .or_else(|| dependency["task"].as_str())
                    .unwrap_or("");
                let Ok(pattern) = glob::Pattern::new(pattern) else {
                    continue;
                };
                for candidate in tasks
                    .keys()
                    .map(String::as_str)
                    .chain(std::iter::once(alias))
                {
                    if pattern.matches(candidate) && visit(candidate, alias, tasks, visited) {
                        return true;
                    }
                }
            }
        }
        false
    }
    visit(action, alias, tasks, &mut BTreeSet::new())
}

pub fn add_central(
    plan: &mut Plan,
    catalog: &Catalog,
    selected: &[String],
    available: &Table,
    existing: &Value,
) -> Result<Table> {
    let template = load_toml(&inside(&catalog.root, "mise/mise.toml")?)?;
    let mut tasks = Table::new();
    for (name, mut original) in table(&template, "tasks")? {
        if existing["tasks"].get(&name).is_some() {
            plan.notes
                .push(format!("KEEP mise task {name}: existing project aggregate"));
            continue;
        }
        let mut dependencies = BTreeSet::new();
        for id in selected {
            if let Some(deps) = catalog.tools[id].aggregates.get(&name) {
                dependencies.extend(
                    deps.iter()
                        .filter(|dep| available.contains_key(*dep))
                        .cloned(),
                );
            }
        }
        if !dependencies.is_empty() {
            original["depends"] = json!(dependencies);
            tasks.insert(name, original);
        }
    }
    let mut graph = available.clone();
    graph.extend(tasks.clone());
    for (name, task) in tasks.clone() {
        if crate::data::strings(&task["depends"])?
            .iter()
            .any(|dep| delegation_cycle(dep, &name, &graph))
        {
            plan.notes.push(format!(
                "WARNING: omitted {name}: aggregate would introduce a dependency cycle"
            ));
            tasks.remove(&name);
            graph.remove(&name);
        }
    }
    let filename = central_filename(&plan.project)?;
    for name in ["mise.toml", ".mise.toml"] {
        if name == filename {
            continue;
        }
        let path = inside(&plan.project, name)?;
        let text = read_text(&path)?;
        if text.contains(AGGREGATE_START) {
            plan.stage(
                name,
                update_managed(&text, "", AGGREGATE_START, AGGREGATE_END)?,
            )?;
        }
    }
    let path = inside(&plan.project, filename)?;
    let block = format!(
        "{AGGREGATE_START}\n{}{AGGREGATE_END}",
        render_toml(&json!({"tasks": tasks}))?
    );
    let after = update_managed(&read_text(&path)?, &block, AGGREGATE_START, AGGREGATE_END)?;
    crate::data::parse_toml(&after, filename)?;
    plan.stage(filename, after)?;
    Ok(tasks)
}
