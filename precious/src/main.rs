#![recursion_limit = "1024"]

use log::error;
use precious_core::precious;

fn main() {
    let app = precious::app();
    if let Err(e) = app.init_logger() {
        eprintln!("Error creating logger: {e}");
        std::process::exit(42);
    }
    let status = match app.run() {
        Ok(s) => s,
        Err(e) => {
            error!("{e}");
            42
        }
    };
    std::process::exit(i32::from(status));
}
