# UTF-8 Path Handling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make precious fail fast with a clear error whenever it encounters a path that is not valid
UTF-8, instead of silently lossy-converting.

**Architecture:** Adopt `camino::Utf8PathBuf` / `Utf8Path` throughout `precious-core` so the UTF-8
invariant is enforced by the type system. Fallible `Path → Utf8Path` conversions happen only at the
four entry points (filesystem walk, `git ls-files -z`, `git rev-parse --show-toplevel`, derived
paths from `canonicalize`/`pathdiff`). CLI args are validated by clap. Config TOML strings are
already UTF-8.

**Tech Stack:** Rust, `camino` (with `serde`), existing crates (`ignore`, `clap`, `anyhow`,
`thiserror`, `mitsein`, `clean-path`, `pathdiff`).

**Source of truth:** `docs/superpowers/specs/2026-05-30-utf8-path-handling-design.md`. Read it
before starting any task.

---

## Pre-flight

Verify the working directory is the `utf8-path-handling` worktree:

```bash
git rev-parse --show-toplevel
# Expected: /home/autarch/.claude/worktrees/precious/utf8-path-handling
git branch --show-current
# Expected: claude/utf8-path-handling
```

To run the full check (used after every task):

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

To satisfy the pre-commit hook (do not bypass it):

```bash
mise exec precious -- precious tidy <paths>
```

---

## File Structure

- **Create:**
  - `precious-core/src/paths/utf8.rs` — `NonUtf8Source`, `NonUtf8PathError`, `Display`.
  - `precious-integration/tests/non_utf8_paths.rs` — end-to-end gated tests.
- **Modify:**
  - `Cargo.toml` (workspace) — add `camino` with `serde` feature.
  - `precious-core/Cargo.toml` — depend on `camino`.
  - `precious-core/src/paths.rs` — `pub mod utf8;`.
  - `precious-core/src/paths/matcher.rs` — `Path → Utf8Path`.
  - `precious-core/src/paths/finder.rs` — internal types, four validation sites, `-z` git
    invocation, error variants update, sig change to `Finder::files`.
  - `precious-core/src/command.rs` — drop `to_string_lossy`; consume `Utf8Path`;
    `Utf8PathBuf::try_from` around `pathdiff`/`canonicalize`; switch fs bridge to `as_std_path()`.
  - `precious-core/src/config.rs` — `WorkingDir::ChdirTo(Utf8PathBuf)`.
  - `precious-core/src/config_init.rs` — `ConfigInitFile.path: Utf8PathBuf`, `extra_files` keys,
    `FileExists.path`, the `clap` `path` arg.
  - `precious-core/src/precious.rs` — CLI `Vec<Utf8PathBuf>`, `--config Utf8PathBuf`, internal
    types; drop residual `to_string_lossy`.
  - `precious-helpers/src/exec.rs` — add a `stdout_bytes()` accessor (raw `Vec<u8>`) on `Output` so
    callers can read the unmodified `git ls-files -z` stdout.

---

## Task 1: Foundation — `camino` dependency and the `NonUtf8PathError` error type

This task only touches new code plus `Cargo.toml`; nothing else compiles against it yet.

**Files:**

- Modify: `Cargo.toml` (workspace deps), `precious-core/Cargo.toml`.
- Modify: `precious-core/src/paths.rs:1-3`.
- Create: `precious-core/src/paths/utf8.rs`.

- [ ] **Step 1: Add `camino` to the workspace**

In `Cargo.toml` workspace dependencies section, add:

```toml
camino = { version = "1.1", features = ["serde1"] }
```

In `precious-core/Cargo.toml` dependencies, add `camino.workspace = true`.

Run `cargo check -p precious-core` and confirm the dependency resolves.

- [ ] **Step 2: Write the failing unit test for `NonUtf8PathError` display**

Create `precious-core/src/paths/utf8.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[cfg(unix)]
    #[test]
    fn display_filesystem_walk() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::path::PathBuf;

        let raw = PathBuf::from(OsStr::from_bytes(b"data\xff.bin"));
        let err = NonUtf8PathError {
            raw,
            source: NonUtf8Source::FilesystemWalk,
        };
        assert_eq!(
            err.to_string(),
            r#"non-UTF-8 path from filesystem walk: "data\u{fffd}.bin" (raw bytes: "data\xff.bin")"#,
        );
    }

    #[cfg(unix)]
    #[test]
    fn display_git_ls_files() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::path::PathBuf;

        let raw = PathBuf::from(OsStr::from_bytes(b"a\xc3\x28b"));
        let err = NonUtf8PathError {
            raw,
            source: NonUtf8Source::GitLsFiles,
        };
        let s = err.to_string();
        assert!(s.starts_with("non-UTF-8 path from git ls-files:"), "{s}");
        assert!(s.contains(r#"raw bytes: "a\xc3(b""#), "{s}");
    }
}
```

