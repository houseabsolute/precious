use anyhow::{Context, Result};
use log::debug;
use once_cell::sync::{Lazy, OnceCell};
use precious_exec as exec;
use regex::Regex;
use std::{
    collections::HashMap,
    env,
    ffi::OsString,
    fs,
    io::prelude::*,
    path::{Path, PathBuf},
};
use tempfile::{tempdir, TempDir};

pub struct TestHelper {
    // While we never access this field we need to hold onto the tempdir or
    // else the directory it references will be deleted.
    _tempdir: TempDir,
    git_root: PathBuf,
    precious_root: PathBuf,
    paths: Vec<PathBuf>,
    root_gitignore_file: PathBuf,
    tests_data_gitignore_file: PathBuf,
}

static RERERE_RE: Lazy<Regex> = Lazy::new(|| Regex::new("Recorded preimage").unwrap());

impl TestHelper {
    const PATHS: &'static [&'static str] = &[
        "README.md",
        "can_ignore.x",
        "merge-conflict-file",
        "src/bar.rs",
        "src/can_ignore.rs",
        "src/main.rs",
        "src/module.rs",
        "src/sub/mod.rs",
        "tests/data/bar.txt",
        "tests/data/foo.txt",
        "tests/data/generated.txt",
    ];

    pub fn new() -> Result<Self> {
        static LOGGER_INIT: OnceCell<bool> = OnceCell::new();
        LOGGER_INIT.get_or_init(|| {
            env_logger::builder().is_test(true).init();
            true
        });

        let temp = tempdir()?;
        let root = maybe_canonicalize(temp.path())?;
        let helper = TestHelper {
            _tempdir: temp,
            git_root: root.clone(),
            precious_root: root,
            paths: Self::PATHS.iter().map(PathBuf::from).collect(),
            root_gitignore_file: PathBuf::from(".gitignore"),
            tests_data_gitignore_file: PathBuf::from("tests/data/.gitignore"),
        };
        Ok(helper)
    }

    pub fn with_precious_root_in_subdir<P: AsRef<Path>>(mut self, subdir: P) -> Self {
        self.precious_root.push(subdir);
        self
    }

    pub fn with_git_repo(self) -> Result<Self> {
        self.create_git_repo()?;
        Ok(self)
    }

    fn create_git_repo(&self) -> Result<()> {
        debug!("Creating git repo in {}", self.git_root.display());
        for p in self.paths.iter() {
            let content = if is_rust_file(&p) {
                "fn foo() {}\n"
            } else {
                "some text"
            };
            self.write_file(&p, content)?;
        }

        self.run_git(&["init", "--initial-branch", "master"])?;

        // If the tests are run in a totally clean environment they will blow
        // up if this isnt't set. This fixes
        // https://github.com/houseabsolute/precious/issues/15.
        self.run_git(&["config", "user.email", "precious@example.com"])?;
        // With this on I get line ending warnings from git on Windows if I
        // don't write out files with CRLF. Disabling this simplifies things
        // greatly.
        self.run_git(&["config", "core.autocrlf", "false"])?;

        self.stage_all()?;
        self.run_git(&["commit", "-m", "initial commit"])?;

        Ok(())
    }

    pub fn with_config_file(self, file_name: &str, content: &str) -> Result<Self> {
        if cfg!(windows) {
            self.write_file(&self.config_file(file_name), &content.replace('\n', "\r\n"))?;
        } else {
            self.write_file(&self.config_file(file_name), content)?;
        }
        Ok(self)
    }

    pub fn pushd_to_git_root(&self) -> Result<Pushd> {
        pushd_to(self.git_root.clone())
    }

    pub fn git_root(&self) -> PathBuf {
        self.git_root.clone()
    }

    pub fn precious_root(&self) -> PathBuf {
        self.precious_root.clone()
    }

    pub fn config_file(&self, file_name: &str) -> PathBuf {
        let mut path = self.precious_root.clone();
        path.push(file_name);
        path
    }

    pub fn all_files(&self) -> Vec<PathBuf> {
        self.paths.clone()
    }

    pub fn stage_all(&self) -> Result<()> {
        self.run_git(&["add", "."])
    }

    pub fn commit_all(&self) -> Result<()> {
        self.run_git(&["commit", "-a", "-m", "committed"])
    }

    const ROOT_GITIGNORE: &'static str = "
/**/bar.*
can_ignore.*
";

    const TESTS_DATA_GITIGNORE: &'static str = "
