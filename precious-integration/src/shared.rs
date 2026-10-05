use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use precious_helpers::exec::Exec;
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

static PRECIOUS_PATH: OnceLock<String> = OnceLock::new();

pub(crate) fn precious_path() -> Result<String> {
    if PRECIOUS_PATH.get().is_none() {
        compile_precious()?;
    }
    PRECIOUS_PATH
        .get()
        .cloned()
        .context("the path to the precious executable was not set by the build")
}

pub(crate) fn compile_precious() -> Result<()> {
    let cargo_build_re = Regex::new("Finished.+dev.+target")?;

    let output = Exec::builder()
        .exe("cargo")
        .args(vec![
            "build",
            "--package",
            "precious",
            "--message-format",
            "json",
        ])
        .ok_exit_codes(&[0])
        .in_dir(Utf8Path::new(".."))
        .ignore_stderr(vec![cargo_build_re])
        .build()
        .run()?;

    if PRECIOUS_PATH.get().is_none() {
        let path = executable_from_build_messages(output.stdout.as_deref().unwrap_or_default())?;
        // Another test may have set this first. That is fine, since every build puts the
        // executable in the same place.
        let _ = PRECIOUS_PATH.set(path);
    }

    Ok(())
}

// We ask cargo where it put the executable instead of assuming it is in `target/debug`. The target
// directory can be moved with `CARGO_TARGET_DIR` or `build.target-dir`, and then a hard-coded path
// points at a stale executable or at nothing.
fn executable_from_build_messages(messages: &str) -> Result<String> {
    for line in messages.lines() {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if msg["reason"] != "compiler-artifact" || msg["target"]["name"] != "precious" {
            continue;
        }
        if let Some(exe) = msg["executable"].as_str() {
            return Ok(Utf8PathBuf::from(exe).canonicalize_utf8()?.into_string());
        }
    }

    anyhow::bail!("cargo build did not report the path to the precious executable")
}
