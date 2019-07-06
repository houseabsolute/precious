use crate::command;
use crate::excluder;
use crate::vcs;
use failure::Error;
use ignore;
use itertools::Itertools;
use log::debug;
use regex::Regex;
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    FromCLI,
    All,
    GitModified,
    GitStaged,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Mode::FromCLI => write!(f, "paths passed on the CLI (recursively)"),
            Mode::All => write!(f, "all files in the project"),
            Mode::GitModified => write!(f, "modified files according to git"),
            Mode::GitStaged => write!(f, "files staged for a git commit"),
        }
    }
}

#[derive(Debug)]
pub struct BasePaths {
    mode: Mode,
    cli_paths: Vec<PathBuf>,
    root: PathBuf,
    excluder: excluder::Excluder,
}

#[derive(Debug, PartialEq)]
pub struct Paths {
    pub dir: PathBuf,
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Fail, PartialEq)]
pub enum BasePathsError {
    #[fail(
        display = "You cannot pass an explicit list of files when looking for {}",
        mode
    )]
    GotPathsFromCLIWithWrongMode { mode: Mode },

    #[fail(display = "Did not find any paths when looking for {}", mode)]
    NoMatchingPaths { mode: Mode },

    #[fail(
        display = "Found some paths when looking for {} but they were all excluded",
        mode
    )]
    AllPathsWereExcluded { mode: Mode },
}

impl BasePaths {
    pub fn new(
        mode: Mode,
        cli_paths: Vec<PathBuf>,
        root: PathBuf,
        exclude_globs: &[String],
    ) -> Result<BasePaths, Error> {
        match mode {
            Mode::FromCLI => (),
            _ => {
                if !cli_paths.is_empty() {
                    return Err(BasePathsError::GotPathsFromCLIWithWrongMode { mode })?;
                }
            }
        };

        let exc = excluder::Excluder::new(exclude_globs)?;
        Ok(BasePaths {
            mode,
            cli_paths,
            root,
            excluder: exc,
        })
    }

    pub fn paths(&self) -> Result<Vec<Paths>, Error> {
        let files = match self.mode {
            Mode::All => self.all_files()?,
            Mode::FromCLI => self.files_from_cli()?,
            Mode::GitModified => self.git_modified_files()?,
            Mode::GitStaged => self.git_staged_files()?,
        };

        if files.is_none() {
            return Err(BasePathsError::NoMatchingPaths {
                mode: self.mode.clone(),
            })?;
        }

        self.files_to_paths(
            files
                .unwrap()
                .iter()
                .filter_map(|p| {
                    if self.excluder.path_is_excluded(&p) {
                        return None;
                    }
                    if self.is_vcs_dir(&p) {
                        return None;
                    }

                    Some(p.clone())
                })
                .collect(),
        )
    }

    fn all_files(&self) -> Result<Option<Vec<PathBuf>>, Error> {
        debug!("Getting all files under {}", self.root.to_string_lossy());
        self.walkdir_files(&self.root)
    }

    fn files_from_cli(&self) -> Result<Option<Vec<PathBuf>>, Error> {
        debug!("Using the list of files passed from the command line");
        let mut files: Vec<PathBuf> = vec![];
        for cli in self.cli_paths.iter() {
            // We want to resolve this path relative to our root, not our
            // current directory. This is necessary for our test code and
            // could come up in real world usage (I think).
            let mut full = self.root.clone();
            full.push(cli);

            if full.is_dir() {
                files.append(self.walkdir_files(&full)?.unwrap().as_mut());
            } else {
                files.push(full.strip_prefix(&self.root)?.to_path_buf());
            }
        }
        Ok(Some(files))
    }

    fn git_modified_files(&self) -> Result<Option<Vec<PathBuf>>, Error> {
        debug!("Getting modified files according to git");
        self.files_from_git(&["diff", "--name-only", "--diff-filter=ACM"])
    }

    fn git_staged_files(&self) -> Result<Option<Vec<PathBuf>>, Error> {
        debug!("Getting staged files according to git");
        self.files_from_git(&["diff", "--cached", "--name-only", "--diff-filter=ACM"])
    }

    fn walkdir_files(&self, root: &PathBuf) -> Result<Option<Vec<PathBuf>>, Error> {
        let mut files: Vec<PathBuf> = vec![];
        for result in ignore::WalkBuilder::new(root).hidden(false).build() {
            if result.is_err() {
                return Err(result.err().unwrap())?;
            }

            let ent = result.ok().unwrap();
            if ent.path().is_dir() {
                continue;
            }

            files.push(ent.into_path());
        }

        Ok(Some(files))
    }

