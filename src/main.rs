mod agent;
mod api;
mod command;
mod config;
mod context;
mod display;
mod error;
mod readline;
mod session;
mod signal;
mod skill;
mod sse;
mod stream;
mod timer;
mod tool;
mod types;
mod util;

use std::process;

const VERSION: &str = "tapir v0.1.0";

fn main() {
    let config_path = match parse_args() {
        Some(path) => path,
        None => return,
    };

    eprintln!(
        r#"
   ░██                          ░██
   ░██
░████████  ░██████   ░████████  ░██░███████
   ░██          ░██  ░██    ░██ ░██░██
   ░██     ░███████  ░██    ░██ ░██░██
   ░██    ░██   ░██  ░██    ░██ ░██░██
    ░████  ░████████ ░████████  ░██░██
                     ░██
                     ░██

                  v0.1.0
"#
    );
    signal::install_handler();

    let mut config = match config::Config::load(config_path.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    };

    if let Err(e) = agent::run(&mut config) {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

/// Returns `Some(config_path)` to continue, `None` to exit.
fn parse_args() -> Option<Option<String>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => Some(None),
        Some("-V") => {
            println!("{VERSION}");
            None
        }
        Some("-c") => {
            let path = args.get(1).unwrap_or_else(|| {
                eprintln!("error: -c requires a path");
                process::exit(1);
            });
            if args.len() > 2 {
                eprintln!("error: unexpected argument: {}", args[2]);
                process::exit(1);
            }
            Some(Some(path.clone()))
        }
        Some(other) => {
            eprintln!("error: unknown argument: {other}");
            eprintln!("usage: tapir [-V] [-c config.json]");
            process::exit(1);
        }
    }
}
