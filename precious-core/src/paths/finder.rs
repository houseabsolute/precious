use crate::{
    paths::{
        matcher::{Matcher, MatcherBuilder},
        mode::Mode,
        utf8::{NonUtf8PathError, NonUtf8Source},
    },
    vcs,
};
use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use log::{debug, error};
use mitsein::prelude::*;
use precious_helpers::exec::Exec;
use regex::Regex;
use std::sync::LazyLock;
use thiserror::Error;

#[derive(Debug)]
pub struct Finder {
    mode: Mode,
    project_root: Utf8PathBuf,
    git_root: Option<Utf8PathBuf>,
    cwd: Utf8PathBuf,
    exclude_globs: Vec<String>,
    stashed: bool,
}

#[derive(Debug, Error, Eq, PartialEq)]
#[allow(clippy::module_name_repetitions)]
pub enum FinderError {
    #[error("You cannot pass an explicit list of files when looking for {mode:}")]
    GotPathsFromCliWithWrongMode { mode: Mode },

    #[error("The path given on the command line ({path}) is excluded in the precious config")]
    CLIPathsWereExcludedSingular { path: String },

    #[error(
        "The paths given on the command line ({paths}) are all excluded in the precious config"
    )]
    CLIPathsWereExcludedMultiple { paths: String },

    #[error(
        "Attempted to find all matching paths but everything was excluded in the precious config"
    )]
    AllPathsWereExcluded,

    #[error("Path passed on the command line does not exist: {path}")]
    NonExistentPathOnCli { path: Utf8PathBuf },

    #[error(r#"Could not determine the repo root by running "git rev-parse --show-toplevel""#)]
    CouldNotDetermineRepoRoot,

    #[error(
        r#"Could not determine the path of {file} by running "git rev-parse --git-path {file}""#
    )]
    CouldNotDetermineGitPath { file: &'static str },

    #[error(r#"The path "{path}" does not contain "{prefix}" as a prefix"#)]
    PrefixNotFound {
        path: Utf8PathBuf,
        prefix: Utf8PathBuf,
    },
}

// Git writes to stderr even when nothing is wrong, so for some commands we only look at the exit
// code.
static ANY_STDERR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(".*").unwrap());

impl Finder {
    pub fn new(
        mode: Mode,
        project_root: &Utf8Path,
        cwd: Utf8PathBuf,
        exclude_globs: Vec<String>,
    ) -> Result<Finder> {
        let canonical_root = project_root
            .canonicalize_utf8()
            .with_context(|| format!("Failed to canonicalize project root path {project_root}"))?;

        Ok(Finder {
            mode,
            project_root: canonical_root,
            git_root: None,
            cwd,
            exclude_globs,
            stashed: false,
        })
    }

