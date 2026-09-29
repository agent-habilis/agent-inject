use xshell::{Shell, cmd};

use crate::TaskOutcome;
use crate::util::{output, repo_root};

pub(crate) fn run(sh: &Shell) -> TaskOutcome {
    output::status("Installing", "agent-inject");
    // The package dir, not the root: `cargo install` needs a package manifest
    // and the workspace root is virtual. Absolute, because no task changes
    // directory and a relative path only resolves from the workspace root.
    let pkg = repo_root().join("crates/agent-inject");
    // `--force`: the crate version rarely changes between builds, and without
    // it `cargo install` treats "already installed" as up-to-date and skips
    // the rebuild, leaving a stale binary in place.
    //
    // `--locked`: without it `cargo install` ignores Cargo.lock and resolves
    // afresh on the host, so a registry release after the lock was cut can
    // change the build.
    cmd!(sh, "cargo install --path {pkg} --force --locked")
        .quiet()
        .run()?;
    output::status("Installed", "~/.cargo/bin/agent-inject");
    Ok(())
}
