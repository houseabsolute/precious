use crate::basepaths;
use crate::chars;
use crate::config;
use crate::filter;
use crate::vcs;
use anyhow::{Error, Result};
use clap::{App, Arg, ArgGroup, ArgMatches};
use fern::colors::{Color, ColoredLevelConfig};
use fern::Dispatch;
use log::{debug, error, info};
use rayon::{prelude::*, ThreadPool, ThreadPoolBuilder};
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
enum PreciousError {
    #[error("No subcommand (lint or tidy) was given in the command line args")]
    NoSubcommandInCliArgs,

    #[error("No mode or paths were provided in the command line args")]
    NoModeOrPathsInCliArgs,

    #[error(r#"Could not parse {arg:} argument, "{val:}", as an integer"#)]
    InvalidIntegerArgument { arg: String, val: String },

    #[error("Could not find a VCS checkout root starting from {cwd:}")]
    CannotFindRoot { cwd: String },

    #[error("No {what:} filters defined in your config")]
    NoFilters { what: String },
}

#[derive(Debug)]
struct Exit {
    status: i8,
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
struct ActionError {
    error: String,
    config_key: String,
    path: PathBuf,
}

#[derive(Debug)]
pub struct Precious<'a> {
    matches: &'a ArgMatches,
    mode: basepaths::Mode,
    root: PathBuf,
    cwd: PathBuf,
    config: config::Config,
    chars: chars::Chars,
    quiet: bool,
    thread_pool: ThreadPool,
}

pub fn app<'a>() -> App<'a> {
    App::new("precious")
        .version(env!("CARGO_PKG_VERSION"))
        .author("Dave Rolsky <autarch@urth.org>")
        .about("One code quality tool to rule them all")
        .arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .takes_value(true)
                .help("Path to config file"),
        )
        .arg(
            Arg::new("jobs")
                .short('j')
                .long("jobs")
                .takes_value(true)
                .help("Number of parallel jobs (threads) to run (defaults to one per core)"),
        )
        .arg(
            Arg::new("ascii")
                .long("ascii")
                .help("Replace super-fun Unicode symbols with terribly boring ASCII"),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .help("Enable verbose output"),
        )
        .arg(
            Arg::new("debug")
                .short('d')
                .long("debug")
                .help("Enable debugging output"),
        )
        .arg(
            Arg::new("trace")
                .short('t')
                .long("trace")
                .help("Enable tracing output (maximum logging)"),
        )
        .arg(
            Arg::new("quiet")
                .short('q')
                .long("quiet")
                .help("Suppresses most output"),
        )
        .group(ArgGroup::new("log-level").args(&["verbose", "debug", "trace", "quiet"]))
        .subcommand(common_subcommand(
            "tidy",
            "Tidies the specified files and/or directories",
        ))
        .subcommand(common_subcommand(
            "lint",
            "Lints the specified files and/or directories",
        ))
}

fn common_subcommand<'a>(name: &'a str, about: &'a str) -> App<'a> {
    App::new(name)
        .about(about)
        .arg(
            Arg::new("all")
                .short('a')
                .long("all")
                .help("Run against all files in the current directory and below"),
        )
        .arg(
            Arg::new("git")
                .short('g')
                .long("git")
                .help("Run against files that have been modified according to git"),
        )
        .arg(
            Arg::new("staged")
                .short('s')
                .long("staged")
                .help("Run against file content that is staged for a git commit"),
        )
        .arg(
            Arg::new("paths")
                .multiple_occurrences(true)
                .takes_value(true)
                .help("A list of paths on which to operate"),
        )
        .group(
            ArgGroup::new("operate-on")
                .args(&["all", "git", "staged", "paths"])
                .required(true),
        )
}

