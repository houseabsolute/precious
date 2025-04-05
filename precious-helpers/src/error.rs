use thiserror::Error;

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
    ProcessKilledBySignal {
        cmd: String,
        signal: i32,
        stdout: String,
        stderr: String,
    },

    #[error("Got unexpected stderr output from `{cmd:}` with exit code {code:}:\n{stderr:}")]
    UnexpectedStderr {
        cmd: String,
        code: i32,
        stdout: String,
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
    }
    output.push('\n');
    output
}
