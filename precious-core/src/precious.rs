use crate::{
    chars,
    command::{self, ActualInvoke, TidyOutcome},
    config,
    config_init::{self, InitComponent},
    paths::{self, finder::Finder},
    vcs,
};
use anyhow::{Context, Error, Result};
use clap::{ArgAction, ArgGroup, Parser};
use comfy_table::{presets::UTF8_FULL, Cell, ContentArrangement, Table};
use fern::{
    colors::{Color, ColoredLevelConfig},
    Dispatch,
};
use itertools::Itertools;
use log::{debug, error, info};
use mitsein::prelude::*;
use rayon::{prelude::*, ThreadPool, ThreadPoolBuilder};
use std::{
    env,
    fmt::Write,
    fs,
    io::stdout,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use thiserror::Error;

#[derive(Debug, Error)]
enum PreciousError {
    #[error("No mode or paths were provided in the command line args")]
    NoModeOrPathsInCliArgs,

    #[error("The path given in --config, {}, has no parent directory", file.display())]
    ConfigFileHasNoParent { file: PathBuf },

    #[error("Could not find a VCS checkout root starting from {cwd:}")]
    CannotFindRoot { cwd: String },

    #[error("No {what:} commands defined in your config")]
    NoCommands { what: String },

    #[error("No {what:} commands match the given command name, {name:}")]
    NoCommandsMatchCommandName { what: String, name: String },

    #[error("No {what:} commands match the given label, {label:}")]
    NoCommandsMatchLabel { what: String, label: String },
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
struct ActionFailure {
    error: String,
    config_key: String,
    paths: Vec<PathBuf>,
}

#[derive(Debug, Parser)]
#[clap(name = "precious")]
#[clap(author, version)]
#[clap(propagate_version = true)]
#[clap(subcommand_required = true, arg_required_else_help = true)]
#[clap(max_term_width = 100)]
#[allow(clippy::struct_excessive_bools)]
/// One code quality tool to rule them all
pub struct App {
    /// Path to the precious config file
    #[clap(long, short)]
    config: Option<PathBuf>,
    /// Number of parallel jobs (threads) to run (defaults to one per core)
    #[clap(long, short)]
    jobs: Option<usize>,
    /// Replace super-fun Unicode symbols with terribly boring ASCII
    #[clap(long, global = true)]
    ascii: bool,
    /// Suppresses most output
    #[clap(long, short, global = true)]
    quiet: bool,
    /// Pass this to disable the use of ANSI colors in the output
    #[clap(long = "no-color", action = ArgAction::SetFalse, global = true)]
    color: bool,

    /// Enable verbose output
    #[clap(long, short, global = true)]
    verbose: bool,
    /// Enable debugging output
    #[clap(long, global = true)]
    debug: bool,
    /// Enable tracing output (maximum logging)
    #[clap(long, global = true)]
    trace: bool,

    #[clap(subcommand)]
    subcommand: Subcommand,
}

#[derive(Debug, Parser)]
pub enum Subcommand {
    Lint(CommonArgs),
    #[clap(alias = "fix")]
    Tidy(CommonArgs),
    Config(ConfigArgs),
}

#[derive(Debug, Parser)]
#[clap(group(
    ArgGroup::new("path-spec")
        .required(true)
        .args(&["all", "git", "staged", "git_diff_from", "staged_with_stash", "paths"]),
))]
#[allow(clippy::struct_excessive_bools)]
pub struct CommonArgs {
    /// The command to run. If specified, only this command will be run. This
    /// should match the command name in your config file.
    #[clap(long)]
    command: Option<String>,
    /// Run against all files in the current directory and below
    #[clap(long, short)]
    all: bool,
    /// Run against files that have been modified according to git
    #[clap(long, short)]
    git: bool,
    /// Run against files that are staged for a git commit
    #[clap(long, short)]
    staged: bool,
    /// Run against files that are different as compared with the given
    /// `<REF>`. This can be a branch name, like `master`, or a ref name like
    /// `HEAD~6` or `master@{2.days.ago}`. See `git help rev-parse` for more
    /// options. Note that this will _not_ see files with uncommitted changes
    /// in the local working directory.
    #[clap(long, short = 'd', value_name = "REF")]
    git_diff_from: Option<String>,
    /// Run against file content that is staged for a git commit, stashing all
    /// unstaged content first. The stash push/pop tends to do weird things to
    /// the working directory, and is not recommended for scripting.
    #[clap(long)]
    staged_with_stash: bool,
    /// If this is set, then only commands matching this label will be run. If
    /// this isn't set then commands without a label or with the label
    /// "default" will be run.
    #[clap(long)]
    label: Option<String>,
    /// A list of paths on which to operate
    #[clap(value_parser)]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Parser)]
pub struct ConfigArgs {
    #[clap(subcommand)]
    subcommand: ConfigSubcommand,
}

#[derive(Debug, Parser)]
enum ConfigSubcommand {
    List,
    Init(ConfigInitArgs),
}

#[derive(Debug, Parser)]
#[clap(group(
    ArgGroup::new("components")
        .required(true)
        .args(&["component", "auto"]),
))]
pub struct ConfigInitArgs {
    #[clap(long, short, value_enum)]
    component: Vec<InitComponent>,
    #[clap(long, short)]
    auto: bool,
    #[clap(long, short, default_value = "precious.toml")]
    path: PathBuf,
}

