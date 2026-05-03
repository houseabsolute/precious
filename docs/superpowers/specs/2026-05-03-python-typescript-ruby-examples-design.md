# Design: Python, TypeScript, and Ruby config init + examples

**Issue:** [#3 Examples](https://github.com/houseabsolute/precious/issues/3) **Date:** 2026-05-03

## Summary

Add `precious config init` support and `examples/` config files for Python, TypeScript, and Ruby.
These are the highest-value languages not yet covered.

## Scope

For each language:

1. Add a new `InitComponent` variant to `config_init.rs`
2. Add command constants and an `*_init()` function
3. Wire into `config_elements` match and `auto_or_component` file-extension detection
4. Generate `examples/<lang>/precious.toml` via `cargo run -- config init --component <lang>`

No extra generated files (no `.mypy.ini`, `.eslintrc`, `.rubocop.yml`). Tools work with defaults or
project-level config the user manages separately.

## Components

### Python

**Enum variant:** `Python` **Auto-detect extensions:** `py` **Excludes:** `.venv`, `__pycache__`,
`dist`, `build`

| Command       | Type | Invocation                                         |
| ------------- | ---- | -------------------------------------------------- |
| `ruff-check`  | both | `ruff check` (lint), `ruff check --fix` (tidy)     |
| `ruff-format` | both | `ruff format --check` (lint), `ruff format` (tidy) |
| `mypy`        | lint | `mypy .` — `invoke = "once"`, `path-args = "none"` |

mypy is invoked once against the whole project because per-file type checking misses cross-module
inference.

**Tool URLs:**

- https://docs.astral.sh/ruff/
- https://mypy.readthedocs.io/

### TypeScript

**Enum variant:** `Typescript` (single word, so clap maps it to `--component typescript` not
`--component type-script`) **Auto-detect extensions:** `ts`, `tsx`, `js`, `jsx` **Excludes:**
`node_modules`, `dist`, `build` **Include pattern:** `**/*.{ts,tsx,js,jsx}`

| Command               | Type | Invocation                                                      |
| --------------------- | ---- | --------------------------------------------------------------- |
| `eslint`              | both | `./node_modules/.bin/eslint` (lint), `--fix` (tidy)             |
| `prettier-typescript` | both | `./node_modules/.bin/prettier --check` (lint), `--write` (tidy) |

Uses local `node_modules` binaries, consistent with the existing `prettier-markdown` and
`prettier-yaml` commands.

**Tool URLs:**

- https://eslint.org/
- https://prettier.io/

### Ruby

**Enum variant:** `Ruby` **Auto-detect extensions:** `rb` **Excludes:** `.bundle`, `vendor/bundle`

| Command   | Type | Invocation                            |
| --------- | ---- | ------------------------------------- |
| `rubocop` | both | `rubocop` (lint), `rubocop -A` (tidy) |

`-A` auto-corrects all offenses including unsafe ones, which applies Layout cops for formatting.
rubocop is the de facto standard for Ruby linting and formatting.

Config will include comments noting common extension gems (`rubocop-performance`, `rubocop-rspec`,
`rubocop-rails`) that rubocop auto-loads when present in the bundle.

**Tool URLs:**

- https://rubocop.org/

## File changes

- `precious-core/src/config_init.rs` — all logic changes
- `examples/python/precious.toml` — generated via `cargo run`
- `examples/typescript/precious.toml` — generated via `cargo run`
- `examples/ruby/precious.toml` — generated via `cargo run`

## Out of scope

- Extra generated config files (`.mypy.ini`, `eslint.config.mjs`, `.rubocop.yml`)
- Other languages (Java, C/C++, Kotlin, PHP) — follow-up issue
- Sample projects with CI in separate repos — separate issue
