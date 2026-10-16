use crate::{
    catalog::Catalog,
    cli::Options,
    data::shell_join,
    plan::{END, START, inside, update_managed},
    project,
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq, Eq)]
pub struct DetectedTool {
    pub directories: Vec<PathBuf>,
    pub evidence: Vec<PathBuf>,
}
pub type DetectedStack = BTreeMap<String, DetectedTool>;

fn scan(directory: &Path, root: &Path, files: &mut Vec<PathBuf>, depth: usize) -> Result<()> {
    ensure!(
        depth <= 32,
        "Stack scan exceeded 32 directory levels; use --tools to select explicitly"
    );
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("Cannot scan {}", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let kind = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if name.starts_with('.')
                || [
                    "node_modules",
                    "target",
                    "vendor",
                    "dist",
                    "build",
                    "coverage",
                    "__pycache__",
                    "venv",
                ]
                .contains(&name.as_ref())
            {
                continue;
            }
            scan(&entry.path(), root, files, depth + 1)?;
        } else if kind.is_file() {
            ensure!(
                files.len() < 100_000,
                "Stack scan exceeded 100000 files; use --tools to select explicitly"
            );
            let path = entry.path();
            // Generated instructions must not move Markdown detection on the next run.
            if path
                .extension()
                .is_some_and(|ext| ext == "md" || ext == "markdown")
                && let Ok(text) = fs::read_to_string(&path)
                && text.trim_start().starts_with(START)
                && update_managed(&text, "", START, END).is_ok_and(|rest| rest.trim().is_empty())
            {
                continue;
            }
            files.push(path.strip_prefix(root)?.to_path_buf());
        }
    }
    Ok(())
}

fn directory(path: &Path) -> PathBuf {
    match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(parent) => parent.into(),
        None => ".".into(),
    }
}

fn common_directory(files: &[PathBuf]) -> PathBuf {
    let mut common = directory(&files[0]);
    for file in files.iter().skip(1) {
        while common != Path::new(".") && !file.starts_with(&common) {
            common = directory(&common);
        }
    }
    common
}

pub fn detect(project: &Path, catalog: &Catalog) -> Result<DetectedStack> {
    let root = project
        .canonicalize()
        .context("Project directory does not exist")?;
    ensure!(
        root.is_dir(),
        "Project must be a directory: {}",
        root.display()
    );
    let mut files = vec![];
    scan(&root, &root, &mut files, 0)?;
    files.sort();
    let mut stack = BTreeMap::new();
    for (id, tool) in &catalog.tools {
        let patterns = tool
            .detection
            .files
            .iter()
            .map(|pattern| glob::Pattern::new(pattern))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let anchors: Vec<_> = files
            .iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| patterns.iter().any(|pattern| pattern.matches(name)))
            })
            .cloned()
            .collect();
        let evidence = if anchors.is_empty() {
            files
                .iter()
                .filter(|path| {
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| tool.detection.extensions.iter().any(|e| e == ext))
                })
                .cloned()
                .collect::<Vec<_>>()
        } else {
            anchors.clone()
        };
        if evidence.is_empty() {
            continue;
        }
        let directories = if anchors.is_empty() {
            vec![common_directory(&evidence)]
        } else {
            let candidates: BTreeSet<_> = anchors.iter().map(|path| directory(path)).collect();
            candidates
                .iter()
                .filter(|path| {
                    !candidates.iter().any(|other| {
                        other != *path && (other == Path::new(".") || path.starts_with(other))
                    })
                })
                .cloned()
                .collect()
        };
        stack.insert(
            id.clone(),
            DetectedTool {
                directories,
                evidence,
            },
        );
    }
    Ok(stack)
}

pub fn resolve_directories(options: &mut Options, stack: &DetectedStack) -> Result<()> {
    let root = options.project.canonicalize()?;
    let (existing, _) = project::existing_mise(&root)?;
    for id in &options.tools {
        if options.dirs.contains_key(id) || existing["vars"].get(format!("{id}_dir")).is_some() {
            continue;
        }
        if let Some(found) = stack.get(id) {
            ensure!(
                found.directories.len() == 1,
                "Multiple {id} project directories detected: {}. Select one with --{id}-dir or --interactive",
                found
                    .directories
                    .iter()
                    .map(|p| crate::ui::clean(&p.display().to_string()))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            options
                .dirs
                .insert(id.clone(), found.directories[0].to_string_lossy().into());
        }
    }
    Ok(())
}

pub fn file_defaults(options: &mut Options, stack: &DetectedStack) -> Result<()> {
    let root = options.project.canonicalize()?;
    let (existing, _) = project::existing_mise(&root)?;
    for id in ["docker", "shell"] {
        if !options.tools.iter().any(|tool| tool == id) {
            continue;
        }
        let Some(found) = stack.get(id) else { continue };
        let var = format!("{id}_dir");
        let rel = options
            .dirs
            .get(id)
            .map(String::as_str)
            .or_else(|| existing["vars"][&var].as_str())
            .unwrap_or(".");
        let base = inside(&root, rel)?;
        let mut paths = found
            .evidence
            .iter()
            .filter_map(|path| {
                root.join(path)
                    .strip_prefix(&base)
                    .ok()
                    .map(Path::to_path_buf)
            })
            .collect::<Vec<_>>();
        paths.sort();
        if paths.is_empty() {
            continue;
        }
        let names = paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        options
            .detected_vars
            .insert(format!("{id}_files"), shell_join(&names)?);
        if id == "docker" {
            let primary = names
                .iter()
                .find(|name| *name == "Dockerfile")
                .or_else(|| names.iter().find(|name| *name == "Containerfile"))
                .unwrap_or(&names[0]);
            options
                .detected_vars
                .insert("docker_file".into(), primary.clone());
        }
    }
    Ok(())
}
