# Avoid Per-Path Canonicalize in Git and All-Files Modes

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the O(N) `canonicalize` syscalls in `Finder` for git modes and all-files mode by
canonicalizing each path _root_ once and computing per-file relative paths via pure path arithmetic.

**Architecture:** `Finder::path_relative_to_project_root` currently calls `canonicalize_utf8()` on
every input path. In `walkdir_files` and `files_from_git` the per-file paths are constructed by
joining a fixed root (`self.project_root`, already canonical, or `git_root`, currently
un-canonicalized) with a clean relative segment produced by `ignore::Walk` or git's `-z` output — no
`..` components, no user-supplied symlinks to resolve. Canonicalize the `git_root` once on first
resolve, cache it, and use a non-canonicalizing variant of the helper that just does
`path_root_canonical.join(rel).strip_prefix(&self.project_root)`. CLI mode keeps the canonicalizing
helper because user input may contain `..` or symlinks.

**Tech Stack:** Rust, `camino::Utf8PathBuf`, `precious_testhelper`, `serial_test` (`#[parallel]`),
`pretty_assertions`.

---

## File Structure

- **Modify:** [precious-core/src/paths/finder.rs](precious-core/src/paths/finder.rs)
  - `Finder` struct: store the canonical git root alongside the raw one.
  - `git_root()`: canonicalize on first resolve and cache.
  - Split the relative-path helper into two variants: `path_relative_to_project_root`
    (canonicalizing, used only by CLI) and a new `path_relative_to_project_root_assuming_canonical`
    (pure path math).
  - `walkdir_files` and `files_from_git`: call the non-canonicalizing variant.
- **Modify:** [Changes.md](Changes.md) — add an entry under the unreleased section.

No new modules, no API changes outside the `Finder` impl.

---

## Pre-flight

- [ ] **Step 0: Create the working branch on top of the current branch**

Run:

```bash
git checkout -b perf-avoid-per-path-canonicalize
```

Expected: `Switched to a new branch 'perf-avoid-per-path-canonicalize'`. The current branch at the
time of writing is `epic-raman-04f47e`; this new branch should be based directly on it.

---

## Task 1: Cache a canonical git root

**Files:**

- Modify: `precious-core/src/paths/finder.rs` (struct + `git_root` method around lines 18-26 and
  146-183)

- [ ] **Step 1: Add a regression test that fails today only if the cache is wrong**

We don't yet have direct read access to `git_root` from tests, so this task's correctness is
verified by Task 4's tests. Skip writing a test here and proceed to the implementation; Task 4 will
exercise both the cache and the canonicalization.

- [ ] **Step 2: Add a `canonical_git_root` field**

In the struct (around [finder.rs:18-26](precious-core/src/paths/finder.rs:18)):

```rust
#[derive(Debug)]
pub struct Finder {
    mode: Mode,
    project_root: Utf8PathBuf,
    git_root: Option<Utf8PathBuf>,
    canonical_git_root: Option<Utf8PathBuf>,
    cwd: Utf8PathBuf,
    exclude_globs: Vec<String>,
    stashed: bool,
}
```

In `Finder::new` (around [finder.rs:73-80](precious-core/src/paths/finder.rs:73)):

```rust
Ok(Finder {
    mode,
    project_root: canonical_root,
    git_root: None,
    canonical_git_root: None,
    cwd,
    exclude_globs,
    stashed: false,
})
```

- [ ] **Step 3: Add a `canonical_git_root` method that canonicalizes once and caches**

Add it directly below `git_root` (after [finder.rs:183](precious-core/src/paths/finder.rs:183)):

```rust
fn canonical_git_root(&mut self) -> Result<Utf8PathBuf> {
    if let Some(r) = &self.canonical_git_root {
        return Ok(r.clone());
    }
    let raw = self.git_root()?;
    let canonical = raw
        .canonicalize_utf8()
        .with_context(|| format!("Failed to canonicalize git root path {raw}"))?;
    self.canonical_git_root = Some(canonical.clone());
    Ok(canonical)
}
```

- [ ] **Step 4: Build the crate**

Run: `mise exec -- cargo build -p precious-core` Expected: clean build, no warnings about unused
field/method (the field/method are referenced again in Tasks 3 and 4).

If the unused-method warning fires, that's fine for this intermediate step — it will go away in
Task 3.

- [ ] **Step 5: Commit**

```bash
git add precious-core/src/paths/finder.rs
git commit -m "Cache a canonicalized git root in Finder"
```

---

## Task 2: Add a non-canonicalizing relative-path helper

**Files:**