generated.*
";

    pub fn non_ignored_files() -> Vec<PathBuf> {
        Self::PATHS
            .iter()
            .filter_map(|&p| {
                if p.contains("can_ignore") || p.contains("bar.") || p.contains("generated.txt") {
                    None
                } else {
                    Some(PathBuf::from(p))
                }
            })
            .collect()
    }

    pub fn switch_to_branch(&self, branch: &str, exists: bool) -> Result<()> {
        let mut args: Vec<&str> = vec!["checkout", "--quiet"];
        if !exists {
            args.push("-b");
        }
        args.push(branch);
        exec::run(
            "git",
            &args,
            &HashMap::new(),
            &[0],
            None,
            Some(&self.git_root),
        )?;
        Ok(())
    }

    pub fn merge_master(&self, expect_fail: bool) -> Result<()> {
        let mut expect_codes = [0].to_vec();
        if expect_fail {
            expect_codes.push(1);
        }

        exec::run(
            "git",
            &["merge", "--quiet", "--no-ff", "--no-commit", "master"],
            &HashMap::new(),
            &expect_codes,
            // If rerere is enabled, it prints to stderr.
            Some(&[RERERE_RE.clone()]),
            Some(&self.git_root),
        )?;
        Ok(())
    }

    pub fn add_gitignore_files(&self) -> Result<Vec<PathBuf>> {
        self.write_file(&self.root_gitignore_file, Self::ROOT_GITIGNORE)?;
        self.write_file(&self.tests_data_gitignore_file, Self::TESTS_DATA_GITIGNORE)?;

        Ok(vec![
            self.root_gitignore_file.clone(),
            self.tests_data_gitignore_file.clone(),
        ])
    }

    fn run_git(&self, args: &[&str]) -> Result<()> {
        exec::run(
            "git",
            args,
            &HashMap::new(),
            &[0],
            None,
            Some(&self.git_root),
        )?;
        Ok(())
    }

    const TO_MODIFY: &'static [&'static str] = &["src/module.rs", "tests/data/foo.txt"];

    pub fn modify_files(&self) -> Result<Vec<PathBuf>> {
        let mut paths: Vec<PathBuf> = vec![];
        for p in Self::TO_MODIFY.iter().map(PathBuf::from) {
            let content = if is_rust_file(&p) {
                "fn bar() {}\n"
            } else {
                "new text"
            };
            self.write_file(&p, content)?;
            paths.push(p.clone());
        }
        Ok(paths)
    }

    pub fn write_file<P: AsRef<Path>>(&self, rel: P, content: &str) -> Result<()> {
        let mut full = self.precious_root.clone();
        full.push(rel.as_ref());
        let parent = full.parent().unwrap();
        debug!("creating dir at {}", parent.display());
        fs::create_dir_all(&parent)
            .with_context(|| format!("Creating dir at {}", parent.display(),))?;
        debug!("writing file at {}", full.display());
        let mut file = fs::File::create(full.clone())
            .context(format!("Creating file at {}", full.display()))?;
        file.write_all(content.as_bytes())
            .context(format!("Writing to file at {}", full.display()))?;

        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    pub fn read_file(&self, rel: &Path) -> Result<String> {
        let mut full = self.precious_root.clone();
        full.push(rel);
        let content = fs::read_to_string(full.clone())
            .context(format!("Reading file at {}", full.display()))?;

        Ok(content)
    }
}

pub fn pushd_to(to: PathBuf) -> Result<Pushd> {
    Pushd::new(to)
}

pub struct Pushd(PathBuf);

impl Pushd {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Pushd> {
        let cwd = env::current_dir()?;
        env::set_current_dir(path.as_ref())
            .with_context(|| format!("setting current directory to {}", path.as_ref().display()))?;
        Ok(Pushd(cwd))
    }
}

impl Drop for Pushd {
    fn drop(&mut self) {
        // If the original path was a tempdir it may be gone now.
        if !self.0.exists() {
            return;
        }

        let res = env::set_current_dir(&self.0);
        if let Err(e) = res {
            panic!(
                "Could not return to original dir, {}: {}",
                self.0.display(),
                e,
            );
        }
    }
}

fn is_rust_file(p: &Path) -> bool {
    if let Some(e) = p.extension() {
        let rs = OsString::from("rs");
        return *e == rs;
    }
    false
}

// The temp directory on macOS in GitHub Actions appears to be a symlink, but
// canonicalizing on Windows breaks tests for some reason.
pub fn maybe_canonicalize(path: &Path) -> Result<PathBuf> {
    if cfg!(windows) {
        return Ok(path.to_owned());
    }
    Ok(fs::canonicalize(path)?)
}
