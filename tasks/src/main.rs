use std::process::ExitCode;

use clap::{Parser, Subcommand};
use xshell::Shell;

mod ci;
mod fmt;
mod lint;
mod naming;
mod run;
mod test;
mod util;
mod web_wasm;

/// Task result; any `Err` is printed and turns into a non-zero exit.
pub(crate) type TaskOutcome = Result<(), Box<dyn std::error::Error>>;

/// Project task runner. Run `cargo task <task>`.
#[derive(Parser)]
#[command(bin_name = "cargo task")]
struct Cli {
    #[command(subcommand)]
    task: Task,
}

/// Variant doc comments *are* the `--help` text.
#[derive(Subcommand)]
enum Task {
    /// Run unit tests.
    Test,
    /// Run the binary (`cargo run`). Extra args go to `agent-inject`.
    Run {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run the CI gate.
    Ci,
    /// Format source files.
    Fmt,
    /// Run clippy lints.
    Lint,
    /// Check file and directory names: snake_case inside a crate, kebab-case
    /// everywhere else.
    Naming,
    /// Build the browser wasm client into `packages/agent-inject-wasm`.
    WebWasm,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let sh = match Shell::new() {
        Ok(sh) => sh,
        Err(error) => {
            util::output::error(&error.to_string());
            return ExitCode::FAILURE;
        }
    };

    let outcome = match cli.task {
        Task::Test => test::run(&sh),
        Task::Run { args } => run::run(&sh, &args),
        Task::Ci => ci::run(&sh),
        Task::Fmt => fmt::run(&sh),
        Task::Lint => lint::run(&sh),
        Task::Naming => naming::run(&sh),
        Task::WebWasm => web_wasm::run(&sh),
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            util::output::error(&error.to_string());
            ExitCode::FAILURE
        }
    }
}
