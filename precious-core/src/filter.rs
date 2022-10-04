use crate::path_matcher;
use anyhow::Result;
use log::{debug, info};
use precious_exec as exec;
use regex::Regex;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum FilterType {
    #[serde(rename = "lint")]
    Lint,
    #[serde(rename = "tidy")]
    Tidy,
    #[serde(rename = "both")]
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum RunMode {
    #[serde(rename = "files")]
    Files,
    #[serde(rename = "dirs")]
    Dirs,
    #[serde(rename = "root")]
    Root,
}

#[derive(Debug, Error)]
enum FilterError {
    #[error(
        "You cannot create a Command which lints and tidies without lint_flags and/or tidy_flags"
    )]
    CommandWhichIsBothRequiresLintOrTidyFlags,

    #[error(
        "You can only pass paths to files to the {method:} method for this filter, you passed {path:}"
    )]
    CanOnlyOperateOnFiles { method: &'static str, path: String },

    #[error(
        "You can only pass paths to directories to the {method:} method for this filter, you passed {path:}"
    )]
    CanOnlyOperateOnDirectories { method: &'static str, path: String },

    #[error("")]
    CannotX {
        what: &'static str,
        typ: &'static str,
        method: &'static str,
    },

    #[error(
        "Cannot compare previous state of {path:} to its current state because we did not record its previous state!"
    )]
    CannotComparePaths { path: String },
}

#[derive(Debug)]
pub struct Filter {
    root: PathBuf,
    pub name: String,
    typ: FilterType,
    includer: path_matcher::Matcher,
    excluder: path_matcher::Matcher,
    pub run_mode: RunMode,
    chdir: bool,
    cmd: Vec<String>,
    env: HashMap<String, String>,
    lint_flags: Option<Vec<String>>,
    tidy_flags: Option<Vec<String>>,
    path_flag: Option<String>,
    ok_exit_codes: HashSet<i32>,
    lint_failure_exit_codes: HashSet<i32>,
    ignore_stderr: Option<Vec<Regex>>,
}

pub struct FilterParams {
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
    pub ignore_stderr: Vec<String>,
}

#[derive(Debug)]
pub struct LintOutcome {
    pub ok: bool,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

#[derive(Debug)]
struct PathInfo {
    mtime: SystemTime,
    size: u64,
    hash: md5::Digest,
}

// This should be safe because we never mutate the Filter struct in any of its
// methods.
unsafe impl Sync for Filter {}

impl Filter {
    pub fn build(params: FilterParams) -> Result<Filter> {
        if let FilterType::Both = params.typ {
            if params.lint_flags.is_empty() && params.tidy_flags.is_empty() {
                return Err(FilterError::CommandWhichIsBothRequiresLintOrTidyFlags.into());
            }
        }

        let ignore_stderr = if params.expect_stderr {
            Some(vec![Regex::new(".*").unwrap()])
        } else if params.ignore_stderr.is_empty() {
            None
        } else {
            Some(
                params
                    .ignore_stderr
                    .into_iter()
                    .map(|i| Regex::new(&i).map_err(|e| e.into()))
                    .collect::<Result<Vec<_>>>()?,
            )
        };

        let cmd = replace_root(params.cmd, &params.root);
        Ok(Filter {
            root: params.root,
            name: params.name,
            typ: params.typ,
            includer: path_matcher::MatcherBuilder::new()
                .with(&params.include)?
                .build()?,
            excluder: path_matcher::MatcherBuilder::new()
                .with(&params.exclude)?
                .build()?,
            run_mode: params.run_mode,
            cmd,
            env: params.env,
            chdir: params.chdir,
            lint_flags: if params.lint_flags.is_empty() {
                None
            } else {
                Some(params.lint_flags)
            },
            tidy_flags: if params.tidy_flags.is_empty() {
                None
            } else {
                Some(params.tidy_flags)
            },
            path_flag: if params.path_flag.is_empty() {
                None
            } else {
                Some(params.path_flag)
            },
            ok_exit_codes: Self::exit_codes_hashset(
                &params.ok_exit_codes,
                Some(&params.lint_failure_exit_codes),
            ),
            lint_failure_exit_codes: Self::exit_codes_hashset(
                &params.lint_failure_exit_codes,
                None,
            ),
            ignore_stderr,
        })
    }

