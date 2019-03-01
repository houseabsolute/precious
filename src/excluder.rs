use crate::gitignore;
use failure::Error;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::fs;
use std::path::PathBuf;

#[derive(Debug)]
pub struct Excluder {
    root: PathBuf,
    ignore: Vec<gitignore::ignore_file::IgnoreFile>,
    exclude: GlobSet,
}

impl Excluder {
    pub fn new(
        root: &PathBuf,
        ignore_files: &[String],
        exclude_globs: &[String],
    ) -> Result<Excluder, Error> {
        let mut ignore: Vec<gitignore::ignore_file::IgnoreFile> = vec![];
        for f in ignore_files {
            let gi = gitignore::ignore_file::IgnoreFile::new(root.clone(), f)?;
            ignore.push(gi);
        }

        let mut builder = GlobSetBuilder::new();
        for g in exclude_globs {
            builder.add(Glob::new(g.as_str())?);
        }

        Ok(Excluder {
            root: root.clone(),
            ignore,
            exclude: builder.build()?,
        })
    }

    pub fn path_is_excluded(&self, path: &PathBuf) -> Result<bool, Error> {
        let mut full = self.root.clone();
        full.push(path);
        let is_dir = fs::metadata(&full)?.is_dir();
        for i in self.ignore.iter() {
            if i.is_ignored(path, is_dir) {
                return Ok(true);
            }
        }
        Ok(self.exclude.is_match(path))
    }
}
