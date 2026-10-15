import importlib.util
import io
import json
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

    def assert_preserved(self, project: Path, files: dict) -> None:
        for relative, original in files.items():
            with self.subTest(preserved=relative):
                assert (project / relative).read_bytes() == original

    def python_targets(self, generated: dict) -> list[str]:
        targets = generated["vars"]["python_files"]
        return shlex.split(targets) if isinstance(targets, str) else targets

    def recorded_commands(self) -> list[list[str]]:
        commands = []
        for call in self.run_mock.call_args_list:
            command = call.args[0] if call.args else call.kwargs["args"]
            commands.append(shlex.split(command) if isinstance(command, str) else [str(part) for part in command])
        return commands

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
        assert not (self.project / "mise.toml").exists()
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
        assert self.backups(self.project, "mise.toml") == []
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

    def test_existing_pinned_rust_toolchain_is_not_replaced_even_with_replace_configs(
        self,
    ) -> None:
        originals = self.monorepo()
        self.succeed("--tools", "rust", "--rust-dir", "engine", "--replace-configs", "--apply")
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

    def test_replace_configs_rejects_unsupported_prettier_formats_before_writes(self) -> None:
        alternatives = (
            ("prettier.config.js", "export default { printWidth: 79 };\n"),
            ("prettier.config.cjs", "module.exports = { printWidth: 79 };\n"),
            (".prettierrc.js", "export default { printWidth: 79 };\n"),
            (".prettierrc.cjs", "module.exports = { printWidth: 79 };\n"),
            (".prettierrc.toml", "printWidth = 79\n"),
        )
        for index, (relative, text) in enumerate(alternatives):
            with self.subTest(config=relative):
                project = self.sandbox / f"unsupported-prettier-{index}"
                project.mkdir()
                self.write(project, "package.json", '{"private":true}\n')
                self.write(project, "AGENTS.md", "# Preserve local instructions\n")
                self.write(project, relative, text)
                self.refuse_without_writes("--tools", "markdown", "--replace-configs", "--apply", project=project)

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
                    assert canonical != existing and canonical.is_file(), "Replacement did not install a config"
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
        assert self.backups(self.project, "mise.toml") == []
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
                        if manager == "npm":
                            assert command[:2] == ["npm", "exec"] or command[0] == "npx", command
                        else:
                            assert command[:2] == [manager, "exec"], command
                commands = self.recorded_commands()
                assert commands, "--install-deps did not execute a dependency command"
                installation = list(commands)
                for command in commands:
                    if Path(command[0]).name == "mise" and "run" in command:
                        for argument in command[1:]:
                            if argument in tasks:
                                installation.extend(self.task_commands(tasks[argument]))
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

    def test_tool_only_pyproject_uses_scripts_auto_mode_without_dependency_commands(self) -> None:
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
                assert command[:2] != ["uv", "sync"]
        assert (self.project / "pyproject.toml").read_bytes() == manifest.encode()
        assert not (self.project / "ruff.toml").exists()
        assert not (self.project / "uv.lock").exists()
        self.run_mock.assert_not_called()

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