    pub fn tidy(&self, path: &Path, files: &[PathBuf]) -> Result<Option<bool>> {
        self.require_is_not_filter_type(FilterType::Lint)?;

        let mut full = self.root.clone();
        full.push(path);

        self.require_path_type("tidy", &full)?;

        if !self.should_process_path(path, files) {
            return Ok(None);
        }

        let info = Self::path_info_map_for(&full)?;
        let mut cmd = self.command_for_path(path, &self.tidy_flags);

        info!(
            "Tidying {} with {} command: {}",
            path.display(),
            self.name,
            cmd.join(" "),
        );

        let ok_exit_codes: Vec<i32> = self.ok_exit_codes.iter().cloned().collect();
        let bin = cmd.remove(0);
        exec::run(
            &bin,
            cmd.iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>()
                .as_slice(),
            &self.env,
            &ok_exit_codes,
            self.ignore_stderr.as_deref(),
            self.in_dir(path),
        )?;
        Ok(Some(Self::path_was_changed(&full, &info)?))
    }

    pub fn lint(&self, path: &Path, files: &[PathBuf]) -> Result<Option<LintOutcome>> {
        self.require_is_not_filter_type(FilterType::Tidy)?;

        let mut full = self.root.clone();
        full.push(path);

        self.require_path_type("lint", &full)?;

        if !self.should_process_path(path, files) {
            return Ok(None);
        }

        let mut cmd = self.command_for_path(path, &self.lint_flags);

        info!(
            "Linting {} with {} command: {}",
            path.display(),
            self.name,
            cmd.join(" "),
        );

        let ok_exit_codes: Vec<i32> = self.ok_exit_codes.iter().cloned().collect();
        let bin = cmd.remove(0);
        let result = exec::run(
            &bin,
            cmd.iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>()
                .as_slice(),
            &self.env,
            &ok_exit_codes,
            self.ignore_stderr.as_deref(),
            self.in_dir(path),
        )?;

        Ok(Some(LintOutcome {
            ok: !self.lint_failure_exit_codes.contains(&result.exit_code),
            stdout: result.stdout,
            stderr: result.stderr,
        }))
    }

    fn require_is_not_filter_type(&self, not_allowed: FilterType) -> Result<()> {
        if std::mem::discriminant(&not_allowed) == std::mem::discriminant(&self.typ) {
            return Err(FilterError::CannotX {
                what: "command",
                typ: self.typ.what(),
                method: "tidy",
            }
            .into());
        }
        Ok(())
    }

    fn require_path_type(&self, method: &'static str, path: &Path) -> Result<()> {
        if self.run_mode_is(RunMode::Root) {
            return Ok(());
        }

        let is_dir = fs::metadata(path)?.is_dir();
        if self.run_mode_is(RunMode::Dirs) && !is_dir {
            return Err(FilterError::CanOnlyOperateOnDirectories {
                method,
                path: path.to_string_lossy().to_string(),
            }
            .into());
        } else if self.run_mode_is(RunMode::Files) && is_dir {
            return Err(FilterError::CanOnlyOperateOnFiles {
                method,
                path: path.to_string_lossy().to_string(),
            }
            .into());
        }
        Ok(())
    }

    pub fn run_mode_is(&self, mode: RunMode) -> bool {
        self.run_mode == mode
    }

    fn should_process_path(&self, path: &Path, files: &[PathBuf]) -> bool {
        if self.excluder.path_matches(path) {
            debug!(
                "Path {} is excluded for the {} filter",
                path.display(),
                self.name,
            );
            return false;
        }

        if self.includer.path_matches(path) {
            debug!(
                "Path {} is included in the {} filter",
                path.display(),
                self.name
            );
            return true;
        }

        if !self.run_mode_is(RunMode::Files) {
            for f in files {
                if self.excluder.path_matches(f) {
                    continue;
                }

                if self.includer.path_matches(f) {
                    debug!(
                        "Directory {} is included in the {} filter because it contains {} which is included",
                        path.display(),
                        self.name,
                        f.display(),
                    );
                    return true;
                }
            }
            debug!(
                "Directory {} is not included in the {} filter because neither it nor its files are included",
                path.display(),
                self.name
            );
            return false;
        }

        debug!(
            "Path {} is not included in the {} filter",
            path.display(),
            self.name
        );
        false
    }