- Modify: `precious-core/src/paths/finder.rs` (around the helper at
  [finder.rs:411-430](precious-core/src/paths/finder.rs:411))

- [ ] **Step 1: Add the helper**

Insert immediately after the existing `path_relative_to_project_root` (after
[finder.rs:430](precious-core/src/paths/finder.rs:430)):

```rust
// Like `path_relative_to_project_root` but without a per-path canonicalize.
// The caller must have already canonicalized `path_root` and must guarantee
// that `rel` is a clean relative path (no `..`, no symlink-bearing
// components beyond `path_root` itself). Used in hot paths where we walk
// many files under a single fixed root.
fn path_relative_to_project_root_assuming_canonical(
    &self,
    path_root_canonical: &Utf8Path,
    rel: &Utf8Path,
) -> Result<Utf8PathBuf> {
    let joined = if rel.is_absolute() {
        rel.to_path_buf()
    } else {
        path_root_canonical.join(rel)
    };

    let stripped = joined.strip_prefix(&self.project_root).map_err(|_| {
        FinderError::PrefixNotFound {
            path: joined.clone(),
            prefix: self.project_root.clone(),
        }
    })?;

    if stripped.as_str().is_empty() {
        Ok(Utf8PathBuf::from("."))
    } else {
        Ok(stripped.to_path_buf())
    }
}
```

- [ ] **Step 2: Build**

Run: `mise exec -- cargo build -p precious-core` Expected: builds. Warning about unused method is
acceptable here; the next task removes it.

- [ ] **Step 3: Commit**

```bash
git add precious-core/src/paths/finder.rs
git commit -m "Add non-canonicalizing path-relative-to-project-root helper"
```

---

## Task 3: Use the non-canonicalizing helper in `walkdir_files`

**Files:**

- Modify: `precious-core/src/paths/finder.rs`
  ([walkdir_files at finder.rs:279-324](precious-core/src/paths/finder.rs:279) and the unused
  `paths_relative_to_project_root` aggregator at
  [finder.rs:391-409](precious-core/src/paths/finder.rs:391))

`walkdir_files` walks under `root` (which in practice is either `self.project_root`, already
canonical, or a path under it from `files_from_cli`). The walker emits paths rooted at the walk root
— no `..`. So the per-file canonicalize is unnecessary _as long as `root` itself is canonical_.

`files_from_cli` calls `walkdir_files(&full)` where `full = self.cwd.join(cli_path)` — that may not
be canonical. So we need to canonicalize `root` once at the top of `walkdir_files` instead of per
file.

- [ ] **Step 1: Update `walkdir_files` to canonicalize `root` once and use the new helper**

Replace the body of `walkdir_files` (currently at
[finder.rs:279-324](precious-core/src/paths/finder.rs:279)) with:

```rust
fn walkdir_files(&self, root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let canonical_root = root
        .canonicalize_utf8()
        .with_context(|| format!("Failed to canonicalize walk root {root}"))?;

    let mut exclude_globs = ignore::overrides::OverrideBuilder::new(&canonical_root);
    for d in vcs::DIRS {
        exclude_globs
            .add(&format!("!{d}/**/*"))
            .with_context(|| format!("Failed to add VCS directory override pattern for {d}"))?;
    }

    let overrides = exclude_globs
        .build()
        .context("Failed to build directory override patterns")?;

    let exclude_matcher = self
        .exclude_matcher()
        .context("Failed to build exclude matcher")?;

    let mut files: Vec<Utf8PathBuf> = vec![];
    for result in ignore::WalkBuilder::new(&canonical_root)
        .hidden(false)
        .overrides(overrides)
        .build()
    {
        match result {
            Ok(ent) => {
                let p = ent.into_path();
                let utf8 = Utf8PathBuf::from_path_buf(p).map_err(|raw| NonUtf8PathError {
                    raw,
                    source: NonUtf8Source::FilesystemWalk,
                })?;
                if utf8.is_dir() {
                    continue;
                }
                let rel = self.path_relative_to_project_root_assuming_canonical(
                    &canonical_root,
                    &utf8,
                )?;
                if exclude_matcher.path_matches(&rel, false) {
                    continue;
                }
                files.push(rel);
            }
            Err(e) => {
                return Err(e).with_context(|| format!("Failed to walk directory {root}"))?
            }
        }
    }

    Ok(files)
}
```

Notes:

- Walker yields absolute paths (because we passed it `canonical_root`, an absolute path). The
  helper's `is_absolute()` branch handles that.
- We now apply `exclude_matcher` _inside_ the walk loop rather than via `Vec::filter`, so we no
  longer need `paths_relative_to_project_root` here.

