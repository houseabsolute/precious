#![recursion_limit = "1024"]

// For some reason I don't understand this needs to be loaded via "extern
// crate" and not "use".
#[macro_use]
extern crate failure_derive;

mod basepaths;
mod command;
mod config;
mod excluder;
mod filter;
mod gitignore;
mod vcs;

use clap::{App, Arg, ArgGroup, SubCommand};
use failure::Error;
use log::{debug, error};
use rayon::prelude::*;
use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let matches = make_app().get_matches();
    init_logger(&matches);
    let main = Main::new(&matches);
    let status = if main.is_ok() {
        match main.unwrap().run() {
            Ok(e) => {
                if e.error.is_some() {
                    error!("{}", e.error.unwrap());
                }
                e.status
            }
            Err(e) => {
                error!("Failed to run precious: {}", e);
                debug!("{}", e.backtrace());
                127 as i32
            }
        }
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
        .version("0.0.1")
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
                .help("Enable tracinng output (maximum logging)"),
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

    #[fail(display = "Could not find a parent for the {} path", path)]
    PathHasNoParent { path: String },
}

#[derive(Debug)]
struct Exit {
    status: i32,
    error: Option<String>,
}

impl From<Error> for Exit {
    fn from(err: Error) -> Exit {
        Exit {
            status: 1,
            error: Some(err.to_string()),
        }
    }
}

#[derive(Debug)]
struct Main<'a> {
    matches: &'a clap::ArgMatches<'a>,
    config: Option<config::Config>,
    root: Option<PathBuf>,
    quiet: bool,
}

impl<'a> Main<'a> {
    fn new(matches: &'a clap::ArgMatches) -> Result<Main<'a>, Error> {
        let mut s = Main {
            matches,
            config: None,
            root: None,
            quiet: matches.is_present("quiet"),
        };
        s.set_config()?;
        Ok(s)
    }

    fn run(&mut self) -> Result<Exit, Error> {
        if self.matches.subcommand_matches("tidy").is_some() {
            return self.tidy();
        } else if self.matches.subcommand_matches("lint").is_some() {
            return self.lint();
        }

        Ok(Exit {
            status: 1,
            error: Some(String::from(
                "You must run either the tidy or lint subcommand",
            )),
        })
    }

    fn tidy(&mut self) -> Result<Exit, Error> {
        let (mode, _) = self.mode();
        println!("Tidying {}", mode);

        let tidiers = self.config().tidy_filters(&self.root_dir())?;
        if tidiers.is_empty() {
            return Err(MainError::NoTidiers)?;
        }

        let mut status = 0 as i32;
        let (files, dirs) = self.files_and_dirs(&tidiers)?;

        for t in tidiers {
            let p = match t.on_dir {
                true => &dirs,
                false => &files,
            };
            let failures: Vec<i32> = p
                .par_iter()
                .map(|p| -> i32 {
                    match t.tidy(p) {
                        Ok(Some(true)) => {
                            if !self.quiet {
                                println!("Tidied by {}:    {}", t.name, p.to_string_lossy());
                            }
                            0 as i32
                        }
                        Ok(Some(false)) => {
                            if !self.quiet {
                                println!("Unchanged by {}: {}", t.name, p.to_string_lossy());
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
        let (mode, _) = self.mode();
        println!("Linting {}", mode);

        let linters = self.config().lint_filters(&self.root_dir())?;
        if linters.is_empty() {
            return Err(MainError::NoLinters)?;
        }

        let mut status = 0 as i32;
        let (files, dirs) = self.files_and_dirs(&linters)?;

        for l in linters {
            let p = match l.on_dir {
                true => &dirs,
                false => &files,
            };
            let failures: Vec<i32> = p
                .par_iter()
                .map(|p| -> i32 {
                    match l.lint(p.clone()) {
                        Ok(Some(r)) => {
                            if r.ok {
                                if !self.quiet {
                                    println!("Passed {}: {}", l.name, p.to_string_lossy());
                                }
                            } else {
                                println!("Failed {}: {}", l.name, p.to_string_lossy());
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

    fn exit_from_status(status: i32, action: &str) -> Exit {
        let error = if status == 0 {
            None
        } else {
            Some(format!("Error when {} files", action))
        };
        Exit {
            status,
            error: error,
        }
    }
    fn files_and_dirs(
        &self,
        filters: &[filter::Filter],
    ) -> Result<(Vec<PathBuf>, Vec<PathBuf>), Error> {
        let files = self.basepaths()?.paths()?;
        let mut dirs: Vec<PathBuf> = vec![];
        if filters.iter().any(|f| f.on_dir) {
            for p in &files {
                dirs.push(Self::to_dir(p)?);
            }
            dirs.dedup();
        }
        Ok((files, dirs))
    }

    fn to_dir(path: &PathBuf) -> Result<PathBuf, Error> {
        let parent = path.parent();
        if parent.is_some() {
            return Ok(parent.unwrap().to_path_buf());
        }

        Err(MainError::PathHasNoParent {
            path: path.to_string_lossy().to_string(),
        })?
    }

    fn basepaths(&self) -> Result<basepaths::BasePaths, Error> {
        let (mode, paths) = self.mode();
        Ok(basepaths::BasePaths::new(
            mode,
            paths,
            self.root_dir(),
            self.config().ignore_from.as_ref(),
            self.config().exclude.as_ref(),
        )?)
    }

    fn mode(&self) -> (basepaths::Mode, Vec<PathBuf>) {
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
        })?
    }

    fn is_checkout_root(path: &Path) -> bool {
        for dir in vcs::VCS_DIRS {
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
