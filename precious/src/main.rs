#![recursion_limit = "1024"]

use log::error;
use precious_core::precious::App;

fn main() {
    let app = App::default();
    if let Err(e) = app.init_logger() {
        eprintln!("Error creating logger: {}", e);
        std::process::exit(1);
    }
    let p = app.into_precious();
    let status = match p {
        Ok(mut p) => p.run(),
        Err(e) => {
            error!("{}", e);
            1
        }
    };
    std::process::exit(status as i32);
}
