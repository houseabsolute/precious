use failure::Error;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::PathBuf;

#[derive(Debug)]
pub struct Matcher {
    globs: GlobSet,
}

impl Matcher {
    pub fn new(globs: &[String]) -> Result<Matcher, Error> {
        let mut builder = GlobSetBuilder::new();
        for g in globs {
            builder.add(Glob::new(g.as_str())?);
        }

        Ok(Matcher {
            globs: builder.build()?,
        })
    }

    pub fn path_matches(&self, path: &PathBuf) -> bool {
        self.globs.is_match(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spectral::prelude::*;

    struct TestSet {
        globs: Vec<String>,
        yes: &'static [&'static str],
        no: &'static [&'static str],
    }

    #[test]
    fn path_matches() -> Result<(), Error> {
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
            let m = Matcher::new(&t.globs)?;
            for y in t.yes {
                assert_that(&m.path_matches(&PathBuf::from(y)))
                    .named(format!("{} matches", y).as_str())
                    .is_true();
            }
            for n in t.no {
                assert_that(&m.path_matches(&PathBuf::from(n)))
                    .named(format!("{} matches", n).as_str())
                    .is_false();
            }
        }

        Ok(())
    }
}
