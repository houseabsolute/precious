# UTF-8 Path Handling Design

Date: 2026-05-30

## Problem

Precious does not handle file paths that are not valid UTF-8 in a coherent way. Today it preserves
such paths in some places and silently corrupts them in others:

- Filesystem discovery via the `ignore` crate preserves the raw bytes as `PathBuf` (lossless).
- `git ls-files` output is decoded with `String::from_utf8_lossy`, so non-UTF-8 bytes become the
  U+FFFD replacement character before the path ever becomes a `PathBuf`. The resulting path no
  longer matches the real file on disk.
- Subprocess invocation converts paths to arguments with `to_string_lossy`, again corrupting any
  non-UTF-8 bytes before passing them to the linter or formatter.
- Error and log output uses `to_string_lossy` everywhere, so the user sees garbled paths with no
  indication that anything was lost.

The net effect is that precious cannot reliably operate on files whose names are not valid UTF-8,
and it gives no clear diagnostic explaining why.

## Goal

Any encounter with a non-UTF-8 path produces a clear, immediate error for the user. Precious must
never silently lossy-convert a path. The UTF-8 invariant should be enforced by the type system so
the bug cannot be reintroduced.

## Decisions

- **Severity: fail fast.** On encountering a non-UTF-8 path, abort the entire run with a clear error
  and a non-zero exit code. No command runs against a corrupted name.
- **Detection sites:** all places a non-UTF-8 path can enter the system — CLI arguments, filesystem
  walk results, `git ls-files` output, and the git root path. Config-file path lists are already
  guaranteed UTF-8 (they come from TOML strings) and require no runtime check, only a type change.
- **Error grouping: report the first bad path, then abort.** A single clear error; the user fixes
  one at a time.
- **Display format: lossy + escaped bytes.** The message shows both the human-friendly lossy
  approximation and the exact raw bytes, e.g.
  `non-UTF-8 path from git ls-files: "data�.bin" (raw bytes: "data\xff.bin")`.

## Architecture

Replace `PathBuf` / `Path` with `camino::Utf8PathBuf` / `Utf8Path` throughout `precious-core`. The
UTF-8 invariant is then enforced by the type system: the only `Path → Utf8Path` conversions happen
at the entry points, every such conversion is fallible and produces a clear error, and downstream
code cannot perform a lossy conversion because there is no `PathBuf` left to convert.

Add `camino` to the workspace with the `serde` feature so `Utf8PathBuf` fields in the config
deserialize directly from TOML.

A new module `precious-core/src/paths/utf8.rs` defines the error type:

```rust
pub enum NonUtf8Source {
    FilesystemWalk,
    GitLsFiles,
    GitRoot,
    DerivedPath, // result of canonicalize / pathdiff that failed UTF-8 conversion
}

pub struct NonUtf8PathError {
    pub raw: PathBuf,          // original bytes, for the error message
    pub source: NonUtf8Source,
}
```

`Display for NonUtf8PathError` renders the lossy + escaped-bytes form described above. The error
converts into `anyhow::Error` and flows into the existing top-level handler in
`precious-core/src/precious.rs`, which already exits with status 1 and prints the message to stderr.
No new plumbing is required past the entry points.

### CLI arguments need no custom code

Declaring the clap path field as `Vec<Utf8PathBuf>` makes clap reject a non-UTF-8 argument with its
own "invalid UTF-8" usage error (exit code 2) before precious sees it. Therefore `NonUtf8Source` has
no `CliArg` variant. The slightly different exit code (clap's 2 vs. precious's 1) is acceptable;
both are clear, immediate errors.

## Components & Data Flow

End to end: **discovery validates → everything downstream is statically UTF-8 → subprocess
invocation and output are byte-faithful.**

### `paths/finder.rs` — discovery boundaries

- **Filesystem walk:** each `ent.path()` from the `ignore` walker goes through
  `Utf8Path::from_path(p).ok_or_else(|| NonUtf8PathError { source: FilesystemWalk, .. })?`. The
  first failure aborts the walk and propagates.
