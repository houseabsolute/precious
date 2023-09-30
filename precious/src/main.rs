#![recursion_limit = "1024"]

use log::error;
use precious_core::precious;

fn main() {
    let app = precious::app();
    if let Err(e) = app.init_logger() {
        eprintln!("Error creating logger: {e}");
        std::process::exit(1);
    }
    let status = match precious::Precious::new(app) {
        Ok(mut p) => p.run(),
        Err(e) => {
            error!("{e}");
            1
        }
    };
    std::process::exit(status as i32);
}