    fn path_was_changed(path: &Path, prev: &HashMap<PathBuf, PathInfo>) -> Result<bool> {
        let meta = fs::metadata(path)?;
        if meta.is_file() {
            if !prev.contains_key(path) {
                return Err(FilterError::CannotComparePaths {
                    path: path.to_string_lossy().to_string(),
                }
                .into());
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
            if let Err(e) = entry {
                return Err(e.into());
            }

            let e = entry.unwrap();
            if e.metadata()?.is_dir() {
                continue;
            }

            if prev.contains_key(&e.path()) && Self::path_was_changed(&e.path(), prev)? {
                return Ok(true);
            }

            // We can only assume that when an entry is not found in the
            // previous hash that the filter must have added a new file.
            return Ok(true);
        }
        Ok(false)
    }

    fn path_info_map_for(path: &Path) -> Result<HashMap<PathBuf, PathInfo>> {
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
                    Err(e) => return Err(e.into()),
                }
            }
            return Ok(info);
        }

        let mut info = HashMap::new();
        info.insert(
            path.to_owned(),
            PathInfo {
                mtime: meta.modified()?,
                size: meta.len(),
                hash: md5::compute(fs::read(path)?),
            },
        );
        Ok(info)
    }

    pub fn config_key(&self) -> String {
        format!("commands.{}", Self::maybe_toml_quote(&self.name),)
    }

    fn maybe_toml_quote(name: &str) -> String {
        if name.contains(' ') {
            return format!(r#""{}""#, name);
        }
        name.to_string()
    }

    fn exit_codes_hashset(
        ok_exit_codes: &[u8],
        lint_failure_exit_codes: Option<&[u8]>,
    ) -> HashSet<i32> {
        let mut len = ok_exit_codes.len();
        if let Some(lfec) = lint_failure_exit_codes {
            len += lfec.len();
        }
        let mut hash: HashSet<i32> = HashSet::with_capacity(len);
        for c in ok_exit_codes {
            hash.insert(i32::from(*c));
        }
        if let Some(lfec) = lint_failure_exit_codes {
            for c in lfec {
                hash.insert(i32::from(*c));
            }
        }
        hash
    }

    fn in_dir<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        if !self.chdir {
            return None;
        }

        if path.is_dir() {
            return Some(path);
        }

        Some(path.parent().unwrap())
    }

    fn command_for_path(&self, path: &Path, flags: &Option<Vec<String>>) -> Vec<String> {
        let mut cmd = self.cmd.clone();
        if let Some(flags) = flags {
            for f in flags {
                cmd.push(f.clone());
            }
        }
        if self.run_mode_is(RunMode::Files) || !self.chdir {
            if let Some(pf) = &self.path_flag {
                cmd.push(pf.clone());
            }
            let file = if self.chdir {
                // We know that this is a file because we already checked this
                // in the tidy() or lint() method by calling
                // require_path_type().
                Path::new(path.file_name().unwrap())
            } else {
                path
            };
            cmd.push(file.to_string_lossy().to_string());
        }

        cmd
    }
}