    pub fn files(&mut self, cli_paths: &[Utf8PathBuf]) -> Result<Option<Vec1<Utf8PathBuf>>> {
        match self.mode {
            Mode::FromCli => (),
            Mode::All
            | Mode::GitModified
            | Mode::GitStaged
            | Mode::GitStagedWithStash
            | Mode::GitDiffFrom(_) => {
                if !cli_paths.is_empty() {
                    return Err(FinderError::GotPathsFromCliWithWrongMode {
                        mode: self.mode.clone(),
                    }
                    .into());
                }
            }
        }

        let mut files = match self.mode.clone() {
            Mode::All => self.all_files().context("Failed to get all files")?,
            Mode::FromCli => self
                .files_from_cli(cli_paths)
                .context("Failed to get files from command line")?,
            Mode::GitModified => self
                .git_modified_files()
                .context("Failed to get git-modified files")?,
            Mode::GitStaged | Mode::GitStagedWithStash => self
                .git_staged_files()
                .context("Failed to get git-staged files")?,
            Mode::GitDiffFrom(ref from) => self
                .git_modified_since(from)
                .with_context(|| format!(r#"Failed to get files modified since "{from}""#))?,
        };
        files.sort();
        // Paths from the command line can overlap, like a directory and a file inside it. Without
        // this, a command would be given the same file more than once.
        files.dedup();

        if files.is_empty() {
            return match self.mode {
                Mode::GitModified
                | Mode::GitStaged
                | Mode::GitStagedWithStash
                | Mode::GitDiffFrom(_) => Ok(None),
                Mode::FromCli => {
                    let err = if cli_paths.len() == 1 {
                        FinderError::CLIPathsWereExcludedSingular {
                            path: cli_paths[0].as_str().to_string(),
                        }
                    } else {
                        FinderError::CLIPathsWereExcludedMultiple {
                            paths: Self::truncate_path_list(cli_paths),
                        }
                    };
                    Err(err.into())
                }
                Mode::All => Err(FinderError::AllPathsWereExcluded {}.into()),
            };
        }

        Ok(Some(
            files
                .try_into()
                .expect("we already checked that this is not empty"),
        ))
    }

    fn git_root(&mut self) -> Result<Utf8PathBuf> {
        if let Some(r) = &self.git_root {
            return Ok(r.clone());
        }

        let res = Exec::builder()
            .exe("git")
            .args(vec!["rev-parse", "--show-toplevel"])
            .ok_exit_codes(&[0])
            .in_dir(&self.project_root)
            .build()
            .run()
            .context("Failed to run git rev-parse to determine repository root")?;

        let bytes = res
            .stdout_bytes
            .as_deref()
            .ok_or(FinderError::CouldNotDetermineRepoRoot)
            .context("git rev-parse did not produce output")?;
        let s = path_from_git_output(bytes, NonUtf8Source::GitRoot)?;
        self.git_root = Some(s);

        Ok(self
            .git_root
            .clone()
            .expect("we know this is Some - look up a couple lines"))
    }

    fn all_files(&self) -> Result<Vec<Utf8PathBuf>> {
        debug!("Getting all files under {}", self.project_root);
        self.walkdir_files(&self.project_root)
    }

    fn files_from_cli(&self, cli_paths: &[Utf8PathBuf]) -> Result<Vec<Utf8PathBuf>> {
        debug!("Using the list of files passed from the command line");
        let exclude_matcher = self.exclude_matcher()?;

        let mut files: Vec<Utf8PathBuf> = vec![];
        for rel_to_cwd in cli_paths {
            let full = self.cwd.join(rel_to_cwd);
            if !full.exists() {
                return Err(FinderError::NonExistentPathOnCli {
                    path: rel_to_cwd.clone(),
                }
                .into());
            }

            let rel_to_root = self.path_relative_to_project_root(&full)?;
            if exclude_matcher.path_matches(&rel_to_root, full.is_dir()) {
                continue;
            }

            if full.is_dir() {
                let mut contents = self.walkdir_files(&full)?;
                files.append(&mut contents);
            } else {
                files.push(rel_to_root);
            }
        }

        Ok(files)
    }

    fn git_modified_files(&mut self) -> Result<Vec<Utf8PathBuf>> {
        debug!("Getting modified files according to git");
        self.files_from_git(vec![
            "diff",
            "--name-only",
            "--relative",
            "-z",
            "--diff-filter=ACMR",
            "HEAD",
        ])
    }

    fn git_staged_files(&mut self) -> Result<Vec<Utf8PathBuf>> {
        debug!("Getting staged files according to git");
        self.maybe_git_stash()?;
        self.files_from_git(vec![
            "diff",
            "--cached",
            "--name-only",
            "--relative",
            "-z",
            "--diff-filter=ACMR",
        ])
    }

    fn maybe_git_stash(&mut self) -> Result<()> {
        if self.mode != Mode::GitStagedWithStash {
            return Ok(());
        }

        let git_root = self.git_root()?;

        if !self.git_operation_in_progress()? {
            let before = Self::stash_tip(&git_root)?;
            Exec::builder()
                .exe("git")
                .args(vec!["stash", "--keep-index"])
                .ok_exit_codes(&[0])
                .ignore_stderr(vec![ANY_STDERR_RE.clone()])
                .in_dir(&git_root)
                .build()
                .run()?;
            // When there's nothing to stash, git exits 0 without creating an entry. If we set
            // `stashed` anyway, dropping the finder would pop a stash the user made earlier.
            self.stashed = Self::stash_tip(&git_root)? != before;
        }

        Ok(())
    }

    fn stash_tip(git_root: &Utf8Path) -> Result<Option<String>> {
        let res = Exec::builder()
            .exe("git")
            .args(vec!["rev-parse", "-q", "--verify", "refs/stash"])
            .ok_exit_codes(&[0, 1])
            .in_dir(git_root)
            .build()
            .run()
            .context("Failed to run git rev-parse to get the stash tip")?;
        if res.exit_code != 0 {
            return Ok(None);
        }
        Ok(res.stdout.map(|s| s.trim().to_string()))
    }

    // Stashing in the middle of a merge, cherry-pick, or revert throws away the state of that
    // operation, so we look for the file that git creates for each one. A conflicted cherry-pick
    // or revert does not create MERGE_MODE.
    //
    // We have to ask git where these files live instead of assuming they're in <git root>/.git. In
    // a worktree or submodule, `.git` is a file containing a gitdir: pointer and the real files
    // live somewhere else entirely.
    fn git_operation_in_progress(&mut self) -> Result<bool> {
        let git_root = self.git_root()?;
        for file in ["MERGE_MODE", "CHERRY_PICK_HEAD", "REVERT_HEAD"] {
            let res = Exec::builder()
                .exe("git")
                .args(vec!["rev-parse", "--git-path", file])
                .ok_exit_codes(&[0])
                .in_dir(&git_root)
                .build()
                .run()
                .with_context(|| {
                    format!("Failed to run git rev-parse to determine the path of {file}")
                })?;

            let bytes = res
                .stdout_bytes
                .as_deref()
                .ok_or(FinderError::CouldNotDetermineGitPath { file })
                .context("git rev-parse --git-path did not produce output")?;
            // This path may be relative, in which case it's relative to the
            // directory we ran git in.
            let path = git_root.join(path_from_git_output(bytes, NonUtf8Source::GitPath)?);
            if path.exists() {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn git_modified_since(&mut self, since: &str) -> Result<Vec<Utf8PathBuf>> {
        let since_dot = format!("{since:}...");
        self.files_from_git(vec![
            "diff",
            "--name-only",
            "--relative",
            "-z",
            "--diff-filter=ACMR",
            &since_dot,
        ])
    }

    fn walkdir_files(&self, root: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
        let canonical_root = root
            .canonicalize_utf8()
            .with_context(|| format!("Failed to canonicalize walk root {root}"))?;

        let mut exclude_globs = ignore::overrides::OverrideBuilder::new(&canonical_root);
        for d in vcs::DIRS {
            exclude_globs
                .add(&format!("!{d}/**/*"))
                .with_context(|| format!("Failed to add VCS directory override pattern for {d}"))?;
        }

        let overrides = exclude_globs
            .build()
            .context("Failed to build directory override patterns")?;

        let exclude_matcher = self
            .exclude_matcher()
            .context("Failed to build exclude matcher")?;

        let mut files: Vec<Utf8PathBuf> = vec![];
        for result in ignore::WalkBuilder::new(&canonical_root)
            .hidden(false)
            .overrides(overrides)
            .build()
        {
            match result {
                Ok(ent) => {
                    let path = match Utf8PathBuf::from_path_buf(ent.into_path()) {
                        Ok(p) => p,
                        Err(raw) => {
                            // An excluded file is never passed to a command, so it does not matter
                            // that its name is not UTF-8.
                            let is_excluded = raw
                                .strip_prefix(&self.project_root)
                                .is_ok_and(|rel| exclude_matcher.raw_path_matches(rel, false));
                            if is_excluded || !raw.is_file() {
                                continue;
                            }
                            return Err(NonUtf8PathError {
                                raw,
                                source: NonUtf8Source::FilesystemWalk,
                            }
                            .into());
                        }
                    };
                    // Besides directories, this skips things like sockets and FIFOs. A command
                    // cannot do anything useful with one of those, and reading from a FIFO can
                    // hang forever.
                    if !path.is_file() {
                        continue;
                    }

                    let rel = self.path_relative_to_canonical_root(&canonical_root, &path)?;
                    if exclude_matcher.path_matches(&rel, false) {
                        continue;
                    }

                    files.push(rel);
                }
                Err(e) => {
                    return Err(e).with_context(|| format!("Failed to walk directory {root}"))?
                }
            }
        }

        Ok(files)
    }

    fn files_from_git(&mut self, args: Vec<&str>) -> Result<Vec<Utf8PathBuf>> {
        let output = Exec::builder()
            .exe("git")
            .args(args)
            .ok_exit_codes(&[0])
            // Git prints warnings here that do not stop it from listing the files, like the line
            // ending warning when `core.autocrlf` is on.
            .ignore_stderr(vec![ANY_STDERR_RE.clone()])
            .in_dir(&self.project_root)
            .build()
            .run()
            .context("Failed to run git to get list of files")?;
        let exclude_matcher = self
            .exclude_matcher()
            .context("Failed to build exclude matcher for git files")?;

        match output.stdout_bytes.as_deref() {
            Some(bytes) => {
                // We pass `--relative` and run git in the project root, so git only lists paths
                // under the project root, relative to it. That matters when the precious root is
                // a subdirectory of the git root, as in a monorepo.
                let mut paths: Vec<Utf8PathBuf> = Vec::new();
                for raw in bytes.split(|b| *b == 0).filter(|s| !s.is_empty()) {
                    let Ok(s) = std::str::from_utf8(raw) else {
                        // An excluded file is never passed to a command, so it does not matter
                        // that its name is not UTF-8.
                        let raw = crate::paths::utf8::bytes_to_pathbuf(raw);
                        if exclude_matcher.raw_path_matches(&raw, false) {
                            continue;
                        }
                        return Err(NonUtf8PathError {
                            raw,
                            source: NonUtf8Source::GitDiff,
                        }
                        .into());
                    };

                    let rel = Utf8PathBuf::from(s);
                    if exclude_matcher.path_matches(&rel, false) {
                        continue;
                    }

                    let full = self.project_root.join(&rel);
                    if !full.exists() {
                        debug!(
                            "The staged file at {rel} (abs path {full}) was deleted so it will be ignored.",
                        );
                        continue;
                    }

                    paths.push(rel);
                }
                Ok(paths)
            }
            None => Ok(vec![]),
        }
    }

    fn exclude_matcher(&self) -> Result<Matcher> {
        MatcherBuilder::new(&self.project_root)
            .with(&self.exclude_globs)
            .context("Failed to add exclude globs to matcher")?
            .with(vcs::DIRS)
            .context("Failed to add VCS directories to matcher")?
            .build()
            .context("Failed to build exclude matcher")
    }

    fn path_relative_to_project_root(&self, path: &Utf8Path) -> Result<Utf8PathBuf> {
        let canonical = path
            .canonicalize_utf8()
            .with_context(|| format!("Failed to canonicalize path {path}"))?;

        let stripped = canonical.strip_prefix(&self.project_root).map_err(|_| {
            FinderError::PrefixNotFound {
                path: path.to_path_buf(),
                prefix: self.project_root.clone(),
            }
        })?;

        // When the input canonicalizes to the project root itself, strip_prefix yields an empty
        // path. Downstream code expects a non-empty value, so return "." in that case.
        if stripped.as_str().is_empty() {
            Ok(Utf8PathBuf::from("."))
        } else {
            Ok(stripped.to_path_buf())
        }
    }

    // Like `path_relative_to_project_root` but without a per-path canonicalize.  The caller must
    // have already canonicalized `path_root` and must guarantee that `rel` is a clean relative path
    // (no `..`, no symlink-bearing components beyond `path_root` itself). Used in hot paths where
    // we walk many files under a single fixed root.
    fn path_relative_to_canonical_root(
        &self,
        path_root: &Utf8Path,
        rel: &Utf8Path,
    ) -> Result<Utf8PathBuf> {
        let joined = if rel.is_absolute() {
            rel.to_path_buf()
        } else {
            path_root.join(rel)
        };

        let stripped =
            joined
                .strip_prefix(&self.project_root)
                .map_err(|_| FinderError::PrefixNotFound {
                    path: joined.clone(),
                    prefix: self.project_root.clone(),
                })?;

        if stripped.as_str().is_empty() {
            Ok(Utf8PathBuf::from("."))
        } else {
            Ok(stripped.to_path_buf())
        }
    }

    fn truncate_path_list(cli_paths: &[Utf8PathBuf]) -> String {
        let is_truncated = cli_paths.len() > 3;
        let truncated = cli_paths
            .iter()
            .map(|p| p.as_str().to_string())
            .take(3)
            .collect::<Vec<_>>()
            .join(", ");
        if is_truncated {
            format!("{truncated}, ... and {} more", cli_paths.len() - 3)
        } else {
            truncated
        }
    }
}

// git rev-parse appends exactly one line terminator to a path it prints: \n on
// unix, \r\n on Windows. Strip that one terminator — never trim spaces, tabs,
// or repeated newlines, all of which are valid trailing characters in a path.
fn path_from_git_output(bytes: &[u8], source: NonUtf8Source) -> Result<Utf8PathBuf> {
    let trimmed = bytes
        .strip_suffix(b"\r\n")
        .or_else(|| bytes.strip_suffix(b"\n"))
        .unwrap_or(bytes);
    let s = std::str::from_utf8(trimmed).map_err(|_| NonUtf8PathError {
        raw: crate::paths::utf8::bytes_to_pathbuf(trimmed),
        source,
    })?;

    Ok(Utf8PathBuf::from(s))
}

impl Drop for Finder {
    fn drop(&mut self) {
        if !self.stashed {
            return;
        }

        let res = Exec::builder()
            .exe("git")
            .args(vec!["stash", "pop"])
            .ok_exit_codes(&[0])
            .in_dir(&self.project_root)
            .build()
            .run();

        if res.is_ok() {
            return;
        }

        error!("Error popping stash: {}", res.unwrap_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use camino::Utf8PathBuf;
    use itertools::Itertools;
    use precious_testhelper as testhelper;
    use pretty_assertions::assert_eq;
    use serial_test::{parallel, serial};
    use std::fs;
    use test_case::test_case;

    fn new_finder(mode: Mode, root: &Utf8Path) -> Result<Finder> {
        new_finder_with_excludes(mode, root, root.to_path_buf(), vec![])
    }

    fn new_finder_with_cwd(mode: Mode, root: &Utf8Path, cwd: Utf8PathBuf) -> Result<Finder> {
        new_finder_with_excludes(mode, root, cwd, vec![])
    }

    fn new_finder_with_excludes(
        mode: Mode,
        root: &Utf8Path,
        cwd: Utf8PathBuf,
        exclude: Vec<String>,
    ) -> Result<Finder> {
        Finder::new(mode, root, cwd, exclude)
    }

    #[cfg(not(target_os = "windows"))]
    fn set_up_post_checkout_hook(helper: &testhelper::TestHelper) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let hook = r#"
            #!/bin/sh
            echo "post checkout hook output"
        "#;

        let mut file_path = helper.precious_root();
        file_path.push(".git/hooks/post-checkout");
        helper.write_file(&file_path, hook)?;

        let path_string = &file_path.into_os_string();
        let metadata = fs::metadata(path_string)?;
        let mut perms = metadata.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path_string, perms)?;
        Ok(())
    }

    // The default macOS filesystems refuse to create a file with a non-UTF-8 name.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    #[parallel]
    fn all_mode_errors_on_non_utf8_filename() -> Result<()> {
        use crate::paths::utf8::{NonUtf8PathError, NonUtf8Source};
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let bad_name = OsStr::from_bytes(b"data\xff.bin");
        let mut full = helper.precious_root().into_std_path_buf();
        full.push(bad_name);
        fs::write(&full, b"contents")?;

        let mut finder = new_finder(Mode::All, &helper.precious_root())?;
        let err = finder.files(&[]).expect_err("expected non-UTF-8 error");
        let downcast = err
            .downcast_ref::<NonUtf8PathError>()
            .expect("expected NonUtf8PathError");
        assert_eq!(downcast.source, NonUtf8Source::FilesystemWalk);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    #[parallel]
    fn all_mode_skips_paths_that_are_not_regular_files() -> Result<()> {
        use std::os::unix::net::UnixListener;

        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let _socket = UnixListener::bind(helper.precious_root().join("src/socket.rs"))?;

        let mut finder = new_finder(Mode::All, &helper.precious_root())?;
        let files = finder.files(&[])?.expect("the test repo has files");
        assert!(files.contains(&Utf8PathBuf::from("src/main.rs")));
        assert!(!files.contains(&Utf8PathBuf::from("src/socket.rs")));

        Ok(())
    }

    // A file that is excluded is never passed to a command, so its name does not have to be valid
    // UTF-8. The default macOS filesystems refuse to create a file with a non-UTF-8 name.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test_case(Mode::All; "all")]
    #[test_case(Mode::GitStaged; "staged")]
    #[parallel]
    fn excluded_non_utf8_filename_is_not_an_error(mode: Mode) -> Result<()> {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut full = helper.precious_root().into_std_path_buf();
        full.push("vendor");
        fs::create_dir(&full)?;
        full.push(OsStr::from_bytes(b"data\xff.txt"));
        fs::write(&full, b"contents")?;
        helper.stage_all()?;

        // In `--all` mode we find every file in the repo. In `--staged` mode the only staged file
        // is the excluded one.
        let expect = if mode == Mode::All {
            Some(helper.all_files1())
        } else {
            None
        };

        let mut finder = new_finder_with_excludes(
            mode,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor".to_string()],
        )?;
        assert_eq!(finder.files(&[])?, expect);
        Ok(())
    }

    #[test]
    #[parallel]
    fn all_mode() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;

        let mut finder = new_finder(Mode::All, &helper.precious_root())?;
        assert_eq!(finder.files(&[])?, Some(helper.all_files1()));
        Ok(())
    }

    #[test]
    #[parallel]
    fn all_mode_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut cwd = helper.precious_root();
        cwd.push("src");

        let mut finder = new_finder_with_cwd(Mode::All, &helper.precious_root(), cwd)?;
        assert_eq!(finder.files(&[])?, Some(helper.all_files1()));
        Ok(())
    }

    #[test]
    #[parallel]
    fn all_mode_with_gitignore() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut gitignores = helper.add_gitignore_files()?;
        let mut expect = testhelper::TestHelper::non_ignored_files();
        expect.append(&mut gitignores);
        expect.sort();
        let expect = Vec1::try_from(expect).unwrap();

        let mut finder = new_finder(Mode::All, &helper.precious_root())?;
        assert_eq!(finder.files(&[])?, Some(expect));
        Ok(())
    }

    #[test]
    #[parallel]
    fn all_mode_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "new content")?;
        let mut finder = new_finder_with_excludes(
            Mode::All,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(&[])?, Some(helper.all_files1()));
        Ok(())
    }

    #[test]
    #[parallel]
    fn all_mode_with_excluded_files_bare_dir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "new content")?;
        let mut finder = new_finder_with_excludes(
            Mode::All,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor".to_string()],
        )?;
        assert_eq!(finder.files(&[])?, Some(helper.all_files1()));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_empty() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut finder = new_finder(Mode::GitModified, &helper.precious_root())?;
        let res = finder.files(&[]);
        assert!(res.is_ok());
        assert!(res.unwrap().is_none());
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_with_changes() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        let mut finder = new_finder(Mode::GitModified, &helper.precious_root())?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_with_changes_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        let mut cwd = helper.precious_root();
        cwd.push("src");
        let mut finder = new_finder_with_cwd(Mode::GitModified, &helper.precious_root(), cwd)?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_with_changes_all_excluded() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;

