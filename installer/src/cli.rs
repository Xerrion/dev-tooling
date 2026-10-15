use clap::{Arg, ArgAction, ArgMatches, Command};
use crate::catalog::Catalog;
use std::{collections::BTreeMap, path::PathBuf};
use anyhow::{Result, Context, ensure};

pub struct Options {
    pub project: PathBuf, pub tools: Vec<String>, pub dirs: BTreeMap<String, String>,
    pub apply: bool, pub overwrite: bool, pub overwrite_rust_toolchain: bool,
    pub install_tools: bool, pub install_deps: bool,
    pub python_files: Option<Vec<String>>, pub python_version: Option<String>, pub python_executable: Option<String>,
    pub python_mode: String, pub node_version: Option<String>, pub agents_file: String,
}
pub fn command(catalog: &Catalog) -> Command {
    let mut cmd = Command::new("xerrion-dev-tooling").version(env!("CARGO_PKG_VERSION"))
        .about("Preview or apply tooling profiles from a TOML catalog")
        .arg(Arg::new("project").required_unless_present("list-tools").help("Existing project directory"))
        .arg(Arg::new("tools").long("tools").num_args(1..).required_unless_present("list-tools")
            .value_parser(catalog.tools.keys().chain(catalog.aliases.keys()).cloned().collect::<Vec<_>>()))
        .arg(Arg::new("templates").long("templates").help("Templates directory containing catalog/ and mise/"))
        .arg(Arg::new("list-tools").long("list-tools").action(ArgAction::SetTrue))
        .arg(Arg::new("apply").long("apply").action(ArgAction::SetTrue).conflicts_with("dry-run"))
        .arg(Arg::new("dry-run").long("dry-run").action(ArgAction::SetTrue))
        .arg(Arg::new("dir").long("dir").action(ArgAction::Append).help("Tool directory as TOOL=PATH; repeat for each tool"))
        .arg(Arg::new("overwrite").long("overwrite").alias("replace-configs").action(ArgAction::SetTrue))
        .arg(Arg::new("overwrite-rust-toolchain").long("overwrite-rust-toolchain").action(ArgAction::SetTrue))
        .arg(Arg::new("install-tools").long("install-tools").action(ArgAction::SetTrue))
        .arg(Arg::new("install-deps").long("install-deps").action(ArgAction::SetTrue))
        .arg(Arg::new("python-files").long("python-files").num_args(1..))
        .arg(Arg::new("python-version").long("python-version"))
        .arg(Arg::new("python-executable").long("python-executable"))
        .arg(Arg::new("python-mode").long("python-mode").value_parser(["auto", "project", "scripts"]).default_value("auto"))
        .arg(Arg::new("node-version").long("node-version"))
        .arg(Arg::new("agents-file").long("agents-file").default_value("AGENTS.md"));
    for id in catalog.tools.keys() {
        let name = format!("{id}-dir");
        cmd = cmd.arg(Arg::new(name.clone()).long(name).help(format!("Project-relative {id} directory")));
    }
    cmd
}
pub fn options(matches: &ArgMatches, catalog: &Catalog) -> Result<Options> {
    let get = |name: &str| matches.get_one::<String>(name).cloned();
    let mut dirs = BTreeMap::new();
    for id in catalog.tools.keys() {
        if let Some(dir) = get(&format!("{id}-dir")) { dirs.insert(id.clone(), dir); }
    }
    for entry in matches.get_many::<String>("dir").into_iter().flatten() {
        let (name, path) = entry.split_once('=').context("Expected --dir TOOL=PATH")?;
        let id = catalog.resolve(name).with_context(|| format!("Unknown tool: {name}"))?;
        ensure!(!path.is_empty(), "Missing directory for {name}");
        if let Some(previous) = dirs.insert(id.into(), path.into()) { ensure!(previous == path, "Conflicting directories for {id}"); }
    }
    let mut tools = Vec::new();
    for name in matches.get_many::<String>("tools").into_iter().flatten() {
        let id = catalog.resolve(name).context("Unknown tool")?.to_owned();
        if !tools.contains(&id) { tools.push(id); }
    }
    let project = get("project").context("Project is required")?;
    let project = if let Some(rest) = project.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").context("Cannot expand ~ without HOME")?).join(rest)
    } else { PathBuf::from(project) };
    Ok(Options { project, tools, dirs, apply: matches.get_flag("apply"), overwrite: matches.get_flag("overwrite"),
        overwrite_rust_toolchain: matches.get_flag("overwrite-rust-toolchain"), install_tools: matches.get_flag("install-tools"), install_deps: matches.get_flag("install-deps"),
        python_files: matches.get_many::<String>("python-files").map(|values| values.cloned().collect()), python_version: get("python-version"), python_executable: get("python-executable"),
        python_mode: get("python-mode").unwrap_or_default(), node_version: get("node-version"), agents_file: get("agents-file").unwrap_or_default() })
}
