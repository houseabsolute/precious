#![recursion_limit = "1024"]

#[cfg(test)]
mod testhelper;

// For some reason I don't understand this needs to be loaded via "extern
// crate" and not "use".
#[macro_use]
extern crate failure_derive;

mod basepaths;
mod cache;
mod command;
mod config;
mod filter;
mod path_matcher;
mod vcs;

use clap::{App, Arg, ArgGroup, SubCommand};
use failure::Error;
use log::{debug, error};
use rayon::prelude::*;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let matches = make_app().get_matches();
    init_logger(&matches);
    let main = Main::new(&matches);
    let status = if let Ok(mut m) = main {
        m.run()
    } else {
        error!("{}", main.unwrap_err());
        127 as i32
    };
    std::process::exit(status);
}

fn init_logger(matches: &clap::ArgMatches) {
    let level: u64 = if matches.is_present("trace") {
        3 // trace level
    } else if matches.is_present("debug") {
        2 // debug level
    } else if matches.is_present("verbose") {
        1 // info level
    } else {
        0 // warn level
    };
    loggerv::init_with_verbosity(level).unwrap();
}

fn make_app<'a>() -> App<'a, 'a> {
    App::new("precious")
        .version(env!("CARGO_PKG_VERSION"))
        .author("Dave Rolsky <autarch@urth.org>")
        .about("One code quality tool to rule them all")
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .takes_value(true)
                .help("Path to config file"),
        )
        .arg(
            Arg::with_name("ascii")
                .long("ascii")
                .help("Replace super-fun Unicode symbols with terribly boring ASCII"),
        )
        .arg(
            Arg::with_name("verbose")
                .short("v")
                .long("verbose")
                .help("Enable verbose output"),
        )
        .arg(
            Arg::with_name("debug")
                .short("d")
                .long("debug")
                .help("Enable debugging output"),
        )
        .arg(
            Arg::with_name("trace")
                .short("t")
                .long("trace")
                .help("Enable tracing output (maximum logging)"),
        )
        .arg(
            Arg::with_name("quiet")
                .short("q")
                .long("quiet")
                .help("Suppresses most output"),
        )
        .group(ArgGroup::with_name("log-level").args(&["verbose", "debug", "trace", "quiet"]))
        .subcommand(common_subcommand(
            "tidy",
            "Tidies the specified files and/or directories",
        ))
        .subcommand(common_subcommand(
            "lint",
            "Lints the specified files and/or directories",
        ))
}

fn common_subcommand<'a>(name: &'a str, about: &'a str) -> App<'a, 'a> {
    SubCommand::with_name(name)
        .about(about)
        .arg(
            Arg::with_name("all")
                .short("a")
                .long("all")
                .help("Run against all files in the current directory and below"),
        )
        .arg(
            Arg::with_name("git")
                .short("g")
                .long("git")
                .help("Run against files that have been modified according to git"),
        )
        .arg(
            Arg::with_name("staged")
                .short("s")
                .long("staged")
                .help("Run against file content that is staged for a git commit"),
        )
        .arg(
            Arg::with_name("paths")
                .multiple(true)
                .takes_value(true)
                .help("A list of paths on which to operate"),
        )
        .group(
            ArgGroup::with_name("operate-on")
                .args(&["all", "git", "staged", "paths"])
                .required(true),
        )
}

#[derive(Debug, Fail)]
enum MainError {
    #[fail(display = "Could not find a VCS checkout root starting from {}", cwd)]
    CannotFindRoot { cwd: String },

    #[fail(display = "No tidiers defined in your config")]
    NoTidiers,

    #[fail(display = "No linters defined in your config")]
    NoLinters,
}

#[derive(Debug)]
struct Exit {
    status: i32,
    message: Option<String>,
    error: Option<String>,
}

impl From<Error> for Exit {
    fn from(err: Error) -> Exit {
        Exit {
            status: 1,
            message: None,
            error: Some(err.to_string()),
        }
    }
}

#[derive(Debug)]
struct Chars {
    ring: &'static str,
    tidied: &'static str,
    unchanged: &'static str,
    lint_free: &'static str,
    lint_dirty: &'static str,
    empty: &'static str,
}

const FUN_CHARS: Chars = Chars {
    ring: "💍",
    tidied: "💧",
    unchanged: "✨",
    lint_free: "💯",
    lint_dirty: "💩",
    empty: "⚫",
};

const BORING_CHARS: Chars = Chars {
    ring: ":",
    tidied: "*",
    unchanged: "|",
    lint_free: "|",
    lint_dirty: "*",
    empty: "_",
};

#[derive(Debug)]
struct Main<'a> {
    matches: &'a clap::ArgMatches<'a>,
    config: Option<config::Config>,
    root: Option<PathBuf>,
    chars: Chars,
    quiet: bool,
    basepaths: Option<basepaths::BasePaths>,
}