pub fn init_logger(matches: &ArgMatches) -> Result<(), log::SetLoggerError> {
    let line_colors = ColoredLevelConfig::new()
        .error(Color::Red)
        .warn(Color::Yellow)
        .info(Color::BrightBlack)
        .debug(Color::BrightBlack)
        .trace(Color::BrightBlack);

    let level = if matches.is_present("trace") {
        log::LevelFilter::Trace
    } else if matches.is_present("debug") {
        log::LevelFilter::Debug
    } else if matches.is_present("verbose") {
        log::LevelFilter::Info
    } else {
        log::LevelFilter::Warn
    };

    let level_colors = line_colors.info(Color::Green).debug(Color::Black);

    Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{color_line}[{target}][{level}{color_line}] {message}\x1B[0m",
                color_line = format_args!(
                    "\x1B[{}m",
                    line_colors.get_color(&record.level()).to_fg_str()
                ),
                target = record.target(),
                level = level_colors.color(record.level()),
                message = message,
            ));
        })
        .level(level)
        .chain(std::io::stderr())
        .apply()
}

impl<'a> Precious<'a> {
    pub fn new(matches: &'a ArgMatches) -> Result<Precious<'a>> {
        if log::log_enabled!(log::Level::Debug) {
            if let Some(path) = env::var_os("PATH") {
                debug!("PATH = {}", path.to_string_lossy());
            }
        }

        let c = if matches.is_present("ascii") {
            chars::BORING_CHARS
        } else {
            chars::FUN_CHARS
        };

        let cwd = env::current_dir()?;
        let root = Self::root(&cwd)?;
        let (config, _) = Self::config(matches, &root)?;

        Ok(Precious {
            matches,
            mode: Self::mode(matches)?,
            config,
            root,
            cwd,
            chars: c,
            quiet: matches.is_present("quiet"),
            thread_pool: ThreadPoolBuilder::new()
                .num_threads(Self::jobs(matches)?)
                .build()?,
        })
    }

    fn mode(matches: &'a ArgMatches) -> Result<basepaths::Mode> {
        match matches.subcommand() {
            Some((_, subc_matches)) => {
                if subc_matches.is_present("all") {
                    return Ok(basepaths::Mode::All);
                } else if subc_matches.is_present("git") {
                    return Ok(basepaths::Mode::GitModified);
                } else if subc_matches.is_present("staged") {
                    return Ok(basepaths::Mode::GitStaged);
                }

                if !subc_matches.is_present("paths") {
                    return Err(PreciousError::NoModeOrPathsInCliArgs.into());
                }

                Ok(basepaths::Mode::FromCli)
            }
            None => Err(PreciousError::NoSubcommandInCliArgs.into()),
        }
    }

    fn jobs(matches: &'a ArgMatches) -> Result<usize> {
        match matches.value_of("jobs") {
            Some(j) => match j.parse::<usize>() {
                Ok(u) => Ok(u),
                Err(_) => Err(PreciousError::InvalidIntegerArgument {
                    arg: "--jobs".to_string(),
                    val: j.to_string(),
                }
                .into()),
            },
            None => Ok(0),
        }
    }

    fn root(cwd: &Path) -> Result<PathBuf> {
        if Self::has_config_file(cwd) {
            return Ok(cwd.into());
        }

        let mut root = PathBuf::new();
        for anc in cwd.ancestors() {
            if Self::is_checkout_root(anc) {
                root.push(anc);
                return Ok(root);
            }
        }

        Err(PreciousError::CannotFindRoot {
            cwd: cwd.to_string_lossy().to_string(),
        }
        .into())
    }

    fn config(matches: &'a ArgMatches, root: &Path) -> Result<(config::Config, PathBuf)> {
        let file = if matches.is_present("config") {
            let conf_file = matches.value_of("config").unwrap();
            debug!("Loading config from {} (set via flag)", conf_file);
            PathBuf::from(conf_file)
        } else {
            let default = Self::default_config_file(root);
            debug!(
                "Loading config from {} (default location)",
                default.to_string_lossy()
            );
            default
        };

        Ok((config::Config::new(file.as_path())?, file))
    }

    fn default_config_file(root: &Path) -> PathBuf {
        let mut file = root.to_path_buf();
        file.push(".precious.toml");
        if !file.exists() {
            file.pop();
            file.push("precious.toml");
        }
        file
    }

    pub fn run(&mut self) -> i8 {
        match self.run_subcommand() {
            Ok(e) => {
                debug!("{:?}", e);
                if let Some(err) = e.error {
                    print!("{}", err);
                }
                if let Some(msg) = e.message {
                    println!("{} {}", self.chars.empty, msg);
                }
                e.status
            }
            Err(e) => {
                error!("Failed to run precious: {}", e);
                1
            }
        }
    }

    fn run_subcommand(&mut self) -> Result<Exit> {
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

    fn tidy(&mut self) -> Result<Exit> {
        println!("{} Tidying {}", self.chars.ring, self.mode);

        let tidiers = self.config.tidy_filters(&self.root)?;
        self.run_all_filters("tidying", tidiers, |s, p, t| s.run_one_tidier(p, t))
    }

    fn lint(&mut self) -> Result<Exit> {
        println!("{} Linting {}", self.chars.ring, self.mode);

        let linters = self.config.lint_filters(&self.root)?;
        self.run_all_filters("linting", linters, |s, p, l| s.run_one_linter(p, l))
    }

    fn run_all_filters<R>(
        &mut self,
        action: &str,
        filters: Vec<filter::Filter>,
        run_filter: R,
    ) -> Result<Exit>
    where
        R: Fn(&mut Self, Vec<basepaths::Paths>, &filter::Filter) -> Option<Vec<ActionError>>,
    {
        if filters.is_empty() {
            return Err(PreciousError::NoFilters {
                what: action.into(),
            }
            .into());
        }

        let cli_paths = match self.mode {
            basepaths::Mode::FromCli => self.paths_from_args(),
            _ => vec![],
        };
        match self.basepaths()?.paths(cli_paths)? {
            None => Ok(self.no_files_exit()),
            Some(paths) => {
                let mut all_errors: Vec<ActionError> = vec![];
                for f in filters {
                    if let Some(mut errors) = run_filter(self, paths.clone(), &f) {
                        all_errors.append(&mut errors);
                    }
                }

                Ok(self.make_exit(all_errors, action))
            }
        }
    }

    fn make_exit(&self, errors: Vec<ActionError>, action: &str) -> Exit {
        let (status, error) = if errors.is_empty() {
            (0, None)
        } else {
            let red = format!("\x1B[{}m", Color::Red.to_fg_str());
            let ansi_off = "\x1B[0m";
            let plural = if errors.len() > 1 { 's' } else { '\0' };

            let error = format!(
                "{}Error{} when {} files:{}\n{}",
                red,
                plural,
                action,
                ansi_off,
                errors
                    .iter()
                    .map(|ae| format!(
                        "  {} {} [{}]\n    {}\n",
                        self.chars.bullet,
                        ae.path.to_string_lossy(),
                        ae.config_key,
                        ae.error,
                    ))
                    .collect::<Vec<String>>()
                    .join("")
            );
            (1, Some(error))
        };
        Exit {
            status,
            message: None,
            error,
        }
    }

    fn run_one_tidier(
        &mut self,
        all_paths: Vec<basepaths::Paths>,
        t: &filter::Filter,
    ) -> Option<Vec<ActionError>> {
        let runner =
            |s: &Self, p: &Path, paths: &basepaths::Paths| -> Option<Result<(), ActionError>> {
                match t.tidy(p, &paths.files) {
                    Ok(Some(true)) => {
                        if !s.quiet {
                            println!(
                                "{} Tidied by {}:    {}",
                                s.chars.tidied,
                                t.name,
                                p.to_string_lossy()
                            );
                        }
                        Some(Ok(()))
                    }
                    Ok(Some(false)) => {
                        if !s.quiet {
                            println!(
                                "{} Unchanged by {}: {}",
                                s.chars.unchanged,
                                t.name,
                                p.to_string_lossy()
                            );
                        }
                        Some(Ok(()))
                    }
                    Ok(None) => None,
                    Err(e) => {
                        println!(
                            "{} error {}: {}",
                            s.chars.execution_error,
                            t.name,
                            p.to_string_lossy()
                        );
                        Some(Err(ActionError {
                            error: format!("{:#}", e),
                            config_key: t.config_key(),
                            path: p.to_owned(),
                        }))
                    }
                }
            };

        self.run_parallel("Tidying", all_paths, t, runner)
    }

    fn run_one_linter(
        &mut self,
        all_paths: Vec<basepaths::Paths>,
        l: &filter::Filter,
    ) -> Option<Vec<ActionError>> {
        let runner =
            |s: &Self, p: &Path, paths: &basepaths::Paths| -> Option<Result<(), ActionError>> {
                match l.lint(p, &paths.files) {
                    Ok(Some(r)) => {
                        if r.ok {
                            if !s.quiet {
                                println!(
                                    "{} Passed {}: {}",
                                    s.chars.lint_free,
                                    l.name,
                                    p.to_string_lossy()
                                );
                            }
                            Some(Ok(()))
                        } else {
                            println!(
                                "{} Failed {}: {}",
                                s.chars.lint_dirty,
                                l.name,
                                p.to_string_lossy()
                            );
                            if let Some(s) = r.stdout {
                                println!("{}", s);
                            }
                            if let Some(s) = r.stderr {
                                println!("{}", s);
                            }

                            Some(Err(ActionError {
                                error: "linting failed".into(),
                                config_key: l.config_key(),
                                path: p.to_owned(),
                            }))
                        }
                    }
                    Ok(None) => None,
                    Err(e) => {
                        println!(
                            "{} error {}: {}",
                            s.chars.execution_error,
                            l.name,
                            p.to_string_lossy()
                        );
                        Some(Err(ActionError {
                            error: format!("{:#}", e),
                            config_key: l.config_key(),
                            path: p.to_owned(),
                        }))
                    }
                }
            };

        self.run_parallel("Linting", all_paths, l, runner)
    }

    fn run_parallel<R>(
        &mut self,
        what: &str,
        all_paths: Vec<basepaths::Paths>,
        f: &filter::Filter,
        runner: R,
    ) -> Option<Vec<ActionError>>
    where
        R: Fn(&Self, &Path, &basepaths::Paths) -> Option<Result<(), ActionError>> + Sync,
    {
        let map = self.path_map(all_paths, f);

        let start = Instant::now();
        let mut results: Vec<Result<(), ActionError>> = vec![];
        self.thread_pool.install(|| {
            results.append(
                &mut map
                    .par_iter()
                    .filter_map(|(p, paths)| runner(self, p, paths))
                    .collect::<Vec<Result<(), ActionError>>>(),
            );
        });

        if !results.is_empty() {
            info!(
                "{} with {} on {} path{}, elapsed time = {}",
                what,
                f.name,
                results.len(),
                if results.len() > 1 { "s" } else { "" },
                format_duration(&start.elapsed())
            );
        }

        let errors = results
            .into_iter()
            .filter_map(|r| match r {
                Ok(_) => None,
                Err(e) => Some(e),
            })
            .collect::<Vec<ActionError>>();
        if errors.is_empty() {
            None
        } else {
            Some(errors)
        }
    }

    fn no_files_exit(&self) -> Exit {
        Exit {
            status: 0,
            message: Some(String::from("No files found")),
            error: None,
        }
    }

    fn path_map(
        &mut self,
        all_paths: Vec<basepaths::Paths>,
        f: &filter::Filter,
    ) -> HashMap<PathBuf, basepaths::Paths> {
        if f.run_mode_is(filter::RunMode::Root) {
            return self.root_as_paths(all_paths);
        } else if f.run_mode_is(filter::RunMode::Dirs) {
            return self.dirs(all_paths);
        }
        self.files(all_paths)
    }

    fn root_as_paths(
        &mut self,
        mut all_paths: Vec<basepaths::Paths>,
    ) -> HashMap<PathBuf, basepaths::Paths> {
        let mut root_map = HashMap::new();

        let mut all: Vec<PathBuf> = vec![];
        for p in all_paths.iter_mut() {
            all.append(&mut p.files);
        }

        let root_paths = basepaths::Paths {
            dir: PathBuf::from("."),
            files: all,
        };
        root_map.insert(PathBuf::from("."), root_paths);
        root_map
    }

    fn dirs(&mut self, all_paths: Vec<basepaths::Paths>) -> HashMap<PathBuf, basepaths::Paths> {
        let mut map = HashMap::new();

        for p in all_paths {
            map.insert(p.dir.clone(), p);
        }
        map
    }

    fn files(&mut self, all_paths: Vec<basepaths::Paths>) -> HashMap<PathBuf, basepaths::Paths> {
        let mut map = HashMap::new();

        for p in all_paths {
            for f in p.files.iter() {
                map.insert(f.clone(), p.clone());
            }
        }
        map
    }

    fn basepaths(&mut self) -> Result<basepaths::BasePaths> {
        basepaths::BasePaths::new(self.mode, self.cwd.clone(), self.config.exclude.clone())
    }

    fn paths_from_args(&self) -> Vec<PathBuf> {
        let subc_matches = self.matched_subcommand();
        subc_matches
            .values_of("paths")
            .unwrap()
            .map(PathBuf::from)
            .collect::<Vec<PathBuf>>()
    }

    fn matched_subcommand(&self) -> &ArgMatches {
        match self.matches.subcommand() {
            Some(("tidy", m)) => m,
            Some(("lint", m)) => m,
            _ => panic!("Somehow none of our subcommands matched and clap did not return an error"),
        }
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

    fn has_config_file(path: &Path) -> bool {
        let mut file = path.to_path_buf();
        file.push(".precious.toml");
        if file.exists() {
            return true;
        }
        file.pop();
        file.push("precious.toml");
        return file.exists();
    }
}

// I tried the humantime crate but it doesn't do what I want. It formats each
// element separately ("1s 243ms 179us 984ns"), which is _way_ more detail
// than I want for this. This algorithm will format to the most appropriate of:
//
//    Xm Y.YYs
//    X.XXs
//    X.XXms
//    X.XXus
//    X.XXns
fn format_duration(d: &Duration) -> String {
    let s = (d.as_secs_f64() * 100.0).round() / 100.0;

    if s >= 60.0 {
        let minutes = (s / 60.0).floor() as u64;
        let secs = s - (minutes as f64 * 60.0);
        return format!("{}m {:.2}s", minutes, secs);
    } else if s >= 0.01 {
        return format!("{:.2}s", s);
    }

    let n = d.as_nanos();
    if n > 1_000_000 {
        return format!("{:.2}ms", n as f64 / 1_000_000.0);
    } else if n > 1_000 {
        return format!("{:.2}us", n as f64 / 1_000.0);
    }

    format!("{}ns", n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testhelper;
    use itertools::Itertools;
    use pretty_assertions::assert_eq;
    // Anything that does pushd must be run serially or else chaos ensues.
    use serial_test::serial;
    use std::path::PathBuf;
    #[cfg(not(target_os = "windows"))]
    use std::str::FromStr;
    #[cfg(not(target_os = "windows"))]
    use which::which;

    const SIMPLE_CONFIG: &str = r#"
[commands.rustfmt]
type    = "both"
include = "**/*.rs"
cmd     = ["rustfmt"]
lint_flags = "--check"
ok_exit_codes = [0]
lint_failure_exit_codes = [1]
"#;

    const CONFIG_FILE_NAME: &str = "precious.toml";

    #[test]
    #[serial]
    fn new() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_config_file(CONFIG_FILE_NAME, SIMPLE_CONFIG)?;
        let _pushd = helper.pushd_to_root()?;

        let app = app();
        let matches = app.try_get_matches_from(&["precious", "tidy", "--all"])?;

        let p = Precious::new(&matches)?;
        assert_eq!(p.chars, chars::FUN_CHARS);
        assert!(!p.quiet);

        let (_, config_file) = Precious::config(&matches, &p.root)?;
        let mut expect_config_file = p.root;
        expect_config_file.push(CONFIG_FILE_NAME);
        assert_eq!(config_file, expect_config_file);

        Ok(())
    }

    #[test]
    #[serial]
    fn new_with_alternate_config_file_name() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_config_file(".precious.toml", SIMPLE_CONFIG)?;
        let _pushd = helper.pushd_to_root()?;

        let app = app();
        let matches = app.try_get_matches_from(&["precious", "tidy", "--all"])?;

        let p = Precious::new(&matches)?;
        assert_eq!(p.chars, chars::FUN_CHARS);
        assert!(!p.quiet);

        let (_, config_file) = Precious::config(&matches, &p.root)?;
        let mut expect_config_file = p.root;
        expect_config_file.push(".precious.toml");
        assert_eq!(config_file, expect_config_file);

        Ok(())
    }

    #[test]
    #[serial]
    fn new_with_ascii_flag() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_config_file(CONFIG_FILE_NAME, SIMPLE_CONFIG)?;
        let _pushd = helper.pushd_to_root()?;

        let app = app();
        let matches = app.try_get_matches_from(&["precious", "--ascii", "tidy", "--all"])?;

        let p = Precious::new(&matches)?;
        assert_eq!(p.chars, chars::BORING_CHARS);

        Ok(())
    }

    #[test]
    #[serial]
    fn new_with_config_path() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_config_file(CONFIG_FILE_NAME, SIMPLE_CONFIG)?;
        let _pushd = helper.pushd_to_root()?;

        let app = app();
        let matches = app.try_get_matches_from(&[
            "precious",
            "--config",
            helper.config_file(CONFIG_FILE_NAME).to_str().unwrap(),
            "tidy",
            "--all",
        ])?;

        let p = Precious::new(&matches)?;

        let (_, config_file) = Precious::config(&matches, &p.root)?;
        let mut expect_config_file = p.root;
        expect_config_file.push(CONFIG_FILE_NAME);
        assert_eq!(config_file, expect_config_file);

        Ok(())
    }

