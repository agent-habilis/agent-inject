//! `agent-inject <dir>`: run one session until ctrl-c.
//!
//! stdout carries everything a person or a script reads: the URL, the QR
//! code, and one absolute path per saved file. stderr carries errors only.

use std::io::Write as _;
use std::path::PathBuf;

use agent_inject_proto::lookup::{LookupOpts, RelayChoice};
use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use fofoca_iroh_webrtc_transport::IceConfig;

use crate::serve::{ServeOpts, serve_with};
use crate::util::output::{home_path, status_out};
use crate::web::web_url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub(crate) enum OutputFormat {
    #[default]
    Human,
    /// One JSON object per line: `{"url":…}` first, then `{"path":…}` per file.
    Json,
}

/// Receive photos and files from a phone into a directory.
#[derive(Debug, Parser)]
#[command(name = "agent-inject", version)]
pub(crate) struct Cli {
    /// Directory the files are written into. Created if missing.
    dir: PathBuf,
    #[arg(long, value_enum, default_value_t)]
    output: OutputFormat,
    /// Do not print the QR code.
    #[arg(long)]
    no_qr: bool,
    /// Serve on 127.0.0.1 only, with no relay. For tests.
    #[arg(long, hide = true)]
    loopback: bool,
}

pub(crate) async fn run(cli: Cli) -> Result<()> {
    std::fs::create_dir_all(&cli.dir).with_context(|| format!("create {}", cli.dir.display()))?;
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
        dir: cli.dir.clone(),
        lookups,
        ice: IceConfig::default(),
    })
    .await?;
    let url = web_url(&session.ticket.encode());
    announce(&cli, &url);

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

fn announce(cli: &Cli, url: &str) {
    match cli.output {
        OutputFormat::Json => println!("{}", serde_json::json!({ "url": url })),
        OutputFormat::Human => {
            if !cli.no_qr
                && let Some(code) = crate::qr::render(url)
            {
                println!("{code}");
            }
            status_out("Open", url);
            status_out("Saving", &format!("to {}", home_path(&cli.dir)));
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
