//! `path-clean` is a Rust port of the the `cleanname` procedure from the Plan 9 C library, and is similar to
//! [`path.Clean`](https://golang.org/pkg/path/#Clean) from the Go standard library. It works as follows:
//!
//! 1. Reduce multiple slashes to a single slash.
//! 2. Eliminate `.` path name elements (the current directory).
//! 3. Eliminate `..` path name elements (the parent directory) and the non-`.` non-`..`, element that precedes them.
//! 4. Eliminate `..` elements that begin a rooted path, that is, replace `/..` by `/` at the beginning of a path.
//! 5. Leave intact `..` elements that begin a non-rooted path.
//!
//! If the result of this process is an empty string, return the string `"."`, representing the current directory.
//!
//! It performs this transform lexically, without touching the filesystem. Therefore it doesn't do
//! any symlink resolution or absolute path resolution. For more information you can see ["Getting Dot-Dot
//! Right"](https://9p.io/sys/doc/lexnames.html).
//!
//! For convenience, the [`PathClean`] trait is exposed and comes implemented for [`std::path::PathBuf`].
//!
//! ```rust
//! use std::path::PathBuf;
//! use path_clean::{clean, PathClean};
//! assert_eq!(clean("hello/world/.."), "hello");
//! assert_eq!(
//!     PathBuf::from("/test/../path/").clean(),
//!     PathBuf::from("/path")
//! );
//! ```

use std::path::PathBuf;

/// The Clean trait implements a `clean` method. It's recommended you use the provided [`clean`]
/// function.
pub trait PathClean<T> {
    fn clean(&self) -> T;
}

/// PathClean implemented for PathBuf
impl PathClean<PathBuf> for PathBuf {
    fn clean(&self) -> PathBuf {
        PathBuf::from(clean(self.to_str().unwrap_or("")))
    }
}

/// The core implementation. It performs the following, lexically:
/// 1. Reduce multiple slashes to a single slash.
/// 2. Eliminate `.` path name elements (the current directory).
/// 3. Eliminate `..` path name elements (the parent directory) and the non-`.` non-`..`, element that precedes them.
/// 4. Eliminate `..` elements that begin a rooted path, that is, replace `/..` by `/` at the beginning of a path.
/// 5. Leave intact `..` elements that begin a non-rooted path.
///
/// If the result of this process is an empty string, return the string `"."`, representing the current directory.
pub fn clean(path: &str) -> String {
    match path {
        "" => return ".".to_string(),
        "." => return ".".to_string(),
        ".." => return "..".to_string(),
        "/" => return "/".to_string(),
        _ => {}
    }

    let mut out = vec![];
    let is_root = path.starts_with("/");

    let path = path.trim_end_matches("/");
    let num_segments = path.split("/").count();

    for segment in path.split("/") {
        match segment {
            "" => continue,
            "." => {
                if num_segments == 1 {
                    out.push(segment);
                };
                continue;
            }
            ".." => {
                let previous = out.pop();
                if previous.is_some() && !can_backtrack(previous.unwrap()) {
                    out.push(previous.unwrap());
                    out.push(segment);
                } else if previous.is_none() && !is_root {
                    out.push(segment);
                };
                continue;
            }
            _ => {
                out.push(segment);
            }
        };
    }

    let mut out_str = out.join("/");

    if is_root {
        out_str = format!("/{}", out_str);
    }

    if out_str.len() == 0 {
        return ".".to_string();
    }

    out_str
}

fn can_backtrack(segment: &str) -> bool {
    match segment {
        "." => false,
        ".." => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{clean, PathClean};
    use std::path::PathBuf;

    #[test]
    fn test_empty_path_is_current_dir() {
        assert_eq!(clean(""), ".");
    }

    #[test]
    fn test_clean_paths_dont_change() {
        let tests = vec![(".", "."), ("..", ".."), ("/", "/")];

        for test in tests {
            assert_eq!(clean(test.0), test.1);
        }
    }

    #[test]
    fn test_replace_multiple_slashes() {
        let tests = vec![
            ("/", "/"),
            ("//", "/"),
            ("///", "/"),
            (".//", "."),
            ("//..", "/"),
            ("..//", ".."),
            ("/..//", "/"),
            ("/.//./", "/"),
            ("././/./", "."),
            ("path//to///thing", "path/to/thing"),
            ("/path//to///thing", "/path/to/thing"),
        ];

        for test in tests {
            assert_eq!(clean(test.0), test.1);
        }
    }

    #[test]
    fn test_eliminate_current_dir() {
        let tests = vec![
            ("./", "."),
            ("/./", "/"),
            ("./test", "test"),
            ("./test/./path", "test/path"),
            ("/test/./path/", "/test/path"),
            ("test/path/.", "test/path"),
        ];

        for test in tests {
            assert_eq!(clean(test.0), test.1);
        }
    }

    #[test]
    fn test_eliminate_parent_dir() {
        let tests = vec![
            ("/..", "/"),
            ("/../test", "/test"),
            ("test/..", "."),
            ("test/path/..", "test"),
            ("test/../path", "path"),
            ("/test/../path", "/path"),
            ("test/path/../../", "."),
            ("test/path/../../..", ".."),
            ("/test/path/../../..", "/"),
            ("/test/path/../../../..", "/"),
            ("test/path/../../../..", "../.."),
            ("test/path/../../another/path", "another/path"),
            ("test/path/../../another/path/..", "another"),
            ("../test", "../test"),
            ("../test/", "../test"),
            ("../test/path", "../test/path"),
            ("../test/..", ".."),
        ];

        for test in tests {
            assert_eq!(clean(test.0), test.1);
        }
    }

    #[test]
    fn test_pathbuf_trait() {
        assert_eq!(
            PathBuf::from("/test/../path/").clean(),
            PathBuf::from("/path")
        );
    }
}
