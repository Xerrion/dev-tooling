use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};
use tempfile::TempDir;
use xerrion_dev_tooling_installer::{
    build_plan,
    catalog::Catalog,
    cli,
    plan::{AGGREGATE_END, AGGREGATE_START, FRAGMENT, Plan, PlannedCommand},
};

pub type Files = BTreeMap<PathBuf, Vec<u8>>;
#[derive(Debug, PartialEq)]
pub enum Content {
    Directory,
    File(Vec<u8>),
    Link(PathBuf),
}
pub type Snapshot = BTreeMap<PathBuf, (SystemTime, u32, Content)>;
pub fn snapshot(root: &Path) -> Snapshot {
    fn visit(root: &Path, path: &Path, result: &mut Snapshot) {
        let metadata = fs::symlink_metadata(path).unwrap();
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode()
        };
        #[cfg(not(unix))]
        let mode = u32::from(metadata.permissions().readonly());
        let content = if metadata.file_type().is_symlink() {
            Content::Link(fs::read_link(path).unwrap())
        } else if metadata.is_dir() {
            Content::Directory
        } else {
            Content::File(fs::read(path).unwrap())
        };
        result.insert(
            path.strip_prefix(root).unwrap().into(),
            (metadata.modified().unwrap(), mode, content),
        );
        if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), result);
            }
        }
    }
    let mut result = Snapshot::new();
    visit(root, root, &mut result);
    result
}
pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .into()
}
pub fn templates() -> PathBuf {
    repo().join("templates")
}
pub fn write(root: &Path, relative: impl AsRef<Path>, text: impl AsRef<[u8]>) -> PathBuf {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
    path
}
pub fn files(root: &Path, entries: &[(&str, &str)]) -> Files {
    entries
        .iter()
        .map(|(name, text)| {
            write(root, name, text);
            (PathBuf::from(name), text.as_bytes().to_vec())
        })
        .collect()
}
pub fn read(root: &Path, relative: impl AsRef<Path>) -> String {
    fs::read_to_string(root.join(relative)).unwrap()
}
pub fn toml_file(path: impl AsRef<Path>) -> Value {
    toml::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
pub fn generated(project: &Path) -> Value {
    let data = toml_file(project.join(FRAGMENT));
    assert!(data.get("include").is_none());
    data
}
pub fn backups(project: &Path, relative: impl AsRef<Path>) -> Vec<PathBuf> {
    let directory = project.join(".tooling-backups");
    if !directory.exists() {
        return vec![];
    }
    fs::read_dir(directory)
        .unwrap()
        .map(|e| e.unwrap().path().join(relative.as_ref()))
        .filter(|p| p.is_file())
        .collect()
}
pub fn assert_backup(project: &Path, relative: impl AsRef<Path>, original: impl AsRef<[u8]>) {
    let paths = backups(project, &relative);
    assert_eq!(paths.len(), 1, "{}: {paths:?}", relative.as_ref().display());
    assert_eq!(fs::read(&paths[0]).unwrap(), original.as_ref());
}
fn managed_parts(text: &str) -> (&str, &str, &str) {
    assert_eq!(text.matches(AGGREGATE_START).count(), 1);
    assert_eq!(text.matches(AGGREGATE_END).count(), 1);
    let (prefix, rest) = text.split_once(AGGREGATE_START).unwrap();
    let (block, suffix) = rest.split_once(AGGREGATE_END).unwrap();
    (prefix, block, suffix)
}
pub fn assert_root_preserved(path: &Path, original: &[u8]) {
    let current = fs::read(path).unwrap();
    toml_file(path);
    if current == original {
        return;
    }
    let current = std::str::from_utf8(&current).unwrap();
    let original = std::str::from_utf8(original).unwrap();
    let (prefix, _, suffix) = managed_parts(current);
    if original.contains(AGGREGATE_START) {
        let (old_prefix, _, old_suffix) = managed_parts(original);
        assert_eq!(prefix, old_prefix);
        assert_eq!(suffix, old_suffix);
    } else {
        assert!(prefix.starts_with(original));
        assert!(prefix[original.len()..].trim().is_empty());
        assert!(suffix.trim().is_empty());
    }
}
pub fn assert_preserved(project: &Path, originals: &Files) {
    for (relative, original) in originals {
        let path = project.join(relative);
        if ["mise.toml", ".mise.toml"]
            .iter()
            .any(|name| relative == Path::new(name))
        {
            assert_root_preserved(&path, original);
        } else {
            assert_eq!(fs::read(&path).unwrap(), *original, "{}", path.display());
        }
    }
}
pub fn root_mise(project: &Path) -> PathBuf {
    let dot = project.join(".mise.toml");
    if dot.is_file() {
        dot
    } else {
        project.join("mise.toml")
    }
}
pub fn central_tasks(project: &Path) -> Value {
    let text = fs::read_to_string(root_mise(project)).unwrap();
    let _: toml::Value = toml::from_str(&text).unwrap();
    let (_, block, _) = managed_parts(&text);
    let data: Value = toml::from_str(block).unwrap();
    data.get("tasks").cloned().unwrap_or(json!({}))
}
pub fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().into())
        .collect()
}
pub fn assert_central_dependencies(project: &Path, selected: &[&str], preserved: &[&str]) {
    let template = toml_file(templates().join("mise/mise.toml"));
    let managed = central_tasks(project);
    let mut sources = vec![root_mise(project)];
    sources.extend(
        fs::read_dir(project.join(".mise/conf.d"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|s| s == "toml")),
    );
    let mut available = BTreeSet::new();
    for source in sources {
        if let Some(tasks) = toml_file(source)["tasks"].as_object() {
            available.extend(tasks.keys().cloned());
        }
    }
    let mut expected = BTreeMap::new();
    for name in ["check", "format", "test", "deps"] {
        if preserved.contains(&name) {
            continue;
        }
        let deps: BTreeSet<_> = strings(&template["tasks"][name]["depends"])
            .into_iter()
            .filter(|d| available.contains(d) && selected.contains(&d.rsplit(':').next().unwrap()))
            .collect();
        if !deps.is_empty() {
            expected.insert(name.to_owned(), deps);
        }
    }
    assert_eq!(
        managed
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected.keys().cloned().collect()
    );
    for (name, expected) in expected {
        let deps = strings(&managed[&name]["depends"]);
        let unique: BTreeSet<_> = deps.iter().cloned().collect();
        assert_eq!(deps.len(), unique.len());
        assert_eq!(unique, expected, "{name}");
        assert!(managed[&name].get("run").is_none());
    }
}
pub fn task_commands(task: &Value) -> Vec<Vec<String>> {
    match &task["run"] {
        Value::String(run) => vec![shlex::split(run).unwrap()],
        Value::Array(runs) => runs
            .iter()
            .map(|run| shlex::split(run.as_str().unwrap()).unwrap())
            .collect(),
        _ => vec![],
    }
}
pub fn resolved(argv: &[String]) -> &[String] {
    if argv.first().is_some_and(|s| s == "mise") && argv.get(1).is_some_and(|s| s == "exec") {
        &argv[argv.iter().position(|s| s == "--").unwrap() + 1..]
    } else {
        argv
    }
}
pub fn python_targets(data: &Value) -> Vec<String> {
    shlex::split(data["vars"]["python_files"].as_str().unwrap()).unwrap()
}
pub fn python_project(project: &Path, embedded: bool) -> Files {
    let mut manifest = "[project]\nname = 'sample'\nversion = '0.1.0'\nrequires-python = '>=3.12'\n[dependency-groups]\ndev = ['pytest>=8']\n".to_owned();
    if embedded {
        manifest.push_str("\n[tool.ruff]\nline-length = 88\ntarget-version = 'py312'\n[tool.ty.environment]\npython-version = '3.12'\n");
    }
    files(
        project,
        &[
            ("pyproject.toml", &manifest),
            ("uv.lock", "version = 1\nrevision = 1\n"),
            (
                "src/sample/__init__.py",
                "def answer() -> int:\n    return 42\n",
            ),
            (
                "tests/test_sample.py",
                "def test_answer() -> None:\n    assert 42 == 42\n",
            ),
        ],
    )
}
pub fn monorepo(project: &Path) -> Files {
    files(
        project,
        &[
            (
                "mise.toml",
                "# Project pins\n[tools]\nnode = '24.15.0'\npython = '3.12.9'\nbun = '1.2.20'\n[tasks.typecheck]\nrun = 'echo existing typecheck'\n[tasks.test]\nrun = 'echo existing test'\n",
            ),
            ("web/package.json", "{}\n"),
            ("web/bun.lock", "{}\n"),
            ("web/.gitignore", "node_modules/\n"),
            (
                "engine/Cargo.toml",
                "[package]\nname = 'engine'\nversion = '0.1.0'\nedition = '2021'\n",
            ),
            ("engine/Cargo.lock", "version = 4\n"),
            (
                "engine/rust-toolchain.toml",
                "[toolchain]\nchannel = '1.85.1'\ncomponents = ['rustfmt', 'clippy']\n",
            ),
            ("engine/src/lib.rs", "pub fn answer() -> u32 { 42 }\n"),
            ("docs/README.md", "# Documentation\n"),
            ("docs/.gitignore", "node_modules/\n"),
            ("scripts/report.py", "print('local script')\n"),
            ("lua/init.lua", "return {}\n"),
        ],
    )
}
pub fn workspace_project(project: &Path, manager: &str) -> (Files, &'static str) {
    let (version, lockfile, lock) = match manager {
        "npm" => (
            "10.9.2",
            "package-lock.json",
            "{\"name\":\"workspace-fixture\",\"lockfileVersion\":3,\"packages\":{}}\n",
        ),
        "pnpm" => (
            "10.5.2",
            "pnpm-lock.yaml",
            "lockfileVersion: '9.0'\nimporters:\n  .: {}\n  web: {}\n  docs: {}\n",
        ),
        "yarn" => ("4.9.1", "yarn.lock", "__metadata:\n  version: 8\n"),
        _ => panic!("Unknown manager"),
    };
    let manifest = json!({"name": "workspace-fixture", "private": true, "packageManager": format!("{manager}@{version}"), "workspaces": ["web", "docs"]}).to_string();
    let mut originals = files(
        project,
        &[
            ("mise.toml", "[tools]\nnode = '24.15.0'\n"),
            ("package.json", &manifest),
            (lockfile, lock),
            (
                "web/package.json",
                r#"{"name":"fixture-web","private":true,"scripts":{"typecheck":"tsc --noEmit","test":"node --test"}}"#,
            ),
            (
                "docs/package.json",
                r#"{"name":"fixture-docs","private":true}"#,
            ),
            ("web/.gitignore", "node_modules/\n"),
            ("docs/.gitignore", "node_modules/\n"),
            ("docs/README.md", "# Workspace documentation\n"),
        ],
    );
    if manager == "pnpm" {
        originals.extend(files(
            project,
            &[("pnpm-workspace.yaml", "packages:\n  - web\n  - docs\n")],
        ));
    }
    (originals, lockfile)
}
pub fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let to = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), to).unwrap();
        }
    }
}
pub struct Fixture {
    _temporary: TempDir,
    pub root: PathBuf,
    pub project: PathBuf,
    pub outside: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let project = root.join("project");
        let outside = root.join("outside");
        fs::create_dir(&project).unwrap();
        fs::create_dir(&outside).unwrap();
        Self {
            _temporary: temporary,
            root,
            project,
            outside,
        }
    }
    pub fn write(&self, relative: impl AsRef<Path>, text: impl AsRef<[u8]>) -> PathBuf {
        write(&self.project, relative, text)
    }
    pub fn invoke_at(&self, project: &Path, args: &[&str]) -> (i32, String) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_installer"));
        command.arg(project);
        if !args.contains(&"--templates") {
            command.arg("--templates").arg(templates());
        }
        let result = command.args(args).env("PATH", "").output().unwrap();
        (
            result.status.code().unwrap_or(-1),
            format!(
                "{}{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            ),
        )
    }
    pub fn invoke(&self, args: &[&str]) -> (i32, String) {
        self.invoke_at(&self.project, args)
    }
    pub fn succeed(&self, args: &[&str]) -> String {
        let (code, output) = self.invoke(args);
        assert_eq!(code, 0, "{output}");
        output
    }
    pub fn refuse(&self, args: &[&str]) -> String {
        let before = snapshot(&self.root);
        let (code, output) = self.invoke(args);
        assert_eq!(code, 1, "{output}");
        assert!(!output.trim().is_empty());
        assert_eq!(snapshot(&self.root), before);
        output
    }
    pub fn plan(&self, args: &[&str]) -> Plan {
        let catalog = Catalog::load(&templates()).unwrap();
        let args = [
            vec!["installer", self.project.to_str().unwrap()],
            args.to_vec(),
        ]
        .concat();
        let matches = cli::command(&catalog).try_get_matches_from(args).unwrap();
        build_plan(&cli::options(&matches, &catalog).unwrap(), &catalog).unwrap()
    }
    pub fn apply_recorded(
        &self,
        args: &[&str],
        run: impl FnMut(&PlannedCommand) -> anyhow::Result<()>,
    ) -> Plan {
        assert!(args.contains(&"--apply"));
        let plan = self.plan(args);
        plan.apply_with(run).unwrap();
        plan
    }
}
