use crate::{
    paths::{
        matcher::{Matcher, MatcherBuilder},
        mode::Mode,
    },
    vcs,
};
use anyhow::Result;
use clean_path::Clean;
use log::{debug, error};
use precious_helpers::exec;
use regex::Regex;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};
use thiserror::Error;

#[derive(Debug)]
pub struct Finder {
    mode: Mode,
    project_root: PathBuf,
    git_root: Option<PathBuf>,
    cwd: PathBuf,
    exclude_globs: Vec<String>,
    stashed: bool,
}

#[derive(Debug, Error, Eq, PartialEq)]
#[allow(clippy::module_name_repetitions)]
pub enum FinderError {
    #[error("You cannot pass an explicit list of files when looking for {mode:}")]
    GotPathsFromCliWithWrongMode { mode: Mode },

    #[error("Found some paths when looking for {mode:} but they were all excluded")]
    AllPathsWereExcluded { mode: Mode },

    #[error("Path passed on the command line does not exist: {}", path.display())]
    NonExistentPathOnCli { path: PathBuf },

    #[error("Could not determine the repo root by running \"git rev-parse --show-toplevel\"")]
    CouldNotDetermineRepoRoot,

    #[error("The path \"{}\" does not contain \"{}\" as a prefix", path.display(), prefix.display())]
    PrefixNotFound { path: PathBuf, prefix: PathBuf },
}

static KEEP_INDEX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(".*").unwrap());

impl Finder {
    pub fn new(
        mode: Mode,
        project_root: PathBuf,
        cwd: PathBuf,
        exclude_globs: Vec<String>,
    ) -> Result<Finder> {
        Ok(Finder {
            mode,
            project_root: fs::canonicalize(project_root)?,
            git_root: None,
            cwd,
            exclude_globs,
            stashed: false,
        })
    }

    pub fn files(&mut self, cli_paths: Vec<PathBuf>) -> Result<Option<Vec<PathBuf>>> {
        match self.mode {
            Mode::FromCli => (),
            _ => {
                if !cli_paths.is_empty() {
                    return Err(FinderError::GotPathsFromCliWithWrongMode {
                        mode: self.mode.clone(),
                    }
                    .into());
                }
            }
        };

        let mut files = match self.mode.clone() {
            Mode::All => self.all_files()?,
            Mode::FromCli => self.files_from_cli(cli_paths)?,
            Mode::GitModified => self.git_modified_files()?,
            Mode::GitStaged | Mode::GitStagedWithStash => self.git_staged_files()?,
            Mode::GitDiffFrom(ref from) => self.git_modified_since(from)?,
        };
        files.sort();

        if files.is_empty() {
            return match self.mode {
                Mode::GitModified
                | Mode::GitStaged
                | Mode::GitStagedWithStash
                | Mode::GitDiffFrom(_) => Ok(None),
                _ => Err(FinderError::AllPathsWereExcluded {
                    mode: self.mode.clone(),
                }
                .into()),
            };
        }

        Ok(Some(files))
    }

    fn git_root(&mut self) -> Result<PathBuf> {
        if let Some(r) = &self.git_root {
            return Ok(r.clone());
        }

        let res = exec::run(
            "git",
            &["rev-parse", "--show-toplevel"],
            &HashMap::new(),
            &[0],
            None,
            Some(&self.project_root),
        )?;

        let stdout = res.stdout.ok_or(FinderError::CouldNotDetermineRepoRoot)?;
        self.git_root = Some(PathBuf::from(stdout.trim()));

        Ok(self.git_root.clone().unwrap())
    }

    fn all_files(&self) -> Result<Vec<PathBuf>> {
        debug!("Getting all files under {}", self.project_root.display());
        self.walkdir_files(self.project_root.as_path())
    }

