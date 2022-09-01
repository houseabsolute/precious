use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::Path;

#[derive(Debug)]
pub struct MatcherBuilder {
    builder: GlobSetBuilder,
}

impl MatcherBuilder {
    pub fn new() -> Self {
        Self {
            builder: GlobSetBuilder::new(),
        }
    }

    pub fn with(mut self, globs: &[impl AsRef<str>]) -> Result<Self> {
        for g in globs {
            self.builder.add(Glob::new(g.as_ref())?);
        }
        Ok(self)
    }

    pub fn build(self) -> Result<Matcher> {
        Ok(Matcher {
            globs: self.builder.build()?,
        })
    }
}

#[derive(Debug)]
pub struct Matcher {
    globs: GlobSet,
}

impl Matcher {
    pub fn path_matches(&self, path: &Path) -> bool {
        self.globs.is_match(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::path::PathBuf;

    struct TestSet {
        globs: Vec<String>,
        yes: &'static [&'static str],
        no: &'static [&'static str],
    }

    #[test]
    fn path_matches() -> Result<()> {
        let tests = vec![
            TestSet {
                globs: vec![String::from("*.foo")],
                yes: &["file.foo", "./file.foo"],
                no: &["file.bar", "./file.bar"],
            },
            TestSet {
                globs: vec![String::from("*.foo"), String::from("**/foo/*")],
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
                globs: vec![String::from("/foo/**/*")],
                yes: &["/foo/file.go", "/foo/bar/baz/file.go"],
                no: &["/bar/file.go"],
            },
        ];
        for t in tests {
            let m = MatcherBuilder::new().with(&t.globs)?.build()?;
            for y in t.yes {
                assert!(m.path_matches(&PathBuf::from(y)), "{} matches", y);
            }
            for n in t.no {
                assert!(!m.path_matches(&PathBuf::from(n)), "{} matches", n);
            }
        }

        Ok(())
    }
}
