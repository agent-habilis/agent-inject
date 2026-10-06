/**
 * Build `crates/agent-inject-wasm-client/` into the wasm-bindgen output the web
 * app consumes, `packages/agent-inject-wasm/src/glue/`.
 *
 * The output lands inside `packages/` because the dev bundler's watcher never
 * leaves it: glue built elsewhere goes stale across a rebuild and meets the
 * fresh binary as `LinkError: … function import requires a callable`.
 *
 * `cargo task web-wasm` shells into this file, so there is one implementation.
 * The crate is a standalone workspace, so cargo runs from its directory.
 */

import { delimiter, dirname, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

import { $ } from 'bun'

import { GLUE_DIR } from './wasm-asset.ts'

const REPO_ROOT = new URL('../', import.meta.url)
const CRATE = new URL('crates/agent-inject-wasm-client/', REPO_ROOT)
const CRATE_DIR = fileURLToPath(CRATE)

const TARGET = 'wasm32-unknown-unknown'
const ARTIFACT = `target/${TARGET}/release/agent_inject_wasm_client.wasm`

function fail(message: string): never {
  console.error(message)
  process.exit(1)
}

/**
 * The environment cargo should see, with the caller's own cargo run scrubbed
 * out. `cargo task web-wasm` reaches this file through `cargo run`, which
 * exports `CARGO_*` and `RUSTUP_TOOLCHAIN` into the child; cargo fingerprints
 * build scripts against their environment, so the two ways in would rebuild
 * ring and the iroh stack every time. An inherited `RUSTFLAGS` would also
 * replace the `getrandom_backend` cfg that `.cargo/config.toml` sets.
 */
function cargoEnv(extra: Record<string, string> = {}): Record<string, string> {
  const env: Record<string, string> = {}
  for (const [key, value] of Object.entries(process.env)) {
    if (value === undefined) continue
    if (key !== 'CARGO_HOME' && key.startsWith('CARGO')) continue
    if (
      [
        'RUSTUP_TOOLCHAIN',
        'RUSTC',
        'RUSTC_WRAPPER',
        'RUSTC_WORKSPACE_WRAPPER',
        'RUSTDOC',
        'RUSTFLAGS',
        'LD_LIBRARY_PATH',
        'DYLD_FALLBACK_LIBRARY_PATH',
      ].includes(key)
    ) {
      continue
    }
    env[key] = value
  }
  return { ...env, ...extra }
}

/**
 * A clang that can emit `wasm32`, or `null`. Apple clang ships no wasm
 * backend, so `ring`'s C core fails with the default `cc` on macOS. Mirrors
 * `wasm_clang` in `tasks/src/ci.rs`.
 */
async function wasmClang(): Promise<string | null> {
  for (const candidate of [
    '/opt/homebrew/opt/llvm/bin/clang',
    '/usr/local/opt/llvm/bin/clang',
  ]) {
    if (await Bun.file(candidate).exists()) return candidate
  }
  const version = await $`clang --version`.nothrow().quiet().text()
  return version.includes('Apple clang') ? null : 'clang'
}

/**
 * The `bin/` of the toolchain that `rust-toolchain.toml` pins, or `null` when
 * `rustup` is not on the `PATH`. A `cargo` found first on the `PATH` (a
 * Homebrew or distro Rust) ignores the pin and lacks the wasm target, so the
 * build puts the pinned toolchain's own `cargo` and `rustc` first.
 */
async function pinnedToolchainBin(): Promise<string | null> {
  const found = await $`rustup which cargo`.cwd(CRATE_DIR).nothrow().quiet()
  return found.exitCode === 0 ? dirname(found.text().trim()) : null
}

/** The `wasm-bindgen` version the crate actually links against. */
async function lockedBindgenVersion(): Promise<string> {
  const lock = await Bun.file(new URL('Cargo.lock', CRATE)).text()
  const match = lock.match(/name = "wasm-bindgen"\nversion = "([^"]+)"/)
  if (!match?.[1]) fail('no wasm-bindgen entry in the wasm client Cargo.lock')
  return match[1]
}

async function installHint(): Promise<string> {
  return `cargo install wasm-bindgen-cli --version ${await lockedBindgenVersion()} --locked`
}

async function ensurePrereqs(): Promise<void> {
  const installed = await $`rustup target list --installed`
    .cwd(CRATE_DIR)
    .nothrow()
    .quiet()
    .text()
  if (!installed.split('\n').includes(TARGET)) {
    fail(`the ${TARGET} target is missing — run \`rustup target add ${TARGET}\``)
  }
  if ((await $`wasm-bindgen --version`.nothrow().quiet()).exitCode !== 0) {
    fail(`the wasm-bindgen CLI is missing — run \`${await installHint()}\``)
  }
}

/** Build the binary and the browser glue. */
export async function buildWasm(): Promise<void> {
  await ensurePrereqs()

  const clang = await wasmClang()
  if (!clang && process.platform === 'darwin') {
    console.warn('  no wasm-capable clang found — if ring fails, `brew install llvm`')
  }

  const toolchainBin = await pinnedToolchainBin()
  const extra: Record<string, string> = clang
    ? { CC: clang, [`CC_${TARGET.replaceAll('-', '_')}`]: clang }
    : {}
  if (toolchainBin) extra['PATH'] = [toolchainBin, process.env['PATH'] ?? ''].join(delimiter)

  console.log(`  building agent-inject-wasm-client (${TARGET}, release)`)
  await $`cargo build --release --target ${TARGET}`
    .cwd(CRATE_DIR)
    .env(cargoEnv(extra))

  const outDir = fileURLToPath(GLUE_DIR)
  const bindgen = await $`wasm-bindgen --target web --out-dir ${outDir} ${ARTIFACT}`
    .cwd(CRATE_DIR)
    .nothrow()
    .quiet()
  // wasm-bindgen checks its own schema against the binary's and says so
  // exactly, so nothing here compares version numbers first.
  if (bindgen.exitCode !== 0) {
    fail(`${bindgen.stderr.toString().trim()}\n\ntry \`${await installHint()}\``)
  }
  console.log(`  bindgen ${relative(fileURLToPath(REPO_ROOT), outDir)}`)
}

if (import.meta.main) await buildWasm()
