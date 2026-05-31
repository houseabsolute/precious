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
    canonical_git_root: Option<Utf8PathBuf>,
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

    #[error(r#"The path "{path}" does not contain "{prefix}" as a prefix"#)]
    PrefixNotFound {
        path: Utf8PathBuf,
        prefix: Utf8PathBuf,
    },
}

static KEEP_INDEX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(".*").unwrap());

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
            canonical_git_root: None,
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
        // git rev-parse appends exactly one line terminator: \n on unix, \r\n
        // on Windows. Strip that one terminator — never trim spaces, tabs, or
        // repeated newlines, all of which are valid trailing characters in a
        // path.
        let trimmed = bytes
            .strip_suffix(b"\r\n")
            .or_else(|| bytes.strip_suffix(b"\n"))
            .unwrap_or(bytes);
        let s = std::str::from_utf8(trimmed).map_err(|_| NonUtf8PathError {
            raw: crate::paths::utf8::bytes_to_pathbuf(trimmed),
            source: NonUtf8Source::GitRoot,
        })?;
        self.git_root = Some(Utf8PathBuf::from(s));

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
            "-z",
            "--diff-filter=ACM",
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
            "-z",
            "--diff-filter=ACM",
        ])
    }

    fn maybe_git_stash(&mut self) -> Result<()> {
        if self.mode != Mode::GitStagedWithStash {
            return Ok(());
        }

        let git_root = self.git_root()?;
        let mut mm = git_root.clone();
        mm.push(".git");
        mm.push("MERGE_MODE");

        if !mm.exists() {
            Exec::builder()
                .exe("git")
                .args(vec!["stash", "--keep-index"])
                .ok_exit_codes(&[0])
                .ignore_stderr(vec![KEEP_INDEX_RE.clone()])
                .in_dir(&git_root)
                .build()
                .run()?;
            self.stashed = true;
        }

        Ok(())
    }

    fn git_modified_since(&mut self, since: &str) -> Result<Vec<Utf8PathBuf>> {
        let since_dot = format!("{since:}...");
        self.files_from_git(vec![
            "diff",
            "--name-only",
            "-z",
            "--diff-filter=ACM",
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
                    let path = Utf8PathBuf::from_path_buf(ent.into_path()).map_err(|raw| {
                        NonUtf8PathError {
                            raw,
                            source: NonUtf8Source::FilesystemWalk,
                        }
                    })?;
                    if path.is_dir() {
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
            .in_dir(&self.project_root)
            .build()
            .run()
            .context("Failed to run git to get list of files")?;
        let exclude_matcher = self
            .exclude_matcher()
            .context("Failed to build exclude matcher for git files")?;

        match output.stdout_bytes.as_deref() {
            Some(bytes) => {
                // In the common case where the git repo root and project root are the same, this
                // isn't necessary, because git will give us paths relative to the project root. But
                // if the precious root _isn't_ the git root, we need to get the path relative to
                // the project root, not the repo root.
                let canonical_git_root = self.canonical_git_root()?;
                let mut paths: Vec<Utf8PathBuf> = Vec::new();
                for raw in bytes.split(|b| *b == 0).filter(|s| !s.is_empty()) {
                    let Ok(s) = std::str::from_utf8(raw) else {
                        return Err(NonUtf8PathError {
                            raw: crate::paths::utf8::bytes_to_pathbuf(raw),
                            source: NonUtf8Source::GitDiff,
                        }
                        .into());
                    };

                    let rel = Utf8PathBuf::from(s);
                    if exclude_matcher.path_matches(&rel, false) {
                        continue;
                    }

                    let full = canonical_git_root.join(&rel);
                    if !full.exists() {
                        debug!(
                            "The staged file at {rel} (abs path {full}) was deleted so it will be ignored.",
                        );
                        continue;
                    }

                    paths.push(self.path_relative_to_canonical_root(&canonical_git_root, &full)?);
                }
                Ok(paths)
            }
            None => Ok(vec![]),
        }
    }

    fn canonical_git_root(&mut self) -> Result<Utf8PathBuf> {
        if let Some(r) = &self.canonical_git_root {
            return Ok(r.clone());
        }

        let raw = self.git_root()?;
        let canonical = raw
            .canonicalize_utf8()
            .with_context(|| format!("Failed to canonicalize git root path {raw}"))?;
        self.canonical_git_root = Some(canonical.clone());

        Ok(canonical)
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
    use serial_test::parallel;
    use std::fs;

    fn new_finder(mode: Mode, root: Utf8PathBuf) -> Result<Finder> {
        new_finder_with_excludes(mode, root.clone(), root, vec![])
    }

    fn new_finder_with_cwd(mode: Mode, root: Utf8PathBuf, cwd: Utf8PathBuf) -> Result<Finder> {
        new_finder_with_excludes(mode, root, cwd, vec![])
    }

    fn new_finder_with_excludes(
        mode: Mode,
        root: Utf8PathBuf,
        cwd: Utf8PathBuf,
        exclude: Vec<String>,
    ) -> Result<Finder> {
        Finder::new(mode, &root, cwd, exclude)
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

    #[cfg(unix)]
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

        let mut finder = new_finder(Mode::All, helper.precious_root())?;
        let err = finder.files(&[]).expect_err("expected non-UTF-8 error");
        let downcast = err
            .downcast_ref::<NonUtf8PathError>()
            .expect("expected NonUtf8PathError");
        assert_eq!(downcast.source, NonUtf8Source::FilesystemWalk);
        Ok(())
    }

    #[test]
    #[parallel]
    fn all_mode() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;

        let mut finder = new_finder(Mode::All, helper.precious_root())?;
        assert_eq!(finder.files(&[])?, Some(helper.all_files1()));
        Ok(())
    }

    #[test]
    #[parallel]
    fn all_mode_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut cwd = helper.precious_root();
        cwd.push("src");

        let mut finder = new_finder_with_cwd(Mode::All, helper.precious_root(), cwd)?;
        assert_eq!(finder.files(&[])?, Some(helper.all_files1()));
        Ok(())
    }

    #[test]
    #[parallel]
    fn all_mode_with_gitignore() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut gitignores: Vec<Utf8PathBuf> = helper
            .add_gitignore_files()?
            .into_iter()
            .map(|p| Utf8PathBuf::try_from(p).expect("gitignore file is valid UTF-8"))
            .collect();
        let mut expect = testhelper::TestHelper::non_ignored_files()
            .into_iter()
            .map(|p| Utf8PathBuf::try_from(p).expect("non-ignored file is valid UTF-8"))
            .collect::<Vec<_>>();
        expect.append(&mut gitignores);
        expect.sort();
        let expect = Vec1::try_from(expect).unwrap();

        let mut finder = new_finder(Mode::All, helper.precious_root())?;
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
            helper.precious_root(),
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
            helper.precious_root(),
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
        let mut finder = new_finder(Mode::GitModified, helper.precious_root())?;
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
        let mut finder = new_finder(Mode::GitModified, helper.precious_root())?;
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
        let mut finder = new_finder_with_cwd(Mode::GitModified, helper.precious_root(), cwd)?;
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
            helper.precious_root(),
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
            helper.precious_root(),
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
            helper.precious_root(),
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
            helper.precious_root(),
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
        let mut finder = new_finder(Mode::GitModified, project_root)?;
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
        let mut finder = new_finder(Mode::GitModified, helper.precious_root())?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_empty() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut finder = new_finder(Mode::GitStaged, helper.precious_root())?;
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
            let mut finder = new_finder(Mode::GitStaged, helper.precious_root())?;
            let res = finder.files(&[]);
            assert!(res.is_ok());
            assert!(res.unwrap().is_none());
        }

        {
            let mut finder = new_finder(Mode::GitStaged, helper.precious_root())?;
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
                new_finder_with_cwd(Mode::GitStaged, helper.precious_root(), cwd.clone())?;
            let res = finder.files(&[]);
            assert!(res.is_ok());
            assert!(res.unwrap().is_none());
        }

        {
            let mut finder = new_finder_with_cwd(Mode::GitStaged, helper.precious_root(), cwd)?;
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
            helper.precious_root(),
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
            helper.precious_root(),
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
            helper.precious_root(),
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
            helper.precious_root(),
            cwd,
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_with_stash_stashes_unindexed() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        helper.stage_all()?;
        let unstaged = "tests/data/bar.txt";
        helper.write_file(Utf8PathBuf::from(unstaged), "new content")?;

        #[cfg(not(target_os = "windows"))]
        set_up_post_checkout_hook(&helper)?;

        {
            let mut finder = new_finder(Mode::GitStagedWithStash, helper.precious_root())?;
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

        let mut finder = new_finder(Mode::GitStaged, helper.precious_root())?;
        assert_eq!(
            finder.files(&[])?,
            Some(vec1![Utf8PathBuf::from("merge-conflict-here")]),
        );
        assert!(!finder.stashed);
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

        let mut finder = new_finder(Mode::GitStaged, helper.precious_root())?;
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
            helper.precious_root(),
        )?;
        assert_eq!(finder.files(&[])?, None);

        let modified = Vec1::try_from(helper.modify_files()?).unwrap();
        helper.commit_all()?;

        let mut finder = new_finder(
            Mode::GitDiffFrom("master".to_string()),
            helper.precious_root(),
        )?;
        assert_eq!(finder.files(&[])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut finder = new_finder(Mode::FromCli, helper.precious_root())?;
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

    #[test]
    #[parallel]
    fn cli_mode_given_dir_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut cwd = helper.precious_root();
        cwd.push("src");
        let mut finder = new_finder_with_cwd(Mode::FromCli, helper.precious_root(), cwd)?;
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
        let mut finder = new_finder_with_cwd(Mode::FromCli, helper.precious_root(), cwd)?;
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
            helper.precious_root(),
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
            helper.precious_root(),
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
            helper.precious_root(),
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
            helper.precious_root(),
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
            helper.precious_root(),
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
            helper.precious_root(),
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
            helper.precious_root(),
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
        let mut finder = new_finder(Mode::FromCli, helper.precious_root())?;
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
            let mut finder = new_finder(Mode::GitStaged, link.clone())?;
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

        let mut finder = new_finder(Mode::FromCli, project_root)?;
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
