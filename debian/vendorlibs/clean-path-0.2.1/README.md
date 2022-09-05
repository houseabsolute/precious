# clean-path

[![crates.io version][1]][2]
[![docs.rs docs][3]][4]
[![license][5]][6]

`clean-path` is a safe fork of the
[`path-clean`](https://crates.io/crates/path-clean) crate.

## Installation

```sh
cargo add clean-path
```

## Usage

```rust
use std::path::{Path, PathBuf};
use clean_path::{clean, Clean};

assert_eq!(clean("foo/../../bar"), PathBuf::from("../bar"));
assert_eq!(Path::new("hello/world/..").clean(), PathBuf::from("hello"));
assert_eq!(
    PathBuf::from("/test/../path/").clean(),
    PathBuf::from("/path")
);
```

## About

This fork aims to provide the same utility as
[`path-clean`](https://crates.io/crates/path-clean), without using unsafe. Additionally, the api
is improved ([`clean`] takes `AsRef<Path>` instead of just `&str`) and `Clean` is implemented on
`Path` in addition to just `PathBuf`.

The main cleaning procedure is implemented using the methods provided by `PathBuf`, thus it should
bring portability benefits over [`path-clean`](https://crates.io/crates/path-clean) w.r.t. correctly
handling cross-platform filepaths.

Additionally, the original implementation in [`path-clean`](https://crates.io/crates/path-clean) is
rather inscrutible, and as such if being able to inspect and understand the code is important to
you, this crate provides a more readable implementation.

However, the current implementation is not highly-optimized, so if performance is top-priority,
consider using [`path-clean`](https://crates.io/crates/path-clean) instead.

## Specification

The cleaning works as follows:
1. Reduce multiple slashes to a single slash.
2. Eliminate `.` path name elements (the current directory).
3. Eliminate `..` path name elements (the parent directory) and the non-`.` non-`..`, element that precedes them.
4. Eliminate `..` elements that begin a rooted path, that is, replace `/..` by `/` at the beginning of a path.
5. Leave intact `..` elements that begin a non-rooted path.

If the result of this process is an empty string, return the
string `"."`, representing the current directory.

This transformation is performed lexically, without touching the filesystem. Therefore it doesn't do
any symlink resolution or absolute path resolution. For more information you can see ["Getting
Dot-Dot Right"](https://9p.io/sys/doc/lexnames.html).

This functionality is exposed in the [`clean`] function and [`Clean`] trait implemented for
[`std::path::PathBuf`] and [`std::path::Path`].

## License
[MIT](./LICENSE-MIT) OR [Apache-2.0](./LICENSE-APACHE)


[1]: https://img.shields.io/crates/v/clean-path.svg?style=flat-square
[2]: https://crates.io/crates/clean-path
[3]: https://img.shields.io/badge/docs-latest-blue.svg?style=flat-square
[4]: https://docs.rs/clean-path
[5]: https://img.shields.io/crates/l/clean-path.svg?style=flat-square
[6]: #license
