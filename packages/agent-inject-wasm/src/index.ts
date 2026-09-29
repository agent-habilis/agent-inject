/**
 * Load the wasm-bindgen client once per page.
 *
 * The glue's default `new URL("…_bg.wasm", import.meta.url)` resolves to a
 * `file://` path under Bun's dev server, which browsers refuse to fetch, so
 * the content-addressed HTTP path is passed in. See `scripts/wasm-asset.ts`
 * for why the name carries a hash.
 */

import type * as WasmExports from './glue/agent_inject_wasm_client.js'
import { WASM_PATH } from './path.ts'

export type WasmModule = typeof WasmExports

/**
 * Caching the *promise* is load-bearing. `__wbg_init` sets its module-global
 * only after its await, so two overlapping calls would each build a
 * `WebAssembly.Instance` sharing one glue module, and pointers from the first
 * would be read against the second's memory.
 */
let wasmModule: Promise<WasmModule> | null = null

export function loadWasm(): Promise<WasmModule> {
  if (!wasmModule) {
    wasmModule = import('./glue/agent_inject_wasm_client.js').then(async (module) => {
      await module.default({ module_or_path: WASM_PATH })
      return module
    })
    // A failed load must not poison every later attempt.
    wasmModule.catch(() => {
      wasmModule = null
    })
  }
  return wasmModule
}