#[must_use]
pub fn app() -> App {
    App::parse()
}

impl App {
    #[allow(clippy::missing_errors_doc)]
    pub fn init_logger(&self) -> Result<(), log::SetLoggerError> {
        let line_colors = ColoredLevelConfig::new()
            .error(Color::Red)
            .warn(Color::Yellow)
            .info(Color::BrightBlack)
            .debug(Color::BrightBlack)
            .trace(Color::BrightBlack);

        let level = if self.trace {
            log::LevelFilter::Trace
        } else if self.debug {
            log::LevelFilter::Debug
        } else if self.verbose {
            log::LevelFilter::Info
        } else {
            log::LevelFilter::Warn
        };

        let use_color = self.color;
        let level_colors = line_colors.info(Color::Green).debug(Color::Black);

        Dispatch::new()
            .format(move |out, message, record| {
                if !use_color {
                    out.finish(format_args!(
                        "[{target}][{level}] {message}",
                        target = record.target(),
                        level = record.level(),
                        message = message,
                    ));
                    return;
                }

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
            .level_for("globset", log::LevelFilter::Info)
            .chain(std::io::stderr())
            .apply()
    }

    #[allow(clippy::missing_errors_doc)]
    pub fn run(self) -> Result<i8> {
        self.run_with_output(stdout())
    }

    fn run_with_output(self, output: impl std::io::Write) -> Result<i8> {
        if let Subcommand::Config(config_args) = &self.subcommand {
            if let ConfigSubcommand::Init(init_args) = &config_args.subcommand {
                config_init::write_config_files(
                    init_args.auto,
                    &init_args.component,
                    &init_args.path,
                )
                .context("Failed to initialize config files")?;
                return Ok(0);
            }
        }

        let (cwd, project_root, config_file, config) = self.load_config()?;

        match self.subcommand {
            Subcommand::Lint(_) | Subcommand::Tidy(_) => {
                Ok(LintOrTidyRunner::new(self, cwd, project_root, config)?.run())
            }
            Subcommand::Config(args) => {
                match args.subcommand {
                    ConfigSubcommand::List => {
                        print_config(output, &config_file, config)
                            .context("Failed to print config")?;
                    }
                    ConfigSubcommand::Init(_) => {
                        unreachable!("This is handled earlier")
                    }
                }

                Ok(0)
            }
        }
    }

    // This exists to make writing tests of the runner easier.
    #[cfg(test)]
    fn new_lint_or_tidy_runner(self) -> Result<LintOrTidyRunner> {
        let (cwd, project_root, _, config) = self.load_config()?;
        LintOrTidyRunner::new(self, cwd, project_root, config)
    }

    fn load_config(&self) -> Result<(PathBuf, PathBuf, PathBuf, config::Config)> {
        let cwd = env::current_dir().context("Failed to get current working directory")?;
        let project_root = project_root(self.config.as_deref(), &cwd)
            .context("Failed to determine project root")?;
        let config_file = self.config_file(&project_root);
        let config = config::Config::new(&config_file)
            .with_context(|| format!("Failed to load config from {}", config_file.display()))?;

        Ok((cwd, project_root, config_file, config))
    }

    fn config_file(&self, dir: &Path) -> PathBuf {
        if let Some(cf) = self.config.as_ref() {
            debug!("Loading config from {} (set via flag)", cf.display());
            return cf.clone();
        }

        let default = default_config_file(dir);
        debug!(
            "Loading config from {} (default location)",
            default.display()
        );
        default
    }
}

fn project_root(config_file: Option<&Path>, cwd: &Path) -> Result<PathBuf> {
    if let Some(file) = config_file {
        if let Some(p) = file.parent() {
            if p.to_string_lossy().is_empty() {
                return Ok(cwd.to_path_buf());
            }
            return fs::canonicalize(p).with_context(|| {
                format!("Canonicalizing config file parent path {}", p.display())
            });
        }
        return Err(PreciousError::ConfigFileHasNoParent {
            file: file.to_path_buf(),
        }
        .into());
    }

    if has_config_file(cwd) {
        return Ok(cwd.into());
    }

    for ancestor in cwd.ancestors() {
        if is_checkout_root(ancestor) {
            return Ok(ancestor.to_owned());
        }
    }

    Err(PreciousError::CannotFindRoot {
        cwd: cwd.to_string_lossy().to_string(),
    }
    .into())
}

fn has_config_file(dir: &Path) -> bool {
    default_config_file(dir).exists()
}

const CONFIG_FILE_NAMES: &[&str] = &["precious.toml", ".precious.toml"];

fn default_config_file(dir: &Path) -> PathBuf {
    // It'd be nicer to use the version of this provided by itertools, but
    // that requires itertools 0.10.1, and we want to keep the version at
    // 0.9.0 for the benefit of Debian.
    find_or_first(
        CONFIG_FILE_NAMES.iter().map(|n| {
            let mut path = dir.to_path_buf();
            path.push(n);
            path
        }),
        Path::exists,
    )
}

fn find_or_first<I, P>(mut iter: I, pred: P) -> PathBuf
where
    I: Iterator<Item = PathBuf>,
    P: Fn(&Path) -> bool,
{
    let first = iter.next().unwrap();
    if pred(&first) {
        return first;
    }
    iter.find(|i| pred(i)).unwrap_or(first)
}

fn is_checkout_root(dir: &Path) -> bool {
    for subdir in vcs::DIRS {
        let mut poss = PathBuf::from(dir);
        poss.push(subdir);
        if poss.exists() {
            return true;
        }
    }
    false
}

fn print_config(
    mut output: impl std::io::Write,
    config_file: &Path,
    config: config::Config,
) -> Result<()> {
    writeln!(output, "Found config file at: {}", config_file.display())?;
    writeln!(output)?;

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Name"),
            Cell::new("Type"),
            Cell::new("Runs"),
        ]);

