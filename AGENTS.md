<!-- xerrion-dev-tooling:start -->
## Development tooling

Run these mise tasks from the repository root. Existing project instructions and prerequisite tasks still apply.
Review formatting and safe-fix diffs; do not rewrite unrelated files or change tool versions as part of a routine check.

### Python
- After Python edits, check Ruff lint and formatting. Use `mise run lint:python`.
- When Python formatting needs correction, run the formatter. Use `mise run format:python`.
- When applying safe Ruff lint fixes, review the resulting diff. Use `mise run fix:python`.
- After Python API, annotation, import, or dependency changes, check types with ty. Use `mise run typecheck:python`.
- This is script-only Python tooling. Do not create a uv project or pytest suite just to run these checks; retain the project's existing script setup and validation tasks.

Tool versions, variables, and tasks in higher-precedence mise files override the generated fragment. Check `mise config` and `mise tasks ls` when behavior differs.
Run installation or dependency-sync commands only when needed; they can download packages or execute dependency build scripts.
<!-- xerrion-dev-tooling:end -->

<!-- dev-tooling:start -->
## Development tooling

Run these mise tasks from the repository root. Existing project instructions and prerequisite tasks still apply.
Review formatting and safe-fix diffs; do not rewrite unrelated files or change tool versions as part of a routine check.

### Python
- After Python edits, check Ruff lint and formatting. Use `mise run lint:python`.
- When Python formatting needs correction, run the formatter. Use `mise run format:python`.
- When applying safe Ruff lint fixes, review the resulting diff. Use `mise run fix:python`.
- After Python API, annotation, import, or dependency changes, check types with ty. Use `mise run typecheck:python`.
- This is script-only Python tooling. Do not create a uv project or pytest suite just to run these checks; retain the project's existing script setup and validation tasks.

Tool versions, variables, and tasks in higher-precedence mise files override the generated fragment. Check `mise config` and `mise tasks ls` when behavior differs.
Run installation or dependency-sync commands only when needed; they can download packages or execute dependency build scripts.
<!-- dev-tooling:end -->