- [ ] **Step 2: Remove the now-unused `paths_relative_to_project_root` aggregator**

If `files_from_git` (next task) still uses it, defer this removal to Task 4. Just confirm via
`cargo build` whether the method is referenced and only delete it once _all_ callers are gone. After
Task 4 it will be unreferenced — delete it then.

For this task, leave it in place.

- [ ] **Step 3: Build and test**

Run:

```bash
mise exec -- cargo build -p precious-core
mise exec -- cargo test -p precious-core --lib paths::finder
```

Expected: build clean; existing `paths::finder` tests pass — in particular `all_files_in_project`,
`all_files_in_project_subdir`, and the `Mode::All` variants.

- [ ] **Step 4: Commit**

```bash
git add precious-core/src/paths/finder.rs
git commit -m "Canonicalize walk root once instead of per file"
```

---

## Task 4: Use the non-canonicalizing helper in `files_from_git`

**Files:**

- Modify: `precious-core/src/paths/finder.rs`
  ([files_from_git at finder.rs:326-375](precious-core/src/paths/finder.rs:326))
- Modify: `precious-core/src/paths/finder.rs` — delete the now-unused
  `paths_relative_to_project_root` aggregator at
  [finder.rs:391-409](precious-core/src/paths/finder.rs:391)

- [ ] **Step 1: Write a regression test for the symlinked git-root case**

Add this test to the `mod tests` block at the bottom of
[finder.rs](precious-core/src/paths/finder.rs) (immediately before the closing `}` of `mod tests` at
[finder.rs:1329](precious-core/src/paths/finder.rs:1329)):

```rust
#[test]
#[parallel]
fn git_mode_works_when_project_root_reached_via_symlink() -> Result<()> {
    // Reaching the project root via a symlink used to work only because we
    // canonicalized every git-produced path. Now we canonicalize the git
    // root once; this test guards that the cached canonical root is what we
    // use, not the symlink path the user passed in.
    let helper = testhelper::TestHelper::new()?.with_git_repo()?;
    helper.write_file(Utf8Path::new("src/foo.rs"), "fn foo() {}\n")?;
    helper.stage_all()?;

    let real_root = precious_root_utf8(&helper);
    let parent = real_root.parent().expect("project root has a parent");
    let link = parent.join(format!(
        "{}-link",
        real_root.file_name().expect("project root has a name"),
    ));
    // Best-effort cleanup if a prior failed run left it behind.
    let _ = std::fs::remove_file(link.as_std_path());
    std::os::unix::fs::symlink(real_root.as_std_path(), link.as_std_path())?;

    let result = (|| -> Result<()> {
        let mut finder = new_finder(Mode::GitStaged, link.clone())?;
        let files = finder
            .files(&[])?
            .expect("expected at least one staged file");
        assert!(
            files.iter().any(|p| p == Utf8Path::new("src/foo.rs")),
            "expected src/foo.rs in {files:?}",
        );
        Ok(())
    })();

    std::fs::remove_file(link.as_std_path())?;
    result
}
```

If `testhelper::TestHelper` does not already expose `write_file` and `stage_all`, replace those two
lines with whatever the existing tests in this file use to create-and-stage a file — search the test
module above for an example pattern (look for other `Mode::GitStaged` tests) and copy it.

- [ ] **Step 2: Run the new test — expect it to PASS already with the current code**

Run:

```bash
mise exec -- cargo test -p precious-core --lib paths::finder::tests::git_mode_works_when_project_root_reached_via_symlink
```

Expected: PASS. Today's per-path canonicalize handles this case fine — the test exists to guard the
new code path against regression. If it fails, stop and investigate: the test setup is wrong, not
the production code.

- [ ] **Step 3: Replace the relative-path loop in `files_from_git`**

In `files_from_git` (around [finder.rs:342-371](precious-core/src/paths/finder.rs:342)), replace the
entire `match output.stdout_bytes.as_deref()` arm body for `Some(bytes)` with:

```rust
Some(bytes) => {
    let canonical_git_root = self.canonical_git_root()?;
    let mut paths: Vec<Utf8PathBuf> = Vec::new();
    for raw in bytes.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let Ok(s) = std::str::from_utf8(raw) else {
            return Err(NonUtf8PathError {
                raw: crate::paths::utf8::bytes_to_pathbuf(raw),
                source: NonUtf8Source::GitLsFiles,
            }
            .into());
        };
        let rel = Utf8PathBuf::from(s);
        if exclude_matcher.path_matches(&rel, false) {
            continue;
        }
        let full = canonical_git_root.join(&rel);
        if !full.exists() {
            debug!(
                "The staged file at {rel} (abs path {full}) was deleted so it will be ignored.",
            );
            continue;
        }
        paths.push(self.path_relative_to_project_root_assuming_canonical(
            &canonical_git_root,
            &full,
        )?);
    }
    Ok(paths)
}
```

