use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Mode {
    FromCli,
    All,
    GitModified,
    GitStaged,
    GitStagedWithStash,
    GitDiffFrom(String),
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Mode::FromCli => write!(f, "paths passed on the command line (recursively)"),
            Mode::All => write!(f, "all files in the project"),
            Mode::GitModified => write!(f, "modified files according to git"),
            Mode::GitStaged => write!(f, "files staged for a git commit"),
            Mode::GitStagedWithStash => write!(
                f,
                "files staged for a git commit, stashing unstaged content"
            ),
            Mode::GitDiffFrom(from) => write!(f, "files modified as compared to {from:}"),
        }
    }
}
