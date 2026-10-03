use std::process::ExitCode;

use clap::{Parser, Subcommand};
use xshell::Shell;

mod ci;
mod fmt;
mod install;
mod lint;
mod naming;
mod publish_web_image;
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
    /// Install the binary.
    Install,
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
    /// Build the web app into a container image (Bun serving the static
    /// `dist/`) and push it to a container registry. Hermetic: the
    /// image rebuilds the wasm from source, so nothing on this machine leaks
    /// into it and no `web-wasm` run is needed first.
    PublishWebImage {
        /// Image tag. Defaults to the short commit sha, marked `-dirty` when
        /// the tree has uncommitted changes. `latest` is always tagged and
        /// pushed alongside it.
        #[arg(long)]
        tag: Option<String>,
        /// Registry host, e.g. `registry.example.com`. Log in first with
        /// `docker login <registry>`.
        #[arg(long, env = "AGENT_INJECT_REGISTRY")]
        registry: String,
        /// User or org that owns the package in the registry.
        #[arg(long, env = "AGENT_INJECT_REGISTRY_OWNER")]
        owner: String,
        /// Build only: skip both pushes.
        #[arg(long)]
        no_push: bool,
        /// Target platform. The default builds natively on Apple Silicon;
        /// building the other one there pulls in qemu and gets slow.
        #[arg(long, default_value = "linux/arm64")]
        platform: publish_web_image::Platform,
    },
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
        Task::Install => install::run(&sh),
        Task::Ci => ci::run(&sh),
        Task::Fmt => fmt::run(&sh),
        Task::Lint => lint::run(&sh),
        Task::Naming => naming::run(&sh),
        Task::WebWasm => web_wasm::run(&sh),
        Task::PublishWebImage {
            tag,
            registry,
            owner,
            no_push,
            platform,
        } => publish_web_image::run(
            &sh,
            &publish_web_image::Options {
                tag,
                registry,
                owner,
                no_push,
                platform,
            },
        ),
    };

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            util::output::error(&error.to_string());
            ExitCode::FAILURE
        }
    }
}