    for (name, c) in config.command_info() {
        table.add_row(vec![
            Cell::new(name),
            Cell::new(c.typ),
            Cell::new(c.cmd.join(" ")),
        ]);
    }
    writeln!(output, "{table}")?;

    Ok(())
}

#[derive(Debug)]
pub struct LintOrTidyRunner {
    mode: paths::mode::Mode,
    project_root: PathBuf,
    cwd: PathBuf,
    config: config::Config,
    command: Option<String>,
    chars: chars::Chars,
    quiet: bool,
    color: bool,
    thread_pool: ThreadPool,
    should_lint: bool,
    paths: Vec<PathBuf>,
    label: Option<String>,
}

macro_rules! maybe_println {
    ($self:expr, $($arg:tt)*) => {
        if !$self.quiet {
            println!("{}", format!($($arg)*))
        }
    };
}

impl LintOrTidyRunner {
    fn new(
        app: App,
        cwd: PathBuf,
        project_root: PathBuf,
        config: config::Config,
    ) -> Result<LintOrTidyRunner> {
        if log::log_enabled!(log::Level::Debug) {
            if let Some(path) = env::var_os("PATH") {
                debug!("PATH = {}", path.to_string_lossy());
            }
        }

        let c = if app.ascii {
            chars::BORING_CHARS
        } else {
            chars::FUN_CHARS
        };

        let mode = Self::mode(&app)?;
        let quiet = app.quiet;
        let jobs = app.jobs.unwrap_or_default();
        let (should_lint, paths, command, label) = match app.subcommand {
            Subcommand::Lint(a) => (true, a.paths, a.command, a.label),
            Subcommand::Tidy(a) => (false, a.paths, a.command, a.label),
            Subcommand::Config(_) => unreachable!("this is handled in App::run"),
        };

        Ok(LintOrTidyRunner {
            mode,
            project_root,
            cwd,
            config,
            command,
            chars: c,
            quiet,
            color: app.color,
            thread_pool: ThreadPoolBuilder::new().num_threads(jobs).build()?,
            should_lint,
            paths,
            label,
        })
    }

    fn mode(app: &App) -> Result<paths::mode::Mode> {
        let common = match &app.subcommand {
            Subcommand::Lint(c) | Subcommand::Tidy(c) => c,
            Subcommand::Config(_) => unreachable!("this is handled in App::run"),
        };
        if common.all {
            return Ok(paths::mode::Mode::All);
        } else if common.git {
            return Ok(paths::mode::Mode::GitModified);
        } else if common.staged {
            return Ok(paths::mode::Mode::GitStaged);
        } else if let Some(from) = &common.git_diff_from {
            return Ok(paths::mode::Mode::GitDiffFrom(from.clone()));
        } else if common.staged_with_stash {
            return Ok(paths::mode::Mode::GitStagedWithStash);
        }

        if common.paths.is_empty() {
            return Err(PreciousError::NoModeOrPathsInCliArgs.into());
        }
        Ok(paths::mode::Mode::FromCli)
    }

