/**
 * `bun run start`: serve a production `dist/` build. Run `bun run build` first.
 * Checks that the wasm in `dist/` is the one the current crate build produced.
 */

import { createFetch, distFile, PORT } from './serve.ts'
import { wasmAsset } from './wasm-asset.ts'

if (!(await distFile('/app/index.html').exists())) {
  console.error('dist/ missing — run `bun run build` first')
  process.exit(1)
}

const asset = await wasmAsset()
if (!(await distFile(asset.path).exists())) {
  console.error(`dist/ has no ${asset.name} — the wasm changed since \`bun run build\`; rebuild`)
  process.exit(1)
}

const server = Bun.serve({ port: PORT, fetch: createFetch() })
console.log(`start ${server.url}`)