    #[test]
    #[serial]
    fn set_root_prefers_config_file() -> Result<()> {
        let helper = testhelper::TestHelper::new()?.with_git_repo()?;

        let mut src_dir = helper.root();
        src_dir.push("src");
        let mut subdir_config = src_dir.clone();
        subdir_config.push(CONFIG_FILE_NAME);
        helper.write_file(&subdir_config, SIMPLE_CONFIG)?;
        let _pushd = testhelper::Pushd::new(src_dir.clone())?;

        let app = app();
        let matches = app.try_get_matches_from(&["precious", "--quiet", "tidy", "--all"])?;

        let p = Precious::new(&matches)?;
        assert_eq!(p.root, src_dir);

        Ok(())
    }

    #[test]
    #[serial]
    fn basepaths_uses_cwd() -> Result<()> {
        let helper = testhelper::TestHelper::new()?
            .with_config_file(CONFIG_FILE_NAME, SIMPLE_CONFIG)?
            .with_git_repo()?;

        let mut src_dir = helper.root();
        src_dir.push("src");
        let _pushd = testhelper::Pushd::new(src_dir)?;

        let app = app();
        let matches = app.try_get_matches_from(&["precious", "--quiet", "tidy", "--all"])?;

        let mut p = Precious::new(&matches)?;
        let mut paths = p.basepaths()?;

        let expect = vec![basepaths::Paths {
            dir: PathBuf::from("."),
            files: ["bar.rs", "can_ignore.rs", "main.rs", "module.rs"]
                .iter()
                .map(PathBuf::from)
                .collect(),
        }];
        assert_eq!(paths.paths(vec![])?, Some(expect));

        Ok(())
    }

