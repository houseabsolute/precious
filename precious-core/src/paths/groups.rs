use crate::{
    paths::{matcher, mode::Mode},
    vcs,
};
use anyhow::Result;
use clean_path::Clean;
use itertools::Itertools;
use log::{debug, error};
use once_cell::sync::Lazy;
use precious_exec as exec;
use regex::Regex;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Debug)]
pub struct GroupMaker {
    mode: Mode,
    project_root: PathBuf,
    cwd: PathBuf,
    exclude_globs: Vec<String>,
    stashed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Group {
    pub dir: PathBuf,
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum GroupMakerError {
    #[error("You cannot pass an explicit list of files when looking for {mode:}")]
    GotPathsFromCliWithWrongMode { mode: Mode },

    #[error("Found some paths when looking for {mode:} but they were all excluded")]
    AllPathsWereExcluded { mode: Mode },

    #[error("Found a path on the Cli which does not exist: {:}", path.display())]
    NonExistentPathOnCli { path: PathBuf },

    #[error("Could not determine the repo root by running \"git rev-parse --show-toplevel\"")]
    CouldNotDetermineRepoRoot,

    #[error("The path \"{}\" does not contain \"{}\" as a prefix", path.display(), prefix.display())]
    PrefixNotFound { path: PathBuf, prefix: PathBuf },
}

static KEEP_INDEX_RE: Lazy<Regex> = Lazy::new(|| Regex::new(".*").unwrap());

impl GroupMaker {
    pub fn new(
        mode: Mode,
        project_root: PathBuf,
        cwd: PathBuf,
        exclude_globs: Vec<String>,
    ) -> Result<GroupMaker> {
        Ok(GroupMaker {
            mode,
            project_root,
            cwd,
            exclude_globs,
            stashed: false,
        })
    }

    pub fn groups(&mut self, cli_paths: Vec<PathBuf>) -> Result<Option<Vec<Group>>> {
        match self.mode {
            Mode::FromCli => (),
            _ => {
                if !cli_paths.is_empty() {
                    return Err(
                        GroupMakerError::GotPathsFromCliWithWrongMode { mode: self.mode }.into(),
                    );
                }
            }
        };

        let files = match self.mode {
            Mode::All => self.all_files()?,
            Mode::FromCli => self.files_from_cli(cli_paths)?,
            Mode::GitModified => self.git_modified_files()?,
            Mode::GitStaged | Mode::GitStagedWithStash => self.git_staged_files()?,
        };

        if files.is_none() {
            return Ok(None);
        }

        self.maybe_git_stash()?;
        self.files_to_groups(files.unwrap())
    }

    fn maybe_git_stash(&mut self) -> Result<()> {
        if self.mode != Mode::GitStagedWithStash {
            return Ok(());
        }

        let res = exec::run(
            "git",
            &["rev-parse", "--show-toplevel"],
            &HashMap::new(),
            &[0],
            None,
            Some(&self.project_root),
        )?;

        let stdout = res
            .stdout
            .ok_or(GroupMakerError::CouldNotDetermineRepoRoot)?;
        let repo_root = stdout.trim();
        let mut mm = PathBuf::from(repo_root);
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
                Some(&self.project_root),
            )?;
            self.stashed = true;
        }

        Ok(())
    }

    fn all_files(&self) -> Result<Option<Vec<PathBuf>>> {
        debug!("Getting all files under {}", self.project_root.display());
        self.walkdir_files(self.project_root.as_path())
    }

    fn files_from_cli(&self, cli_paths: Vec<PathBuf>) -> Result<Option<Vec<PathBuf>>> {
        debug!("Using the list of files passed from the command line");
        let excluder = self.excluder()?;

        let mut files: Vec<PathBuf> = vec![];
        for rel in self.relative_files(&self.cwd, cli_paths)? {
            let full = self.project_root.clone().join(rel.clone());
            if !full.exists() {
                return Err(GroupMakerError::NonExistentPathOnCli { path: rel }.into());
            }

            let rel_to_root = self.relative_to_project_root(&full)?;
            if excluder.path_matches(&rel_to_root) {
                continue;
            }

            if full.is_dir() {
                if let Some(mut contents) = self.walkdir_files(&full)? {
                    files.append(&mut contents);
                }
            } else {
                files.push(rel_to_root);
            }
        }
        Ok(Some(files))
    }

