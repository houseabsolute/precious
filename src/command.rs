use failure::Error;
use log::debug;
use std::collections::HashSet;
#[cfg(target_family = "unix")]
use std::os::unix::prelude::*;
use std::process::{Command, ExitStatus};

#[derive(Debug, Fail)]
pub enum CommandError {
    #[fail(display = "Got unexpected exit code {} from `{}`", code, cmd)]
    ExitCodeIsNotZero { cmd: String, code: i32 },

    #[fail(display = "Ran `{}` and it was killed by a signal: {}", cmd, signal)]
    ProcessKilledBySignal { cmd: String, signal: i32 },

    #[fail(display = "Got unexpected stderr output from `{}`: {}", cmd, stderr)]
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
    ok_exit_codes: &HashSet<i32>,
    expect_stderr: bool,
) -> Result<CommandResult, Error> {
    let mut c = Command::new(cmd.clone());
    for a in args.iter() {
        c.arg(a);
    }

    let output = c.output()?;
    match output.status.code() {
        Some(code) => {
            if !ok_exit_codes.contains(&code) {
                let cstr = command_string(cmd, args);
                debug!("Ran {} and got exit code of {}", cstr.clone(), code);
                return Err(CommandError::ExitCodeIsNotZero { cmd: cstr, code })?;
            }
        }
        None => {
            if !output.status.success() {
                let cstr = command_string(cmd, args);
                let signal = signal_from_status(output.status);
                debug!(
                    "Ran {} which exited because of {} signal",
                    cstr.clone(),
                    signal
                );
                return Err(CommandError::ProcessKilledBySignal { cmd: cstr, signal })?;
            }
        }
    }

    if !output.stderr.is_empty() && !expect_stderr {
        return Err(CommandError::UnexpectedStderr {
            cmd: command_string(cmd, args),
            stderr: String::from_utf8(output.stderr)?,
        })?;
    }

    let code = match output.status.code() {
        Some(c) => c,
        None => -1,
    };

    Ok(CommandResult {
        exit_code: code,
        stdout: to_option_string(output.stdout),
        stderr: to_option_string(output.stderr),
    })
}

fn to_option_string(v: Vec<u8>) -> Option<String> {
    if v.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&v).into_owned())
    }
}

fn command_string(mut cmd: String, args: Vec<String>) -> String {
    if !args.is_empty() {
        cmd.push(' ');
        cmd.push_str(args.join(" ").as_str());
    }
    cmd
}

fn signal_from_status(output: ExitStatus) -> i32 {
    #[cfg(target_family = "unix")]
    match output.signal() {
        Some(s) => s,
        None => 0,
    }
    #[cfg(target_family = "windows")]
    0
}
