use crate::gitignore;
use failure::Error;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::fs;
use std::path::PathBuf;

#[derive(Debug)]
pub struct Excluder {
    root: PathBuf,
    repo: gitignore::repo::Repo,
    exclude: GlobSet,
}

impl Excluder {
    pub fn new(root: &PathBuf, exclude_globs: &[String]) -> Result<Excluder, Error> {
        let mut builder = GlobSetBuilder::new();
        for g in exclude_globs {
            builder.add(Glob::new(g.as_str())?);
        }

        Ok(Excluder {
            root: root.clone(),
            repo: gitignore::repo::Repo::new(root)?,
            exclude: builder.build()?,
        })
    }

    pub fn path_is_excluded(&self, path: &PathBuf) -> Result<bool, Error> {
        let mut full = self.root.clone();
        full.push(path);
        let is_dir = fs::metadata(&full)?.is_dir();
        if self.repo.is_ignored(path, is_dir) {
            return Ok(true);
        }
        Ok(self.exclude.is_match(path))
    }
}