Run: `cargo test -p precious-core paths::utf8::tests --no-run` Expected: FAIL — `utf8` module does
not exist yet (well, the test file does but its `use super::*;` finds no items). Add `pub mod utf8;`
to `precious-core/src/paths.rs`. Re-run; FAIL on missing `NonUtf8PathError`, `NonUtf8Source`.

- [ ] **Step 3: Implement `NonUtf8PathError`**

Prepend to `precious-core/src/paths/utf8.rs` (above the test module):

```rust
use std::{fmt, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonUtf8Source {
    FilesystemWalk,
    GitLsFiles,
    GitRoot,
    DerivedPath,
}

impl fmt::Display for NonUtf8Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::FilesystemWalk => "filesystem walk",
            Self::GitLsFiles => "git ls-files",
            Self::GitRoot => "git rev-parse --show-toplevel",
            Self::DerivedPath => "derived path",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub struct NonUtf8PathError {
    pub raw: PathBuf,
    pub source: NonUtf8Source,
}

impl fmt::Display for NonUtf8PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            r#"non-UTF-8 path from {}: "{}" (raw bytes: "{}")"#,
            self.source,
            self.raw.display(),
            escape_raw(&self.raw),
        )
    }
}

fn escape_raw(path: &std::path::Path) -> String {
    // Render each byte: printable ASCII as itself, everything else as \xNN.
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let bytes = path.as_os_str().as_bytes();
        let mut out = String::with_capacity(bytes.len());
        for &b in bytes {
            if (0x20..0x7f).contains(&b) && b != b'\\' && b != b'"' {
                out.push(b as char);
            } else {
                use std::fmt::Write as _;
                let _ = write!(out, r"\x{b:02x}");
            }
        }
        out
    }

    #[cfg(not(unix))]
    {
        // On non-unix platforms `OsStr` is not byte-addressable; the lossy
        // string form is the best we can do. camino still enforces UTF-8 by
        // construction, so this branch is informational only.
        path.to_string_lossy().into_owned()
    }
}
```

- [ ] **Step 4: Verify tests pass**

Run: `cargo test -p precious-core paths::utf8::tests -- --nocapture` Expected: PASS (2 tests on
unix, 0 on Windows).

- [ ] **Step 5: Tidy and commit**

```bash
mise exec precious -- precious tidy precious-core/src/paths/utf8.rs precious-core/src/paths.rs Cargo.toml precious-core/Cargo.toml
git add Cargo.toml Cargo.lock precious-core/Cargo.toml precious-core/src/paths.rs precious-core/src/paths/utf8.rs
git commit -m "Add NonUtf8PathError and camino dependency"
```

---

## Task 2: Add raw-bytes accessor on `Exec` output

`git ls-files -z` emits NUL-separated raw bytes; we cannot validate per-entry once `Vec<u8>` has
been lossy-decoded to `String`. Today `Exec::run()` returns `Output { stdout: Option<String> }`
built via `from_utf8_lossy`. Add a sibling raw-bytes path.

**Files:**

- Modify: `precious-helpers/src/exec.rs`.

- [ ] **Step 1: Inspect current `Output` definition**

Read `precious-helpers/src/exec.rs:200-320` to confirm the `Output` struct shape and the
`bytes_to_option_string` helper around line 314.

- [ ] **Step 2: Write a failing test**

Append to the existing test module in `precious-helpers/src/exec.rs`:

```rust
#[test]
fn raw_stdout_preserves_non_utf8_bytes() -> anyhow::Result<()> {
    // Use printf to emit a NUL-separated stream containing an invalid UTF-8 byte.
    let out = Exec::builder()
        .exe("printf")
        .args(vec![r"a\x00b\xff\x00"])
        .ok_exit_codes(&[0])
        .in_dir(std::env::current_dir()?)
        .build()
        .run()?;
    let bytes = out.stdout_bytes().expect("expected raw stdout");
    assert_eq!(bytes, b"a\x00b\xff\x00".to_vec());
    Ok(())
}
```

Run: `cargo test -p precious-helpers raw_stdout_preserves_non_utf8_bytes` Expected: FAIL — `Output`
has no `stdout_bytes` method.

- [ ] **Step 3: Implement raw-bytes capture**

In `precious-helpers/src/exec.rs`:

1. Add `stdout_bytes: Option<Vec<u8>>` to `Output`.
2. In `handle_output` (or wherever the successful path constructs `Output`), populate both `stdout`
   (the existing lossy `Option<String>`) and `stdout_bytes` (`Some(output.stdout.clone())` when
   non-empty).
