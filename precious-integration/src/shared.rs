use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use precious_helpers::exec::Exec;
use regex::Regex;
use std::env;

pub(crate) fn compile_precious() -> Result<()> {
    let cargo_build_re = Regex::new("Finished.+dev.+target")?;

    Exec::builder()
        .exe("cargo")
        .args(vec!["build", "--package", "precious"])
        .ok_exit_codes(&[0])
        .in_dir(Utf8Path::new(".."))
        .ignore_stderr(vec![cargo_build_re])
        .build()
        .run()?;
    Ok(())
}

pub(crate) fn precious_path() -> Result<String> {
    let man_dir = env::var("CARGO_MANIFEST_DIR")?;
    assert_ne!(man_dir, "");

    let mut precious = Utf8PathBuf::from(man_dir);
    precious.push("..");
    precious.push("target");
    precious.push("debug");
    precious.push("precious");
    Ok(precious.canonicalize_utf8()?.into_string())
}
