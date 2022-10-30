use anyhow::{Context, Result};
use itertools::Itertools;
use precious_helpers::exec;
use precious_testhelper::TestHelper;
use pretty_assertions::assert_eq;
use regex::Regex;
use serial_test::serial;
use std::{collections::HashMap, env, fs, path::PathBuf};

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

#[test]
#[serial]
fn all_invocation_options() -> Result<()> {
    let helper = do_test_setup()?;
    write_bash_script(&helper)?;
    create_file_tree(&helper)?;

    let docs = fs::read_to_string(PathBuf::from("../docs/invocation-examples.md"))?;
    let docs_re = Regex::new(
        r#"(?xsm)
            ```toml\n
            \[commands\.some-linter\]\n
            (?P<config>.+?)
            ```
            \s+
            ```\n
            (?P<output>.+?)
            ```
        "#,
    )?;

    for caps in docs_re.captures_iter(&docs) {
        run_one_invocation_test(&helper, &caps["config"], &caps["output"])?;
    }

    Ok(())
}

fn write_bash_script(helper: &TestHelper) -> Result<()> {
    let script_contents = r#"
if [ -z "$PRECIOUS_INTEGRATION_TEST_OUTPUT_FILE" ]; then
    echo "No PRECIOUS_INTEGRATION_TEST_OUTPUT_FILE set!"
    exit 1
fi

if [ -z "$PRECIOUS_INTEGRATION_TEST_ROOT" ]; then
    echo "No PRECIOUS_INTEGRATION_TEST_ROOT set!"
    exit 1
fi

# Since precious runs the linter in parallel on different files we need to
# lock the output file.
(
    flock --exclusive --wait 2.0 42 || exit 1

    echo "----" 1>&42

    cwd=$(pwd)
    if [ "$cwd" != "$PRECIOUS_INTEGRATION_TEST_ROOT" ]; then
        echo "cd $cwd" 1>&42
    fi

    echo "some-linter $@" 1>&42

) 42>>"$PRECIOUS_INTEGRATION_TEST_OUTPUT_FILE"

exit 0
"#;
    let mut script_file = helper.precious_root();
    script_file.push("some-linter.sh");
    fs::write(&script_file, script_contents)?;

    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = script_file.metadata()?.permissions();
        perms.set_mode(0o0755);
        fs::set_permissions(&script_file, perms)?;
    }

    Ok(())
}

// example
// ├── app.go
// ├── main.go
// ├── pkg1
// │  ├── pkg1.go
// ├── pkg2
// │  ├── pkg2.go
// │  ├── pkg2_test.go
// │  └── subpkg
// │     └── subpkg.go
fn create_file_tree(helper: &TestHelper) -> Result<()> {
    let root = helper.precious_root();

    for path in &[
        "app.go",
        "main.go",
        "pkg1/pkg1.go",
        "pkg2/pkg2.go",
        "pkg2/pkg2_test.go",
        "pkg2/subpkg/subpkg.go",
    ] {
        let mut file = root.clone();
        file.push(path);
        fs::create_dir_all(file.parent().unwrap())?;
        fs::write(&file, "x")?;
    }

    Ok(())
}

fn run_one_invocation_test(helper: &TestHelper, config: &str, expect: &str) -> Result<()> {
    let mut precious_toml = helper.precious_root();
    precious_toml.push("precious.toml");
    let precious = precious_path()?;

    let full_config = format!(
        r#"
[commands.some-linter]
type = "lint"
include = "**/*.go"
cmd = [ "bash", "some-linter.sh" ]
ok_exit_codes = 0
{config}
"#
    );
    fs::write(&precious_toml, &full_config)?;

    let output_dir = tempfile::Builder::new()
        .prefix("precious-all_invocation_options-")
        .tempdir()?;
    let mut output_file = output_dir.path().to_path_buf();
    output_file.push("linter-output.txt");

    let env = HashMap::from([
        (
            String::from("PRECIOUS_INTEGRATION_TEST_OUTPUT_FILE"),
            output_file.to_string_lossy().to_string(),
        ),
        (
            String::from("PRECIOUS_INTEGRATION_TEST_ROOT"),
            helper.precious_root().to_string_lossy().to_string(),
        ),
    ]);
    let result = exec::run(
        &precious,
        &["lint", "--all"],
        &env,
        &[0],
        None,
        Some(&helper.precious_root()),
    )?;
    println!("{}", result.stderr.as_deref().unwrap_or(""));

    let got = fs::read_to_string(&output_file)
        .with_context(|| format!("Could not read file {}", output_file.display()))?;
    let output_re = Regex::new(
        r#"(?x)
           ----\n
           (?:(P<cd>cd\ .+?)\n)?
           (?P<cmd>some-linter\ (?P<paths>.+?))\n
        "#,
    )?;

    let mut output_by_paths: HashMap<String, Vec<String>> = HashMap::new();
    for caps in output_re.captures_iter(&got) {
        let mut output = vec![];
        let cd = caps
            .name("cd")
            .map(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        if !cd.is_empty() {
            output.push(cd);
        }
        output.push(caps["cmd"].to_string());
        output_by_paths.insert(caps["paths"].to_string(), output);
    }
    let mut output = vec![];
    for (_, mut v) in output_by_paths
        .into_iter()
        .sorted_by(|a, b| Ord::cmp(&a.0, &b.0))
    {
        output.append(&mut v);
    }
    assert_eq!(
        output,
        expect.trim().split('\n').collect::<Vec<_>>(),
        "{config}"
    );

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
