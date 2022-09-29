use anyhow::{Context, Result};
use log::{
    Level::Debug,
    {debug, error, log_enabled},
};
use std::{collections::HashMap, env, fs, path::Path, process};
use thiserror::Error;
use which::which;

#[cfg(target_family = "unix")]
use std::os::unix::prelude::*;

#[derive(Debug, Error)]
pub enum CommandError {
    #[error(r#"Could not find "{exe:}" in your path ({path:}"#)]
    ExecutableNotInPath { exe: String, path: String },

    #[error(
        "Got unexpected exit code {code:} from `{cmd:}`.{}",
        command_output_summary(stdout, stderr)
    )]
    UnexpectedExitCode {
        cmd: String,
        code: i32,
        stdout: String,
        stderr: String,
    },

    #[error("Ran `{cmd:}` and it was killed by signal {signal:}")]
    ProcessKilledBySignal { cmd: String, signal: i32 },

    #[error("Got unexpected stderr output from `{cmd:}`:\n{stderr:}")]
    UnexpectedStderr { cmd: String, stderr: String },
}

fn command_output_summary(stdout: &str, stderr: &str) -> String {
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
pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

pub fn run_command(
    cmd: &str,
    args: &[&str],
    env: &HashMap<String, String>,
    ok_exit_codes: &[i32],
    expect_stderr: bool,
    in_dir: Option<&Path>,
) -> Result<CommandOutput> {
    if which(cmd).is_err() {
        let path = match env::var("PATH") {
            Ok(p) => p,
            Err(e) => format!("<could not get PATH environment variable: {e}>"),
        };
        return Err(CommandError::ExecutableNotInPath {
            exe: cmd.to_string(),
            path,
        }
        .into());
    }

    let mut c = process::Command::new(cmd);
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
        let cstr = command_string(cmd, args);
        debug!("Running command [{}] with cwd = {}", cstr, cwd.display());
    }

    let output = output_from_command(c, ok_exit_codes, cmd, args).with_context(|| {
        format!(
            r#"Failed to execute command `{}`"#,
            command_string(cmd, args)
        )
    })?;

    if log_enabled!(Debug) && !output.stdout.is_empty() {
        debug!("Stdout was:\n{}", String::from_utf8(output.stdout.clone())?);
    }

    if !output.stderr.is_empty() {
        if log_enabled!(Debug) {
            debug!("Stderr was:\n{}", String::from_utf8(output.stderr.clone())?);
        }

        if !expect_stderr {
            return Err(CommandError::UnexpectedStderr {
                cmd: command_string(cmd, args),
                stderr: String::from_utf8(output.stderr)?,
            }
            .into());
        }
    }

    let code = output.status.code().unwrap_or(-1);

    Ok(CommandOutput {
        exit_code: code,
        stdout: to_option_string(output.stdout),
        stderr: to_option_string(output.stderr),
    })
}

fn output_from_command(
    mut c: process::Command,
    ok_exit_codes: &[i32],
    cmd: &str,
    args: &[&str],
) -> Result<process::Output> {
    let output = c.output()?;
    match output.status.code() {
        Some(code) => {
            let cstr = command_string(cmd, args);
            debug!("Ran {} and got exit code of {}", cstr, code);
            if !ok_exit_codes.contains(&code) {
                return Err(CommandError::UnexpectedExitCode {
                    cmd: cstr,
                    code,
                    stdout: String::from_utf8(output.stdout)?,
                    stderr: String::from_utf8(output.stderr)?,
                }
                .into());
            }
        }
        None => {
            let cstr = command_string(cmd, args);
            if output.status.success() {
                error!("Ran {} successfully but it had no exit code", cstr);
            } else {
                let signal = signal_from_status(output.status);
                debug!("Ran {} which exited because of signal {}", cstr, signal);
                return Err(CommandError::ProcessKilledBySignal { cmd: cstr, signal }.into());
            }
        }
    }

    Ok(output)
}

