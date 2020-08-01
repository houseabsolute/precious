use anyhow::Result;
use log::debug;
use std::collections::HashMap;
#[cfg(target_family = "unix")]
use std::os::unix::prelude::*;
use std::path::PathBuf;
use std::process;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("Got unexpected exit code {code:} from `{cmd:}`")]
    UnexpectedExitCode { cmd: String, code: i32 },

    #[error("Got unexpected exit code {code:} from `{cmd:}`. Stderr was {stderr:}")]
    UnexpectedExitCodeWithStderr {
        cmd: String,
        code: i32,
        stderr: String,
    },

    #[error("Ran `{cmd:}` and it was killed by signal {signal:}")]
    ProcessKilledBySignal { cmd: String, signal: i32 },

    #[error("Got unexpected stderr output from `{cmd:}`: {stderr:}")]
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
    ok_exit_codes: Vec<i32>,
    expect_stderr: bool,
    in_dir: Option<&PathBuf>,
) -> Result<CommandResult> {
    let mut c = process::Command::new(cmd.clone());
    for a in args.iter() {
        c.arg(a);
    }
    if let Some(id) = in_dir {
        c.current_dir(id);
    }

    c.envs(env);

    let cstr = command_string(&cmd, &args);
    debug!("Running command: {}", cstr);

    let output = output_from_command(c, ok_exit_codes, &cmd, &args)?;

    if !output.stderr.is_empty() && !expect_stderr {
        return Err(CommandError::UnexpectedStderr {
            cmd: command_string(&cmd, &args),
            stderr: String::from_utf8(output.stderr)?,
        }
        .into());
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
) -> Result<process::Output> {
    let output = c.output()?;
    match output.status.code() {
        Some(code) => {
            if !ok_exit_codes.contains(&code) {
                let cstr = command_string(cmd, args);
                debug!("Ran {} and got exit code of {}", cstr, code);
                if output.stderr.is_empty() {
                    return Err(CommandError::UnexpectedExitCode { cmd: cstr, code }.into());
                } else {
                    return Err(CommandError::UnexpectedExitCodeWithStderr {
                        cmd: cstr,
                        code,
                        stderr: String::from_utf8(output.stderr)?,
                    }
                    .into());
                }
            }
        }
        None => {
            if !output.status.success() {
                let cstr = command_string(cmd, args);
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
    use anyhow::Result;
    use spectral::prelude::*;
    use std::collections::HashMap;
    use std::env;

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
    fn run_command() -> Result<()> {
        let res = super::run_command(
            String::from("echo"),
            vec![String::from("foo")],
            &HashMap::new(),
            vec![0],
            false,
            None,
        )?;
        assert_that(&res.exit_code)
            .named("command exits 0")
            .is_equal_to(&0);

        let env_key = "PRECIOUS_ENV_TEST";
        let mut env = HashMap::new();
        env.insert(String::from(env_key), String::from("foo"));
        let res = super::run_command(
            String::from("sh"),
            vec![
                String::from("-c"),
                String::from(format!("echo ${}", env_key)),
            ],
            &env,
            vec![0],
            false,
            None,
        )?;
        assert_that(&res.exit_code)
            .named("command exits 0")
            .is_equal_to(&0);
        assert_that(&res.stdout.is_some())
            .named("command has stdout output")
            .is_true();
        assert_that(&res.stdout.unwrap())
            .named(format!("{} env var was set when command was run", env_key).as_str())
            .is_equal_to(&String::from("foo\n"));

        let val = env::var(env_key);
        assert_that(&val.err().unwrap())
            .named(format!("{} env var is not set after command was run", env_key).as_str())
            .is_equal_to(&std::env::VarError::NotPresent);

        let res = super::run_command(
            String::from("sh"),
            vec![String::from("-c"), String::from("exit 32")],
            &HashMap::new(),
            vec![0],
            false,
            None,
        );
        assert_that(&res).named("command exits non-zero").is_err();

        match res {
            Ok(_) => assert!(false, "did not get an error in the returned Result"),
            Err(e) => {
                let r = e.downcast_ref::<super::CommandError>();
                match r {
                    Some(c) => match c {
                        super::CommandError::UnexpectedExitCode { cmd: _, code } => {
                            assert_that(code)
                                .named("command unexpectedly exits 32")
                                .is_equal_to(&32);
                        }
                        _ => assert!(false, "expected a CommandError::UnexpectedExitCode "),
                    },
                    None => assert!(false, "expected an error, not a None"),
                }
            }
        }

        Ok(())
    }
}
