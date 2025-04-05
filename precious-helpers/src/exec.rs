#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]
use crate::error::Error;
use anyhow::{Context, Result};
use bon::bon;
use itertools::Itertools;
use log::{
    Level::{Debug, Info},
    {debug, error, info, log_enabled, warn},
};
use regex::Regex;
use std::{
    collections::HashMap,
    env, fs,
    path::Path,
    process::{self, Command},
    sync::mpsc::{self, RecvTimeoutError},
    thread::{self, JoinHandle},
    time::Duration,
};
use which::which;

#[cfg(target_family = "unix")]
use std::os::unix::prelude::*;

enum ThreadMessage {
    Terminate,
}

#[derive(Debug)]
pub struct Exec<'a> {
    exe: &'a str,
    args: Vec<&'a str>,
    num_paths: usize,
    env: HashMap<String, String>,
    ok_exit_codes: &'a [i32],
    ignore_stderr: Vec<Regex>,
    in_dir: Option<&'a Path>,
    pub loggable_command: String,
}

#[derive(Debug)]
pub struct Output {
    pub exit_code: i32,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

#[bon]
impl<'a> Exec<'a> {
    #[builder]
    pub fn new(
        exe: &'a str,
        #[builder(default)] args: Vec<&'a str>,
        #[builder(default)] num_paths: usize,
        #[builder(default)] env: HashMap<String, String>,
        ok_exit_codes: &'a [i32],
        #[builder(default)] ignore_stderr: Vec<Regex>,
        in_dir: Option<&'a Path>,
    ) -> Self {
        let mut s = Self {
            exe,
            args,
            num_paths,
            env,
            ok_exit_codes,
            ignore_stderr,
            in_dir,
            loggable_command: String::new(),
        };
        // We use this a bunch of times so we'll just calculate it once. The full command is only
        // used when we return an error, so it's okay to generate that on demand.
        s.loggable_command = s.make_loggable_command();

        s
    }

    #[must_use]
    pub fn make_loggable_command(&self) -> String {
        let mut cmd = vec![self.exe];

        let mut args = self.args.iter();

        // If we don't have any paths, or if we have <= 3 arguments, we'll just include the whole
        // thing, no matter whether those args are paths or not.
        if self.num_paths == 0 || self.args.len() <= 3 {
            cmd.extend(args);
            return cmd.join(" ");
        }

        let num_non_paths = self.args.len() - self.num_paths;

        // At this point, we know we have more than 3 arguments. We will always include all the
        // arguments that are _not_ paths.
        cmd.extend(args.by_ref().take(num_non_paths));

        // If we have 3 paths or less, we'll include all of them.
        if args.len() <= 3 {
            cmd.extend(args);
            return cmd.join(" ");
        }

        // Otherwise we'll include 2 paths and then "and N more paths". We know that N will always
        // be >= 2. We never want to include "... and 1 more path", since in that case we might as
        // well have included that 1 path instead.
        cmd.extend(args.by_ref().take(2));

        let and_more = format!("... and {} more paths", args.len());
        cmd.push(&and_more);

        cmd.join(" ")
    }