3. Add `pub fn stdout_bytes(&self) -> Option<&[u8]> { self.stdout_bytes.as_deref() }`.

Do not remove the existing `stdout: Option<String>` field — other callers still rely on it.

- [ ] **Step 4: Verify the test passes**

Run: `cargo test -p precious-helpers raw_stdout_preserves_non_utf8_bytes` Expected: PASS.

Run: `cargo test -p precious-helpers` Expected: all pre-existing tests still pass.

- [ ] **Step 5: Tidy and commit**

```bash
mise exec precious -- precious tidy precious-helpers/src/exec.rs
git add precious-helpers/src/exec.rs
git commit -m "Expose raw stdout bytes from Exec for binary output"
```

---

## Task 3: Convert `Matcher` to `Utf8Path`

Smallest leaf type-change; isolated from everything else.

**Files:**

- Modify: `precious-core/src/paths/matcher.rs`.

- [ ] **Step 1: Change the type signatures**

Edit `precious-core/src/paths/matcher.rs`:

- Replace `use std::path::Path;` with `use camino::{Utf8Path, Utf8PathBuf};`.
- Change `MatcherBuilder::new<P: AsRef<Path>>(root: P)` to `pub fn new<P: AsRef<Utf8Path>>(root: P)`
  and pass `root.as_ref().as_std_path()` to `GitignoreBuilder::new`.
- Change `Matcher::path_matches(&self, path: &Path, is_dir: bool)` to
  `pub fn path_matches(&self, path: &Utf8Path, is_dir: bool) -> bool`; internally pass
  `path.as_std_path()` to `matched_path_or_any_parents`.

- [ ] **Step 2: Update the existing matcher test**

In the same file's `tests` module, replace `Path::new(y)` and `Path::new(n)` with `Utf8Path::new(y)`
and `Utf8Path::new(n)`. Add `use camino::Utf8Path;`.

- [ ] **Step 3: Run the matcher tests**

Run: `cargo test -p precious-core paths::matcher` Expected: PASS (same coverage as before; only the
public API types changed).

Note: `cargo check --workspace` will fail at this point because `Finder` still passes `&Path` to
`Matcher`. That gets fixed in Task 4; do not commit yet.

- [ ] **Step 4: Hold the commit**

Skip the commit; this task's changes ship together with Task 4 because the workspace will not build
between them.

---

## Task 4: Convert `Finder` to `Utf8PathBuf` with discovery-boundary validation

The big migration. `Finder` is where the four discovery boundaries live; converting it forces the
validation in.

**Files:**

- Modify: `precious-core/src/paths/finder.rs`.

- [ ] **Step 1: Write the new boundary unit tests (failing)**

Add to the `tests` module in `precious-core/src/paths/finder.rs` (gated unix only — these touch real
on-disk filenames):

```rust
#[cfg(unix)]
#[test]
#[parallel]
fn all_mode_errors_on_non_utf8_filename() -> Result<()> {
    use crate::paths::utf8::{NonUtf8PathError, NonUtf8Source};
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let helper = testhelper::TestHelper::new()?.with_git_repo()?;
    let bad_name = OsStr::from_bytes(b"data\xff.bin");
    let mut full = helper.precious_root();
    full.push(bad_name);
    fs::write(&full, b"contents")?;

    let mut finder = new_finder(Mode::All, helper.precious_root())?;
    let err = finder.files(vec![]).expect_err("expected non-UTF-8 error");
    let downcast = err
        .downcast_ref::<NonUtf8PathError>()
        .expect("expected NonUtf8PathError");
    assert_eq!(downcast.source, NonUtf8Source::FilesystemWalk);
    Ok(())
}
```

Run:
`cargo test -p precious-core paths::finder::tests::all_mode_errors_on_non_utf8_filename --no-run`
Expected: FAIL to compile (the rest of the migration is still pending).

- [ ] **Step 2: Convert internal types**

In `precious-core/src/paths/finder.rs`:

- Replace `use std::{fs, path::{Path, PathBuf}, sync::LazyLock};` with
  `use std::{fs, sync::LazyLock}; use camino::{Utf8Path, Utf8PathBuf};`.
- Add `use crate::paths::utf8::{NonUtf8PathError, NonUtf8Source};`.
- Replace every `PathBuf` field in `Finder` with `Utf8PathBuf` and every `&Path` parameter on its
  methods with `&Utf8Path`. Specifically:
  - `project_root: Utf8PathBuf`
  - `git_root: Option<Utf8PathBuf>`
  - `cwd: Utf8PathBuf`
- Change `FinderError` variants to use `Utf8PathBuf` (and drop the `.display()` calls —
  `Utf8PathBuf` already `Display`s as a plain `&str`).
