use anyhow::{Context, Result};
use itertools::Itertools;
use log::{
    Level::Debug,
    {debug, error, log_enabled},
};
use regex::Regex;
use std::{collections::HashMap, env, fs, path::Path, process};
use thiserror::Error;
use which::which;

#[cfg(target_family = "unix")]
use std::os::unix::prelude::*;

#[derive(Debug, Error)]
pub enum Error {
    #[error(r#"Could not find "{exe:}" in your path ({path:}"#)]
    ExecutableNotInPath { exe: String, path: String },

    #[error(
        "Got unexpected exit code {code:} from `{cmd:}`.{}",
        exec_output_summary(stdout, stderr)
    )]
    UnexpectedExitCode {
        cmd: String,
        code: i32,
        stdout: String,
        stderr: String,
    },

    #[error("Ran `{cmd:}` and it was killed by signal {signal:}")]
    ProcessKilledBySignal { cmd: String, signal: i32 },

    #[error("Got unexpected stderr output from `{cmd:}` with exit code {code:}:\n{stderr:}")]
    UnexpectedStderr {
        cmd: String,
        code: i32,
        stderr: String,
    },
}

fn exec_output_summary(stdout: &str, stderr: &str) -> String {
    let mut output = if stdout.is_empty() {
        String::from("\nStdout was empty.")
    } else {
        format!("\nStdout:\n{stdout}")
    };
    if stderr.is_empty() {
        output.push_str("\nStderr was empty.");
    } else {
        output.push_str("\nStderr:\n");
        output.push_str(stderr);
    };
    output.push('\n');
    output
}

#[derive(Debug)]
pub struct ExecOutput {
    pub exit_code: i32,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

pub fn run(
    exe: &str,
    args: &[&str],
    env: &HashMap<String, String>,
    ok_exit_codes: &[i32],
    ignore_stderr: Option<&[Regex]>,
    in_dir: Option<&Path>,
) -> Result<ExecOutput> {
    if which(exe).is_err() {
        let path = match env::var("PATH") {
            Ok(p) => p,
            Err(e) => format!("<could not get PATH environment variable: {e}>"),
        };
        return Err(Error::ExecutableNotInPath {
            exe: exe.to_string(),
            path,
        }
        .into());
    }

    let mut c = process::Command::new(exe);
    for a in args.iter() {
        c.arg(a);
    }

    // We are canonicalizing this primarily for the benefit of our debugging
    // output, because otherwise we might see the current dir as just `.`,
    // which is not helpful.
    let cwd = if let Some(d) = in_dir {
        fs::canonicalize(d)?
    } else {
        fs::canonicalize(env::current_dir()?)?
    };
    c.current_dir(cwd.clone());

    c.envs(env);

    if log_enabled!(Debug) {
        debug!(
            "Running command [{}] with cwd = {}",
            exec_string(exe, args),
            cwd.display()
        );
        for k in env.keys().sorted() {
            debug!(r#"  with env: {k} = "{}""#, env.get(k).unwrap());
        }
    }

    let output = output_from_command(c, ok_exit_codes, exe, args)
        .with_context(|| format!(r#"Failed to execute command `{}`"#, exec_string(exe, args)))?;

    if log_enabled!(Debug) && !output.stdout.is_empty() {
        debug!("Stdout was:\n{}", String::from_utf8(output.stdout.clone())?);
    }

    let code = output.status.code().unwrap_or(-1);
    if !output.stderr.is_empty() {
        let stderr = String::from_utf8(output.stderr.clone())?;
        if log_enabled!(Debug) {
            debug!("Stderr was:\n{stderr}");
        }

        let ok = if let Some(ignore) = ignore_stderr {
            ignore.iter().any(|i| i.is_match(&stderr))
        } else {
            false
        };
        if !ok {
            return Err(Error::UnexpectedStderr {
                cmd: exec_string(exe, args),
                code,
                stderr,
            }
            .into());
        }
    }

    Ok(ExecOutput {
        exit_code: code,
        stdout: to_option_string(output.stdout),
        stderr: to_option_string(output.stderr),
    })
}

fn output_from_command(
    mut c: process::Command,
    ok_exit_codes: &[i32],
    exe: &str,
    args: &[&str],
) -> Result<process::Output> {
    let output = c.output()?;
    match output.status.code() {
        Some(code) => {
            let estr = exec_string(exe, args);
            debug!("Ran {} and got exit code of {}", estr, code);
            if !ok_exit_codes.contains(&code) {
                return Err(Error::UnexpectedExitCode {
                    cmd: estr,
                    code,
                    stdout: String::from_utf8(output.stdout)?,
                    stderr: String::from_utf8(output.stderr)?,
                }
                .into());
            }
        }
        None => {
            let estr = exec_string(exe, args);
            if output.status.success() {
                error!("Ran {} successfully but it had no exit code", estr);
            } else {
                let signal = signal_from_status(output.status);
                debug!("Ran {} which exited because of signal {}", estr, signal);
                return Err(Error::ProcessKilledBySignal { cmd: estr, signal }.into());
            }
        }
    }

    Ok(output)
}

fn exec_string(exe: &str, args: &[&str]) -> String {
    let mut estr = exe.to_string();
    if !args.is_empty() {
        estr.push(' ');
        estr.push_str(args.join(" ").as_str());
    }
    estr
}

fn to_option_string(v: Vec<u8>) -> Option<String> {
    if v.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&v).into_owned())
    }
}

#[cfg(target_family = "unix")]
fn signal_from_status(status: process::ExitStatus) -> i32 {
    status.signal().unwrap_or(0)
}

#[cfg(target_family = "windows")]
fn signal_from_status(_: process::ExitStatus) -> i32 {
    0
}

#[cfg(test)]
mod tests {
    use super::Error;
    use anyhow::{format_err, Result};
    use pretty_assertions::assert_eq;
    use regex::Regex;
    // Anything that does pushd must be run serially or else chaos ensues.
    use serial_test::{parallel, serial};
    use std::{
        collections::HashMap,
        env, fs,
        path::{Path, PathBuf},
    };
    use tempfile::tempdir;