    pub fn run(self) -> Result<Output> {
        if which(self.exe).is_err() {
            let path = match env::var("PATH") {
                Ok(p) => p,
                Err(e) => format!("<could not get PATH environment variable: {e}>"),
            };
            return Err(Error::ExecutableNotInPath {
                exe: self.exe.to_string(),
                path,
            }
            .into());
        }

        let cmd = self.as_command()?;

        if log_enabled!(Debug) {
            debug!(
                "Running command [{}] with cwd = {}",
                self.loggable_command,
                cmd.get_current_dir()
                    .expect("we just set the current_dir in as_command so this should be Some")
                    .display(),
            );
            for kv in self.env.iter().sorted_by(|a, b| a.0.cmp(b.0)) {
                debug!(r#"  with env: {} = "{}""#, kv.0, kv.1);
            }
        }

        let output = self
            .output_from_command(cmd)
            .with_context(|| format!(r"Failed to execute command `{}`", self.full_command()))?;

        if log_enabled!(Debug) && !output.stdout.is_empty() {
            debug!("Stdout was:\n{}", String::from_utf8(output.stdout.clone())?);
        }

        let code = output.status.code().unwrap_or(-1);
        if !output.stderr.is_empty() {
            let stderr = String::from_utf8(output.stderr.clone())?;
            if log_enabled!(Debug) {
                debug!("Stderr was:\n{stderr}");
            }

            if !self.ignore_stderr.iter().any(|i| i.is_match(&stderr)) {
                return Err(Error::UnexpectedStderr {
                    cmd: self.full_command(),
                    code,
                    stdout: String::from_utf8(output.stdout)
                        .unwrap_or("<could not turn stdout into a UTF-8 string>".to_string()),
                    stderr,
                }
                .into());
            }
        }

        Ok(Output {
            exit_code: code,
            stdout: bytes_to_option_string(&output.stdout),
            stderr: bytes_to_option_string(&output.stderr),
        })
    }

    fn output_from_command(&self, mut c: process::Command) -> Result<process::Output> {
        let status = self.maybe_spawn_status_thread();

        let output = c.output()?;
        if let Some((sender, thread)) = status {
            if let Err(err) = sender.send(ThreadMessage::Terminate) {
                warn!("Error terminating background status thread: {err}");
            }
            if let Err(err) = thread.join() {
                warn!("Error joining background status thread: {err:?}");
            }
        }

        self.handle_output(output)
    }

    fn handle_output(&self, output: process::Output) -> Result<process::Output> {
        if let Some(code) = output.status.code() {
            debug!(
                "Ran [{}] and got exit code of {}",
                self.loggable_command, code
            );
            return if self.ok_exit_codes.contains(&code) {
                Ok(output)
            } else {
                Err(Error::UnexpectedExitCode {
                    cmd: self.full_command(),
                    code,
                    stdout: String::from_utf8(output.stdout)?,
                    stderr: String::from_utf8(output.stderr)?,
                }
                .into())
            };
        }

        if output.status.success() {
            // I don't know under what circumstances this would happen. How does a process exit
            // successfully without a status code? Is this a Windows-only thing? But the way the
            // `process::Output` API works, this is a possibility, so we're gonna check for it.
            error!(
                "The {} command was successful but it had no exit code",
                self.loggable_command,
            );
            return Ok(output);
        }

        let signal = signal_from_status(output.status);
        debug!(
            "Ran {} which exited because of signal {}",
            self.full_command(),
            signal
        );
        Err(Error::ProcessKilledBySignal {
            cmd: self.full_command(),
            signal,
            stdout: String::from_utf8(output.stdout)?,
            stderr: String::from_utf8(output.stderr)?,
        }
        .into())
    }

    fn maybe_spawn_status_thread(&self) -> Option<(mpsc::Sender<ThreadMessage>, JoinHandle<()>)> {
        if !log_enabled!(Info) {
            return None;
        }

        let loggable_command = self.loggable_command.clone();
        let (sender, receiver) = mpsc::channel();

        let handle = thread::spawn(move || loop {
            match receiver.recv_timeout(Duration::from_secs(5)) {
                Ok(ThreadMessage::Terminate) => {
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {
                    info!("Still running [{loggable_command}]");
                }
                Err(RecvTimeoutError::Disconnected) => {
                    warn!("Got a disconnected error receiving message from main thread");
                    break;
                }
            }
        });

        Some((sender, handle))
    }

    pub fn as_command(&self) -> Result<Command> {
        let mut cmd = Command::new(self.exe);
        cmd.args(&self.args);

        let in_dir = if let Some(d) = &self.in_dir {
            d.to_path_buf()
        } else {
            env::current_dir()?
        };

        let in_dir = fs::canonicalize(in_dir)?;
        debug!("Setting current dir to {}", in_dir.display());

        // We are canonicalizing this primarily for the benefit of our debugging output, because
        // otherwise we might see the current dir as just `.`, which is not helpful.
        cmd.current_dir(in_dir);

        cmd.envs(&self.env);

        Ok(cmd)
    }

    #[must_use]
    pub fn full_command(&self) -> String {
        let mut cmd = vec![self.exe];
        cmd.extend(&self.args);
        cmd.join(" ")
    }
}

fn bytes_to_option_string(v: &[u8]) -> Option<String> {
    if v.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(v).into_owned())
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
    use super::*;
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
    use test_case::test_case;
    use which::which;

    #[test]
    #[parallel]
    fn run_exit_0() -> Result<()> {
        if !which("echo").is_ok() {
            return Ok(());
        }
        let res = Exec::builder()
            .exe("echo")
            .args(vec!["foo"])
            .ok_exit_codes(&[0])
            .build()
            .run()?;
        assert_eq!(res.exit_code, 0, "process exits 0");

        Ok(())
    }

    // This gets used for a number of tests, so we'll just define it once.
    const BASH_ECHO_TO_STDERR_SCRIPT: &str = "echo 'some stderr output' 1>&2";

    #[test]
    #[parallel]
    fn run_exit_0_with_unexpected_stderr() -> Result<()> {
        if which("bash").is_err() {
            println!("Skipping test since bash is not in path");
            return Ok(());
        }

        let res = Exec::builder()
            .exe("bash")
            .args(vec!["-c", BASH_ECHO_TO_STDERR_SCRIPT])
            .ok_exit_codes(&[0])
            .build()
            .run();
        assert!(res.is_err(), "run returned Err");
        match error_from_run(res)? {
            Error::UnexpectedStderr {
                cmd: _,
                code,
                stdout,
                stderr,
            } => {
                assert_eq!(code, 0, "process exited 0");
                assert_eq!(stdout, "", "process had no stdout output");
                assert_eq!(
                    stderr, "some stderr output\n",
                    "process had expected stderr output"
                );
            }
            e => return Err(e.into()),
        }
        Ok(())
    }

    #[test]
    #[parallel]
    fn run_exit_0_with_matching_ignore_stderr() -> Result<()> {
        if which("bash").is_err() {
            println!("Skipping test since bash is not in path");
            return Ok(());
        }

        let regex = Regex::new("some.+output").unwrap();
        let res = Exec::builder()
            .exe("bash")
            .args(vec!["-c", BASH_ECHO_TO_STDERR_SCRIPT])
            .ok_exit_codes(&[0])
            .ignore_stderr(vec![regex])
            .build()
            .run()?;
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
        if which("bash").is_err() {
            println!("Skipping test since bash is not in path");
            return Ok(());
        }

        let regex = Regex::new("some.+output is ok").unwrap();
        let res = Exec::builder()
            .exe("bash")
            .args(vec!["-c", BASH_ECHO_TO_STDERR_SCRIPT])
            .ok_exit_codes(&[0])
            .ignore_stderr(vec![regex])
            .build()
            .run();
        assert!(res.is_err(), "run returned Err");
        match error_from_run(res)? {
            Error::UnexpectedStderr {
                cmd: _,
                code,
                stdout,
                stderr,
            } => {
                assert_eq!(code, 0, "process exited 0");
                assert_eq!(stdout, "", "process had no stdout output");
                assert_eq!(
                    stderr, "some stderr output\n",
                    "process had expected stderr output"
                );
            }
            e => return Err(e.into()),
        }
        Ok(())
    }

    #[test]
    #[parallel]
    fn run_exit_0_with_multiple_ignore_stderr() -> Result<()> {
        if which("bash").is_err() {
            println!("Skipping test since bash is not in path");
            return Ok(());
        }

        let regex1 = Regex::new("will not match").unwrap();
        let regex2 = Regex::new("some.+output is ok").unwrap();
        let res = Exec::builder()
            .exe("bash")
            .args(vec!["-c", BASH_ECHO_TO_STDERR_SCRIPT])
            .ok_exit_codes(&[0])
            .ignore_stderr(vec![regex1, regex2])
            .build()
            .run();
        assert!(res.is_err(), "run returned Err");
        match error_from_run(res)? {
            Error::UnexpectedStderr {
                cmd: _,
                code,
                stdout,
                stderr,
            } => {
                assert_eq!(code, 0, "process exited 0");
                assert_eq!(stdout, "", "process had no stdout output");
                assert_eq!(
                    stderr, "some stderr output\n",
                    "process had expected stderr output"
                );
            }
            e => return Err(e.into()),
        }
        Ok(())
    }

    #[test]
    #[parallel]
    fn run_with_env() -> Result<()> {
        if which("bash").is_err() {
            println!("Skipping test since bash is not in path");
            return Ok(());
        }

        let env_key = "PRECIOUS_ENV_TEST";
        let mut env = HashMap::new();
        env.insert(String::from(env_key), String::from("foo"));

        let res = Exec::builder()
            .exe("bash")
            .args(vec!["-c", &format!("echo ${env_key}")])
            .ok_exit_codes(&[0])
            .env(env)
            .build()
            .run()?;
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
        if which("bash").is_err() {
            println!("Skipping test since bash is not in path");
            return Ok(());
        }

        let res = Exec::builder()
            .exe("bash")
            .args(vec!["-c", "exit 32"])
            .ok_exit_codes(&[0])
            .build()
            .run();
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
        if which("bash").is_err() {
            println!("Skipping test since bash is not in path");
            return Ok(());
        }

        let res = Exec::builder()
            .exe("bash")
            .args(vec!["-c", r#"echo "STDOUT" && exit 32"#])
            .ok_exit_codes(&[0])
            .build()
            .run();
        assert!(res.is_err(), "process exits non-zero");
        let e = error_from_run(res)?;
        let expect = r#"Got unexpected exit code 32 from `bash -c echo "STDOUT" && exit 32`.
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
        if which("bash").is_err() {
            println!("Skipping test since bash is not in path");
            return Ok(());
        }

        let res = Exec::builder()
            .exe("bash")
            .args(vec!["-c", r#"echo "STDERR" 1>&2 && exit 32"#])
            .ok_exit_codes(&[0])
            .build()
            .run();
        assert!(res.is_err(), "process exits non-zero");
        let e = error_from_run(res)?;
        let expect = r#"Got unexpected exit code 32 from `bash -c echo "STDERR" 1>&2 && exit 32`.
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
        if which("bash").is_err() {
            println!("Skipping test since bash is not in path");
            return Ok(());
        }

        let res = Exec::builder()
            .exe("bash")
            .args(vec![
                "-c",
                r#"echo "STDOUT" && echo "STDERR" 1>&2 && exit 32"#,
            ])
            .ok_exit_codes(&[0])
            .build()
            .run();
        assert!(res.is_err(), "process exits non-zero");

        let e = error_from_run(res)?;
        let expect = r#"Got unexpected exit code 32 from `bash -c echo "STDOUT" && echo "STDERR" 1>&2 && exit 32`.
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

    #[cfg(target_family = "unix")]
    #[test]
    #[parallel]
    fn run_exit_from_sig_kill() -> Result<()> {
        if which("bash").is_err() {
            println!("Skipping test since bash is not in path");
            return Ok(());
        }

        let res = Exec::builder()
            .exe("bash")
            .args(vec!["-c", r#"sleep 0.1 && kill -TERM "$$""#])
            .ok_exit_codes(&[0])
            .build()
            .run();
        assert!(res.is_err(), "process exits non-zero");

        match error_from_run(res)? {
            Error::ProcessKilledBySignal {
                cmd: _,
                signal,
                stdout,
                stderr,
            } => {
                assert_eq!(signal, libc::SIGTERM, "process exited because of SIGTERM");
                assert_eq!(stdout, "", "process had no stdout");
                assert_eq!(stderr, "", "process had no stderr");
            }
            e => return Err(e.into()),
        }

        Ok(())
    }

    fn error_from_run(result: Result<super::Output>) -> Result<Error> {
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

        let res = Exec::builder()
            .exe("pwd")
            .ok_exit_codes(&[0])
            .in_dir(&td_path)
            .build()
            .run()?;
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
        let res = Exec::builder()
            .exe("I hope this binary does not exist on any system!")
            .args(vec!["--arg", "42"])
            .ok_exit_codes(&[0])
            .build()
            .run();
        assert!(res.is_err());
        if let Err(e) = res {
            assert!(e.to_string().contains(
                r#"Could not find "I hope this binary does not exist on any system!" in your path"#,
            ));
        }
    }

    #[test_case("foo", &[], 0, "foo"; "no arguments")]
    #[test_case("foo", &["--bar"], 0, "foo --bar"; "one flag")]
    #[test_case("foo", &["--bar", "--baz"], 0, "foo --bar --baz"; "two flags")]
    #[test_case(
        "foo",
        &["--bar", "--baz", "--buz"],
        0,
        "foo --bar --baz --buz";
        "three flags"
    )]
    #[test_case(
        "foo",
        &["--bar", "--baz", "--buz", "--quux"],
        0,
        "foo --bar --baz --buz --quux";
        "four flags"
    )]
    #[test_case(
        "foo",
        &["--bar", "--baz", "--buz", "--quux"],
        0,
        "foo --bar --baz --buz --quux";
        "five flags"
    )]
    #[test_case(
        "foo",
        &["bar"],
        1,
        "foo bar";
        "one path"
    )]
    #[test_case(
        "foo",
        &["bar", "baz"],
        2,
        "foo bar baz";
        "two paths"
    )]
    #[test_case(
        "foo",
        &["bar", "baz", "buz"],
        3,
        "foo bar baz buz";
        "three paths"
    )]
    #[test_case(
        "foo",
        &["bar", "baz", "buz", "quux"],
        4,
        "foo bar baz ... and 2 more paths";
        "four paths"
    )]
    #[test_case(
        "foo",
        &["bar", "baz", "buz", "quux", "corge"],
        5,
        "foo bar baz ... and 3 more paths";
        "five paths"
    )]
    #[test_case(
        "foo",
        &["bar", "baz", "buz", "quux", "corge", "grault"],
        6,
        "foo bar baz ... and 4 more paths";
        "six paths"
    )]
    #[test_case(
        "foo",
        &["--bar", "--baz", "--buz", "--quux", "bar"],
        1,
        "foo --bar --baz --buz --quux bar";
        "four flags and one path"
    )]
    #[test_case(
        "foo",
        &["--bar", "--baz", "--buz", "--quux", "bar", "baz"],
        2,
        "foo --bar --baz --buz --quux bar baz";
        "four flags and two paths"
    )]
    #[test_case(
        "foo",
        &["--bar", "--baz", "--buz", "--quux", "bar", "baz", "buz"],
        2,
        "foo --bar --baz --buz --quux bar baz buz";
        "four flags and three paths"
    )]
    #[test_case(
        "foo",
        &["--bar", "--baz", "--buz", "--quux", "bar", "baz", "buz", "quux"],
        4,
        "foo --bar --baz --buz --quux bar baz ... and 2 more paths";
        "four flags and four paths"
    )]
    #[test_case(
        "foo",
        &["--bar", "--baz", "--buz", "--quux", "bar", "baz", "buz", "quux", "corge"],
        5,
        "foo --bar --baz --buz --quux bar baz ... and 3 more paths";
        "four flags and five paths"
    )]
    #[test_case(
        "foo",
        &["--bar", "--baz", "--buz", "--quux", "bar", "baz", "buz", "quux", "corge", "grault"],
        6,
        "foo --bar --baz --buz --quux bar baz ... and 4 more paths";
        "four flags and six paths"
    )]
    #[parallel]
    fn loggable_command(exe: &str, args: &[&str], num_paths: usize, expect: &str) {
        let exec = Exec::builder()
            .exe(exe)
            .args(args.to_vec())
            .num_paths(num_paths)
            .ok_exit_codes(&[0])
            .build();
        assert_eq!(exec.loggable_command, expect);
    }

    // The temp directory on macOS in GitHub Actions appears to be a symlink, but
    // canonicalizing on Windows breaks tests for some reason.
    fn maybe_canonicalize(path: &Path) -> Result<PathBuf> {
        if cfg!(windows) {
            return Ok(path.to_owned());
        }
        Ok(fs::canonicalize(path)?)
    }
}
