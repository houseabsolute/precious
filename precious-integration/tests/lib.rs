use anyhow::Result;
use precious_helpers::exec;
use precious_testhelper::TestHelper;
use regex::Regex;
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

[commands.true]
type    = "lint"
include = "**/*.rs"
cmd     = [ "true" ]
ok_exit_codes = 0
lint_failure_exit_codes = 1

[commands.stderr]
type    = "lint"
include = "**/*.rs"
cmd     = [ "sh", "-c", "echo 'some stderr output' 1>&2" ]
ok_exit_codes = 0
lint_failure_exit_codes = 1
ignore_stderr = "some.+output"
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
    exec::run(
        &precious,
        &["lint", "--all"],
        &env,
        &[0],
        None,
        Some(&helper.precious_root()),
    )?;
    exec::run(
        &precious,
        &["tidy", "--all"],
        &env,
        &[0],
        None,
        Some(&helper.precious_root()),
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
    exec::run(
        &precious,
        &["lint", "--git"],
        &env,
        &[0],
        None,
        Some(&helper.precious_root()),
    )?;
    exec::run(
        &precious,
        &["tidy", "--git"],
        &env,
        &[0],
        None,
        Some(&helper.precious_root()),
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
    exec::run(
        &precious,
        &["lint", "--staged"],
        &env,
        &[0],
        None,
        Some(&helper.precious_root()),
    )?;
    exec::run(
        &precious,
        &["tidy", "--staged"],
        &env,
        &[0],
        None,
        Some(&helper.precious_root()),
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
    exec::run(
        &precious,
        &args,
        &env,
        &[0],
        None,
        Some(&helper.precious_root()),
    )?;

    let mut args = vec!["tidy"];
    args.append(&mut files.iter().map(|p| p.to_str().unwrap()).collect());
    exec::run(
        &precious,
        &args,
        &env,
        &[0],
        None,
        Some(&helper.precious_root()),
    )?;

    Ok(())
}

#[test]
#[serial]
fn all_in_subdir() -> Result<()> {
    let helper = do_test_setup()?;

    let precious = precious_path()?;
    let env = HashMap::new();

    let mut cwd = helper.precious_root();
    cwd.push("src");

    exec::run(&precious, &["lint", "--all"], &env, &[0], None, Some(&cwd))?;
    exec::run(&precious, &["tidy", "--all"], &env, &[0], None, Some(&cwd))?;

    Ok(())
}

#[test]
#[serial]
fn git_in_subdir() -> Result<()> {
    let helper = do_test_setup()?;
    helper.modify_files()?;

    let precious = precious_path()?;
    let env = HashMap::new();

    let mut cwd = helper.precious_root();
    cwd.push("src");

    exec::run(&precious, &["lint", "--git"], &env, &[0], None, Some(&cwd))?;
    exec::run(&precious, &["tidy", "--git"], &env, &[0], None, Some(&cwd))?;

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

    let mut cwd = helper.precious_root();
    cwd.push("src");

    exec::run(
        &precious,
        &["lint", "--staged"],
        &env,
        &[0],
        None,
        Some(&cwd),
    )?;
    exec::run(
        &precious,
        &["tidy", "--staged"],
        &env,
        &[0],
        None,
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

    let mut cwd = helper.precious_root();
    cwd.push("src");

    exec::run(
        &precious,
        &["lint", "module.rs", "../README.md", "../tests/data/foo.txt"],
        &env,
        &[0],
        None,
        Some(&cwd),
    )?;
    exec::run(
        &precious,
        &["tidy", "module.rs", "../README.md", "../tests/data/foo.txt"],
        &env,
        &[0],
        None,
        Some(&cwd),
    )?;

    Ok(())
}

#[test]
#[serial]
fn one_command() -> Result<()> {
    let helper = do_test_setup()?;
    let content = r#"
fn foo() -> u8   {
    42
}
"#;
    helper.write_file("src/module.rs", content)?;

    let precious = precious_path()?;
    let env = HashMap::new();

    let mut cwd = helper.precious_root();
    cwd.push("src");

    // This succeeds because we're not checking with rustfmt.
    exec::run(
        &precious,
        &["lint", "--command", "true", "module.rs"],
        &env,
        &[0],
        None,
        Some(&cwd),
    )?;
    // This fails now that we check with rustfmt.
    exec::run(
        &precious,
        &["lint", "module.rs"],
        &env,
        &[1],
        None,
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
    let cargo_build_re = Regex::new("Finished dev")?;
    let env = HashMap::new();
    exec::run(
        "cargo",
        &["build", "--package", "precious"],
        &env,
        &[0],
        Some(&[cargo_build_re]),
        Some(&PathBuf::from("..")),
    )?;

    let helper = TestHelper::new()?
        .with_git_repo()?
        .with_config_file("precious.toml", CONFIG)?;
    helper.write_file("src/good.rs", GOOD_RUST.trim_start())?;

    Ok(helper)
}