    #[test]
    #[serial]
    fn tidy_succeeds() -> Result<()> {
        let config = r#"
[commands.true]
type    = "tidy"
include = "**/*"
cmd     = ["true"]
ok_exit_codes = [0]
"#;
        let helper = testhelper::TestHelper::new()?.with_config_file(CONFIG_FILE_NAME, config)?;
        let _pushd = helper.pushd_to_root()?;

        let app = app();
        let matches = app.try_get_matches_from(&["precious", "--quiet", "tidy", "--all"])?;

        let mut p = Precious::new(&matches)?;
        let status = p.run();

        assert_eq!(status, 0);

        Ok(())
    }

    #[test]
    #[serial]
    fn tidy_fails() -> Result<()> {
        let config = r#"
[commands.false]
type    = "tidy"
include = "**/*"
cmd     = ["false"]
ok_exit_codes = [0]
"#;
        let helper = testhelper::TestHelper::new()?.with_config_file(CONFIG_FILE_NAME, config)?;
        let _pushd = helper.pushd_to_root()?;

        let app = app();
        let matches = app.try_get_matches_from(&["precious", "--quiet", "tidy", "--all"])?;

        let mut p = Precious::new(&matches)?;
        let status = p.run();

        assert_eq!(status, 1);

        Ok(())
    }

