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