- Change `Finder::new` signature to
  `pub fn new(mode: Mode, project_root: &Utf8Path, cwd: Utf8PathBuf, exclude_globs: Vec<String>) -> Result<Finder>`.
  Canonicalize via `fs::canonicalize(project_root.as_std_path())` and wrap with
  `Utf8PathBuf::try_from(...).map_err(|e| NonUtf8PathError { raw: e.into_path_buf(), source: NonUtf8Source::DerivedPath })`.
- Change `Finder::files` signature to
  `pub fn files(&mut self, cli_paths: Vec<Utf8PathBuf>) -> Result<Option<Vec1<Utf8PathBuf>>>`.
  Remove the `#[allow(clippy::needless_pass_by_value)]` if no longer needed.
- Update `truncate_path_list` to take `&[Utf8PathBuf]` and call `.as_str()` instead of
  `.display().to_string()`.

- [ ] **Step 3: Validate at the filesystem-walk boundary**

In `walkdir_files`, after each `Ok(ent)` from the walker, replace `files.push(ent.into_path());`
with:

```rust
let p = ent.into_path();
let utf8 = Utf8PathBuf::from_path_buf(p).map_err(|raw| NonUtf8PathError {
    raw,
    source: NonUtf8Source::FilesystemWalk,
})?;
if utf8.as_std_path().is_dir() {
    continue;
}
files.push(utf8);
```

(Move the `is_dir` check after the conversion so we still skip directories; the existing
`ent.path().is_dir()` block goes away.)

Change the locals `files: Vec<PathBuf>` to `Vec<Utf8PathBuf>`. Pass `root.as_std_path()` to
`ignore::overrides::OverrideBuilder::new` and `ignore::WalkBuilder::new` since those still want
`&Path`.

- [ ] **Step 4: Validate at the git-root boundary**

In `git_root()`, replace:

```rust
self.git_root = Some(PathBuf::from(stdout.trim()));
```

with:

```rust
let trimmed = stdout.trim().to_string();
let raw = std::path::PathBuf::from(&trimmed);
let utf8 = Utf8PathBuf::try_from(raw.clone()).map_err(|_| NonUtf8PathError {
    raw,
    source: NonUtf8Source::GitRoot,
})?;
self.git_root = Some(utf8);
```

