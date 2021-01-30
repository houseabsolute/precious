#[cfg(test)]
use crate::command;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Once;
use tempfile::{tempdir, TempDir};

static START: Once = Once::new();

pub struct TestHelper {
    // While we never access this field we need to hold onto the tempdir or
    // else the directory it references will be deleted.
    _tempdir: TempDir,
    root: PathBuf,
    paths: Vec<PathBuf>,
    root_gitignore_file: PathBuf,
    tests_data_gitignore_file: PathBuf,
}

impl TestHelper {
    const PATHS: &'static [&'static str] = &[
        "README.md",
        "can_ignore.x",
        "src/can_ignore.rs",
        "src/bar.rs",
        "src/main.rs",
        "src/module.rs",
        "tests/data/foo.txt",
        "tests/data/bar.txt",
        "tests/data/generated.txt",
    ];

    pub fn new() -> Result<Self> {
        // We only want to call git config once per `cargo test` invocation,
        // or else git might give us an error saying it could not lock the
        // config file.
        START.call_once(|| {
            match env::var("CI") {
                Err(_) => (),
                Ok(_) => {
                    match command::run_command(
                        "git".to_string(),
                        ["config", "--global", "init.defaultBranch", "master"]
                            .iter()
                            .map(|a| a.to_string())
                            .collect(),
                        &HashMap::new(),
                        [0].to_vec(),
                        false,
                        None,
                    ) {
                        Ok(_) => (),
                        Err(e) => panic!(e),
                    }
                }
            };
        });

        let temp = tempdir()?;
        let root = if cfg!(windows) {
            temp.path().to_owned()
        } else {
            // The temp directory on macOS in GitHub Actions appears to be a
            // symlink, but canonicalizing on Windows breaks tests for some
            // reason.
            fs::canonicalize(temp.path().to_owned())?
        };
        let helper = TestHelper {
            _tempdir: temp,
            root,
            paths: Self::PATHS.iter().map(PathBuf::from).collect(),
            root_gitignore_file: PathBuf::from(".gitignore"),
            tests_data_gitignore_file: PathBuf::from("tests/data/.gitignore"),
        };
        Ok(helper)
    }

    pub fn with_git_repo(self) -> Result<Self> {
        self.create_git_repo()?;
        Ok(self)
    }

    pub fn with_config_file(self, content: &str) -> Result<Self> {
        if cfg!(windows) {
            self.write_file(&self.config_file(), &content.replace("\n", "\r\n"))?;
        } else {
            self.write_file(&self.config_file(), content)?;
        }
        Ok(self)
    }

    pub fn pushd_to_root(&self) -> Result<Pushd> {
        Pushd::new(self.root.clone())
    }

    fn create_git_repo(&self) -> Result<()> {
        for p in self.paths.iter() {
            self.write_file(&p, "some content")?;
        }

        command::run_command(
            "git".to_string(),
            ["init"].iter().map(|a| a.to_string()).collect(),
            &HashMap::new(),
            [0].to_vec(),
            false,
            Some(&self.root),
        )?;
        self.stage_all()?;
        command::run_command(
            "git".to_string(),
            ["commit", "-m", "initial commit"]
                .iter()
                .map(|a| a.to_string())
                .collect(),
            &HashMap::new(),
            [0].to_vec(),
            false,
            Some(&self.root),
        )?;

        Ok(())
    }

    pub fn root(&self) -> PathBuf {
        self.root.clone()
    }

    pub fn config_file(&self) -> PathBuf {
        let mut path = self.root.clone();
        path.push("precious.toml");
        path
    }

    pub fn all_files(&self) -> Vec<PathBuf> {
        self.paths.to_vec()
    }

    pub fn stage_all(&self) -> Result<()> {
        command::run_command(
            "git".to_string(),
            ["add", "."].iter().map(|a| a.to_string()).collect(),
            &HashMap::new(),
            [0].to_vec(),
            false,
            Some(&self.root()),
        )?;
        Ok(())
    }

    pub fn commit_all(&self) -> Result<()> {
        command::run_command(
            "git".to_string(),
            ["commit", "-a", "-m", "committed"]
                .iter()
                .map(|a| a.to_string())
                .collect(),
            &HashMap::new(),
            [0].to_vec(),
            false,
            Some(&self.root()),
        )?;
        Ok(())
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

    pub fn switch_to_branch(&self) -> Result<()> {
        command::run_command(
            "git".to_string(),
            ["checkout", "--quiet", "-b", "new-branch"]
                .iter()
                .map(|a| a.to_string())
                .collect(),
            &HashMap::new(),
            [0].to_vec(),
            false,
            Some(&self.root()),
        )?;
        Ok(())
    }

    pub fn reset_backwards(&self, back: i8) -> Result<()> {
        command::run_command(
            "git".to_string(),
            [
                "reset",
                "--quiet",
                "--hard",
                format!("HEAD~{}", back).as_str(),
            ]
            .iter()
            .map(|a| a.to_string())
            .collect(),
            &HashMap::new(),
            [0].to_vec(),
            false,
            Some(&self.root()),
        )?;
        Ok(())
    }

    pub fn merge_master(&self) -> Result<()> {
        command::run_command(
            "git".to_string(),
            ["merge", "--quiet", "--no-ff", "--no-commit", "master"]
                .iter()
                .map(|a| a.to_string())
                .collect(),
            &HashMap::new(),
            [0].to_vec(),
            true,
            Some(&self.root()),
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

    const TO_MODIFY: &'static [&'static str] = &["src/module.rs", "tests/data/foo.txt"];

    pub fn modify_files(&self) -> Result<Vec<PathBuf>> {
        let mut paths: Vec<PathBuf> = vec![];
        for p in Self::TO_MODIFY.iter().map(PathBuf::from) {
            self.write_file(&p, "new content")?;
            paths.push(p.clone());
        }
        Ok(paths)
    }

    pub fn write_file(&self, rel: &Path, content: &str) -> Result<()> {
        let mut full = self.root.clone();
        full.push(rel);
        fs::create_dir_all(full.parent().unwrap()).with_context(|| {
            format!(
                "Creating dir at {}",
                full.parent().unwrap().to_string_lossy(),
            )
        })?;
        let mut file = fs::File::create(full.clone())
            .context(format!("Creating file at {}", full.to_string_lossy()))?;
        file.write_all(content.as_bytes())
            .context(format!("Writing to file at {}", full.to_string_lossy()))?;

        Ok(())
    }
}

pub struct Pushd(PathBuf);

impl Pushd {
    pub fn new(path: PathBuf) -> Result<Pushd> {
        let cwd = env::current_dir()?;
        env::set_current_dir(path)?;
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
                self.0.to_string_lossy(),
                e,
            );
        }
    }
}
