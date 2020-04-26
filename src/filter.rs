use crate::command;
use crate::path_matcher;
use failure::Error;
use log::{debug, info};
use md5;
use std::collections::{HashMap, HashSet};
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

#[derive(Clone, Debug)]
pub enum RunMode {
    Files,
    Dirs,
    Root,
}

#[derive(Debug, Fail)]
enum FilterError {
    #[fail(
        display = "You cannot create a Command which lints and tidies without lint_flags and/or tidy_flags"
    )]
    CommandWhichIsBothRequiresLintOrTidyFlags,

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

    #[fail(
        display = "Cannot compare previous state of {} to its current state because we did not record its previous state!",
        path
    )]
    CannotComparePaths { path: String },
}

pub struct Filter {
    root: PathBuf,
    pub name: String,
    typ: FilterType,
    includer: path_matcher::Matcher,
    excluder: path_matcher::Matcher,
    pub run_mode: RunMode,
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
            "{{ root: {:?}, name: {:?}, typ: {:?}, includer: {:?}, excluder: {:?}, run_mode: {:?} }}",
            self.root, self.name, self.typ, self.includer, self.excluder, self.run_mode,
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

#[derive(Debug)]
struct PathInfo {
    mtime: SystemTime,
    size: u64,
    hash: md5::Digest,
}

fn run_mode_is(mode1: &RunMode, mode2: &RunMode) -> bool {
    return std::mem::discriminant(mode1) == std::mem::discriminant(mode2);
}

impl Filter {
    pub fn tidy(&self, path: &PathBuf, files: &[PathBuf]) -> Result<Option<bool>, Error> {
        self.require_is_not_filter_type(FilterType::Lint)?;

        let mut full = self.root.clone();
        full.push(path.clone());

        self.require_path_type("tidy", &full)?;

        if !self.should_process_path(path, files)? {
            return Ok(None);
        }

        let info = Self::path_info_map_for(&full)?;
        self.implementation.tidy(&self.name, path)?;
        Ok(Some(Self::path_was_changed(&full, &info)?))
    }

    pub fn lint(&self, path: &PathBuf, files: &[PathBuf]) -> Result<Option<LintResult>, Error> {
        self.require_is_not_filter_type(FilterType::Tidy)?;

        let mut full = self.root.clone();
        full.push(path.clone());

        self.require_path_type("lint", &full)?;

        if !self.should_process_path(path, files)? {
            return Ok(None);
        }

        let r = self.implementation.lint(&self.name, &path)?;
        Ok(Some(r))
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
        if self.run_mode_is(RunMode::Root) {
            return Ok(());
        }

        let is_dir = fs::metadata(path)?.is_dir();
        if self.run_mode_is(RunMode::Dirs) && !is_dir {
            return Err(FilterError::CanOnlyOperateOnDirectories {
                method,
                path: path.to_string_lossy().to_string(),
            })?;
        } else if self.run_mode_is(RunMode::Files) && is_dir {
            return Err(FilterError::CanOnlyOperateOnFiles {
                method,
                path: path.to_string_lossy().to_string(),
            })?;
        }
        Ok(())
    }

    pub fn run_mode_is(&self, mode: RunMode) -> bool {
        return run_mode_is(&self.run_mode, &mode);
    }

    fn should_process_path(&self, path: &PathBuf, files: &[PathBuf]) -> Result<bool, Error> {
        if self.excluder.path_matches(path) {
            debug!(
                "Path {} is excluded for the {} filter",
                path.to_string_lossy(),
                self.name,
            );
            return Ok(false);
        }

        if self.includer.path_matches(path) {
            debug!(
                "Path {} is included in the {} filter",
                path.to_string_lossy(),
                self.name
            );
            return Ok(true);
        }

        if !self.run_mode_is(RunMode::Files) {
            for f in files {
                if self.excluder.path_matches(f) {
                    continue;
                }

                if self.includer.path_matches(f) {
                    debug!(
                        "Directory {} is included in the {} filter because it contains {} which is included",
                        path.to_string_lossy(),
                        self.name,
                        f.to_string_lossy(),
                    );
                    return Ok(true);
                }
            }
            debug!(
                "Directory {} is not included in the {} filter because neither it nor its files are included",
                path.to_string_lossy(),
                self.name
            );
            return Ok(false);
        }

        debug!(
            "Path {} is not included in the {} filter",
            path.to_string_lossy(),
            self.name
        );
        Ok(false)
    }