- **git ls-files:** switch the invocation to `-z` (NUL-separated output, which also disables git's
  default C-quoting of non-ASCII bytes). Read stdout as raw `Vec<u8>`, split on `0x00`, and validate
  each entry with `str::from_utf8` → `Utf8PathBuf` or `NonUtf8PathError { source: GitLsFiles, .. }`.
  This replaces the current `from_utf8_lossy` decode on this call path.
- **git root:** validate the trimmed stdout as UTF-8 → `Utf8PathBuf` or
  `NonUtf8PathError { source: GitRoot, .. }`.
- `Finder::files` signature becomes
  `fn files(&mut self, cli_paths: Vec<Utf8PathBuf>) -> Result<Option<Vec1<Utf8PathBuf>>>`.

### `command.rs` — consumes only `Utf8Path`

- Subprocess args: pass `Utf8Path` directly to `Command::arg` (it is `AsRef<OsStr>`) or use
  `.as_str()` / `.to_string()`. No lossy conversion.
- `fs::*` calls bridge to std via `Utf8Path::as_std_path()` (or camino's `read_dir_utf8`).
- `pathdiff` / `canonicalize` return std `PathBuf`; wrap their results with
  `Utf8PathBuf::try_from(...)`, mapping failure to `NonUtf8PathError { source: DerivedPath, .. }`.
  These derive from already-validated inputs, but the OS-provided absolute-path prefix is the one
  component not yet validated, so the conversion remains fallible for correctness rather than
  panicking.
- The `to_string_lossy()` sites in log and error messages become `.as_str()` or direct `Display`.

### `config.rs`, `config_init.rs`, `matcher.rs`

- `WorkingDir::ChdirTo` and any other path fields become `Utf8PathBuf`, deserialized directly from
  TOML (already UTF-8).
- `config_init.rs` and `matcher.rs` follow mechanically from the type change.

## Error Handling

- All discovery boundaries and the derived-path conversion produce `NonUtf8PathError`, converted to
  `anyhow::Error` and surfaced by the existing top-level handler → exit 1, message on stderr. The
  first bad path aborts.
- CLI arguments are rejected by clap directly → exit 2. Noted as an intentional difference rather
  than worked around.
- No code path panics on a non-UTF-8 path; every conversion is fallible and yields the clear error.

## Testing

### Unit tests (run on all platforms, no disk writes, no gating)

- `paths/utf8.rs`: `Display` formatting for `NonUtf8PathError`, including the lossy + escaped-bytes
  rendering, driven by an in-memory byte sequence containing an invalid byte (e.g. `0xFF`).
  Constructed in memory via `OsStr`/`Vec<u8>`; nothing is written to disk, so these run on every
  platform without gating.

### Integration tests (`precious-integration`)

- A real run against a checkout containing a file whose name is not valid UTF-8 exits non-zero with
  the expected stderr. This guards the end-to-end behavior, including the `-z` git change, covering
  both the filesystem-walk and git-ls-files discovery paths.
- A regression test confirms that a _valid_ non-ASCII UTF-8 name (`café.txt`) is handled correctly
  through git — the `-z` change fixes the previously-broken C-quoting case — proving the change
  rejects only genuinely invalid UTF-8, not all non-ASCII names.

### Cross-platform gating

Only tests that actually write a non-UTF-8 filename to disk require gating, because such names
cannot be created on Windows. These tests are conditionally compiled / skipped on Windows (e.g.
`#[cfg_attr(windows, ignore)]` or a `#[cfg(unix)]` guard with a comment explaining the platform
limitation) and use `std::os::unix::ffi::OsStrExt::from_bytes` to construct the illegal name on
unix. Unit tests do not write to disk and are therefore not gated.

## Out of Scope

- Windows ill-formed UTF-16 path names. The unix byte-construction approach does not apply on
  Windows, and camino enforces the UTF-8 invariant on all platforms regardless. Behavior there is
  left unverified by tests but is still correct by construction.
- Any configurable "skip and continue" behavior. The chosen behavior is unconditional fail-fast.