fn replace_root(cmd: Vec<String>, root: &Path) -> Vec<String> {
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
    use anyhow::Result;
    use precious_testhelper as testhelper;
    use pretty_assertions::assert_eq;

    fn matcher(globs: &[&str]) -> Result<path_matcher::Matcher> {
        path_matcher::MatcherBuilder::new().with(globs)?.build()
    }

    fn default_filter_params() -> Result<Filter> {
        Ok(Filter {
            // These params will be ignored
            root: PathBuf::new(),
            name: String::new(),
            typ: FilterType::Lint,
            includer: matcher(&[])?,
            excluder: matcher(&[])?,
            run_mode: RunMode::Dirs,
            // These will supply defaults,
            chdir: false,
            cmd: vec![],
            env: HashMap::new(),
            lint_flags: None,
            tidy_flags: None,
            path_flag: None,
            ok_exit_codes: HashSet::new(),
            lint_failure_exit_codes: HashSet::new(),
            ignore_stderr: None,
        })
    }

    #[test]
    fn require_path_type_dir() -> Result<()> {
        let filter = Filter {
            root: PathBuf::from("/foo/bar"),
            name: String::from("Test"),
            typ: FilterType::Lint,
            includer: matcher(&[])?,
            excluder: matcher(&[])?,
            run_mode: RunMode::Dirs,
            ..default_filter_params()?
        };

        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        assert!(filter.require_path_type("tidy", &helper.root()).is_ok());

        let mut file = helper.root();
        file.push(helper.all_files()[0].clone());
        let res = filter.require_path_type("tidy", &file);
        assert!(res.is_err());
        assert_eq!(
            std::mem::discriminant(res.unwrap_err().downcast_ref().unwrap(),),
            std::mem::discriminant(&FilterError::CanOnlyOperateOnDirectories {
                method: "tidy",
                path: file.to_string_lossy().to_string(),
            }),
        );

        Ok(())
    }

    #[test]
    fn require_path_type_file() -> Result<()> {
        let filter = Filter {
            root: PathBuf::from("/foo/bar"),
            name: String::from("Test"),
            typ: FilterType::Lint,
            includer: matcher(&[])?,
            excluder: matcher(&[])?,
            run_mode: RunMode::Files,
            ..default_filter_params()?
        };

        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let res = filter.require_path_type("tidy", &helper.root());
        assert!(res.is_err());
        assert_eq!(
            std::mem::discriminant(res.unwrap_err().downcast_ref().unwrap()),
            std::mem::discriminant(&FilterError::CanOnlyOperateOnFiles {
                method: "tidy",
                path: helper.root().to_string_lossy().to_string(),
            }),
        );

        let mut file = helper.root();
        file.push(helper.all_files()[0].clone());
        assert!(filter.require_path_type("tidy", &file).is_ok());

        Ok(())
    }

    #[test]
    fn should_process_path() -> Result<()> {
        let filter = Filter {
            root: PathBuf::from("/foo/bar"),
            name: String::from("Test"),
            typ: FilterType::Lint,
            includer: matcher(&["**/*.go"])?,
            excluder: matcher(&["foo/**/*", "baz/bar/**/quux/*"])?,
            run_mode: RunMode::Files,
            ..default_filter_params()?
        };

        let include = &["something.go", "dir/foo.go", ".foo.go", "bar/foo/x.go"];
        for i in include.iter().map(PathBuf::from) {
            let name = i.clone();
            assert!(
                filter.should_process_path(&i.clone(), &[i]),
                "{}",
                name.display(),
            );
        }

        let exclude = &[
            "something.pl",
            "dir/foo.pl",
            "foo/bar.go",
            "baz/bar/anything/here/quux/file.go",
        ];
        for e in exclude.iter().map(PathBuf::from) {
            let name = e.clone();
            assert!(
                !filter.should_process_path(&e.clone(), &[e]),
                "{}",
                name.display(),
            );
        }

        Ok(())
    }

    #[test]
    fn should_process_path_run_mode_dirs() -> Result<()> {
        let filter = Filter {
            root: PathBuf::from("/foo/bar"),
            name: String::from("Test"),
            typ: FilterType::Lint,
            includer: matcher(&["**/*.go"])?,
            excluder: matcher(&["foo/**/*", "baz/bar/**/quux/*"])?,
            run_mode: RunMode::Dirs,
            ..default_filter_params()?
        };

        let include = &[
            &[".", "foo.go", "README.md"],
            &["dir/foo", "dir/foo/foo.pl", "dir/foo/file.go"],
        ];
        for i in include.iter() {
            let dir = PathBuf::from(i[0]);
            let files = i[1..].iter().map(PathBuf::from).collect::<Vec<PathBuf>>();
            let name = dir.clone();
            assert!(
                filter.should_process_path(&dir, &files),
                "{}",
                name.display(),
            );
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
            assert!(
                !filter.should_process_path(&dir, &files),
                "{}",
                name.display(),
            );
        }

        Ok(())
    }

    #[test]
    fn should_process_path_run_mode_root() -> Result<()> {
        let filter = Filter {
            root: PathBuf::from("/foo/bar"),
            name: String::from("Test"),
            typ: FilterType::Lint,
            includer: matcher(&["**/*.go"])?,
            excluder: matcher(&["foo/**/*", "baz/bar/**/quux/*"])?,
            run_mode: RunMode::Root,
            ..default_filter_params()?
        };

        let include = &[
            &[".", "foo.go", "README.md"],
            &["dir/foo", "dir/foo/foo.pl", "dir/foo/file.go"],
        ];
        for i in include.iter() {
            let dir = PathBuf::from(i[0]);
            let files = i[1..].iter().map(PathBuf::from).collect::<Vec<PathBuf>>();
            let name = dir.clone();
            assert!(
                filter.should_process_path(&dir, &files),
                "{}",
                name.display(),
            );
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
            assert!(
                !filter.should_process_path(&dir, &files),
                "{}",
                name.display(),
            );
        }

        Ok(())
    }

    #[test]
    fn command_for_path() -> Result<()> {
        {
            let filter = Filter {
                cmd: vec!["test".to_string()],
                env: HashMap::new(),
                chdir: false,
                lint_flags: None,
                tidy_flags: None,
                path_flag: None,
                ok_exit_codes: HashSet::new(),
                lint_failure_exit_codes: HashSet::new(),
                run_mode: RunMode::Root,
                ..default_filter_params()?
            };
            assert_eq!(
                filter.command_for_path(Path::new("foo.go"), &None),
                vec!["test".to_string(), "foo.go".to_string()],
                "root mode, no chdir",
            );
        }

        {
            let filter = Filter {
                cmd: vec!["test".to_string()],
                env: HashMap::new(),
                chdir: false,
                lint_flags: None,
                tidy_flags: None,
                path_flag: None,
                ok_exit_codes: HashSet::new(),
                lint_failure_exit_codes: HashSet::new(),
                run_mode: RunMode::Root,
                ..default_filter_params()?
            };
            assert_eq!(
                filter.command_for_path(Path::new("foo.go"), &Some(vec!["--flag".to_string()])),
                vec![
                    "test".to_string(),
                    "--flag".to_string(),
                    "foo.go".to_string(),
                ],
                "root mode, no chdir with flags",
            );
        }

        {
            let filter = Filter {
                cmd: vec!["test".to_string()],
                env: HashMap::new(),
                chdir: true,
                lint_flags: None,
                tidy_flags: None,
                path_flag: None,
                ok_exit_codes: HashSet::new(),
                lint_failure_exit_codes: HashSet::new(),
                run_mode: RunMode::Root,
                ..default_filter_params()?
            };
            assert_eq!(
                filter.command_for_path(Path::new("foo.go"), &None),
                vec!["test".to_string()],
                "root mode, with chdir",
            );
        }

        {
            let filter = Filter {
                cmd: vec!["test".to_string()],
                env: HashMap::new(),
                chdir: true,
                lint_flags: None,
                tidy_flags: None,
                path_flag: None,
                ok_exit_codes: HashSet::new(),
                lint_failure_exit_codes: HashSet::new(),
                run_mode: RunMode::Files,
                ..default_filter_params()?
            };
            assert_eq!(
                filter.command_for_path(Path::new("some_dir/foo.go"), &None),
                vec!["test".to_string(), "foo.go".to_string()],
                "files mode, with chdir",
            );
        }

        {
            let filter = Filter {
                cmd: vec!["test".to_string()],
                env: HashMap::new(),
                chdir: false,
                lint_flags: None,
                tidy_flags: None,
                path_flag: None,
                ok_exit_codes: HashSet::new(),
                lint_failure_exit_codes: HashSet::new(),
                run_mode: RunMode::Files,
                ..default_filter_params()?
            };
            assert_eq!(
                filter.command_for_path(Path::new("some_dir/foo.go"), &None),
                vec!["test".to_string(), "some_dir/foo.go".to_string()],
                "files mode, no chdir",
            );
        }

        {
            let filter = Filter {
                cmd: vec!["test".to_string()],
                env: HashMap::new(),
                chdir: false,
                lint_flags: None,
                tidy_flags: None,
                path_flag: Some("--file".to_string()),
                ok_exit_codes: HashSet::new(),
                lint_failure_exit_codes: HashSet::new(),
                run_mode: RunMode::Files,
                ..default_filter_params()?
            };
            assert_eq!(
                filter.command_for_path(Path::new("some_dir/foo.go"), &None),
                vec![
                    "test".to_string(),
                    "--file".to_string(),
                    "some_dir/foo.go".to_string(),
                ],
                "files mode, no chdir, with path flag"
            );
        }

        {
            let filter = Filter {
                cmd: vec!["test".to_string()],
                env: HashMap::new(),
                chdir: true,
                lint_flags: None,
                tidy_flags: None,
                path_flag: Some("--file".to_string()),
                ok_exit_codes: HashSet::new(),
                lint_failure_exit_codes: HashSet::new(),
                run_mode: RunMode::Files,
                ..default_filter_params()?
            };
            assert_eq!(
                filter.command_for_path(Path::new("some_dir/foo.go"), &None),
                vec![
                    "test".to_string(),
                    "--file".to_string(),
                    "foo.go".to_string(),
                ],
                "files mode, with chdir, with path flag",
            );
        }
        Ok(())
    }
}