    fn path_was_changed(path: &PathBuf, prev: &HashMap<PathBuf, PathInfo>) -> Result<bool, Error> {
        let meta = fs::metadata(path)?;
        if meta.is_file() {
            if !prev.contains_key(path) {
                return Err(FilterError::CannotComparePaths {
                    path: path.to_string_lossy().to_string(),
                })?;
            }
            let prev_info = prev.get(path).unwrap();
            // If the mtime is unchanged we don't need to compare anything
            // else. Unfortunately there's no guarantee a filter won't modify
            // the mtime even if it doesn't change the file's contents. For
            // example, Perl::Tidy does this :(
            if prev_info.mtime == meta.modified()? {
                return Ok(false);
            }

            // If the size changed we know the contents changed.
            if prev_info.size != meta.len() {
                return Ok(true);
            }

            // Otherwise we need to compare the content hash.
            return Ok(prev_info.hash != md5::compute(fs::read(path)?));
        }

        for entry in path.read_dir()? {
            match entry {
                Ok(e) => {
                    if !e.metadata()?.is_dir() {
                        if prev.contains_key(&e.path()) {
                            if Self::path_was_changed(&e.path(), &prev)? {
                                return Ok(true);
                            }
                        } else {
                            // We can only assume that when an entry is not
                            // found in the previous hash that the filter must
                            // have added a new file.
                            return Ok(true);
                        }
                    }
                }
                Err(e) => Err(e)?,
            }
        }
        Ok(false)
    }

    fn path_info_map_for(path: &PathBuf) -> Result<HashMap<PathBuf, PathInfo>, Error> {
        let meta = fs::metadata(path)?;
        if meta.is_dir() {
            let mut info = HashMap::new();
            for entry in path.read_dir()? {
                match entry {
                    Ok(e) => {
                        // We do not recurse into subdirs. Our assumption is
                        // that filters which operate on a dir do not recurse
                        // either (thinking of things like golint, etc.).
                        if !e.metadata()?.is_dir() {
                            for (k, v) in Self::path_info_map_for(&e.path())?.drain() {
                                info.insert(k.clone(), v);
                            }
                        }
                    }
                    Err(e) => return Err(e)?,
                }
            }
            return Ok(info);
        }

        let mut info = HashMap::new();
        info.insert(
            path.clone(),
            PathInfo {
                mtime: meta.modified()?,
                size: meta.len(),
                hash: md5::compute(fs::read(path)?),
            },
        );
        Ok(info)
    }
}

#[derive(Debug)]
pub struct Command {
    cmd: Vec<String>,
    env: HashMap<String, String>,
    chdir: bool,
    lint_flags: Vec<String>,
    tidy_flags: Vec<String>,
    path_flag: String,
    ok_exit_codes: HashSet<i32>,
    lint_failure_exit_codes: HashSet<i32>,
    run_mode: RunMode,
    expect_stderr: bool,
}

pub struct CommandParams {
    pub root: PathBuf,
    pub name: String,
    pub typ: FilterType,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub run_mode: RunMode,
    pub chdir: bool,
    pub cmd: Vec<String>,
    pub env: HashMap<String, String>,
    pub lint_flags: Vec<String>,
    pub tidy_flags: Vec<String>,
    pub path_flag: String,
    pub ok_exit_codes: Vec<u8>,
    pub lint_failure_exit_codes: Vec<u8>,
    pub expect_stderr: bool,
}

impl Command {
    pub fn build(params: CommandParams) -> Result<Filter, Error> {
        if let FilterType::Both = params.typ {
            if params.lint_flags.is_empty() && params.tidy_flags.is_empty() {
                return Err(FilterError::CommandWhichIsBothRequiresLintOrTidyFlags)?;
            }
        }

        Ok(Filter {
            root: params.root.clone(),
            name: params.name,
            typ: params.typ,
            includer: path_matcher::Matcher::new(&params.include)?,
            excluder: path_matcher::Matcher::new(&params.exclude)?,
            run_mode: params.run_mode.clone(),
            implementation: Box::new(Command {
                cmd: replace_root(params.cmd, &params.root),
                env: params.env,
                chdir: params.chdir,
                lint_flags: params.lint_flags,
                tidy_flags: params.tidy_flags,
                path_flag: params.path_flag,
                ok_exit_codes: Self::exit_codes_hashset(
                    &params.ok_exit_codes,
                    Some(&params.lint_failure_exit_codes),
                ),
                lint_failure_exit_codes: Self::exit_codes_hashset(
                    &params.lint_failure_exit_codes,
                    None,
                ),
                run_mode: params.run_mode,
                expect_stderr: params.expect_stderr,
            }),
        })
    }

