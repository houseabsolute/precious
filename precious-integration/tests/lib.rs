use anyhow::Result;
use precious_command as command;
use precious_testhelper::TestHelper;
use serial_test::serial;
use std::{collections::HashMap, env, path::PathBuf};

const CONFIG: &str = r#"
exclude = [
  "target",
]

[commands.rustfmt]
type    = "both"
include = "**/*.rs"
cmd     = [ "rustfmt", "--edition", "2021" ]
lint_flags = "--check"
ok_exit_codes = 0
lint_failure_exit_codes = 1
"#;

const GOOD_RUST: &str = r#"
fn good_func() {
    let a = 1 + 2;
    println!("a = {}", a);
}
"#;

#[test]
#[serial]
fn all() -> Result<()> {
    let helper = do_test_setup()?;

    let precious = precious_path()?;
    let env = HashMap::new();
    command::run_command(
        &precious,
        &["lint", "--all"],
        &env,
        &[0],
        false,
        Some(&helper.root()),
    )?;
    command::run_command(
        &precious,
        &["tidy", "--all"],
        &env,
        &[0],
        false,
        Some(&helper.root()),
    )?;

    Ok(())
}

#[test]
#[serial]
fn git() -> Result<()> {
    let helper = do_test_setup()?;
    helper.modify_files()?;

    let precious = precious_path()?;
    let env = HashMap::new();
    command::run_command(
        &precious,
        &["lint", "--git"],
        &env,
        &[0],
        false,
        Some(&helper.root()),
    )?;
    command::run_command(
        &precious,
        &["tidy", "--git"],
        &env,
        &[0],
        false,
        Some(&helper.root()),
    )?;

    Ok(())
}

#[test]
#[serial]
fn staged() -> Result<()> {
    let helper = do_test_setup()?;
    helper.modify_files()?;
    helper.stage_all()?;

    let precious = precious_path()?;
    let env = HashMap::new();
    command::run_command(
        &precious,
        &["lint", "--staged"],
        &env,
        &[0],
        false,
        Some(&helper.root()),
    )?;
    command::run_command(
        &precious,
        &["tidy", "--staged"],
        &env,
        &[0],
        false,
        Some(&helper.root()),
    )?;

    Ok(())
}

#[test]
#[serial]
fn cli_paths() -> Result<()> {
    let helper = do_test_setup()?;
    let files = helper.modify_files()?;

    let precious = precious_path()?;
    let env = HashMap::new();
    let mut args = vec!["lint"];
    args.append(&mut files.iter().map(|p| p.to_str().unwrap()).collect());
    command::run_command(&precious, &args, &env, &[0], false, Some(&helper.root()))?;

    let mut args = vec!["tidy"];
    args.append(&mut files.iter().map(|p| p.to_str().unwrap()).collect());
    command::run_command(&precious, &args, &env, &[0], false, Some(&helper.root()))?;

    Ok(())
}

#[test]
#[serial]
fn all_in_subdir() -> Result<()> {
    let helper = do_test_setup()?;

    let precious = precious_path()?;
    let env = HashMap::new();

    let mut cwd = helper.root();
    cwd.push("src");

    command::run_command(&precious, &["lint", "--all"], &env, &[0], false, Some(&cwd))?;
    command::run_command(&precious, &["tidy", "--all"], &env, &[0], false, Some(&cwd))?;

    Ok(())
}

#[test]
#[serial]
fn git_in_subdir() -> Result<()> {
    let helper = do_test_setup()?;
    helper.modify_files()?;

    let precious = precious_path()?;
    let env = HashMap::new();

    let mut cwd = helper.root();
    cwd.push("src");

    command::run_command(&precious, &["lint", "--git"], &env, &[0], false, Some(&cwd))?;
    command::run_command(&precious, &["tidy", "--git"], &env, &[0], false, Some(&cwd))?;

    Ok(())
}

#[test]
#[serial]
fn staged_in_subdir() -> Result<()> {
    let helper = do_test_setup()?;
    helper.modify_files()?;
    helper.stage_all()?;

    let precious = precious_path()?;
    let env = HashMap::new();

    let mut cwd = helper.root();
    cwd.push("src");

    command::run_command(
        &precious,
        &["lint", "--staged"],
        &env,
        &[0],
        false,
        Some(&cwd),
    )?;
    command::run_command(
        &precious,
        &["tidy", "--staged"],
        &env,
        &[0],
        false,
        Some(&cwd),
    )?;

    Ok(())
}

#[test]
#[serial]
fn cli_paths_in_subdir() -> Result<()> {
    let helper = do_test_setup()?;
    helper.modify_files()?;

    let precious = precious_path()?;
    let env = HashMap::new();

    let mut cwd = helper.root();
    cwd.push("src");

    command::run_command(
        &precious,
        &["lint", "module.rs", "../README.md", "../tests/data/foo.txt"],
        &env,
        &[0],
        false,
        Some(&cwd),
    )?;
    command::run_command(
        &precious,
        &["tidy", "module.rs", "../README.md", "../tests/data/foo.txt"],
        &env,
        &[0],
        false,
        Some(&cwd),
    )?;

    Ok(())
}

fn precious_path() -> Result<String> {
    let mut precious = env::current_dir()?;
    precious.push("..");
    precious.push("target");
    precious.push("debug");
    precious.push("precious");
    Ok(precious.to_string_lossy().to_string())
}

fn do_test_setup() -> Result<TestHelper> {
    let env = HashMap::new();
    command::run_command(
        "cargo",
        &["build", "--package", "precious"],
        &env,
        &[0],
        true,
        Some(&PathBuf::from("..")),
    )?;

    let helper = TestHelper::new()?
        .with_git_repo()?
        .with_config_file("precious.toml", CONFIG)?;
    helper.write_file("src/good.rs", GOOD_RUST.trim_start())?;

    Ok(helper)
}