    #[test]
    #[parallel]
    fn exec_string() {
        assert_eq!(
            super::exec_string("foo", &[]),
            String::from("foo"),
            "command without args",
        );
        assert_eq!(
            super::exec_string("foo", &["bar"],),
            String::from("foo bar"),
            "command with one arg"
        );
        assert_eq!(
            super::exec_string("foo", &["--bar", "baz"],),
            String::from("foo --bar baz"),
            "command with multiple args",
        );
    }

    #[test]
    #[parallel]
    fn run_exit_0() -> Result<()> {
        let res = super::run("echo", &["foo"], &HashMap::new(), &[0], None, None)?;
        assert_eq!(res.exit_code, 0, "process exits 0");

        Ok(())
    }

    #[test]
    #[parallel]
    fn run_exit_0_with_unexpected_stderr() -> Result<()> {
        let args = ["-c", "echo 'some stderr output' 1>&2"];
        let res = super::run("sh", &args, &HashMap::new(), &[0], None, None);
        assert!(res.is_err(), "run returned Err");
        match error_from_run(res)? {
            Error::UnexpectedStderr {
                cmd: _,
                code,
                stderr,
            } => {
                assert_eq!(code, 0, "process exited 0");
                assert_eq!(stderr, "some stderr output\n", "process had no stderr");
            }
            e => return Err(e.into()),
        }
        Ok(())
    }

    #[test]
    #[parallel]
    fn run_exit_0_with_matching_ignore_stderr() -> Result<()> {
        let args = ["-c", "echo 'some stderr output' 1>&2"];
        let res = super::run(
            "sh",
            &args,
            &HashMap::new(),
            &[0],
            Some(&[Regex::new("some.+output").unwrap()]),
            None,
        )?;
        assert_eq!(res.exit_code, 0, "process exits 0");
        assert!(res.stdout.is_none(), "process has no stdout output");
        assert_eq!(
            res.stderr.unwrap(),
            "some stderr output\n",
            "process has stderr output",
        );
        Ok(())
    }