    fn run(&mut self) -> i8 {
        match self.run_subcommand() {
            Ok(e) => {
                debug!("{e:?}");
                if let Some(e) = e.error {
                    print!("{e:?}");
                }
                if let Some(msg) = e.message {
                    println!("{} {}", self.chars.empty, msg);
                }
                e.status
            }
            Err(e) => {
                error!("Failed to run precious: {e:?}");
                42
            }
        }
    }

    fn run_subcommand(&mut self) -> Result<Exit> {
        if self.should_lint {
            self.lint()
        } else {
            self.tidy()
        }
    }

    fn tidy(&mut self) -> Result<Exit> {
        maybe_println!(self, "{} Tidying {}", self.chars.ring, self.mode);

        let tidiers = self
            .config
            // XXX - This clone can be removed if config is passed into this
            // method instead of being a field of self.
            .clone()
            .into_tidy_commands(
                &self.project_root,
                self.command.as_deref(),
                self.label.as_deref(),
            )
            .context("Failed to get tidy commands from config")?;
        self.run_all_commands("tidying", tidiers, Self::run_one_tidier)
    }

    fn lint(&mut self) -> Result<Exit> {
        maybe_println!(self, "{} Linting {}", self.chars.ring, self.mode);

        let linters = self
            .config
            // XXX - same as above.
            .clone()
            .into_lint_commands(
                &self.project_root,
                self.command.as_deref(),
                self.label.as_deref(),
            )
            .context("Failed to get lint commands from config")?;
        self.run_all_commands("linting", linters, Self::run_one_linter)
    }

