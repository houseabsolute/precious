use crate::shared::{compile_precious, precious_path};
use anyhow::Result;
use precious_helpers::exec::{Exec, Output};
use pushd::Pushd;
use regex::Regex;
use serial_test::serial;
#[cfg(target_family = "unix")]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs::{self, File},
    path::Path,
};
use tempfile::TempDir;

#[test]
#[serial]
fn init_go() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;
    let output = init_with_components(&["go"], None)?;

    assert_eq!(output.exit_code, 0);
    assert!(output.stderr.is_none());

    assert_file_exists("precious.toml")?;
    assert_file_contains("precious.toml", &["golangci-lint", "check-go-mod.sh"])?;
    assert_file_exists(".golangci.yml")?;
    assert_file_contains(
        ".golangci.yml",
        &["gofumpt", "govet", "check-type-assertions"],
    )?;
    assert_file_exists("dev/bin/check-go-mod.sh")?;
    #[cfg(target_family = "unix")]
    assert_file_is_executable("dev/bin/check-go-mod.sh")?;

    let stdout = output.stdout.unwrap();
    assert!(stdout.contains("dev/bin/check-go-mod.sh"));
    assert!(stdout.contains("https://golangci-lint.run"));

    Ok(())
}

#[test]
#[serial]
fn init_rust() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;
    let output = init_with_components(&["rust"], None)?;

    assert_eq!(output.exit_code, 0);
    assert!(output.stderr.is_none());

    assert_file_exists("precious.toml")?;
    assert_file_contains("precious.toml", &["clippy", "rustfmt"])?;

    let stdout = output.stdout.unwrap();
    assert!(stdout.contains("clippy"));

    Ok(())
}

#[test]
#[serial]
fn init_perl() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;
    let output = init_with_components(&["perl"], None)?;

    assert_eq!(output.exit_code, 0);
    assert!(output.stderr.is_none());

    assert_file_exists("precious.toml")?;
    assert_file_contains("precious.toml", &["perlcritic", "perlimports", "perltidy"])?;

    let stdout = output.stdout.unwrap();
    assert!(stdout.contains("App-perlimports"));

    Ok(())
}

#[test]
#[serial]
fn init_python() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;
    let output = init_with_components(&["python"], None)?;

    assert_eq!(output.exit_code, 0);
    assert!(output.stderr.is_none());

    assert_file_exists("precious.toml")?;
    assert_file_contains("precious.toml", &["ruff-check", "ruff-format", "mypy"])?;

    let stdout = output.stdout.unwrap();
    assert!(stdout.contains("astral.sh/ruff"));
    assert!(stdout.contains("mypy.readthedocs.io"));

    Ok(())
}

#[test]
#[serial]
fn init_auto_detects_python() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;

    File::create("main.py")?;

    let output = init_with_auto()?;

    assert_eq!(output.exit_code, 0);
    assert_file_exists("precious.toml")?;
    assert_file_contains("precious.toml", &["ruff-check", "ruff-format", "mypy"])?;

    Ok(())
}

#[test]
#[serial]
fn init_typescript() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;
    let output = init_with_components(&["typescript"], None)?;

    assert_eq!(output.exit_code, 0);
    assert!(output.stderr.is_none());

    assert_file_exists("precious.toml")?;
    assert_file_contains("precious.toml", &["eslint", "prettier-typescript"])?;

    let stdout = output.stdout.unwrap();
    assert!(stdout.contains("eslint.org"));
    assert!(stdout.contains("prettier.io"));

    Ok(())
}

#[test]
#[serial]
fn init_auto_detects_typescript() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;

    File::create("index.ts")?;

    let output = init_with_auto()?;

    assert_eq!(output.exit_code, 0);
    assert_file_exists("precious.toml")?;
    assert_file_contains("precious.toml", &["eslint", "prettier-typescript"])?;

    Ok(())
}

#[test]
#[serial]
fn init_ruby() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;
    let output = init_with_components(&["ruby"], None)?;

    assert_eq!(output.exit_code, 0);
    assert!(output.stderr.is_none());

    assert_file_exists("precious.toml")?;
    assert_file_contains("precious.toml", &["rubocop"])?;

    let stdout = output.stdout.unwrap();
    assert!(stdout.contains("rubocop.org"));

    Ok(())
}

#[test]
#[serial]
fn init_auto_detects_ruby() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;

    File::create("app.rb")?;

    let output = init_with_auto()?;

    assert_eq!(output.exit_code, 0);
    assert_file_exists("precious.toml")?;
    assert_file_contains("precious.toml", &["rubocop"])?;

    Ok(())
}

#[test]
#[serial]
fn init_does_not_overwrite_existing_file() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;

    File::create("precious.toml")?;
    let output = init_with_components(&["rust"], None)?;

    assert_eq!(output.exit_code, 42);
    assert!(output.stderr.is_some());
    assert!(output
        .stderr
        .unwrap()
        .contains("A file already exists at the given path: precious.toml"));

    Ok(())
}

