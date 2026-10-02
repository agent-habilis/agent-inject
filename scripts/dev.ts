/**
 * Dev server: the app under `/app/*` with hot reload, and the wasm binary at
 * its content-addressed path.
 *
 * The binary is read per request rather than captured at start, so a
 * `cargo task web-wasm` mid-session is picked up; the watcher regenerates
 * `agent-inject-wasm`'s `path.ts`, and `--hot` rebundles onto the new hash.
 */

import { watch } from 'node:fs'
import { stat } from 'node:fs/promises'
import { basename, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

import index from '../packages/agent-inject-web/src/pages/index.html'
import {
  tryWasmAsset,
  wasmResponse,
  withGzip,
  writeWasmPath,
  WASM_FILE,
  type WasmAsset,
} from './wasm-asset.ts'

const WASM_PATH = fileURLToPath(WASM_FILE)
const WASM_NAME = basename(WASM_PATH)

/** Re-hashed only when the file's `mtime:size` changes. */
let cached: { key: string; asset: WasmAsset } | null = null

async function currentAsset(): Promise<WasmAsset | null> {
  let key: string
  try {
    const info = await stat(WASM_PATH)
    key = `${info.mtimeMs}:${info.size}`
  } catch {
    cached = null
    return null
  }
  if (cached?.key === key) return cached.asset
  const asset = await tryWasmAsset()
  cached = asset ? { key, asset: withGzip(asset) } : null
  return cached?.asset ?? null
}

async function writeCurrentPath(): Promise<WasmAsset | null> {
  const asset = await currentAsset()
  // Content-guarded, so this only writes when the hash moved; an
  // unconditional write into `packages/` would make `--hot` rebuild in a loop.
  if (asset) await writeWasmPath(asset)
  return asset
}

const initial = await writeCurrentPath()
if (!initial) {
  console.error('wasm missing — run `cargo task web-wasm` first')
  process.exit(1)
}

const BASE_PORT = 3000
const PORT_ATTEMPTS = 10

const serve = (port: number) =>
  Bun.serve({
    port,
    routes: {
      '/wasm/:name': async (req) => {
        const current = await currentAsset()
        if (current && req.params.name === current.name) {
          return wasmResponse(current, req.headers.get('accept-encoding'))
        }
        // Never the SPA shell: a page asking for a hash we do not have is stale.
        return new Response(
          `no such wasm build: ${req.params.name}\ncurrent: ${current?.name ?? 'none'}\n` +
            'this page predates the current build; hard-refresh it\n',
          { status: 404, headers: { 'content-type': 'text/plain;charset=utf-8' } },
        )
      },
      '/app': index,
      '/app/*': index,
    },
    fetch: () => Response.redirect('/app', 302),
    development: { hmr: true, console: true },
  })

// An explicit PORT stays strict: `dev-phone.ts` sets it and then polls that
// exact port, so moving up the ladder would break it silently.
function serveOnFreePort() {
  if (process.env.PORT) return serve(Number(process.env.PORT))
  for (let port = BASE_PORT; port < BASE_PORT + PORT_ATTEMPTS; port++) {
    try {
      return serve(port)
    } catch (err) {
      if ((err as { code?: string }).code !== 'EADDRINUSE') throw err
    }
  }
  console.error(`ports ${BASE_PORT}–${BASE_PORT + PORT_ATTEMPTS - 1} are all in use; set PORT`)
  process.exit(1)
}

const server = serveOnFreePort()

// Watch the directory rather than the file: a rebuild replaces the file, and a
// watch on the path would follow the old inode. Debounced because
// wasm-bindgen writes in stages.
try {
  let pending: ReturnType<typeof setTimeout> | null = null
  watch(dirname(WASM_PATH), (_event, filename) => {
    if (filename && filename !== WASM_NAME) return
    if (pending) clearTimeout(pending)
    pending = setTimeout(() => {
      pending = null
      void writeCurrentPath().then((asset) => {
        console.log(asset ? `  wasm ${asset.name}` : '  wasm missing')
      })
    }, 150)
  }).unref()
} catch {
  console.warn('  wasm rebuilds will not hot-reload — could not watch the build directory')
}

console.log(`dev ${server.url}`)
console.log(`  wasm ${initial.name}`)
