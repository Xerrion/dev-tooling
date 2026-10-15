import importlib.util
import io
import json
import re
import shlex
import subprocess
import sys
import tempfile
import tomllib
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch

import pytest

REPO_ROOT = Path(__file__).resolve().parents[1]
FRAGMENT = Path(".mise/conf.d/tooling.toml")
START = "<!-- dev-tooling:start -->"
END = "<!-- dev-tooling:end -->"
AGGREGATES_START = "# dev-tooling:aggregates:start"
AGGREGATES_END = "# dev-tooling:aggregates:end"
CENTRAL_TEMPLATE = REPO_ROOT / "templates/mise/mise.toml"
CENTRAL_TASKS = {"check", "format", "test", "deps"}


class InstallTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        spec = importlib.util.spec_from_file_location("install", REPO_ROOT / "install.py")
        if spec is None or spec.loader is None:
            raise RuntimeError("Cannot load the repository's install.py")
        cls.install = importlib.util.module_from_spec(spec)
        previous = sys.modules.get("install")
        sys.modules["install"] = cls.install

        def restore_module() -> None:
            if previous is None:
                sys.modules.pop("install", None)
            else:
                sys.modules["install"] = previous

        cls.addClassCleanup(restore_module)
        spec.loader.exec_module(cls.install)

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix=".test-install-", dir=REPO_ROOT)
        self.addCleanup(temporary.cleanup)
        self.sandbox = Path(temporary.name)
        self.project = self.sandbox / "project"
        self.outside = self.sandbox / "outside"
        self.project.mkdir()
        self.outside.mkdir()
        runner = patch.object(
            self.install.subprocess,
            "run",
            side_effect=AssertionError("Installer tests must not execute external commands"),
        )
        self.run_mock = runner.start()
        self.addCleanup(runner.stop)

    def write(self, project: Path, relative: str | Path, text: str) -> Path:
        path = project / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        return path

    def snapshot(self) -> dict:
        # lstat and non-following traversal keep even adversarial symlink fixtures local.
        result = {}
        for path in [self.sandbox, *sorted(self.sandbox.rglob("*"))]:
            relative = path.relative_to(self.sandbox).as_posix()
            metadata = path.lstat()
            if path.is_symlink():
                content = ("symlink", Path.readlink(path))
            elif path.is_dir():
                content = ("directory",)
            else:
                content = ("file", path.read_bytes())
            result[relative] = (metadata.st_mode, metadata.st_mtime_ns, content)
        return result

    def invoke(self, *arguments: str, project: Path | None = None) -> tuple[int, str]:
        output = io.StringIO()
        with redirect_stdout(output), redirect_stderr(output):
            status = self.install.main([str(project or self.project), *arguments])
        assert type(status) is int
        return status, output.getvalue()

    def succeed(self, *arguments: str, project: Path | None = None) -> str:
        status, output = self.invoke(*arguments, project=project)
        assert status == 0, output
        return output

    def refuse_without_writes(self, *arguments: str, project: Path | None = None) -> str:
        before = self.snapshot()
        status, output = self.invoke(*arguments, project=project)
        assert status == 1, output
        assert output.strip(), "A rejected installation must explain the error"
        assert self.snapshot() == before
        self.run_mock.assert_not_called()
        return output

    def generated(self, project: Path | None = None) -> dict:
        path = (project or self.project) / FRAGMENT
        assert path.is_file(), f"Missing generated mise fragment: {path}"
        with path.open("rb") as file:
            data = tomllib.load(file)
        assert "include" not in data, "Profiles must be merged, not included"
        return data

    def backups(self, project: Path, relative: str | Path) -> list[Path]:
        directory = project / ".tooling-backups"
        return sorted(directory.glob("*/" + Path(relative).as_posix()))

    def assert_backup(self, project: Path, relative: str | Path, original: bytes) -> None:
        paths = self.backups(project, relative)
        assert len(paths) == 1, f"Expected one backup for {relative}: {paths}"
        assert paths[0].read_bytes() == original
        assert paths[0].relative_to(project / ".tooling-backups").parts[1:] == Path(relative).parts

    def python_project(self, project: Path | None = None, *, embedded: bool = True) -> dict:
        project = project or self.project
        manifest = (
            '[project]\nname = "sample"\nversion = "0.1.0"\nrequires-python = ">=3.12"\n'
            '\n[dependency-groups]\ndev = ["pytest>=8"]\n'
        )
        if embedded:
            manifest += (
                '\n[tool.ruff]\nline-length = 88\ntarget-version = "py312"\n'
                '\n[tool.ty.environment]\npython-version = "3.12"\n'
            )
        files = {
            "pyproject.toml": manifest,
            "uv.lock": "version = 1\nrevision = 1\n",
            "src/sample/__init__.py": "def answer() -> int:\n    return 42\n",
            "tests/test_sample.py": "def test_answer() -> None:\n    assert 42 == 42\n",
        }
        for relative, text in files.items():
            self.write(project, relative, text)
        return {relative: text.encode() for relative, text in files.items()}

    def monorepo(self) -> dict:
        files = {
            "mise.toml": (
                "# Keep the project-owned pins and tasks.\n[tools]\n"
                'node = "24.15.0"\npython = "3.12.9"\nbun = "1.2.20"\n'
                '\n[tasks.typecheck]\nrun = "echo existing typecheck"\n'
                '\n[tasks.test]\nrun = "echo existing test"\n'
            ),
            "web/package.json": "{}\n",
            "web/bun.lock": "{}\n",
            "web/.gitignore": "node_modules/\n",
            "engine/Cargo.toml": ('[package]\nname = "engine"\nversion = "0.1.0"\nedition = "2021"\n'),
            "engine/Cargo.lock": "version = 4\n",
            "engine/rust-toolchain.toml": ('[toolchain]\nchannel = "1.85.1"\ncomponents = ["rustfmt", "clippy"]\n'),
            "engine/src/lib.rs": "pub fn answer() -> u32 { 42 }\n",
            "docs/README.md": "# Documentation\n",
            "docs/.gitignore": "node_modules/\n",
            "scripts/report.py": 'print("local script")\n',
            "lua/init.lua": "return {}\n",
        }
        for relative, text in files.items():
            self.write(self.project, relative, text)
        return {relative: text.encode() for relative, text in files.items()}

    def workspace_project(self, project: Path, manager: str) -> tuple[dict, str]:
        versions = {"npm": "10.9.2", "pnpm": "10.5.2", "yarn": "4.9.1"}
        locks = {
            "npm": ("package-lock.json", '{"name":"workspace-fixture","lockfileVersion":3,"packages":{}}\n'),
            "pnpm": ("pnpm-lock.yaml", "lockfileVersion: '9.0'\nimporters:\n  .: {}\n  web: {}\n  docs: {}\n"),
            "yarn": ("yarn.lock", "__metadata:\n  version: 8\n"),
        }
        lockfile, lock = locks[manager]
        manifest = {
            "name": "workspace-fixture",
            "private": True,
            "packageManager": f"{manager}@{versions[manager]}",
            "workspaces": ["web", "docs"],
        }
        files = {
            "mise.toml": '[tools]\nnode = "24.15.0"\n',
            "package.json": json.dumps(manifest) + "\n",
            lockfile: lock,
            "web/package.json": json.dumps(
                {
                    "name": "fixture-web",
                    "private": True,
                    "scripts": {"typecheck": "tsc --noEmit", "test": "node --test"},
                }
            )
            + "\n",
            "docs/package.json": '{"name":"fixture-docs","private":true}\n',
            "web/.gitignore": "node_modules/\n",
            "docs/.gitignore": "node_modules/\n",
            "docs/README.md": "# Workspace documentation\n",
        }
        if manager == "pnpm":
            files["pnpm-workspace.yaml"] = "packages:\n  - web\n  - docs\n"
        for relative, text in files.items():
            self.write(project, relative, text)
        return {relative: text.encode() for relative, text in files.items()}, lockfile

    def run_workspace_dependency_commands(
        self, project: Path, manager: str, originals: dict, lockfile: str
    ) -> list[list[str]]:
        protected = ("package.json", lockfile, "web/package.json", "docs/package.json")
        observed = []

        def inspect_command(arguments: list[str], **kwargs: object) -> subprocess.CompletedProcess:
            parsed = self.resolved_command(arguments)
            if Path(parsed[0]).name == "mise" and parsed[1:2] == ["trust"]:
                return subprocess.CompletedProcess(arguments, 0, stdout="", stderr="")
            commands = [parsed]
            if Path(parsed[0]).name == "mise" and parsed[1:2] == ["run"]:
                tasks = self.generated(project)["tasks"]
                commands = [
                    self.resolved_command(command)
                    for name in parsed[2:]
                    if name in tasks
                    for command in self.task_commands(tasks[name])
                ]
            assert commands, f"Unrecognized dependency operation: {arguments}"
            for command in commands:
                assert Path(command[0]).name == manager, command
                assert any(word in command[1:] for word in ("ci", "install", "add")), command
            for relative in protected:
                self.assert_backup(project, relative, originals[relative])
            directory = kwargs.get("cwd", project)
            assert isinstance(directory, (str, Path))
            assert Path(directory).resolve().is_relative_to(project.resolve()), directory
            observed.extend(commands)
            path = project / lockfile
            if manager == "npm":
                contents = json.loads(path.read_text(encoding="utf-8"))
                contents["fixtureRevision"] = len(observed)
                path.write_text(json.dumps(contents) + "\n", encoding="utf-8")
            else:
                path.write_bytes(path.read_bytes() + f"\n# Mock dependency operation {len(observed)}\n".encode())
            return subprocess.CompletedProcess(arguments, 0, stdout="", stderr="")

        self.run_mock.reset_mock()
        self.run_mock.side_effect = inspect_command
        self.succeed(
            "--tools",
            "javascript",
            "markdown",
            "--javascript-dir",
            "web",
            "--markdown-dir",
            "docs",
            "--install-deps",
            "--apply",
            project=project,
        )
        assert len(observed) >= 2, "Expected dependency operations for both selected packages"
        return observed

    def managed_root_parts(self, contents: bytes) -> tuple[bytes, bytes, bytes]:
        start = AGGREGATES_START.encode()
        end = AGGREGATES_END.encode()
        assert contents.count(start) == 1, "Expected exactly one aggregate start marker"
        assert contents.count(end) == 1, "Expected exactly one aggregate end marker"
        prefix, remainder = contents.split(start, 1)
        assert end in remainder, "Aggregate markers are reversed"
        block, suffix = remainder.split(end, 1)
        return prefix, block, suffix

    def root_mise(self, project: Path) -> Path:
        for name in ("mise.toml", ".mise.toml"):
            path = project / name
            if path.is_file():
                return path
        raise AssertionError("Missing root mise configuration")

    def central_tasks(self, project: Path | None = None) -> dict:
        contents = self.root_mise(project or self.project).read_bytes()
        tomllib.loads(contents.decode())
        _, block, _ = self.managed_root_parts(contents)
        return tomllib.loads(block.decode()).get("tasks", {})

    def assert_root_preserved(self, path: Path, original: bytes) -> None:
        current = path.read_bytes()
        tomllib.loads(current.decode())
        if current == original:
            return
        prefix, _, suffix = self.managed_root_parts(current)
        if AGGREGATES_START.encode() in original:
            old_prefix, _, old_suffix = self.managed_root_parts(original)
            assert prefix == old_prefix
            assert suffix == old_suffix
        else:
            assert prefix.startswith(original), "Original root mise content must remain an exact prefix"
            assert not prefix[len(original) :].strip(), "Only padding may precede the appended managed block"
            assert not suffix.strip(), "Unexpected content outside the appended managed block"

    def assert_preserved(self, project: Path, files: dict) -> None:
        for relative, original in files.items():
            with self.subTest(preserved=relative):
                path = project / relative
                if Path(relative).as_posix() in ("mise.toml", ".mise.toml"):
                    self.assert_root_preserved(path, original)
                else:
                    assert path.read_bytes() == original

    def assert_central_dependencies(
        self, project: Path, selected: set[str], preserved_aggregates: set[str] | None = None
    ) -> None:
        with CENTRAL_TEMPLATE.open("rb") as file:
            template = tomllib.load(file)["tasks"]
        assert template.keys() >= CENTRAL_TASKS
        managed = self.central_tasks(project)
        sources = [*sorted((project / ".mise/conf.d").glob("*.toml")), self.root_mise(project)]
        available = set()
        for source in sources:
            with source.open("rb") as file:
                available.update(tomllib.load(file).get("tasks", {}))
        expected = {}
        for name in sorted(CENTRAL_TASKS - (preserved_aggregates or set())):
            dependencies = {
                dependency
                for dependency in template[name].get("depends", [])
                if dependency in available and dependency.rsplit(":", 1)[-1] in selected
            }
            if dependencies:
                expected[name] = dependencies
        assert set(managed) == set(expected), f"Expected aggregates {sorted(expected)}, got {sorted(managed)}"
        for name, task in managed.items():
            actual = task.get("depends", [])
            assert set(actual) == expected[name], f"{name}: expected {sorted(expected[name])}, got {actual}"
            assert len(actual) == len(set(actual)), f"Duplicate dependencies in {name}"
            assert "run" not in task, f"{name} should delegate through dependencies"

    def python_targets(self, generated: dict) -> list[str]:
        targets = generated["vars"]["python_files"]
        return shlex.split(targets) if isinstance(targets, str) else targets

    def recorded_commands(self) -> list[list[str]]:
        commands = []
        for call in self.run_mock.call_args_list:
            command = call.args[0] if call.args else call.kwargs["args"]
            commands.append(shlex.split(command) if isinstance(command, str) else [str(part) for part in command])
        return commands

    def resolved_command(self, command: list[str]) -> list[str]:
        if len(command) > 3 and Path(command[0]).name == "mise" and command[1] == "exec" and "--" in command:
            return command[command.index("--") + 1 :]
        return command

    def task_commands(self, task: dict) -> list[list[str]]:
        commands = task.get("run", [])
        if isinstance(commands, str):
            commands = [commands]
        return [shlex.split(command) for command in commands]

    def test_default_preview_does_not_write(self) -> None:
        self.python_project()
        before = self.snapshot()
        output = self.succeed("--tools", "python")
        assert output.strip()
        assert "tooling.toml" in output
        assert self.snapshot() == before
        self.run_mock.assert_not_called()

    def test_explicit_dry_run_does_not_write(self) -> None:
        self.python_project()
        before = self.snapshot()
        self.succeed("--tools", "python", "--dry-run")
        assert self.snapshot() == before
        self.run_mock.assert_not_called()

    def test_python_project_preserves_embedded_configs_and_manifests(self) -> None:
        originals = self.python_project()
        self.succeed("--tools", "python", "--apply")
        data = self.generated()
        assert self.python_targets(data) == ["src", "tests"]
        assert data["vars"]["python_dir"] == "."
        assert "deps:python" in data["tasks"]
        assert "pytest" in str(data["tasks"]["test:python"]["run"])
        assert "check:python" in data["tasks"]
        for relative in ("ruff.toml", ".ruff.toml", "ty.toml"):
            assert not (self.project / relative).exists(), relative
        self.assert_preserved(self.project, originals)
        self.assert_central_dependencies(self.project, {"python"})
        self.run_mock.assert_not_called()

    def test_apply_preserves_agent_surroundings_and_is_idempotent(self) -> None:
        prefix = "# Project guidance\n\nKeep these local instructions exactly.\n\n"
        suffix = "\n\n## Local workflow\nDo not change this trailing text.\n"
        original = prefix + START + "\nobsolete generated guidance\n" + END + suffix
        agent = self.write(self.project, "AGENTS.md", original)
        self.succeed("--tools", "lua", "--apply")
        text = agent.read_text(encoding="utf-8")
        assert text.startswith(prefix)
        assert text.endswith(suffix)
        assert text.count(START) == 1
        assert text.count(END) == 1
        assert "obsolete generated guidance" not in text
        self.assert_backup(self.project, "AGENTS.md", original.encode())
        before = self.snapshot()
        self.succeed("--tools", "lua", "--apply")
        assert self.snapshot() == before

    def test_agent_file_default_selection_and_preservation(self) -> None:
        cases = (
            ((), "AGENTS.md"),
            (("AGENS.md",), "AGENTS.md"),
            (("AGENTS.md",), "AGENTS.md"),
            (("AGENS.md", "AGENTS.md"), "AGENTS.md"),
        )
        for index, (existing, selected) in enumerate(cases):
            with self.subTest(existing=existing):
                project = self.sandbox / f"agents-{index}"
                project.mkdir()
                for name in existing:
                    self.write(project, name, f"# Original {name}\n")
                self.succeed("--tools", "lua", "--apply", project=project)
                text = (project / selected).read_text(encoding="utf-8")
                assert text.count(START) == 1
                assert text.count(END) == 1
                if selected in existing:
                    assert text.startswith(f"# Original {selected}\n")
                for name in {"AGENS.md", "AGENTS.md"} - {selected}:
                    path = project / name
                    if name in existing:
                        assert path.read_text(encoding="utf-8") == f"# Original {name}\n"
                    else:
                        assert not path.exists()

    def test_explicit_agent_file(self) -> None:
        for index, relative in enumerate(("notes/TEAM.md", "AGENS.md")):
            with self.subTest(agents_file=relative):
                project = self.sandbox / f"explicit-agent-{index}"
                project.mkdir()
                agent = self.write(project, relative, "# Team-specific instructions\n")
                self.succeed("--tools", "lua", "--agents-file", relative, "--apply", project=project)
                text = agent.read_text(encoding="utf-8")
                assert text.startswith("# Team-specific instructions\n")
                assert text.count(START) == 1
                assert text.count(END) == 1
                if relative != "AGENS.md":
                    assert not (project / "AGENS.md").exists()
                assert not (project / "AGENTS.md").exists()

    def test_malformed_or_duplicate_markers_fail_before_any_writes(self) -> None:
        invalid = (
            START,
            END,
            END + "\n" + START,
            START + "\n" + START + "\n" + END,
            START + "\n" + END + "\n" + END,
            START + "\n" + END + "\n" + START + "\n" + END,
            START + "\n" + START + "\n" + END + "\n" + END,
        )
        for index, text in enumerate(invalid):
            with self.subTest(markers=text):
                project = self.sandbox / f"markers-{index}"
                project.mkdir()
                self.write(project, "AGENTS.md", "# Local instructions\n" + text + "\n")
                self.write(project, "stylua.toml", 'syntax = "Lua54"\n')
                self.refuse_without_writes("--tools", "lua", "--replace-configs", "--apply", project=project)

    def test_replace_configs_backs_up_changed_files_at_relative_paths(self) -> None:
        self.python_project(self.project / "backend", embedded=False)
        originals = {
            "backend/ruff.toml": b"line-length = 79\n",
            "backend/ty.toml": b'[environment]\npython-version = "3.10"\n',
        }
        for relative, original in originals.items():
            self.write(self.project, relative, original.decode())
        arguments = (
            "--tools",
            "python",
            "--python-dir",
            "backend",
            "--python-version",
            "3.13",
            "--replace-configs",
            "--apply",
        )
        self.succeed(*arguments)
        for relative, original in originals.items():
            with self.subTest(config=relative):
                path = self.project / relative
                assert path.read_bytes() != original
                with path.open("rb") as file:
                    tomllib.load(file)
                self.assert_backup(self.project, relative, original)
        before = self.snapshot()
        self.succeed(*arguments)
        assert self.snapshot() == before

    def test_replace_configs_allows_standalone_replacement_of_embedded_settings(
        self,
    ) -> None:
        originals = self.python_project()
        self.succeed("--tools", "python", "--replace-configs", "--apply")
        assert (self.project / "ruff.toml").is_file()
        assert (self.project / "ty.toml").is_file()
        self.assert_preserved(self.project, originals)

    def test_changed_generated_fragment_is_backed_up(self) -> None:
        self.succeed("--tools", "lua", "--apply")
        path = self.project / FRAGMENT
        changed = path.read_text(encoding="utf-8") + "\n# Locally edited generated file\n"
        path.write_text(changed, encoding="utf-8")
        self.succeed("--tools", "lua", "--apply")
        assert "Locally edited generated file" not in path.read_text(encoding="utf-8")
        self.assert_backup(self.project, FRAGMENT, changed.encode())

    def test_existing_standalone_and_alternate_configs_are_not_shadowed(self) -> None:
        cases = (
            ("python", "ruff.toml", "ruff.toml", "line-length = 79\n"),
            ("python", ".ruff.toml", "ruff.toml", "line-length = 79\n"),
            (
                "python",
                "ty.toml",
                "ty.toml",
                '[environment]\npython-version = "3.12"\n',
            ),
            (
                "javascript",
                "biome.json",
                "biome.json",
                '{"formatter":{"lineWidth":100}}\n',
            ),
            (
                "javascript",
                "biome.jsonc",
                "biome.json",
                '{\n// local config\n"root":true\n}\n',
            ),
            ("lua", "stylua.toml", "stylua.toml", 'syntax = "Lua54"\n'),
            ("lua", ".stylua.toml", "stylua.toml", 'syntax = "Lua54"\n'),
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
            ("markdown", ".markdownlint.yaml", ".markdownlint.json", "MD013: false\n"),
            ("markdown", ".prettierrc", ".prettierrc.json", '{"proseWrap":"always"}\n'),
            (
                "markdown",
                "prettier.config.js",
                ".prettierrc.json",
                "export default {};\n",
            ),
        )
        for index, (tool, existing, canonical, text) in enumerate(cases):
            with self.subTest(tool=tool, config=existing):
                project = self.sandbox / f"config-{index}"
                project.mkdir()
                if tool == "python":
                    self.python_project(project, embedded=False)
                elif tool == "javascript":
                    self.write(project, "package.json", "{}\n")
                elif tool == "rust":
                    self.write(
                        project,
                        "Cargo.toml",
                        '[package]\nname = "sample"\nversion = "0.1.0"\nedition = "2021"\n',
                    )
                    self.write(project, "src/lib.rs", "pub fn answer() -> u32 { 42 }\n")
                path = self.write(project, existing, text)
                self.succeed("--tools", tool, "--apply", project=project)
                assert path.read_bytes() == text.encode()
                assert self.backups(project, existing) == []
                if existing != canonical:
                    assert not (project / canonical).exists(), "Existing config was shadowed"

    def test_monorepo_merges_profiles_preserves_pins_and_delegates_root_tasks(
        self,
    ) -> None:
        originals = self.monorepo()
        arguments = (
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
        )
        self.succeed(*arguments)
        data = self.generated()
        for language, directory in (
            ("python", "scripts"),
            ("javascript", "web"),
            ("rust", "engine"),
            ("lua", "lua"),
            ("markdown", "docs"),
        ):
            with self.subTest(language=language):
                assert data["vars"][language + "_dir"] == directory
                assert "check:" + language in data["tasks"]
                assert "format:" + language in data["tasks"]
        for name in ("typecheck", "test"):
            task = data["tasks"][name + ":javascript"]
            assert task["depends"] == [name]
            assert "run" not in task, "Delegation must not invent a package script"
        assert "deps:python" not in data["tasks"]
        assert "test:python" not in data["tasks"]
        self.assert_preserved(self.project, originals)
        for relative in (
            "scripts/ruff.toml",
            "scripts/ty.toml",
            "web/biome.json",
            "engine/rustfmt.toml",
            "engine/clippy.toml",
            "lua/stylua.toml",
            "lua/.styluaignore",
            "docs/.markdownlint.json",
            "docs/.prettierrc.json",
            "docs/.prettierignore",
        ):
            assert (self.project / relative).is_file(), relative
        for relative in (
            "ruff.toml",
            "biome.json",
            "rustfmt.toml",
            "stylua.toml",
            ".markdownlint.json",
        ):
            assert not (self.project / relative).exists(), relative
        self.assert_backup(self.project, "mise.toml", originals["mise.toml"])
        assert self.backups(self.project, "engine/rust-toolchain.toml") == []
        before = self.snapshot()
        self.succeed(*arguments)
        assert self.snapshot() == before
        self.run_mock.assert_not_called()

    def test_javascript_missing_package_scripts_and_root_tasks_omits_tasks_with_warning(
        self,
    ) -> None:
        self.write(self.project, "web/package.json", "{}\n")
        output = self.succeed("--tools", "javascript", "--javascript-dir", "web", "--apply")
        tasks = self.generated()["tasks"]
        for name in ("typecheck", "test"):
            assert name + ":javascript" not in tasks
            assert name in output.lower()
        assert "warn" in output.lower()
        assert (self.project / "web/package.json").read_bytes() == b"{}\n"

    def test_javascript_existing_package_scripts_are_used(self) -> None:
        manifest = '{"scripts":{"typecheck":"tsc --noEmit","test":"node --test"}}\n'
        self.write(self.project, "package.json", manifest)
        self.succeed("--tools", "javascript", "--apply")
        tasks = self.generated()["tasks"]
        for name in ("typecheck", "test"):
            assert name in str(tasks[name + ":javascript"]["run"])
        assert (self.project / "package.json").read_bytes() == manifest.encode()

    def test_typescript_is_a_javascript_selection_alias(self) -> None:
        profiles = []
        for tool in ("javascript", "typescript"):
            with self.subTest(tool=tool):
                project = self.sandbox / tool
                project.mkdir()
                self.write(project, "package.json", "{}\n")
                self.succeed("--tools", tool, "--apply", project=project)
                data = self.generated(project)
                assert (project / "biome.json").is_file()
                assert "check:javascript" in data["tasks"]
                assert "check:typescript" not in data["tasks"]
                assert "check:python" not in data["tasks"]
                profiles.append(data)
        assert profiles[0] == profiles[1]

    def test_only_selected_tool_configs_and_tasks_are_installed(self) -> None:
        self.succeed("--tools", "lua", "--apply")
        data = self.generated()
        assert (self.project / "stylua.toml").is_file()
        assert (self.project / ".styluaignore").is_file()
        for language in ("python", "javascript", "rust", "markdown"):
            assert "check:" + language not in data["tasks"]
        for relative in (
            "ruff.toml",
            "ty.toml",
            "biome.json",
            "rust-toolchain.toml",
            ".markdownlint.json",
        ):
            assert not (self.project / relative).exists()

    def test_python_auto_mode_uses_scripts_without_pyproject(self) -> None:
        self.write(self.project, "scripts/report.py", 'print("report")\n')
        self.succeed("--tools", "python", "--python-dir", "scripts", "--apply")
        data = self.generated()
        assert "deps:python" not in data["tasks"]
        assert "test:python" not in data["tasks"]
        assert self.python_targets(data) == ["."]
        assert (self.project / "scripts/ruff.toml").is_file()
        assert (self.project / "scripts/ty.toml").is_file()
        assert not list(self.project.rglob("pyproject.toml"))

    def test_explicit_python_scripts_mode_omits_project_tasks(self) -> None:
        for has_manifest in (False, True):
            with self.subTest(has_manifest=has_manifest):
                project = self.sandbox / f"scripts-{has_manifest}"
                project.mkdir()
                originals = self.python_project(project) if has_manifest else {}
                self.write(project, "report.py", 'print("report")\n')
                self.succeed(
                    "--tools",
                    "python",
                    "--python-mode",
                    "scripts",
                    "--apply",
                    project=project,
                )
                tasks = self.generated(project)["tasks"]
                assert "deps:python" not in tasks
                assert "test:python" not in tasks
                assert "typecheck:python" in tasks
                self.assert_preserved(project, originals)
                assert (project / "pyproject.toml").exists() == has_manifest

    def test_python_file_autodetection(self) -> None:
        for index, directories in enumerate((("src", "tests"), ("src",), ("tests",), ())):
            with self.subTest(directories=directories):
                project = self.sandbox / f"targets-{index}"
                project.mkdir()
                for directory in directories:
                    self.write(project, f"{directory}/sample.py", "pass\n")
                self.succeed(
                    "--tools",
                    "python",
                    "--python-mode",
                    "scripts",
                    "--apply",
                    project=project,
                )
                assert self.python_targets(self.generated(project)) == (list(directories) or ["."])

    def test_explicit_python_files_version_and_project_mode(self) -> None:
        originals = self.python_project(self.project / "backend", embedded=False)
        self.write(self.project, "backend/cli.py", "pass\n")
        self.write(self.project, "backend/jobs.py", "pass\n")
        self.succeed(
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
        )
        data = self.generated()
        assert self.python_targets(data) == ["cli.py", "jobs.py"]
        assert data["vars"]["python_version"] == "3.13"
        assert data["tools"]["python"] == "3.15"
        assert "deps:python" in data["tasks"]
        assert "test:python" in data["tasks"]
        with (self.project / "backend/ruff.toml").open("rb") as file:
            assert tomllib.load(file)["target-version"] == "py313"
        with (self.project / "backend/ty.toml").open("rb") as file:
            assert tomllib.load(file)["environment"]["python-version"] == "3.13"
        self.assert_preserved(self.project / "backend", originals)

    def test_shared_node_conflict_requires_resolution_before_writes(self) -> None:
        self.write(self.project, "package.json", "{}\n")
        self.refuse_without_writes("--tools", "javascript", "markdown", "--apply")

    def test_explicit_node_version_resolves_shared_conflict(self) -> None:
        self.write(self.project, "package.json", "{}\n")
        self.succeed("--tools", "javascript", "markdown", "--node-version", "24.15.0", "--apply")
        data = self.generated()
        assert data["tools"]["node"] == "24.15.0"
        assert "check:javascript" in data["tasks"]
        assert "check:markdown" in data["tasks"]

    def test_existing_pinned_rust_toolchain_is_not_replaced_by_config_overwrite(self) -> None:
        originals = self.monorepo()
        for flag in ("--replace-configs", "--overwrite"):
            with self.subTest(flag=flag):
                self.succeed("--tools", "rust", "--rust-dir", "engine", flag, "--apply")
                self.assert_preserved(self.project, originals)
                assert self.backups(self.project, "engine/rust-toolchain.toml") == []

    def test_relative_language_directory_traversal_is_rejected(self) -> None:
        for tool in ("python", "javascript", "rust", "lua", "markdown"):
            with self.subTest(tool=tool):
                self.refuse_without_writes("--tools", tool, f"--{tool}-dir", "../outside", "--apply")

    def test_absolute_outside_directory_is_rejected(self) -> None:
        self.refuse_without_writes("--tools", "lua", "--lua-dir", str(self.outside), "--apply")

    def test_agent_file_traversal_is_rejected(self) -> None:
        self.write(self.outside, "AGENS.md", "# Outside instructions\n")
        self.refuse_without_writes("--tools", "lua", "--agents-file", "../outside/AGENS.md", "--apply")

    def test_python_file_traversal_is_rejected(self) -> None:
        self.write(self.outside, "report.py", "pass\n")
        self.refuse_without_writes("--tools", "python", "--python-files", "../outside/report.py", "--apply")

    def test_symlinked_language_directories_outside_project_are_rejected(self) -> None:
        (self.project / "linked").symlink_to(self.outside, target_is_directory=True)
        for tool in ("python", "javascript", "rust", "lua", "markdown"):
            with self.subTest(tool=tool):
                self.refuse_without_writes("--tools", tool, f"--{tool}-dir", "linked", "--apply")

    def test_symlinked_python_file_outside_project_is_rejected(self) -> None:
        target = self.write(self.outside, "report.py", "pass\n")
        (self.project / "report.py").symlink_to(target)
        self.refuse_without_writes("--tools", "python", "--python-files", "report.py", "--apply")

    def test_symlinked_config_agent_fragment_and_backup_paths_are_rejected(
        self,
    ) -> None:
        cases = (
            ("lua", ".mise", True),
            ("lua", ".mise/conf.d", True),
            ("lua", FRAGMENT.as_posix(), False),
            ("lua", "mise.toml", False),
            ("lua", ".mise.toml", False),
            ("lua", "stylua.toml", False),
            ("lua", ".stylua.toml", False),
            ("lua", ".styluaignore", False),
            ("lua", "AGENS.md", False),
            ("lua", "AGENTS.md", False),
            ("lua", ".tooling-backups", True),
            ("python", ".ruff.toml", False),
        )
        for index, (tool, relative, is_directory) in enumerate(cases):
            with self.subTest(path=relative):
                project = self.sandbox / f"symlink-{index}"
                project.mkdir()
                target = self.outside / f"target-{index}"
                if is_directory:
                    target.mkdir()
                else:
                    target.write_text("# Outside file must remain untouched\n", encoding="utf-8")
                link = project / relative
                link.parent.mkdir(parents=True, exist_ok=True)
                link.symlink_to(target, target_is_directory=is_directory)
                if relative not in ("AGENS.md", "AGENTS.md"):
                    self.write(
                        project,
                        "AGENTS.md",
                        "# Existing instructions requiring a backup\n",
                    )
                arguments = ("--agents-file", "AGENS.md") if relative == "AGENS.md" else ()
                self.refuse_without_writes("--tools", tool, *arguments, "--apply", project=project)

    def test_explicit_symlinked_agent_file_is_rejected(self) -> None:
        target = self.write(self.outside, "TEAM.md", "# Outside team guidance\n")
        (self.project / "TEAM.md").symlink_to(target)
        self.refuse_without_writes("--tools", "lua", "--agents-file", "TEAM.md", "--apply")

    def test_optional_install_commands_are_planned_but_never_run_in_preview_or_dry_run(
        self,
    ) -> None:
        self.python_project()
        for mode in ((), ("--dry-run",)):
            for flags in (
                ("--install-tools",),
                ("--install-deps",),
                ("--install-tools", "--install-deps"),
            ):
                with self.subTest(mode=mode, flags=flags):
                    before = self.snapshot()
                    output = self.succeed("--tools", "python", *mode, *flags)
                    assert output.strip()
                    assert "mise" in output.lower()
                    assert self.snapshot() == before
                    self.run_mock.assert_not_called()

    def test_optional_install_commands_execute_only_on_apply(self) -> None:
        self.run_mock.side_effect = None
        self.run_mock.return_value = subprocess.CompletedProcess([], 0, stdout="", stderr="")
        for index, flags in enumerate(
            (
                ("--install-tools",),
                ("--install-deps",),
                ("--install-tools", "--install-deps"),
            )
        ):
            with self.subTest(flags=flags):
                project = self.sandbox / f"install-commands-{index}"
                project.mkdir()
                originals = self.python_project(project)
                self.run_mock.reset_mock()
                self.succeed("--tools", "python", *flags, "--apply", project=project)
                self.run_mock.assert_called()
                self.assert_preserved(project, originals)

    def test_external_command_oserror_is_reported_as_failure(self) -> None:
        self.run_mock.side_effect = FileNotFoundError("mise executable is unavailable")
        status, output = self.invoke("--tools", "lua", "--install-tools", "--apply")
        assert status == 1, output
        assert "mise executable is unavailable" in output
        self.run_mock.assert_called()

    def test_missing_project_is_reported_without_writes(self) -> None:
        self.refuse_without_writes("--tools", "lua", "--apply", project=self.sandbox / "missing")

    def test_config_overwrite_converts_prettier_alternatives_with_deletion_backups(self) -> None:
        alternatives = (
            ("prettier.config.js", "export default { printWidth: 79 };\n"),
            ("prettier.config.cjs", "module.exports = { printWidth: 79 };\n"),
            (".prettierrc.js", "export default { printWidth: 79 };\n"),
            (".prettierrc.cjs", "module.exports = { printWidth: 79 };\n"),
            (".prettierrc.toml", "printWidth = 79\n"),
        )
        for flag in ("--replace-configs", "--overwrite"):
            for index, (relative, text) in enumerate(alternatives):
                with self.subTest(flag=flag, config=relative):
                    project = self.sandbox / f"prettier-conversion-{flag.removeprefix('--')}-{index}"
                    project.mkdir()
                    manifest = '{"private":true}\n'
                    self.write(project, "package.json", manifest)
                    self.write(project, relative, text)
                    self.succeed("--tools", "markdown", flag, "--apply", project=project)
                    assert not (project / relative).exists(), "Do not put JSON into the old JS/CJS/TOML filename"
                    canonical = project / ".prettierrc.json"
                    assert canonical.is_file()
                    assert isinstance(json.loads(canonical.read_text(encoding="utf-8")), dict)
                    self.assert_backup(project, relative, text.encode())
                    assert (project / "package.json").read_bytes() == manifest.encode()
                    before = self.snapshot()
                    self.succeed("--tools", "markdown", flag, "--apply", project=project)
                    assert self.snapshot() == before
                    self.run_mock.assert_not_called()

    def test_replace_configs_accepts_compatible_prettier_formats(self) -> None:
        alternatives = (
            (".prettierrc", '{"printWidth":79}\n'),
            (".prettierrc.json", '{"printWidth":79}\n'),
            (".prettierrc.jsonc", '{\n// Existing formatting preferences\n"printWidth":79,\n}\n'),
            (".prettierrc.yaml", "printWidth: 79\n"),
            (".prettierrc.yml", "printWidth: 79\n"),
        )
        for index, (relative, text) in enumerate(alternatives):
            with self.subTest(config=relative):
                project = self.sandbox / f"compatible-prettier-{index}"
                project.mkdir()
                manifest = self.write(project, "package.json", '{"private":true}\n')
                original_manifest = manifest.read_bytes()
                existing = self.write(project, relative, text)
                self.succeed("--tools", "markdown", "--replace-configs", "--apply", project=project)
                canonical = project / ".prettierrc.json"
                assert existing.is_file() or canonical.is_file()
                if not existing.exists() or existing.read_bytes() != text.encode():
                    self.assert_backup(project, relative, text.encode())
                else:
                    assert canonical != existing, "Replacement did not install a config"
                    assert canonical.is_file(), "Replacement did not install a config"
                    assert canonical.read_bytes() != text.encode()
                assert manifest.read_bytes() == original_manifest
                self.run_mock.assert_not_called()

    def test_root_aggregate_cycles_omit_unsafe_javascript_aliases_with_warning(self) -> None:
        for index, dependency in enumerate(("*", "javascript")):
            with self.subTest(dependency=dependency):
                project = self.sandbox / f"aggregate-cycle-{index}"
                project.mkdir()
                self.write(project, "web/package.json", "{}\n")
                original = (
                    '[tasks.typecheck]\ndepends = ["typecheck:' + dependency + '"]\n'
                    '\n[tasks.test]\ndepends = ["test:' + dependency + '"]\n'
                )
                self.write(project, "mise.toml", original)
                output = self.succeed("--tools", "javascript", "--javascript-dir", "web", "--apply", project=project)
                tasks = self.generated(project)["tasks"]
                assert "typecheck:javascript" not in tasks
                assert "test:javascript" not in tasks
                assert "check:javascript" in tasks
                assert "warn" in output.lower()
                assert any(word in output.lower() for word in ("cycl", "circular"))
                self.assert_preserved(project, {"mise.toml": original.encode(), "web/package.json": b"{}\n"})

    def test_other_mise_fragment_supplies_directories_node_pin_and_existing_tasks(self) -> None:
        files = {
            "mise.toml": '# Preserve root configuration.\n[tools]\nruff = "0.11.13"\n',
            ".mise/conf.d/10-project.toml": (
                '[tools]\nnode = "22.14.0"\n'
                '\n[vars]\npython_dir = "scripts"\njavascript_dir = "web"\nmarkdown_dir = "docs"\n'
                '\n[tasks.typecheck]\nrun = "echo project typecheck"\n'
                '\n[tasks.test]\nrun = "echo project test"\n'
                '\n[tasks."check:javascript"]\nrun = "echo custom javascript check"\n'
                '\n[tasks."typecheck:javascript"]\nrun = "echo custom javascript types"\n'
                '\n[tasks."format:markdown"]\nrun = "echo custom markdown format"\n'
            ),
            "scripts/report.py": 'print("report")\n',
            "web/package.json": "{}\n",
            "docs/package.json": '{"private":true}\n',
            "docs/README.md": "# Project docs\n",
        }
        for relative, text in files.items():
            self.write(self.project, relative, text)
        self.succeed("--tools", "python", "javascript", "markdown", "--apply")
        data = self.generated()
        inherited = tomllib.loads(files[".mise/conf.d/10-project.toml"])
        assert data.get("tools", {}).get("node", inherited["tools"]["node"]) == "22.14.0"
        for name, directory in inherited["vars"].items():
            assert data.get("vars", {}).get(name, directory) == directory
        for name, task in inherited["tasks"].items():
            assert data["tasks"].get(name, task) == task, f"Existing fragment task was overridden: {name}"
        assert data["tasks"]["test:javascript"]["depends"] == ["test"]
        assert "run" not in data["tasks"]["test:javascript"]
        for relative in ("scripts/ruff.toml", "scripts/ty.toml", "web/biome.json", "docs/.markdownlint.json"):
            assert (self.project / relative).is_file(), relative
        for relative in ("ruff.toml", "ty.toml", "biome.json", ".markdownlint.json"):
            assert not (self.project / relative).exists(), relative
        self.assert_preserved(self.project, {relative: text.encode() for relative, text in files.items()})
        self.assert_backup(self.project, "mise.toml", files["mise.toml"].encode())
        assert self.backups(self.project, ".mise/conf.d/10-project.toml") == []
        self.run_mock.assert_not_called()

    def test_markdown_package_manager_controls_cli_and_dependency_installation(self) -> None:
        self.run_mock.side_effect = None
        self.run_mock.return_value = subprocess.CompletedProcess([], 0, stdout="", stderr="")
        managers = (
            ("npm", "10.9.2", "package-lock.json", '{"lockfileVersion":3,"packages":{}}\n'),
            ("pnpm", "10.5.2", "pnpm-lock.yaml", "lockfileVersion: '9.0'\n"),
            ("yarn", "4.9.1", "yarn.lock", "__metadata:\n  version: 8\n"),
        )
        for manager, version, lockfile, lock in managers:
            with self.subTest(manager=manager):
                project = self.sandbox / f"markdown-{manager}"
                project.mkdir()
                manifest = json.dumps({"private": True, "packageManager": f"{manager}@{version}"}) + "\n"
                files = {
                    "docs/package.json": manifest,
                    f"docs/{lockfile}": lock,
                    "docs/README.md": "# Documentation\n",
                    "docs/.gitignore": "node_modules/\n",
                }
                for relative, text in files.items():
                    self.write(project, relative, text)
                self.run_mock.reset_mock()
                self.succeed(
                    "--tools", "markdown", "--markdown-dir", "docs", "--install-deps", "--apply", project=project
                )
                tasks = self.generated(project)["tasks"]
                for name in ("check:markdown", "format:markdown"):
                    commands = self.task_commands(tasks[name])
                    assert commands, name
                    for command in commands:
                        parsed = self.resolved_command(command)
                        if manager == "npm":
                            assert parsed[:2] == ["npm", "exec"] or parsed[0] == "npx", parsed
                        else:
                            assert parsed[:2] == [manager, "exec"], parsed
                commands = self.recorded_commands()
                assert commands, "--install-deps did not execute a dependency command"
                installation = [self.resolved_command(command) for command in commands]
                for command in commands:
                    if Path(command[0]).name == "mise" and "run" in command:
                        for argument in command[1:]:
                            if argument in tasks:
                                installation.extend(
                                    self.resolved_command(nested) for nested in self.task_commands(tasks[argument])
                                )
                assert any(
                    Path(command[0]).name == manager and any(word in command[1:] for word in ("install", "add"))
                    for command in installation
                ), installation
                for command in installation:
                    assert not any(Path(word).name in ("bun", "bunx") for word in command), command
                for task in tasks.values():
                    for command in self.task_commands(task):
                        assert not any(Path(word).name in ("bun", "bunx") for word in command), command
                assert not list(project.rglob("bun.lock"))
                assert not list(project.rglob("bun.lockb"))
                self.assert_preserved(project, {relative: text.encode() for relative, text in files.items()})

    def test_workspace_root_package_manager_is_inherited_by_nested_web_and_docs(self) -> None:
        for manager in ("npm", "pnpm", "yarn"):
            with self.subTest(manager=manager):
                project = self.sandbox / f"inherited-workspace-manager-{manager}"
                project.mkdir()
                originals, _ = self.workspace_project(project, manager)
                arguments = (
                    "--tools",
                    "javascript",
                    "markdown",
                    "--javascript-dir",
                    "web",
                    "--markdown-dir",
                    "docs",
                    "--apply",
                )
                self.succeed(*arguments, project=project)
                generated = self.generated(project)
                assert generated["vars"]["javascript_dir"] == "web"
                assert generated["vars"]["markdown_dir"] == "docs"
                tasks = generated["tasks"]
                for name in ("deps:javascript", "typecheck:javascript", "test:javascript"):
                    commands = self.task_commands(tasks[name])
                    assert commands, name
                    for command in commands:
                        argv = self.resolved_command(command)
                        assert Path(argv[0]).name == manager, argv
                for name in ("check:markdown", "format:markdown"):
                    commands = self.task_commands(tasks[name])
                    assert commands, name
                    for command in commands:
                        argv = self.resolved_command(command)
                        program = Path(argv[0]).name
                        if manager == "npm" and program == "npx":
                            continue
                        assert program == manager, argv
                        assert argv[1:2] == ["exec"], argv
                for task in tasks.values():
                    for command in self.task_commands(task):
                        assert not any(Path(word).name in ("bun", "bunx") for word in command), command
                assert not list(project.rglob("bun.lock"))
                assert not list(project.rglob("bun.lockb"))
                self.assert_central_dependencies(project, {"javascript", "markdown"})
                self.assert_preserved(project, originals)
                before = self.snapshot()
                self.succeed(*arguments, project=project)
                assert self.snapshot() == before
                self.run_mock.assert_not_called()

    def test_workspace_shared_dependency_backups_precede_commands_and_keep_original_lock(self) -> None:
        for manager in ("npm", "pnpm", "yarn"):
            with self.subTest(manager=manager):
                project = self.sandbox / f"workspace-backups-{manager}"
                project.mkdir()
                originals, lockfile = self.workspace_project(project, manager)
                commands = self.run_workspace_dependency_commands(project, manager, originals, lockfile)
                assert commands
                assert (project / lockfile).read_bytes() != originals[lockfile]
                for relative in ("package.json", lockfile, "web/package.json", "docs/package.json"):
                    self.assert_backup(project, relative, originals[relative])
                for directory in ("web", "docs"):
                    for name in ("package-lock.json", "pnpm-lock.yaml", "yarn.lock", "bun.lock", "bun.lockb"):
                        assert not (project / directory / name).exists(), f"Unexpected nested workspace lock: {name}"
                self.assert_preserved(
                    project, {relative: contents for relative, contents in originals.items() if relative != lockfile}
                )

    def test_tool_only_pyproject_uses_scripts_auto_mode_without_dependency_commands(self) -> None:
        self.run_mock.side_effect = None
        self.run_mock.return_value = subprocess.CompletedProcess([], 0, stdout="", stderr="")
        manifest = '[tool.ruff]\nline-length = 88\ntarget-version = "py312"\n'
        self.write(self.project, "pyproject.toml", manifest)
        self.write(self.project, "report.py", 'print("report")\n')
        self.succeed("--tools", "python", "--install-deps", "--apply")
        data = self.generated()
        assert "deps:python" not in data["tasks"]
        assert "test:python" not in data["tasks"]
        assert "typecheck:python" in data["tasks"]
        assert self.python_targets(data) == ["."]
        for task in data["tasks"].values():
            for command in self.task_commands(task):
                assert "pytest" not in command
                assert command[:2] not in (["uv", "add"], ["uv", "sync"])
        assert (self.project / "pyproject.toml").read_bytes() == manifest.encode()
        assert not (self.project / "ruff.toml").exists()
        assert not (self.project / "uv.lock").exists()
        for command in self.recorded_commands():
            argv = self.resolved_command(command)
            program = Path(argv[0]).name
            assert not (program == "uv" and argv[1:2] in (["add"], ["sync"])), argv
            assert program not in {"npm", "npx", "pnpm", "yarn", "bun", "bunx", "pip", "pip3", "poetry", "pdm"}, argv
            assert not (program.startswith("python") and argv[1:3] == ["-m", "pip"]), argv
            assert "deps:python" not in argv, argv
            assert "pytest" not in argv, argv

    def test_rust_install_tools_uses_inherited_root_toolchain_pin(self) -> None:
        self.run_mock.side_effect = None
        self.run_mock.return_value = subprocess.CompletedProcess([], 0, stdout="", stderr="")
        pin = "1.85.1"
        for index, (name, text) in enumerate(
            (
                ("rust-toolchain.toml", f'[toolchain]\nchannel = "{pin}"\ncomponents = ["rustfmt", "clippy"]\n'),
                ("rust-toolchain", pin + "\n"),
            )
        ):
            with self.subTest(toolchain=name):
                project = self.sandbox / f"inherited-rust-{index}"
                project.mkdir()
                files = {
                    name: text,
                    "engine/Cargo.toml": '[package]\nname = "engine"\nversion = "0.1.0"\nedition = "2021"\n',
                    "engine/Cargo.lock": "version = 4\n",
                    "engine/src/lib.rs": "pub fn answer() -> u32 { 42 }\n",
                }
                for relative, content in files.items():
                    self.write(project, relative, content)
                self.run_mock.reset_mock()
                self.succeed("--tools", "rust", "--rust-dir", "engine", "--install-tools", "--apply", project=project)
                commands = [command for command in self.recorded_commands() if Path(command[0]).name == "rustup"]
                assert commands, "No optional rustup command was planned for the inherited toolchain"
                assert any(pin in command or f"--toolchain={pin}" in command for command in commands), commands
                assert not any("stable" in command or "--toolchain=stable" in command for command in commands), commands
                child_toolchain = project / "engine/rust-toolchain.toml"
                if child_toolchain.exists():
                    with child_toolchain.open("rb") as file:
                        assert tomllib.load(file)["toolchain"]["channel"] == pin
                self.assert_preserved(project, {relative: content.encode() for relative, content in files.items()})
                assert self.backups(project, name) == []

    def test_markdown_cli2_config_does_not_suppress_legacy_cli_config(self) -> None:
        alternatives = (
            (".markdownlint-cli2.jsonc", '{\n// CLI2-specific configuration\n"config":{"MD013":false}\n}\n'),
            (".markdownlint-cli2.yaml", "config:\n  MD013: false\n"),
            (".markdownlint-cli2.cjs", "module.exports = { config: { MD013: false } };\n"),
        )
        for index, (relative, text) in enumerate(alternatives):
            with self.subTest(config=relative):
                project = self.sandbox / f"markdown-cli2-{index}"
                project.mkdir()
                self.write(project, "package.json", '{"private":true}\n')
                self.write(project, relative, text)
                output = self.succeed("--tools", "markdown", "--apply", project=project)
                canonical = project / ".markdownlint.json"
                assert canonical.is_file(), "CLI2 config cannot configure the legacy markdownlint CLI"
                assert isinstance(json.loads(canonical.read_text(encoding="utf-8")), dict)
                assert (project / relative).read_bytes() == text.encode()
                assert self.backups(project, relative) == []
                assert "cli2" in output.lower(), "The incompatible CLI2 configuration should be mentioned"

    def test_existing_python_files_variable_is_respected_by_default(self) -> None:
        for index, source in enumerate(("mise.toml", ".mise/conf.d/10-project.toml")):
            with self.subTest(source=source):
                project = self.sandbox / f"inherited-python-files-{index}"
                project.mkdir()
                originals = self.python_project(project)
                self.write(project, "custom/report.py", "pass\n")
                self.write(project, "checks/test_report.py", "pass\n")
                settings = '[vars]\npython_files = "custom checks"\n'
                self.write(project, source, settings)
                self.succeed("--tools", "python", "--apply", project=project)
                variables = {"python_files": "custom checks", **self.generated(project).get("vars", {})}
                assert self.python_targets({"vars": variables}) == ["custom", "checks"]
                self.assert_preserved(project, {**originals, source: settings.encode()})

    def test_conflicting_explicit_python_file_or_version_vars_fail_before_writes(self) -> None:
        for index, source in enumerate(("mise.toml", ".mise/conf.d/10-project.toml")):
            for flag, values in (("--python-files", ("src",)), ("--python-version", ("3.13",))):
                with self.subTest(source=source, flag=flag):
                    project = self.sandbox / f"conflicting-vars-{index}-{flag.removeprefix('--')}"
                    project.mkdir()
                    self.python_project(project, embedded=False)
                    self.write(project, "custom/report.py", "pass\n")
                    self.write(project, "checks/test_report.py", "pass\n")
                    self.write(project, source, '[vars]\npython_files = "custom checks"\npython_version = "3.12"\n')
                    self.write(project, "AGENTS.md", "# Existing instructions must not change\n")
                    self.refuse_without_writes("--tools", "python", flag, *values, "--apply", project=project)

    def test_matching_explicit_python_file_and_version_vars_are_allowed(self) -> None:
        for index, source in enumerate(("mise.toml", ".mise/conf.d/10-project.toml")):
            with self.subTest(source=source):
                project = self.sandbox / f"matching-vars-{index}"
                project.mkdir()
                originals = self.python_project(project, embedded=False)
                self.write(project, "custom/report.py", "pass\n")
                self.write(project, "checks/test_report.py", "pass\n")
                settings = '[vars]\npython_files = "custom checks"\npython_version = "3.12"\n'
                self.write(project, source, settings)
                self.succeed(
                    "--tools",
                    "python",
                    "--python-files",
                    "custom",
                    "checks",
                    "--python-version",
                    "3.12",
                    "--apply",
                    project=project,
                )
                variables = {
                    "python_files": "custom checks",
                    "python_version": "3.12",
                    **self.generated(project).get("vars", {}),
                }
                assert self.python_targets({"vars": variables}) == ["custom", "checks"]
                assert variables["python_version"] == "3.12"
                self.assert_preserved(project, {**originals, source: settings.encode()})

    def test_central_aggregates_include_only_selected_available_language_tasks(self) -> None:
        cases = (
            ("python-project", ("python",), ()),
            ("python-scripts", ("python",), ("--python-mode", "scripts")),
            ("javascript", ("javascript",), ()),
            ("rust", ("rust",), ()),
            ("lua", ("lua",), ()),
            ("markdown", ("markdown",), ()),
            ("lua-rust", ("lua", "rust"), ()),
        )
        for label, languages, extra in cases:
            with self.subTest(selection=label):
                project = self.sandbox / f"central-{label}"
                project.mkdir()
                originals = {}
                if label == "python-project":
                    originals.update(self.python_project(project))
                elif "python" in languages:
                    self.write(project, "report.py", "pass\n")
                if "javascript" in languages or "markdown" in languages:
                    self.write(project, "package.json", "{}\n")
                    originals["package.json"] = b"{}\n"
                if "rust" in languages:
                    cargo = '[package]\nname = "sample"\nversion = "0.1.0"\nedition = "2021"\n'
                    self.write(project, "Cargo.toml", cargo)
                    self.write(project, "src/lib.rs", "pub fn answer() -> u32 { 42 }\n")
                    originals["Cargo.toml"] = cargo.encode()
                unselected = "python" if "rust" in languages else "rust"
                original_root = (
                    '# Preserve unrelated project tasks.\n[tasks.build]\nrun = "echo custom build"\n'
                    f'\n[tasks."check:{unselected}"]\nrun = "echo unselected check"\n'
                    f'\n[tasks."test:{unselected}"]\nrun = "echo unselected tests"\n'
                )
                self.write(project, "mise.toml", original_root)
                originals["mise.toml"] = original_root.encode()
                self.succeed("--tools", *languages, *extra, "--apply", project=project)
                self.assert_central_dependencies(project, set(languages))
                central = self.central_tasks(project)
                for language in languages:
                    assert f"check:{language}" in central["check"]["depends"]
                    assert f"format:{language}" in central["format"]["depends"]
                if label in ("python-scripts", "javascript"):
                    assert "test" not in central
                if label == "python-scripts":
                    assert "deps" not in central
                self.assert_preserved(project, originals)
                self.assert_backup(project, "mise.toml", original_root.encode())
                agent = (project / "AGENTS.md").read_text(encoding="utf-8")
                block = agent.split(START, 1)[1].split(END, 1)[0]
                assert "mise run" in block
                for name in central:
                    pattern = rf"\bmise\s+run\s+{re.escape(name)}(?![\w:-])"
                    assert re.search(pattern, block), f"Missing agent guidance for mise run {name}"
                before = self.snapshot()
                self.succeed("--tools", *languages, *extra, "--apply", project=project)
                assert self.snapshot() == before
                self.run_mock.assert_not_called()

    def test_user_aggregate_collisions_and_existing_bare_tasks_are_preserved(self) -> None:
        for root_name in ("mise.toml", ".mise.toml"):
            for name in sorted(CENTRAL_TASKS):
                with self.subTest(root=root_name, collision=name):
                    project = self.sandbox / f"collision-{root_name}-{name.replace(':', '-')}"
                    project.mkdir()
                    original = (
                        '# Keep project-owned entry points.\n[tasks.build]\nrun = "echo custom build"\n'
                        '\n[tasks.test]\nrun = "echo custom test"\ndepends = ["build"]\n'
                    )
                    if name == "test":
                        original += 'description = "User owned"\n'
                    original += '\n[tasks.typecheck]\nrun = "echo custom typecheck"\n'
                    if name != "test":
                        original += (
                            f'\n[tasks."{name}"]\nrun = "echo user aggregate"\n'
                            'description = "User owned"\ndepends = ["build"]\n'
                        )
                    self.write(project, root_name, original)
                    before_tasks = tomllib.loads(original)["tasks"]
                    self.succeed("--tools", "lua", "--apply", project=project)
                    root = self.root_mise(project)
                    assert root.name == root_name
                    with root.open("rb") as file:
                        tasks = tomllib.load(file)["tasks"]
                    for preserved in sorted({"build", "test", "typecheck", name}):
                        assert tasks[preserved] == before_tasks[preserved]
                        assert preserved not in self.central_tasks(project)
                    self.assert_central_dependencies(project, {"lua"}, preserved_aggregates={name, "test"})
                    self.assert_preserved(project, {root_name: original.encode()})
                    before = self.snapshot()
                    self.succeed("--tools", "lua", "--apply", project=project)
                    assert self.snapshot() == before

    def test_managed_aggregates_compose_with_explicit_tasks_parent_and_inline_children(self) -> None:
        cases = (
            ("shorthand", '[tasks]\nbuild = "echo existing build"\n'),
            ("inline-child", '[tasks]\nbuild = { run = "echo existing build", description = "User owned" }\n'),
            ("child-table", '[tasks]\n[tasks.build]\nrun = "echo existing build"\n'),
            (
                "inline-aggregate",
                '[tasks]\nbuild = "echo existing build"\ncheck = { run = "echo user check", depends = ["build"] }\n',
            ),
            ("shorthand-aggregate", '[tasks]\nbuild = { run = "echo existing build" }\ncheck = "echo user check"\n'),
        )
        for label, source in cases:
            with self.subTest(tasks_style=label):
                project = self.sandbox / f"explicit-tasks-{label}"
                project.mkdir()
                original = "# Preserve the explicit tasks parent and user syntax.\n" + source
                original_tasks = tomllib.loads(original)["tasks"]
                root = self.write(project, "mise.toml", original)
                self.succeed("--tools", "lua", "--apply", project=project)
                with root.open("rb") as file:
                    tasks = tomllib.load(file)["tasks"]
                for name, task in original_tasks.items():
                    assert tasks[name] == task, name
                _, block, _ = self.managed_root_parts(root.read_bytes())
                assert not re.search(rb"(?m)^[ \t]*\[tasks\][ \t]*(?:#.*)?$", block)
                self.assert_central_dependencies(
                    project, {"lua"}, preserved_aggregates=set(original_tasks) & CENTRAL_TASKS
                )
                self.assert_preserved(project, {"mise.toml": original.encode()})
                self.assert_backup(project, "mise.toml", original.encode())
                before = self.snapshot()
                self.succeed("--tools", "lua", "--apply", project=project)
                assert self.snapshot() == before
                self.run_mock.assert_not_called()

    def test_central_block_replacement_preserves_surroundings_and_updates_selection(self) -> None:
        prefix = '# Exact prefix stays.\n\n[tasks.before]\nrun = "echo before"\n\n'
        suffix = '\n\n# Exact suffix stays.\n[tasks.after]\nrun = "echo after"\n'
        stale = (
            '[tasks.check]\ndepends = ["check:rust"]\n'
            '\n[tasks.format]\ndepends = ["format:rust"]\n'
            '\n[tasks.test]\ndepends = ["test:rust"]\n'
            '\n[tasks.deps]\ndepends = ["deps:python"]\n'
        )
        original = prefix + AGGREGATES_START + "\n" + stale + AGGREGATES_END + suffix
        root = self.write(self.project, "mise.toml", original)
        self.write(self.project, "report.py", "pass\n")
        self.succeed("--tools", "lua", "--apply")
        self.assert_central_dependencies(self.project, {"lua"})
        self.assert_preserved(self.project, {"mise.toml": original.encode()})
        self.assert_backup(self.project, "mise.toml", original.encode())
        lua_root = root.read_bytes()
        before = self.snapshot()
        self.succeed("--tools", "lua", "--apply")
        assert self.snapshot() == before
        self.succeed("--tools", "python", "--python-mode", "scripts", "--apply")
        self.assert_central_dependencies(self.project, {"python"})
        central = self.central_tasks()
        assert "check:python" in central["check"]["depends"]
        assert "format:python" in central["format"]["depends"]
        assert "test" not in central
        assert "deps" not in central
        self.assert_preserved(self.project, {"mise.toml": original.encode()})
        assert any(path.read_bytes() == lua_root for path in self.backups(self.project, "mise.toml"))
        before = self.snapshot()
        self.succeed("--tools", "python", "--python-mode", "scripts", "--apply")
        assert self.snapshot() == before

    def test_malformed_root_toml_or_aggregate_markers_fail_before_any_writes(self) -> None:
        start, end = AGGREGATES_START, AGGREGATES_END
        invalid = (
            start + "\n",
            end + "\n",
            end + "\n" + start + "\n",
            start + "\n" + start + "\n" + end + "\n",
            start + "\n" + end + "\n" + end + "\n",
            start + "\n" + end + "\n" + start + "\n" + end + "\n",
            start + "\n[tasks.\n" + end + "\n",
            "[tools]\npython = [\n",
        )
        for root_index, root_name in enumerate(("mise.toml", ".mise.toml")):
            for index, contents in enumerate(invalid):
                with self.subTest(root=root_name, contents=contents):
                    project = self.sandbox / f"invalid-central-{root_index}-{index}"
                    project.mkdir()
                    self.write(project, root_name, contents)
                    self.write(project, "AGENTS.md", "# Keep local instructions\n")
                    self.write(project, "stylua.toml", 'syntax = "Lua54"\n')
                    self.refuse_without_writes("--tools", "lua", "--overwrite", "--apply", project=project)

    def test_overwrite_preview_never_replaces_or_deletes_existing_configs(self) -> None:
        self.write(self.project, "package.json", '{"private":true,"packageManager":"npm@10.9.2"}\n')
        self.write(self.project, "mise.toml", '# Preserve existing root settings.\n[vars]\ncustom = "keep"\n')
        self.write(self.project, "prettier.config.js", "export default { printWidth: 79 };\n")
        self.write(self.project, ".prettierrc.json", '{"printWidth":81}\n')
        self.write(self.project, "AGENTS.md", "# Preserve local instructions\n")
        for mode in ((), ("--dry-run",)):
            with self.subTest(mode=mode):
                before = self.snapshot()
                self.succeed("--tools", "markdown", "--overwrite", "--install-deps", *mode)
                assert self.snapshot() == before
                self.run_mock.assert_not_called()

    def test_overwrite_rejects_outside_symlinks_before_config_replacement_or_deletion(self) -> None:
        cases = (
            ("markdown", "prettier.config.js", "export default {};\n"),
            ("markdown", ".prettierrc.cjs", "module.exports = {};\n"),
            ("markdown", ".prettierrc.toml", "printWidth = 79\n"),
            ("rust", "rust-toolchain.toml", '[toolchain]\nchannel = "1.85.1"\n'),
        )
        for index, (tool, relative, contents) in enumerate(cases):
            with self.subTest(tool=tool, config=relative):
                project = self.sandbox / f"overwrite-symlink-{index}"
                project.mkdir()
                target = self.write(self.outside, f"config-{index}", contents)
                (project / relative).symlink_to(target)
                if tool == "rust":
                    self.write(project, "Cargo.toml", '[package]\nname = "sample"\nversion = "0.1.0"\n')
                    self.write(project, "src/lib.rs", "pub fn answer() -> u32 { 42 }\n")
                    extra = ("--overwrite-rust-toolchain",)
                else:
                    self.write(project, "package.json", "{}\n")
                    self.write(project, ".prettierrc.json", '{"printWidth":81}\n')
                    extra = ()
                self.write(project, "AGENTS.md", "# Existing local instructions\n")
                self.refuse_without_writes("--tools", tool, "--overwrite", *extra, "--apply", project=project)

    def test_overwrite_replaces_canonical_configs_and_cleans_up_alternates_with_backups(self) -> None:
        cases = (
            ("python", "ruff.toml", ".ruff.toml", "line-length = 79\n", "line-length = 81\n"),
            (
                "javascript",
                "biome.json",
                "biome.jsonc",
                '{"formatter":{"lineWidth":79}}\n',
                '{\n// Alternate config\n"formatter":{"lineWidth":81}\n}\n',
            ),
            ("lua", "stylua.toml", ".stylua.toml", "indent_width = 2\n", "indent_width = 3\n"),
        )
        for tool, canonical, alternate, original, old_alternate in cases:
            with self.subTest(tool=tool):
                project = self.sandbox / f"overwrite-cleanup-{tool}"
                project.mkdir()
                manifests = {}
                if tool == "python":
                    manifests.update(self.python_project(project, embedded=False))
                elif tool == "javascript":
                    self.write(project, "package.json", "{}\n")
                    manifests["package.json"] = b"{}\n"
                self.write(project, canonical, original)
                self.write(project, alternate, old_alternate)
                self.succeed("--tools", tool, "--overwrite", "--apply", project=project)
                path = project / canonical
                assert path.read_bytes() != original.encode()
                if canonical.endswith(".json"):
                    assert isinstance(json.loads(path.read_text(encoding="utf-8")), dict)
                else:
                    with path.open("rb") as file:
                        tomllib.load(file)
                assert not (project / alternate).exists(), "Obsolete alternate config was not removed"
                self.assert_backup(project, canonical, original.encode())
                self.assert_backup(project, alternate, old_alternate.encode())
                self.assert_preserved(project, manifests)
                before = self.snapshot()
                self.succeed("--tools", tool, "--overwrite", "--apply", project=project)
                assert self.snapshot() == before
                self.run_mock.assert_not_called()

    def test_explicit_rust_toolchain_overwrite_opt_in_replaces_pin_with_backup(self) -> None:
        with (REPO_ROOT / "templates/rust/rust-toolchain.toml").open("rb") as file:
            expected = tomllib.load(file)["toolchain"]["channel"]
        for index, (relative, original) in enumerate(
            (
                ("engine/rust-toolchain.toml", '[toolchain]\nchannel = "1.85.1"\ncomponents = ["rustfmt"]\n'),
                ("engine/rust-toolchain", "1.85.1\n"),
            )
        ):
            with self.subTest(toolchain=relative):
                project = self.sandbox / f"explicit-rust-overwrite-{index}"
                project.mkdir()
                files = {
                    "engine/Cargo.toml": '[package]\nname = "engine"\nversion = "0.1.0"\nedition = "2021"\n',
                    "engine/Cargo.lock": "version = 4\n",
                    "engine/src/lib.rs": "pub fn answer() -> u32 { 42 }\n",
                }
                for path, contents in files.items():
                    self.write(project, path, contents)
                self.write(project, relative, original)
                arguments = (
                    "--tools",
                    "rust",
                    "--rust-dir",
                    "engine",
                    "--overwrite",
                    "--overwrite-rust-toolchain",
                    "--apply",
                )
                self.succeed(*arguments, project=project)
                canonical = project / "engine/rust-toolchain.toml"
                if canonical.exists():
                    with canonical.open("rb") as file:
                        assert tomllib.load(file)["toolchain"]["channel"] == expected
                    if relative != "engine/rust-toolchain.toml":
                        assert not (project / relative).exists(), "A legacy pin must not shadow the replacement"
                    else:
                        assert canonical.read_bytes() != original.encode()
                else:
                    assert relative == "engine/rust-toolchain"
                    assert (project / relative).read_text(encoding="utf-8").strip() == expected
                self.assert_backup(project, relative, original.encode())
                self.assert_preserved(project, {path: contents.encode() for path, contents in files.items()})
                before = self.snapshot()
                self.succeed(*arguments, project=project)
                assert self.snapshot() == before
                self.run_mock.assert_not_called()

    def test_tools_are_required_and_unknown_choices_are_rejected(self) -> None:
        for arguments in ((), ("--tools", "unknown-tool")):
            with self.subTest(arguments=arguments):
                before = self.snapshot()
                with (
                    redirect_stdout(io.StringIO()),
                    redirect_stderr(io.StringIO()),
                    pytest.raises(SystemExit) as raised,
                ):
                    self.install.main([str(self.project), *arguments])
                assert raised.value.code != 0
                assert self.snapshot() == before
                self.run_mock.assert_not_called()


if __name__ == "__main__":
    unittest.main()