fn command_string(cmd: &str, args: &[&str]) -> String {
    let mut cstr = cmd.to_string();
    if !args.is_empty() {
        cstr.push(' ');
        cstr.push_str(args.join(" ").as_str());
    }
    cstr
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
    use super::CommandError;
    use anyhow::{format_err, Result};
    use pretty_assertions::assert_eq;
    use std::{
        collections::HashMap,
        env, fs,
        path::{Path, PathBuf},
    };
    use tempfile::tempdir;

    #[test]
    fn command_string() {
        assert_eq!(
            super::command_string("foo", &[]),
            String::from("foo"),
            "command without args",
        );
        assert_eq!(
            super::command_string("foo", &["bar"],),
            String::from("foo bar"),
            "command with one arg"
        );
        assert_eq!(
            super::command_string("foo", &["--bar", "baz"],),
            String::from("foo --bar baz"),
            "command with multiple args",
        );
    }

    #[test]
    fn run_command_exit_0() -> Result<()> {
        let res = super::run_command("echo", &["foo"], &HashMap::new(), &[0], false, None)?;
        assert_eq!(res.exit_code, 0, "command exits 0");

        Ok(())
    }

    #[test]
    fn run_command_wth_env() -> Result<()> {
        let env_key = "PRECIOUS_ENV_TEST";
        let mut env = HashMap::new();
        env.insert(String::from(env_key), String::from("foo"));
        let res = super::run_command(
            "sh",
            &["-c", &format!("echo ${env_key}")],
            &env,
            &[0],
            false,
            None,
        )?;
        assert_eq!(res.exit_code, 0, "command exits 0");
        assert!(res.stdout.is_some(), "command has stdout output");
        assert_eq!(
            res.stdout.unwrap(),
            String::from("foo\n"),
            "{} env var was set when command was run",
            env_key,
        );
        let val = env::var(env_key);
        assert_eq!(
            val.err().unwrap(),
            std::env::VarError::NotPresent,
            "{} env var is not set after command was run",
            env_key,
        );

        Ok(())
    }

    #[test]
    fn run_command_exit_32() -> Result<()> {
        let res = super::run_command("sh", &["-c", "exit 32"], &HashMap::new(), &[0], false, None);
        assert!(res.is_err(), "command exits non-zero");
        match error_from_run_command(res)? {
            CommandError::UnexpectedExitCode {
                cmd: _,
                code,
                stdout,
                stderr,
            } => {
                assert_eq!(code, 32, "command unexpectedly exits 32");
                assert_eq!(stdout, "", "command had no stdout");
                assert_eq!(stderr, "", "command had no stderr");
            }
            e => return Err(e.into()),
        }

        Ok(())
    }

    #[test]
    fn run_command_exit_32_with_stdout() -> Result<()> {
        let res = super::run_command(
            "sh",
            &["-c", r#"echo "STDOUT" && exit 32"#],
            &HashMap::new(),
            &[0],
            false,
            None,
        );
        assert!(res.is_err(), "command exits non-zero");
        let e = error_from_run_command(res)?;
        let expect = r#"Got unexpected exit code 32 from `sh -c echo "STDOUT" && exit 32`.
Stdout:
STDOUT

Stderr was empty.
"#;
        assert_eq!(format!("{e}"), expect, "error display output");

        match e {
            CommandError::UnexpectedExitCode {
                cmd: _,
                code,
                stdout,
                stderr,
            } => {
                assert_eq!(code, 32, "command unexpectedly exits 32");
                assert_eq!(stdout, "STDOUT\n", "stdout was captured");
                assert_eq!(stderr, "", "stderr was empty");
            }
            e => return Err(e.into()),
        }

        Ok(())
    }

    #[test]
    fn run_command_exit_32_with_stderr() -> Result<()> {
        let res = super::run_command(
            "sh",
            &["-c", r#"echo "STDERR" 1>&2 && exit 32"#],
            &HashMap::new(),
            &[0],
            false,
            None,
        );
        assert!(res.is_err(), "command exits non-zero");
        let e = error_from_run_command(res)?;
        let expect = r#"Got unexpected exit code 32 from `sh -c echo "STDERR" 1>&2 && exit 32`.
Stdout was empty.
Stderr:
STDERR

"#;
        assert_eq!(format!("{e}"), expect, "error display output");

        match e {
            CommandError::UnexpectedExitCode {
                cmd: _,
                code,
                stdout,
                stderr,
            } => {
                assert_eq!(
                    code, 32,
                    "command unexpectedly
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
    fn run_command_exit_32_with_stdout_and_stderr() -> Result<()> {
        let res = super::run_command(
            "sh",
            &["-c", r#"echo "STDOUT" && echo "STDERR" 1>&2 && exit 32"#],
            &HashMap::new(),
            &[0],
            false,
            None,
        );
        assert!(res.is_err(), "command exits non-zero");

        let e = error_from_run_command(res)?;
        let expect = r#"Got unexpected exit code 32 from `sh -c echo "STDOUT" && echo "STDERR" 1>&2 && exit 32`.
Stdout:
STDOUT

Stderr:
STDERR

"#;
        assert_eq!(format!("{e}"), expect, "error display output");
        match e {
            CommandError::UnexpectedExitCode {
                cmd: _,
                code,
                stdout,
                stderr,
            } => {
                assert_eq!(code, 32, "command unexpectedly exits 32");
                assert_eq!(stdout, "STDOUT\n", "stdout was captured");
                assert_eq!(stderr, "STDERR\n", "stderr was captured");
            }
            e => return Err(e.into()),
        }

        Ok(())
    }

    fn error_from_run_command(result: Result<super::CommandOutput>) -> Result<CommandError> {
        match result {
            Ok(_) => Err(format_err!("did not get an error in the returned Result")),
            Err(e) => e.downcast::<super::CommandError>(),
        }
    }

    #[test]
    fn run_command_in_dir() -> Result<()> {
        // On windows the path we get from `pwd` is a Windows path (C:\...)
        // but `td.path()` contains a Unix path (/tmp/...). Very confusing.
        if cfg!(windows) {
            return Ok(());
        }

        let td = tempdir()?;
        let td_path = maybe_canonicalize(td.path())?;

        let res = super::run_command("pwd", &[], &HashMap::new(), &[0], false, Some(&td_path))?;
        assert_eq!(res.exit_code, 0, "command exits 0");
        assert!(res.stdout.is_some(), "command produced stdout output");

        let stdout = res.stdout.unwrap();
        let stdout_trimmed = stdout.trim_end();
        assert_eq!(
            stdout_trimmed,
            td_path.to_string_lossy(),
            "command runs in another dir",
        );

        Ok(())
    }

    #[test]
    fn executable_does_not_exist() {
        let exe = "I hope this binary does not exist on any system!";
        let args = &["--arg", "42"];
        let res = super::run_command(exe, args, &HashMap::new(), &[0], false, None);
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
