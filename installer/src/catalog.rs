use crate::{
    data::{load_toml, table},
    plan::inside,
};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Adapter {
    #[default]
    Generic,
    Python,
    Javascript,
    Rust,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub source: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub alternatives: Vec<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guidance {
    pub task: String,
    pub when: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub schema_version: u32,
    pub title: String,
    pub profile: String,
    #[serde(default)]
    pub adapter: Adapter,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub configs: Vec<Config>,
    #[serde(default)]
    pub guidance: Vec<Guidance>,
    #[serde(default)]
    pub aggregates: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub install_dependencies: Vec<String>,
    #[serde(default)]
    pub dependency_files: Vec<String>,
    pub gitignore: Option<String>,
}
pub struct Catalog {
    pub root: PathBuf,
    pub tools: BTreeMap<String, Tool>,
    pub aliases: BTreeMap<String, String>,
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}
impl Catalog {
    pub fn load(root: &Path) -> Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("Cannot find templates at {}", root.display()))?;
        let mut catalog = Self {
            root,
            tools: BTreeMap::new(),
            aliases: BTreeMap::new(),
        };
        let mut paths = fs::read_dir(inside(&catalog.root, "catalog")?)?
            .map(|entry| entry.map(|e| e.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();
        for path in paths {
            if path.extension().is_none_or(|e| e != "toml") {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .context("Invalid catalog filename")?
                .to_owned();
            ensure!(valid_name(&name), "Invalid catalog id: {name}");
            let path = inside(&catalog.root, path.strip_prefix(&catalog.root)?)?;
            let tool: Tool = toml::from_str(&fs::read_to_string(&path)?)
                .with_context(|| format!("Invalid catalog entry {}", path.display()))?;
            ensure!(
                tool.schema_version == 1,
                "Unsupported schema_version in {name}: {}",
                tool.schema_version
            );
            let adapter_id = match tool.adapter {
                Adapter::Generic => None,
                Adapter::Python => Some("python"),
                Adapter::Javascript => Some("javascript"),
                Adapter::Rust => Some("rust"),
            };
            ensure!(
                adapter_id.is_none_or(|id| id == name),
                "The {:?} adapter is reserved for its built-in profile; use generic for {name}",
                tool.adapter
            );
            for file in &tool.dependency_files {
                ensure!(!file.is_empty(), "Empty dependency file in {name}");
                inside(&catalog.root, file)?;
            }
            let profile = inside(&catalog.root, &tool.profile)?;
            ensure!(profile.is_file(), "Missing profile: {}", profile.display());
            let data = load_toml(&profile)?;
            for section in ["tools", "vars", "tasks"] {
                table(&data, section)?;
            }
            let tasks = table(&data, "tasks")?;
            let mut destinations = BTreeSet::new();
            for config in &tool.configs {
                let source = inside(&catalog.root, &config.source)?;
                ensure!(source.is_file(), "Missing template: {}", source.display());
                ensure!(
                    destinations.insert(source.file_name().map(|s| s.to_owned())),
                    "Duplicate config destination in {name}"
                );
                for alt in &config.alternatives {
                    ensure!(
                        Path::new(alt).components().count() == 1 && !alt.is_empty() && alt != ".",
                        "Config alternatives must be filenames: {alt}"
                    );
                    inside(&catalog.root, alt)?;
                }
            }
            for (aggregate, deps) in &tool.aggregates {
                ensure!(
                    ["check", "format", "test", "deps"].contains(&aggregate.as_str()),
                    "Unknown aggregate {aggregate} in {name}"
                );
                for dep in deps {
                    ensure!(
                        tasks.contains_key(dep),
                        "Unknown task {dep} in {name} aggregate"
                    );
                }
            }
            for item in &tool.guidance {
                ensure!(
                    tasks.contains_key(&item.task),
                    "Unknown guidance task {} in {name}",
                    item.task
                );
            }
            for task in &tool.install_dependencies {
                ensure!(
                    tasks.contains_key(task),
                    "Unknown installation task {task} in {name}"
                );
            }
            catalog.tools.insert(name, tool);
        }
        ensure!(!catalog.tools.is_empty(), "Tool catalog is empty");
        for (id, tool) in &catalog.tools {
            for alias in &tool.aliases {
                ensure!(
                    valid_name(alias)
                        && !catalog.tools.contains_key(alias)
                        && !catalog.aliases.contains_key(alias),
                    "Duplicate or invalid alias: {alias}"
                );
                catalog.aliases.insert(alias.clone(), id.clone());
            }
        }
        Ok(catalog)
    }
    pub fn profile(&self, id: &str) -> Result<Value> {
        load_toml(&inside(&self.root, &self.tools[id].profile)?)
    }
    pub fn resolve(&self, name: &str) -> Option<&str> {
        self.tools
            .get_key_value(name)
            .map(|(id, _)| id.as_str())
            .or_else(|| self.aliases.get(name).map(String::as_str))
    }
}
