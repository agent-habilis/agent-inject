use xshell::{Shell, cmd};

use crate::TaskOutcome;
use crate::util::{output, repo_root};

pub(crate) fn run(sh: &Shell) -> TaskOutcome {
    // First because it has no toolchain prerequisite: a misnamed file should
    // not be reported after a multi-minute `clippy --all-targets`.
    output::status("Checking", "file and directory names");
    crate::naming::run(sh)?;

    output::status("Checking", "formatting");
    crate::fmt::check(sh)?;

    output::status("Running", "clippy");
    cmd!(sh, "cargo clippy --workspace --all-targets -- -D warnings")
        .quiet()
        .run()?;

    output::status("Running", "tests");
    cmd!(sh, "cargo test --workspace").quiet().run()?;

    web_app(sh)?;
    no_redeclared_wire_constants(sh)?;
    wasm32(sh)?;

    crate::util::sweep_stale_artifacts(sh);
    Ok(())
}

/// Typecheck and test the web app. Skipped with a message when bun is absent,
/// so the gate never fails for a reason unrelated to the change.
fn web_app(sh: &Shell) -> TaskOutcome {
    output::status("Checking", "the web app");
    let _guard = sh.push_dir(repo_root());
    if !sh.path_exists("package.json") {
        output::status("Skipping", "the web app (no package.json yet)");
        return Ok(());
    }
    if cmd!(sh, "bun --version")
        .quiet()
        .ignore_status()
        .read()
        .is_err()
    {
        output::status("Skipping", "the web app (bun is not installed)");
        return Ok(());
    }
    if !sh.path_exists("node_modules") {
        cmd!(sh, "bun install --frozen-lockfile").quiet().run()?;
    }
    cmd!(sh, "bun run typecheck").quiet().run()?;
    cmd!(sh, "bun run test").quiet().run()?;
    Ok(())
}

/// The WebRTC transport id and signal envelope are owned by
/// `fofoca-iroh-webrtc-transport`. Peers that disagree on either fail to
/// connect with no useful error, so a local copy must fail the gate.
fn no_redeclared_wire_constants(sh: &Shell) -> TaskOutcome {
    output::status("Checking", "no redeclared wire constants");
    for needle in ["0x5752_5443", "enum SignalEnvelope"] {
        let hits = cmd!(sh, "grep -rl --include=*.rs {needle} crates")
            .quiet()
            .ignore_status()
            .read()?;
        let files: Vec<_> = hits.lines().filter(|line| !line.is_empty()).collect();
        if !files.is_empty() {
            return Err(format!(
                "`{needle}` is owned by fofoca-iroh-webrtc-transport and must not be \
                 redeclared here, found in {files:?}"
            )
            .into());
        }
    }
    Ok(())
}

/// The wire format exists to be linked by the browser client, so a host-only
/// dependency must fail here. The wasm client is excluded from the workspace,
/// so its check and tests run here, on wasm32.
fn wasm32(sh: &Shell) -> TaskOutcome {
    output::status("Checking", "wasm32 targets");
    let installed = cmd!(sh, "rustup target list --installed").quiet().read()?;
    if !installed
        .lines()
        .any(|target| target == "wasm32-unknown-unknown")
    {
        output::status(
            "Skipping",
            "wasm32 checks (rustup target add wasm32-unknown-unknown)",
        );
        return Ok(());
    }
    cmd!(
        sh,
        "cargo check --target wasm32-unknown-unknown -p agent-inject-proto"
    )
    .quiet()
    .run()?;

    let client = repo_root().join("crates/agent-inject-wasm-client");
    if !sh.path_exists(&client) {
        return Ok(());
    }
    let Some(clang) = wasm_clang(sh) else {
        output::status("Skipping", "wasm client (no wasm-capable clang)");
        return Ok(());
    };
    let _guard = sh.push_dir(client);
    for args in [
        "check --target wasm32-unknown-unknown",
        "test --target wasm32-unknown-unknown --lib",
    ] {
        let args = args.split(' ');
        cmd!(sh, "cargo {args...}")
            .env("CC", &clang)
            .env("CC_wasm32_unknown_unknown", &clang)
            .quiet()
            .run()?;
    }
    Ok(())
}

/// A clang that can emit `wasm32`, or `None`. Apple clang ships no wasm
/// backend; Homebrew LLVM does.
pub(crate) fn wasm_clang(sh: &Shell) -> Option<String> {
    for candidate in [
        "/opt/homebrew/opt/llvm/bin/clang",
        "/usr/local/opt/llvm/bin/clang",
    ] {
        if sh.path_exists(candidate) {
            return Some(candidate.to_owned());
        }
    }
    let version = cmd!(sh, "clang --version")
        .quiet()
        .ignore_status()
        .read()
        .ok()?;
    (!version.contains("Apple clang")).then(|| "clang".to_owned())
}
