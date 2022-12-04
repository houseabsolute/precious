use crate::{
    chars::{self, Chars},
    command::{self, TidyOutcome},
    config,
    exit::Exit,
    output::{Event, OutputWriter, UnstructuredTextWriter},
    paths::{self, finder::Finder},
    vcs,
};
use anyhow::Result;
use clap::{ArgGroup, Parser};
use crossbeam::{
    channel::{unbounded, Receiver, RecvError, Sender},
    thread,
};
use fern::{
    colors::{Color, ColoredLevelConfig},
    Dispatch,
};
use itertools::Itertools;
use log::{debug, error, info};
use rayon::{prelude::*, ThreadPool, ThreadPoolBuilder};
use std::{
    env,
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
    NoCommandsMatch { what: String, name: String },

    #[error(transparent)]
    CrossbeamRecv(#[from] RecvError),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

const CONFIG_FILE_NAMES: &[&str] = &["precious.toml", ".precious.toml"];

#[derive(Debug, Parser)]
#[clap(name = "precious")]
#[clap(author, version)]
#[clap(propagate_version = true)]
#[clap(subcommand_required = true, arg_required_else_help = true)]
/// One code quality tool to rule them all
pub struct App {
    /// Path to the precious config file
    #[clap(long, short)]
    config: Option<PathBuf>,
    /// Number of parallel jobs (threads) to run (defaults to one per core)
    #[clap(long, short, default_value_t = rayon::current_num_threads())]
    jobs: usize,
    /// Output format
    #[clap(long, short, value_enum, default_value_t = OutputFormat::Unstructured)]
    output_format: OutputFormat,
    /// Replace super-fun Unicode symbols with terribly boring ASCII
    #[clap(long, short)]
    ascii: bool,
    /// Suppresses most output
    #[clap(long, short)]
    quiet: bool,
    /// Enable verbose output
    #[clap(long, short)]
    verbose: bool,
    /// Enable debugging output
    #[clap(long, short)]
    debug: bool,
    /// Enable tracing output (maximum logging)
    #[clap(long, short)]
    trace: bool,
    #[clap(subcommand)]
    subcommand: Subcommand,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum OutputFormat {
    Unstructured,
}

#[derive(Debug, Parser)]
pub enum Subcommand {
    Lint(CommonArgs),
    Tidy(CommonArgs),
}

#[derive(Debug, Parser)]
#[clap(group(
    ArgGroup::new("path-spec")
        .required(true)
        .args(&["all", "git", "staged", "staged_with_stash", "paths"]),
))]
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
    /// Run against file content that is staged for a git commit, stashing all
    /// unstaged content first
    #[clap(long)]
    staged_with_stash: bool,
    /// A list of paths on which to operate
    #[clap(value_parser)]
    paths: Vec<PathBuf>,
}

impl Default for App {
    fn default() -> Self {
        Self::parse()
    }
}

impl App {
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
            .level_for("globset", log::LevelFilter::Info)
            .chain(std::io::stderr())
            .apply()
    }

    pub fn into_precious(self) -> Result<Precious> {
        Precious::new(self)
    }
}

#[derive(Debug)]
struct ActionFailure {
    error: String,
    config_key: String,
    paths: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct Precious {
    mode: paths::mode::Mode,
    project_root: PathBuf,
    cwd: PathBuf,
    config: config::Config,
    command: Option<String>,
    thread_pool: ThreadPool,
    should_lint: bool,
    paths: Vec<PathBuf>,
    output_format: OutputFormat,
    chars: &'static Chars,
    quiet: bool,
}

impl Precious {
    fn new(app: App) -> Result<Self> {
        if log::log_enabled!(log::Level::Debug) {
            if let Some(path) = env::var_os("PATH") {
                debug!("PATH = {}", path.to_string_lossy());
            }
        }

        let mode = Self::mode(&app)?;
        let cwd = env::current_dir()?;
        let project_root = Self::project_root(app.config.as_ref(), &cwd)?;
        let config_file = Self::config_file(app.config.as_ref(), &project_root);
        let config = config::Config::new(config_file)?;
        let jobs = app.jobs;
        let (should_lint, paths, command) = match app.subcommand {
            Subcommand::Lint(a) => (true, a.paths, a.command),
            Subcommand::Tidy(a) => (false, a.paths, a.command),
        };

        let c = if app.ascii {
            &chars::BORING_CHARS
        } else {
            &chars::FUN_CHARS
        };

        Ok(Precious {
            mode,
            project_root,
            cwd,
            config,
            command,
            thread_pool: ThreadPoolBuilder::new().num_threads(jobs).build()?,
            should_lint,
            paths,
            chars: c,
            quiet: app.quiet,
            output_format: app.output_format,
        })
    }