    fn files_from_cli(&self, cli_paths: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
        debug!("Using the list of files passed from the command line");
        let excluder = self.excluder()?;

        let mut files: Vec<PathBuf> = vec![];
        for rel_to_cwd in cli_paths {
            let full = self.cwd.clone().join(rel_to_cwd.clone());
            if !full.exists() {
                return Err(FinderError::NonExistentPathOnCli { path: rel_to_cwd }.into());
            }

            let rel_to_root = self.path_relative_to_project_root(&full)?;
            if excluder.path_matches(&rel_to_root, full.is_dir()) {
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

    fn git_modified_files(&mut self) -> Result<Vec<PathBuf>> {
        debug!("Getting modified files according to git");
        self.files_from_git(&["diff", "--name-only", "--diff-filter=ACM", "HEAD"])
    }

    fn git_staged_files(&mut self) -> Result<Vec<PathBuf>> {
        debug!("Getting staged files according to git");
        self.maybe_git_stash()?;
        self.files_from_git(&["diff", "--cached", "--name-only", "--diff-filter=ACM"])
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
            exec::run(
                "git",
                &["stash", "--keep-index"],
                &HashMap::new(),
                &[0],
                // If there is a post-checkout hook, git will show any output
                // it prints to stdout on stderr instead.
                Some(&[KEEP_INDEX_RE.clone()]),
                Some(&git_root),
            )?;
            self.stashed = true;
        }

        Ok(())
    }

    fn git_modified_since(&mut self, since: &str) -> Result<Vec<PathBuf>> {
        let since_dot = format!("{since:}...");
        self.files_from_git(&["diff", "--name-only", "--diff-filter=ACM", &since_dot])
    }

    fn walkdir_files(&self, root: &Path) -> Result<Vec<PathBuf>> {
        let mut exclude_globs = ignore::overrides::OverrideBuilder::new(root);
        for d in vcs::DIRS {
            exclude_globs.add(&format!("!{d}/**/*"))?;
        }

        let mut files: Vec<PathBuf> = vec![];
        for result in ignore::WalkBuilder::new(root)
            .hidden(false)
            .overrides(exclude_globs.build()?)
            .build()
        {
            match result {
                Ok(ent) => {
                    if ent.path().is_dir() {
                        continue;
                    }
                    files.push(ent.into_path());
                }
                Err(e) => return Err(e.into()),
            };
        }

        let excluder = self.excluder()?;
        Ok(self
            .paths_relative_to_project_root(&self.project_root, files)?
            .into_iter()
            .filter(|f| !excluder.path_matches(f, false))
            .collect::<Vec<_>>())
    }

    fn files_from_git(&mut self, args: &[&str]) -> Result<Vec<PathBuf>> {
        let git_root = self.git_root()?;
        let result = exec::run(
            "git",
            args,
            &HashMap::new(),
            &[0],
            None,
            Some(&self.project_root),
        )?;
        let excluder = self.excluder()?;

        match result.stdout {
            Some(s) => Ok(
                // In the common case where the git repo root and project root
                // are the same, this isn't necessary, because git will give
                // us paths relative to the project root. But if the precious
                // root _isn't_ the git root, we need to get the path relative
                // to the project root, not the repo root.
                self.paths_relative_to_project_root(
                    &git_root,
                    s.lines()
                        .filter_map(|rel| {
                            let pb = PathBuf::from(rel);
                            if excluder.path_matches(&pb, false) {
                                return None;
                            }

                            let mut f = git_root.clone();
                            f.push(&pb);
                            if !f.exists() {
                                debug!(
                                    "The staged file at {rel:} was deleted so it will be ignored.",
                                );
                                return None;
                            }
                            Some(f)
                        })
                        .collect(),
                )?,
            ),
            None => Ok(vec![]),
        }
    }

    fn excluder(&self) -> Result<Matcher> {
        MatcherBuilder::new(&self.project_root)
            .with(&self.exclude_globs)?
            .with(vcs::DIRS)?
            .build()
    }

    // We want to make all files relative. This lets us consistently produce
    // path names starting at the root dir (without "./"). The given root is
    // the _current_ root for the relative file, which can be the cwd or the
    // git root instead of the project root.
    fn paths_relative_to_project_root(
        &self,
        // This is the root to which the given paths are relative. This might
        // be the project root or it might be the git root, which are not
        // guaranteed to be the same thing.
        path_root: &Path,
        paths: Vec<PathBuf>,
    ) -> Result<Vec<PathBuf>> {
        let mut relative: Vec<PathBuf> = vec![];
        for mut f in paths {
            if !f.is_absolute() {
                f = path_root.join(f);
            }

            relative.push(self.path_relative_to_project_root(&f)?);
        }

        Ok(relative)
    }

    fn path_relative_to_project_root(&self, path: &Path) -> Result<PathBuf> {
        // If the directory given is just "." then the first clean() removes
        // that and we then strip the prefix, leaving an empty string. The
        // second clean turns that back into ".".
        Ok(fs::canonicalize(path)?
            .clean()
            .strip_prefix(&self.project_root)
            .map_err(|_| FinderError::PrefixNotFound {
                path: path.to_path_buf(),
                prefix: self.project_root.clone(),
            })?
            .to_path_buf()
            .clean())
    }
}

impl Drop for Finder {
    fn drop(&mut self) {
        if !self.stashed {
            return;
        }

        let res = exec::run(
            "git",
            &["stash", "pop"],
            &HashMap::new(),
            &[0],
            None,
            Some(&self.project_root),
        );

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
    use itertools::Itertools;
    use precious_testhelper as testhelper;
    use pretty_assertions::assert_eq;
    use serial_test::parallel;
    use std::fs;

    fn new_finder(mode: Mode, root: PathBuf) -> Result<Finder> {
        new_finder_with_excludes(mode, root.clone(), root, vec![])
    }

    fn new_finder_with_cwd(mode: Mode, root: PathBuf, cwd: PathBuf) -> Result<Finder> {
        new_finder_with_excludes(mode, root, cwd, vec![])
    }

    fn new_finder_with_excludes(
        mode: Mode,
        root: PathBuf,
        cwd: PathBuf,
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

    #[test]
    #[parallel]
    fn all_mode() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;

        let mut finder = new_finder(Mode::All, helper.precious_root())?;
        assert_eq!(finder.files(vec![])?, Some(helper.all_files()));
        Ok(())
    }

    #[test]
    #[parallel]
    fn all_mode_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut cwd = helper.precious_root();
        cwd.push("src");

        let mut finder = new_finder_with_cwd(Mode::All, helper.precious_root(), cwd)?;
        assert_eq!(finder.files(vec![])?, Some(helper.all_files()));
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

        let mut finder = new_finder(Mode::All, helper.precious_root())?;
        assert_eq!(finder.files(vec![])?, Some(expect));
        Ok(())
    }

    #[test]
    #[parallel]
    fn all_mode_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(PathBuf::from("vendor/foo/bar.txt"), "new content")?;
        let mut finder = new_finder_with_excludes(
            Mode::All,
            helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(vec![])?, Some(helper.all_files()));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_empty() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut finder = new_finder(Mode::GitModified, helper.precious_root())?;
        let res = finder.files(vec![]);
        assert!(res.is_ok());
        assert!(res.unwrap().is_none());
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_with_changes() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;
        let mut finder = new_finder(Mode::GitModified, helper.precious_root())?;
        assert_eq!(finder.files(vec![])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_with_changes_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;
        let mut cwd = helper.precious_root();
        cwd.push("src");
        let mut finder = new_finder_with_cwd(Mode::GitModified, helper.precious_root(), cwd)?;
        assert_eq!(finder.files(vec![])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_with_changes_all_excluded() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;

        let mut finder = new_finder_with_excludes(
            Mode::GitModified,
            helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(vec![])?, None);
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;
        helper.commit_all()?;

        let modified = helper.modify_files()?;
        helper.write_file(PathBuf::from("vendor/foo/bar.txt"), "new content")?;
        let mut finder = new_finder_with_excludes(
            Mode::GitModified,
            helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(vec![])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_with_excluded_files_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;
        helper.commit_all()?;

        let modified = helper.modify_files()?;
        helper.write_file(PathBuf::from("vendor/foo/bar.txt"), "new content")?;
        let mut cwd = helper.precious_root();
        cwd.push("src");
        let mut finder = new_finder_with_excludes(
            Mode::GitModified,
            helper.precious_root(),
            cwd,
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(vec![])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_when_repo_root_ne_precious_root() -> Result<()> {
        let helper = testhelper::TestHelper::new()?
            .with_precious_root_in_subdir("subdir")
            .with_git_repo()?;
        let modified = helper.modify_files()?;
        let mut project_root = helper.git_root();
        project_root.push("subdir");
        let mut finder = new_finder(Mode::GitModified, project_root)?;
        assert_eq!(finder.files(vec![])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_modified_mode_includes_staged() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;
        helper.stage_some(&[&modified[0]])?;
        let mut finder = new_finder(Mode::GitModified, helper.precious_root())?;
        assert_eq!(finder.files(vec![])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_empty() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut finder = new_finder(Mode::GitStaged, helper.precious_root())?;
        let res = finder.files(vec![]);
        assert!(res.is_ok());
        assert!(res.unwrap().is_none());
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_with_changes() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;

        {
            let mut finder = new_finder(Mode::GitStaged, helper.precious_root())?;
            let res = finder.files(vec![]);
            assert!(res.is_ok());
            assert!(res.unwrap().is_none());
        }

        {
            let mut finder = new_finder(Mode::GitStaged, helper.precious_root())?;
            helper.stage_all()?;
            assert_eq!(finder.files(vec![])?, Some(modified));
        }
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_with_changes_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;

        let mut cwd = helper.precious_root();
        cwd.push("src");

        {
            let mut finder =
                new_finder_with_cwd(Mode::GitStaged, helper.precious_root(), cwd.clone())?;
            let res = finder.files(vec![]);
            assert!(res.is_ok());
            assert!(res.unwrap().is_none());
        }

        {
            let mut finder = new_finder_with_cwd(Mode::GitStaged, helper.precious_root(), cwd)?;
            helper.stage_all()?;
            assert_eq!(finder.files(vec![])?, Some(modified));
        }
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_with_changes_all_excluded() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;

        let mut finder = new_finder_with_excludes(
            Mode::GitStaged,
            helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(vec![])?, None);
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;
        helper.write_file(PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;
        let mut finder = new_finder_with_excludes(
            Mode::GitStaged,
            helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(vec![])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_with_excluded_files_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;
        helper.write_file(PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;
        let mut cwd = helper.precious_root();
        cwd.push("src");
        let mut finder = new_finder_with_excludes(
            Mode::GitStaged,
            helper.precious_root(),
            cwd,
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(finder.files(vec![])?, Some(modified));
        Ok(())
    }

    #[test]
    #[parallel]
    fn git_staged_mode_with_stash_stashes_unindexed() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;
        helper.stage_all()?;
        let unstaged = "tests/data/bar.txt";
        helper.write_file(PathBuf::from(unstaged), "new content")?;

        #[cfg(not(target_os = "windows"))]
        set_up_post_checkout_hook(&helper)?;

        {
            let mut finder = new_finder(Mode::GitStagedWithStash, helper.precious_root())?;
            assert_eq!(finder.files(vec![])?, Some(modified));
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

        let file = Path::new("merge-conflict-here");
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
            finder.files(vec![])?,
            Some(vec![PathBuf::from("merge-conflict-here")]),
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
        helper.delete_file(modified.remove(0))?;

        let mut finder = new_finder(Mode::GitStaged, helper.precious_root())?;
        assert_eq!(finder.files(vec![])?, Some(modified));
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
        assert_eq!(finder.files(vec![])?, None);

        let modified = helper.modify_files()?;
        helper.commit_all()?;

        let mut finder = new_finder(
            Mode::GitDiffFrom("master".to_string()),
            helper.precious_root(),
        )?;
        assert_eq!(finder.files(vec![])?, Some(modified));
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
            .collect::<Vec<PathBuf>>();
        assert_eq!(finder.files(vec![PathBuf::from("tests")])?, Some(expect));
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
            .collect::<Vec<PathBuf>>();
        assert_eq!(finder.files(vec![PathBuf::from(".")])?, Some(expect));
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
            .map(PathBuf::from)
            .collect::<Vec<PathBuf>>();
        assert_eq!(
            finder.files(vec![PathBuf::from("main.rs"), PathBuf::from("module.rs")])?,
            Some(expect),
        );
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_dir_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        let mut finder = new_finder_with_excludes(
            Mode::FromCli,
            helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(
            finder.files(vec![PathBuf::from(".")])?,
            Some(helper.all_files()),
        );
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_dir_with_excluded_files_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
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
        .map(PathBuf::from)
        .collect();
        assert_eq!(finder.files(vec![PathBuf::from(".")])?, Some(expect));
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_files_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        let mut finder = new_finder_with_excludes(
            Mode::FromCli,
            helper.precious_root(),
            helper.precious_root(),
            vec!["vendor/**/*".to_string()],
        )?;
        let expect = vec![helper.all_files().pop().unwrap()];
        let cli_paths = vec![
            helper.all_files().pop().unwrap(),
            PathBuf::from("vendor/foo/bar.txt"),
        ];
        assert_eq!(finder.files(cli_paths)?, Some(expect));
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_files_with_excluded_files_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(PathBuf::from("src/main.rs"), "initial content")?;
        let mut cwd = helper.precious_root();
        cwd.push("src");
        let mut finder = new_finder_with_excludes(
            Mode::FromCli,
            helper.precious_root(),
            cwd,
            vec!["src/main.rs".to_string()],
        )?;
        let expect = ["src/module.rs"].iter().map(PathBuf::from).collect();
        let cli_paths = ["main.rs", "module.rs"].iter().map(PathBuf::from).collect();
        assert_eq!(finder.files(cli_paths)?, Some(expect));
        Ok(())
    }

    #[test]
    #[parallel]
    fn cli_mode_given_files_with_nonexistent_path() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut finder = new_finder(Mode::FromCli, helper.precious_root())?;
        let cli_paths = vec![
            helper.all_files()[0].clone(),
            PathBuf::from("does/not/exist"),
        ];
        let res = finder.files(cli_paths);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(
            err.downcast_ref(),
            Some(&FinderError::NonExistentPathOnCli {
                path: PathBuf::from("does/not/exist")
            })
        );
        Ok(())
    }
}
