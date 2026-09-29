/**
 * Hand out a built `dist/` over HTTP — the rules a static host has to follow,
 * stated once. Used by `start.ts`, and runnable on its own as the production
 * server.
 *
 * - `/wasm/<name>` serves the content-addressed binary as `application/wasm`,
 *   with its precompressed siblings, and 404s a name it does not have rather
 *   than answering with the SPA shell (which the browser would feed to
 *   `WebAssembly.instantiate` as `<!do…`).
 * - `/app/inject/<ticket>` and every other `/app/*` path get the SPA shell.
 *
 * `PORT` picks the port. `DIST_DIR` overrides the directory served.
 */

import { pathToFileURL } from 'node:url'

export const DIST_ROOT = process.env.DIST_DIR
  ? pathToFileURL(`${process.env.DIST_DIR}/`)
  : new URL('../dist/', import.meta.url)

export const PORT = Number(process.env.PORT ?? 3000)

export function distFile(pathname: string, root: URL = DIST_ROOT) {
  return Bun.file(new URL(`.${pathname}`, root))
}

const APP_ROUTE = /^\/app(\/|$)/

/** Bun names chunks `<name>-<hash>.<ext>`, so a far-future expiry is safe. */
const HASHED = /-[a-z0-9]{8,}\.[a-z0-9]+$/
const IMMUTABLE = { 'cache-control': 'public, max-age=31536000, immutable' }
/** The shell names the current chunk graph, so it must never be cached. */
const NO_CACHE = { 'cache-control': 'no-cache' }

export function createFetch(root: URL = DIST_ROOT) {
  const file = (pathname: string) => distFile(pathname, root)
  const shell = file('/app/index.html')

  return async function fetch(req: Request): Promise<Response> {
    const { pathname } = new URL(req.url)

    if (pathname.startsWith('/wasm/')) {
      if (!(await file(pathname).exists())) {
        return new Response(`no such wasm build: ${pathname}\n`, { status: 404 })
      }
      const accepted = req.headers.get('accept-encoding') ?? ''
      const headers: Record<string, string> = {
        'content-type': 'application/wasm',
        ...IMMUTABLE,
        vary: 'accept-encoding',
      }
      for (const [token, suffix] of [
        ['br', '.br'],
        ['gzip', '.gz'],
      ] as const) {
        if (!new RegExp(`\\b${token}\\b`).test(accepted)) continue
        const compressed = file(pathname + suffix)
        if (!(await compressed.exists())) continue
        headers['content-encoding'] = token
        return new Response(compressed, { headers })
      }
      return new Response(file(pathname), { headers })
    }

    if (pathname !== '/' && !pathname.endsWith('/')) {
      const target = file(pathname)
      if (await target.exists()) {
        return new Response(target, { headers: HASHED.test(pathname) ? IMMUTABLE : NO_CACHE })
      }
    }
    if (pathname === '/' || APP_ROUTE.test(pathname)) {
      return new Response(shell, { headers: { ...NO_CACHE, 'content-type': 'text/html;charset=utf-8' } })
    }
    return new Response('Not Found', { status: 404 })
  }
}

if (import.meta.main) {
  const server = Bun.serve({ port: PORT, fetch: createFetch() })
  console.log(`serve ${server.url}`)
}
