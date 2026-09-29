/**
 * Production bundle for the web app into `dist/`.
 *
 * The wasm is written under a content-addressed name so a CDN or browser
 * cannot serve yesterday's build under today's URL. See `wasm-asset.ts`.
 */

import { buildWasm } from './build-wasm.ts'
import { APP_HTML } from './entrypoints.ts'
import { brotli, wasmAsset, writeWasmPath } from './wasm-asset.ts'

await Bun.$`rm -rf dist`

// Every build: cargo is incremental, and bundling yesterday's wasm after a
// Rust edit is the staleness the content-addressed name exists to prevent.
await buildWasm()

// Before the bundle: `agent-inject-wasm` imports the generated path.
const asset = await wasmAsset()
await writeWasmPath(asset)

const result = await Bun.build({
  entrypoints: [APP_HTML],
  outdir: './dist/app',
  minify: true,
  target: 'browser',
})
if (!result.success) {
  for (const log of result.logs) console.error(log)
  process.exit(1)
}

await Bun.write(`./dist${asset.path}`, asset.bytes)
await Bun.write(`./dist${asset.path}.br`, await brotli(asset.bytes))
await Bun.write(`./dist${asset.path}.gz`, Bun.gzipSync(asset.bytes, { level: 9 }))

// Bun writes chunk URLs relative to the page, but the SPA shell answers every
// route: under `/app/inject/<ticket>` a `./chunk-…` would resolve to
// `/app/inject/chunk-…` and 404. Every chunk sits at the root of `dist/app/`.
for (const output of result.outputs) {
  if (output.path.endsWith('.html')) {
    const html = await Bun.file(output.path).text()
    await Bun.write(output.path, html.replace(/(src|href)="(?:\.\.?\/)+/g, '$1="/app/'))
  }
  console.log(`  ${output.path}`)
}
console.log(`  dist${asset.path} (+.br, +.gz)`)
