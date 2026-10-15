use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use toml::Value;

fn templates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("templates")
}
fn load(relative: &str) -> Value {
    toml::from_str(&fs::read_to_string(templates().join(relative)).unwrap()).unwrap()
}
fn paths(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut result = vec![];
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            result.extend(paths(&path, extension));
        } else if path.extension().is_some_and(|ext| ext == extension) {
            result.push(path);
        }
    }
    result.sort();
    result
}
fn strings(value: &Value) -> BTreeSet<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect()
}
#[test]
fn test_toml_configs_parse() {
    let paths = paths(&templates(), "toml");
    assert!(!paths.is_empty());
    for path in paths {
        toml::from_str::<Value>(&fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    }
}
#[test]
fn test_json_configs_parse() {
    let paths = paths(&templates(), "json");
    assert!(!paths.is_empty());
    for path in paths {
        let data: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(data.is_object(), "{}", path.display());
    }
}
#[test]
fn test_profile_task_dependencies_resolve() {
    for path in paths(&templates().join("mise/profiles"), "toml") {
        let data: Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        if let Some(tasks) = data.get("tasks").and_then(Value::as_table) {
            for (name, task) in tasks {
                if let Some(dependencies) = task.get("depends") {
                    for dependency in strings(dependencies) {
                        assert!(
                            tasks.contains_key(dependency),
                            "{}: {name}: {dependency}",
                            path.display()
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn test_python_tasks_use_cli_entry_points() {
    let data = load("mise/profiles/python.toml");
    let tasks = &data["tasks"];
    for name in [
        "lint:python",
        "format:python",
        "fix:python",
        "typecheck:python",
    ] {
        let commands = match &tasks[name]["run"] {
            Value::String(run) => vec![run.as_str()],
            Value::Array(runs) => runs.iter().map(|run| run.as_str().unwrap()).collect(),
            _ => panic!("Invalid run: {name}"),
        };
        let executable = if name == "typecheck:python" {
            "ty"
        } else {
            "ruff"
        };
        for command in commands {
            assert_eq!(
                command.split_whitespace().next(),
                Some(executable),
                "{name}"
            );
        }
    }
    assert_eq!(
        tasks["test:python"]["run"].as_str(),
        Some("uv run pytest -q --tb=short")
    );
}
#[test]
fn test_python_targets_agree() {
    let profile = load("mise/profiles/python.toml");
    let version = profile["vars"]["python_version"].as_str().unwrap();
    assert_eq!(
        load("python/ruff.toml")["target-version"].as_str().unwrap(),
        format!("py{}", version.replace('.', ""))
    );
    assert_eq!(
        load("python/ty.toml")["environment"]["python-version"].as_str(),
        Some(version)
    );
}
#[test]
fn test_ruff_strict_selections() {
    let ruff = load("python/ruff.toml");
    let additions: BTreeSet<_> = [
        "T10", "BLE", "PLE", "PGH", "LOG", "ASYNC", "S113", "S501", "S506", "S608", "PERF101",
        "PERF102", "PERF402", "FURB", "ICN", "ANN",
    ]
    .into_iter()
    .collect();
    assert!(additions.is_subset(&strings(&ruff["lint"]["extend-select"])));
    assert_eq!(ruff["preview"].as_bool(), Some(false));
}
#[test]
fn test_ruff_formatter_conflicts_are_ignored() {
    let ruff = load("python/ruff.toml");
    assert!(
        ["COM812", "COM819", "W191"]
            .into_iter()
            .collect::<BTreeSet<_>>()
            .is_subset(&strings(&ruff["lint"]["ignore"]))
    );
}
#[test]
fn test_python_check_includes_types() {
    let profile = load("mise/profiles/python.toml");
    let tasks = &profile["tasks"];
    assert_eq!(
        strings(&tasks["check:python"]["depends"]),
        ["lint:python", "typecheck:python"].into_iter().collect()
    );
    assert!(
        tasks["typecheck:python"]["run"]
            .as_str()
            .unwrap()
            .starts_with("ty check ")
    );
}
