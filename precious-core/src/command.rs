use crate::paths::matcher::{Matcher, MatcherBuilder};
use anyhow::Result;
use itertools::Itertools;
use log::{debug, info};
use precious_helpers::exec::Exec;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fmt, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::SystemTime,
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum CommandType {
    #[serde(rename = "lint")]
    Lint,
    #[serde(rename = "tidy")]
    Tidy,
    #[serde(rename = "both")]
    Both,
}

impl CommandType {
    fn what(self) -> &'static str {
        match self {
            CommandType::Lint => "linter",
            CommandType::Tidy => "tidier",
            CommandType::Both => "linter/tidier",
        }
    }
}

impl fmt::Display for CommandType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CommandType::Lint => "lint",
            CommandType::Tidy => "tidy",
            CommandType::Both => "both",
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Invoke {
    #[serde(rename = "per-file")]
    PerFile,
    #[serde(rename = "per-file-or-dir")]
    PerFileOrDir(usize),
    #[serde(rename = "per-file-or-once")]
    PerFileOrOnce(usize),
    #[serde(rename = "per-dir")]
    PerDir,
    #[serde(rename = "per-dir-or-once")]
    PerDirOrOnce(usize),
    #[serde(rename = "once")]
    Once,
    #[serde(rename = "once-by-dir")]
    OnceByDir,
}

