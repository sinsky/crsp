//! Entry point (spec §3.2): parse synchronously so `--version`, `--help`, and
//! usage errors never initialize a Tokio runtime; map every failure —
//! including clap usage and unknown-command failures — to exit code 1
//! (commander-compatible; clap's default exit code 2 is overridden).

use clap::Parser as _;
use clap::error::ErrorKind;
use crsp::{Cli, run};

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            if error.kind() == ErrorKind::DisplayVersion {
                // commander prints the bare version string on stdout.
                println!("{}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            let _ = error.print();
            std::process::exit(if error.kind() == ErrorKind::DisplayHelp {
                0
            } else {
                1
            });
        }
    };

    if let Err(error) = run(&cli) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
