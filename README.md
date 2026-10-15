# Development tooling templates

Copy the configs you need into a project and combine the matching mise profiles.
The templates do not create application code or replace your project manifests.

## Installer

Run the root `install.py` from this repository with Python 3.14 or newer. It uses
only the standard library and targets an existing project. Select `python`,
`javascript` (`typescript` is an alias), `rust`, `lua`, or `markdown` with
`--tools`. Preview and config generation need no installed tooling beyond
Python. Installation flags require `mise` on `PATH`; installing Rust toolchains
also requires an existing `rustup` installation.

```sh
python3 install.py /Users/lasn/Projects/servicenow-platform-mcp --tools python
python3 install.py /Users/lasn/Projects/particle-foundry \
  --tools rust typescript python \
  --rust-dir engine --javascript-dir web \
  --python-mode scripts --python-files docs/scripts
```

The default is a preview: no files are written and no commands are executed,
even when installation flags are supplied. Append `--apply` to write the
selected configs, `.mise/conf.d/tooling.toml`, managed blocks in the root mise
config and `AGENTS.md`, and the `/.tooling-backups/` ignore entry in `.gitignore`.

Language task implementations live in `.mise/conf.d/tooling.toml`. The installer
uses `templates/mise/mise.toml` as the source for the central root block. It
chooses an existing `.mise.toml` if present, otherwise `mise.toml`; it does not
create a new `mise.toml` when `.mise.toml` exists. It appends or updates the
managed block without replacing existing user content, tools, or tasks. The
root aggregates `check`, `format`, `test`, and `deps` have explicit dependencies
on selected, available language tasks. Aggregates with no dependencies are
omitted; wildcard dependencies are not used, avoiding cycles through the
aggregates themselves. Existing project tasks with those names are kept and
reported instead of replaced.

Installation is optional and requires `--apply` to execute:

- `--install-tools` trusts the project's mise config and runs `mise install`,
  plus Rustup toolchain installation when Rust is selected.
- `--install-deps` separately installs project dependencies, pytest for Python
  project mode, and JavaScript/Markdown dependencies. It backs up current
  dependency manifests and lockfiles before running commands, which can change
  those files and run dependency build scripts. Backups do not provide a full
  rollback of build-script side effects or environment changes.

For configuration only, append `--apply` to either example. If installation is
also wanted, append `--apply --install-tools --install-deps` instead. Review the
preview first: installation can download packages and execute mise hooks or
project and third-party code.

The selected root mise file gains a managed block; existing user text before
and after that block is preserved. Other existing mise files stay untouched,
and higher-precedence settings can override the generated fragment. Existing
tool pins and configs, including tool settings in `pyproject.toml`, are kept by
default.

To replace tool configs, append `--apply --overwrite` to either example
(`--replace-configs` is an alias for `--overwrite`). Backups go under
`.tooling-backups/<timestamp>-<UUID>/`. Obsolete alternative
configs may be removed after backup. Incompatible Prettier JavaScript or TOML
configs are backed up and removed, then a canonical `.prettierrc.json` is
created. JSON is never written into a JavaScript or TOML config file. An
embedded `prettier` field in `package.json` is removed after backing up the
manifest so the canonical JSON config takes effect; other manifest values are
preserved. `--overwrite` alone preserves existing Rust compiler pins; changing
one also requires `--overwrite-rust-toolchain`.

Instructions always go to `AGENTS.md` by default. Use `--agents-file AGENS.md`
only if that literal filename is intended. Reruns replace only the block between
`<!-- dev-tooling:start -->` and `<!-- dev-tooling:end -->` idempotently, keeping
all text outside it. Supply the complete tool selection on each run: the
language fragment, central mise block, and managed instructions are regenerated.
Deselecting a language does not delete its old tool configs.

Language directories such as `--rust-dir` and `--javascript-dir` are relative to
the target project; `--python-files` paths are relative to `--python-dir`.
The installer rejects path traversal and symlinked paths within the target.

Python auto mode recognizes a project from `[project]` or `[tool.uv.workspace]`
in `pyproject.toml`; otherwise it uses scripts mode. `--python-mode scripts`
provides linting, formatting, and type checking without creating a uv project
or pytest/dependency tasks. `--python-version` controls the lint/type-check
minimum, not the runtime pin. Existing pytest markers and coverage settings are
preserved; quiet test flags do not suppress coverage output.

JavaScript tasks use existing package scripts. If `typecheck` or `test` is
missing, they delegate to the matching bare root mise task and its prerequisites
unless that would create a cycle; otherwise they are omitted. The installer
never invents package scripts. JavaScript and Markdown honor the existing
`packageManager` and lockfile. Workspace packages can inherit the root
`packageManager`, and dependency backups include shared root lockfiles.
Package-manager and workspace detection reads metadata directly without
executing commands. When combining JavaScript and Markdown in a fresh setup,
use `--node-version` to resolve their conflicting default Node versions.
Existing tool pins are kept.

## Available configs

Destinations are relative to the directory where the tool runs. For a monorepo,
use the package directory selected by the profile's `*_dir` variable.