    fn git_modified_files(&self) -> Result<Option<Vec<PathBuf>>> {
        debug!("Getting modified files according to git");
        self.files_from_git(&["diff", "--name-only", "--diff-filter=ACM"])
    }

    fn git_staged_files(&self) -> Result<Option<Vec<PathBuf>>> {
        debug!("Getting staged files according to git");
        self.files_from_git(&["diff", "--cached", "--name-only", "--diff-filter=ACM"])
    }

    fn walkdir_files(&self, root: &Path) -> Result<Option<Vec<PathBuf>>> {
        let mut excludes = ignore::overrides::OverrideBuilder::new(root);
        for d in vcs::DIRS {
            excludes.add(&format!("!{}/**/*", d))?;
        }

        let mut files: Vec<PathBuf> = vec![];
        for result in ignore::WalkBuilder::new(root)
            .hidden(false)
            .overrides(excludes.build()?)
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
        Ok(Some(
            self.relative_files(&self.project_root, files)?
                .into_iter()
                .filter(|p| !excluder.path_matches(p))
                .collect::<Vec<_>>(),
        ))
    }

    fn files_from_git(&self, args: &[&str]) -> Result<Option<Vec<PathBuf>>> {
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
            Some(s) => Ok(Some(
                self.relative_files(
                    &self.project_root,
                    s.lines()
                        .filter_map(|rel| {
                            if excluder.path_matches(&PathBuf::from(rel)) {
                                return None;
                            }

                            let mut f = self.project_root.clone();
                            f.push(rel);
                            Some(f)
                        })
                        .collect(),
                )?,
            )),
            None => Ok(None),
        }
    }

    fn excluder(&self) -> Result<matcher::Matcher> {
        matcher::MatcherBuilder::new()
            .with(&self.exclude_globs)?
            .with(vcs::DIRS)?
            .build()
    }

    fn files_to_groups(&self, files: Vec<PathBuf>) -> Result<Option<Vec<Group>>> {
        let mut entries: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();

        for f in files {
            let dir = f.parent().unwrap().to_path_buf();
            entries
                .entry(dir)
                .and_modify(|e| e.push(f.clone()))
                .or_insert_with(|| vec![f.clone()]);
        }

        if entries.is_empty() {
            return match self.mode {
                Mode::GitModified | Mode::GitStaged | Mode::GitStagedWithStash => Ok(None),
                _ => Err(GroupMakerError::AllPathsWereExcluded { mode: self.mode }.into()),
            };
        }

        Ok(Some(
            entries
                .keys()
                .sorted()
                .map(|k| {
                    let mut files = entries.get(k).unwrap().to_vec();
                    files.sort();
                    Group {
                        dir: k.to_path_buf().clean(),
                        files,
                    }
                })
                .collect(),
        ))
    }

    // We want to make all files relative. This lets us consistently produce
    // path names starting at the root dir (without "./").
    fn relative_files(&self, root: &Path, files: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
        let mut relative: Vec<PathBuf> = vec![];

        for mut f in files {
            if !f.is_absolute() {
                f = root.to_path_buf().join(f);
            }

            // If the directory given is just "." then the first clean()
            // removes that and we then strip the prefix, leaving an empty
            // string. The second clean turns that back into ".".
            relative.push(self.relative_to_project_root(&f)?);
        }

        Ok(relative)
    }

    fn relative_to_project_root(&self, file: &Path) -> Result<PathBuf> {
        Ok(file
            .clean()
            .strip_prefix(&self.project_root)
            .map_err(|_| GroupMakerError::PrefixNotFound {
                path: file.to_path_buf(),
                prefix: self.project_root.clone(),
            })?
            .to_path_buf()
            .clean())
    }
}

