use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonUtf8Source {
    FilesystemWalk,
    GitDiff,
    GitRoot,
    Cwd,
    DerivedPath,
}

impl fmt::Display for NonUtf8Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::FilesystemWalk => "filesystem walk",
            Self::GitDiff => "git diff",
            Self::GitRoot => "git rev-parse --show-toplevel",
            Self::Cwd => "current working directory",
            Self::DerivedPath => "derived path",
        };
        f.write_str(s)
    }
}

#[derive(Debug, PartialEq, Eq)]
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

impl Error for NonUtf8PathError {}

/// Build a `PathBuf` from raw bytes when reconstructing a non-UTF-8 path for
/// error reporting. On unix the bytes are preserved as-is; on other platforms
/// `OsStr` is not byte-addressable, so we fall back to a lossy decode (the
/// resulting `PathBuf` is informational only — camino prevents the rest of the
/// program from ever seeing it).
pub(crate) fn bytes_to_pathbuf(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

fn escape_raw(path: &Path) -> String {
    #[cfg(unix)]
    {
        use std::fmt::Write as _;
        use std::os::unix::ffi::OsStrExt;
        let bytes = path.as_os_str().as_bytes();
        let mut out = String::with_capacity(bytes.len());
        for &b in bytes {
            if (0x20..0x7f).contains(&b) && b != b'\\' && b != b'"' {
                out.push(b as char);
            } else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[cfg(unix)]
    #[test]
    fn display_filesystem_walk() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let raw = PathBuf::from(OsStr::from_bytes(b"data\xff.bin"));
        let err = NonUtf8PathError {
            raw,
            source: NonUtf8Source::FilesystemWalk,
        };
        assert_eq!(
            err.to_string(),
            "non-UTF-8 path from filesystem walk: \"data\u{fffd}.bin\" (raw bytes: \"data\\xff.bin\")",
        );
    }

    #[cfg(unix)]
    #[test]
    fn display_git_diff() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let raw = PathBuf::from(OsStr::from_bytes(b"a\xc3\x28b"));
        let err = NonUtf8PathError {
            raw,
            source: NonUtf8Source::GitDiff,
        };
        let s = err.to_string();
        assert!(s.starts_with("non-UTF-8 path from git diff:"), "{s}");
        assert!(s.contains(r#"raw bytes: "a\xc3(b""#), "{s}");
    }

    #[cfg(unix)]
    #[test]
    fn display_git_root() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let raw = PathBuf::from(OsStr::from_bytes(b"/repo\xff"));
        let err = NonUtf8PathError {
            raw,
            source: NonUtf8Source::GitRoot,
        };
        let s = err.to_string();
        assert!(
            s.starts_with("non-UTF-8 path from git rev-parse --show-toplevel:"),
            "{s}",
        );
        assert!(s.contains(r"\xff"), "{s}");
    }

    #[cfg(unix)]
    #[test]
    fn display_cwd() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let raw = PathBuf::from(OsStr::from_bytes(b"/tmp/\xfe"));
        let err = NonUtf8PathError {
            raw,
            source: NonUtf8Source::Cwd,
        };
        let s = err.to_string();
        assert!(
            s.starts_with("non-UTF-8 path from current working directory:"),
            "{s}",
        );
        assert!(s.contains(r"\xfe"), "{s}");
    }

    #[cfg(unix)]
    #[test]
    fn display_derived_path() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let raw = PathBuf::from(OsStr::from_bytes(b"/canon/\xfd"));
        let err = NonUtf8PathError {
            raw,
            source: NonUtf8Source::DerivedPath,
        };
        let s = err.to_string();
        assert!(s.starts_with("non-UTF-8 path from derived path:"), "{s}");
        assert!(s.contains(r"\xfd"), "{s}");
    }
}
