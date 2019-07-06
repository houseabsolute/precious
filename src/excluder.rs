use failure::Error;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::PathBuf;

#[derive(Debug)]
pub struct Excluder {
    exclude: GlobSet,
}

impl Excluder {
    pub fn new(exclude_globs: &[String]) -> Result<Excluder, Error> {
        let mut builder = GlobSetBuilder::new();
        for g in exclude_globs {
            builder.add(Glob::new(g.as_str())?);
        }

        Ok(Excluder {
            exclude: builder.build()?,
        })
    }

    pub fn path_is_excluded(&self, path: &PathBuf) -> bool {
        self.exclude.is_match(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusions() -> Result<(), Error> {
        let e1 = Excluder::new(&[String::from("*.foo")])?;
        assert!(e1.path_is_excluded(&PathBuf::from("file.foo")));
        assert!(!e1.path_is_excluded(&PathBuf::from("file.bar")));
        assert!(e1.path_is_excluded(&PathBuf::from("./file.foo")));
        assert!(!e1.path_is_excluded(&PathBuf::from("./file.bar")));

        let e2 = Excluder::new(&[String::from("*.foo"), String::from("**/foo/*")])?;
        assert!(e2.path_is_excluded(&PathBuf::from("file.foo")));
        assert!(!e2.path_is_excluded(&PathBuf::from("file.bar")));
        assert!(e2.path_is_excluded(&PathBuf::from("/baz/bar/file.foo")));
        assert!(!e2.path_is_excluded(&PathBuf::from("/baz/bar/file.bar")));
        assert!(e2.path_is_excluded(&PathBuf::from("/contains/foo/any.txt")));
        assert!(e2.path_is_excluded(&PathBuf::from("./file.foo")));
        assert!(!e2.path_is_excluded(&PathBuf::from("./file.bar")));
        assert!(e2.path_is_excluded(&PathBuf::from("./baz/bar/file.foo")));
        assert!(!e2.path_is_excluded(&PathBuf::from("./baz/bar/file.bar")));
        assert!(e2.path_is_excluded(&PathBuf::from("./contains/foo/any.txt")));

        Ok(())
    }
}