| Tool             | Template                                | Destination          | Mise profile      |
| ---------------- | --------------------------------------- | -------------------- | ----------------- |
| Biome            | `templates/biome/biome.json`            | `biome.json`         | `javascript.toml` |
| StyLua           | `templates/stylua/stylua.toml`          | `stylua.toml`        | `lua.toml`        |
| StyLua ignores   | `templates/stylua/.styluaignore`        | `.styluaignore`      | `lua.toml`        |
| Markdownlint     | `templates/markdown/.markdownlint.json` | `.markdownlint.json` | `markdown.toml`   |
| Prettier         | `templates/markdown/.prettierrc.json`   | `.prettierrc.json`   | `markdown.toml`   |
| Prettier ignores | `templates/markdown/.prettierignore`    | `.prettierignore`    | `markdown.toml`   |
| Ruff             | `templates/python/ruff.toml`            | `ruff.toml`          | `python.toml`     |
| ty               | `templates/python/ty.toml`              | `ty.toml`            | `python.toml`     |

| Rust toolchain | `templates/rust/rust-toolchain.toml` | `rust-toolchain.toml` | `rust.toml` |
| rustfmt | `templates/rust/rustfmt.toml` | `rustfmt.toml` | `rust.toml` |
| Clippy | `templates/rust/clippy.toml` | `clippy.toml` | `rust.toml` |

## Combining mise profiles

`templates/mise/mise.toml` is the central aggregate template. Language profiles
live in `templates/mise/profiles/`. To copy them manually:

1. Use an existing `.mise.toml` for the central root config, otherwise
   `mise.toml`. Copy `templates/mise/mise.toml` only for a new config. For an
   existing file, merge missing aggregate definitions without replacing user
   content, tools, or tasks.
2. Merge the selected profiles' `[tools]`, `[vars]`, and `[tasks.*]` entries into
   `.mise/conf.d/tooling.toml`. Do not concatenate whole files: repeated TOML
   table headers are invalid.
3. In the root template, keep only explicit dependencies on selected, available
   language tasks for `check`, `format`, `test`, and `deps`. Keep existing project
   tasks with those names. Omit aggregates with no dependencies. Do not use
   wildcard dependencies, which can include the aggregates themselves and
   create cycles.
4. Resolve shared tool versions explicitly. JavaScript currently selects Node
   `26`, while Markdown selects `24.15.0`; choose one version for the project.
   Preserve existing project pins unless intentionally changing them.
5. Copy the selected tool configs from the table above. Merge existing settings
   or back up configs before replacing them. Include the dotfiles; a shell `*`
   glob usually skips them. Manual copying does not provide the installer's
   backups or managed updates. For those, use `--apply --overwrite`; changing an
   existing Rust compiler pin also requires `--overwrite-rust-toolchain`.
6. Adjust the `*_dir` and `*_files` variables to match the project layout.
7. Review the resulting configuration, then run `mise trust` and `mise install`
   in the target project only when installation is wanted. Installation can
   download tools and execute project or third-party code.

Mise does not automatically discover templates in this repository's
`templates/` directory. After copying, the root config provides the aggregates
and `.mise/conf.d/tooling.toml` supplies the language-scoped implementations.

For reproducible CI runs, replace floating tool versions such as `latest`, `2`,
or `stable` with exact versions and commit the project's dependency lockfiles.

## JavaScript and TypeScript

Biome handles linting, formatting, and import organization using its recommended
lint rules. The config uses two-space indentation, an 88-character line width,
and Git ignore files. Create a `.gitignore` in the package directory before
running checks; the enabled VCS integration requires it. Set `vcs.enabled` to
`false` for projects without Git integration. Its schema targets Biome 2.5.15;
keep the schema URL in step with the Biome version selected for a project.

The mise profile checks all Biome-supported files beneath `javascript_files`.
Biome ignores unknown file types. Keep build output and generated files in the
project's `.gitignore`.

`deps:javascript` expects an existing `package.json` and `bun.lock`. Run
`bun install` once when setting up a new package, then commit the lockfile.
Define project-specific `typecheck` and `test` scripts in `package.json`; the
profile delegates to them. A TypeScript project's `tsconfig.json` depends on
its runtime or framework and is intentionally left to that project.

```sh
mise run deps:javascript
mise run check:javascript
mise run format:javascript
mise run typecheck:javascript
mise run test:javascript
```

`format:javascript` only formats. To apply safe lint fixes and organize imports,
run `biome check --write .` in the JavaScript package directory, or substitute
the paths selected by `javascript_files`.

## Lua

The StyLua config selects Lua 5.1 syntax to match the profile, four-space
indentation, and a 100-character line width. Change `syntax` if a project uses
LuaJIT, Luau, or another Lua version. Require sorting stays disabled because
reordering module initialization can affect behavior.

StyLua uses `.styluaignore` during directory traversal. Explicit file arguments
bypass ignores unless you also pass `--respect-ignores`.

```sh
mise run check:lua
mise run format:lua
```

## Markdown

Install these dependencies in the target project's existing package, or create
a `package.json` first for a documentation-only project:

```sh
bun add --dev --exact markdownlint-cli prettier
```

