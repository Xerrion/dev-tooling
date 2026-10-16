# Xerrion dev tooling

Set up linting, formatting, type checks, and test commands in an existing
project. Choose the languages you use, preview the changes, then apply them.
The installer creates tool configs and [mise](https://mise.jdx.dev/getting-started.html)
tasks that you can run from your project root.

| Selection                    | Tools                       | What you get                                                            |
| ---------------------------- | --------------------------- | ----------------------------------------------------------------------- |
| `javascript` or `typescript` | Biome                       | Linting and formatting, plus your existing type-check and test commands |
| `python`                     | Ruff, ty, uv                | Linting, formatting, type checking, and pytest for Python projects      |
| `rust`                       | rustfmt, Clippy, Cargo      | Formatting, linting, and workspace tests                                |
| `lua`                        | StyLua                      | Formatting checks and formatting                                        |
| `markdown`                   | markdownlint-cli2, Prettier | Markdown linting and formatting                                         |
| `shell` or `shellcheck`      | ShellCheck                  | Shell script linting                                                    |
| `docker` or `hadolint`       | Hadolint, Docker            | Dockerfile linting, image builds, and container runs                    |

Your project must already exist. The installer configures development tools;
it does not create an application, a Cargo workspace, or a JavaScript package.

## Get started

You need [Rust installed through rustup](https://rust-lang.org/tools/install/)
to build the installer, and [mise](https://mise.jdx.dev/getting-started.html)
to install tools and run the generated tasks. Both must be available in your
terminal.

Start interactive setup from this repository:

```sh
./installer/target/release/installer /path/to/your/project
```

Build the binary first using step 1 below. The installer detects supported
tooling, shows the matching files, and lets you change the selection and each
working directory. It asks separately about installing tools and syncing
dependencies, then displays the plan before asking whether to apply it.
Press Enter at the final prompt to keep the preview without writing files.
Omit the project path to use the current directory.

Setup has four steps: tooling, directories, installation preferences, and
review. On interactive terminals, use the arrow keys to move through the
tooling checklist, Space to toggle a tool, the right arrow to select all, and
the left arrow to clear the selection. Type to filter the list, Enter to
continue, or Esc to cancel. Detected tools start selected, with matching files
shown beside them.

Piped input and terminals without interactive support use a numbered list.
Enter numbers or tool names separated by spaces, or type `all`. Invalid answers
and directory paths can be corrected at the same prompt. Press Esc to cancel in
interactive terminals; type `q` to cancel numbered prompts.

The review groups new, updated, and removed files and lists installation
commands separately. Choose Show plan details to inspect all plan notes, Apply
setup to write changes, or Keep preview to exit without writing. Keep preview
is selected by default. Output respects `NO_COLOR` and `CLICOLOR=0`; redirected
output contains no terminal control codes.

These examples use Markdown, which needs no project dependencies. Replace
`/path/to/your/project` with your project's directory.

1. Build the installer from this repository:

   ```sh
   cd installer
   cargo build --release --locked
   cd ..
   ```

   Building inside `installer/` uses its configured Rust toolchain, currently
   nightly. The compiled installer itself needs no Python interpreter or Cargo.

2. Preview the setup, including tool installation:

   ```sh
   ./installer/target/release/installer /path/to/your/project \
     --tools markdown --install-tools
   ```

   The output lists planned file changes and commands to run.
   Preview mode writes no files and runs no installation commands.

3. Apply the setup:

   ```sh
   ./installer/target/release/installer /path/to/your/project \
     --tools markdown --install-tools --apply
   ```

   `--install-tools` trusts the project's mise configuration and installs its
   tools. This can download packages and run mise hooks. To write configuration
   without installing tools, omit `--install-tools`.

4. Run the checks in your project:

   ```sh
   cd /path/to/your/project
   mise run check
   ```

   Review the generated files and commit the configuration you want to share
   with your team.

Keep the binary with this repository's `templates/` directory. If you move it,
pass `--templates /path/to/templates` when running it.

## Download a release

Download the archive for Linux x64, macOS Intel, macOS Apple Silicon, or Windows
x64 from [Releases](https://github.com/Xerrion/xerrion-dev-tooling/releases).
Extract it into an empty directory. The archive contains only `installer`
(`installer.exe` on Windows) at its root and the required `templates/` directory,
including hidden configuration files. Keep them together. Rust is not needed
to run the downloaded installer.

From the extracted directory, preview a Markdown setup:

```sh
./installer /path/to/your/project --tools markdown
```

In Windows PowerShell, use `./installer.exe`. Add `--apply` to write the setup.
Install mise to install tools and run the generated tasks. `SHA256SUMS` on the
release contains checksums for all four archives.

## Publish a release

1. Merge the release workflow and the changes you want to release into `main`.
   Set `version` in `installer/Cargo.toml` and update `installer/Cargo.lock` when
   changing the version.
2. Open **Actions > Release > Run workflow**, select `main`, and enter a new tag
   matching the Cargo version, such as `v0.1.0`. Existing tags are rejected.
3. Run the workflow. It runs `mise run check` and `mise run test` on Linux, builds
   all four platforms with `mise run build:installer:release`, and tests preview
   and apply from each extracted archive before publishing
   the release, its archives, and checksums. Tags containing a prerelease suffix,
   such as `v0.2.0-rc.1`, create a prerelease.

The release tag points to the selected workflow commit. A failed build or package
test prevents publication. GitHub also supplies its standard source archives;
use the platform archives to get the compiled installer.

## Choose languages and directories

Run the installer examples below from this repository's root. Run mise tasks
from your target project's root.

Select several languages in one command. For a repository with a Rust workspace
in `engine/`, a web application in `web/`, and documentation in `docs/`:

```sh
./installer/target/release/installer /path/to/your/project \
  --tools rust typescript markdown \
  --rust-dir engine --javascript-dir web --markdown-dir docs
```

This is a preview. Add `--apply` when you are ready. Add `--install-tools` if
the tools also need installing. Rust toolchain installation requires rustup.
Include every language you want to keep each time you run the installer.

Directories are relative to the target project and must already exist. A
language uses the project root unless you specify a directory or have an
existing mise directory setting. The generic form, such as `--dir rust=engine`,
is also available. Paths containing `..` or symlinks within the target are
rejected.

To see the available selections and options:

```sh
./installer/target/release/installer --list-tools
./installer/target/release/installer --help
```

### Automatic stack detection

For scripts and CI, use `--detect` to select tooling from project files without
prompts. It previews by default; add `--apply` to write the configuration:

```sh
./installer/target/release/installer /path/to/your/project --detect
```

Detection uses the following catalog rules, in a fixed order:

| Selection    | Manifest or filename matches                                              | Source extensions used when no manifest matches                       |
| ------------ | ------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| `javascript` | `package.json`, `tsconfig.json`                                           | `js`, `jsx`, `mjs`, `cjs`, `ts`, `tsx`, `mts`, `cts`, `svelte`, `vue` |
| `python`     | `pyproject.toml`, `setup.py`, `setup.cfg`, `requirements*.txt`, `Pipfile` | `py`                                                                  |
| `rust`       | `Cargo.toml`                                                              | None                                                                  |
| `docker`     | `Dockerfile`, `Dockerfile.*`, `Containerfile`, `Containerfile.*`          | None                                                                  |
| `lua`        | None                                                                      | `lua`                                                                 |
| `markdown`   | None                                                                      | `md`, `markdown`                                                      |
| `shell`      | None                                                                      | `sh`, `bash`, `bats`                                                  |

The scan sorts paths and selections. It matches filenames without executing
project code or checking installed tools. Files containing only generated
tooling instructions are excluded so reruns keep the same Markdown directory.
It skips symlinks, hidden directories,
and directories named `node_modules`, `target`, `vendor`, `dist`, `build`,
`coverage`, `__pycache__`, or `venv`. Other Git ignore patterns do not affect
detection. A scan exceeding 32 directory levels or 100,000 files stops with an
error; use explicit `--tools` selections for such projects.

A manifest directory takes precedence over matching manifests below it. Without
a manifest, the nearest shared parent of the matching source files becomes the
working directory. Independent manifest directories require a choice through
`--interactive` or an explicit directory option such as `--javascript-dir web`.
Existing mise directory and file settings take precedence over detection.

Detected shell tasks list every matching script inside the selected directory,
including nested scripts. Detected Docker tasks lint all matching Dockerfiles
inside that directory. Builds prefer `Dockerfile`, then `Containerfile`, then
the first matching path in sorted order. Override `docker_file` to choose
another build file. Detection does not inspect Docker Compose files.

Explicit `--tools` selections bypass detection and remain suitable for scripts.
Without a terminal, omitting both `--tools` and `--detect` fails instead of
waiting for input. Use `--interactive` to request prompts explicitly, including
with piped input. Add `--dry-run` to suppress the final apply prompt and keep
the setup as a preview. `--apply` explicitly authorizes applying the plan.

## Run your daily checks

Run these commands from your project's root:

| Command           | Action                                            |
| ----------------- | ------------------------------------------------- |
| `mise run check`  | Check lint, formatting, and available type checks |
| `mise run format` | Format files for the selected languages           |
| `mise run test`   | Run the available test suites                     |
| `mise run deps`   | Sync project dependencies where supported         |
| `mise tasks ls`   | Show the tasks available in your project          |

Tasks with no applicable work are omitted. For example, a Markdown-only project
has no generated `test` or `deps` task. Existing tasks with these names are
preserved, so check `mise tasks ls` if your project already defines them.

To work on one language, use its task name:

```sh
mise run check:rust
mise run format:markdown
mise run test:python
```

Formatting writes files. Review the changes before committing them.

### JavaScript and TypeScript

The installer uses the package manager declared in `package.json` or detected
from a lockfile, including workspace settings. It supports npm, pnpm, Yarn,
and Bun; Bun is the fallback when no package manager is detected.

Create `package.json` before installing project dependencies. Add `typecheck`
and `test` scripts for your application, or keep existing root mise tasks with
those names. If neither is available, the corresponding task is omitted with
a warning. TypeScript configuration stays with your application.

Biome's preset uses two-space indentation and an 88-character line width.
`mise run format:javascript` formats files and applies Biome's safe lint fixes,
including import organization. `mise run check:javascript` reports issues
without changing files. Keep generated files in that directory's `.gitignore`.

### Python projects and scripts

An existing `pyproject.toml` with `[project]` or `[tool.uv.workspace]` enables
project mode. This adds uv dependency tasks and a pytest test task alongside
Ruff and ty. Sync the environment before running type checks or tests.

To install project dependencies as part of setup, preview this command, then
repeat it with `--apply`:

```sh
./installer/target/release/installer /path/to/your/project \
  --tools python --install-tools --install-deps
```

`--install-deps` can add pytest to the dev dependencies and update
`pyproject.toml` and `uv.lock`. It also installs JavaScript dependencies when
JavaScript is selected. Existing manifests and lockfiles are backed up before
these commands run. Dependency installation can execute package build scripts.

For standalone scripts, use scripts mode:

```sh
./installer/target/release/installer /path/to/your/project \
  --tools python --python-mode scripts --python-files scripts
```

Scripts mode adds Ruff and ty checks without creating a uv project or pytest
task. It is selected automatically when Python project metadata is absent.
`--python-files` paths are relative to `--python-dir` when that directory is set.

The Ruff preset is strict, including function annotations, and uses a
120-character line limit. `mise run fix:python` applies safe Ruff fixes.
`--python-version` sets the language version used for linting and type checking;
the Python runtime version is a separate mise setting. Existing Ruff and ty
configs are preserved, so adjust their version settings too when needed.

### Rust

Create your crate or workspace and its `Cargo.lock` before running the Rust
tasks. Checks and tests use Cargo's locked workspace mode. Clippy warnings cause
the check to fail.

Existing toolchain pins are preserved. A project without a toolchain file gets
one selecting nightly with rustfmt and Clippy. Pin an exact compiler version in
`rust-toolchain.toml` when you need reproducible builds.

### Shell scripts

Choose `shell` to add ShellCheck. To check scripts in a `scripts/` directory,
preview this command, then repeat it with `--apply`:

```sh
./installer/target/release/installer /path/to/your/project \
  --tools shell --shell-dir scripts --install-tools
```

The profile adds `mise run check:shell` to the shared `mise run check` command.
`shellcheck` is also accepted as a selection name.

The default file pattern is `*.sh`, relative to `shell_dir`. It checks files
directly in that directory. To include nested paths or scripts without a `.sh`
extension, set `shell_files` in your root mise configuration:

```toml
[vars]
shell_files = "*.sh lib/*.sh deploy"
```

Keep a shebang such as `#!/bin/sh` or `#!/usr/bin/env bash` in each script so
ShellCheck can select its dialect. Existing `.shellcheckrc` settings are used.
This profile adds linting only; it has no formatter or test task.

### Docker

Preview Dockerfile linting with [Hadolint](https://github.com/hadolint/hadolint):

```sh
./installer/target/release/installer /path/to/your/project \
  --tools docker --install-tools
```

Add `--apply` to write the setup. The profile adds `mise run check:docker` to
the shared `mise run check` command. `hadolint` is also accepted as a selection
name. Hadolint runs directly, so a Docker daemon is not required.

By default, it checks `Dockerfile` in the project root. Use `--docker-dir docker`
or `--dir docker=docker` for another existing directory. To check several files,
set `docker_files` in your root mise configuration. Paths and patterns are
relative to `docker_dir`; quote paths containing spaces inside the value:

```toml
[vars]
docker_files = "Dockerfile Dockerfile.dev 'images/api service/Dockerfile'"
```

Hadolint uses existing `.hadolint.yaml` or `.hadolint.yml` settings. It has no
formatter or test task.

To build and run your application, install Docker separately and start its
daemon, then run these tasks from your project root:

```sh
mise run build:docker
mise run run:docker
```

`build:docker` builds `Dockerfile` with the current directory as its context and
tags the image `app:local`. `run:docker` builds the image first, then runs it in
the foreground and removes the container when it exits. Both tasks run inside
`docker_dir`. Configure them in your root mise file, for example:

```toml
[vars]
docker_file = "Dockerfile.dev"
docker_context = "."
docker_image = "my-app:dev"
docker_build_args = "--build-arg NODE_ENV=development"
docker_run_args = "-p 3000:3000 --env-file .env"
```

`docker_file` selects one file for builds; `docker_files` selects files for
linting. Build file and context paths are relative to `docker_dir`. Docker
Compose validation is outside this profile's scope.

### Lua and Markdown

The Lua preset uses Lua 5.1 syntax, four-space indentation, and a 100-character
line width. Adjust `syntax` in `stylua.toml` for another Lua dialect.

Markdown tools are installed globally through mise. They need no `package.json`
and add no project dependencies. The generated tasks format Markdown files
only and respect Git ignore rules. Existing markdownlint-cli2 configs keep
the generated `.markdownlint-cli2.jsonc` combines rule settings and CLI2 options.

## What changes in your project

| File or directory                        | Purpose                                                                       |
| ---------------------------------------- | ----------------------------------------------------------------------------- |
| `.mise/conf.d/tooling.toml`              | Generated tool versions and language tasks                                    |
| `mise.toml` or an existing `.mise.toml`  | A marked section for shared commands such as `check` and `format`             |
| Tool configs in the selected directories | Settings such as `biome.json`, `ruff.toml`, or `rustfmt.toml`                 |
| `AGENTS.md`                              | A marked section listing the project's tooling commands for coding assistants |
| `.gitignore` and `.tooling-backups/`     | Ignore the backup directory and store originals when files change             |

Existing tool configs and version pins are kept by default, including supported
settings embedded in `pyproject.toml`. Text outside the marked sections in the
root mise file and `AGENTS.md` stays intact. Use `--agents-file path/to/file.md`
to choose a different instructions file.

### Replace existing configs

To adopt the supplied presets over your existing tool configs, preview with
`--overwrite`, then add `--apply`:

```sh
./installer/target/release/installer /path/to/your/project \
  --tools python markdown --overwrite
```

Changed configs are backed up under `.tooling-backups/run-<unique-id>/`, keeping
their project-relative paths. Alternative config files may be removed after
backup so the replacement takes effect. `--replace-configs` is an alias for
`--overwrite`.

Replacing an existing Rust toolchain pin requires both `--overwrite` and
`--overwrite-rust-toolchain`.

If you need to undo a change, compare the backup with the current file and copy
the original back to the same relative path. Newly created files have no backup.
An interrupted apply can leave some changes in place; backups do not undo
installed packages or build-script side effects.

### Customize files and versions

Edit tool configs directly. For mise overrides, use your project's root
`mise.toml` or `.mise.toml`, outside the generated section. For example, to
include both `.md` and `.markdown` files, add this to its `[vars]` table:

```toml
[vars]
markdown_files = "'**/*.{md,markdown}'"
```

Use the existing table if there is one. Changes made directly to
`.mise/conf.d/tooling.toml` are replaced when you rerun the installer.

Existing project settings take precedence. If an explicit directory or Python
option conflicts with a mise variable, update that variable in the project
config first. Use `mise config` and `mise tasks ls` to inspect the active setup.

## Add or update tooling later

Rerun the installer with the **complete selection** you want to keep. For
example, to add Markdown to a project already using Rust:

```sh
./installer/target/release/installer /path/to/your/project \
  --tools rust markdown --rust-dir engine
```

Keep the directory options that match your project. Review the preview, then
repeat with `--apply`. Selecting fewer languages removes their generated tasks
but leaves their tool config files in place.

For a tool that is not listed, add a TOML profile under
`templates/mise/profiles/` and a matching entry under `templates/catalog/`.
The profile defines versions and commands; the catalog connects it to the
installer and any config templates. The existing Lua files provide a small
example. These additions are loaded at runtime and need no Rust rebuild.
Add optional `[detection]` rules with `files` (filename glob patterns) and
`extensions` (without dots) to include the tool in automatic detection.
Special behavior such as detecting a new project format may require Rust changes.

## Working on this repository

To check changes to the installer or templates, run these commands from this
repository's root:

```sh
mise run check
mise run test
```

All installer and template tests run through Cargo. Python remains a supported
target language, but is not required to run this repository's tests.
