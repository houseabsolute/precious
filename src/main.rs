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

use clap::ArgMatches;
use log::error;

fn main() {
    let matches = precious::app().get_matches();
    init_logger(&matches);
    let p = precious::Precious::new(&matches);
    let status = match p {
        Ok(mut p) => p.run(),
        Err(e) => {
            error!("{}", e);
            127 as i32
        }
    };
    std::process::exit(status);
}

fn init_logger(matches: &ArgMatches) {
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
