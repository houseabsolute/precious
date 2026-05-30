use anyhow::{Context, Result};
use camino::Utf8Path;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

#[derive(Debug)]
#[allow(clippy::module_name_repetitions)]
pub struct MatcherBuilder {
    builder: GitignoreBuilder,
}

#[allow(clippy::new_without_default)]
impl MatcherBuilder {
    pub fn new<P: AsRef<Utf8Path>>(root: P) -> Self {
        Self {
            builder: GitignoreBuilder::new(root.as_ref()),
        }
    }

    pub fn with(mut self, globs: &[impl AsRef<str>]) -> Result<Self> {
        for g in globs {
            self.builder.add_line(None, g.as_ref()).with_context(|| {
                format!(r#"Failed to add glob pattern "{}" to matcher"#, g.as_ref())
            })?;
        }
        Ok(self)
    }

    pub fn build(self) -> Result<Matcher> {
        Ok(Matcher {
            gitignore: self
                .builder
                .build()
                .context("Failed to build gitignore matcher")?,
        })
    }
}

#[derive(Debug)]
pub struct Matcher {
    gitignore: Gitignore,
}

impl Matcher {
    pub fn path_matches(&self, path: &Utf8Path, is_dir: bool) -> bool {
        self.gitignore
            .matched_path_or_any_parents(path, is_dir)
            .is_ignore()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8Path;
    use serial_test::parallel;

    #[test]
    #[parallel]
    fn path_matches() -> Result<()> {
        struct TestSet {
            globs: &'static [&'static str],
            yes: &'static [&'static str],
            no: &'static [&'static str],
        }

        let tests = &[
            TestSet {
                globs: &["*.foo"],
                yes: &["file.foo", "./file.foo"],
                no: &["file.bar", "./file.bar"],
            },
            TestSet {
                globs: &["*.foo", "**/foo/*"],
                yes: &[
                    "file.foo",
                    "/baz/bar/file.foo",
                    "/contains/foo/any.txt",
                    "./file.foo",
                    "./baz/bar/file.foo",
                    "./contains/foo/any.txt",
                ],
                no: &[
                    "file.bar",
                    "/baz/bar/file.bar",
                    "./file.bar",
                    "./baz/bar/file.bar",
                ],
            },
            TestSet {
                globs: &["/foo/**/*"],
                yes: &["/foo/file.go", "/foo/bar/baz/file.go"],
                no: &["/bar/file.go"],
            },
            TestSet {
                globs: &["/foo/**/*", "!/foo/bar/baz.*"],
                yes: &["/foo/file.go", "/foo/bar/quux/file.go"],
                no: &["/bar/file.go", "/foo/bar/baz.txt"],
            },
            // Bare directory names should match files inside that directory,
            // just like gitignore does.
            TestSet {
                globs: &["target"],
                yes: &["target", "target/file.rs", "target/debug/build/foo/bar.rs"],
                no: &["src/main.rs", "nottarget/file.rs"],
            },
            TestSet {
                globs: &["target", "!target/important.rs"],
                yes: &["target/file.rs", "target/debug/build/foo.rs"],
                no: &["src/main.rs", "target/important.rs"],
            },
        ];

        for t in tests {
            let globs = t.globs.join(" ");
            let m = MatcherBuilder::new("/").with(t.globs)?.build()?;
            for y in t.yes {
                assert!(
                    m.path_matches(Utf8Path::new(y), false),
                    "{y} matches [{globs}]"
                );
            }
            for n in t.no {
                assert!(
                    !m.path_matches(Utf8Path::new(n), false),
                    "{n} does not match [{globs}]",
                );
            }
        }

        Ok(())
    }
}