    #[test]
    #[parallel]
    fn run_exit_0_with_non_matching_ignore_stderr() -> Result<()> {
        let args = ["-c", "echo 'some stderr output' 1>&2"];
        let res = super::run(
            "sh",
            &args,
            &HashMap::new(),
            &[0],
            Some(&[Regex::new("some.+output is ok").unwrap()]),
            None,
        );
        assert!(res.is_err(), "run returned Err");
        match error_from_run(res)? {
            Error::UnexpectedStderr {
                cmd: _,
                code,
                stderr,
            } => {
                assert_eq!(code, 0, "process exited 0");
                assert_eq!(stderr, "some stderr output\n", "process had no stderr");
            }
            e => return Err(e.into()),
        }
        Ok(())
    }

    #[test]
    #[parallel]
    fn run_exit_0_with_multiple_ignore_stderr() -> Result<()> {
        let args = ["-c", "echo 'some stderr output' 1>&2"];
        let res = super::run(
            "sh",
            &args,
            &HashMap::new(),
            &[0],
            Some(&[
                Regex::new("will not match").unwrap(),
                Regex::new("some.+output is ok").unwrap(),
            ]),
            None,
        );
        assert!(res.is_err(), "run returned Err");
        match error_from_run(res)? {
            Error::UnexpectedStderr {
                cmd: _,
                code,
                stderr,
            } => {
                assert_eq!(code, 0, "process exited 0");
                assert_eq!(stderr, "some stderr output\n", "process had no stderr");
            }
            e => return Err(e.into()),
        }
        Ok(())
    }

