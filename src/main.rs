#![recursion_limit = "1024"]

#[cfg(test)]
mod testhelper;

mod basepaths;
mod chars;
mod command;
mod config;
mod filter;
mod path_matcher;
mod precious;
mod vcs;

use anyhow::Result;
use clap::ArgMatches;
use fern::colors::{Color, ColoredLevelConfig};
use fern::Dispatch;
use log::error;

fn main() {
    let matches = precious::app().get_matches();
    let res = init_logger(&matches);
    if let Err(e) = res {
        eprintln!("Error creating logger: {}", e);
        std::process::exit(126);
    }
    let p = precious::Precious::new(&matches);
    let status = match p {
        Ok(mut p) => p.run(),
        Err(e) => {
            error!("{}", e);
            127
        }
    };
    std::process::exit(status);
}

fn init_logger(matches: &ArgMatches) -> Result<(), log::SetLoggerError> {
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

    let level_colors = line_colors.clone().info(Color::Green).debug(Color::Black);
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
        .chain(std::io::stdout())
        .apply()
}
