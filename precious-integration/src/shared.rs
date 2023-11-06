use anyhow::Result;
use precious_helpers::exec;
use regex::Regex;
use std::{collections::HashMap, env, fs, path::PathBuf};

pub(crate) fn compile_precious() -> Result<()> {
    let cargo_build_re = Regex::new("Finished.+dev.+target")?;
    let env = HashMap::new();
    exec::run(
        "cargo",
        &["build", "--package", "precious"],
        &env,
        &[0],
        Some(&[cargo_build_re]),
        Some(&PathBuf::from("..")),
    )?;
    Ok(())
}

pub(crate) fn precious_path() -> Result<String> {
    let man_dir = env::var("CARGO_MANIFEST_DIR")?;
    assert_ne!(man_dir, "");

    let mut precious = PathBuf::from(man_dir);
    precious.push("..");
    precious.push("target");
    precious.push("debug");
    precious.push("precious");
    Ok(fs::canonicalize(precious)?.to_string_lossy().to_string())
}
