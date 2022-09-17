#![recursion_limit = "1024"]

use log::error;
use precious_core::precious;

fn main() {
    let matches = precious::app().get_matches();
    let res = precious::init_logger(&matches);
    if let Err(e) = res {
        eprintln!("Error creating logger: {}", e);
        std::process::exit(1);
    }
    let p = precious::Precious::new(&matches);
    let status = match p {
        Ok(mut p) => p.run(),
        Err(e) => {
            error!("{}", e);
            1
        }
    };
    std::process::exit(status as i32);
}