    #[test]
    #[serial]
    fn lint_succeeds() -> Result<()> {
        let config = r#"
[commands.true]
type    = "lint"
include = "**/*"
cmd     = ["true"]
ok_exit_codes = [0]
lint_failure_exit_codes = [1]
"#;
        let helper = testhelper::TestHelper::new()?.with_config_file(CONFIG_FILE_NAME, config)?;
        let _pushd = helper.pushd_to_root()?;

        let app = app();
        let matches = app.try_get_matches_from(&["precious", "--quiet", "lint", "--all"])?;

        let mut p = Precious::new(&matches)?;
        let status = p.run();

        assert_eq!(status, 0);

        Ok(())
    }

    #[test]
    #[serial]
    fn lint_fails() -> Result<()> {
        let config = r#"
[commands.false]
type    = "lint"
include = "**/*"
cmd     = ["false"]
ok_exit_codes = [0]
lint_failure_exit_codes = [1]
"#;
        let helper = testhelper::TestHelper::new()?.with_config_file(CONFIG_FILE_NAME, config)?;
        let _pushd = helper.pushd_to_root()?;

        let app = app();
        let matches = app.try_get_matches_from(&["precious", "--quiet", "lint", "--all"])?;

        let mut p = Precious::new(&matches)?;
        let status = p.run();

        assert_eq!(status, 1);

        Ok(())
    }