    fn mode(app: &App) -> Result<paths::mode::Mode> {
        let common = match &app.subcommand {
            Subcommand::Lint(c) => c,
            Subcommand::Tidy(c) => c,
        };
        if common.all {
            return Ok(paths::mode::Mode::All);
        } else if common.git {
            return Ok(paths::mode::Mode::GitModified);
        } else if common.staged {
            return Ok(paths::mode::Mode::GitStaged);
        } else if common.staged_with_stash {
            return Ok(paths::mode::Mode::GitStagedWithStash);
        }
        if common.paths.is_empty() {
            return Err(PreciousError::NoModeOrPathsInCliArgs.into());
        }
        Ok(paths::mode::Mode::FromCli)
    }

    fn project_root(file: Option<&PathBuf>, cwd: &Path) -> Result<PathBuf> {
        if let Some(file) = file {
            if let Some(p) = file.parent() {
                return Ok(p.to_path_buf());
            }
            return Err(PreciousError::ConfigFileHasNoParent { file: file.clone() }.into());
        }

        if Self::has_config_file(cwd) {
            return Ok(cwd.into());
        }

        for anc in cwd.ancestors() {
            if Self::is_checkout_root(anc) {
                return Ok(anc.to_owned());
            }
        }

        Err(PreciousError::CannotFindRoot {
            cwd: cwd.to_string_lossy().to_string(),
        }
        .into())
    }

    fn has_config_file(dir: &Path) -> bool {
        Self::default_config_file(dir).exists()
    }

