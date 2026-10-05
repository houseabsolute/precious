use crate::shared::{compile_precious, precious_path};
use anyhow::Result;
use precious_helpers::exec::{Exec, Output};
use pushd::Pushd;
use regex::Regex;
use serial_test::serial;
#[cfg(target_family = "unix")]
use std::os::unix::fs::PermissionsExt;
use std::{
    env,
    fs::{self, File},
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use test_case::test_case;

#[test]
#[serial]
fn init_go() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;
    let output = init_with_components(&["go"], None)?;

    assert_eq!(output.exit_code, 0);
    assert!(output.stderr.is_none());

    assert_file_exists("precious.toml");
    assert_file_contains("precious.toml", &["golangci-lint", "check-go-mod.sh"])?;
    assert_file_exists(".golangci.yml");
    assert_file_contains(
        ".golangci.yml",
        &["gofumpt", "govet", "check-type-assertions"],
    )?;
    assert_file_exists("dev/bin/check-go-mod.sh");
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

    assert_file_exists("precious.toml");
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

    assert_file_exists("precious.toml");
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

    assert_file_exists("precious.toml");
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
    assert_file_exists("precious.toml");
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

    assert_file_exists("precious.toml");
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
    assert_file_exists("precious.toml");
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

    assert_file_exists("precious.toml");
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
    assert_file_exists("precious.toml");
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
    assert_file_exists("precious.toml");
    assert_file_contains("precious.toml", &["clippy", "prettier"])?;

    let stdout = output.stdout.unwrap();
    assert!(stdout.contains("clippy"));
    assert!(stdout.contains("prettier"));

    Ok(())
}

// The files a VCS keeps for itself are not part of the project. For example, git stores each branch
// and tag as a file under `.git`, so a branch named `release/notes.md` is a file with a `.md`
// extension.
#[test]
#[serial]
fn init_auto_ignores_files_under_vcs_dirs() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;

    for path in [
        "src/foo.rs",
        ".git/refs/heads/release/notes.md",
        ".git/refs/tags/v1.pl",
        ".hg/store/data/foo.py",
        ".svn/pristine/foo.rb",
    ]
    .iter()
    .map(Path::new)
    {
        fs::create_dir_all(path.parent().unwrap())?;
        File::create(path)?;
    }

    let output = init_with_auto()?;

    assert_eq!(output.exit_code, 0);
    assert_file_contains("precious.toml", &["clippy"])?;

    let contents = fs::read_to_string("precious.toml")?;
    for c in ["prettier-markdown", "perltidy", "ruff", "rubocop"] {
        assert!(
            !contents.contains(c),
            "precious.toml should not contain {c:?}:\n{contents}",
        );
    }

    Ok(())
}

// A config file with no commands is not valid, so every later run of `precious` would fail. It is
// better to fail here, where the error can say what went wrong.
#[test]
#[serial]
fn init_auto_fails_when_no_components_are_detected() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;
    File::create("README.txt")?;

    let precious = precious_path()?;
    let out = std::process::Command::new(&precious)
        .args(["config", "init", "--auto"])
        .output()?;
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert_eq!(out.status.code(), Some(42), "stderr was:\n{stderr}");
    assert!(
        stderr.contains("did not find any files"),
        "expected an error about finding no files, got stderr:\n{stderr}",
    );
    assert!(
        !Path::new("precious.toml").exists(),
        "no config file was written",
    );

    Ok(())
}

// The default macOS filesystems refuse to create a file with a non-UTF-8 name.
#[cfg(all(unix, not(target_os = "macos")))]
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

// The default macOS filesystems refuse to create a file with a non-UTF-8 name.
#[cfg(all(unix, not(target_os = "macos")))]
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

#[cfg(all(unix, not(target_os = "macos")))]
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

// The README tells people that the examples match what `precious config init` generates. We compare
// the parsed TOML, not the text, so that the examples can have extra comments.
#[test_case("golang", &["go", "gitignore"]; "golang")]
#[test_case("perl", &["perl", "gitignore"]; "perl")]
#[test_case("python", &["python"]; "python")]
#[test_case("ruby", &["ruby"]; "ruby")]
#[test_case("rust", &["rust", "gitignore"]; "rust")]
#[test_case("typescript", &["typescript"]; "typescript")]
#[serial]
fn example_config_matches_init_output(example: &str, components: &[&str]) -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;
    let output = init_with_components(components, None)?;
    assert_eq!(output.exit_code, 0);

    let generated: toml::Table = fs::read_to_string("precious.toml")?.parse()?;
    let example_file = examples_dir()?.join(example).join("precious.toml");
    let from_example: toml::Table = fs::read_to_string(example_file)?.parse()?;

    pretty_assertions::assert_eq!(from_example, generated);

    Ok(())
}

#[test]
#[serial]
fn example_check_go_mod_script_matches_init_output() -> Result<()> {
    compile_precious()?;
    let (_td, _pd) = chdir_to_tempdir()?;
    let output = init_with_components(&["go"], None)?;
    assert_eq!(output.exit_code, 0);

    // On Windows git may check out the example with CRLF line endings, depending on
    // `core.autocrlf`. The generated file is left alone so that this still fails if `config init`
    // ever writes CRLF into a shell script.
    let generated = fs::read_to_string("dev/bin/check-go-mod.sh")?;
    let example_file = examples_dir()?.join("golang/helpers/check-go-mod.sh");
    let from_example = fs::read_to_string(example_file)?.replace("\r\n", "\n");

    pretty_assertions::assert_eq!(from_example, generated);

    Ok(())
}

fn examples_dir() -> Result<PathBuf> {
    let mut dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    dir.push("..");
    dir.push("examples");
    Ok(dir)
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

fn assert_file_exists(path: impl AsRef<Path>) {
    let path = path.as_ref();
    assert!(path.exists(), "file {} does not exist", path.display());
}

fn assert_file_contains(path: impl AsRef<Path>, contains: &[&str]) -> Result<()> {
    let path = path.as_ref();
    let contents = std::fs::read_to_string(path)?;
    for c in contains {
        assert!(
            contents.contains(c),
            "file {} does not contain {c:?}:\n{contents}",
            path.display(),
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
        "file {} is not executable",
        path.display(),
    );
    Ok(())
}
