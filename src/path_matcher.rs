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

    #[test]
    fn path_matches() -> Result<(), Error> {
        let e1 = Matcher::new(&[String::from("*.foo")])?;
        assert!(e1.path_matches(&PathBuf::from("file.foo")));
        assert!(!e1.path_matches(&PathBuf::from("file.bar")));
        assert!(e1.path_matches(&PathBuf::from("./file.foo")));
        assert!(!e1.path_matches(&PathBuf::from("./file.bar")));

        let e2 = Matcher::new(&[String::from("*.foo"), String::from("**/foo/*")])?;
        assert!(e2.path_matches(&PathBuf::from("file.foo")));
        assert!(!e2.path_matches(&PathBuf::from("file.bar")));
        assert!(e2.path_matches(&PathBuf::from("/baz/bar/file.foo")));
        assert!(!e2.path_matches(&PathBuf::from("/baz/bar/file.bar")));
        assert!(e2.path_matches(&PathBuf::from("/contains/foo/any.txt")));
        assert!(e2.path_matches(&PathBuf::from("./file.foo")));
        assert!(!e2.path_matches(&PathBuf::from("./file.bar")));
        assert!(e2.path_matches(&PathBuf::from("./baz/bar/file.foo")));
        assert!(!e2.path_matches(&PathBuf::from("./baz/bar/file.bar")));
        assert!(e2.path_matches(&PathBuf::from("./contains/foo/any.txt")));

        Ok(())
    }
}
