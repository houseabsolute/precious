use crate::command;
use crate::excluder;
use failure::Error;
use globset::{Glob, GlobSet, GlobSetBuilder};
use log::{debug, info};
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub enum FilterType {
    Lint,
    Tidy,
    Both,
}

impl FilterType {
    fn what(&self) -> &'static str {
        match self {
            FilterType::Lint => "lint",
            FilterType::Tidy => "tidier",
            FilterType::Both => "linter/tidier",
        }
    }
}

#[derive(Debug, Fail)]
enum FilterError {
    #[fail(display = "You cannot create a Command which lints and tidies without a lint_flag")]
    CommandWhichIsBothRequiresLintFlag,

    #[fail(
        display = "You can only pass paths to files to the {} method for this filter, you passed {}",
        method, path
    )]
    CanOnlyOperateOnFiles { method: &'static str, path: String },

    #[fail(
        display = "You can only pass paths to directories to the {} method for this filter, you passed {}",
        method, path
    )]
    CanOnlyOperateOnDirectories { method: &'static str, path: String },

    #[fail(
        display = "This {} is a {}. You cannot call {}() on it.",
        what, typ, method
    )]
    CannotX {
        what: &'static str,
        typ: &'static str,
        method: &'static str,
    },
}

pub struct Filter {
    root: PathBuf,
    pub name: String,
    typ: FilterType,
    includer: Includer,
    excluder: excluder::Excluder,
    pub on_dir: bool,
    implementation: Box<dyn FilterImplementation>,
}

// This should be safe because we never mutate the Filter struct in any of its
// methods.
unsafe impl Sync for Filter {}

impl fmt::Debug for Filter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // I'm not sure how to get any useful info for the implementation
        // field so we'll just leave it out for now.
        write!(
            f,
            "{{ root: {:?}, name: {:?}, typ: {:?}, includer: {:?}, excluder: {:?}, on_dir: {:?}",
            self.root, self.name, self.typ, self.includer, self.excluder, self.on_dir,
        )
    }
}
#[derive(Debug)]
pub struct LintResult {
    pub ok: bool,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

pub trait FilterImplementation {
    fn tidy(&self, name: &str, path: &PathBuf) -> Result<(), Error>;
    fn lint(&self, name: &str, path: &PathBuf) -> Result<LintResult, Error>;
}

impl Filter {
    pub fn tidy(&self, path: &PathBuf) -> Result<bool, Error> {
        self.require_is_not_filter_type(FilterType::Lint)?;

        let mut full = self.root.clone();
        full.push(path.clone());

        self.require_path_type("tidy", &full)?;

        if !self.should_process_file(path.clone())? {
            return Ok(false);
        }

        let mtime = Self::mtime_for(&full)?;
        self.implementation.tidy(&self.name, path)?;
        Ok(mtime != Self::mtime_for(&full)?)
    }

    pub fn lint(&self, path: PathBuf) -> Result<LintResult, Error> {
        self.require_is_not_filter_type(FilterType::Tidy)?;

        let mut full = self.root.clone();
        full.push(path.clone());

        self.require_path_type("lint", &full)?;

        if !self.should_process_file(path.clone())? {
            return Ok(LintResult {
                ok: true,
                stdout: None,
                stderr: None,
            });
        }

        self.implementation.lint(&self.name, &path)
    }

    fn require_is_not_filter_type(&self, not_allowed: FilterType) -> Result<(), Error> {
        if std::mem::discriminant(&not_allowed) == std::mem::discriminant(&self.typ) {
            return Err(FilterError::CannotX {
                what: "command",
                typ: self.typ.what(),
                method: "tidy",
            })?;
        }
        Ok(())
    }

    fn require_path_type(&self, method: &'static str, path: &PathBuf) -> Result<(), Error> {
        let is_dir = fs::metadata(path)?.is_dir();
        if self.on_dir && !is_dir {
            return Err(FilterError::CanOnlyOperateOnDirectories {
                method,
                path: path.to_string_lossy().to_string(),
            })?;
        } else if is_dir && !self.on_dir {
            return Err(FilterError::CanOnlyOperateOnFiles {
                method,
                path: path.to_string_lossy().to_string(),
            })?;
        }
        Ok(())
    }

    fn should_process_file(&self, path: PathBuf) -> Result<bool, Error> {
        if self.excluder.path_is_excluded(&path)? {
            debug!(
                "Path {} is excluded for the {} filter",
                path.to_string_lossy(),
                self.name
            );
            return Ok(false);
        }

        if !self.includer.path_is_included(&path) {
            debug!(
                "Path {} is not included in the {} filter",
                path.to_string_lossy(),
                self.name
            );
            return Ok(false);
        }

        Ok(true)
    }

