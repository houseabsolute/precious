#[cfg(test)]
use crate::command;
use failure::Error;
use failure::ResultExt;
use std::fs;
use std::io::prelude::*;
use std::path::PathBuf;
use tempdir::TempDir;

const PATHS: &'static [&'static str] = &[
    "can_ignore.x",
    "README.md",
    "src/can_ignore.rs",
    "src/bar.rs",
    "src/main.rs",
    "src/module.rs",
    "tests/data/foo.txt",
    "tests/data/bar.txt",
    "tests/data/generated.txt",
];

const TO_MODIFY: &'static [&'static str] = &["src/module.rs", "tests/data/foo.txt"];

pub fn paths() -> &'static [&'static str] {
    PATHS
}

pub fn create_git_repo() -> Result<TempDir, Error> {
    let tempdir = TempDir::new("precious-testhelper")?;
    for p in PATHS {
        write_file(&tempdir, &p, "some content")?;
    }

    let root = tempdir.path().to_owned();
    command::run_command(
        "git".to_string(),
        ["init"].iter().map(|a| a.to_string()).collect(),
        [0].to_vec(),
        false,
        Some(&root),
    )?;
    stage_all_in(&tempdir)?;
    command::run_command(
        "git".to_string(),
        ["commit", "-m", "initial commit"]
            .iter()
            .map(|a| a.to_string())
            .collect(),
        [0].to_vec(),
        false,
        Some(&root),
    )?;

    Ok(tempdir)
}

pub fn stage_all_in(root: &TempDir) -> Result<(), Error> {
    command::run_command(
        "git".to_string(),
        ["add", "."].iter().map(|a| a.to_string()).collect(),
        [0].to_vec(),
        false,
        Some(&root.path().to_owned()),
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

pub fn add_gitignore_files(root: &TempDir) -> Result<(), Error> {
    write_file(root, ".gitignore", ROOT_GITIGNORE)?;
    write_file(root, "tests/data/.gitignore", TESTS_DATA_GITIGNORE)?;

    Ok(())
}

pub fn modify_files(root: &TempDir) -> Result<Vec<PathBuf>, Error> {
    let mut paths: Vec<PathBuf> = vec![];
    for p in TO_MODIFY {
        write_file(&root, &p, "new content")?;
        paths.push(PathBuf::from(p));
    }
    Ok(paths)
}

fn write_file(root: &TempDir, rel: &str, content: &str) -> Result<(), Error> {
    let mut full = root.path().to_owned().clone();
    full.push(rel);
    fs::create_dir_all(full.parent().unwrap()).context(format!(
        "Creating dir at {}",
        full.parent().unwrap().to_string_lossy(),
    ))?;
    let mut file = fs::File::create(full.clone())
        .context(format!("Creating file at {}", full.to_string_lossy()))?;
    file.write_all(content.as_bytes())
        .context(format!("Writing to file at {}", full.to_string_lossy()))?;

    Ok(())
}