impl Drop for GroupMaker {
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
    use precious_testhelper as testhelper;
    use pretty_assertions::assert_eq;
    use std::fs;

    fn new_group_maker(mode: Mode, root: PathBuf) -> Result<GroupMaker> {
        new_group_maker_with_excludes(mode, root.clone(), root, vec![])
    }

    fn new_group_maker_with_cwd(mode: Mode, root: PathBuf, cwd: PathBuf) -> Result<GroupMaker> {
        new_group_maker_with_excludes(mode, root, cwd, vec![])
    }

    fn new_group_maker_with_excludes(
        mode: Mode,
        root: PathBuf,
        cwd: PathBuf,
        exclude: Vec<String>,
    ) -> Result<GroupMaker> {
        GroupMaker::new(mode, root, cwd, exclude)
    }

    #[cfg(not(target_os = "windows"))]
    fn set_up_post_checkout_hook(helper: &testhelper::TestHelper) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let hook = r#"
            #!/bin/sh
            echo "post checkout hook output"
        "#;

        let mut file_path = helper.root();
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
    fn files_to_paths() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;

        let bp = new_group_maker(Mode::All, helper.root())?;
        let paths = bp.files_to_groups(helper.all_files())?.unwrap();
        assert_eq!(paths.len(), 4, "got three paths entries");
        assert_eq!(
            paths[0],
            Group {
                dir: PathBuf::from("."),
                files: ["README.md", "can_ignore.x", "merge-conflict-file"]
                    .iter()
                    .map(PathBuf::from)
                    .collect(),
            }
        );
        assert_eq!(
            paths[1],
            Group {
                dir: PathBuf::from("src"),
                files: [
                    "src/bar.rs",
                    "src/can_ignore.rs",
                    "src/main.rs",
                    "src/module.rs",
                ]
                .iter()
                .map(PathBuf::from)
                .collect(),
            }
        );
        assert_eq!(
            paths[2],
            Group {
                dir: PathBuf::from("src/sub"),
                files: ["src/sub/mod.rs",].iter().map(PathBuf::from).collect(),
            }
        );
        assert_eq!(
            paths[3],
            Group {
                dir: PathBuf::from("tests/data"),
                files: [
                    "tests/data/bar.txt",
                    "tests/data/foo.txt",
                    "tests/data/generated.txt",
                ]
                .iter()
                .map(PathBuf::from)
                .collect(),
            }
        );
        Ok(())
    }

    #[test]
    fn all_mode() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut bp = new_group_maker(Mode::All, helper.root())?;
        assert_eq!(bp.groups(vec![])?, bp.files_to_groups(helper.all_files())?);
        Ok(())
    }

    #[test]
    fn all_mode_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut cwd = helper.root();
        cwd.push("src");
        let mut bp = new_group_maker_with_cwd(Mode::All, helper.root(), cwd)?;
        assert_eq!(bp.groups(vec![])?, bp.files_to_groups(helper.all_files())?);
        Ok(())
    }

    #[test]
    fn all_mode_with_gitignore() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut gitignores = helper.add_gitignore_files()?;
        let mut expect = testhelper::TestHelper::non_ignored_files();
        expect.append(&mut gitignores);

        let mut bp = new_group_maker(Mode::All, helper.root())?;
        assert_eq!(bp.groups(vec![])?, bp.files_to_groups(expect)?);
        Ok(())
    }

    #[test]
    fn all_mode_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(&PathBuf::from("vendor/foo/bar.txt"), "new content")?;
        let mut bp = new_group_maker_with_excludes(
            Mode::All,
            helper.root(),
            helper.root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(bp.groups(vec![])?, bp.files_to_groups(helper.all_files())?);
        Ok(())
    }

    #[test]
    fn git_modified_mode_empty() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut bp = new_group_maker(Mode::GitModified, helper.root())?;
        let res = bp.groups(vec![]);
        assert!(res.is_ok());
        assert!(res.unwrap().is_none());
        Ok(())
    }

    #[test]
    fn git_modified_mode_with_changes() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;
        let mut bp = new_group_maker(Mode::GitModified, helper.root())?;
        let expect = bp.files_to_groups(
            modified
                .iter()
                .sorted_by(|a, b| a.cmp(b))
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_eq!(bp.groups(vec![])?, expect);
        Ok(())
    }

    #[test]
    fn git_modified_mode_with_changes_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;
        let mut cwd = helper.root();
        cwd.push("src");
        let mut bp = new_group_maker_with_cwd(Mode::GitModified, helper.root(), cwd)?;
        let expect = bp.files_to_groups(
            modified
                .iter()
                .sorted_by(|a, b| a.cmp(b))
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_eq!(bp.groups(vec![])?, expect);
        Ok(())
    }

    #[test]
    fn git_modified_mode_with_changes_all_excluded() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(&PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;

        let mut bp = new_group_maker_with_excludes(
            Mode::GitModified,
            helper.root(),
            helper.root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(bp.groups(vec![])?, None);
        Ok(())
    }

    #[test]
    fn git_modified_mode_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(&PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;
        helper.commit_all()?;

        let modified = helper.modify_files()?;
        helper.write_file(&PathBuf::from("vendor/foo/bar.txt"), "new content")?;
        let mut bp = new_group_maker_with_excludes(
            Mode::GitModified,
            helper.root(),
            helper.root(),
            vec!["vendor/**/*".to_string()],
        )?;
        let expect = bp.files_to_groups(
            modified
                .iter()
                .sorted_by(|a, b| a.cmp(b))
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_eq!(bp.groups(vec![])?, expect);
        Ok(())
    }

    #[test]
    fn git_modified_mode_with_excluded_files_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(&PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;
        helper.commit_all()?;

        let modified = helper.modify_files()?;
        helper.write_file(&PathBuf::from("vendor/foo/bar.txt"), "new content")?;
        let mut cwd = helper.root();
        cwd.push("src");
        let mut bp = new_group_maker_with_excludes(
            Mode::GitModified,
            helper.root(),
            cwd,
            vec!["vendor/**/*".to_string()],
        )?;
        let expect = bp.files_to_groups(
            modified
                .iter()
                .sorted_by(|a, b| a.cmp(b))
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_eq!(bp.groups(vec![])?, expect);
        Ok(())
    }

    #[test]
    fn git_staged_mode_empty() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut bp = new_group_maker(Mode::GitStaged, helper.root())?;
        let res = bp.groups(vec![]);
        assert!(res.is_ok());
        assert!(res.unwrap().is_none());
        Ok(())
    }

    #[test]
    fn git_staged_mode_with_changes() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;

        {
            let mut bp = new_group_maker(Mode::GitStaged, helper.root())?;
            let res = bp.groups(vec![]);
            assert!(res.is_ok());
            assert!(res.unwrap().is_none());
        }

        {
            let mut bp = new_group_maker(Mode::GitStaged, helper.root())?;
            helper.stage_all()?;
            let expect = bp.files_to_groups(
                modified
                    .iter()
                    .sorted_by(|a, b| a.cmp(b))
                    .map(PathBuf::from)
                    .collect::<Vec<PathBuf>>(),
            )?;
            assert_eq!(bp.groups(vec![])?, expect);
        }
        Ok(())
    }

    #[test]
    fn git_staged_mode_with_changes_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;

        let mut cwd = helper.root();
        cwd.push("src");

        {
            let mut bp = new_group_maker_with_cwd(Mode::GitStaged, helper.root(), cwd.clone())?;
            let res = bp.groups(vec![]);
            assert!(res.is_ok());
            assert!(res.unwrap().is_none());
        }

        {
            let mut bp = new_group_maker_with_cwd(Mode::GitStaged, helper.root(), cwd)?;
            helper.stage_all()?;
            let expect = bp.files_to_groups(
                modified
                    .iter()
                    .sorted_by(|a, b| a.cmp(b))
                    .map(PathBuf::from)
                    .collect::<Vec<PathBuf>>(),
            )?;
            assert_eq!(bp.groups(vec![])?, expect);
        }
        Ok(())
    }

    #[test]
    fn git_staged_mode_with_changes_all_excluded() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(&PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;

        let mut bp = new_group_maker_with_excludes(
            Mode::GitStaged,
            helper.root(),
            helper.root(),
            vec!["vendor/**/*".to_string()],
        )?;
        assert_eq!(bp.groups(vec![])?, None);
        Ok(())
    }

    #[test]
    fn git_staged_mode_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;
        helper.write_file(&PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;
        let mut bp = new_group_maker_with_excludes(
            Mode::GitStaged,
            helper.root(),
            helper.root(),
            vec!["vendor/**/*".to_string()],
        )?;
        let expect = bp.files_to_groups(
            modified
                .iter()
                .sorted_by(|a, b| a.cmp(b))
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_eq!(bp.groups(vec![])?, expect);
        Ok(())
    }

    #[test]
    fn git_staged_mode_with_excluded_files_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;
        helper.write_file(&PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        helper.stage_all()?;
        let mut cwd = helper.root();
        cwd.push("src");
        let mut bp = new_group_maker_with_excludes(
            Mode::GitStaged,
            helper.root(),
            cwd,
            vec!["vendor/**/*".to_string()],
        )?;
        let expect = bp.files_to_groups(
            modified
                .iter()
                .sorted_by(|a, b| a.cmp(b))
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_eq!(bp.groups(vec![])?, expect);
        Ok(())
    }

    #[test]
    fn git_staged_mode_with_stash_stashes_unindexed() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let modified = helper.modify_files()?;
        helper.stage_all()?;
        let unstaged = "tests/data/bar.txt";
        helper.write_file(&PathBuf::from(unstaged), "new content")?;

        #[cfg(not(target_os = "windows"))]
        set_up_post_checkout_hook(&helper)?;

        {
            let mut bp = new_group_maker(Mode::GitStagedWithStash, helper.root())?;
            let expect = bp.files_to_groups(
                modified
                    .iter()
                    .sorted_by(|a, b| a.cmp(b))
                    .map(PathBuf::from)
                    .collect::<Vec<PathBuf>>(),
            )?;
            assert_eq!(bp.groups(vec![])?, expect);
            assert_eq!(
                String::from_utf8(fs::read(helper.root().join(unstaged))?)?,
                String::from("some text"),
            );
        }
        assert_eq!(
            String::from_utf8(fs::read(helper.root().join(unstaged))?)?,
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

        let mut bp = new_group_maker(Mode::GitStaged, helper.root())?;
        let expect = bp.files_to_groups(vec![PathBuf::from("merge-conflict-here")])?;

        assert_eq!(bp.groups(vec![])?, expect);
        assert!(!bp.stashed);
        Ok(())
    }

    #[test]
    fn cli_mode() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut bp = new_group_maker(Mode::FromCli, helper.root())?;
        let expect = bp.files_to_groups(
            helper
                .all_files()
                .iter()
                .filter(|p| p.starts_with("tests/"))
                .sorted_by(|a, b| a.cmp(b))
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_eq!(bp.groups(vec![PathBuf::from("tests")])?, expect);
        Ok(())
    }

    #[test]
    fn cli_mode_given_dir_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut cwd = helper.root();
        cwd.push("src");
        let mut bp = new_group_maker_with_cwd(Mode::FromCli, helper.root(), cwd)?;
        let expect = bp.files_to_groups(
            helper
                .all_files()
                .iter()
                .filter(|p| p.starts_with("src/"))
                .sorted_by(|a, b| a.cmp(b))
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_eq!(bp.groups(vec![PathBuf::from(".")])?, expect);
        Ok(())
    }

    #[test]
    fn cli_mode_given_files_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut cwd = helper.root();
        cwd.push("src");
        let mut bp = new_group_maker_with_cwd(Mode::FromCli, helper.root(), cwd)?;
        let expect = bp.files_to_groups(
            ["src/main.rs", "src/module.rs"]
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_eq!(
            bp.groups(vec![PathBuf::from("main.rs"), PathBuf::from("module.rs")])?,
            expect,
        );
        Ok(())
    }

    #[test]
    fn cli_mode_given_dir_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(&PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        let mut bp = new_group_maker_with_excludes(
            Mode::FromCli,
            helper.root(),
            helper.root(),
            vec!["vendor/**/*".to_string()],
        )?;
        let expect = bp.files_to_groups(
            helper
                .all_files()
                .iter()
                .sorted_by(|a, b| a.cmp(b))
                .map(PathBuf::from)
                .collect::<Vec<PathBuf>>(),
        )?;
        assert_eq!(bp.groups(vec![PathBuf::from(".")])?, expect);
        Ok(())
    }

    #[test]
    fn cli_mode_given_dir_with_excluded_files_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(&PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        let mut cwd = helper.root();
        cwd.push("src");
        let mut bp = new_group_maker_with_excludes(
            Mode::FromCli,
            helper.root(),
            cwd,
            vec!["src/main.rs".to_string()],
        )?;
        let expect = bp.files_to_groups(
            [
                "src/bar.rs",
                "src/can_ignore.rs",
                "src/module.rs",
                "src/sub/mod.rs",
            ]
            .iter()
            .map(PathBuf::from)
            .collect(),
        )?;
        assert_eq!(bp.groups(vec![PathBuf::from(".")])?, expect);
        Ok(())
    }

    #[test]
    fn cli_mode_given_files_with_excluded_files() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(&PathBuf::from("vendor/foo/bar.txt"), "initial content")?;
        let mut bp = new_group_maker_with_excludes(
            Mode::FromCli,
            helper.root(),
            helper.root(),
            vec!["vendor/**/*".to_string()],
        )?;
        let expect = bp.files_to_groups(vec![helper.all_files()[0].clone()])?;
        let cli_paths = vec![
            helper.all_files()[0].clone(),
            PathBuf::from("vendor/foo/bar.txt"),
        ];
        assert_eq!(bp.groups(cli_paths)?, expect);
        Ok(())
    }

    #[test]
    fn cli_mode_given_files_with_excluded_files_in_subdir() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        helper.write_file(&PathBuf::from("src/main.rs"), "initial content")?;
        let mut cwd = helper.root();
        cwd.push("src");
        let mut bp = new_group_maker_with_excludes(
            Mode::FromCli,
            helper.root(),
            cwd,
            vec!["src/main.rs".to_string()],
        )?;
        let expect = bp.files_to_groups(["src/module.rs"].iter().map(PathBuf::from).collect())?;
        let cli_paths = ["main.rs", "module.rs"].iter().map(PathBuf::from).collect();
        assert_eq!(bp.groups(cli_paths)?, expect);
        Ok(())
    }

    #[test]
    fn cli_mode_given_files_with_nonexistent_path() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;
        let mut bp = new_group_maker(Mode::FromCli, helper.root())?;
        let cli_paths = vec![
            helper.all_files()[0].clone(),
            PathBuf::from("does/not/exist"),
        ];
        let res = bp.groups(cli_paths);
        assert!(res.is_err());
        assert_eq!(
            std::mem::discriminant(res.unwrap_err().downcast_ref().unwrap(),),
            std::mem::discriminant(&GroupMakerError::NonExistentPathOnCli {
                path: PathBuf::from("does/not/exist"),
            }),
        );
        Ok(())
    }
}