    fn mtime_for(path: &PathBuf) -> Result<SystemTime, Error> {
        let meta = fs::metadata(path)?;
        let mtime = meta.modified();
        match mtime {
            Ok(mtime) => Ok(mtime),
            Err(e) => Err(e)?,
        }
    }
}

#[derive(Debug)]
pub struct Command {
    cmd: Vec<String>,
    lint_flag: String,
    path_flag: String,
    ok_exit_codes: HashSet<i32>,
    lint_failure_exit_codes: HashSet<i32>,
    expect_stderr: bool,
}

impl Command {
    pub fn build(
        root: &PathBuf,
        name: String,
        typ: FilterType,
        include: Vec<String>,
        ignore: Vec<String>,
        exclude: Vec<String>,
        on_dir: bool,
        cmd: Vec<String>,
        lint_flag: String,
        path_flag: String,
        ok_exit_codes: Vec<u8>,
        lint_failure_exit_codes: Vec<u8>,
        expect_stderr: bool,
    ) -> Result<Filter, Error> {
        if let FilterType::Both = typ {
            if lint_flag == "" {
                return Err(FilterError::CommandWhichIsBothRequiresLintFlag)?;
            }
        }

        Ok(Filter {
            root: root.clone(),
            name,
            typ,
            includer: Includer::new(&include)?,
            excluder: excluder::Excluder::new(root, &ignore, &exclude)?,
            on_dir,
            implementation: Box::new(Command {
                cmd: replace_root(cmd, root),
                lint_flag,
                path_flag,
                ok_exit_codes: Self::exit_codes_hashset(ok_exit_codes),
                lint_failure_exit_codes: Self::exit_codes_hashset(lint_failure_exit_codes),
                expect_stderr,
            }),
        })
    }

    fn exit_codes_hashset(ok_exit_codes: Vec<u8>) -> HashSet<i32> {
        let mut hash: HashSet<i32> = HashSet::with_capacity(ok_exit_codes.len());
        for c in ok_exit_codes {
            hash.insert(i32::from(c));
        }
        hash
    }
}

impl FilterImplementation for Command {
    fn tidy(&self, name: &str, path: &PathBuf) -> Result<(), Error> {
        let mut cmd = self.cmd.clone();
        if self.path_flag != "" {
            cmd.push(self.path_flag.clone());
        }
        cmd.push(path.to_string_lossy().to_string());

        info!(
            "Tidying {} with {} command: {}",
            path.to_string_lossy(),
            name,
            cmd.join(" "),
        );

        match command::run_command(cmd.remove(0), cmd, &self.ok_exit_codes, self.expect_stderr) {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn lint(&self, name: &str, path: &PathBuf) -> Result<LintResult, Error> {
        let mut cmd = self.cmd.clone();
        if self.lint_flag != "" {
            cmd.push(self.lint_flag.clone());
        }
        if self.path_flag != "" {
            cmd.push(self.path_flag.clone());
        }
        cmd.push(path.to_string_lossy().to_string());

        info!(
            "Linting {} with {} command: {}",
            path.to_string_lossy(),
            name,
            cmd.join(" "),
        );

        match command::run_command(cmd.remove(0), cmd, &self.ok_exit_codes, self.expect_stderr) {
            Ok(result) => Ok(LintResult {
                ok: !self.lint_failure_exit_codes.contains(&result.exit_code),
                stdout: result.stdout,
                stderr: result.stderr,
            }),
            Err(e) => Err(e),
        }
    }
}

#[derive(Debug)]
pub struct Server {
    name: String,
    typ: FilterType,
    include: GlobSet,
    excluder: excluder::Excluder,
    cmd: Vec<String>,
    on_dir: bool,
    port: u16,
}

fn replace_root(cmd: Vec<String>, root: &PathBuf) -> Vec<String> {
    cmd.iter()
        .map(|c| {
            c.replace(
                "$PRECIOUS_ROOT",
                root.to_string_lossy().into_owned().as_str(),
            )
        })
        .collect()
}

#[derive(Debug)]
struct Includer {
    include: GlobSet,
}

impl Includer {
    fn new(globs: &[String]) -> Result<Includer, Error> {
        let mut builder = GlobSetBuilder::new();
        for g in globs {
            builder.add(Glob::new(g.as_str())?);
        }
        Ok(Includer {
            include: builder.build()?,
        })
    }

    fn path_is_included(&self, path: &PathBuf) -> bool {
        self.include.is_match(path)
    }
}
