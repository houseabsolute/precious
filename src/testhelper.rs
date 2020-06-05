#[cfg(test)]
use crate::command;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::io::prelude::*;
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};

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
        let temp = tempdir()?;
        let root = temp.path().to_owned();
        let helper = TestHelper {
            _tempdir: temp,
            root: root,
            paths: Self::PATHS.iter().map(|p| PathBuf::from(p)).collect(),
            root_gitignore_file: PathBuf::from(".gitignore"),
            tests_data_gitignore_file: PathBuf::from("tests/data/.gitignore"),
        };
        helper.create_git_repo()?;
        Ok(helper)
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

    pub fn all_files(&self) -> Vec<PathBuf> {
        self.paths.iter().map(|p| p.clone()).collect()
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
        for p in Self::TO_MODIFY.iter().map(|p| PathBuf::from(p)) {
            self.write_file(&p, "new content")?;
            paths.push(p.clone());
        }
        Ok(paths)
    }

    pub fn write_file(&self, rel: &PathBuf, content: &str) -> Result<()> {
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