Note: we now compute `full` using `canonical_git_root` (was `git_root`), call the no-canonicalize
helper directly per-path, and no longer accumulate then post-process.

- [ ] **Step 4: Re-run the regression test**

Run:

```bash
mise exec -- cargo test -p precious-core --lib paths::finder::tests::git_mode_works_when_project_root_reached_via_symlink
```

Expected: PASS.

- [ ] **Step 5: Delete the now-unused `paths_relative_to_project_root` aggregator**

Remove the entire method body at [finder.rs:391-409](precious-core/src/paths/finder.rs:391):

```rust
// We want to make all files relative. This lets us consistently produce
// path names starting at the root dir (without "./"). The given root is
// the _current_ root for the relative file, which can be the cwd or the
// git root instead of the project root.
fn paths_relative_to_project_root(
    &self,
    path_root: &Utf8Path,
    paths: Vec<Utf8PathBuf>,
) -> Result<Vec<Utf8PathBuf>> {
    let mut relative: Vec<Utf8PathBuf> = vec![];
    for mut f in paths {
        if !f.is_absolute() {
            f = path_root.join(f);
        }

        relative.push(self.path_relative_to_project_root(&f)?);
    }

    Ok(relative)
}
```

- [ ] **Step 6: Full finder test run**

Run:

```bash
mise exec -- cargo test -p precious-core --lib paths::finder
```

Expected: all tests pass, including the existing `git_*` mode tests and
`cli_mode_given_path_outside_project_root`.

- [ ] **Step 7: Commit**

```bash
git add precious-core/src/paths/finder.rs
git commit -m "Canonicalize git root once instead of per file"
```

---

## Task 5: Full verification and Changes.md

**Files:**

- Modify: `Changes.md`

- [ ] **Step 1: Run the full workspace test suite**

Run: `mise exec -- cargo test --workspace` Expected: all tests pass.

- [ ] **Step 2: Run clippy**

Run: `mise exec -- cargo clippy --workspace --all-targets -- -D warnings` Expected: no warnings.

- [ ] **Step 3: Run `precious tidy` to satisfy the pre-commit hook**

Run: `mise exec precious -- precious tidy` Expected: clean exit. (Per project memory, this is
required before committing.)

- [ ] **Step 4: Add a Changes.md entry**

Open [Changes.md](Changes.md) and add a bullet to the top unreleased section describing the change:

```markdown
- Reduced filesystem syscalls when finding files. Previously, every path produced by `precious` in
  git modes and all-files mode was canonicalized individually. Now the relevant root directory (the
  project root, git root, or walk root) is canonicalized once and per-file relative paths are
  computed via path arithmetic. No user-visible behavior change.
```

If the changelog already has an unreleased section header, place the bullet under it. If not, follow
whatever convention the previous releases use — check the first 20 lines of `Changes.md` and match
the style.

- [ ] **Step 5: Final commit**

```bash
git add Changes.md
git commit -m "Mention canonicalize-once optimization in Changes.md"
```

- [ ] **Step 6: Hand off**

Report: branch `perf-avoid-per-path-canonicalize` is ready, based on `epic-raman-04f47e`. Suggest
the user open a PR against `epic-raman-04f47e` (or rebase onto `master` if `epic-raman-04f47e` has
already merged by then).

---

## Self-Review Notes

- **Spec coverage:** The reviewer's complaint ("Per-path canonicalize in git mode is O(N) syscalls")
  is addressed in Task 4 (git mode) and Task 3 (all-files mode, same underlying issue). CLI mode is
  intentionally untouched because user input may contain `..` and symlinks.
- **Placeholders:** None. All code blocks are complete.
- **Type consistency:** `path_relative_to_project_root_assuming_canonical` takes `&Utf8Path` for
  both arguments and returns `Result<Utf8PathBuf>`, used consistently in Tasks 3 and 4.
  `canonical_git_root` returns `Result<Utf8PathBuf>` like `git_root`. `FinderError::PrefixNotFound`
  is reused unchanged.
- **Risk:** The behavior change in Task 3 — applying `exclude_matcher` inline rather than via
  `Vec::filter` — is semantically identical: both filter the same path against the same matcher with
  `is_dir = false`. The walker already skips directories above the matcher check.