impl<'a> Main<'a> {
    fn new(matches: &'a clap::ArgMatches) -> Result<Main<'a>, Error> {
        let chars = if matches.is_present("ascii") {
            BORING_CHARS
        } else {
            FUN_CHARS
        };

        let mut s = Main {
            matches,
            config: None,
            root: None,
            chars,
            quiet: matches.is_present("quiet"),
            basepaths: None,
        };
        s.set_config()?;

        Ok(s)
    }

    fn run(&mut self) -> i32 {
        match self.run_subcommand() {
            Ok(e) => {
                if e.error.is_some() {
                    error!("{}", e.error.unwrap());
                }
                if e.message.is_some() {
                    println!("{} {}", self.chars.empty, e.message.unwrap());
                }
                e.status
            }
            Err(e) => {
                error!("Failed to run precious: {}", e);
                let bt = format!("{}", e.backtrace());
                if !bt.is_empty() {
                    debug!("{}", bt);
                }
                127 as i32
            }
        }
    }

    fn run_subcommand(&mut self) -> Result<Exit, Error> {
        if self.matches.subcommand_matches("tidy").is_some() {
            return self.tidy();
        } else if self.matches.subcommand_matches("lint").is_some() {
            return self.lint();
        }

        Ok(Exit {
            status: 1,
            message: None,
            error: Some(String::from(
                "You must run either the tidy or lint subcommand",
            )),
        })
    }

    fn tidy(&mut self) -> Result<Exit, Error> {
        println!("{} Tidying {}", self.chars.ring, self.mode());

        let tidiers = self.config().tidy_filters(&self.root_dir())?;
        if tidiers.is_empty() {
            return Err(MainError::NoTidiers.into());
        }

        let mut status = 0 as i32;

        for t in tidiers {
            let map = if t.run_mode_is(filter::RunMode::Root) {
                self.root_as_paths()?
            } else if t.run_mode_is(filter::RunMode::Dirs) {
                self.dirs()?
            } else {
                self.files()?
            };

            if map.is_none() {
                return Ok(self.no_files_exit());
            }

            let failures: Vec<i32> = map
                .unwrap()
                .par_iter()
                .map(|(p, paths)| -> i32 {
                    match t.tidy(p, &paths.files) {
                        Ok(Some(true)) => {
                            if !self.quiet {
                                println!(
                                    "{} Tidied by {}:    {}",
                                    self.chars.tidied,
                                    t.name,
                                    p.to_string_lossy()
                                );
                            }
                            0 as i32
                        }
                        Ok(Some(false)) => {
                            if !self.quiet {
                                println!(
                                    "{} Unchanged by {}: {}",
                                    self.chars.unchanged,
                                    t.name,
                                    p.to_string_lossy()
                                );
                            }
                            0 as i32
                        }
                        Ok(None) => 0,
                        Err(e) => {
                            error!("{}", e);
                            1 as i32
                        }
                    }
                })
                .collect();
            for f in failures {
                status += f;
            }
        }
        Ok(Self::exit_from_status(status, "tidying"))
    }

    fn lint(&mut self) -> Result<Exit, Error> {
        println!("{} Linting {}", self.chars.ring, self.mode());

        let linters = self.config().lint_filters(&self.root_dir())?;
        if linters.is_empty() {
            return Err(MainError::NoLinters.into());
        }

        let mut status = 0 as i32;

        for l in linters {
            let map = if l.run_mode_is(filter::RunMode::Root) {
                self.root_as_paths()?
            } else if l.run_mode_is(filter::RunMode::Dirs) {
                self.dirs()?
            } else {
                self.files()?
            };

            if map.is_none() {
                return Ok(self.no_files_exit());
            }

            let failures: Vec<i32> = map
                .unwrap()
                .par_iter()
                .map(|(p, paths)| -> i32 {
                    match l.lint(p, &paths.files) {
                        Ok(Some(r)) => {
                            if r.ok {
                                if !self.quiet {
                                    println!(
                                        "{} Passed {}: {}",
                                        self.chars.lint_free,
                                        l.name,
                                        p.to_string_lossy()
                                    );
                                }
                            } else {
                                println!(
                                    "{} Failed {}: {}",
                                    self.chars.lint_dirty,
                                    l.name,
                                    p.to_string_lossy()
                                );
                                if r.stdout.is_some() {
                                    println!("{}", r.stdout.unwrap());
                                }
                                if r.stderr.is_some() {
                                    println!("{}", r.stderr.unwrap());
                                }
                            }
                            0
                        }
                        Ok(None) => 0,
                        Err(e) => {
                            error!("{}", e);
                            1
                        }
                    }
                })
                .collect();
            for f in failures {
                status += f;
            }
        }
        Ok(Self::exit_from_status(status, "linting"))
    }

    fn no_files_exit(&self) -> Exit {
        Exit {
            status: 0,
            message: Some(String::from("No files found")),
            error: None,
        }
    }

    fn exit_from_status(status: i32, action: &str) -> Exit {
        let error = if status == 0 {
            None
        } else {
            Some(format!("Error when {} files", action))
        };
        Exit {
            status,
            message: None,
            error,
        }
    }

    fn root_as_paths(&mut self) -> Result<Option<HashMap<PathBuf, basepaths::Paths>>, Error> {
        let mut m = HashMap::new();
        let paths = self.basepaths()?.paths()?;
        if paths.is_none() {
            return Ok(None);
        }

        let mut all: Vec<PathBuf> = vec![];
        for p in paths.unwrap().iter_mut() {
            all.append(&mut p.files);
        }

        let root_paths = basepaths::Paths {
            dir: PathBuf::from("."),
            files: all,
        };
        m.insert(PathBuf::from("."), root_paths);
        Ok(Some(m))
    }

    fn dirs(&mut self) -> Result<Option<HashMap<PathBuf, basepaths::Paths>>, Error> {
        let mut map = HashMap::new();
        let paths = self.basepaths()?.paths()?;
        if paths.is_none() {
            return Ok(None);
        }

        for p in paths.unwrap() {
            map.insert(p.dir.clone(), p);
        }
        Ok(Some(map))
    }

    fn files(&mut self) -> Result<Option<HashMap<PathBuf, basepaths::Paths>>, Error> {
        let mut map = HashMap::new();
        let paths = self.basepaths()?.paths()?;
        if paths.is_none() {
            return Ok(None);
        }

        for p in paths.unwrap() {
            // This is gross
            let p_clone = p.clone();
            for f in p.files {
                map.insert(f.clone(), p_clone.clone());
            }
        }
        Ok(Some(map))
    }

    fn basepaths(&mut self) -> Result<&mut basepaths::BasePaths, Error> {
        if self.basepaths.is_none() {
            let (mode, paths) = self.mode_and_paths_from_args();
            self.basepaths = Some(basepaths::BasePaths::new(
                mode,
                paths,
                self.root_dir(),
                self.config().exclude.clone(),
            )?);
        }
        Ok(self.basepaths.as_mut().unwrap())
    }

    fn mode(&self) -> basepaths::Mode {
        let (mode, _) = self.mode_and_paths_from_args();
        mode
    }

    fn mode_and_paths_from_args(&self) -> (basepaths::Mode, Vec<PathBuf>) {
        let subc_matches = self.matched_subcommand();

        let mut paths: Vec<PathBuf> = vec![];
        if subc_matches.is_present("all") {
            return (basepaths::Mode::All, paths);
        } else if subc_matches.is_present("git") {
            return (basepaths::Mode::GitModified, paths);
        } else if subc_matches.is_present("staged") {
            return (basepaths::Mode::GitStaged, paths);
        }

        if !subc_matches.is_present("paths") {
            panic!("No mode or paths were provided but clap did not return an error");
        }
        subc_matches.values_of("paths").unwrap().for_each(|p| {
            paths.push(PathBuf::from(p));
        });
        (basepaths::Mode::FromCLI, paths)
    }

    fn matched_subcommand(&self) -> &clap::ArgMatches<'a> {
        match self.matches.subcommand() {
            ("tidy", Some(m)) => m,
            ("lint", Some(m)) => m,
            _ => panic!("Somehow none of our subcommands matched and clap did not return an error"),
        }
    }

