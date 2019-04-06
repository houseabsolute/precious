use crate::gitignore::ignore_file::*;
use failure::Error;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Repo {
    ignore_files: HashMap<PathBuf, IgnoreFile>,
}

/// Given the path to a Git repository, load up all of the ignore files in the
/// usual Git heirachy and allow checking of ignore status against all of them.
impl Repo {
    pub fn new<P: AsRef<Path>>(root: P) -> Result<Repo, Error> {
        let glob = root
            .as_ref()
            .join("**/.gitignore")
            .to_string_lossy()
            .into_owned();
        let files = glob::glob(&glob)?;

        let ignore_files: HashMap<PathBuf, IgnoreFile> = files
            .flat_map(|glob_result| glob_result.ok())
            .flat_map(|file| IgnoreFile::new(&root, &file).map(|ignore_file| (file, ignore_file)))
            .collect();

        Ok(Repo { ignore_files })
    }

    pub fn is_ignored<P: AsRef<Path>>(&self, path: P, is_dir: bool) -> bool {
        // When given a path, for each segment in the path, find any `.gitignore`
        // corresponding to it that segment.
        // Try the deepest first, recursing up to the root.
        // If a file is excluded by an ignore file, stop recursing (?)
        // (FIXME: is is possible to be re-included by a higher-up file?)

        // path.parent().join(".gitignore")
        // recurse until path.parent() == root
        // find matching ignorefiles

        self.ignore_files
            .values()
            .any(|ignore_file| ignore_file.is_ignored(&path, is_dir))
    }
}

#[cfg(test)]
mod test {
    use super::Repo;
    use std::path::PathBuf;

    macro_rules! test_repo {
        () => {{
            let cargo_root: PathBuf = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
            let root: PathBuf = cargo_root.join("tests/resources/fake_repo").to_path_buf();

            Repo::new(root).unwrap()
        }};
    }

    #[test]
    fn is_ignored_is_false_for_all_expected_files() {
        let repo = test_repo!();

        assert!(!repo.is_ignored(".badgitignore", false));
        assert!(!repo.is_ignored(".gitignore", false));
        assert!(!repo.is_ignored("also_include_me", false));
        assert!(!repo.is_ignored("include_me", false));
        assert!(!repo.is_ignored("a_dir/a_nested_dir/.gitignore", false));
        // FIXME: This last test won't work until we do cascading properly.
        // assert!(!repo.is_ignored("a_dir/a_nested_dir/deeper_still/bit_now_i_work.no", false));
    }

    #[test]
    fn is_ignored_is_true_for_all_expected_files() {
        let repo = test_repo!();

        assert!(repo.is_ignored("not_me.no", false));
        assert!(repo.is_ignored("or_even_me", false));
        assert!(repo.is_ignored("or_me.no", false));
        assert!(repo.is_ignored("a_dir/a_nested_dir/deeper_still/hello.greeting", false));
        assert!(repo.is_ignored("a_dir/a_nested_dir/deeper_still/hola.greeting", false));
    }
}
