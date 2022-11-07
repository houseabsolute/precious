use anyhow::Result;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::Path;

#[derive(Debug)]
pub struct MatcherBuilder {
    builder: GitignoreBuilder,
}

#[allow(clippy::new_without_default)]
impl MatcherBuilder {
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        Self {
            builder: GitignoreBuilder::new(root),
        }
    }

    pub fn with(mut self, globs: &[impl AsRef<str>]) -> Result<Self> {
        for g in globs {
            self.builder.add_line(None, g.as_ref())?;
        }
        Ok(self)
    }

    pub fn build(self) -> Result<Matcher> {
        Ok(Matcher {
            gitignore: self.builder.build()?,
        })
    }
}

#[derive(Debug)]
pub struct Matcher {
    gitignore: Gitignore,
}

impl Matcher {
    pub fn path_matches(&self, path: &Path, is_dir: bool) -> bool {
        self.gitignore.matched(path, is_dir).is_ignore()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        ];

        for t in tests {
            let globs = t.globs.join(" ");
            let m = MatcherBuilder::new("/").with(t.globs)?.build()?;
            for y in t.yes {
                assert!(
                    m.path_matches(Path::new(y), false),
                    "{} matches [{}]",
                    y,
                    globs,
                );
            }
            for n in t.no {
                assert!(
                    !m.path_matches(Path::new(n), false),
                    "{} does not match [{}]",
                    n,
                    globs,
                );
            }
        }

        Ok(())
    }
}