Use `markdownlint-cli`, which provides the `markdownlint` executable expected by
the profile. `markdownlint-cli2` has different flags and is not a drop-in
replacement. Bun can run the local executables directly; no wrapper scripts
are required. Commit `package.json` and `bun.lock`, and use
`bun install --frozen-lockfile` in CI.

Prettier preserves existing prose wrapping and uses an 88-character print width
where applicable. Markdownlint's line-length rule is disabled so it does not
reject long links or paragraphs that Prettier leaves intact. Repeated headings
are allowed under different parents, and ordered lists use sequential numbers.

Keep Prettier scoped to Markdown when combining this profile with Biome. The
provided overrides configure Markdown formatting; they do not stop a broad
`prettier .` command from formatting other languages.

The tasks select `**/*.md` by default. Change `markdown_files` to
`'**/*.{md,markdown}'` if you also use `.markdown` files. Both check commands use
`.gitignore`, which must exist in `markdown_dir`; Prettier additionally uses
`.prettierignore`. The explicit `--ignore-path .gitignore` means Markdownlint
will not read a separate `.markdownlintignore`.

```sh
mise run check:markdown
mise run format:markdown
```

## Python

The Python profile runs Ruff and ty.

The Ruff config uses a 120-character line limit and a strict lint selection.
Alongside the existing rules, it enables debugger detection (`T10`), broad
exception checks (`BLE`), Pylint errors (`PLE`), suppression and mock assertion
checks (`PGH`), logging rules (`LOG`), async checks (`ASYNC`), modern Python
idioms (`FURB`), conventional import aliases (`ICN`), and type annotation
requirements (`ANN`). Selected security checks cover explicit HTTP timeouts
(`S113`), certificate verification (`S501`), YAML loading (`S506`), and SQL
string construction (`S608`). Performance checks target unnecessary list
allocation (`PERF101`), dictionary iteration (`PERF102`), and manual list
copying (`PERF402`).

Preview rules stay disabled. Formatter-conflicting rules `COM812`, `COM819`,
and `W191` are ignored. `E501` remains enabled: the formatter can leave some
lines longer than 120 characters, which may need manual adjustment.

This preset requires annotations on function arguments and return values,
including tests, and rejects explicit `Any` annotations through `ANN401`.
`LOG015` requires named loggers instead of root-logger calls. Keep the existing
test-specific ignores when adapting the config. Security checks are static
heuristics, so review findings in context; they do not replace a security
audit.

The lint/type-check target is Python 3.14, while the current mise runtime is
3.15. Keep `python_version`, Ruff's `target-version`, and ty's `python-version`
setting consistent with the versions the project supports.
Check the project's `requires-python` and any `.python-version` file when
choosing its runtime.

In an existing uv project, add pytest and any test plugins to the dev group:

```sh
uv add --dev pytest
```

For a new project, run `uv init` first. Commit `pyproject.toml` and `uv.lock`.
Ruff and ty run directly using the binaries installed by mise; they do not
need to be duplicated in the project's dev dependencies. The test task runs
`uv run pytest -q --tb=short` so pytest can access the project's dependencies
and test plugins. Quiet mode reduces session and progress output; short
tracebacks keep failures concise. Warnings and the final summary remain
visible, and all tests still run. Run `deps:python` before type checking so the
configured project interpreter and dependencies exist.

The default paths are `src tests`, and the default interpreter is
`.venv/bin/python`, relative to `python_dir`. Create those directories or adjust
the variables. On Windows, set `python_executable` to `.venv/Scripts/python.exe`.

```sh
mise run deps:python
mise run check:python
mise run typecheck:python
mise run format:python
mise run fix:python
mise run test:python

```

`format:python` formats code; `fix:python` separately applies safe lint fixes.
The standalone tool configs avoid having to replace or merge a project's
`pyproject.toml`. If you already keep tool settings there, merge them instead
of maintaining competing configurations.

## Rust

Rustup owns the compiler version and installs rustfmt and Clippy through
`rust-toolchain.toml`. The template selects `stable`; replace it with an exact
release when reproducible compiler versions matter. Keep an existing project's
toolchain selection if it already has one.

rustfmt uses four-space indentation and a 100-character line width. The config
does not override the Rust edition; Cargo supplies it from the project's
manifest when running `cargo fmt`.

`clippy.toml` configures lint behavior and thresholds. It does not enable lint
groups; use the project's `Cargo.toml` lint tables for additional rules. The
mise task treats warnings as errors with `-D warnings`.

Create the crate or workspace first with Cargo, and generate and commit
`Cargo.lock` before using the locked tasks. These templates do not replace
`Cargo.toml` or set a minimum supported Rust version.

```sh
mise run check:rust
mise run format:rust
mise run test:rust
```

## Checking this repository

With Python 3.14 or newer and pytest available in that environment:

```sh
python3 -m unittest discover -s tests -v
```

The tests cover both the installer and templates. Installer tests use temporary
project fixtures and mocked commands to check previews, writes, preservation,
reruns, installation plans, and path protection. Template tests parse every
TOML/JSON config, check mise task dependencies, and verify that Python target
versions agree. They do not install tools or run commands against real target
projects.
