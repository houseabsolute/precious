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
    exclude_globs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
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
        exclude_globs: Vec<String>,
    ) -> Result<BasePaths, Error> {
        match mode {
            Mode::FromCLI => (),
            _ => {
                if !cli_paths.is_empty() {
                    return Err(BasePathsError::GotPathsFromCLIWithWrongMode { mode })?;
                }
            }
        };
        Ok(BasePaths {
            mode,
            cli_paths,
            root,
            exclude_globs,
        })
    }

    pub fn paths(&self) -> Result<Option<Vec<Paths>>, Error> {
        let files = match self.mode {
            Mode::All => self.all_files()?,
            Mode::FromCLI => self.files_from_cli()?,
            Mode::GitModified => self.git_modified_files()?,
            Mode::GitStaged => self.git_staged_files()?,
        };

        if files.is_none() {
            return Ok(None);
        }

        self.files_to_paths(files.unwrap())
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
        let mut excludes = ignore::overrides::OverrideBuilder::new(root);
        for e in self.exclude_globs.clone() {
            excludes.add(format!("!{}", e).as_ref())?;
        }
        for d in vcs::dirs() {
            excludes.add(format!("!{}/**/*", d).as_ref())?;
        }

        let mut files: Vec<PathBuf> = vec![];
        for result in ignore::WalkBuilder::new(root)
            .hidden(false)
            .overrides(excludes.build()?)
            .build()
        {
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

        let mut globs = self.exclude_globs.clone();
        let mut v = vcs::dirs().clone();
        globs.append(&mut v);
        let excluder = excluder::Excluder::new(globs.as_ref())?;
        match result.stdout {
            Some(s) => Ok(Some(
                s.lines()
                    .filter_map(|rel| {
                        if excluder.path_is_excluded(&PathBuf::from(rel)) {
                            return None;
                        }

                        let mut f = self.root.clone();
                        f.push(rel);
                        Some(f)
                    })
                    .collect(),
            )),
            None => Ok(None),
        }
    }

    fn files_to_paths(&self, files: Vec<PathBuf>) -> Result<Option<Vec<Paths>>, Error> {
        let mut entries: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();

        for f in self.filtered_relative_files(files)? {
            let dir = f.parent().unwrap().to_path_buf();
            if entries.contains_key(&dir) {
                entries.get_mut(&dir).unwrap().push(f.clone());
            } else {
                let files: Vec<PathBuf> = vec![f.clone()];
                entries.insert(dir, files);
            }
        }

        if entries.is_empty() {
            return Err(BasePathsError::AllPathsWereExcluded {
                mode: self.mode.clone(),
            })?;
        }

        Ok(Some(
            entries
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
                .collect(),
        ))
    }

    fn filtered_relative_files(&self, files: Vec<PathBuf>) -> Result<Vec<PathBuf>, Error> {
        let mut pat = String::from("^");
        pat.push_str(&self.root.to_string_lossy());
        pat.push('/');
        let root_path_re = Regex::new(pat.as_str())?;

        let mut cleaned: Vec<PathBuf> = vec![];

        for mut f in files {
            // We want to make all files absolute so we can canonicalize them
            // and _then_ strip off the root prefix. This lets us consistently
            // produce path names starting with "./".
            if !f.is_absolute() {
                f = self.root.clone().join(f);
            }

            let mut rel_str = root_path_re
                .replace(&f.canonicalize()?.to_string_lossy(), "./")
                .into_owned();
            if !rel_str.starts_with("./") {
                rel_str.insert_str(0, "./");
            }
            let rel = PathBuf::from(rel_str);

            cleaned.push(rel);
        }

        Ok(cleaned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testhelper;
    use spectral::prelude::*;

    fn new_basepaths(mode: Mode, paths: Vec<PathBuf>, root: PathBuf) -> Result<BasePaths, Error> {
        new_basepaths_with_excludes(mode, paths, root, vec![])
    }

    fn new_basepaths_with_excludes(
        mode: Mode,
        paths: Vec<PathBuf>,
        root: PathBuf,
        exclude: Vec<String>,
    ) -> Result<BasePaths, Error> {
        BasePaths::new(mode, paths, root, exclude)
    }

    #[test]
    fn files_to_paths() -> Result<(), Error> {
        let root = testhelper::create_git_repo()?;
        let mut files: Vec<PathBuf> = vec![];
        for p in testhelper::paths().iter() {
            files.push(PathBuf::from(p));
        }

        let bp = new_basepaths(Mode::All, vec![], root.path().to_owned())?;
        let paths = bp.files_to_paths(files)?.unwrap();
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
    fn files_to_paths_prefix() -> Result<(), Error> {
        let root = testhelper::create_git_repo()?;

        for p in vec![
            "./src/bar.rs",
            "src/bar.rs",
            root.path()
                .join("src/bar.rs")
                .to_string_lossy()
                .into_owned()
                .as_str(),
        ] {
            let cli_paths = vec![PathBuf::from(p)];
            let bp = new_basepaths(Mode::FromCLI, cli_paths.clone(), root.path().to_owned())?;
            let paths = bp.files_to_paths(cli_paths)?.unwrap();

            assert_that(&paths.len())
                .named("got one paths entry")
                .is_equal_to(1);
            assert_that(&paths[0]).is_equal_to(Paths {
                dir: PathBuf::from("./src"),
                files: ["./src/bar.rs"].iter().map(PathBuf::from).collect(),
            });
        }

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
        assert_that(&res).is_ok();
        assert_that(&res.unwrap()).is_none();
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
                .sorted_by(|a, b| a.cmp(b))
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_that(&bp.paths()?).is_equal_to(expect);
        Ok(())
    }

    #[test]
    fn git_modified_mode_with_excluded_files() -> Result<(), Error> {
        let root = testhelper::create_git_repo()?;
        testhelper::write_file(&root, "vendor/foo/bar.txt", "initial content")?;
        testhelper::stage_all_in(&root)?;
        testhelper::commit_all_in(&root)?;

        let modified = testhelper::modify_files(&root)?;
        testhelper::write_file(&root, "vendor/foo/bar.txt", "new content")?;
        let bp = new_basepaths_with_excludes(
            Mode::GitModified,
            vec![],
            root.path().to_owned(),
            vec!["vendor/**/*".to_string()],
        )?;
        let expect = bp.files_to_paths(
            modified
                .iter()
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
        assert_that(&res).is_ok();
        assert_that(&res.unwrap()).is_none();
        Ok(())
    }

    #[test]
    fn git_staged_mode_with_changes() -> Result<(), Error> {
        let root = testhelper::create_git_repo()?;
        let modified = testhelper::modify_files(&root)?;
        let bp = new_basepaths(Mode::GitStaged, vec![], root.path().to_owned())?;
        let res = bp.paths();
        assert_that(&res).is_ok();
        assert_that(&res.unwrap()).is_none();

        testhelper::stage_all_in(&root)?;
        let expect = bp.files_to_paths(
            modified
                .iter()
                .sorted_by(|a, b| a.cmp(b))
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_that(&bp.paths()?).is_equal_to(expect);

        Ok(())
    }

    #[test]
    fn git_staged_mode_with_excluded_files() -> Result<(), Error> {
        let root = testhelper::create_git_repo()?;
        let modified = testhelper::modify_files(&root)?;
        testhelper::write_file(&root, "vendor/foo/bar.txt", "initial content")?;
        testhelper::stage_all_in(&root)?;
        let bp = new_basepaths_with_excludes(
            Mode::GitStaged,
            vec![],
            root.path().to_owned(),
            vec!["vendor/**/*".to_string()],
        )?;
        let expect = bp.files_to_paths(
            modified
                .iter()
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
}
