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

    #[error("Got unexpected exit code {code:} from `{cmd:}`")]
    UnexpectedExitCode { cmd: String, code: i32 },

    #[error(
        "Got unexpected exit code {code:} from `{cmd:}`. Stdout:\n{stdout:}\nStderr:\n{stderr:}"
    )]
    UnexpectedExitCodeWithStderr {
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

#[derive(Debug)]
pub struct CommandResult {
    pub exit_code: i32,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

pub fn run_command(
    cmd: String,
    args: Vec<String>,
    env: &HashMap<String, String>,
    ok_exit_codes: &[i32],
    expect_stderr: bool,
    in_dir: Option<&Path>,
) -> Result<CommandResult> {
    if which(&cmd).is_err() {
        let path = match env::var("PATH") {
            Ok(p) => p,
            Err(e) => format!("<could not get PATH environment variable: {}>", e),
        };
        return Err(CommandError::ExecutableNotInPath { exe: cmd, path }.into());
    }

    let mut c = process::Command::new(&cmd);
    for a in args.iter() {
        c.arg(a);
    }

    // We are canonicalizing this primarily for the benefit of our debugging
    // output, because otherwise we might see the current dir as just `.`,
    // which is not helpful.
    let cwd = if let Some(id) = in_dir {
        fs::canonicalize(id)?
    } else {
        fs::canonicalize(env::current_dir()?)?
    };
    c.current_dir(cwd.clone());

    c.envs(env);

    if log_enabled!(Debug) {
        let cstr = command_string(&cmd, &args);
        debug!("Running command [{}] with cwd = {}", cstr, cwd.display());
    }

    let output = output_from_command(c, ok_exit_codes, &cmd, &args).with_context(|| {
        format!(
            r#"Failed to execute command `{}`"#,
            command_string(&cmd, &args)
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
                cmd: command_string(&cmd, &args),
                stderr: String::from_utf8(output.stderr)?,
            }
            .into());
        }
    }

    let code = output.status.code().unwrap_or(-1);

    Ok(CommandResult {
        exit_code: code,
        stdout: to_option_string(output.stdout),
        stderr: to_option_string(output.stderr),
    })
}

fn output_from_command(
    mut c: process::Command,
    ok_exit_codes: &[i32],
    cmd: &str,
    args: &[String],
) -> Result<process::Output> {
    let output = c.output()?;
    match output.status.code() {
        Some(code) => {
            let cstr = command_string(cmd, args);
            debug!("Ran {} and got exit code of {}", cstr, code);
            if !ok_exit_codes.contains(&code) {
                if output.stderr.is_empty() {
                    return Err(CommandError::UnexpectedExitCode { cmd: cstr, code }.into());
                } else {
                    return Err(CommandError::UnexpectedExitCodeWithStderr {
                        cmd: cstr,
                        code,
                        stdout: String::from_utf8(output.stdout)?,
                        stderr: String::from_utf8(output.stderr)?,
                    }
                    .into());
                }
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

fn command_string(cmd: &str, args: &[String]) -> String {
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
    use crate::testhelper;
    use anyhow::Result;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use std::env;
    use tempfile::tempdir;

    #[test]
    fn command_string() {
        assert_eq!(
            super::command_string(&String::from("foo"), &[]),
            String::from("foo"),
            "command without args",
        );
        assert_eq!(
            super::command_string(&String::from("foo"), &[String::from("bar")],),
            String::from("foo bar"),
            "command with one arg"
        );
        assert_eq!(
            super::command_string(
                &String::from("foo"),
                &[String::from("--bar"), String::from("baz")],
            ),
            String::from("foo --bar baz"),
            "command with multiple args",
        );
    }

    #[test]
    fn run_command() -> Result<()> {
        let res = super::run_command(
            String::from("echo"),
            vec![String::from("foo")],
            &HashMap::new(),
            &[0],
            false,
            None,
        )?;
        assert_eq!(res.exit_code, 0, "command exits 0");

        let env_key = "PRECIOUS_ENV_TEST";
        let mut env = HashMap::new();
        env.insert(String::from(env_key), String::from("foo"));
        let res = super::run_command(
            String::from("sh"),
            vec![String::from("-c"), format!("echo ${}", env_key)],
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

        let res = super::run_command(
            String::from("sh"),
            vec![String::from("-c"), String::from("exit 32")],
            &HashMap::new(),
            &[0],
            false,
            None,
        );
        assert!(res.is_err(), "command exits non-zero");

        match res {
            Ok(_) => panic!("did not get an error in the returned Result"),
            Err(e) => {
                let r = e.downcast_ref::<super::CommandError>();
                match r {
                    Some(c) => match c {
                        super::CommandError::UnexpectedExitCode { cmd: _, code } => {
                            assert_eq!(code, &32, "command unexpectedly exits 32");
                        }
                        _ => panic!("expected a CommandError::UnexpectedExitCode "),
                    },
                    None => panic!("expected an error, not a None"),
                }
            }
        }

        Ok(())
    }

    #[test]
    fn run_command_in_dir() -> Result<()> {
        // On windows the path we get from `pwd` is a Windows path (C:\...)
        // but `td.path()` contains a Unix path (/tmp/...). Very confusing.
        if cfg!(windows) {
            return Ok(());
        }

        let td = tempdir()?;
        let td_path = testhelper::maybe_canonicalize(td.path())?;

        let res = super::run_command(
            String::from("pwd"),
            vec![],
            &HashMap::new(),
            &[0],
            false,
            Some(td_path.as_ref()),
        )?;
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
        let args = vec![String::from("--arg"), String::from("42")];
        let res = super::run_command(String::from(exe), args, &HashMap::new(), &[0], false, None);
        assert!(res.is_err());
        if let Err(e) = res {
            assert!(e.to_string().contains(
                r#"Could not find "I hope this binary does not exist on any system!" in your path"#,
            ));
        }
    }
}
