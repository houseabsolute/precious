use anyhow::{Context, Result};
use log::debug;
use std::{
    env,
    path::{Path, PathBuf},
};

pub struct Pushd(PathBuf);

impl Pushd {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Pushd> {
        let cwd = env::current_dir()?;
        env::set_current_dir(path.as_ref())
            .with_context(|| format!("setting current directory to {}", path.as_ref().display()))?;
        Ok(Pushd(cwd))
    }
}

impl Drop for Pushd {
    fn drop(&mut self) {
        // If the original path was a tempdir it may be gone now.
        if !self.0.exists() {
            return;
        }

        debug!("setting current dir back to {}", self.0.display());
        let res = env::set_current_dir(&self.0);
        if let Err(e) = res {
            panic!(
                "Could not return to original dir, {}: {}",
                self.0.display(),
                e,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    // Anything that does pushd must be run serially or else chaos ensues.
    use serial_test::serial;
    use tempfile::tempdir;

    #[test]
    #[serial]
    fn pushd() -> Result<()> {
        let cwd = fs::canonicalize(env::current_dir()?)?;
        {
            let td = tempdir()?;
            let _pushd = Pushd::new(td.path());
            assert_eq!(
                fs::canonicalize(env::current_dir()?)?,
                fs::canonicalize(td.path())?,
            );
        }
        assert_eq!(fs::canonicalize(env::current_dir()?)?, cwd);

        Ok(())
    }
}