    #[test]
    #[serial]
    // This fails in CI on Windows with a confusing error - "Cannot complete
    // in-place edit of test.replace: Work file is missing - did you change
    // directory?" I don't know what this means, and it's not really important
    // to run this test on every OS.
    #[cfg(not(target_os = "windows"))]
    fn command_order_is_preserved_when_running() -> Result<()> {
        if which("perl").is_err() {
            println!("Skipping test since perl is not in path");
            return Ok(());
        }

        let config = r#"
[commands.perl-replace-a-with-b]
type    = "tidy"
include = "test.replace"
cmd     = ["perl", "-pi", "-e", "s/a/b/i"]
ok_exit_codes = [0]

[commands.perl-replace-a-with-c]
type    = "tidy"
include = "test.replace"
cmd     = ["perl", "-pi", "-e", "s/a/c/i"]
ok_exit_codes = [0]
lint_failure_exit_codes = [1]

[commands.perl-replace-a-with-d]
type    = "tidy"
include = "test.replace"
cmd     = ["perl", "-pi", "-e", "s/a/d/i"]
ok_exit_codes = [0]
lint_failure_exit_codes = [1]
"#;
        let helper = testhelper::TestHelper::new()?.with_config_file(CONFIG_FILE_NAME, config)?;
        let test_replace = PathBuf::from_str("test.replace")?;
        helper.write_file(test_replace.as_ref(), "The letter A")?;
        let _pushd = helper.pushd_to_root()?;

        let app = app();
        let matches = app.try_get_matches_from(&["precious", "--quiet", "tidy", "-a"])?;

        let mut p = Precious::new(&matches)?;
        let status = p.run();

        assert_eq!(status, 0);

        let content = helper.read_file(test_replace.as_ref())?;
        assert_eq!(content, "The letter b".to_string());

        Ok(())
    }