#[test]
#[serial]
fn init_does_not_overwrite_existing_file_with_nonstandard_name() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;

    File::create("my-precious.toml")?;
    let output = init_with_components(&["rust"], Some("my-precious.toml"))?;

    assert_eq!(output.exit_code, 42);
    assert!(output.stderr.is_some());
    assert!(output
        .stderr
        .unwrap()
        .contains("A file already exists at the given path: my-precious.toml"));

    Ok(())
}

#[test]
#[serial]
fn init_auto() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;

    for path in ["src/foo.rs", "README.md", ".github/workflows/ci.yml"]
        .iter()
        .map(Path::new)
    {
        fs::create_dir_all(path.parent().unwrap())?;
        File::create(path)?;
    }

    let output = init_with_auto()?;

    assert_eq!(output.exit_code, 0);
    assert_file_exists("precious.toml")?;
    assert_file_contains("precious.toml", &["clippy", "prettier"])?;

    let stdout = output.stdout.unwrap();
    assert!(stdout.contains("clippy"));
    assert!(stdout.contains("prettier"));

    Ok(())
}

#[cfg(unix)]
#[test]
#[serial]
fn init_auto_fails_on_non_utf8_filename() -> Result<()> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;

    // A real on-disk file whose name is not valid UTF-8. `config init --auto`
    // walks the cwd to detect components and must fail-fast here.
    let bad = OsStr::from_bytes(b"data\xff.bin");
    File::create(bad)?;

    let stderr = run_precious_expecting_failure(&["config", "init", "--auto"])?;
    assert!(
        stderr.contains("non-UTF-8 path from filesystem walk"),
        "expected FilesystemWalk diagnostic, got stderr:\n{stderr}",
    );
    assert!(
        stderr.contains(r"data\xff.bin"),
        "expected raw-byte escape in stderr, got:\n{stderr}",
    );
    Ok(())
}

#[cfg(unix)]
#[test]
#[serial]
fn init_fails_on_non_utf8_cwd() -> Result<()> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    compile_precious()?;
    let td = tempfile::Builder::new()
        .prefix("precious-integration-")
        .tempdir()?;

    // Create a subdirectory with a non-UTF-8 name and chdir into it. Any
    // precious subcommand, including `config init`, must reject this cwd.
    let bad_dir = td.path().join(OsStr::from_bytes(b"sub\xff"));
    fs::create_dir(&bad_dir)?;
    let _pd = Pushd::new(&bad_dir)?;

    let stderr = run_precious_expecting_failure(&["config", "init", "--component", "go"])?;
    assert!(
        stderr.contains("non-UTF-8 path from current working directory"),
        "expected Cwd diagnostic, got stderr:\n{stderr}",
    );
    assert!(
        stderr.contains(r"sub\xff"),
        "expected raw-byte escape in stderr, got:\n{stderr}",
    );
    Ok(())
}

#[cfg(unix)]
fn run_precious_expecting_failure(args: &[&str]) -> Result<String> {
    use std::process::Command;
    let precious = precious_path()?;
    let out = Command::new(&precious).args(args).output()?;
    assert_ne!(
        out.status.code(),
        Some(0),
        "expected non-zero exit; stderr was:\n{}",
        String::from_utf8_lossy(&out.stderr),
    );
    Ok(String::from_utf8_lossy(&out.stderr).into_owned())
}

fn chdir_to_tempdir() -> Result<(TempDir, Pushd)> {
    let td = tempfile::Builder::new()
        .prefix("precious-integration-")
        .tempdir()?;
    let pd = Pushd::new(td.path())?;
    Ok((td, pd))
}

fn init_with_components(components: &[&str], init_path: Option<&str>) -> Result<Output> {
    let precious = precious_path()?;
    let mut args = vec!["config", "init"];
    for c in components {
        args.push("--component");
        args.push(c);
    }
    if let Some(p) = init_path {
        args.push("--path");
        args.push(p);
    }

    Exec::builder()
        .exe(&precious)
        .args(args)
        .ok_exit_codes(&[0, 42])
        .ignore_stderr(vec![Regex::new(".*")?])
        .build()
        .run()
}

fn init_with_auto() -> Result<Output> {
    let precious = precious_path()?;

    Exec::builder()
        .exe(&precious)
        .args(vec!["config", "init", "--auto"])
        .ok_exit_codes(&[0, 42])
        .build()
        .run()
}

fn assert_file_exists(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    assert!(path.exists(), "file {:?} does not exist", path);
    Ok(())
}

fn assert_file_contains(path: impl AsRef<Path>, contains: &[&str]) -> Result<()> {
    let path = path.as_ref();
    let contents = std::fs::read_to_string(path)?;
    for c in contains {
        assert!(
            contents.contains(c),
            "file {:?} does not contain {:?}:\n{contents}",
            path,
            c,
        );
    }
    Ok(())
}

#[cfg(target_family = "unix")]
fn assert_file_is_executable(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let perms = path.metadata()?.permissions();
    assert!(
        perms.mode() & 0o111 != 0,
        "file {:?} is not executable",
        path,
    );
    Ok(())
}