    fn run_all_commands<R>(
        &mut self,
        action: &str,
        commands: Vec<command::Command>,
        run_command: R,
    ) -> Result<Exit>
    where
        R: Fn(&mut Self, &Slice1<PathBuf>, &command::Command) -> Result<Option<Vec<ActionFailure>>>,
    {
        if commands.is_empty() {
            if let Some(c) = &self.command {
                return Err(PreciousError::NoCommandsMatchCommandName {
                    what: action.into(),
                    name: c.into(),
                }
                .into());
            }
            if let Some(l) = &self.label {
                return Err(PreciousError::NoCommandsMatchLabel {
                    what: action.into(),
                    label: l.into(),
                }
                .into());
            }
            return Err(PreciousError::NoCommands {
                what: action.into(),
            }
            .into());
        }

        let cli_paths = match self.mode {
            paths::mode::Mode::FromCli => self.paths.clone(),
            _ => vec![],
        };

        let files = self
            .finder()
            .context("Failed to create file finder")?
            .files(cli_paths)
            .with_context(|| format!("Failed to find files for {action}"))?;

        match files {
            None => Ok(Self::no_files_exit()),
            Some(files) => {
                let mut all_failures: Vec<ActionFailure> = vec![];
                for c in commands {
                    debug!(r"Command config for {}: {}", c.name, c.config_debug());
                    if let Some(mut failures) =
                        run_command(self, &files, &c).with_context(|| {
                            format!(r#"Failed to run command "{}" for {action}"#, c.name)
                        })?
                    {
                        all_failures.append(&mut failures);
                    }
                }

                Ok(self.make_exit(&all_failures, action))
            }
        }
    }

    fn finder(&mut self) -> Result<Finder> {
        Finder::new(
            self.mode.clone(),
            &self.project_root,
            self.cwd.clone(),
            self.config.exclude.clone(),
        )
    }

    fn make_exit(&self, failures: &[ActionFailure], action: &str) -> Exit {
        let (status, error) = if failures.is_empty() {
            (0, None)
        } else {
            let (red, ansi_off) = if self.color {
                (format!("\x1B[{}m", Color::Red.to_fg_str()), "\x1B[0m")
            } else {
                (String::new(), "")
            };
            let plural = if failures.len() > 1 { 's' } else { '\0' };

            let error = format!(
                "{}Error{} when {} files:{}\n{}",
                red,
                plural,
                action,
                ansi_off,
                failures.iter().fold(String::new(), |mut out, af| {
                    let _ = write!(
                        out,
                        "  {} [{}] failed for [{}]\n    {}\n",
                        self.chars.bullet,
                        af.config_key,
                        af.paths.iter().map(|p| p.to_string_lossy()).join(" "),
                        af.error,
                    );
                    out
                }),
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
        files: &Slice1<PathBuf>,
        t: &command::Command,
    ) -> Result<Option<Vec<ActionFailure>>> {
        let runner = |s: &Self,
                      actual_invoke: ActualInvoke,
                      files: &Slice1<&Path>|
         -> Option<Result<(), ActionFailure>> {
            match t.tidy(actual_invoke, files) {
                Ok(Some(TidyOutcome::Changed)) => {
                    maybe_println!(
                        s,
                        "{} Tidied by {}:    {}",
                        s.chars.tidied,
                        t.name,
                        t.paths_summary(actual_invoke, files),
                    );
                    Some(Ok(()))
                }
                Ok(Some(TidyOutcome::Unchanged)) => {
                    maybe_println!(
                        s,
                        "{} Unchanged by {}: {}",
                        s.chars.unchanged,
                        t.name,
                        t.paths_summary(actual_invoke, files),
                    );
                    Some(Ok(()))
                }
                Ok(Some(TidyOutcome::Unknown)) => {
                    maybe_println!(
                        s,
                        "{} Maybe changed by {}: {}",
                        s.chars.unknown,
                        t.name,
                        t.paths_summary(actual_invoke, files),
                    );
                    Some(Ok(()))
                }
                Ok(None) => None,
                Err(e) => {
                    println!(
                        "{} Error from {}: {}",
                        s.chars.execution_error,
                        t.name,
                        t.paths_summary(actual_invoke, files),
                    );
                    Some(Err(ActionFailure {
                        error: format!("{e:#}"),
                        config_key: t.config_key(),
                        paths: files.iter().map(|f| f.to_path_buf()).collect(),
                    }))
                }
            }
        };

        self.run_parallel("Tidying", files, t, runner)
    }

    fn run_one_linter(
        &mut self,
        files: &Slice1<PathBuf>,
        l: &command::Command,
    ) -> Result<Option<Vec<ActionFailure>>> {
        let runner = |s: &Self,
                      actual_invoke: ActualInvoke,
                      files: &Slice1<&Path>|
         -> Option<Result<(), ActionFailure>> {
            match l.lint(actual_invoke, files) {
                Ok(Some(lo)) => {
                    if lo.ok {
                        maybe_println!(
                            s,
                            "{} Passed {}: {}",
                            s.chars.lint_free,
                            l.name,
                            l.paths_summary(actual_invoke, files),
                        );
                        Some(Ok(()))
                    } else {
                        println!(
                            "{} Failed {}: {}",
                            s.chars.lint_dirty,
                            l.name,
                            l.paths_summary(actual_invoke, files),
                        );
                        if let Some(s) = lo.stdout {
                            println!("{s}");
                        }
                        if let Some(s) = lo.stderr {
                            println!("{s}");
                        }
                        if let Ok(ga) = env::var("GITHUB_ACTIONS") {
                            if !ga.is_empty() {
                                if files.len() == NonZeroUsize::new(1).unwrap() {
                                    println!(
                                        "::error file={}::Linting with {} failed",
                                        files[0].display(),
                                        l.name
                                    );
                                } else {
                                    println!("::error::Linting with {} failed", l.name);
                                }
                            }
                        }

                        Some(Err(ActionFailure {
                            error: "linting failed".into(),
                            config_key: l.config_key(),
                            paths: files.iter().map(|f| f.to_path_buf()).collect(),
                        }))
                    }
                }
                Ok(None) => None,
                Err(e) => {
                    println!(
                        "{} error {}: {}",
                        s.chars.execution_error,
                        l.name,
                        l.paths_summary(actual_invoke, files),
                    );
                    Some(Err(ActionFailure {
                        error: format!("{e:#}"),
                        config_key: l.config_key(),
                        paths: files.iter().map(|f| f.to_path_buf()).collect(),
                    }))
                }
            }
        };

        self.run_parallel("Linting", files, l, runner)
    }

    fn run_parallel<R>(
        &mut self,
        what: &str,
        files: &Slice1<PathBuf>,
        c: &command::Command,
        runner: R,
    ) -> Result<Option<Vec<ActionFailure>>>
    where
        R: Fn(&Self, ActualInvoke, &Slice1<&Path>) -> Option<Result<(), ActionFailure>> + Sync,
    {
        let (sets, actual_invoke) = c.files_to_args_sets(files).with_context(|| {
            format!(
                r#"Failed to prepare file argument sets for command "{}""#,
                c.name
            )
        })?;

        let start = Instant::now();
        let results = self
            .thread_pool
            .install(|| -> Result<Vec<Result<(), ActionFailure>>> {
                let mut res: Vec<Result<(), ActionFailure>> = vec![];
                res.append(
                    &mut sets
                        .into_par_iter()
                        .filter_map(|set| runner(self, actual_invoke, &set))
                        .collect::<Vec<Result<(), ActionFailure>>>(),
                );
                Ok(res)
            })?;

        if !results.is_empty() {
            info!(
                "{} with {} on {} path{}, elapsed time = {}",
                what,
                c.name,
                results.len(),
                if results.len() > 1 { "s" } else { "" },
                format_duration(&start.elapsed())
            );
        }

        let failures = results
            .into_iter()
            .filter_map(Result::err)
            .collect::<Vec<ActionFailure>>();
        if failures.is_empty() {
            Ok(None)
        } else {
            Ok(Some(failures))
        }
    }

    fn no_files_exit() -> Exit {
        Exit {
            status: 0,
            message: Some(String::from("No files found")),
            error: None,
        }
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
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn format_duration(d: &Duration) -> String {
    let s = (d.as_secs_f64() * 100.0).round() / 100.0;

    if s >= 60.0 {
        let minutes = (s / 60.0).floor() as u64;
        let secs = s - (minutes as f64 * 60.0);
        return format!("{minutes}m {secs:.2}s");
    } else if s >= 0.01 {
        return format!("{s:.2}s");
    }

    let n = d.as_nanos();
    if n > 1_000_000 {
        return format!("{:.2}ms", n as f64 / 1_000_000.0);
    } else if n > 1_000 {
        return format!("{:.2}us", n as f64 / 1_000.0);
    }

    format!("{n}ns")
}

#[cfg(test)]
mod tests {
    use super::*;
    use itertools::Itertools;
    use precious_testhelper::TestHelper;
    use pretty_assertions::assert_eq;
    use pushd::Pushd;
    // Anything that does pushd must be run serially or else chaos ensues.
    use serial_test::serial;
    #[cfg(not(target_os = "windows"))]
    use std::str::FromStr;
    use std::{collections::HashMap, path::PathBuf};
    use test_case::test_case;
    #[cfg(not(target_os = "windows"))]
    use which::which;

    const SIMPLE_CONFIG: &str = r#"
[commands.rustfmt]
type    = "both"
include = "**/*.rs"
cmd     = ["rustfmt"]
lint-flags = "--check"
ok-exit-codes = [0]
lint-failure-exit-codes = [1]
"#;

    const DEFAULT_CONFIG_FILE_NAME: &str = super::CONFIG_FILE_NAMES[0];

    #[test]
    #[serial]
    fn new() -> Result<()> {
        for name in super::CONFIG_FILE_NAMES {
            let helper = TestHelper::new()?.with_config_file(name, SIMPLE_CONFIG)?;
            let _pushd = helper.pushd_to_git_root()?;

            let app = App::try_parse_from(["precious", "tidy", "--all"])?;

            let (_, project_root, config_file, _) = app.load_config()?;
            let mut expect_config_file = project_root;
            expect_config_file.push(name);
            assert_eq!(config_file, expect_config_file);
        }

        Ok(())
    }

    #[test]
    #[serial]
    fn new_with_ascii_flag() -> Result<()> {
        let helper =
            TestHelper::new()?.with_config_file(DEFAULT_CONFIG_FILE_NAME, SIMPLE_CONFIG)?;
        let _pushd = helper.pushd_to_git_root()?;

        let app = App::try_parse_from(["precious", "--ascii", "tidy", "--all"])?;

        let lt = app.new_lint_or_tidy_runner()?;
        assert_eq!(lt.chars, chars::BORING_CHARS);

        Ok(())
    }

    #[test]
    #[serial]
    fn new_with_config_path() -> Result<()> {
        let helper =
            TestHelper::new()?.with_config_file(DEFAULT_CONFIG_FILE_NAME, SIMPLE_CONFIG)?;
        let _pushd = helper.pushd_to_git_root()?;

        let app = App::try_parse_from([
            "precious",
            "--config",
            helper
                .config_file(DEFAULT_CONFIG_FILE_NAME)
                .to_str()
                .unwrap(),
            "tidy",
            "--all",
        ])?;

        let (_, project_root, config_file, _) = app.load_config()?;
        let mut expect_config_file = project_root;
        expect_config_file.push(DEFAULT_CONFIG_FILE_NAME);
        assert_eq!(config_file, expect_config_file);

        Ok(())
    }

    #[test]
    #[serial]
    fn set_root_prefers_config_file() -> Result<()> {
        let helper = TestHelper::new()?.with_git_repo()?;

        let mut src_dir = helper.precious_root();
        src_dir.push("src");
        let mut subdir_config = src_dir.clone();
        subdir_config.push(DEFAULT_CONFIG_FILE_NAME);
        helper.write_file(&subdir_config, SIMPLE_CONFIG)?;
        let _pushd = Pushd::new(src_dir.clone())?;

        let app = App::try_parse_from(["precious", "--quiet", "tidy", "--all"])?;

        let lt = app.new_lint_or_tidy_runner()?;
        assert_eq!(lt.project_root, src_dir);

        Ok(())
    }

    #[test_case(None, true; "no config file specified, in project root")]
    #[test_case(None, false; "no config file specified, in subdir")]
    #[test_case(Some("precious.toml"), true; "precious.toml in project root")]
    #[test_case(Some("./precious.toml"), true; "./precious.toml in project root")]
    #[test_case(Some("../precious.toml"), false; "../precious.toml in subdir")]
    #[serial]
    fn project_root(config_file: Option<&str>, in_project_root: bool) -> Result<()> {
        let helper = TestHelper::new()?.with_git_repo()?;
        let _pushd = if in_project_root {
            helper.pushd_to_git_root()?
        } else {
            helper.pushd_to_subdir()?
        };

        let cwd = env::current_dir()?;

        let root = super::project_root(config_file.map(Path::new), &cwd)?;
        assert_eq!(root, helper.precious_root());

        Ok(())
    }

    type FinderTestAction = Box<dyn Fn(&TestHelper) -> Result<()>>;

    #[test_case(
        "--all",
        &[],
        Box::new(|_| Ok(())),
        &vec1![
            "README.md",
            "can_ignore.x",
            "merge-conflict-file",
            "precious.toml",
            "src/bar.rs",
            "src/can_ignore.rs",
            "src/main.rs",
            "src/module.rs",
            "src/sub/mod.rs",
            "tests/data/bar.txt",
            "tests/data/foo.txt",
            "tests/data/generated.txt",
        ] ;
        "--all"
    )]
    #[test_case(
        "--git",
        &[],
        Box::new(|th| {
            th.modify_files()?;
            Ok(())
        }),
        &vec1!["src/module.rs", "tests/data/foo.txt"] ;
        "--git"
    )]
    #[test_case(
        "--staged",
        &[],
        Box::new(|th| {
            th.modify_files()?;
            th.stage_all()?;
            Ok(())
        }),
        &vec1!["src/module.rs", "tests/data/foo.txt"] ;
        "--staged"
    )]
    #[test_case(
        "",
        &["main.rs", "module.rs"],
        Box::new(|_| Ok(())),
        &vec1!["src/main.rs", "src/module.rs"] ;
        "file paths from cli"
    )]
    #[test_case(
        "",
        &["."],
        Box::new(|_| Ok(())),
        &vec1![
            "src/bar.rs",
            "src/can_ignore.rs",
            "src/main.rs",
            "src/module.rs",
            "src/sub/mod.rs",
        ] ;
        "dir paths from cli"
    )]
    #[serial]
    fn finder_uses_project_root(
        flag: &str,
        paths: &[&str],
        action: FinderTestAction,
        expect: &Slice1<&str>,
    ) -> Result<()> {
        let helper = TestHelper::new()?
            .with_config_file(DEFAULT_CONFIG_FILE_NAME, SIMPLE_CONFIG)?
            .with_git_repo()?;
        action(&helper)?;

        let mut src_dir = helper.precious_root();
        src_dir.push("src");
        let _pushd = Pushd::new(src_dir)?;

        let mut cmd = vec!["precious", "--quiet", "tidy"];
        if !flag.is_empty() {
            cmd.push(flag);
        } else {
            cmd.append(&mut paths.to_vec());
        }
        let app = App::try_parse_from(&cmd)?;

        let mut lt = app.new_lint_or_tidy_runner()?;

        assert_eq!(
            lt.finder()?
                .files(paths.iter().map(PathBuf::from).collect())?,
            Some(expect.iter1().map(PathBuf::from).collect1()),
            "finder_uses_project_root: {} [{}]",
            if flag.is_empty() { "<none>" } else { flag },
            paths.join(" ")
        );

        Ok(())
    }

    #[test]
    #[serial]
    #[cfg(not(target_os = "windows"))]
    fn tidy_succeeds() -> Result<()> {
        let config = r#"
    [commands.precious]
    type    = "tidy"
    include = "**/*"
    cmd     = ["true"]
    ok-exit-codes = [0]
    "#;
        let helper = TestHelper::new()?.with_config_file(DEFAULT_CONFIG_FILE_NAME, config)?;
        let _pushd = helper.pushd_to_git_root()?;

        let app = App::try_parse_from(["precious", "--quiet", "tidy", "--all"])?;

        let mut lt = app.new_lint_or_tidy_runner()?;
        let status = lt.run();

        assert_eq!(status, 0);

        Ok(())
    }

    #[test]
    #[serial]
    #[cfg(not(target_os = "windows"))]
    fn tidy_fails() -> Result<()> {
        let config = r#"
    [commands.false]
    type    = "tidy"
    include = "**/*"
    cmd     = ["false"]
    ok-exit-codes = [0]
    "#;
        let helper = TestHelper::new()?.with_config_file(DEFAULT_CONFIG_FILE_NAME, config)?;
        let _pushd = helper.pushd_to_git_root()?;

        let app = App::try_parse_from(["precious", "--quiet", "tidy", "--all"])?;

        let mut lt = app.new_lint_or_tidy_runner()?;
        let status = lt.run();

        assert_eq!(status, 1);

        Ok(())
    }

    #[test]
    #[serial]
    #[cfg(not(target_os = "windows"))]
    fn lint_succeeds() -> Result<()> {
        let config = r#"
    [commands.true]
    type    = "lint"
    include = "**/*"
    cmd     = ["true"]
    ok-exit-codes = [0]
    lint-failure-exit-codes = [1]
    "#;
        let helper = TestHelper::new()?.with_config_file(DEFAULT_CONFIG_FILE_NAME, config)?;
        let _pushd = helper.pushd_to_git_root()?;

        let app = App::try_parse_from(["precious", "--quiet", "lint", "--all"])?;

        let mut lt = app.new_lint_or_tidy_runner()?;
        let status = lt.run();

        assert_eq!(status, 0);

        Ok(())
    }

    #[test]
    #[serial]
    fn one_command_given() -> Result<()> {
        let helper =
            TestHelper::new()?.with_config_file(DEFAULT_CONFIG_FILE_NAME, SIMPLE_CONFIG)?;
        let _pushd = helper.pushd_to_git_root()?;

        let app = App::try_parse_from([
            "precious",
            "--quiet",
            "lint",
            "--command",
            "rustfmt",
            "--all",
        ])?;

        let mut lt = app.new_lint_or_tidy_runner()?;
        let status = lt.run();

        assert_eq!(status, 0);

        Ok(())
    }

    #[test]
    #[serial]
    fn one_command_given_which_does_not_exist() -> Result<()> {
        let helper =
            TestHelper::new()?.with_config_file(DEFAULT_CONFIG_FILE_NAME, SIMPLE_CONFIG)?;
        let _pushd = helper.pushd_to_git_root()?;

        let app = App::try_parse_from([
            "precious",
            "--quiet",
            "lint",
            "--command",
            "no-such-command",
            "--all",
        ])?;

        let mut lt = app.new_lint_or_tidy_runner()?;
        let status = lt.run();

        assert_eq!(status, 42);

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
            ok-exit-codes = [0]

            [commands.perl-replace-a-with-c]
            type    = "tidy"
            include = "test.replace"
            cmd     = ["perl", "-pi", "-e", "s/a/c/i"]
            ok-exit-codes = [0]
            lint-failure-exit-codes = [1]

            [commands.perl-replace-a-with-d]
            type    = "tidy"
            include = "test.replace"
            cmd     = ["perl", "-pi", "-e", "s/a/d/i"]
            ok-exit-codes = [0]
            lint-failure-exit-codes = [1]
        "#;
        let helper = TestHelper::new()?.with_config_file(DEFAULT_CONFIG_FILE_NAME, config)?;
        let test_replace = PathBuf::from_str("test.replace")?;
        helper.write_file(&test_replace, "The letter A")?;
        let _pushd = helper.pushd_to_git_root()?;

        let app = App::try_parse_from(["precious", "--quiet", "tidy", "-a"])?;

        let status = app.run()?;

        assert_eq!(status, 0);

        let content = helper.read_file(test_replace.as_ref())?;
        assert_eq!(content, "The letter b".to_string());

        Ok(())
    }

    #[test]
    #[serial]
    fn print_config() -> Result<()> {
        let config = r#"
            [commands.foo]
            type    = "lint"
            include = "*.foo"
            cmd     = ["foo", "--lint", "--with-vigor"]
            ok-exit-codes = [0]

            [commands.bar]
            type    = "tidy"
            include = "*.bar"
            cmd     = ["bar", "--fix-broken-things", "--aggressive"]
            ok-exit-codes = [0]

            [commands.baz]
            type    = "both"
            include = "*.baz"
            cmd     = ["baz", "--fast-mode", "--no-verify"]
            lint-flags = "--lint"
            ok-exit-codes = [0]
        "#;
        let helper = TestHelper::new()?.with_config_file(DEFAULT_CONFIG_FILE_NAME, config)?;
        let _pushd = helper.pushd_to_git_root()?;

        let app = App::try_parse_from(["precious", "config", "list"])?;
        let mut buffer = Vec::new();
        let status = app.run_with_output(&mut buffer)?;

        assert_eq!(status, 0);

        let output = String::from_utf8(buffer)?;
        let expect = format!(
            r#"Found config file at: {}

┌──────┬──────┬──────────────────────────────────────┐
│ Name ┆ Type ┆ Runs                                 │
╞══════╪══════╪══════════════════════════════════════╡
│ foo  ┆ lint ┆ foo --lint --with-vigor              │
├╌╌╌╌╌╌┼╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ bar  ┆ tidy ┆ bar --fix-broken-things --aggressive │
├╌╌╌╌╌╌┼╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ baz  ┆ both ┆ baz --fast-mode --no-verify          │
└──────┴──────┴──────────────────────────────────────┘
"#,
            helper.config_file(DEFAULT_CONFIG_FILE_NAME).display(),
        );
        assert_eq!(output, expect);

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