    #[test]
    #[parallel]
    fn run_wth_env() -> Result<()> {
        let env_key = "PRECIOUS_ENV_TEST";
        let mut env = HashMap::new();
        env.insert(String::from(env_key), String::from("foo"));
        let res = super::run(
            "sh",
            &["-c", &format!("echo ${env_key}")],
            &env,
            &[0],
            None,
            None,
        )?;
        assert_eq!(res.exit_code, 0, "process exits 0");
        assert!(res.stdout.is_some(), "process has stdout output");
        assert_eq!(
            res.stdout.unwrap(),
            String::from("foo\n"),
            "{} env var was set when process was run",
            env_key,
        );
        let val = env::var(env_key);
        assert_eq!(
            val.err().unwrap(),
            std::env::VarError::NotPresent,
            "{} env var is not set after process was run",
            env_key,
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn run_exit_32() -> Result<()> {
        let res = super::run("sh", &["-c", "exit 32"], &HashMap::new(), &[0], None, None);
        assert!(res.is_err(), "process exits non-zero");
        match error_from_run(res)? {
            Error::UnexpectedExitCode {
                cmd: _,
                code,
                stdout,
                stderr,
            } => {
                assert_eq!(code, 32, "process unexpectedly exits 32");
                assert_eq!(stdout, "", "process had no stdout");
                assert_eq!(stderr, "", "process had no stderr");
            }
            e => return Err(e.into()),
        }

        Ok(())
    }

    #[test]
    #[parallel]
    fn run_exit_32_with_stdout() -> Result<()> {
        let res = super::run(
            "sh",
            &["-c", r#"echo "STDOUT" && exit 32"#],
            &HashMap::new(),
            &[0],
            None,
            None,
        );
        assert!(res.is_err(), "process exits non-zero");
        let e = error_from_run(res)?;
        let expect = r#"Got unexpected exit code 32 from `sh -c echo "STDOUT" && exit 32`.
Stdout:
STDOUT

Stderr was empty.
"#;
        assert_eq!(format!("{e}"), expect, "error display output");

        match e {
            Error::UnexpectedExitCode {
                cmd: _,
                code,
                stdout,
                stderr,
            } => {
                assert_eq!(code, 32, "process unexpectedly exits 32");
                assert_eq!(stdout, "STDOUT\n", "stdout was captured");
                assert_eq!(stderr, "", "stderr was empty");
            }
            e => return Err(e.into()),
        }

        Ok(())
    }

    #[test]
    #[parallel]
    fn run_exit_32_with_stderr() -> Result<()> {
        let res = super::run(
            "sh",
            &["-c", r#"echo "STDERR" 1>&2 && exit 32"#],
            &HashMap::new(),
            &[0],
            None,
            None,
        );
        assert!(res.is_err(), "process exits non-zero");
        let e = error_from_run(res)?;
        let expect = r#"Got unexpected exit code 32 from `sh -c echo "STDERR" 1>&2 && exit 32`.
Stdout was empty.
Stderr:
STDERR

"#;
        assert_eq!(format!("{e}"), expect, "error display output");

        match e {
            Error::UnexpectedExitCode {
                cmd: _,
                code,
                stdout,
                stderr,
            } => {
                assert_eq!(
                    code, 32,
                    "process unexpectedly
            exits 32"
                );
                assert_eq!(stdout, "", "stdout was empty");
                assert_eq!(stderr, "STDERR\n", "stderr was captured");
            }
            e => return Err(e.into()),
        }

        Ok(())
    }

    #[test]
    #[parallel]
    fn run_exit_32_with_stdout_and_stderr() -> Result<()> {
        let res = super::run(
            "sh",
            &["-c", r#"echo "STDOUT" && echo "STDERR" 1>&2 && exit 32"#],
            &HashMap::new(),
            &[0],
            None,
            None,
        );
        assert!(res.is_err(), "process exits non-zero");

        let e = error_from_run(res)?;
        let expect = r#"Got unexpected exit code 32 from `sh -c echo "STDOUT" && echo "STDERR" 1>&2 && exit 32`.
Stdout:
STDOUT

Stderr:
STDERR

"#;
        assert_eq!(format!("{e}"), expect, "error display output");
        match e {
            Error::UnexpectedExitCode {
                cmd: _,
                code,
                stdout,
                stderr,
            } => {
                assert_eq!(code, 32, "process unexpectedly exits 32");
                assert_eq!(stdout, "STDOUT\n", "stdout was captured");
                assert_eq!(stderr, "STDERR\n", "stderr was captured");
            }
            e => return Err(e.into()),
        }

        Ok(())
    }

    fn error_from_run(result: Result<super::ExecOutput>) -> Result<Error> {
        match result {
            Ok(_) => Err(format_err!("did not get an error in the returned Result")),
            Err(e) => e.downcast::<super::Error>(),
        }
    }

    #[test]
    #[serial]
    fn run_in_dir() -> Result<()> {
        // On windows the path we get from `pwd` is a Windows path (C:\...)
        // but `td.path()` contains a Unix path (/tmp/...). Very confusing.
        if cfg!(windows) {
            return Ok(());
        }

        let td = tempdir()?;
        let td_path = maybe_canonicalize(td.path())?;

        let res = super::run("pwd", &[], &HashMap::new(), &[0], None, Some(&td_path))?;
        assert_eq!(res.exit_code, 0, "process exits 0");
        assert!(res.stdout.is_some(), "process produced stdout output");

        let stdout = res.stdout.unwrap();
        let stdout_trimmed = stdout.trim_end();
        assert_eq!(
            stdout_trimmed,
            td_path.to_string_lossy(),
            "process runs in another dir",
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn executable_does_not_exist() {
        let exe = "I hope this binary does not exist on any system!";
        let args = ["--arg", "42"];
        let res = super::run(exe, &args, &HashMap::new(), &[0], None, None);
        assert!(res.is_err());
        if let Err(e) = res {
            assert!(e.to_string().contains(
                r#"Could not find "I hope this binary does not exist on any system!" in your path"#,
            ));
        }
    }

    // The temp directory on macOS in GitHub Actions appears to be a symlink, but
    // canonicalizing on Windows breaks tests for some reason.
    pub fn maybe_canonicalize(path: &Path) -> Result<PathBuf> {
        if cfg!(windows) {
            return Ok(path.to_owned());
        }
        Ok(fs::canonicalize(path)?)
    }
}