    #[test]
    fn format_duration_output() {
        let mut tests: HashMap<Duration, &'static str> = HashMap::new();
        tests.insert(Duration::new(0, 24), "24ns");
        tests.insert(Duration::new(0, 124), "124ns");
        tests.insert(Duration::new(0, 1_243), "1.24us");
        tests.insert(Duration::new(0, 12_443), "12.44us");
        tests.insert(Duration::new(0, 124_439), "124.44us");
        tests.insert(Duration::new(0, 1_244_392), "1.24ms");
        tests.insert(Duration::new(0, 12_443_924), "0.01s");
        tests.insert(Duration::new(0, 124_439_246), "0.12s");
        tests.insert(Duration::new(1, 1), "1.00s");
        tests.insert(Duration::new(1, 12), "1.00s");
        tests.insert(Duration::new(1, 124), "1.00s");
        tests.insert(Duration::new(1, 1_243), "1.00s");
        tests.insert(Duration::new(1, 12_443), "1.00s");
        tests.insert(Duration::new(1, 124_439), "1.00s");
        tests.insert(Duration::new(1, 1_244_392), "1.00s");
        tests.insert(Duration::new(1, 12_443_926), "1.01s");
        tests.insert(Duration::new(1, 124_439_267), "1.12s");
        tests.insert(Duration::new(59, 1), "59.00s");
        tests.insert(Duration::new(59, 1_000_000), "59.00s");
        tests.insert(Duration::new(59, 10_000_000), "59.01s");
        tests.insert(Duration::new(59, 90_000_000), "59.09s");
        tests.insert(Duration::new(59, 99_999_999), "59.10s");
        tests.insert(Duration::new(59, 100_000_000), "59.10s");
        tests.insert(Duration::new(59, 900_000_000), "59.90s");
        tests.insert(Duration::new(59, 990_000_000), "59.99s");
        tests.insert(Duration::new(59, 999_000_000), "1m 0.00s");
        tests.insert(Duration::new(59, 999_999_999), "1m 0.00s");
        tests.insert(Duration::new(60, 0), "1m 0.00s");
        tests.insert(Duration::new(60, 10_000_000), "1m 0.01s");
        tests.insert(Duration::new(60, 100_000_000), "1m 0.10s");
        tests.insert(Duration::new(60, 110_000_000), "1m 0.11s");
        tests.insert(Duration::new(60, 990_000_000), "1m 0.99s");
        tests.insert(Duration::new(60, 999_000_000), "1m 1.00s");
        tests.insert(Duration::new(61, 10_000_000), "1m 1.01s");
        tests.insert(Duration::new(61, 100_000_000), "1m 1.10s");
        tests.insert(Duration::new(61, 120_000_000), "1m 1.12s");
        tests.insert(Duration::new(61, 990_000_000), "1m 1.99s");
        tests.insert(Duration::new(61, 999_000_000), "1m 2.00s");
        tests.insert(Duration::new(120, 99_000_000), "2m 0.10s");
        tests.insert(Duration::new(120, 530_000_000), "2m 0.53s");
        tests.insert(Duration::new(120, 990_000_000), "2m 0.99s");
        tests.insert(Duration::new(152, 240_123_456), "2m 32.24s");

        for k in tests.keys().sorted() {
            let f = format_duration(k);
            let e = tests.get(k).unwrap().to_string();
            assert_eq!(f, e, "{}s {}ns", k.as_secs(), k.as_nanos());
        }
    }
}
