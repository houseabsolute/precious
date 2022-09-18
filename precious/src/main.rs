#![recursion_limit = "1024"]

use log::error;
use precious_core::precious;

fn main() {
    let app = precious::app();
    let res = precious::init_logger(&app);
    if let Err(e) = res {
        eprintln!("Error creating logger: {}", e);
        std::process::exit(1);
    }
    let p = precious::Precious::new(app);
    let status = match p {
        Ok(mut p) => p.run(),
        Err(e) => {
            error!("{}", e);
            1
        }
    };
    std::process::exit(status as i32);
}
