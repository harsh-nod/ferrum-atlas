#![feature(rustc_private)]

use atlas_rustc_adapter::{Options, run};
use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::args().skip(1).any(|arg| arg == "--help") {
        println!(
            "atlas-rustc --trusted-local --root DIR --source FILE --output FILE [--crate-name NAME] [--edition 2021] [--panic unwind|abort] [--cfg KEY]"
        );
        return ExitCode::SUCCESS;
    }
    let result = (|| {
        let options = Options::parse(std::env::args().skip(1))?;
        std::env::set_current_dir(&options.root)?;
        // This process has not started any threads. Scrub all inherited compiler
        // inputs before rustc starts its worker threads, including env! secrets.
        for key in std::env::vars_os().map(|(key, _)| key).collect::<Vec<_>>() {
            unsafe { std::env::remove_var(key) };
        }
        run(options)
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("atlas-rustc: {error:#}");
            ExitCode::FAILURE
        }
    }
}
