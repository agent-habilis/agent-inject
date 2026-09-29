//! Build `crates/agent-inject-wasm-client/` into the wasm-bindgen output the
//! web app consumes, `packages/agent-inject-wasm/src/glue/`. The build itself
//! lives in `scripts/build-wasm.ts` so that `bun run build` is self-contained.
//!
//! Never part of `ci`: it needs the wasm target, the `wasm-bindgen` CLI, and a
//! wasm-capable clang (ring's C core — Apple clang cannot target wasm32,
//! Homebrew LLVM can).

use xshell::{Shell, cmd};

use crate::TaskOutcome;
use crate::util::{output, repo_root};

pub(crate) fn run(sh: &Shell) -> TaskOutcome {
    if cmd!(sh, "bun --version").quiet().read().is_err() {
        return Err("bun is missing — see https://bun.sh".into());
    }

    output::status("Building", "agent-inject-wasm-client (wasm32, release)");
    let _guard = sh.push_dir(repo_root());
    cmd!(sh, "bun scripts/build-wasm.ts").run()?;

    output::status("Finished", "packages/agent-inject-wasm/src/glue");
    Ok(())
}