    fn set_config(&mut self) -> Result<(), Error> {
        self.set_root()?;
        let file = if self.matches.is_present("config") {
            let conf_file = self.matches.value_of("config").unwrap();
            debug!("Loading config from {} (set via flag)", conf_file);
            PathBuf::from(conf_file)
        } else {
            let default = self.default_config_file();
            debug!(
                "Loading config from {} (default location)",
                default.to_string_lossy()
            );
            default
        };

        self.config = Some(config::Config::new_from_file(file)?);

        Ok(())
    }

    fn set_root(&mut self) -> Result<(), Error> {
        let cwd = env::current_dir()?;
        let mut root = PathBuf::new();
        for anc in cwd.ancestors() {
            if Self::is_checkout_root(&anc) {
                root.push(anc);
                self.root = Some(root);
                return Ok(());
            }
        }

        Err(MainError::CannotFindRoot {
            cwd: cwd.to_string_lossy().to_string(),
        }
        .into())
    }

    fn is_checkout_root(path: &Path) -> bool {
        for dir in vcs::dirs() {
            let mut poss = PathBuf::from(path);
            poss.push(dir);
            if poss.exists() {
                return true;
            }
        }
        false
    }

    fn default_config_file(&self) -> PathBuf {
        let mut file = self.root_dir();
        file.push("precious.toml");
        file
    }

    fn root_dir(&self) -> PathBuf {
        self.root.as_ref().unwrap().clone()
    }

    fn config(&self) -> &config::Config {
        self.config.as_ref().unwrap()
    }
}
