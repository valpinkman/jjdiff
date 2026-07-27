// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use jjdiff_app::cli::{self, Args, Headless};

fn main() {
    // Parse argv once, before any window is created. Headless commands
    // (`--help`, `--version`, `--walkthrough-guide`, `--diff`,
    // `--print-hunks`, `--install-terminal-helper`) write to stdout and exit
    // without bringing up the GUI — a bundled macOS binary still has a usable
    // stdout when invoked from a terminal.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match Args::parse(&argv) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("jjdiff: {error}");
            eprintln!();
            eprintln!("Run `jjdiff --help` for usage.");
            std::process::exit(2);
        }
    };

    if let Some(headless) = &args.headless {
        if let Err(error) = cli::run_headless(headless, &args) {
            eprintln!("jjdiff: {error}");
            std::process::exit(1);
        }
        // Help/version/guide/diff/print-hunks/install all exit 0 here.
        match headless {
            Headless::Help
            | Headless::Version
            | Headless::WalkthroughGuide
            | Headless::Diff(_)
            | Headless::PrintHunks(_)
            | Headless::InstallTerminalHelper => std::process::exit(0),
        }
    }

    jjdiff_app::run(args);
}