    fn files_from_git(&self, args: &[&str]) -> Result<Option<Vec<PathBuf>>, Error> {
        let result = command::run_command(
            String::from("git"),
            args.iter().map(|a| String::from(*a)).collect(),
            [0].to_vec(),
            false,
            Some(&self.root),
        )?;

        match result.stdout {
            Some(s) => Ok(Some(
                s.lines()
                    .map(|r| {
                        let mut f = self.root.clone();
                        f.push(r);
                        f
                    })
                    .collect(),
            )),
            None => Ok(None),
        }
    }

    fn is_vcs_dir(&self, path: &PathBuf) -> bool {
        for dir in vcs::VCS_DIRS {
            if path.starts_with(dir) {
                return true;
            }
        }
        false
    }

    fn files_to_paths(&self, files: Vec<PathBuf>) -> Result<Vec<Paths>, Error> {
        let mut entries: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        let mut pat = String::from("^");
        pat.push_str(&self.root.to_string_lossy());
        pat.push('/');
        let re = Regex::new(pat.as_str())?;

        for p in files {
            let rel = PathBuf::from(re.replace(&p.to_string_lossy(), "./").into_owned());

            let dir = rel.parent().unwrap().to_path_buf();
            if dir.starts_with("./.git/") || dir == PathBuf::from("./.git") {
                continue;
            }

            if entries.contains_key(&dir) {
                entries.get_mut(&dir).unwrap().push(rel.clone());
            } else {
                let files: Vec<PathBuf> = vec![rel.clone()];
                entries.insert(dir, files);
            }
        }

        if entries.is_empty() {
            return Err(BasePathsError::AllPathsWereExcluded {
                mode: self.mode.clone(),
            })?;
        }

        Ok(entries
            .keys()
            .sorted()
            .map(|k| {
                let mut files = entries.get(k).unwrap().to_vec();
                files.sort();
                Paths {
                    dir: k.to_path_buf(),
                    files,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testhelper;
    use spectral::prelude::*;

    fn new_basepaths(mode: Mode, paths: Vec<PathBuf>, root: PathBuf) -> Result<BasePaths, Error> {
        BasePaths::new(mode, paths, root, &[])
    }

    #[test]
    fn test_files_to_paths() -> Result<(), Error> {
        let root = testhelper::create_git_repo()?;
        let mut files: Vec<PathBuf> = vec![];
        for p in testhelper::paths().iter() {
            files.push(PathBuf::from(p));
        }

        let bp = new_basepaths(Mode::All, vec![], root.path().to_owned())?;
        let paths = bp.files_to_paths(files)?;
        assert_that(&paths.len())
            .named("got three paths entries")
            .is_equal_to(3);
        assert_that(&paths[0]).is_equal_to(Paths {
            dir: PathBuf::from("."),
            files: ["./README.md", "./can_ignore.x"]
                .iter()
                .map(PathBuf::from)
                .collect(),
        });
        assert_that(&paths[1]).is_equal_to(Paths {
            dir: PathBuf::from("./src"),
            files: [
                "./src/bar.rs",
                "./src/can_ignore.rs",
                "./src/main.rs",
                "./src/module.rs",
            ]
            .iter()
            .map(PathBuf::from)
            .collect(),
        });
        assert_that(&paths[2]).is_equal_to(Paths {
            dir: PathBuf::from("./tests/data"),
            files: [
                "./tests/data/bar.txt",
                "./tests/data/foo.txt",
                "./tests/data/generated.txt",
            ]
            .iter()
            .map(PathBuf::from)
            .collect(),
        });

        Ok(())
    }

    #[test]
    fn all_mode() -> Result<(), Error> {
        let root = testhelper::create_git_repo()?;
        let bp = new_basepaths(Mode::All, vec![], root.path().to_owned())?;
        assert_that(&bp.paths()?).is_equal_to(
            bp.files_to_paths(testhelper::paths().iter().map(PathBuf::from).collect())?,
        );
        Ok(())
    }

    #[test]
    fn all_mode_with_gitignore() -> Result<(), Error> {
        let root = testhelper::create_git_repo()?;
        let mut gitignores = testhelper::add_gitignore_files(&root)?;
        let mut expect = testhelper::non_ignored_files();
        expect.append(&mut gitignores);

        let bp = new_basepaths(Mode::All, vec![], root.path().to_owned())?;
        assert_that(&bp.paths()?)
            .is_equal_to(bp.files_to_paths(expect.iter().map(PathBuf::from).collect())?);
        Ok(())
    }

    #[test]
    fn git_modified_mode_empty() -> Result<(), Error> {
        let root = testhelper::create_git_repo()?;
        let bp = new_basepaths(Mode::GitModified, vec![], root.path().to_owned())?;
        let res = bp.paths();
        assert_that(&res).is_err();
        assert_that(&std::mem::discriminant(
            res.unwrap_err()
                .as_fail()
                .find_root_cause()
                .downcast_ref()
                .unwrap(),
        ))
        .is_equal_to(std::mem::discriminant(&BasePathsError::NoMatchingPaths {
            mode: Mode::GitModified,
        }));
        Ok(())
    }

    #[test]
    fn git_modified_mode_with_changes() -> Result<(), Error> {
        let root = testhelper::create_git_repo()?;
        let modified = testhelper::modify_files(&root)?;
        let bp = new_basepaths(Mode::GitModified, vec![], root.path().to_owned())?;
        let expect = bp.files_to_paths(
            modified
                .iter()
                .filter(|p| !p.starts_with(".git"))
                .sorted_by(|a, b| a.cmp(b))
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_that(&bp.paths()?).is_equal_to(expect);
        Ok(())
    }

    #[test]
    fn git_staged_mode_empty() -> Result<(), Error> {
        let root = testhelper::create_git_repo()?;
        let bp = new_basepaths(Mode::GitStaged, vec![], root.path().to_owned())?;
        let res = bp.paths();
        assert_that(&res).is_err();
        assert_that(&std::mem::discriminant(
            res.unwrap_err()
                .as_fail()
                .find_root_cause()
                .downcast_ref()
                .unwrap(),
        ))
        .is_equal_to(std::mem::discriminant(&BasePathsError::NoMatchingPaths {
            mode: Mode::GitStaged,
        }));
        Ok(())
    }

    #[test]
    fn git_staged_mode_with_changes() -> Result<(), Error> {
        let root = testhelper::create_git_repo()?;
        let modified = testhelper::modify_files(&root)?;
        let bp = new_basepaths(Mode::GitStaged, vec![], root.path().to_owned())?;
        let res = bp.paths();
        assert_that(&res).is_err();
        assert_that(&std::mem::discriminant(
            res.unwrap_err()
                .as_fail()
                .find_root_cause()
                .downcast_ref()
                .unwrap(),
        ))
        .is_equal_to(std::mem::discriminant(&BasePathsError::NoMatchingPaths {
            mode: Mode::GitStaged,
        }));

        testhelper::stage_all_in(&root)?;
        let expect = bp.files_to_paths(
            modified
                .iter()
                .filter(|p| !p.starts_with(".git"))
                .sorted_by(|a, b| a.cmp(b))
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_that(&bp.paths()?).is_equal_to(expect);

        Ok(())
    }

    #[test]
    fn cli_mode() -> Result<(), Error> {
        let root = testhelper::create_git_repo()?;
        let bp = new_basepaths(
            Mode::FromCLI,
            vec![PathBuf::from("tests")],
            root.path().to_owned(),
        )?;
        let expect = bp.files_to_paths(
            testhelper::paths()
                .iter()
                .filter(|p| p.starts_with("./tests/"))
                .sorted_by(|a, b| a.cmp(b))
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_that(&bp.paths()?).is_equal_to(expect);
        Ok(())
    }

    #[test]
    fn is_vcs_dir() -> Result<(), Error> {
        let bp = new_basepaths(Mode::All, vec![], PathBuf::from("/root"))?;

        assert!(bp.is_vcs_dir(&PathBuf::from(".git")));
        assert!(bp.is_vcs_dir(&PathBuf::from(".hg")));
        assert!(bp.is_vcs_dir(&PathBuf::from(".svn")));
        assert!(!bp.is_vcs_dir(&PathBuf::from("git")));
        assert!(!bp.is_vcs_dir(&PathBuf::from("hg")));
        assert!(!bp.is_vcs_dir(&PathBuf::from("svn")));
        assert!(!bp.is_vcs_dir(&PathBuf::from(".config")));
        assert!(!bp.is_vcs_dir(&PathBuf::from(".local")));

        Ok(())
    }
}
