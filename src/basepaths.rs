use crate::command;
use crate::excluder;
use crate::gitignore;
use crate::vcs;
use failure::Error;
use itertools::Itertools;
use log::debug;
use regex::Regex;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use walkdir;

#[derive(Debug)]
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

#[derive(Debug, Fail)]
pub enum ConstructorError {
    #[fail(
        display = "You cannot pass an explicit list of files when looking for {}",
        mode
    )]
    GotPathsFromCLIWithWrongMode { mode: Mode },
}

#[derive(Debug, Fail)]
pub enum GitError {
    #[fail(display = "Not output when running git `{}`", args)]
    NoOutput { args: String },
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
                    return Err(ConstructorError::GotPathsFromCLIWithWrongMode { mode })?;
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

    pub fn paths(&self) -> Result<Vec<PathBuf>, Error> {
        let files = match self.mode {
            Mode::All => self.all_files()?,
            Mode::FromCLI => self.files_from_cli()?,
            Mode::GitModified => self.git_modified_files()?,
            Mode::GitStaged => self.git_staged_files()?,
        };

        let mut pat = String::from("^");
        pat.push_str(&self.root.to_string_lossy());
        pat.push('/');
        let re = Regex::new(pat.as_str())?;

        let repo_ignorer = gitignore::repo::Repo::new(&self.root)?;
        let mut paths: Vec<PathBuf> = vec![];
        for p in files {
            let rel = PathBuf::from(
                re.replace(p.to_string_lossy().into_owned().as_str(), "")
                    .into_owned(),
            );
            if repo_ignorer.is_ignored(&rel, fs::metadata(&p)?.is_dir()) {
                continue;
            }
            if self.excluder.path_is_excluded(&rel) {
                continue;
            }
            if self.is_vcs_dir(&rel) {
                continue;
            }
            paths.push(rel);
        }

        Ok(paths
            .drain(..)
            .sorted_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()))
            .collect())
    }

    fn all_files(&self) -> Result<Vec<PathBuf>, Error> {
        debug!("Getting all files under {}", self.root.to_string_lossy());
        self.walkdir_files(&self.root)
    }

    fn files_from_cli(&self) -> Result<Vec<PathBuf>, Error> {
        debug!("Using the list of files passed from the command line");
        let mut files: Vec<PathBuf> = vec![];
        // We need to canonicalize these paths so we can strip the repo root
        // from the front of them. Otherwise when given a path on the CLI like
        // "." we end up with paths like "./src", versus the all-files mode
        // where we'd have "src".
        for f in self.cli_paths.iter().map(|a| a.canonicalize()) {
            match f {
                Ok(f) => {
                    let meta = fs::metadata(&f)?;
                    if meta.is_dir() {
                        files.append(self.walkdir_files(&f)?.as_mut());
                    } else {
                        files.push(f);
                    }
                }
                Err(f) => return Err(f)?,
            }
        }
        Ok(files)
    }

    fn walkdir_files(&self, root: &PathBuf) -> Result<Vec<PathBuf>, Error> {
        let mut files: Vec<PathBuf> = vec![];
        for e in walkdir::WalkDir::new(root).into_iter() {
            match e {
                Ok(e) => {
                    if e.file_type().is_file() {
                        files.push(e.into_path())
                    }
                }
                Err(e) => return Err(e)?,
            }
        }
        Ok(files)
    }

    fn git_modified_files(&self) -> Result<Vec<PathBuf>, Error> {
        debug!("Getting modified files according to git");
        Self::files_from_git(&["diff", "--name-only", "--diff-filter=ACM"])
    }

    fn git_staged_files(&self) -> Result<Vec<PathBuf>, Error> {
        debug!("Getting staged files according to git");
        Self::files_from_git(&["diff", "--cached", "--name-only", "--diff-filter=ACM"])
    }

    fn files_from_git(args: &[&str]) -> Result<Vec<PathBuf>, Error> {
        let mut ok_exit_codes: HashSet<i32> = HashSet::with_capacity(1);
        ok_exit_codes.insert(0);

        let result = command::run_command(
            String::from("git"),
            args.iter().map(|a| String::from(*a)).collect(),
            &ok_exit_codes,
            false,
        )?;

        match result.stdout {
            Some(s) => Ok(s.lines().map(PathBuf::from).collect()),
            _ => Err(GitError::NoOutput {
                args: args.join(" "),
            })?,
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
}