    fn exit_codes_hashset(
        ok_exit_codes: &[u8],
        lint_failure_exit_codes: Option<&[u8]>,
    ) -> HashSet<i32> {
        let mut len = ok_exit_codes.len();
        if lint_failure_exit_codes.is_some() {
            len += lint_failure_exit_codes.unwrap().len();
        }
        let mut hash: HashSet<i32> = HashSet::with_capacity(len);
        for c in ok_exit_codes {
            hash.insert(i32::from(*c));
        }
        if lint_failure_exit_codes.is_some() {
            for c in lint_failure_exit_codes.unwrap() {
                hash.insert(i32::from(*c));
            }
        }
        hash
    }

    fn in_dir(&self, path: &PathBuf) -> Option<PathBuf> {
        if !self.chdir {
            return None;
        }

        if path.is_dir() {
            return Some(path.clone());
        }

        Some(path.parent().unwrap().to_path_buf())
    }

    fn run_mode_is(&self, mode: RunMode) -> bool {
        return run_mode_is(&self.run_mode, &mode);
    }
}

impl FilterImplementation for Command {
    fn tidy(&self, name: &str, path: &PathBuf) -> Result<(), Error> {
        let mut cmd = self.cmd.clone();
        if !self.tidy_flags.is_empty() {
            cmd.append(&mut self.tidy_flags.clone());
        }
        if self.path_flag != "" {
            cmd.push(self.path_flag.clone());
        }
        if self.run_mode_is(RunMode::Files) || !self.chdir {
            cmd.push(path.to_string_lossy().to_string());
        }

        info!(
            "Tidying {} with {} command: {}",
            path.to_string_lossy(),
            name,
            cmd.join(" "),
        );

        match command::run_command(
            cmd.remove(0),
            cmd,
            &self.env,
            self.ok_exit_codes.iter().cloned().collect(),
            self.expect_stderr,
            self.in_dir(path).as_ref(),
        ) {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn lint(&self, name: &str, path: &PathBuf) -> Result<LintResult, Error> {
        let mut cmd = self.cmd.clone();
        if !self.lint_flags.is_empty() {
            cmd.append(&mut self.lint_flags.clone());
        }
        if self.path_flag != "" {
            cmd.push(self.path_flag.clone());
        }
        if self.run_mode_is(RunMode::Files) || !self.chdir {
            cmd.push(path.to_string_lossy().to_string());
        }

        info!(
            "Linting {} with {} command: {}",
            path.to_string_lossy(),
            name,
            cmd.join(" "),
        );

        match command::run_command(
            cmd.remove(0),
            cmd,
            &self.env,
            self.ok_exit_codes.iter().cloned().collect(),
            self.expect_stderr,
            self.in_dir(path).as_ref(),
        ) {
            Ok(result) => Ok(LintResult {
                ok: !self.lint_failure_exit_codes.contains(&result.exit_code),
                stdout: result.stdout,
                stderr: result.stderr,
            }),
            Err(e) => Err(e),
        }
    }
}

// #[derive(Debug)]
// pub struct Server {
//     name: String,
//     typ: FilterType,
//     include: GlobSet,
//     excluder: path_matcher::Matcher,
//     cmd: Vec<String>,
//     run_mode: RunMode,
//     port: u16,
// }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path_matcher;
    use crate::testhelper;
    use spectral::prelude::*;

    type Mock = i8;

    impl FilterImplementation for Mock {
        fn tidy(&self, _: &str, _: &PathBuf) -> Result<(), Error> {
            Ok(())
        }

        fn lint(&self, _: &str, _: &PathBuf) -> Result<LintResult, Error> {
            Ok(LintResult {
                ok: true,
                stdout: None,
                stderr: None,
            })
        }
    }

    fn mock_filter() -> Box<dyn FilterImplementation> {
        Box::new(1 as Mock)
    }

    fn matcher(globs: &[&str]) -> Result<path_matcher::Matcher, Error> {
        path_matcher::Matcher::new(
            &globs
                .iter()
                .map(|g| String::from(*g))
                .collect::<Vec<String>>(),
        )
    }

    #[test]
    fn require_path_type_dir() -> Result<(), Error> {
        let filter = Filter {
            root: PathBuf::from("/foo/bar"),
            name: String::from("Test"),
            typ: FilterType::Lint,
            includer: matcher(&[])?,
            excluder: matcher(&[])?,
            run_mode: RunMode::Dirs,
            implementation: mock_filter(),
        };

        let helper = testhelper::TestHelper::new()?;
        assert_that(&filter.require_path_type("tidy", &helper.root())).is_ok();
        assert_that(&filter.require_path_type("tidy", &helper.all_files()[0].clone())).is_err();

        Ok(())
    }

    #[test]
    fn require_path_type_file() -> Result<(), Error> {
        let filter = Filter {
            root: PathBuf::from("/foo/bar"),
            name: String::from("Test"),
            typ: FilterType::Lint,
            includer: matcher(&[])?,
            excluder: matcher(&[])?,
            run_mode: RunMode::Files,
            implementation: mock_filter(),
        };

        let helper = testhelper::TestHelper::new()?;
        assert_that(&filter.require_path_type("tidy", &helper.root())).is_err();
        assert_that(&filter.require_path_type("tidy", &helper.all_files()[0].clone())).is_ok();

        Ok(())
    }

    #[test]
    fn should_process_path() -> Result<(), Error> {
        let filter = Filter {
            root: PathBuf::from("/foo/bar"),
            name: String::from("Test"),
            typ: FilterType::Lint,
            includer: matcher(&["**/*.go"])?,
            excluder: matcher(&["foo/**/*", "baz/bar/**/quux/*"])?,
            run_mode: RunMode::Files,
            implementation: mock_filter(),
        };

        let include = &["something.go", "dir/foo.go", ".foo.go", "bar/foo/x.go"];
        for i in include.iter().map(PathBuf::from) {
            let name = i.clone();
            assert_that(&filter.should_process_path(&i.clone(), &[i])?)
                .named(&name.to_string_lossy())
                .is_true();
        }

        let exclude = &[
            "something.pl",
            "dir/foo.pl",
            "foo/bar.go",
            "baz/bar/anything/here/quux/file.go",
        ];
        for e in exclude.iter().map(PathBuf::from) {
            let name = e.clone();
            assert_that(&filter.should_process_path(&e.clone(), &[e])?)
                .named(&name.to_string_lossy())
                .is_false();
        }

        Ok(())
    }

    #[test]
    fn should_process_path_run_mode_dirs() -> Result<(), Error> {
        let filter = Filter {
            root: PathBuf::from("/foo/bar"),
            name: String::from("Test"),
            typ: FilterType::Lint,
            includer: matcher(&["**/*.go"])?,
            excluder: matcher(&["foo/**/*", "baz/bar/**/quux/*"])?,
            run_mode: RunMode::Dirs,
            implementation: mock_filter(),
        };

        let include = &[
            &[".", "foo.go", "README.md"],
            &["dir/foo", "dir/foo/foo.pl", "dir/foo/file.go"],
        ];
        for i in include.iter() {
            let dir = PathBuf::from(i[0]);
            let files = i[1..].iter().map(PathBuf::from).collect::<Vec<PathBuf>>();
            let name = dir.clone();
            assert_that(&filter.should_process_path(&dir, &files)?)
                .named(&name.to_string_lossy())
                .is_true();
        }

        let exclude = &[
            &["foo", "foo/bar.go", "foo/baz.go"],
            &[
                "baz/bar/foo/quux",
                "baz/bar/foo/quux/file.go",
                "baz/bar/foo/quux/other.go",
            ],
            &["dir", "dir/foo.pl", "dir/file.txt"],
        ];
        for e in exclude.iter() {
            let dir = PathBuf::from(e[0]);
            let files = e[1..].iter().map(PathBuf::from).collect::<Vec<PathBuf>>();
            let name = dir.clone();
            assert_that(&filter.should_process_path(&dir, &files)?)
                .named(&name.to_string_lossy())
                .is_false();
        }

        Ok(())
    }

    #[test]
    fn should_process_path_run_mode_root() -> Result<(), Error> {
        let filter = Filter {
            root: PathBuf::from("/foo/bar"),
            name: String::from("Test"),
            typ: FilterType::Lint,
            includer: matcher(&["**/*.go"])?,
            excluder: matcher(&["foo/**/*", "baz/bar/**/quux/*"])?,
            run_mode: RunMode::Root,
            implementation: mock_filter(),
        };

        let include = &[
            &[".", "foo.go", "README.md"],
            &["dir/foo", "dir/foo/foo.pl", "dir/foo/file.go"],
        ];
        for i in include.iter() {
            let dir = PathBuf::from(i[0]);
            let files = i[1..].iter().map(PathBuf::from).collect::<Vec<PathBuf>>();
            let name = dir.clone();
            assert_that(&filter.should_process_path(&dir, &files)?)
                .named(&name.to_string_lossy())
                .is_true();
        }

        let exclude = &[
            &["foo", "foo/bar.go", "foo/baz.go"],
            &[
                "baz/bar/foo/quux",
                "baz/bar/foo/quux/file.go",
                "baz/bar/foo/quux/other.go",
            ],
            &["dir", "dir/foo.pl", "dir/file.txt"],
        ];
        for e in exclude.iter() {
            let dir = PathBuf::from(e[0]);
            let files = e[1..].iter().map(PathBuf::from).collect::<Vec<PathBuf>>();
            let name = dir.clone();
            assert_that(&filter.should_process_path(&dir, &files)?)
                .named(&name.to_string_lossy())
                .is_false();
        }

        Ok(())
    }
}
