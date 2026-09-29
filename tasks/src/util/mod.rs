use std::path::{Path, PathBuf};

use xshell::{Shell, cmd};

#[expect(
    dead_code,
    reason = "shared cargo-style helpers included from the binary; the task runner uses a subset"
)]
pub(crate) mod output;

/// Workspace root: the parent of this crate's `tasks/` manifest dir.
pub(crate) fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(
            || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            Path::to_path_buf,
        )
}

/// Install `krate` via `cargo install --locked` if the probe command
/// (`check`) fails. Best-effort: the calling task surfaces a clear error if
/// the tool is genuinely missing.
pub(crate) fn ensure_installed(sh: &Shell, krate: &str, check: &[&str]) {
    let ok = cmd!(sh, "cargo {check...}")
        .quiet()
        .ignore_stdout()
        .ignore_stderr()
        .run()
        .is_ok();

    if !ok {
        output::status("Installing", krate);
        let _ = cmd!(sh, "cargo install --locked {krate}").quiet().run();
    }
}

/// Prune `target/` artifacts not touched in the last week. Best-effort.
pub(crate) fn sweep_stale_artifacts(sh: &Shell) {
    ensure_installed(sh, "cargo-sweep", &["sweep", "--version"]);
    output::status("Pruning", "build artifacts older than 7 days");
    let _ = cmd!(sh, "cargo sweep --time 7").quiet().run();
}