        let mut finder = new_finder_with_excludes(
            Mode::GitModified,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(&[])?, None);
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;
        helper.commit_all()?;

        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "new content")?;
        let mut finder = new_finder_with_excludes(
            Mode::GitModified,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_with_excluded_files_bare_dir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;
        helper.commit_all()?;

        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "new content")?;
        let mut finder = new_finder_with_excludes(
            Mode::GitModified,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor".to_string()],
        )?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_with_excluded_files_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;
        helper.commit_all()?;

        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "new content")?;
        let mut cwd = helper.precious_root();
        cwd.push("src");
        let mut finder = new_finder_with_excludes(
            Mode::GitModified,
            &helper.precious_root(),
            cwd,
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_when_repo_root_ne_precious_root() -> Result<()> {
        let helper = testhelper::TestHelper::new()?
            .with_precious_root_in_subdir("subdir")
            .with_git_repo()?;
        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        let mut project_root = helper.git_root();
        project_root.push("subdir");
        let mut finder = new_finder(Mode::GitModified, &project_root)?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_ignores_changes_outside_precious_root() -> Result<()> {
        git_mode_ignores_changes_outside_precious_root(Mode::GitModified)
    }

    #[test]
    #[parallel]
    fn git_staged_mode_ignores_changes_outside_precious_root() -> Result<()> {
        git_mode_ignores_changes_outside_precious_root(Mode::GitStaged)
    }

    fn git_mode_ignores_changes_outside_precious_root(mode: Mode) -> Result<()> {
        let helper = testhelper::TestHelper::new()?
            .with_precious_root_in_subdir("subdir")
            .with_git_repo()?;
        let outside = helper.git_root().join("outside.txt");
        fs::write(&outside, "some text")?;
        helper.stage_all()?;
        helper.commit_all()?;

        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        fs::write(&outside, "new text")?;
        helper.stage_all()?;

        let mut finder = new_finder(mode, &helper.precious_root())?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    // An anchored exclude like `vendor/**/*` is relative to the precious root, so it has to be
    // matched against paths relative to that root and not to the git root.
    #[test_case(Mode::GitModified; "git")]
    #[test_case(Mode::GitStaged; "staged")]
    #[test_case(Mode::GitDiffFrom("master".to_string()); "git-diff-from")]
    #[parallel]
    fn git_modes_apply_excludes_when_repo_root_ne_precious_root(mode: Mode) -> Result<()> {
        let helper = testhelper::TestHelper::new()?
            .with_precious_root_in_subdir("subdir")
            .with_git_repo()?;
        helper.write_file("vendor/foo/bar.txt", "initial content")?;
        helper.stage_all()?;
        helper.commit_all()?;

        helper.switch_to_branch("new-branch", false)?;
        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        helper.write_file("vendor/foo/bar.txt", "new content")?;
        helper.stage_all()?;
        if matches!(mode, Mode::GitDiffFrom(_)) {
            helper.commit_all()?;
        }

        let mut finder = new_finder_with_excludes(
            mode,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    // Git prints warnings to stderr that do not mean anything went wrong. The best known one is
    // the line ending warning that Windows users see when `core.autocrlf` is on.
    #[test]
    #[parallel]
    fn git_modified_mode_ignores_git_warnings_on_stderr() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        Exec::builder()
            .exe("git")
            .args(vec!["config", "core.autocrlf", "true"])
            .ok_exit_codes(&[0])
            .in_dir(&helper.git_root())
            .build()
            .run()?;

        let modified = Vec1::try_from(helper.modify_files()?).unwrap();

        let warning = Exec::builder()
            .exe("git")
            .args(vec!["diff", "--name-only", "HEAD"])
            .ok_exit_codes(&[0])
            .ignore_stderr(vec![ANY_STDERR_RE.clone()])
            .in_dir(&helper.git_root())
            .build()
            .run()?;
        assert!(
            warning.stderr.unwrap_or_default().contains("warning:"),
            "git diff prints a warning to stderr",
        );

        let mut finder = new_finder(Mode::GitModified, &helper.precious_root())?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_includes_staged() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        let first = modified[0].clone();
        helper.stage_some(&[first.as_std_path()])?;
        let mut finder = new_finder(Mode::GitModified, &helper.precious_root())?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_empty() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut finder = new_finder(Mode::GitStaged, &helper.precious_root())?;
        let res = finder.files(&[]);
        assert!(res.is_ok());
        assert!(res.unwrap().is_none());
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_with_changes() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = Vec1::try_from(helper.modify_files()?).unwrap();

        {
            let mut finder = new_finder(Mode::GitStaged, &helper.precious_root())?;
            let res = finder.files(&[]);
            assert!(res.is_ok());
            assert!(res.unwrap().is_none());
        }

        {
            let mut finder = new_finder(Mode::GitStaged, &helper.precious_root())?;
            helper.stage_all()?;
            assert_eq!(finder.files(&[])?, Some(modified));
        }
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_with_changes_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = Vec1::try_from(helper.modify_files()?).unwrap();

        let mut cwd = helper.precious_root();
        cwd.push("src");

        {
            let mut finder =
                new_finder_with_cwd(Mode::GitStaged, &helper.precious_root(), cwd.clone())?;
            let res = finder.files(&[]);
            assert!(res.is_ok());
            assert!(res.unwrap().is_none());
        }

        {
            let mut finder = new_finder_with_cwd(Mode::GitStaged, &helper.precious_root(), cwd)?;
            helper.stage_all()?;
            assert_eq!(finder.files(&[])?, Some(modified));
        }
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_with_changes_all_excluded() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;

        let mut finder = new_finder_with_excludes(
            Mode::GitStaged,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(&[])?, None);
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;
        let mut finder = new_finder_with_excludes(
            Mode::GitStaged,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_with_excluded_files_bare_dir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;
        let mut finder = new_finder_with_excludes(
            Mode::GitStaged,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor".to_string()],
        )?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_with_excluded_files_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;
        let mut cwd = helper.precious_root();
        cwd.push("src");
        let mut finder = new_finder_with_excludes(
            Mode::GitStaged,
            &helper.precious_root(),
            cwd,
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    // This writes a hook script that git then runs. If a test in another thread forks while the
    // script is open for writing, the child holds the file open and running it fails with "Text
    // file busy".
    #[test]
    #[serial]
    fn git_staged_mode_with_stash_stashes_unindexed() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        helper.stage_all()?;
        let unstaged = "tests/data/bar.txt";
        helper.write_file(Utf8PathBuf::from(unstaged), "new content")?;

        #[cfg(not(target_os = "windows"))]
        set_up_post_checkout_hook(&helper)?;

        {
            let mut finder = new_finder(Mode::GitStagedWithStash, &helper.precious_root())?;
            assert_eq!(finder.files(&[])?, Some(modified));
            assert_eq!(
                String::from_utf8(fs::read(helper.precious_root().join(unstaged))?)?,
                String::from("some text"),
            );
        }
        assert_eq!(
            String::from_utf8(fs::read(helper.precious_root().join(unstaged))?)?,
            String::from("new content"),
        );
        Ok(())
    }

    // When there are no unstaged changes, `git stash` creates no entry. We must not pop a stash
    // that the user made before running precious.
    #[test]
    #[parallel]
    fn git_staged_mode_with_stash_leaves_existing_stash_alone() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let file = Utf8PathBuf::from("tests/data/bar.txt");
        helper.write_file(&file, "old work in progress")?;
        Exec::builder()
            .exe("git")
            .args(vec!["stash"])
            .ok_exit_codes(&[0])
            .ignore_stderr(vec![ANY_STDERR_RE.clone()])
            .in_dir(&helper.git_root())
            .build()
            .run()?;

        {
            let mut finder = new_finder(Mode::GitStagedWithStash, &helper.precious_root())?;
            assert_eq!(finder.files(&[])?, None);
            assert!(!finder.stashed);
        }

        assert_eq!(helper.read_file(&file)?, "some text");
        let list = Exec::builder()
            .exe("git")
            .args(vec!["stash", "list"])
            .ok_exit_codes(&[0])
            .in_dir(&helper.git_root())
            .build()
            .run()?;
        assert_eq!(list.stdout.unwrap_or_default().lines().count(), 1);
        Ok(())
    }

    // This tests the issue reported in
    // https://github.com/houseabsolute/precious/issues/9. I had tried to test
    // for this earlier, but I thought it was a non-issue because I couldn't
    // replicate it. Later, I realized that this only happens if a merge
    // commit leads to a conflict. Otherwise, `git diff --cached` won't report
    // any files at all for the commit. But if you've had a conflict and
    // resolved it, any files that had a conflict will be reported as having a
    // diff.
    #[test]
    #[parallel]
    fn git_staged_mode_with_stash_merge_stash() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;

        let file = Utf8Path::new("merge-conflict-here");
        helper.write_file(file, "line 1\nline 2\n")?;
        helper.stage_all()?;
        helper.commit_all()?;

        helper.switch_to_branch("new-branch", false)?;
        helper.write_file(file, "line 1\nline 1.5\nline 2\n")?;
        helper.commit_all()?;

        helper.switch_to_branch("master", true)?;
        helper.write_file(file, "line 1\nline 1.6\nline 2\n")?;
        helper.commit_all()?;

        helper.switch_to_branch("new-branch", true)?;
        helper.merge_master(true)?;
        helper.write_file(file, "line 1\nline 1.7\nline 2\n")?;
        helper.stage_all()?;

        let mut finder = new_finder(Mode::GitStaged, &helper.precious_root())?;
        assert_eq!(
            finder.files(&[])?,
            Some(vec1![Utf8PathBuf::from("merge-conflict-here")]),
        );
        assert!(!finder.stashed);
        Ok(())
    }

    // In a worktree, `.git` is a file containing a gitdir: pointer, so the
    // MERGE_MODE file is not at <git root>/.git/MERGE_MODE. If we don't detect
    // the in-progress merge we'll stash in the middle of a conflicted merge.
    #[test]
    #[parallel]
    fn git_staged_mode_with_stash_does_not_stash_during_merge_in_worktree() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;

        let file = Utf8Path::new("merge-conflict-here");
        helper.write_file(file, "line 1\nline 2\n")?;
        helper.stage_all()?;
        helper.commit_all()?;

        helper.switch_to_branch("new-branch", false)?;
        helper.write_file(file, "line 1\nline 1.5\nline 2\n")?;
        helper.commit_all()?;

        helper.switch_to_branch("master", true)?;
        helper.write_file(file, "line 1\nline 1.6\nline 2\n")?;
        helper.commit_all()?;

        // A second TestHelper just gives us an empty temp directory that will
        // be cleaned up when it's dropped. Git is happy to create a worktree
        // in an existing empty directory.
        let wt_helper = testhelper::TestHelper::new()?;
        let wt_root = wt_helper.git_root();
        Exec::builder()
            .exe("git")
            .args(vec!["worktree", "add", wt_root.as_str(), "new-branch"])
            .ok_exit_codes(&[0])
            .ignore_stderr(vec![ANY_STDERR_RE.clone()])
            .in_dir(&helper.git_root())
            .build()
            .run()?;

        Exec::builder()
            .exe("git")
            .args(vec!["merge", "--quiet", "--no-ff", "--no-commit", "master"])
            .ok_exit_codes(&[0, 1])
            .ignore_stderr(vec![ANY_STDERR_RE.clone()])
            .in_dir(&wt_root)
            .build()
            .run()?;
        fs::write(wt_root.join(file), "line 1\nline 1.7\nline 2\n")?;
        Exec::builder()
            .exe("git")
            .args(vec!["add", "."])
            .ok_exit_codes(&[0])
            .in_dir(&wt_root)
            .build()
            .run()?;

        let mut finder = new_finder(Mode::GitStagedWithStash, &wt_root)?;
        assert_eq!(finder.files(&[])?, Some(vec1![file.to_path_buf()]));
        assert!(!finder.stashed);
        Ok(())
    }

    // A conflicted cherry-pick or revert does not create MERGE_MODE, but stashing in the middle of
    // one still throws away its state. Git deletes CHERRY_PICK_HEAD or REVERT_HEAD when it stashes,
    // so the user can no longer continue the operation.
    #[test_case("cherry-pick", "CHERRY_PICK_HEAD"; "cherry-pick")]
    #[test_case("revert", "REVERT_HEAD"; "revert")]
    #[parallel]
    fn git_staged_mode_with_stash_does_not_stash_during(op: &str, head_file: &str) -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let git_root = helper.git_root();

        let file = Utf8Path::new("conflict-here");
        helper.write_file(file, "line 1\nline 2\n")?;
        helper.stage_all()?;
        helper.commit_all()?;

        // Both operations conflict because the commit they apply changes a line that the current
        // HEAD has also changed.
        let commit = if op == "cherry-pick" {
            helper.switch_to_branch("new-branch", false)?;
            helper.write_file(file, "line 1\nline 1.5\nline 2\n")?;
            helper.commit_all()?;

            helper.switch_to_branch("master", true)?;
            helper.write_file(file, "line 1\nline 1.6\nline 2\n")?;
            helper.commit_all()?;

            "new-branch"
        } else {
            helper.write_file(file, "line 1\nline 1.5\nline 2\n")?;
            helper.commit_all()?;
            helper.write_file(file, "line 1\nline 1.6\nline 2\n")?;
            helper.commit_all()?;

            "HEAD~1"
        };

        Exec::builder()
            .exe("git")
            .args(vec![op, "--no-edit", commit])
            .ok_exit_codes(&[1])
            .ignore_stderr(vec![ANY_STDERR_RE.clone()])
            .in_dir(&git_root)
            .build()
            .run()?;
        assert!(
            git_root.join(".git").join(head_file).exists(),
            "{head_file} exists after a conflicted {op}",
        );

        helper.write_file(file, "line 1\nline 1.7\nline 2\n")?;
        helper.stage_all()?;
        // Without an unstaged change there would be nothing to stash.
        helper.write_file("README.md", "an unstaged change\n")?;

        let mut finder = new_finder(Mode::GitStagedWithStash, &helper.precious_root())?;
        assert_eq!(finder.files(&[])?, Some(vec1![file.to_path_buf()]));
        assert!(!finder.stashed, "did not stash during a {op}");
        assert!(
            git_root.join(".git").join(head_file).exists(),
            "{head_file} still exists after finding the staged files",
        );
        Ok(())
    }

    // Git reports a file that was moved and then edited as a rename, not as an added file. The new
    // path still has changed content, so it has to be included.
    #[test]
    #[parallel]
    fn git_modes_include_renamed_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let root = helper.precious_root();

        // The file needs enough content that git still sees it as the same file after the edit.
        let content = (1..=20).map(|i| format!("line {i}")).join("\n") + "\n";
        helper.write_file("old-name.txt", &content)?;
        helper.stage_all()?;
        helper.commit_all()?;

        helper.switch_to_branch("new-branch", false)?;
        fs::rename(root.join("old-name.txt"), root.join("new-name.txt"))?;
        helper.write_file("new-name.txt", &format!("{content}one more line\n"))?;
        helper.stage_all()?;

        let status = Exec::builder()
            .exe("git")
            .args(vec!["diff", "--cached", "--name-status"])
            .ok_exit_codes(&[0])
            .in_dir(&helper.git_root())
            .build()
            .run()?;
        assert!(
            status.stdout.unwrap_or_default().starts_with('R'),
            "git reports the file as renamed",
        );

        let expect = Some(vec1![Utf8PathBuf::from("new-name.txt")]);

        let mut finder = new_finder(Mode::GitStaged, &root)?;
        assert_eq!(finder.files(&[])?, expect, "--staged finds the new path");

        let mut finder = new_finder(Mode::GitModified, &root)?;
        assert_eq!(finder.files(&[])?, expect, "--git finds the new path");

        helper.commit_all()?;
        let mut finder = new_finder(Mode::GitDiffFrom("master".to_string()), &root)?;
        assert_eq!(
            finder.files(&[])?,
            expect,
            "--git-diff-from finds the new path",
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_with_deleted_file() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut modified = helper.modify_files()?;
        helper.stage_all()?;
        let first = modified.remove(0);
        helper.delete_file(&first)?;

        let mut finder = new_finder(Mode::GitStaged, &helper.precious_root())?;
        assert_eq!(finder.files(&[])?, Some(Vec1::try_from(modified).unwrap()));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_since() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.switch_to_branch("some-branch", false)?;

        // When there are no commits in the branch the diff between master and
        // the branch finds no files.
        let mut finder = new_finder(
            Mode::GitDiffFrom("master".to_string()),
            &helper.precious_root(),
        )?;
        assert_eq!(finder.files(&[])?, None);

        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        helper.commit_all()?;

        let mut finder = new_finder(
            Mode::GitDiffFrom("master".to_string()),
            &helper.precious_root(),
        )?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut finder = new_finder(Mode::FromCli, &helper.precious_root())?;
        let expect = helper
            .all_files()
            .into_iter()
            .filter(|p| p.starts_with("tests/"))
            .sorted()
            .try_collect1()
            .unwrap();
        assert_eq!(finder.files(&[Utf8PathBuf::from("tests")])?, Some(expect));
        Ok(())
    }

    // A file that is listed twice would be passed to a command twice, and a command that runs once
    // per file would run on it twice, maybe at the same time.
    #[test_case(&["src", "src/main.rs"]; "a dir and a file inside it")]
    #[test_case(&["src", "src"]; "the same dir twice")]
    #[test_case(&["src", "./src"]; "the same dir spelled two ways")]
    #[parallel]
    fn cli_mode_given_overlapping_paths_has_no_duplicates(cli_paths: &[&str]) -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut finder = new_finder(Mode::FromCli, &helper.precious_root())?;
        let expect = helper
            .all_files()
            .into_iter()
            .filter(|p| p.starts_with("src/"))
            .sorted()
            .try_collect1()
            .unwrap();
        let cli_paths = cli_paths.iter().map(Utf8PathBuf::from).collect::<Vec<_>>();
        assert_eq!(finder.files(&cli_paths)?, Some(expect));
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_the_same_file_twice_has_no_duplicates() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut finder = new_finder(Mode::FromCli, &helper.precious_root())?;
        let expect = vec1![Utf8PathBuf::from("src/main.rs")];
        assert_eq!(
            finder.files(&[
                Utf8PathBuf::from("src/main.rs"),
                Utf8PathBuf::from("src/main.rs"),
            ])?,
            Some(expect),
        );
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_dir_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut cwd = helper.precious_root();
        cwd.push("src");
        let mut finder = new_finder_with_cwd(Mode::FromCli, &helper.precious_root(), cwd)?;
        let expect = helper
            .all_files()
            .into_iter()
            .filter(|p| p.starts_with("src/"))
            .sorted()
            .try_collect1()
            .unwrap();
        assert_eq!(finder.files(&[Utf8PathBuf::from(".")])?, Some(expect));
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_files_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut cwd = helper.precious_root();
        cwd.push("src");
        let mut finder = new_finder_with_cwd(Mode::FromCli, &helper.precious_root(), cwd)?;
        let expect = ["src/main.rs", "src/module.rs"]
            .iter()
            .map(Utf8PathBuf::from)
            .try_collect1()
            .unwrap();
        assert_eq!(
            finder.files(&[Utf8PathBuf::from("main.rs"), Utf8PathBuf::from("module.rs")])?,
            Some(expect),
        );
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_dir_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        let mut finder = new_finder_with_excludes(
            Mode::FromCli,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(
            finder.files(&[Utf8PathBuf::from(".")])?,
            Some(helper.all_files1()),
        );
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_dir_with_excluded_files_bare_dir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        let mut finder = new_finder_with_excludes(
            Mode::FromCli,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor".to_string()],
        )?;
        assert_eq!(
            finder.files(&[Utf8PathBuf::from(".")])?,
            Some(helper.all_files1()),
        );
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_dir_with_excluded_files_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        let mut cwd = helper.precious_root();
        cwd.push("src");
        let mut finder = new_finder_with_excludes(
            Mode::FromCli,
            &helper.precious_root(),
            cwd,
            vec!["src/main.rs".to_string()],
        )?;
        let expect = [
            "src/bar.rs",
            "src/can_ignore.rs",
            "src/module.rs",
            "src/sub/mod.rs",
        ]
        .iter()
        .map(Utf8PathBuf::from)
        .try_collect1()
        .unwrap();
        assert_eq!(finder.files(&[Utf8PathBuf::from(".")])?, Some(expect));
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_files_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        let mut finder = new_finder_with_excludes(
            Mode::FromCli,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        let last_file = helper.all_files().pop().unwrap();
        let expect = vec1![last_file.clone()];
        let cli_paths = vec![last_file, Utf8PathBuf::from("vendor/foo/bar.txt")];
        assert_eq!(finder.files(&cli_paths)?, Some(expect));
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_files_with_excluded_files_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8PathBuf::from("src/main.rs"), "initial content")?;
        let mut cwd = helper.precious_root();
        cwd.push("src");
        let mut finder = new_finder_with_excludes(
            Mode::FromCli,
            &helper.precious_root(),
            cwd,
            vec!["src/main.rs".to_string()],
        )?;
        let expect = ["src/module.rs"]
            .iter()
            .map(Utf8PathBuf::from)
            .try_collect1()
            .unwrap();
        let cli_paths = ["main.rs", "module.rs"]
            .iter()
            .map(Utf8PathBuf::from)
            .collect::<Vec<_>>();
        assert_eq!(finder.files(&cli_paths)?, Some(expect));
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_dir_all_excluded_singular() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        let mut finder = new_finder_with_excludes(
            Mode::FromCli,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        let res = finder.files(&[Utf8PathBuf::from("vendor")]);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(
            matches!(
                err.downcast_ref(),
                Some(FinderError::CLIPathsWereExcludedSingular { .. })
            ),
            "expected CLIPathsWereExcludedSingular, got {err}",
        );
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_dir_all_excluded_multiple() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        let mut finder = new_finder_with_excludes(
            Mode::FromCli,
            &helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        let res = finder.files(&[Utf8PathBuf::from("vendor"), Utf8PathBuf::from("vendor")]);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(
            matches!(
                err.downcast_ref(),
                Some(FinderError::CLIPathsWereExcludedMultiple { .. })
            ),
            "expected CLIPathsWereExcludedSingular, got {err}",
        );
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_files_with_nonexistent_path() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut finder = new_finder(Mode::FromCli, &helper.precious_root())?;
        let cli_paths = vec![
            helper.all_files()[0].clone(),
            Utf8PathBuf::from("does/not/exist"),
        ];
        let res = finder.files(&cli_paths);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(
            err.downcast_ref(),
            Some(&FinderError::NonExistentPathOnCli {
                path: Utf8PathBuf::from("does/not/exist")
            })
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    #[parallel]
    fn git_mode_works_when_project_root_reached_via_symlink() -> Result<()> {
        // Reaching the project root via a symlink used to work only because we
        // canonicalized every git-produced path. Now we canonicalize the git
        // root once; this test guards that the cached canonical root is what we
        // use, not the symlink path the user passed in.
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(Utf8Path::new("src/foo.rs"), "fn foo() {}\n")?;
        helper.stage_all()?;

        let real_root = helper.precious_root();
        let parent = real_root.parent().expect("project root has a parent");
        let link = parent.join(format!(
            "{}-link",
            real_root.file_name().expect("project root has a name"),
        ));
        // Best-effort cleanup if a prior failed run left it behind.
        let _ = std::fs::remove_file(link.as_std_path());
        std::os::unix::fs::symlink(real_root.as_std_path(), link.as_std_path())?;

        let result = (|| -> Result<()> {
            let mut finder = new_finder(Mode::GitStaged, &link)?;
            let files = finder
                .files(&[])?
                .expect("expected at least one staged file");
            assert!(
                files.iter().any(|p| p == Utf8Path::new("src/foo.rs")),
                "expected src/foo.rs in {files:?}",
            );
            Ok(())
        })();

        std::fs::remove_file(link.as_std_path())?;
        result
    }

    #[test]
    #[parallel]
    fn cli_mode_given_path_outside_project_root() -> Result<()> {
        // When precious_root is a subdir of git_root, a file that exists in
        // git_root but above precious_root is outside the project root and
        // path_relative_to_project_root must reject it with PrefixNotFound.
        let helper = testhelper::TestHelper::new()?
            .with_precious_root_in_subdir("subdir")
            .with_git_repo()?;
        let project_root = helper.precious_root();
        let canonical_project_root = project_root.canonicalize_utf8()?;
        let mut outside = helper.git_root();
        outside.push("outside.txt");
        std::fs::write(&outside, b"content")?;

        let mut finder = new_finder(Mode::FromCli, &project_root)?;
        let err = finder
            .files(&[outside.clone()])
            .expect_err("expected PrefixNotFound");
        assert_eq!(
            err.downcast_ref(),
            Some(&FinderError::PrefixNotFound {
                path: outside,
                prefix: canonical_project_root,
            }),
        );
        Ok(())
    }
}
