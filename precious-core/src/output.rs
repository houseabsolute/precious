use crate::{chars::Chars, paths::mode::Mode};
use itertools::Itertools;
use std::{
    env,
    fmt::{self, Debug, Formatter},
    path::Path,
};

pub(crate) trait OutputWriter: Debug {
    fn write_subcommand_exit_error(&self, err: String);

    fn write_subcommand_exit_message(&self, err: String);

    fn write_starting_action(&self, action: &str, mode: Mode);

    fn write_command_tidied_files(&self, command: &str, files: &[&Path]);

    fn write_command_did_not_tidy_files(&self, command: &str, files: &[&Path]);

    fn write_command_maybe_tidied_files(&self, command: &str, files: &[&Path]);

    fn write_command_found_lint_clean_files(&self, command: &str, files: &[&Path]);

    fn write_command_found_lint_dirty_files(
        &self,
        command: &str,
        files: &[&Path],
        stdout: Option<String>,
        stderr: Option<String>,
    );

    fn write_command_errored_for_files(&self, command: &str, files: &[&Path]);

    fn flush(&self);

    fn chars(&self) -> &Chars;
}

pub(crate) struct UnstructuredTextWriter {
    chars: Chars,
    quiet: bool,
}

impl UnstructuredTextWriter {
    pub(crate) fn new(chars: Chars, quiet: bool) -> Self {
        Self { chars, quiet }
    }
}

impl OutputWriter for UnstructuredTextWriter {
    fn write_subcommand_exit_error(&self, err: String) {
        print!("{}", err);
    }

    fn write_subcommand_exit_message(&self, msg: String) {
        println!("{} {}", self.chars.empty, msg);
    }

    fn write_starting_action(&self, action: &str, mode: Mode) {
        println!("{} {} {}", self.chars.ring, action, mode);
    }

    fn write_command_tidied_files(&self, command: &str, files: &[&Path]) {
        if self.quiet {
            return;
        }
        println!(
            "{} Tidied {}: [{}]",
            self.chars.tidied,
            command,
            files.iter().map(|p| p.to_string_lossy()).join(" ")
        );
    }

    fn write_command_did_not_tidy_files(&self, command: &str, files: &[&Path]) {
        if self.quiet {
            return;
        }
        println!(
            "{} Unchanged {}: [{}]",
            self.chars.tidied,
            command,
            files.iter().map(|p| p.to_string_lossy()).join(" ")
        );
    }

    fn write_command_maybe_tidied_files(&self, command: &str, files: &[&Path]) {
        if self.quiet {
            return;
        }
        println!(
            "{} Maybe changed {}: [{}]",
            self.chars.tidied,
            command,
            files.iter().map(|p| p.to_string_lossy()).join(" ")
        );
    }

    fn write_command_found_lint_clean_files(&self, command: &str, files: &[&Path]) {
        if self.quiet {
            return;
        }
        println!(
            "{} Passed {}: [{}]",
            self.chars.tidied,
            command,
            files.iter().map(|p| p.to_string_lossy()).join(" ")
        );
    }

    fn write_command_found_lint_dirty_files(
        &self,
        command: &str,
        files: &[&Path],
        stdout: Option<String>,
        stderr: Option<String>,
    ) {
        println!(
            "{} Failed {}: [{}]",
            self.chars.tidied,
            command,
            files.iter().map(|p| p.to_string_lossy()).join(" ")
        );
        if let Some(s) = stdout {
            println!("{}", s);
        }
        if let Some(s) = stderr {
            println!("{}", s);
        }

        if let Ok(ga) = env::var("GITHUB_ACTIONS") {
            if !ga.is_empty() {
                if files.len() == 1 {
                    println!(
                        "::error file={}::Linting with {} failed",
                        files[0].display(),
                        command
                    );
                } else {
                    println!("::error::Linting with {} failed", command);
                }
            }
        }
    }

    fn write_command_errored_for_files(&self, command: &str, files: &[&Path]) {
        println!(
            "{} Error from {}: [{}]",
            self.chars.tidied,
            command,
            files.iter().map(|p| p.to_string_lossy()).join(" ")
        );
    }

    fn flush(&self) {}

    fn chars(&self) -> &Chars {
        &self.chars
    }
}

impl Debug for UnstructuredTextWriter {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "UnstructuredTextWriter")
    }
}

unsafe impl Sync for UnstructuredTextWriter {}
