import json
import tomllib
import unittest
from pathlib import Path

TEMPLATES = Path(__file__).resolve().parents[1] / "templates"
PROFILES = TEMPLATES / "mise" / "profiles"


def load_toml(path: Path) -> dict:
    with path.open("rb") as file:
        return tomllib.load(file)


class TemplateTests(unittest.TestCase):
    def test_toml_configs_parse(self) -> None:
        paths = sorted(TEMPLATES.rglob("*.toml"))
        assert paths
        for path in paths:
            with self.subTest(path=path.relative_to(TEMPLATES)):
                load_toml(path)

    def test_json_configs_parse(self) -> None:
        paths = sorted(TEMPLATES.rglob("*.json"))
        assert paths
        for path in paths:
            with (
                self.subTest(path=path.relative_to(TEMPLATES)),
                path.open(encoding="utf-8") as file,
            ):
                assert isinstance(json.load(file), dict)

    def test_profile_task_dependencies_resolve(self) -> None:
        for path in sorted(PROFILES.glob("*.toml")):
            with self.subTest(profile=path.name):
                tasks = load_toml(path).get("tasks", {})

                for name, task in tasks.items():
                    for dependency in task.get("depends", []):
                        assert dependency in tasks, f"{name}: {dependency}"

    def test_python_tasks_use_cli_entry_points(self) -> None:
        tasks = load_toml(PROFILES / "python.toml")["tasks"]
        for name in ("lint:python", "format:python", "fix:python", "typecheck:python"):
            with self.subTest(task=name):
                commands = tasks[name]["run"]
                if isinstance(commands, str):
                    commands = [commands]
                executable = "ty" if name == "typecheck:python" else "ruff"
                for command in commands:
                    assert command.split()[0] == executable
        assert tasks["test:python"]["run"] == "uv run pytest -q --tb=short"

    def test_python_targets_agree(self) -> None:
        version = load_toml(PROFILES / "python.toml")["vars"]["python_version"]
        ruff = load_toml(TEMPLATES / "python" / "ruff.toml")
        ty = load_toml(TEMPLATES / "python" / "ty.toml")

        assert ruff["target-version"] == "py" + version.replace(".", "")
        assert ty["environment"]["python-version"] == version

    def test_ruff_strict_selections(self) -> None:
        ruff = load_toml(TEMPLATES / "python" / "ruff.toml")
        additions = {
            "T10",
            "BLE",
            "PLE",
            "PGH",
            "LOG",
            "ASYNC",
            "S113",
            "S501",
            "S506",
            "S608",
            "PERF101",
            "PERF102",
            "PERF402",
            "FURB",
            "ICN",
            "ANN",
        }
        assert additions <= set(ruff["lint"]["extend-select"])
        assert not ruff["preview"]

    def test_ruff_formatter_conflicts_are_ignored(self) -> None:
        ruff = load_toml(TEMPLATES / "python" / "ruff.toml")
        assert {"COM812", "COM819", "W191"} <= set(ruff["lint"]["ignore"])

    def test_python_check_includes_types(self) -> None:
        tasks = load_toml(PROFILES / "python.toml")["tasks"]
        assert set(tasks["check:python"]["depends"]) == {"lint:python", "typecheck:python"}
        assert tasks["typecheck:python"]["run"].startswith("ty check ")


if __name__ == "__main__":
    unittest.main()
