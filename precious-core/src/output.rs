use crate::{chars::Chars, paths::mode::Mode};
use anyhow::Result;
use itertools::Itertools;
use std::{
    env,
    fmt::{self, Debug, Formatter},
    path::PathBuf,
};

pub(crate) trait OutputWriter: Debug + Sync {
    fn handle_event(&mut self, event: Event) -> Result<()>;

    fn flush(&self) -> Result<()>;

    fn chars(&self) -> &Chars;
}

pub(crate) struct UnstructuredTextWriter {
    chars: &'static Chars,
    quiet: bool,
}

impl Debug for UnstructuredTextWriter {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "UnstructuredTextWriter")
    }
}

impl OutputWriter for UnstructuredTextWriter {
    fn handle_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::SubcommandExitWithError(err) => self.write_subcommand_exit_error(err),
            Event::SubcommandExitWithMessage(msg) => self.write_subcommand_exit_message(msg),
            Event::StartingAction(action, mode) => self.write_starting_action(action, mode),
            Event::TidiedFiles(command, files) => self.write_command_tidied_files(command, files),
            Event::MaybeTidiedFiles(command, files) => {
                self.write_command_maybe_tidied_files(command, files)
            }
            Event::DidNotTidyFiles(command, files) => {
                self.write_command_did_not_tidy_files(command, files)
            }
            Event::FoundLintCleanFiles(command, files) => {
                self.write_command_found_lint_clean_files(command, files)
            }
            Event::FoundLintDirtyFiles(command, files, stdout, stderr) => {
                self.write_command_found_lint_dirty_files(command, files, stdout, stderr)
            }
            Event::CommandError(command, files) => {
                self.write_command_errored_for_files(command, files)
            }
        }
        Ok(())
    }

    fn flush(&self) -> Result<()> {
        Ok(())
    }

    fn chars(&self) -> &Chars {
        self.chars
    }
}

impl UnstructuredTextWriter {
    pub(crate) fn new(chars: &'static Chars, quiet: bool) -> Self {
        Self { chars, quiet }
    }

    fn write_subcommand_exit_error(&self, err: String) {
        print!("{err}");
    }

    fn write_subcommand_exit_message(&self, msg: String) {
        println!("{} {msg}", self.chars.empty);
    }

    fn write_starting_action(&self, action: &str, mode: Mode) {
        println!("{} {action} {mode}", self.chars.ring,);
    }

    fn write_command_tidied_files(&self, command: String, files: Vec<PathBuf>) {
        if self.quiet {
            return;
        }
        println!(
            "{} Tidied {command}: [{}]",
            self.chars.tidied,
            files.iter().map(|p| p.to_string_lossy()).join(" ")
        );
    }

    fn write_command_did_not_tidy_files(&self, command: String, files: Vec<PathBuf>) {
        if self.quiet {
            return;
        }
        println!(
            "{} Unchanged {command}: [{}]",
            self.chars.unchanged,
            files.iter().map(|p| p.to_string_lossy()).join(" ")
        );
    }

    fn write_command_maybe_tidied_files(&self, command: String, files: Vec<PathBuf>) {
        if self.quiet {
            return;
        }
        println!(
            "{} Maybe changed {command}: [{}]",
            self.chars.maybe_changed,
            files.iter().map(|p| p.to_string_lossy()).join(" ")
        );
    }

    fn write_command_found_lint_clean_files(&self, command: String, files: Vec<PathBuf>) {
        if self.quiet {
            return;
        }
        println!(
            "{} Passed {command}: [{}]",
            self.chars.lint_clean,
            files.iter().map(|p| p.to_string_lossy()).join(" ")
        );
    }

    fn write_command_found_lint_dirty_files(
        &self,
        command: String,
        files: Vec<PathBuf>,
        stdout: Option<String>,
        stderr: Option<String>,
    ) {
        println!(
            "{} Failed {command}: [{}]",
            self.chars.lint_dirty,
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

    fn write_command_errored_for_files(&self, command: String, files: Vec<PathBuf>) {
        println!(
            "{} Error from {command}: [{}]",
            self.chars.execution_error,
            files.iter().map(|p| p.to_string_lossy()).join(" ")
        );
    }
}

#[derive(Debug)]
pub(crate) enum Event {
    SubcommandExitWithError(String),
    SubcommandExitWithMessage(String),
    StartingAction(&'static str, Mode),
    TidiedFiles(String, Vec<PathBuf>),
    DidNotTidyFiles(String, Vec<PathBuf>),
    MaybeTidiedFiles(String, Vec<PathBuf>),
    FoundLintCleanFiles(String, Vec<PathBuf>),
    FoundLintDirtyFiles(String, Vec<PathBuf>, Option<String>, Option<String>),
    CommandError(String, Vec<PathBuf>),
}
