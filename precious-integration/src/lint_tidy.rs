use crate::shared::{compile_precious, precious_path};
use anyhow::{Context, Result};
use itertools::Itertools;
use precious_helpers::exec::Exec;
use precious_testhelper::TestHelper;
use pretty_assertions::{assert_eq, assert_str_eq};
use regex::{Captures, Regex};
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
lint-flags = "--check"
ok-exit-codes = 0
lint-failure-exit-codes = 1

[commands.true]
type    = "lint"
include = "**/*.rs"
cmd     = [ "true" ]
ok-exit-codes = 0
lint-failure-exit-codes = 1

[commands.stderr]
type    = "lint"
include = "**/*.rs"
cmd     = [ "sh", "-c", "echo 'some stderr output' 1>&2" ]
ok-exit-codes = 0
lint-failure-exit-codes = 1
ignore-stderr = "some.+output"
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
    let helper = set_up_for_tests()?;

    let precious = precious_path()?;

    Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--all"])
        .ok_exit_codes(&[0])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;

    Exec::builder()
        .exe(&precious)
        .args(vec!["tidy", "--all"])
        .ok_exit_codes(&[0])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;

    Ok(())
}

#[test]
#[serial]
fn git() -> Result<()> {
    let helper = set_up_for_tests()?;
    helper.modify_files()?;

    let precious = precious_path()?;

    Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--git"])
        .ok_exit_codes(&[0])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;

    Exec::builder()
        .exe(&precious)
        .args(vec!["tidy", "--git"])
        .ok_exit_codes(&[0])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;

    Ok(())
}

#[test]
#[serial]
fn staged() -> Result<()> {
    let helper = set_up_for_tests()?;
    helper.modify_files()?;
    helper.stage_all()?;

    let precious = precious_path()?;

    Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--staged"])
        .ok_exit_codes(&[0])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;

    Exec::builder()
        .exe(&precious)
        .args(vec!["tidy", "--staged"])
        .ok_exit_codes(&[0])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;

    Ok(())
}

#[test]
#[serial]
fn cli_paths() -> Result<()> {
    let helper = set_up_for_tests()?;
    let files = helper.modify_files()?;

    let precious = precious_path()?;

    let mut args = vec!["lint"];
    args.append(&mut files.iter().map(|p| p.to_str().unwrap()).collect());
    Exec::builder()
        .exe(&precious)
        .args(args)
        .ok_exit_codes(&[0])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;

    let mut args = vec!["tidy"];
    args.append(&mut files.iter().map(|p| p.to_str().unwrap()).collect());
    Exec::builder()
        .exe(&precious)
        .args(args)
        .ok_exit_codes(&[0])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;

    Ok(())
}

#[test]
#[serial]
fn all_in_subdir() -> Result<()> {
    let helper = set_up_for_tests()?;

    let precious = precious_path()?;

    let mut cwd = helper.precious_root();
    cwd.push("src");

    Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--all"])
        .ok_exit_codes(&[0])
        .in_dir(&cwd)
        .build()
        .run()?;

    Exec::builder()
        .exe(&precious)
        .args(vec!["tidy", "--all"])
        .ok_exit_codes(&[0])
        .in_dir(&cwd)
        .build()
        .run()?;

    Ok(())
}

#[test]
#[serial]
fn git_in_subdir() -> Result<()> {
    let helper = set_up_for_tests()?;
    helper.modify_files()?;

    let precious = precious_path()?;

    let mut cwd = helper.precious_root();
    cwd.push("src");

    Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--git"])
        .ok_exit_codes(&[0])
        .in_dir(&cwd)
        .build()
        .run()?;

    Exec::builder()
        .exe(&precious)
        .args(vec!["tidy", "--git"])
        .ok_exit_codes(&[0])
        .in_dir(&cwd)
        .build()
        .run()?;

    Ok(())
}

#[test]
#[serial]
fn staged_in_subdir() -> Result<()> {
    let helper = set_up_for_tests()?;
    helper.modify_files()?;
    helper.stage_all()?;

    let precious = precious_path()?;

    let mut cwd = helper.precious_root();
    cwd.push("src");

    Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--staged"])
        .ok_exit_codes(&[0])
        .in_dir(&cwd)
        .build()
        .run()?;

    Exec::builder()
        .exe(&precious)
        .args(vec!["tidy", "--staged"])
        .ok_exit_codes(&[0])
        .in_dir(&cwd)
        .build()
        .run()?;

    Ok(())
}

#[test]
#[serial]
fn cli_paths_in_subdir() -> Result<()> {
    let helper = set_up_for_tests()?;
    helper.modify_files()?;

    let precious = precious_path()?;

    let mut cwd = helper.precious_root();
    cwd.push("src");

    Exec::builder()
        .exe(&precious)
        .args(vec![
            "lint",
            "module.rs",
            "../README.md",
            "../tests/data/foo.txt",
        ])
        .ok_exit_codes(&[0])
        .in_dir(&cwd)
        .build()
        .run()?;

    Exec::builder()
        .exe(&precious)
        .args(vec![
            "tidy",
            "module.rs",
            "../README.md",
            "../tests/data/foo.txt",
        ])
        .ok_exit_codes(&[0])
        .in_dir(&cwd)
        .build()
        .run()?;

    Ok(())
}

