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

Your project must already exist. The installer configures development tools;
it does not create an application, a Cargo workspace, or a JavaScript package.

## Get started

You need [Rust installed through rustup](https://rust-lang.org/tools/install/)
to build the installer, and [mise](https://mise.jdx.dev/getting-started.html)
to install tools and run the generated tasks. Both must be available in your
terminal.

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
`mise run format:javascript` formats files. For Biome's safe lint fixes and
import organization, run `mise exec -- biome check --write .` from the
JavaScript directory. Keep generated files in that directory's `.gitignore`.

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

### Lua and Markdown

The Lua preset uses Lua 5.1 syntax, four-space indentation, and a 100-character
line width. Adjust `syntax` in `stylua.toml` for another Lua dialect.

Markdown tools are installed globally through mise. They need no `package.json`
and add no project dependencies. The generated tasks format Markdown files
only and respect Git ignore rules. Existing markdownlint-cli2 configs keep
their own ignore settings.

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