(`stdout` is already `String` from the existing `Exec::run` — the early lossy decode in
`precious-helpers` would mask a truly invalid git root, but cases where the user's git root is on a
non-UTF-8 path are vanishingly rare and the lossy-converted string will still fail the
`Utf8PathBuf::try_from` round-trip on real-world bytes; we accept this as an acceptable
approximation in line with the spec's "fail fast at boundary" intent.)

Return type of `git_root` becomes `Result<Utf8PathBuf>`.

- [ ] **Step 5: Switch `git ls-files` to `-z` with byte-level validation**

In `files_from_git`, change from reading `output.stdout` (`Option<String>`) to reading
`output.stdout_bytes()`. Replace the existing `s.lines().filter_map(...)` block with:

```rust
match output.stdout_bytes() {
    Some(bytes) => {
        let mut paths: Vec<Utf8PathBuf> = Vec::new();
        for raw in bytes.split(|b| *b == 0).filter(|s| !s.is_empty()) {
            let s = match std::str::from_utf8(raw) {
                Ok(s) => s,
                Err(_) => {
                    let raw_path = {
                        #[cfg(unix)]
                        {
                            use std::os::unix::ffi::OsStrExt;
                            std::path::PathBuf::from(std::ffi::OsStr::from_bytes(raw))
                        }
                        #[cfg(not(unix))]
                        {
                            std::path::PathBuf::from(String::from_utf8_lossy(raw).into_owned())
                        }
                    };
                    return Err(NonUtf8PathError {
                        raw: raw_path,
                        source: NonUtf8Source::GitLsFiles,
                    }
                    .into());
                }
            };
            let rel = Utf8PathBuf::from(s);
            if exclude_matcher.path_matches(&rel, false) {
                continue;
            }
            let mut f = git_root.clone();
            f.push(&rel);
            if !f.as_std_path().exists() {
                debug!(
                    "The staged file at {rel} (abs path {f}) was deleted so it will be ignored.",
                );
                continue;
            }
            paths.push(f);
        }
        Ok(self.paths_relative_to_project_root(&git_root, paths)?)
    }
    None => Ok(vec![]),
}
```

The three call sites that build `args` for `files_from_git` must each have `"-z"` appended:

- `git_modified_files`: `vec!["diff", "--name-only", "-z", "--diff-filter=ACM", "HEAD"]`
- `git_staged_files`: `vec!["diff", "--cached", "--name-only", "-z", "--diff-filter=ACM"]`
- `git_modified_since`: `vec!["diff", "--name-only", "-z", "--diff-filter=ACM", &since_dot]`

- [ ] **Step 6: Update `path_relative_to_project_root` to validate `canonicalize` output**

Replace:

```rust
let canonical = fs::canonicalize(path)
    .with_context(|| format!("Failed to canonicalize path {}", path.display()))?
    .clean();
```

with:

```rust
let canonical_std = fs::canonicalize(path.as_std_path())
    .with_context(|| format!("Failed to canonicalize path {path}"))?
    .clean();
let canonical = Utf8PathBuf::try_from(canonical_std.clone()).map_err(|_| {
    NonUtf8PathError { raw: canonical_std, source: NonUtf8Source::DerivedPath }
})?;
```

`clean_path::Clean` returns a `PathBuf`; convert with `Utf8PathBuf::try_from` and treat failure as
`DerivedPath`. Apply the same wrap to the second `.clean()` call at the end of the function (which
is called on the result of `strip_prefix`).

Adjust `paths_relative_to_project_root` to take `Vec<Utf8PathBuf>` and return
`Result<Vec<Utf8PathBuf>>`; `path_root: &Utf8Path`.

- [ ] **Step 7: Update `files_from_cli`**

Change its signature to
`fn files_from_cli(&self, cli_paths: Vec<Utf8PathBuf>) -> Result<Vec<Utf8PathBuf>>`. Replace
`self.cwd.clone().join(rel_to_cwd.clone())` with `self.cwd.join(&rel_to_cwd)`. `Utf8PathBuf::exists`
and `is_dir` exist; use them. Update `NonExistentPathOnCli` to hold `Utf8PathBuf`.

- [ ] **Step 8: Update the in-file tests**

Update every test that uses `PathBuf::from("…")` to `Utf8PathBuf::from("…")`, every `&Path`
parameter to `&Utf8Path`. Update `new_finder*` helpers' signatures. Keep the existing pre-existing
tests' coverage intact — they exercise the same behavior, just with stricter types.

Add a `use camino::Utf8PathBuf;` at the top of the `tests` module.

- [ ] **Step 9: Run the finder tests**

Run: `cargo test -p precious-core paths::finder` Expected: All pre-existing tests pass; the new
`all_mode_errors_on_non_utf8_filename` test passes (and is skipped on Windows). Note that the
workspace as a whole will still not build until Tasks 5–7 land.

- [ ] **Step 10: Hold the commit**

Same as Task 3 — hold all commits until the workspace builds again at the end of Task 7.

---

## Task 5: Migrate `command.rs` to `Utf8Path`

**Files:**

- Modify: `precious-core/src/command.rs`.

- [ ] **Step 1: Replace `Path`/`PathBuf` types**

In `precious-core/src/command.rs`:

- Replace `use std::path::{Path, PathBuf}` with `use std::path::PathBuf` (still needed for the
  `Utf8PathBuf::try_from` failure case) and add `use camino::{Utf8Path, Utf8PathBuf};`.
- Convert every field, parameter, and local that holds a path to `Utf8Path`/`Utf8PathBuf`:
  - `Command::project_root: Utf8PathBuf`
  - `PathInfo.dir: Option<Utf8PathBuf>`
  - `PathMap.path_map: HashMap<Utf8PathBuf, PathInfo>`
  - `ChdirTo(Utf8PathBuf)` (this lives in the `WorkingDir` enum imported from `config.rs` — wait
    until Task 6 lands the change there; for this task, just adjust the local matches and inserts to
    match).
  - `operating_on` signatures:
    `fn operating_on(&self, files: &Slice1<&Utf8Path>, in_dir: &Utf8Path) -> Result<Vec<Utf8PathBuf>>`
  - `path_relative_to(&self, path: &Utf8Path, in_dir: &Utf8Path) -> Utf8PathBuf`
  - `in_dir(&self, file: &Utf8Path) -> Result<Utf8PathBuf>`
  - `filter_and_sort_files<'a>(&self, files: &'a Slice1<Utf8PathBuf>) -> Vec<&'a Utf8Path>`

- [ ] **Step 2: Drop `to_string_lossy` everywhere**

Inside this file, replace every `p.to_string_lossy().to_string()`, `p.to_string_lossy()`,
`f.to_string_lossy().to_string()`, `path.to_string_lossy().to_string()`, etc. with one of:

- `p.as_str().to_string()` if a `String` is needed.
- `p` directly (used in `format!` / `Display` contexts).
- `p` directly in `Command::arg(...)` (`Utf8Path: AsRef<OsStr>` via deref).

Audit lines 535, 721, 864, 925, 985, 999, 1012, 1115, 1137, 1139, 1148 (and any that move during
editing).

- [ ] **Step 3: Bridge to std where required**

`pathdiff::diff_paths` returns `Option<PathBuf>` and `fs::canonicalize` returns `PathBuf`. Wrap each
call site:

```rust
let canon_std = fs::canonicalize(path.as_std_path())
    .with_context(|| format!("Failed to canonicalize {path}"))?;
let canon = Utf8PathBuf::try_from(canon_std.clone()).map_err(|_| NonUtf8PathError {
    raw: canon_std,
    source: NonUtf8Source::DerivedPath,
})?;
```

```rust
let diff_std = pathdiff::diff_paths(path.as_std_path(), in_dir.as_std_path())
    .ok_or_else(|| /* existing error */)?;
let diff = Utf8PathBuf::try_from(diff_std.clone()).map_err(|_| NonUtf8PathError {
    raw: diff_std,
    source: NonUtf8Source::DerivedPath,
})?;
```

Use `path.as_std_path()` whenever you need an `&Path` for an external API (`fs::*`, `ignore::*`,
`clean_path`).

- [ ] **Step 4: Update in-file tests**

Convert `PathBuf::from` → `Utf8PathBuf::from` in the test module (lines ~1172, 1205–1320). The
`Slice1::from_slice_unchecked` calls keep the same shape.

- [ ] **Step 5: Compile-only verification**

Run: `cargo check -p precious-core` Expected: errors remain only in `config.rs`, `config_init.rs`,
and `precious.rs` (callers of `command.rs` and `Finder`). The body of `command.rs` itself compiles.

Hold the commit.

---

## Task 6: Migrate `config.rs` and `config_init.rs`

**Files:**

- Modify: `precious-core/src/config.rs`, `precious-core/src/config_init.rs`.

- [ ] **Step 1: `WorkingDir::ChdirTo` becomes `Utf8PathBuf`**

In `precious-core/src/config.rs`:

- Replace `use std::path::{Path, PathBuf}` with `use std::path::PathBuf` only if still referenced;
  add `use camino::{Utf8Path, Utf8PathBuf};`.
- Change `ChdirTo(PathBuf)` to `ChdirTo(Utf8PathBuf)`. Because `Utf8PathBuf` has `serde` support (we
  enabled the feature in Task 1), TOML deserialization works without custom code. Update line 176 to
  `Ok(Some(WorkingDir::ChdirTo(Utf8PathBuf::from(value))))`.
- Replace `FileCannotBeRead { file: PathBuf, ... }` with `Utf8PathBuf`. The config-file path comes
  from the CLI (Task 7 makes it `Utf8PathBuf`) — chain the type through.
- Update the in-file tests: `PathBuf::from("foo")` → `Utf8PathBuf::from("foo")` and the
  `Vec1<PathBuf>` test fixtures → `Vec1<Utf8PathBuf>` (lines 545, 552, 697–906).

- [ ] **Step 2: Migrate `config_init.rs`**

In `precious-core/src/config_init.rs`:

- Replace `use std::path::{Path, PathBuf}` with
  `use camino::{Utf8Path, Utf8PathBuf}; use std::path::PathBuf;` (PathBuf may still be needed
  transiently).
- `ConfigInitFile.path: Utf8PathBuf` (line 27).
- `FileExists { path: Utf8PathBuf }` (line 50).
- The hardcoded paths at lines 225 and 230: `path: Utf8PathBuf::from("dev/bin/check-go-mod.sh")`,
  etc.
- `extra_files: HashMap<Utf8PathBuf, ConfigInitFile>` (line 623).
- `write_extra_files(extra_files: &HashMap<Utf8PathBuf, ConfigInitFile>)` (line 832). Inside, use
  `path.as_std_path()` for the actual `fs::write` etc.
- The clap `path` field in `ConfigInitArgs` is in `precious.rs` (Task 7).

- [ ] **Step 3: Compile-only verification**

Run: `cargo check -p precious-core` Expected: errors now confined to `precious.rs`. Hold the commit.

---

## Task 7: Migrate `precious.rs` CLI and propagate to compile-clean

**Files:**

- Modify: `precious-core/src/precious.rs`.

- [ ] **Step 1: CLI fields become `Utf8PathBuf`**

In `precious-core/src/precious.rs`:

- Add `use camino::{Utf8Path, Utf8PathBuf};`; trim `use std::path::{Path, PathBuf};` to whichever of
  those is still genuinely needed (keep `PathBuf` only if it appears in fallback-conversion sites).
- `App.config: Option<Utf8PathBuf>` (line 87).
- `CommonArgs.paths: Vec<Utf8PathBuf>` (line 163).
- `ConfigInitArgs.path: Utf8PathBuf` (line 190). Adjust the `default_value` literal — clap can parse
  `Utf8PathBuf` from a string, so `default_value = "precious.toml"` continues to work.
- `Precious.project_root: Utf8PathBuf`, `Precious.cwd: Utf8PathBuf`,
  `Precious.paths: Vec<Utf8PathBuf>` (lines 417–426).
- `ActionFailure.paths: Vec<Utf8PathBuf>` (line 73).
- `PreciousError::ConfigFileHasNoParent { file: Utf8PathBuf }` (line 37).
- `PreciousError::CannotFindRoot { cwd: String }` keeps `String`.

- [ ] **Step 2: Drop `to_string_lossy` in this file**

Specifically lines 326, 350, 447, 666:

- Line 326: `p.to_string_lossy().is_empty()` → `p.as_str().is_empty()`.
- Line 350: `cwd.to_string_lossy().to_string()` → `cwd.to_string()` (since `cwd: &Utf8Path` after
  the signature change in step 3).
- Line 447: `path.to_string_lossy()` (the `$PATH` debug log — `path` here is the env var value, an
  `OsString`; keep `to_string_lossy()` because env vars are not paths in our taxonomy).
- Line 666: `af.paths.iter().map(|p| p.to_string_lossy())` → `af.paths.iter().map(|p| p.as_str())`.

- [ ] **Step 3: Fix helper signatures**

- `fn project_root(config_file: Option<&Utf8Path>, cwd: &Utf8Path) -> Result<Utf8PathBuf>` (line
  323).
- `fn default_config_file(dir: &Utf8Path) -> Utf8PathBuf` (line 361). Replace the inner
  `PathBuf::from(dir)` with `Utf8PathBuf::from(dir.as_str())` (line 375).
- `fn config_file(&self, dir: &Utf8Path) -> Utf8PathBuf` (line 308).
- `load_config`: return `Result<(Utf8PathBuf, Utf8PathBuf, Utf8PathBuf, config::Config)>` (line
  297).
- The `Finder::new` call site: now requires `&Utf8Path` for `project_root` and `Utf8PathBuf` for
  `cwd`. Use `env::current_dir()` plus `Utf8PathBuf::try_from(...)` (map failure to
  `NonUtf8PathError { source: DerivedPath, .. }`).
- The `from_iter` test fixtures at lines 923–1340: convert `PathBuf` → `Utf8PathBuf`. Line 1132
  already maps `PathBuf::from`; switch to `Utf8PathBuf::from`. Line 1346's
  `String::from_utf8(buffer)?` is unrelated and stays.

- [ ] **Step 4: Run the full workspace check**

Run: `cargo check --workspace --all-targets` Expected: clean build.

- [ ] **Step 5: Run the workspace tests**

Run: `cargo test --workspace` Expected: all pre-existing tests pass, plus the new tests added in
Tasks 1, 2, and 4 pass. Tests gated `#[cfg(unix)]` skip on Windows.

- [ ] **Step 6: Clippy check**

Run: `cargo clippy --workspace --all-targets -- -D warnings` Expected: clean.

- [ ] **Step 7: Tidy, then a single migration commit**

```bash
mise exec precious -- precious tidy precious-core precious-helpers
```

If `precious tidy` reports nothing to do for any path, that is fine. Then:

```bash
git add precious-core precious-helpers
git commit -m "Migrate precious-core to camino::Utf8Path with fail-fast UTF-8 validation"
```

This commit covers Tasks 3 through 7 because the workspace cannot compile in between.

---

## Task 8: End-to-end integration tests for non-UTF-8 filenames

Verify the user-facing behavior of a real precious run against a real on-disk non-UTF-8 filename,
and add the `café.txt` regression test confirming the `-z` change does not break valid non-ASCII
names.

**Files:**

- Create: `precious-integration/tests/non_utf8_paths.rs` (or extend an existing integration-test
  file if precious-integration already has one — check first).

- [ ] **Step 1: Find the existing integration-test entry point**

```bash
ls precious-integration/tests/
```

Choose: extend an existing file if one already drives end-to-end precious invocations, otherwise
create `non_utf8_paths.rs`.

- [ ] **Step 2: Write the failing test for a non-UTF-8 filename on disk**

```rust
#[cfg(unix)]
#[test]
fn non_utf8_filename_fails_with_clear_error() -> anyhow::Result<()> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::process::Command;

    let helper = precious_testhelper::TestHelper::new()?.with_git_repo()?;
    let bad = OsStr::from_bytes(b"data\xff.bin");
    let mut path = helper.precious_root();
    path.push(bad);
    std::fs::write(&path, b"contents")?;

    // Write a minimal precious config that defines no commands so the run
    // exercises path discovery and nothing else.
    let cfg = helper.precious_root().join("precious.toml");
    std::fs::write(&cfg, "[commands]\n")?;

    let out = Command::new(env!("CARGO_BIN_EXE_precious"))
        .current_dir(helper.precious_root())
        .args(["--config", "precious.toml", "lint", "--all"])
        .output()?;

    assert_ne!(out.status.code(), Some(0), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("non-UTF-8 path from filesystem walk"),
        "expected fail-fast diagnostic, got stderr:\n{stderr}",
    );
    assert!(
        stderr.contains(r"data\xff.bin"),
        "expected raw-byte escape in stderr, got:\n{stderr}",
    );
    Ok(())
}
```

Run: `cargo test -p precious-integration non_utf8_filename_fails_with_clear_error` Expected: PASS
(the migration in Tasks 1–7 should already make this work). If the assertion fails because of a
different exit path (e.g. the empty config errors before discovery), adjust the config to declare a
no-op lint command that operates on `**/*` so discovery actually runs.

- [ ] **Step 3: Write the `café.txt` regression test**

```rust
#[test]
fn cafe_txt_handled_through_git() -> anyhow::Result<()> {
    use std::process::Command;

    let helper = precious_testhelper::TestHelper::new()?.with_git_repo()?;
    let name = "café.txt";
    std::fs::write(helper.precious_root().join(name), "hello")?;

    Command::new("git")
        .current_dir(helper.precious_root())
        .args(["add", name])
        .status()?;

    let cfg = helper.precious_root().join("precious.toml");
    std::fs::write(
        &cfg,
        // A no-op lint that just echoes filenames so the run succeeds.
        r#"[commands.echo]
type = "lint"
include = "**/*"
cmd = ["true"]
ok-exit-codes = [0]
"#,
    )?;

    let out = Command::new(env!("CARGO_BIN_EXE_precious"))
        .current_dir(helper.precious_root())
        .args(["--config", "precious.toml", "lint", "--staged"])
        .output()?;

    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(())
}
```

(The exact config keys must match the project's current TOML schema — read an existing example from
`precious-core/src/config.rs` tests or `Changes.md` to confirm.)

Run: `cargo test -p precious-integration cafe_txt_handled_through_git` Expected: PASS. The `-z`
change to `git ls-files` is what makes this case work — without it, git C-quotes `caf\303\251.txt`
and the path no longer resolves to disk.

- [ ] **Step 4: Run the full test suite once more**

Run: `cargo test --workspace` Expected: PASS.

Run: `cargo clippy --workspace --all-targets -- -D warnings` Expected: clean.

- [ ] **Step 5: Tidy and commit**

```bash
mise exec precious -- precious tidy precious-integration
git add precious-integration
git commit -m "Add integration tests for non-UTF-8 path fail-fast and café.txt regression"
```

---

## Task 9: Documentation and changelog

**Files:**

- Modify: `Changes.md`.

- [ ] **Step 1: Add a `Changes.md` entry**

Following the format of the most-recent entries (read the top of `Changes.md` first), add a new
entry under the unreleased section:

```
* Precious now fails fast with a clear error when it encounters a file path
  that is not valid UTF-8, instead of silently lossy-converting. Previously
  non-UTF-8 paths were corrupted at the `git ls-files` decode step and at
  subprocess-argument construction. The error message includes both a
  human-readable lossy approximation and the exact raw bytes of the path.
  Internally, `precious-core` now uses `camino::Utf8PathBuf` throughout so
  the UTF-8 invariant is enforced by the type system.
```

- [ ] **Step 2: Tidy and commit**

```bash
mise exec precious -- precious tidy Changes.md
git add Changes.md
git commit -m "Document UTF-8 fail-fast behavior in Changes.md"
```

---

## Post-flight

- [ ] Run `git log --oneline origin/master..` to confirm the commit list reads cleanly (spec, exec
      raw-bytes, big migration, integration tests, changelog).
- [ ] Remove the loose-end directory mentioned in the handoff if it still exists:

```bash
rmdir /home/autarch/projects/precious/.claude/worktrees 2>/dev/null || true
```

- [ ] Delete the handoff document — its content is now superseded by the merged work:

```bash
git rm docs/superpowers/specs/2026-05-30-utf8-path-handling-HANDOFF.md
git commit -m "Remove session handoff doc"
```

(Only run that final `git rm` if the user confirms; it's tracked here as a checkbox so it doesn't
get forgotten.)

- [ ] Hand off to user for review / merge using the `superpowers:finishing-a-development-branch`
      skill.
