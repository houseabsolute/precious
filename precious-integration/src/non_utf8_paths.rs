use crate::shared::{compile_precious, precious_path};
use anyhow::Result;
use precious_helpers::exec::Exec;
use precious_testhelper::TestHelper;
use serial_test::serial;

const CONFIG: &str = r#"
[commands.echo]
type = "lint"
include = "**/*"
cmd = ["true"]
ok-exit-codes = 0
lint-failure-exit-codes = 1
"#;

#[cfg(unix)]
#[test]
#[serial]
fn non_utf8_filename_fails_with_clear_error() -> Result<()> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    compile_precious()?;
    let helper = TestHelper::new()?
        .with_git_repo()?
        .with_config_file("precious.toml", CONFIG)?;

    let bad = OsStr::from_bytes(b"data\xff.bin");
    let mut path = helper.precious_root().into_std_path_buf();
    path.push(bad);
    std::fs::write(&path, b"contents")?;

    let precious = precious_path()?;
    let res = Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--all"])
        .ok_exit_codes(&[1])
        .in_dir(&helper.precious_root())
        .build()
        .run();

    let err = res.expect_err("expected non-zero exit");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("non-UTF-8 path from filesystem walk"),
        "expected fail-fast diagnostic, got:\n{msg}",
    );
    assert!(
        msg.contains(r"data\xff.bin"),
        "expected raw-byte escape in stderr, got:\n{msg}",
    );
    Ok(())
}

#[cfg(unix)]
#[test]
#[serial]
fn non_utf8_git_index_entry_fails_with_clear_error() -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    compile_precious()?;
    let helper = TestHelper::new()?
        .with_git_repo()?
        .with_config_file("precious.toml", CONFIG)?;

    // Hash a real blob so the index entry is structurally valid; the blob
    // contents don't matter — only the path bytes do.
    let blob_path = helper.precious_root().join("seed.txt");
    std::fs::write(&blob_path, b"seed")?;
    let blob_out = Command::new("git")
        .args(["hash-object", "-w", "seed.txt"])
        .current_dir(helper.precious_root())
        .output()?;
    assert!(blob_out.status.success(), "git hash-object failed");
    let oid = std::str::from_utf8(&blob_out.stdout)?.trim();

    // Stage a non-UTF-8 path via `git update-index --index-info`, which is the
    // one git porcelain that accepts raw bytes for the path component.
    let mut entry: Vec<u8> = format!("100644 {oid}\t").into_bytes();
    entry.extend_from_slice(b"data\xff.bin\n");
    let mut child = Command::new("git")
        .args(["update-index", "--index-info"])
        .current_dir(helper.precious_root())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    child.stdin.as_mut().unwrap().write_all(&entry)?;
    let status = child.wait()?;
    assert!(status.success(), "git update-index failed");

    let precious = precious_path()?;
    let res = Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--staged"])
        .ok_exit_codes(&[1])
        .in_dir(&helper.precious_root())
        .build()
        .run();

    let err = res.expect_err("expected non-zero exit");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("non-UTF-8 path from git diff"),
        "expected GitDiff diagnostic, got:\n{msg}",
    );
    assert!(
        msg.contains(r"data\xff.bin"),
        "expected raw-byte escape in stderr, got:\n{msg}",
    );
    Ok(())
}

#[test]
#[serial]
fn cafe_txt_handled_through_git() -> Result<()> {
    compile_precious()?;
    let helper = TestHelper::new()?
        .with_git_repo()?
        .with_config_file("precious.toml", CONFIG)?;

    let name = "café.txt";
    helper.write_file(name, "hello")?;
    helper.stage_all()?;

    let precious = precious_path()?;
    Exec::builder()
        .exe(&precious)
        .args(vec!["lint", "--staged"])
        .ok_exit_codes(&[0])
        .in_dir(&helper.precious_root())
        .build()
        .run()?;

    Ok(())
}
