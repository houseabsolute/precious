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

use log::error;

fn main() {
    let matches = precious::app().get_matches();
    let res = precious::init_logger(&matches);
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