impl fmt::Display for Invoke {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Invoke::PerFile => write!(f, r#"invoke = "per-file""#),
            Invoke::PerFileOrDir(n) => write!(f, "invoke.per-file-or-dir = {n}"),
            Invoke::PerFileOrOnce(n) => write!(f, "invoke.per-file-or-once = {n}"),
            Invoke::PerDir => write!(f, r#"invoke = "per-dir""#),
            Invoke::PerDirOrOnce(n) => write!(f, "invoke.per-dir-or-once = {n}"),
            Invoke::Once => write!(f, r#"invoke = "once""#),
            Invoke::OnceByDir => write!(f, r#"invoke = "once-by-dir""#),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActualInvoke {
    PerFile,
    PerDir,
    Once,
}

impl ActualInvoke {
    #[cfg(test)]
    fn as_invoke(&self) -> Invoke {
        match *self {
            ActualInvoke::PerFile => Invoke::PerFile,
            ActualInvoke::PerDir => Invoke::PerDir,
            ActualInvoke::Once => Invoke::Once,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub enum WorkingDir {
    Root,
    Dir,
    ChdirTo(PathBuf),
}

impl TryFrom<&str> for WorkingDir {
    type Error = &'static str;

    fn try_from(from: &str) -> Result<WorkingDir, Self::Error> {
        match from {
            "root" => Ok(WorkingDir::Root),
            "dir" => Ok(WorkingDir::Dir),
            _ => Err("expected one of `root` or `dir`"),
        }
    }
}

impl fmt::Display for WorkingDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkingDir::Root => f.write_str(r#""root""#),
            WorkingDir::Dir => f.write_str(r#""dir""#),
            WorkingDir::ChdirTo(cd) => {
                f.write_str(r#"chdir-to = ""#)?;
                f.write_str(&format!("{}", cd.display()))?;
                f.write_str(r#"""#)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PathArgs {
    #[serde(rename = "file")]
    File,
    #[serde(rename = "dir")]
    Dir,
    #[serde(rename = "none")]
    None,
    #[serde(rename = "dot")]
    Dot,
    #[serde(rename = "absolute-file")]
    AbsoluteFile,
    #[serde(rename = "absolute-dir")]
    AbsoluteDir,
}

impl fmt::Display for PathArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PathArgs::File => r#""file""#,
            PathArgs::Dir => r#""dir""#,
            PathArgs::None => r#""none""#,
            PathArgs::Dot => r#""dot""#,
            PathArgs::AbsoluteFile => r#""absolute-file""#,
            PathArgs::AbsoluteDir => r#""absolute-dir""#,
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
enum CommandError {
    #[error(
        "You cannot define a command which lints and tidies without lint-flags and/or tidy-flags"
    )]
    CommandWhichIsBothRequiresLintOrTidyFlags,

    #[error("Cannot {method:} with the {command:} command, which is a {command_type:}")]
    CannotMethodWithCommand {
        method: &'static str,
        command: String,
        command_type: &'static str,
    },

    #[error("Path {path:} has no parent")]
    PathHasNoParent { path: String },

    #[error("Path {path:} should exist but it does not")]
    PathDoesNotExist { path: String },
}

#[derive(Debug)]
#[allow(clippy::module_name_repetitions)]
pub struct Command {
    project_root: PathBuf,
    pub name: String,
    typ: CommandType,
    filter: Filter,
    invocation: Invocation,
    execution: Execution,
}

#[derive(Debug)]
struct Filter {
    includer: Matcher,
    include: Vec<String>,
    excluder: Matcher,
}

#[derive(Debug)]
struct Invocation {
    invoke: Invoke,
    working_dir: WorkingDir,
    path_args: PathArgs,
}

#[derive(Debug)]
struct Execution {
    cmd: Vec<String>,
    env: HashMap<String, String>,
    lint_flags: Option<Vec<String>>,
    tidy_flags: Option<Vec<String>>,
    path_flag: Option<String>,
    ok_exit_codes: Vec<i32>,
    lint_failure_exit_codes: HashSet<i32>,
    ignore_stderr: Vec<Regex>,
}

#[derive(Debug)]
pub struct CommandParams {
    pub project_root: PathBuf,
    pub name: String,
    pub typ: CommandType,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub invoke: Invoke,
    pub working_dir: WorkingDir,
    pub path_args: PathArgs,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub enum TidyOutcome {
    Unchanged,
    Changed,
    Unknown,
}

#[derive(Debug)]
pub struct LintOutcome {
    pub ok: bool,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

#[derive(Clone, Debug)]
struct PathMetadata {
    dir: Option<PathBuf>,
    path_map: HashMap<PathBuf, PathInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PathInfo {
    mtime: SystemTime,
    size: u64,
    hash: md5::Digest,
}

// This should be safe because we never mutate the Command struct in any of its
// methods.
unsafe impl Sync for Command {}

impl Command {
    pub fn new(params: CommandParams) -> Result<Command> {
        if let CommandType::Both = params.typ {
            if params.lint_flags.is_empty() && params.tidy_flags.is_empty() {
                return Err(CommandError::CommandWhichIsBothRequiresLintOrTidyFlags.into());
            }
        }

        let ignore_stderr = if params.expect_stderr {
            let re = Regex::new(".*")
                .unwrap_or_else(|e| unreachable!("The '.*' regex should always compile: {}", e));
            vec![re]
        } else if params.ignore_stderr.is_empty() {
            vec![]
        } else {
            params
                .ignore_stderr
                .into_iter()
                .map(|i| Regex::new(&i).map_err(Into::into))
                .collect::<Result<Vec<_>>>()?
        };

        let cmd = replace_root(&params.cmd, &params.project_root);
        let root = params.project_root.clone();
        Ok(Command {
            project_root: params.project_root,
            name: params.name,
            typ: params.typ,
            filter: Filter {
                includer: MatcherBuilder::new(&root).with(&params.include)?.build()?,
                include: params.include,
                excluder: MatcherBuilder::new(&root).with(&params.exclude)?.build()?,
            },
            invocation: Invocation {
                invoke: params.invoke,
                working_dir: params.working_dir,
                path_args: params.path_args,
            },
            execution: Execution {
                cmd,
                env: params.env,
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
                ok_exit_codes: Self::unique_exit_codes(
                    &params.ok_exit_codes,
                    Some(&params.lint_failure_exit_codes),
                ),
                lint_failure_exit_codes: params
                    .lint_failure_exit_codes
                    .into_iter()
                    .map(i32::from)
                    .collect(),
                ignore_stderr,
            },
        })
    }

    fn unique_exit_codes(ok_exit_codes: &[u8], lint_failure_exit_codes: Option<&[u8]>) -> Vec<i32> {
        let unique_codes: HashSet<i32> = ok_exit_codes
            .iter()
            .merge(lint_failure_exit_codes.unwrap_or(&[]).iter())
            .map(|c| i32::from(*c))
            .collect();
        unique_codes.into_iter().collect()
    }

    // This returns a vec of vecs where each of the sub-vecs contains 1+
    // files. Each of those sub-vecs represents one invocation of the
    // program. The exact paths that are passed to that invocation are later
    // determined based on the command's `path-args` field.
    pub fn files_to_args_sets<'a>(
        &self,
        files: &'a [PathBuf],
    ) -> Result<(Vec<Vec<&'a Path>>, ActualInvoke)> {
        let files = files.iter().filter(|f| self.file_matches_rules(f));
        Ok(match self.invocation.invoke {
            // Every file becomes its own one one-element Vec.
            Invoke::PerFile => (
                files.sorted().map(|f| vec![f.as_path()]).collect(),
                ActualInvoke::PerFile,
            ),
            Invoke::PerFileOrDir(n) => {
                let count = files.clone().count();
                if count < n {
                    debug!(
                        "Invoking {} once per file for {count} files, which is less than {n}.",
                        self.name,
                    );
                    (
                        files.sorted().map(|f| vec![f.as_path()]).collect(),
                        ActualInvoke::PerFile,
                    )
                } else {
                    debug!(
                        "Invoking {} once per directory for {count} files, which is at least {n}.",
                        self.name,
                    );
                    (Self::files_to_dirs(files)?, ActualInvoke::PerDir)
                }
            }
            Invoke::PerFileOrOnce(n) => {
                let count = files.clone().count();
                if count < n {
                    debug!(
                        "Invoking {} once per file for {count} files, which is less than {n}.",
                        self.name,
                    );
                    (
                        files.sorted().map(|f| vec![f.as_path()]).collect(),
                        ActualInvoke::PerFile,
                    )
                } else {
                    debug!(
                        "Invoking {} once for all {count} files, which is at least {n}.",
                        self.name,
                    );
                    (
                        vec![files.sorted().map(PathBuf::as_path).collect()],
                        ActualInvoke::Once,
                    )
                }
            }
            // Every directory becomes a Vec of its files.
            Invoke::PerDir => (Self::files_to_dirs(files)?, ActualInvoke::PerDir),
            Invoke::PerDirOrOnce(n) => {
                let dirs = Self::files_to_dirs(files.clone())?;
                let count = dirs.len();
                if count < n {
                    debug!("Invoking {} once per directory because there are fewer than {n} directories.", self.name);
                    (dirs, ActualInvoke::PerDir)
                } else {
                    debug!(
                        "Invoking {} once for all {count} directories, which is at least {n}.",
                        self.name,
                    );
                    (
                        vec![files.sorted().map(PathBuf::as_path).collect()],
                        ActualInvoke::Once,
                    )
                }
            }
            // All the files in one Vec.
            Invoke::Once => (
                vec![files.sorted().map(PathBuf::as_path).collect()],
                ActualInvoke::Once,
            ),
            // All directories in one Vec as a batch.
            Invoke::OnceByDir => {
                let files_vec: Vec<&Path> = files.map(PathBuf::as_path).collect();
                let unique_dirs: Vec<&Path> = Self::files_by_dir(&files_vec)?
                    .into_keys()
                    .sorted()
                    .collect();
                (vec![unique_dirs], ActualInvoke::Once)
            }
        })
    }

    fn files_to_dirs<'a>(files: impl Iterator<Item = &'a PathBuf>) -> Result<Vec<Vec<&'a Path>>> {
        let files = files.map(AsRef::as_ref).collect::<Vec<_>>();
        let by_dir = Self::files_by_dir(&files)?;
        Ok(by_dir
            .into_iter()
            .sorted_by_key(|(k, _)| *k)
            .map(|(_, v)| v.into_iter().sorted().collect())
            .collect())
    }

    fn files_by_dir<'a>(files: &[&'a Path]) -> Result<HashMap<&'a Path, Vec<&'a Path>>> {
        let mut by_dir: HashMap<&Path, Vec<&Path>> = HashMap::new();
        for f in files {
            let d = f.parent().ok_or_else(|| CommandError::PathHasNoParent {
                path: f.to_string_lossy().to_string(),
            })?;
            by_dir.entry(d).or_default().push(f);
        }
        Ok(by_dir)
    }

    pub fn tidy(
        &self,
        actual_invoke: ActualInvoke,
        files: &[&Path],
    ) -> Result<Option<TidyOutcome>> {
        self.require_is_not_command_type("tidy", CommandType::Lint)?;

        if !self.should_act_on_files(actual_invoke, files)? {
            return Ok(None);
        }

        let path_metadata = self.maybe_path_metadata_for(actual_invoke, files)?;

        let in_dir = self.in_dir(files[0])?;
        let operating_on = self.operating_on(files, &in_dir)?;
        let (cmd, args) =
            self.cmd_and_args_for_exec(self.execution.tidy_flags.as_deref(), &operating_on);

        let exec = Exec::builder()
            .exe(&cmd)
            .args(args.iter().map(String::as_str).collect::<Vec<_>>())
            .num_paths(operating_on.len())
            .env(self.execution.env.clone())
            .ok_exit_codes(&self.execution.ok_exit_codes)
            .ignore_stderr(self.execution.ignore_stderr.clone())
            .in_dir(&in_dir)
            .build();

        info!(
            "Tidying [{}] with {} in [{}] using command [{}]",
            file_summary_for_log(files),
            self.name,
            in_dir.display(),
            exec.loggable_command,
        );
        exec.run()?;

        if let Some(pm) = path_metadata {
            if self.paths_were_changed(pm)? {
                return Ok(Some(TidyOutcome::Changed));
            }
            return Ok(Some(TidyOutcome::Unchanged));
        }
        Ok(Some(TidyOutcome::Unknown))
    }

    pub fn lint(
        &self,
        actual_invoke: ActualInvoke,
        files: &[&Path],
    ) -> Result<Option<LintOutcome>> {
        self.require_is_not_command_type("lint", CommandType::Tidy)?;

        if !self.should_act_on_files(actual_invoke, files)? {
            return Ok(None);
        }

        let in_dir = self.in_dir(files[0])?;
        let operating_on = self.operating_on(files, &in_dir)?;

        let (cmd, args) =
            self.cmd_and_args_for_exec(self.execution.lint_flags.as_deref(), &operating_on);

        let exec = Exec::builder()
            .exe(&cmd)
            .args(args.iter().map(String::as_str).collect::<Vec<_>>())
            .num_paths(operating_on.len())
            .env(self.execution.env.clone())
            .ok_exit_codes(&self.execution.ok_exit_codes)
            .ignore_stderr(self.execution.ignore_stderr.clone())
            .in_dir(&in_dir)
            .build();

        info!(
            "Linting [{}] with {} in [{}] using command [{}]",
            file_summary_for_log(files),
            self.name,
            in_dir.display(),
            exec.loggable_command,
        );

        let result = exec.run()?;

        Ok(Some(LintOutcome {
            ok: !self
                .execution
                .lint_failure_exit_codes
                .contains(&result.exit_code),
            stdout: result.stdout,
            stderr: result.stderr,
        }))
    }

    fn require_is_not_command_type(
        &self,
        method: &'static str,
        not_allowed: CommandType,
    ) -> Result<()> {
        if not_allowed == self.typ {
            return Err(CommandError::CannotMethodWithCommand {
                method,
                command: self.name.clone(),
                command_type: self.typ.what(),
            }
            .into());
        }
        Ok(())
    }

    fn should_act_on_files(&self, actual_invoke: ActualInvoke, files: &[&Path]) -> Result<bool> {
        match actual_invoke {
            ActualInvoke::PerFile => {
                let f = &files[0];
                // This check isn't strictly necessary since we default to not
                // matching, but the debug output is helpful.
                if self.filter.excluder.path_matches(f, false) {
                    debug!(
                        "File {} is excluded for the {} command",
                        f.display(),
                        self.name,
                    );
                    return Ok(false);
                }
                if self.filter.includer.path_matches(f, false) {
                    debug!(
                        "File {} is included for the {} command",
                        f.display(),
                        self.name,
                    );
                    return Ok(true);
                }
            }
            ActualInvoke::PerDir => {
                let dir = files[0]
                    .parent()
                    .ok_or_else(|| CommandError::PathHasNoParent {
                        path: files[0].to_string_lossy().to_string(),
                    })?;
                for f in files {
                    if self.filter.excluder.path_matches(f, false) {
                        debug!(
                            "File {} is excluded for the {} command",
                            f.display(),
                            self.name,
                        );
                        continue;
                    }
                    if self.filter.includer.path_matches(f, false) {
                        debug!(
                            "Directory {} is included for the {} command because it contains {} which is included",
                            dir.display(),
                            self.name,
                            f.display(),
                        );
                        return Ok(true);
                    }
                }
                debug!(
                    "Directory {} is not included in the {} command because none of its files are included",
                    dir.display(),
                    self.name
                );
            }
            ActualInvoke::Once => {
                for f in files {
                    if self.filter.excluder.path_matches(f, false) {
                        debug!(
                            "File {} is excluded for the {} command",
                            f.display(),
                            self.name,
                        );
                        continue;
                    }
                    if self.filter.includer.path_matches(f, false) {
                        debug!(
                            "File {} is included for the {} command",
                            f.display(),
                            self.name,
                        );
                        return Ok(true);
                    }
                }
                debug!(
                    "The {} command will not run because none of the files in the list are included",
                    self.name,
                );
            }
        }

        // The default is to not match.
        Ok(false)
    }

    // This takes the list of files relevant to the command. That list comes
    // the filenames which were produced by the call to
    // `files_to_args_sets`. This turns those files into the actual paths to
    // be passed to the command, which is passed on the command's `PathArgs`
    // type. Those files are all relative to the _project root_. We may return
    // them as is (but sorted), or we may turn them paths relative to the
    // given directory. The given directory is the directory in which the
    // command will be run, and may not be the project root.
    fn operating_on(&self, files: &[&Path], in_dir: &Path) -> Result<Vec<PathBuf>> {
        match self.invocation.path_args {
            PathArgs::File => Ok(files
                .iter()
                .sorted()
                .map(|r| self.path_relative_to(r, in_dir))
                .collect::<Vec<_>>()),
            PathArgs::Dir => Ok(Self::files_by_dir(files)?
                .into_keys()
                .sorted()
                .map(|r| self.path_relative_to(r, in_dir))
                .collect::<Vec<_>>()),
            PathArgs::None => Ok(vec![]),
            PathArgs::Dot => Ok(vec![PathBuf::from(".")]),
            PathArgs::AbsoluteFile => Ok(files
                .iter()
                .sorted()
                .map(|f| {
                    let mut abs = self.project_root.clone();
                    abs.push(f);
                    abs
                })
                .collect()),
            PathArgs::AbsoluteDir => Ok(Self::files_by_dir(files)?
                .into_keys()
                .map(|d| {
                    let mut abs = self.project_root.clone();
                    if d.components().count() != 0 {
                        abs.push(d);
                    }
                    abs
                })
                .sorted()
                .collect()),
        }
    }

    fn path_relative_to(&self, path: &Path, in_dir: &Path) -> PathBuf {
        let mut abs = self.project_root.clone();
        abs.push(path);

        if let Some(mut diff) = pathdiff::diff_paths(&abs, in_dir) {
            if diff == Path::new("") {
                diff = PathBuf::from(".");
            }
            return diff;
        }

        path.to_path_buf()
    }

    // This takes the list of files relevant to the command. That list comes
    // the filenames which were produced by the call to
    // `files_to_args_sets`. Based on the command's `Invoke` type, it
    // determines what paths it should collect metadata for (which may be
    // none). This metadata is collected for tidy commands so we can determine
    // whether the command changed anything.
    fn maybe_path_metadata_for(
        &self,
        actual_invoke: ActualInvoke,
        files: &[&Path],
    ) -> Result<Option<PathMetadata>> {
        match actual_invoke {
            // If it's invoked per file we know that we only have one file in
            // `files`.
            ActualInvoke::PerFile => Ok(Some(self.path_metadata_for(files[0])?)),
            // If it's invoked per dir we can look at the first file's
            // parent. All the files should have the same dir.
            ActualInvoke::PerDir => {
                let dir = files[0]
                    .parent()
                    .ok_or_else(|| CommandError::PathHasNoParent {
                        path: files[0].to_string_lossy().to_string(),
                    })?;
                Ok(Some(self.path_metadata_for(dir)?))
            }
            // If it's invoked once we would have to look at the entire
            // tree. That might be too expensive so we won't report a tidy
            // outcome in this case.
            ActualInvoke::Once => Ok(None),
        }
    }

    // Given a directory, this gets the metadata for all files in the
    // directory that match the command's include/exclude rules.
    fn path_metadata_for(&self, path: &Path) -> Result<PathMetadata> {
        let mut path_map = HashMap::new();
        let mut dir = None;
        let mut full_path = self.project_root.clone();
        full_path.push(path);

        if full_path.is_file() {
            let meta = Self::metadata_for_file(&full_path)?;
            path_map.insert(full_path, meta);
        } else if full_path.is_dir() {
            dir = Some(path.to_path_buf());
            for entry in fs::read_dir(full_path)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() && self.file_matches_rules(&path) {
                    let meta = entry.metadata()?;
                    let hash = md5::compute(fs::read(&path)?);
                    path_map.insert(
                        path,
                        PathInfo {
                            mtime: meta.modified()?,
                            size: meta.len(),
                            hash,
                        },
                    );
                }
            }
        } else if !path.exists() {
            return Err(CommandError::PathDoesNotExist {
                path: path.to_string_lossy().to_string(),
            }
            .into());
        } else {
            unreachable!(
                "I sure hope is_file(), is_dir(), and !exists() are the only three states"
            );
        }

        Ok(PathMetadata { dir, path_map })
    }

    fn file_matches_rules(&self, file: &Path) -> bool {
        if self.filter.excluder.path_matches(file, false) {
            return false;
        }
        if self.filter.includer.path_matches(file, false) {
            return true;
        }
        false
    }

    fn metadata_for_file(file: &Path) -> Result<PathInfo> {
        let meta = fs::metadata(file)?;
        Ok(PathInfo {
            mtime: meta.modified()?,
            size: meta.len(),
            hash: md5::compute(fs::read(file)?),
        })
    }

    fn cmd_and_args_for_exec(
        &self,
        flags: Option<&[String]>,
        paths: &[PathBuf],
    ) -> (String, Vec<String>) {
        let mut args = self.execution.cmd.clone();
        let cmd = args.remove(0);

        if let Some(flags) = flags {
            for f in flags {
                args.push(f.clone());
            }
        }

        for p in paths {
            if let Some(pf) = &self.execution.path_flag {
                args.push(pf.clone());
            }
            args.push(p.to_string_lossy().to_string());
        }

        (cmd, args)
    }

    pub(crate) fn paths_summary(&self, actual_invoke: ActualInvoke, paths: &[&Path]) -> String {
        let all = paths
            .iter()
            .sorted()
            .map(|p| p.to_string_lossy().to_string())
            .join(" ");
        if paths.len() <= 3 {
            return all;
        }

        match actual_invoke {
            ActualInvoke::Once | ActualInvoke::PerDir => {
                let initial = paths
                    .iter()
                    .sorted()
                    .take(2)
                    .map(|p| p.to_string_lossy())
                    .join(" ");
                format!(
                    "{} files matching {}, starting with {}",
                    paths.len(),
                    self.filter.include.join(" "),
                    initial
                )
            }
            ActualInvoke::PerFile => format!("{} files: {}", paths.len(), all),
        }
    }

    fn paths_were_changed(&self, prev: PathMetadata) -> Result<bool> {
        for (prev_file, prev_meta) in &prev.path_map {
            debug!("Checking {} for changes", prev_file.display());
            let current_meta = match fs::metadata(prev_file) {
                Ok(m) => m,
                // If the file no longer exists the command must've deleted
                // it.
                Err(e) if e.kind() == ErrorKind::NotFound => return Ok(true),
                Err(e) => return Err(e.into()),
            };
            // If the mtime is unchanged we don't need to compare anything
            // else. Unfortunately there's no guarantee a command won't modify
            // the mtime even if it doesn't change the file's contents, so we
            // cannot assume anything was changed just because the mtime
            // changed. For example, Perl::Tidy does this :(
            if prev_meta.mtime == current_meta.modified()? {
                continue;
            }

            // If the size changed we know the contents changed.
            if prev_meta.size != current_meta.len() {
                return Ok(true);
            }

            // Otherwise we need to compare the content hash.
            if prev_meta.hash != md5::compute(fs::read(prev_file)?) {
                return Ok(true);
            }
        }

        if let Some(dir) = prev.dir {
            let entries = match fs::read_dir(dir) {
                Ok(rd) => rd,
                Err(e) if e.kind() == ErrorKind::NotFound => return Ok(true),
                Err(e) => return Err(e.into()),
            };
            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                if path.is_file()
                    && self.file_matches_rules(&path)
                    && !prev.path_map.contains_key(&path)
                {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    pub fn config_key(&self) -> String {
        format!("commands.{}", Self::maybe_toml_quote(&self.name),)
    }

    fn maybe_toml_quote(name: &str) -> String {
        if name.contains(' ') {
            return format!(r#""{name}""#);
        }
        name.to_string()
    }

    fn in_dir(&self, file: &Path) -> Result<PathBuf> {
        match &self.invocation.working_dir {
            WorkingDir::Root => Ok(self.project_root.clone()),
            WorkingDir::Dir => {
                let mut abs = self.project_root.clone();
                abs.push(file);
                let parent = abs.parent().ok_or_else(|| CommandError::PathHasNoParent {
                    path: file.to_string_lossy().to_string(),
                })?;
                Ok(parent.to_path_buf())
            }
            WorkingDir::ChdirTo(cd) => {
                let mut dir = self.project_root.clone();
                dir.push(cd);
                Ok(dir)
            }
        }
    }

    pub fn config_debug(&self) -> String {
        format!(
            "{} | working-dir = {} | path-args = {}",
            self.invocation.invoke, self.invocation.working_dir, self.invocation.path_args
        )
    }
}

fn file_summary_for_log(files: &[&Path]) -> String {
    if files.len() <= 3 {
        return files.iter().map(|p| p.to_string_lossy()).join(" ");
    }
    let first3 = files.iter().take(3).map(|p| p.to_string_lossy()).join(" ");
    format!("{} ... and {} more", first3, files.len() - 3)
}

fn replace_root(cmd: &[String], root: &Path) -> Vec<String> {
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
    use anyhow::Result;
    use precious_testhelper as testhelper;
    use pretty_assertions::assert_eq;
    use serial_test::parallel;
    use std::env;
    use test_case::test_case;
    use testhelper::TestHelper;

    fn matcher(globs: &[&str]) -> Result<Matcher> {
        MatcherBuilder::new("/").with(globs)?.build()
    }

    fn default_command() -> Command {
        Command {
            // These params will be ignored
            project_root: PathBuf::new(),
            name: String::new(),
            typ: CommandType::Lint,
            filter: Filter {
                includer: matcher(&[]).unwrap(),
                include: vec![],
                excluder: matcher(&[]).unwrap(),
            },
            invocation: Invocation {
                invoke: Invoke::PerFile,
                working_dir: WorkingDir::Root,
                path_args: PathArgs::File,
            },
            execution: Execution {
                cmd: vec![],
                env: HashMap::new(),
                lint_flags: None,
                tidy_flags: None,
                path_flag: None,
                ok_exit_codes: vec![],
                lint_failure_exit_codes: HashSet::new(),
                ignore_stderr: vec![],
            },
        }
    }

    #[test]
    #[parallel]
    fn files_to_args_sets_per_file() -> Result<()> {
        let mut command = default_command();
        command.invocation.invoke = Invoke::PerFile;
        command.filter.includer = matcher(&["**/*.go"])?;

        let files = &["foo.go", "test/foo.go", "bar.go", "subdir/baz.go"]
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let bar = PathBuf::from("bar.go");
        let foo = PathBuf::from("foo.go");
        let baz = PathBuf::from("subdir/baz.go");
        let test_foo = PathBuf::from("test/foo.go");
        assert_eq!(
            command.files_to_args_sets(files)?,
            (
                vec![
                    vec![bar.as_path()],
                    vec![foo.as_path()],
                    vec![baz.as_path()],
                    vec![test_foo.as_path()],
                ],
                ActualInvoke::PerFile,
            ),
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn files_to_args_sets_per_file_or_dir() -> Result<()> {
        let mut command = default_command();
        command.invocation.invoke = Invoke::PerFileOrDir(3);
        command.filter.includer = matcher(&["**/*.go"])?;

        let files = &["foo.go", "test/foo.go", "bar.go", "subdir/baz.go"]
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let bar = PathBuf::from("bar.go");
        let foo = PathBuf::from("foo.go");
        let baz = PathBuf::from("subdir/baz.go");
        let test_foo = PathBuf::from("test/foo.go");
        assert_eq!(
            command.files_to_args_sets(&files[0..2])?,
            (
                vec![vec![foo.as_path()], vec![test_foo.as_path()],],
                ActualInvoke::PerFile,
            ),
            "with two paths invoke is PerFile",
        );
        assert_eq!(
            command.files_to_args_sets(files)?,
            (
                vec![
                    vec![bar.as_path(), foo.as_path()],
                    vec![baz.as_path()],
                    vec![test_foo.as_path()],
                ],
                ActualInvoke::PerDir,
            ),
            "with four paths invoke is PerDir",
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn files_to_args_sets_per_file_or_once() -> Result<()> {
        let mut command = default_command();
        command.invocation.invoke = Invoke::PerFileOrOnce(3);
        command.filter.includer = matcher(&["**/*.go"])?;

        let files = &["foo.go", "test/foo.go", "bar.go", "subdir/baz.go"]
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let bar = PathBuf::from("bar.go");
        let foo = PathBuf::from("foo.go");
        let baz = PathBuf::from("subdir/baz.go");
        let test_foo = PathBuf::from("test/foo.go");
        assert_eq!(
            command.files_to_args_sets(&files[0..2])?,
            (
                vec![vec![foo.as_path()], vec![test_foo.as_path()],],
                ActualInvoke::PerFile,
            ),
            "with 2 paths invoke is PerFile",
        );
        assert_eq!(
            command.files_to_args_sets(files)?,
            (
                vec![vec![
                    bar.as_path(),
                    foo.as_path(),
                    baz.as_path(),
                    test_foo.as_path()
                ]],
                ActualInvoke::Once,
            ),
            "with four paths invoke is Once",
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn files_to_args_sets_per_dir() -> Result<()> {
        let mut command = default_command();
        command.invocation.invoke = Invoke::PerDir;
        command.filter.includer = matcher(&["**/*.go"])?;

        let files = &["foo.go", "test/foo.go", "bar.go", "subdir/baz.go"]
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let bar = PathBuf::from("bar.go");
        let foo = PathBuf::from("foo.go");
        let baz = PathBuf::from("subdir/baz.go");
        let test_foo = PathBuf::from("test/foo.go");
        assert_eq!(
            command.files_to_args_sets(files)?,
            (
                vec![
                    vec![bar.as_path(), foo.as_path()],
                    vec![baz.as_path()],
                    vec![test_foo.as_path()],
                ],
                ActualInvoke::PerDir,
            ),
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn files_to_args_sets_once() -> Result<()> {
        let mut command = default_command();
        command.invocation.invoke = Invoke::Once;
        command.filter.includer = matcher(&["**/*.go"])?;

        let files = &["foo.go", "test/foo.go", "bar.go", "subdir/baz.go"]
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let bar = PathBuf::from("bar.go");
        let foo = PathBuf::from("foo.go");
        let baz = PathBuf::from("subdir/baz.go");
        let test_foo = PathBuf::from("test/foo.go");
        assert_eq!(
            command.files_to_args_sets(files)?,
            (
                vec![vec![
                    bar.as_path(),
                    foo.as_path(),
                    baz.as_path(),
                    test_foo.as_path(),
                ]],
                ActualInvoke::Once,
            ),
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn files_to_args_sets_once_by_dir() -> Result<()> {
        let mut command = default_command();
        command.invocation.invoke = Invoke::OnceByDir;
        command.filter.includer = matcher(&["**/*.go"])?;

        let files = &["foo.go", "test/foo.go", "bar.go", "subdir/baz.go"]
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        assert_eq!(
            command.files_to_args_sets(files)?,
            (
                vec![vec![Path::new(""), Path::new("subdir"), Path::new("test"),]],
                ActualInvoke::Once,
            ),
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn require_is_not_command_type_with_lint_command() -> Result<()> {
        let mut command = default_command();
        command.typ = CommandType::Lint;

        assert!(command
            .require_is_not_command_type("lint", CommandType::Tidy)
            .is_ok());
        assert_eq!(
            command
                .require_is_not_command_type("tidy", CommandType::Lint)
                .unwrap_err()
                .downcast::<CommandError>()
                .unwrap(),
            CommandError::CannotMethodWithCommand {
                method: "tidy",
                command: command.name,
                command_type: "linter",
            },
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn require_is_not_command_type_with_tidy_command() -> Result<()> {
        let mut command = default_command();
        command.typ = CommandType::Tidy;

        assert!(command
            .require_is_not_command_type("tidy", CommandType::Lint)
            .is_ok());
        assert_eq!(
            command
                .require_is_not_command_type("lint", CommandType::Tidy)
                .unwrap_err()
                .downcast::<CommandError>()
                .unwrap(),
            CommandError::CannotMethodWithCommand {
                method: "lint",
                command: command.name,
                command_type: "tidier",
            },
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn require_is_not_command_type_with_both_command() -> Result<()> {
        let mut command = default_command();
        command.typ = CommandType::Both;

        assert!(command
            .require_is_not_command_type("tidy", CommandType::Lint)
            .is_ok());
        assert!(command
            .require_is_not_command_type("lint", CommandType::Tidy)
            .is_ok());

        Ok(())
    }

    #[test]
    #[parallel]
    fn should_act_on_files_invoke_per_file() -> Result<()> {
        let mut command = default_command();
        command.project_root = PathBuf::from("/foo/bar");
        command.name = String::from("Test");
        command.typ = CommandType::Lint;
        command.filter.includer = matcher(&["**/*.go", "!this/file.go"])?;
        command.filter.excluder = matcher(&["foo/**/*", "!foo/some/file.go", "baz/bar/**/quux/*"])?;

        let include = [
            "something.go",
            "dir/foo.go",
            ".foo.go",
            "bar/foo/x.go",
            "foo/some/file.go",
        ];
        for i in include.iter().map(PathBuf::from) {
            let name = i.clone();
            assert!(
                command.should_act_on_files(ActualInvoke::PerFile, &[&i])?,
                "{}",
                name.display(),
            );
        }

        let exclude = [
            "this/file.go",
            "something.pl",
            "dir/foo.pl",
            "foo/bar.go",
            "baz/bar/anything/here/quux/file.go",
        ];
        for e in exclude.iter().map(PathBuf::from) {
            let name = e.clone();
            assert!(
                !command.should_act_on_files(ActualInvoke::PerFile, &[&e])?,
                "{}",
                name.display(),
            );
        }

        Ok(())
    }

    #[test]
    #[parallel]
    fn should_act_on_files_invoke_per_dir() -> Result<()> {
        let mut command = default_command();
        command.project_root = PathBuf::from("/foo/bar");
        command.name = String::from("Test");
        command.typ = CommandType::Lint;
        command.filter.includer = matcher(&["**/*.go", "!this/file.go"])?;
        command.filter.excluder = matcher(&["foo/**/*", "!foo/some/file.go", "baz/bar/**/quux/*"])?;
        command.invocation.invoke = Invoke::PerDir;
        command.invocation.path_args = PathArgs::Dir;

        let include = [
            ["foo.go", "README.md"],
            ["dir/foo/foo.pl", "dir/foo/file.go"],
            ["dir/some.go", "dir/some.rs"],
            ["foo/some/file.go", "foo/excluded.go"],
        ];
        for i in include.iter() {
            let files = i.iter().map(PathBuf::from).collect::<Vec<_>>();
            assert!(
                command.should_act_on_files(
                    ActualInvoke::PerDir,
                    &files.iter().map(|f| f.as_ref()).collect::<Vec<_>>(),
                )?,
                "{}",
                i.join(", ")
            );
        }

        let exclude = [
            ["foo/bar.go", "foo/baz.go"],
            ["baz/bar/foo/quux/file.go", "baz/bar/foo/quux/other.go"],
            ["dir/foo.pl", "dir/file.txt"],
            ["this/file.go", "foo/excluded.go"],
        ];
        for e in exclude.iter() {
            let files = e.iter().map(PathBuf::from).collect::<Vec<_>>();
            assert!(
                !command.should_act_on_files(
                    ActualInvoke::PerDir,
                    &files.iter().map(|f| f.as_ref()).collect::<Vec<_>>(),
                )?,
                "{}",
                e.join(", ")
            );
        }

        Ok(())
    }

    #[test]
    #[parallel]
    fn should_act_on_files_invoke_once() -> Result<()> {
        let mut command = default_command();
        command.project_root = PathBuf::from("/foo/bar");
        command.name = String::from("Test");
        command.typ = CommandType::Lint;
        command.filter.includer = matcher(&["**/*.go", "!this/file.go"])?;
        command.filter.excluder = matcher(&["foo/**/*", "!foo/some/file.go", "baz/bar/**/quux/*"])?;
        command.invocation.invoke = Invoke::Once;

        let include = [
            [".", "foo.go", "README.md"],
            ["dir/foo", "dir/foo/foo.pl", "dir/foo/file.go"],
            [".", "foo/bar.go", "foo/some/file.go"],
        ];
        for i in include.iter() {
            let dir = PathBuf::from(i[0]);
            let files = i[1..].iter().map(PathBuf::from).collect::<Vec<PathBuf>>();
            let name = dir.clone();
            assert!(
                command.should_act_on_files(
                    ActualInvoke::Once,
                    &files.iter().map(|f| f.as_ref()).collect::<Vec<_>>()
                )?,
                "{}",
                name.display()
            );
        }

        let exclude = [
            ["foo", "foo/bar.go", "foo/baz.go"],
            [
                "baz/bar/foo/quux",
                "baz/bar/foo/quux/file.go",
                "baz/bar/foo/quux/other.go",
            ],
            ["dir", "dir/foo.pl", "dir/file.txt"],
            [".", "this/file.go", "foo/also/excluded.go"],
        ];
        for e in exclude.iter() {
            let dir = PathBuf::from(e[0]);
            let files = e[1..].iter().map(PathBuf::from).collect::<Vec<PathBuf>>();
            let name = dir.clone();
            assert!(
                !command.should_act_on_files(
                    ActualInvoke::Once,
                    &files.iter().map(|f| f.as_ref()).collect::<Vec<_>>()
                )?,
                "{}",
                name.display()
            );
        }

        Ok(())
    }

    #[test]
    #[parallel]
    fn operating_on_with_path_args_file_in_project_root() -> Result<()> {
        let mut command = default_command();
        command.invocation.path_args = PathArgs::File;

        let file1 = Path::new("file1");
        assert_eq!(
            command.operating_on(&[file1], &command.project_root)?,
            vec![file1],
        );

        let file2 = Path::new("subdir/file2");
        assert_eq!(
            command.operating_on(&[file2], &command.project_root)?,
            vec![file2],
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn operating_on_with_path_args_file_in_subdir() -> Result<()> {
        let mut command = default_command();
        command.invocation.path_args = PathArgs::File;

        let mut in_dir = command.project_root.clone();
        in_dir.push("subdir");
        let file = Path::new("subdir/file");
        assert_eq!(
            command.operating_on(&[file], &in_dir)?,
            vec![PathBuf::from("file")],
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn operating_on_with_path_args_dir_in_project_root() -> Result<()> {
        let mut command = default_command();
        command.invocation.path_args = PathArgs::Dir;

        let files = [Path::new("file1"), Path::new("subdir/file2")];
        assert_eq!(
            command.operating_on(&files, &command.project_root,)?,
            vec![PathBuf::from("."), PathBuf::from("subdir")],
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn operating_on_with_path_args_dir_in_subdir() -> Result<()> {
        let mut command = default_command();
        command.invocation.path_args = PathArgs::Dir;

        let files = [Path::new("subdir/file1"), Path::new("subdir/more/file2")];
        let mut in_dir = command.project_root.clone();
        in_dir.push("subdir");
        assert_eq!(
            command.operating_on(&files, &in_dir)?,
            vec![PathBuf::from("."), PathBuf::from("more")],
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn operating_on_with_path_args_absolute_file() -> Result<()> {
        let cwd = env::current_dir()?;
        let mut command = default_command();
        command.project_root = cwd.clone();
        command.invocation.path_args = PathArgs::AbsoluteFile;

        let mut file1 = cwd.clone();
        file1.push("file1");
        assert_eq!(
            command.operating_on(&[Path::new("file1")], &command.project_root)?,
            vec![file1],
        );

        let mut file1 = cwd;
        file1.push("subdir/file2");
        assert_eq!(
            command.operating_on(&[Path::new("subdir/file2")], &command.project_root)?,
            vec![file1],
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn operating_on_with_path_args_absolute_file_in_dir() -> Result<()> {
        let cwd = env::current_dir()?;
        let mut command = default_command();
        command.project_root = cwd.clone();
        command.invocation.path_args = PathArgs::AbsoluteFile;

        let mut in_dir = command.project_root.clone();
        in_dir.push("subdir");

        let mut file1 = cwd.clone();
        file1.push("file1");
        assert_eq!(
            command.operating_on(&[Path::new("file1")], &in_dir)?,
            vec![file1],
        );

        let mut file1 = cwd;
        file1.push("subdir/file2");
        assert_eq!(
            command.operating_on(&[Path::new("subdir/file2")], &in_dir)?,
            vec![file1],
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn operating_on_with_path_args_absolute_dir_in_project_root() -> Result<()> {
        let cwd = env::current_dir()?;
        let mut command = default_command();
        command.project_root = cwd.clone();
        command.invocation.path_args = PathArgs::AbsoluteDir;

        assert_eq!(
            command.operating_on(&[Path::new("file1")], &command.project_root)?,
            vec![cwd.clone()],
        );

        let mut subdir = cwd;
        subdir.push("subdir");
        assert_eq!(
            command.operating_on(&[Path::new("subdir/file2")], &command.project_root)?,
            vec![subdir],
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn operating_on_with_path_args_absolute_dir_in_dir() -> Result<()> {
        let cwd = env::current_dir()?;
        let mut command = default_command();
        command.project_root = cwd.clone();
        command.invocation.path_args = PathArgs::AbsoluteDir;

        let mut in_dir = command.project_root.clone();
        in_dir.push("subdir");

        assert_eq!(
            command.operating_on(&[Path::new("file1")], &in_dir)?,
            vec![cwd.clone()],
        );

        let mut subdir = cwd;
        subdir.push("subdir");
        assert_eq!(
            command.operating_on(&[Path::new("subdir/file2")], &in_dir)?,
            vec![subdir],
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn operating_on_with_path_args_dot_in_project_root() -> Result<()> {
        let mut command = default_command();
        command.invocation.path_args = PathArgs::Dot;

        let files = [Path::new("file1"), Path::new("subdir/file2")];
        assert_eq!(
            command.operating_on(&files, &command.project_root)?,
            vec![PathBuf::from(".")],
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn operating_on_with_path_args_dot_in_dir() -> Result<()> {
        let mut command = default_command();
        command.invocation.path_args = PathArgs::Dot;

        let mut in_dir = command.project_root.clone();
        in_dir.push("subdir");

        let files = [Path::new("file1"), Path::new("subdir/file2")];
        assert_eq!(
            command.operating_on(&files, &in_dir)?,
            vec![PathBuf::from(".")],
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn operating_on_with_path_args_none_in_project_root() -> Result<()> {
        let mut command = default_command();
        command.invocation.path_args = PathArgs::None;

        let files = [Path::new("file1"), Path::new("subdir/file2")];
        let expect: Vec<PathBuf> = vec![];
        assert_eq!(command.operating_on(&files, &command.project_root)?, expect);

        Ok(())
    }

    #[test]
    #[parallel]
    fn operating_on_with_path_args_none_in_dir() -> Result<()> {
        let mut command = default_command();
        command.invocation.path_args = PathArgs::None;

        let mut in_dir = command.project_root.clone();
        in_dir.push("subdir");

        let files = [Path::new("file1"), Path::new("subdir/file2")];
        let expect: Vec<PathBuf> = vec![];
        assert_eq!(command.operating_on(&files, &in_dir)?, expect);

        Ok(())
    }

    #[test]
    #[parallel]
    fn maybe_path_metadata_for_per_file() -> Result<()> {
        let mut command = default_command();
        command.invocation.invoke = Invoke::PerFile;
        command.filter.includer = MatcherBuilder::new("/").with(&["**/*.rs"])?.build()?;

        let helper = TestHelper::new()?.with_git_repo()?;
        let mut file = helper.git_root();
        file.push("src/bar.rs");
        let metadata = command
            .maybe_path_metadata_for(ActualInvoke::PerFile, &[&file])?
            .unwrap_or_else(|| unreachable!("Should always have metadata with Invoke::PerFile"));
        assert!(metadata.path_map.contains_key(&file));

        Ok(())
    }

    #[test]
    #[parallel]
    fn maybe_path_metadata_for_per_dir() -> Result<()> {
        let mut command = default_command();
        command.invocation.invoke = Invoke::PerFile;
        command.filter.includer = MatcherBuilder::new("/").with(&["**/*.rs"])?.build()?;
        command.filter.excluder = MatcherBuilder::new("/")
            .with(&["**/can_ignore.rs"])?
            .build()?;

        let helper = TestHelper::new()?.with_git_repo()?;
        let mut dir = helper.git_root();
        dir.push("src");
        let metadata = command
            .maybe_path_metadata_for(ActualInvoke::PerFile, &[&dir])?
            .unwrap_or_else(|| unreachable!("Should always have metadata with Invoke::PerFile"));
        let expect_files = ["bar.rs", "main.rs", "module.rs"];
        for name in expect_files {
            let mut file = dir.clone();
            file.push(name);
            assert!(
                metadata.path_map.contains_key(&file),
                "contains {}",
                file.display(),
            );
        }
        assert_eq!(metadata.path_map.len(), expect_files.len());
        assert_eq!(metadata.dir, Some(dir));

        Ok(())
    }

    #[test]
    #[parallel]
    fn maybe_path_metadata_for_once() -> Result<()> {
        let mut command = default_command();
        command.invocation.invoke = Invoke::Once;

        let cwd = env::current_dir()?;
        assert!(command
            .maybe_path_metadata_for(ActualInvoke::Once, &[&cwd])?
            .is_none());

        Ok(())
    }

    #[test]
    #[parallel]
    fn paths_were_not_changed_when_only_mtime_changes() -> Result<()> {
        let mut command = default_command();
        command.invocation.invoke = Invoke::PerFile;
        command.filter.includer = MatcherBuilder::new("/").with(&["**/*.rs"])?.build()?;
        command.filter.excluder = MatcherBuilder::new("/")
            .with(&["**/can_ignore.rs"])?
            .build()?;

        let helper = TestHelper::new()?.with_git_repo()?;
        let mut file = helper.git_root();
        file.push("src/main.rs");
        let files = vec![file.as_ref()];

        let prev = command.maybe_path_metadata_for(ActualInvoke::PerFile, &files)?;
        assert!(prev.is_some());
        assert!(!command.paths_were_changed(prev.clone().unwrap())?);

        filetime::set_file_mtime(&file, filetime::FileTime::from_unix_time(0, 0))?;
        assert!(!command.paths_were_changed(prev.unwrap())?);

        Ok(())
    }

    #[test]
    #[parallel]
    fn paths_were_changed_when_size_changes() -> Result<()> {
        let mut command = default_command();
        command.invocation.invoke = Invoke::PerFile;
        command.filter.includer = MatcherBuilder::new("/").with(&["**/*.rs"])?.build()?;
        command.filter.excluder = MatcherBuilder::new("/")
            .with(&["**/can_ignore.rs"])?
            .build()?;

        let helper = TestHelper::new()?.with_git_repo()?;
        let mut file = helper.git_root();
        file.push("src/main.rs");
        let files = vec![file.as_ref()];

        let prev = command.maybe_path_metadata_for(ActualInvoke::PerFile, &files)?;
        assert!(prev.is_some());
        assert!(!command.paths_were_changed(prev.clone().unwrap())?);

        helper.write_file(&file, "new content that is longer than the old content")?;
        assert!(command.paths_were_changed(prev.unwrap())?);

        Ok(())
    }

    #[test]
    #[parallel]
    fn paths_were_changed_when_content_changes() -> Result<()> {
        let mut command = default_command();
        command.invocation.invoke = Invoke::PerFile;
        command.filter.includer = MatcherBuilder::new("/").with(&["**/*.rs"])?.build()?;
        command.filter.excluder = MatcherBuilder::new("/")
            .with(&["**/can_ignore.rs"])?
            .build()?;

        let helper = TestHelper::new()?.with_git_repo()?;
        let mut file = helper.git_root();
        file.push("src/main.rs");
        let files = vec![file.as_ref()];

        let prev = command.maybe_path_metadata_for(ActualInvoke::PerFile, &files)?;
        assert!(prev.is_some());
        assert!(!command.paths_were_changed(prev.clone().unwrap())?);

        // This needs to be the same size as the old content.
        let new_content = fs::read_to_string(&file)?.chars().rev().collect::<String>();
        helper.write_file(&file, &new_content)?;

        assert!(command.paths_were_changed(prev.unwrap())?);

        Ok(())
    }

    #[test]
    #[parallel]
    fn paths_were_changed_when_dir_has_new_file() -> Result<()> {
        let mut command = default_command();
        command.invocation.invoke = Invoke::PerDir;
        command.filter.includer = MatcherBuilder::new("/").with(&["**/*.rs"])?.build()?;
        command.filter.excluder = MatcherBuilder::new("/")
            .with(&["**/can_ignore.rs"])?
            .build()?;

        let helper = TestHelper::new()?.with_git_repo()?;
        let mut files = vec![];
        for path in helper.all_files() {
            if path.starts_with("src/")
                && path.to_str().unwrap().ends_with(".rs")
                && path.ancestors().count() == 3
            {
                let mut file = helper.git_root();
                file.push(path);
                files.push(file);
            }
        }

        let prev = command.maybe_path_metadata_for(
            ActualInvoke::PerDir,
            &files.iter().map(|f| f.as_ref()).collect::<Vec<_>>(),
        )?;
        assert!(prev.is_some());
        let prev = prev.unwrap();
        assert_eq!(
            prev.path_map.len(),
            3,
            "excluded files are not in the path map",
        );
        assert!(!command.paths_were_changed(prev.clone())?);

        let mut file = helper.git_root();
        file.push("src/new.rs");
        fs::write(&file, "a new file")?;
        assert!(command.paths_were_changed(prev)?);

        Ok(())
    }

    #[test]
    #[parallel]
    fn paths_were_changed_when_dir_has_file_deleted() -> Result<()> {
        let mut command = default_command();
        command.invocation.invoke = Invoke::PerDir;
        command.filter.includer = MatcherBuilder::new("/").with(&["**/*.rs"])?.build()?;
        command.filter.excluder = MatcherBuilder::new("/")
            .with(&["**/can_ignore.rs"])?
            .build()?;

        let helper = TestHelper::new()?.with_git_repo()?;
        let mut files = vec![];
        for path in helper.all_files() {
            if path.starts_with("src/")
                && path.to_str().unwrap().ends_with(".rs")
                && path.ancestors().count() == 3
            {
                let mut file = helper.git_root();
                file.push(path);
                files.push(file);
            }
        }

        let prev = command.maybe_path_metadata_for(
            ActualInvoke::PerDir,
            &files.iter().map(|f| f.as_ref()).collect::<Vec<_>>(),
        )?;
        assert!(prev.is_some());
        let prev = prev.unwrap();
        assert_eq!(
            prev.path_map.len(),
            3,
            "excluded files are not in the path map",
        );
        assert!(!command.paths_were_changed(prev.clone())?);

        fs::remove_file(files.pop().unwrap())?;
        assert!(command.paths_were_changed(prev)?);

        Ok(())
    }

    #[test_case(
        ActualInvoke::Once,
        &["**/*.go"],
        &["foo.go"],
        "foo.go";
        "invoke = once with one path"
    )]
    #[test_case(
        ActualInvoke::Once,
        &["**/*.go"],
        &["foo.go", "bar.go"],
        "bar.go foo.go";
        "invoke = once with two paths"
    )]
    #[test_case(
        ActualInvoke::Once,
        &["**/*.go"],
        &["foo.go", "bar.go", "baz.go"],
        "bar.go baz.go foo.go";
        "invoke = once with three paths"
    )]
    #[test_case(
        ActualInvoke::Once,
        &["**/*.go"],
        &["foo.go", "bar.go", "baz.go", "quux.go"],
        "4 files matching **/*.go, starting with bar.go baz.go";
        "invoke = once with four paths"
    )]
    #[test_case(
        ActualInvoke::Once,
        &["**/*.go", "!food.go"],
        &["foo.go", "bar.go", "baz.go", "quux.go"],
        "4 files matching **/*.go !food.go, starting with bar.go baz.go";
        "invoke = once with four paths and two includes"
    )]
    #[test_case(
        ActualInvoke::PerDir,
        &["**/*.go"],
        &["foo.go"],
        "foo.go";
        "invoke = dir with one path"
    )]
    #[test_case(
        ActualInvoke::PerDir,
        &["**/*.go"],
        &["foo.go", "bar.go"],
        "bar.go foo.go";
        "invoke = dir with two paths"
    )]
    #[test_case(
        ActualInvoke::PerDir,
        &["**/*.go"],
        &["foo.go", "bar.go", "baz.go"],
        "bar.go baz.go foo.go";
        "invoke = dir with three paths"
    )]
    #[test_case(
        ActualInvoke::PerDir,
        &["**/*.go"],
        &["foo.go", "bar.go", "baz.go", "quux.go"],
        "4 files matching **/*.go, starting with bar.go baz.go";
        "invoke = dir with four paths"
    )]
    #[test_case(
        ActualInvoke::PerDir,
        &["**/*.go", "!food.go"],
        &["foo.go", "bar.go", "baz.go", "quux.go"],
        "4 files matching **/*.go !food.go, starting with bar.go baz.go";
        "invoke = dir with four paths and two includes"
    )]
    #[test_case(
        ActualInvoke::PerFile,
        &["**/*.go", "!food.go"],
        &["foo.go"],
        "foo.go";
        "invoke = file"
    )]
    #[parallel]
    fn paths_summary(
        actual_invoke: ActualInvoke,
        include: &[&str],
        paths: &[&str],
        expect: &str,
    ) -> Result<()> {
        let mut command = default_command();
        command.name = String::from("Test");
        command.invocation.invoke = actual_invoke.as_invoke();
        command.filter.include = include.iter().map(|i| i.to_string()).collect();
        assert_eq!(
            &command.paths_summary(
                actual_invoke,
                &paths.iter().map(Path::new).collect::<Vec<_>>()
            ),
            expect,
        );

        Ok(())
    }
}