#[test]
#[serial]
fn one_command() -> Result<()> {
    let helper = set_up_for_tests()?;
    let content = r#"
fn foo() -> u8   {
    42
}
"#;
    helper.write_file("src/module.rs", content)?;

    let precious = precious_path()?;

    let mut cwd = helper.precious_root();
    cwd.push("src");

    // This succeeds because we're not checking with rustfmt.
    Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--command", "true", "module.rs"])
        .ok_exit_codes(&[0])
        .in_dir(&cwd)
        .build()
        .run()?;

    // This fails now that we check with rustfmt.
    Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "module.rs"])
        .ok_exit_codes(&[1])
        .in_dir(&cwd)
        .build()
        .run()?;

    Ok(())
}

#[test]
#[serial]
fn exit_codes() -> Result<()> {
    let helper = set_up_for_tests()?;

    let all_codes = Vec::from_iter(0..=255);
    let match_all_re = Regex::new(".*")?;

    let precious = precious_path()?;

    let out = Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--all"])
        .ok_exit_codes(&all_codes)
        .ignore_stderr(vec![match_all_re.clone()])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;
    assert_eq!(out.exit_code, 0);

    helper.write_file("src/good.rs", "this is not valid rust")?;

    let out = Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--all"])
        .ok_exit_codes(&all_codes)
        .ignore_stderr(vec![match_all_re.clone()])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;
    assert_eq!(out.exit_code, 1);

    let out = Exec::builder()
        .exe(&precious)
        .args(vec!["foo", "--all"])
        .ok_exit_codes(&all_codes)
        .ignore_stderr(vec![match_all_re.clone()])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;
    assert_eq!(out.exit_code, 2);

    let out = Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--foo"])
        .ok_exit_codes(&all_codes)
        .ignore_stderr(vec![match_all_re.clone()])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;
    assert_eq!(out.exit_code, 2);

    helper.write_file("precious.toml", "this is not valid config")?;
    let out = Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--all"])
        .ok_exit_codes(&all_codes)
        .ignore_stderr(vec![match_all_re.clone()])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;
    assert_eq!(out.exit_code, 42);

    let config_missing_key = r#"
[commands.rustfmt]
type    = "both"
include = "**/*.rs"
cmd     = [ "rustfmt", "--edition", "2021" ]
ok-exit-codes = 0
lint-failure-exit-codes = 1
"#;
    helper.write_file("precious.toml", config_missing_key)?;
    let out = Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--all"])
        .ok_exit_codes(&all_codes)
        .ignore_stderr(vec![match_all_re.clone()])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;
    assert_eq!(out.exit_code, 42);

    Ok(())
}

#[test]
#[serial]
fn all_invocation_options() -> Result<()> {
    let helper = set_up_for_tests()?;
    write_perl_script(&helper)?;
    create_file_tree(&helper)?;

    let docs =
        fs::read_to_string(PathBuf::from("../docs/invocation-examples.md"))?.replace("\r\n", "\n");
    let docs_re = Regex::new(
        r"(?xsm)
            ```toml\n
            \[commands\.some-linter\]\n
            (?P<config>.+?)
            ```
            \n+
            ```\n
            (?P<output>.+?)
            ```
        ",
    )?;

    let mut count = 0;
    for caps in docs_re.captures_iter(&docs) {
        let config = &caps["config"];
        match run_one_invocation_test(&helper, config, &caps["output"]) {
            Ok(..) => (),
            Err(e) => {
                eprintln!("Error from this config:\n{config}");
                return Err(e);
            }
        }
        count += 1;
    }
    const EXPECT_COUNT: u8 = 28;
    assert_eq!(count, EXPECT_COUNT, "tested {EXPECT_COUNT} examples");

    Ok(())
}

#[test]
#[serial]
fn fix_is_tidy() -> Result<()> {
    let helper = set_up_for_tests()?;

    let precious = precious_path()?;

    Exec::builder()
        .exe(&precious)
        .args(vec!["fix", "--all"])
        .ok_exit_codes(&[0])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;

    Ok(())
}

