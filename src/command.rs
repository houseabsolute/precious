use failure::Error;
use log::debug;
#[cfg(target_family = "unix")]
use std::os::unix::prelude::*;
use std::path::PathBuf;
use std::process;

#[derive(Debug, Fail)]
pub enum CommandError {
    #[fail(
        display = "Got unexpected exit code {} from `{}`. Stderr was {}",
        code, cmd, stderr
    )]
    ExitCodeIsNotZero {
        cmd: String,
        code: i32,
        stderr: String,
    },

    #[fail(display = "Ran `{}` and it was killed by signal {}", cmd, signal)]
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
    ok_exit_codes: Vec<i32>,
    expect_stderr: bool,
    in_dir: Option<&PathBuf>,
) -> Result<CommandResult, Error> {
    let mut c = process::Command::new(cmd.clone());
    for a in args.iter() {
        c.arg(a);
    }
    if in_dir.is_some() {
        c.current_dir(in_dir.unwrap());
    }

    let cstr = command_string(&cmd, &args);
    debug!("Running command: {}", cstr);

    let output = output_from_command(c, ok_exit_codes, &cmd, &args)?;

    if !output.stderr.is_empty() && !expect_stderr {
        return Err(CommandError::UnexpectedStderr {
            cmd: command_string(&cmd, &args),
            stderr: String::from_utf8(output.stderr)?,
        })?;
    }

    let code = match output.status.code() {
        Some(code) => code,
        None => -1,
    };

    Ok(CommandResult {
        exit_code: code,
        stdout: to_option_string(output.stdout),
        stderr: to_option_string(output.stderr),
    })
}

fn output_from_command(
    mut c: process::Command,
    ok_exit_codes: Vec<i32>,
    cmd: &str,
    args: &[String],
) -> Result<process::Output, Error> {
    let output = c.output()?;
    match output.status.code() {
        Some(code) => {
            if !ok_exit_codes.contains(&code) {
                let cstr = command_string(cmd, args);
                debug!("Ran {} and got exit code of {}", cstr.clone(), code);
                return Err(CommandError::ExitCodeIsNotZero {
                    cmd: cstr,
                    code,
                    stderr: String::from_utf8(output.stderr)?,
                })?;
            }
        }
        None => {
            if !output.status.success() {
                let cstr = command_string(cmd, args);
                let signal = signal_from_status(output.status);
                debug!(
                    "Ran {} which exited because of signal {}",
                    cstr.clone(),
                    signal
                );
                return Err(CommandError::ProcessKilledBySignal { cmd: cstr, signal })?;
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

fn signal_from_status(status: process::ExitStatus) -> i32 {
    #[cfg(target_family = "unix")]
    match status.signal() {
        Some(s) => s,
        None => 0,
    }
    #[cfg(target_family = "windows")]
    0
}

#[cfg(test)]
mod tests {
    use failure::Error;
    use spectral::prelude::*;

    #[test]
    fn command_string() {
        assert_that(&super::command_string(&String::from("foo"), &vec![]))
            .named("command without args")
            .is_equal_to(String::from("foo"));

        assert_that(&super::command_string(
            &String::from("foo"),
            &vec![String::from("bar")],
        ))
        .named("command with one arg")
        .is_equal_to(String::from("foo bar"));

        assert_that(&super::command_string(
            &String::from("foo"),
            &vec![String::from("--bar"), String::from("baz")],
        ))
        .named("command with multiple args")
        .is_equal_to(String::from("foo --bar baz"));
    }

    #[test]
    fn run_command() -> Result<(), Error> {
        let res = super::run_command(
            String::from("echo"),
            vec![String::from("foo")],
            vec![0],
            false,
            None,
        )?;
        assert_that(&res.exit_code)
            .named("command exits 0")
            .is_equal_to(&0);

        let res = super::run_command(
            String::from("sh"),
            vec![String::from("-c"), String::from("exit 32")],
            vec![0],
            false,
            None,
        );
        assert_that(&res).named("command exits non-zero").is_err();

        match res {
            Ok(_) => assert!(false, "did not get an error in the returned Result"),
            Err(e) => {
                let r = e
                    .as_fail()
                    .find_root_cause()
                    .downcast_ref::<super::CommandError>();
                match r {
                    Some(c) => match c {
                        super::CommandError::ExitCodeIsNotZero {
                            code,
                            cmd: _,
                            stderr: _,
                        } => {
                            assert_that(code)
                                .named("command unexpectedly exits 32")
                                .is_equal_to(&32);
                        }
                        _ => assert!(false, "expected a CommandError::ExitCodeIsNotZero "),
                    },
                    None => assert!(false, "expected an error, not a None"),
                }
            }
        }

        Ok(())
    }
}