    fn default_config_file(dir: &Path) -> PathBuf {
        // It'd be nicer to use the version of this provided by itertools, but
        // that requires itertools 0.10.1, and we want to keep the version at
        // 0.9.0 for the benefit of Debian.
        Self::find_or_first(
            CONFIG_FILE_NAMES.iter().map(|n| {
                let mut path = dir.to_path_buf();
                path.push(n);
                path
            }),
            |p| p.exists(),
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

    fn config_file(file: Option<&PathBuf>, dir: &Path) -> PathBuf {
        if let Some(cf) = file {
            debug!("Loading config from {} (set via flag)", cf.display());
            return cf.to_path_buf();
        }

        let default = Self::default_config_file(dir);
        debug!(
            "Loading config from {} (default location)",
            default.display()
        );
        default
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

    pub fn run(&mut self) -> i8 {
        // We use a channel to send and receive events in order to avoid
        // having to make the output writer Sync, which is hard (or
        // impossible?) because we want to use a Box<dyn OutputWriter>.
        let (tx, rx) = unbounded();
        let mut status: i8 = 0;
        if let Err(e) = thread::scope(|s| {
            let format = self.output_format;
            let chars = self.chars;
            let quiet = self.quiet;
            let h = s.spawn(move |_| Self::handle_events(rx, format, chars, quiet));

            debug!("Running subcommand");
            status = match self.run_subcommand(&tx) {
                Ok(e) => {
                    debug!("run_subcommand exit: {:?}", e);
                    let status = e.status();
                    match e {
                        Exit::Ok => (),
                        Exit::NoFiles => {
                            println!("{}", e.error_message());
                        }
                        Exit::FromActionFailures(_) => {
                            if let Err(e) =
                                tx.send(Some(Event::SubcommandExitWithMessage(e.error_message())))
                            {
                                error!(
                                    "Error writing Event::SubcommandExitWithError event to channel: {e}"
                                );
                            }
                        }
                        Exit::FromError(_) => {
                            if let Err(e) =
                                tx.send(Some(Event::SubcommandExitWithError(e.error_message())))
                            {
                                error!(
                                    "Error writing Event::SubcommandExitWithMessage event to channel: {e}"
                                );
                            }
                        }
                    }
                    status
                }
                Err(e) => {
                    error!("Failed to run precious: {}", e);
                    1
                }
            };
            if let Err(e) = tx.send(None) {
                error!("Error writing None to channel: {e}");
            }

            h.join()
        }) {
            error!("Error from thread::scope: {e:?}");
            return 127;
        }

        status
    }

    fn handle_events(
        rx: Receiver<Option<Event>>,
        format: OutputFormat,
        chars: &'static Chars,
        quiet: bool,
    ) -> Result<()> {
        let mut output_writer: Box<dyn OutputWriter> = match format {
            OutputFormat::Unstructured => Box::new(UnstructuredTextWriter::new(chars, quiet)),
        };
        debug!("Entering recv loop");
        loop {
            let e = rx.recv();
            debug!("Got {e:?} from channel");
            match e {
                Ok(Some(e)) => output_writer.handle_event(e)?,
                Ok(None) => {
                    output_writer.flush()?;
                    break;
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    fn run_subcommand(&mut self, tx: &Sender<Option<Event>>) -> Result<Exit> {
        if self.should_lint {
            self.lint(tx)
        } else {
            self.tidy(tx)
        }
    }

    fn tidy(&mut self, tx: &Sender<Option<Event>>) -> Result<Exit> {
        tx.send(Some(Event::StartingAction("Tidying", self.mode)))?;

        let tidiers = self
            .config
            // XXX - This clone can be removed if config is passed into this
            // method instead of being a field of self.
            .clone()
            .into_tidy_commands(&self.project_root, self.command.as_deref())?;
        self.run_all_commands(
            "tidying",
            tx,
            tidiers,
            |self_: &mut Self,
             tx: &Sender<Option<Event>>,
             files: &[PathBuf],
             tidier: &command::Command| { self_.run_one_tidier(tx, files, tidier) },
        )
    }

    fn lint(&mut self, tx: &Sender<Option<Event>>) -> Result<Exit> {
        tx.send(Some(Event::StartingAction("Linting", self.mode)))?;

        let linters = self
            .config
            // XXX - same as above.
            .clone()
            .into_lint_commands(&self.project_root, self.command.as_deref())?;
        self.run_all_commands(
            "linting",
            tx,
            linters,
            |self_: &mut Self,
             tx: &Sender<Option<Event>>,
             files: &[PathBuf],
             linter: &command::Command| { self_.run_one_linter(tx, files, linter) },
        )
    }

    fn run_all_commands<R>(
        &mut self,
        action: &str,
        tx: &Sender<Option<Event>>,
        commands: Vec<command::Command>,
        run_command: R,
    ) -> Result<Exit>
    where
        R: Fn(
            &mut Self,
            &Sender<Option<Event>>,
            &[PathBuf],
            &command::Command,
        ) -> Result<Option<Vec<ActionFailure>>>,
    {
        if commands.is_empty() {
            if let Some(c) = &self.command {
                return Err(PreciousError::NoCommandsMatch {
                    what: action.into(),
                    name: c.into(),
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

        match self.finder()?.files(cli_paths)? {
            None => Ok(Exit::NoFiles),
            Some(files) => {
                let mut all_failures: Vec<ActionFailure> = vec![];
                for c in commands {
                    debug!(r#"Command config for {}: {}"#, c.name, c.config_debug(),);
                    if let Some(mut failures) = run_command(self, tx, &files, &c)? {
                        all_failures.append(&mut failures);
                    }
                }

                Ok(self.make_exit(all_failures, action))
            }
        }
    }

    fn finder(&mut self) -> Result<Finder> {
        Finder::new(
            self.mode,
            self.project_root.clone(),
            self.cwd.clone(),
            self.config.exclude.clone(),
        )
    }

    fn make_exit(&self, failures: Vec<ActionFailure>, action: &str) -> Exit {
        if failures.is_empty() {
            return Exit::Ok;
        }

        let red = format!("\x1B[{}m", Color::Red.to_fg_str());
        let ansi_off = "\x1B[0m";
        let plural = if failures.len() > 1 { 's' } else { '\0' };

        let error = format!(
            "{}Error{} when {} files:{}\n{}",
            red,
            plural,
            action,
            ansi_off,
            failures
                .iter()
                .map(|af| format!(
                    "  {} [{}] [{}]\n    {}\n",
                    "*", // self.chars.bullet,
                    af.paths.iter().map(|p| p.to_string_lossy()).join(" "),
                    af.config_key,
                    af.error,
                ))
                .collect::<Vec<String>>()
                .join("")
        );

        Exit::FromActionFailures(error)
    }

    fn run_one_tidier(
        &mut self,
        tx: &Sender<Option<Event>>,
        files: &[PathBuf],
        t: &command::Command,
    ) -> Result<Option<Vec<ActionFailure>>> {
        let runner = |tx: Sender<Option<Event>>,
                      files: &[&Path]|
         -> Result<Option<Result<(), ActionFailure>>> {
            match t.tidy(files) {
                Ok(Some(TidyOutcome::Changed)) => {
                    tx.send(Some(Event::TidiedFiles(
                        t.name.clone(),
                        files.iter().map(PathBuf::from).collect(),
                    )))?;
                    Ok(Some(Ok(())))
                }
                Ok(Some(TidyOutcome::Unchanged)) => {
                    tx.send(Some(Event::DidNotTidyFiles(
                        t.name.clone(),
                        files.iter().map(PathBuf::from).collect(),
                    )))?;
                    Ok(Some(Ok(())))
                }
                Ok(Some(TidyOutcome::Unknown)) => {
                    tx.send(Some(Event::MaybeTidiedFiles(
                        t.name.clone(),
                        files.iter().map(PathBuf::from).collect(),
                    )))?;
                    Ok(Some(Ok(())))
                }
                Ok(None) => Ok(None),
                Err(e) => {
                    tx.send(Some(Event::CommandError(
                        t.name.clone(),
                        files.iter().map(PathBuf::from).collect(),
                    )))?;
                    Ok(Some(Err(ActionFailure {
                        error: format!("{:#}", e),
                        config_key: t.config_key(),
                        paths: files.iter().map(|f| f.to_path_buf()).collect(),
                    })))
                }
            }
        };

        self.run_parallel("Tidying", tx, files, t, runner)
    }

    fn run_one_linter(
        &mut self,
        tx: &Sender<Option<Event>>,
        files: &[PathBuf],
        l: &command::Command,
    ) -> Result<Option<Vec<ActionFailure>>> {
        let runner = |tx: Sender<Option<Event>>,
                      files: &[&Path]|
         -> Result<Option<Result<(), ActionFailure>>> {
            match l.lint(files) {
                Ok(Some(lo)) => {
                    if lo.ok {
                        tx.send(Some(Event::FoundLintCleanFiles(
                            l.name.clone(),
                            files.iter().map(PathBuf::from).collect(),
                        )))?;
                        Ok(Some(Ok(())))
                    } else {
                        tx.send(Some(Event::FoundLintDirtyFiles(
                            l.name.clone(),
                            files.iter().map(PathBuf::from).collect(),
                            lo.stdout,
                            lo.stderr,
                        )))?;
                        Ok(Some(Err(ActionFailure {
                            error: "linting failed".into(),
                            config_key: l.config_key(),
                            paths: files.iter().map(|f| f.to_path_buf()).collect(),
                        })))
                    }
                }
                Ok(None) => Ok(None),
                Err(e) => {
                    tx.send(Some(Event::CommandError(
                        l.name.clone(),
                        files.iter().map(PathBuf::from).collect(),
                    )))?;
                    Ok(Some(Err(ActionFailure {
                        error: format!("{:#}", e),
                        config_key: l.config_key(),
                        paths: files.iter().map(|f| f.to_path_buf()).collect(),
                    })))
                }
            }
        };

        self.run_parallel("Linting", tx, files, l, runner)
    }

    fn run_parallel<R>(
        &mut self,
        what: &str,
        tx: &Sender<Option<Event>>,
        files: &[PathBuf],
        c: &command::Command,
        runner: R,
    ) -> Result<Option<Vec<ActionFailure>>>
    where
        R: Fn(Sender<Option<Event>>, &[&Path]) -> Result<Option<Result<(), ActionFailure>>> + Sync,
    {
        let sets = c.files_to_args_sets(files)?;

        let start = Instant::now();
        let results = self
            .thread_pool
            .install(|| -> Result<Vec<Result<(), ActionFailure>>> {
                let mut res: Vec<Result<(), ActionFailure>> = vec![];
                res.append(
                    &mut sets
                        .into_par_iter()
                        .filter_map(|set| match runner(tx.clone(), &set) {
                            Ok(None) => None,
                            Ok(Some(r)) => Some(Ok(r)),
                            Err(e) => Some(Err(e)),
                        })
                        .collect::<Result<Vec<Result<(), ActionFailure>>>>()?,
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
            .filter_map(|r| match r {
                Ok(_) => None,
                Err(e) => Some(e),
            })
            .collect::<Vec<ActionFailure>>();
        if failures.is_empty() {
            Ok(None)
        } else {
            Ok(Some(failures))
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
    use itertools::Itertools;
    use precious_helpers::pushd::Pushd;
    use precious_testhelper::TestHelper;
    use pretty_assertions::assert_eq;
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
lint_flags = "--check"
ok_exit_codes = [0]
lint_failure_exit_codes = [1]
"#;

    const DEFAULT_CONFIG_FILE_NAME: &str = super::CONFIG_FILE_NAMES[0];

    #[test]
    #[serial]
    fn new() -> Result<()> {
        for name in super::CONFIG_FILE_NAMES {
            let helper = TestHelper::new()?.with_config_file(name, SIMPLE_CONFIG)?;
            let _pushd = helper.pushd_to_git_root()?;

            let app = App::try_parse_from(["precious", "tidy", "--all"])?;
            let config = app.config.clone();

            let p = Precious::new(app)?;
            assert_eq!(p.chars, &chars::FUN_CHARS);

            let config_file = Precious::config_file(config.as_ref(), &p.project_root);
            let mut expect_config_file = p.project_root;
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

        let p = Precious::new(app)?;
        assert_eq!(p.chars, &chars::BORING_CHARS);

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
        let config = app.config.clone();
        let p = Precious::new(app)?;

        let config_file = Precious::config_file(config.as_ref(), &p.project_root);
        let mut expect_config_file = p.project_root;
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

        let p = Precious::new(app)?;
        assert_eq!(p.project_root, src_dir);

        Ok(())
    }

    type FinderTestAction = Box<dyn Fn(&TestHelper) -> Result<()>>;

    #[test_case(
        "--all",
        &[],
        Box::new(|_| Ok(())),
        &[
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
        &["src/module.rs", "tests/data/foo.txt"] ;
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
        &["src/module.rs", "tests/data/foo.txt"] ;
        "--staged"
    )]
    #[test_case(
        "",
        &["main.rs", "module.rs"],
        Box::new(|_| Ok(())),
        &["src/main.rs", "src/module.rs"] ;
        "file paths from cli"
    )]
    #[test_case(
        "",
        &["."],
        Box::new(|_| Ok(())),
        &[
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
        expect: &[&str],
    ) -> Result<()> {
        let helper = TestHelper::new()?
            .with_config_file(DEFAULT_CONFIG_FILE_NAME, SIMPLE_CONFIG)?
            .with_git_repo()?;
        (action)(&helper)?;

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

        let mut p = Precious::new(app)?;

        assert_eq!(
            p.finder()?
                .files(paths.iter().map(PathBuf::from).collect())?,
            Some(expect.iter().map(PathBuf::from).collect::<Vec<_>>()),
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
    ok_exit_codes = [0]
    "#;
        let helper = TestHelper::new()?.with_config_file(DEFAULT_CONFIG_FILE_NAME, config)?;
        let _pushd = helper.pushd_to_git_root()?;

        let app = App::try_parse_from(["precious", "--quiet", "tidy", "--all"])?;

        let mut p = Precious::new(app)?;
        let status = p.run();

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
    ok_exit_codes = [0]
    "#;
        let helper = TestHelper::new()?.with_config_file(DEFAULT_CONFIG_FILE_NAME, config)?;
        let _pushd = helper.pushd_to_git_root()?;

        let app = App::try_parse_from(["precious", "--quiet", "tidy", "--all"])?;

        let mut p = Precious::new(app)?;
        let status = p.run();

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
    ok_exit_codes = [0]
    lint_failure_exit_codes = [1]
    "#;
        let helper = TestHelper::new()?.with_config_file(DEFAULT_CONFIG_FILE_NAME, config)?;
        let _pushd = helper.pushd_to_git_root()?;

        let app = App::try_parse_from(["precious", "--quiet", "lint", "--all"])?;

        let mut p = Precious::new(app)?;
        let status = p.run();

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

        let mut p = Precious::new(app)?;
        let status = p.run();

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

        let mut p = Precious::new(app)?;
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
        let helper = TestHelper::new()?.with_config_file(DEFAULT_CONFIG_FILE_NAME, config)?;
        let test_replace = PathBuf::from_str("test.replace")?;
        helper.write_file(&test_replace, "The letter A")?;
        let _pushd = helper.pushd_to_git_root()?;

        let app = App::try_parse_from(["precious", "--quiet", "tidy", "-a"])?;

        let mut p = Precious::new(app)?;
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
