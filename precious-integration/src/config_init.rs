use crate::shared::{compile_precious, precious_path};
use anyhow::Result;
use precious_helpers::exec::{self, ExecOutput};
use pushd::Pushd;
use serial_test::serial;
#[cfg(target_family = "unix")]
use std::os::unix::fs::PermissionsExt;
use std::{collections::HashMap, path::Path};
use tempfile::TempDir;

#[test]
#[serial]
fn init_go() -> Result<()> {
    let (_td, _pd, output) = init_with_components(&["go"])?;

    assert_file_exists("precious.toml")?;
    assert_file_contains("precious.toml", &["golangci-lint", "check-go-mod.sh"])?;
    assert_file_exists("dev/bin/check-go-mod.sh")?;
    #[cfg(target_family = "unix")]
    assert_file_is_executable("dev/bin/check-go-mod.sh")?;

    let stdout = output.stdout.unwrap();
    assert!(stdout.contains("dev/bin/check-go-mod.sh"));
    assert!(stdout.contains("https://golangci-lint.run"));
    assert!(output.stderr.is_none());

    Ok(())
}

#[test]
#[serial]
fn init_rust() -> Result<()> {
    let (_td, _pd, output) = init_with_components(&["rust"])?;

    assert_file_exists("precious.toml")?;
    assert_file_contains("precious.toml", &["clippy", "rustfmt"])?;

    let stdout = output.stdout.unwrap();
    assert!(stdout.contains("clippy"));
    assert!(output.stderr.is_none());

    Ok(())
}

#[test]
#[serial]
fn init_perl() -> Result<()> {
    let (_td, _pd, output) = init_with_components(&["perl"])?;

    assert_file_exists("precious.toml")?;
    assert_file_contains("precious.toml", &["perlcritic", "perlimports", "perltidy"])?;

    let stdout = output.stdout.unwrap();
    assert!(stdout.contains("App-perlimports"));
    assert!(output.stderr.is_none());

    Ok(())
}

fn init_with_components(components: &[&str]) -> Result<(TempDir, Pushd, ExecOutput)> {
    compile_precious()?;

    let td = tempfile::Builder::new()
        .prefix("precious-integration-")
        .tempdir()?;
    let pd = Pushd::new(td.path())?;

    let precious = precious_path()?;
    let env = HashMap::new();
    let mut args = vec!["config", "init"];
    for c in components {
        args.push("--component");
        args.push(c);
    }
    let output = exec::run(&precious, &args, &env, &[0], None, None)?;
    Ok((td, pd, output))
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
            "file {:?} does not contain {:?}",
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