// Since precious runs the linter in parallel on different files we to force
// the execution to be serialized. On Linux we can use the flock command but
// that doesn't exist on macOS so we'll use this Perl script instead.
fn write_perl_script(helper: &TestHelper) -> Result<()> {
    let script = r#"
use strict;
use warnings;

use Cwd qw( abs_path );
use File::Spec;

my $output_dir = $ENV{PRECIOUS_INTEGRATION_TEST_OUTPUT_DIR}
    or die "The PRECIOUS_INTEGRATION_TEST_OUTPUT_DIR env var is not set";

my $test_root = $ENV{PRECIOUS_INTEGRATION_TEST_ROOT}
    or die "The PRECIOUS_INTEGRATION_TEST_ROOT env var is not set";

my $output_file = File::Spec->catfile($output_dir, "invocation.$$");
open my $output_fh, '>>', $output_file or die "Cannot open $output_file: $!";
my $cwd = abs_path('.');
print {$output_fh} <<"EOF" or die "Cannot write to $output_file: $!";
----
cwd = $cwd
some-linter @ARGV
EOF
close $output_fh or die "Cannot close $output_file: $!";
"#;

    let mut script_file = helper.precious_root();
    script_file.push("some-linter");
    fs::write(&script_file, script)?;

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

    for path in [
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
cmd = [ "perl", "$PRECIOUS_ROOT/some-linter" ]
ok-exit-codes = 0
{config}
"#
    );

    if cfg!(windows) {
        fs::write(&precious_toml, full_config.replace('\n', "\r\n"))?;
    } else {
        fs::write(&precious_toml, full_config)?;
    }

    let td = tempfile::Builder::new()
        .prefix("precious-integration-")
        .tempdir()?;
    let td_path = td.path().to_path_buf();
    let (_output_dir, _preserved_tempdir) = match env::var("PRECIOUS_TESTS_PRESERVE_TEMPDIR") {
        Ok(v) if !(v.is_empty() || v == "0") => (None, Some(td.into_path())),
        _ => (Some(td), None),
    };

    let env = HashMap::from([
        (
            String::from("PRECIOUS_INTEGRATION_TEST_OUTPUT_DIR"),
            td_path.to_string_lossy().to_string(),
        ),
        (
            String::from("PRECIOUS_INTEGRATION_TEST_ROOT"),
            helper.precious_root().to_string_lossy().to_string(),
        ),
    ]);

    Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--all"])
        .ok_exit_codes(&[0])
        .env(env)
        .in_dir(&helper.precious_root())
        .build()
        .run()?;

    let got = munge_invocation_output(td_path)?;

    let expect = expect.replace(" \\\n    ", " ");
    assert_str_eq!(got, expect, "\n{config}");

    Ok(())
}

fn munge_invocation_output(output_dir: PathBuf) -> Result<String> {
    let mut got = String::new();
    for entry in fs::read_dir(output_dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let path = entry.path();
        let mut output = fs::read_to_string(&path)
            .with_context(|| format!("Could not read file {}", path.display()))?
            .replace("\r\n", "\n");
        if cfg!(windows) {
            output = output.replace('\\', "/");
        }
        got.push_str(&output);
    }

    // println!("RAW GOT");
    // println!("{got}");
    let output_re = Regex::new(
        r"(?x)
           ----\n
           # We strip off the actual leading path, since on Windows this can
           # end up in a different form from what we expect.
           cwd\ =\ .+?[/\\]precious-testhelper-[^/\\]+?(?:[/\\](?P<cwd>.+?))?\n
           (?P<cmd>some-linter)(?:\ (?P<paths>.+?)?)\n
        ",
    )?;

    #[derive(Debug)]
    struct Invocation<'a> {
        cwd: &'a str,
        cmd: &'a str,
        paths: Option<&'a str>,
    }

    let mut invocations: Vec<Invocation> = vec![];
    for caps in output_re.captures_iter(&got) {
        invocations.push(Invocation {
            cwd: caps.name("cwd").map(|c| c.as_str()).unwrap_or(""),
            cmd: caps.name("cmd").unwrap().as_str(),
            paths: caps.name("paths").map(|p| p.as_str()),
        });
    }
    invocations.sort_by(|a, b| {
        if a.cwd != b.cwd {
            return a.cwd.cmp(b.cwd);
        }
        a.paths.unwrap_or("").cmp(b.paths.unwrap_or(""))
    });

    // This will match the portion of the path up to the temp dir in which we
    // ran `precious`. This will be replaced with "/example" so it matches the
    // docs.
    let path_re = Regex::new(r"[^ ]+?[/\\]precious-testhelper-[^/\\ ]+(?P<path>[/\\][^/\\ ]+\b)?")?;

    let mut last_cd = "";
    Ok(invocations
        .iter()
        .map(|i| {
            let mut output = String::new();
            if last_cd != i.cwd {
                output.push_str("cd /example/");
                output.push_str(i.cwd);
                output.push('\n');
            }
            last_cd = i.cwd;
            output.push_str(i.cmd);
            if let Some(paths) = i.paths {
                output.push(' ');
                output.push_str(&path_re.replace_all(paths, |caps: &Captures| {
                    format!(
                        "/example{}",
                        caps.name("path").map(|p| p.as_str()).unwrap_or(""),
                    )
                }));
            }
            output.push('\n');
            output
        })
        .join(""))
}

pub(crate) fn set_up_for_tests() -> Result<TestHelper> {
    compile_precious()?;

    let helper = TestHelper::new()?
        .with_git_repo()?
        .with_config_file("precious.toml", CONFIG)?;
    helper.write_file("src/good.rs", GOOD_RUST.trim_start())?;

    Ok(helper)
}
