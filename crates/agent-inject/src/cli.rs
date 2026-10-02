//! `agent-inject [dir]`: run one session until Done or ctrl-c. With no `dir`,
//! the session saves into a fresh folder (see [`crate::session_dir`]).
//! `agent-inject plug` / `unplug`: install or remove the agent skills.
//!
//! stdout carries everything a person or a script reads: the URL, the QR
//! code, and one absolute path per saved file. stderr carries errors only.

use std::io::Write as _;
use std::path::PathBuf;

use agent_inject_proto::Accept;
use agent_inject_proto::lookup::{LookupOpts, RelayChoice};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use fofoca_iroh_webrtc_transport::IceConfig;

use crate::plug::{self, Agent};
use crate::serve::{ServeOpts, serve_with};
use crate::util::output::{home_path, status_out};
use crate::web::web_url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub(crate) enum OutputFormat {
    #[default]
    Human,
    /// One JSON object per line: `{"url":…,"qr":…,"dir":…}` first, then
    /// `{"path":…}` per file.
    Json,
}

/// What the phone may send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub(crate) enum AcceptArg {
    #[default]
    Any,
    Images,
}

/// Receive photos and files from a phone into a directory.
#[derive(Debug, Parser)]
#[command(
    name = "agent-inject",
    version,
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Directory the files are written into. Created if missing. Default: a
    /// fresh `<session-id>` folder under `$AGENT_INJECT_DIR`, or under
    /// `/tmp/agent-inject`. A directory named like a subcommand needs a path
    /// prefix, for example `./plug`.
    dir: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t)]
    output: OutputFormat,
    /// What the phone may send. `images` hides the file picker and refuses
    /// other files.
    #[arg(long, value_enum, default_value_t)]
    accept: AcceptArg,
    /// Do not print the QR code.
    #[arg(long)]
    no_qr: bool,
    /// Serve on 127.0.0.1 only, with no relay. For tests.
    #[arg(long, hide = true)]
    loopback: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Install the agent skills (for example `/inject-photo`).
    Plug {
        /// The agent to install into. Repeat for more. Default: every agent
        /// found on this machine.
        #[arg(long = "agent", value_enum)]
        agents: Vec<Agent>,
    },
    /// Remove the agent skills.
    Unplug {
        /// The agent to remove from. Repeat for more. Default: every agent
        /// that has them.
        #[arg(long = "agent", value_enum)]
        agents: Vec<Agent>,
    },
}

pub(crate) async fn run(cli: Cli) -> Result<()> {
    match &cli.command {
        Some(Command::Plug { agents }) => return plug::plug(agents),
        Some(Command::Unplug { agents }) => return plug::unplug(agents),
        None => {}
    }
    let dir = crate::session_dir::resolve(cli.dir.clone());
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let lookups = if cli.loopback {
        LookupOpts::loopback()
    } else {
        LookupOpts {
            mdns: false,
            dht: false,
            relay: RelayChoice::Pinned,
        }
    };
    let mut session = serve_with(ServeOpts {
        dir,
        lookups,
        accept: match cli.accept {
            AcceptArg::Any => Accept::Any,
            AcceptArg::Images => Accept::Images,
        },
        ice: IceConfig::default(),
    })
    .await?;
    let url = web_url(&session.ticket.encode());
    announce(&cli, &session.dir, &url);

    let mut files = Vec::new();
    let finished = session.finished();
    tokio::pin!(finished);
    loop {
        tokio::select! {
            saved = session.saved.recv() => {
                let Some(path) = saved else { break };
                print_saved(cli.output, &path);
                files.push(path);
            }
            () = &mut finished => {
                // Every file the sender sent before done is already queued.
                while let Ok(path) = session.saved.try_recv() {
                    print_saved(cli.output, &path);
                    files.push(path);
                }
                print_done(cli.output, &session.dir, &files);
                break;
            }
            // No done line: a caller can tell "finished" from "stopped".
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    session.shutdown().await;
    Ok(())
}

fn announce(cli: &Cli, dir: &std::path::Path, url: &str) {
    let qr = if cli.no_qr {
        None
    } else {
        crate::qr::render(url)
    };
    match cli.output {
        // An agent runs this in the background, where nobody sees stdout, so
        // the QR rides along for it to show.
        OutputFormat::Json => {
            let mut start = serde_json::json!({ "url": url, "dir": dir });
            if let Some(qr) = qr {
                start["qr"] = qr.into();
            }
            println!("{start}");
        }
        OutputFormat::Human => {
            if let Some(code) = qr {
                println!("{code}");
            }
            status_out("Open", url);
            status_out("Saving", &format!("to {}", home_path(dir)));
            status_out(
                "Waiting",
                "for files; press Done on the phone, or ctrl-c to stop",
            );
        }
    }
    let _ = std::io::stdout().flush();
}

fn print_saved(output: OutputFormat, path: &std::path::Path) {
    match output {
        OutputFormat::Json => println!("{}", serde_json::json!({ "path": path })),
        OutputFormat::Human => println!("{}", path.display()),
    }
    let _ = std::io::stdout().flush();
}

fn print_done(output: OutputFormat, dir: &std::path::Path, files: &[PathBuf]) {
    match output {
        OutputFormat::Json => println!(
            "{}",
            serde_json::json!({ "done": { "dir": dir, "files": files } })
        ),
        OutputFormat::Human => {
            let count = match files.len() {
                1 => "1 file".to_owned(),
                count => format!("{count} files"),
            };
            status_out("Done", &format!("{count} in {}", home_path(dir)));
        }
    }
    let _ = std::io::stdout().flush();
}
