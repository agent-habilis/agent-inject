/**
 * `bun run dev:phone [dir]`: the whole flow, reachable from a phone on your
 * tailnet.
 *
 * 1. Checks that Tailscale runs and the tailnet has HTTPS certificates.
 * 2. Starts the dev server (`scripts/dev.ts`) on localhost, and a proxy in
 *    front of it (see `startProxy`).
 * 3. Publishes the proxy with `tailscale serve` at
 *    `https://<this machine>:8443`. The page needs HTTPS, or the phone refuses
 *    the live camera.
 * 4. Runs `agent-inject <dir>` with that origin, so its link and QR code open
 *    the page on the phone.
 *
 * ctrl-c stops all three and removes the `tailscale serve` entry.
 *
 * Port 8443, not 443, so this never touches anything else served on 443. Only
 * the page goes through `tailscale serve`; file bytes ride WebRTC.
 */

import { $ } from 'bun'

const HTTPS_PORT = 8443
const DEV_PORT = Number(process.env.DEV_PORT ?? 3000)
const PROXY_PORT = DEV_PORT + 1
const DEFAULT_DIR = '/tmp/agent-inject-inbox'
const REPO_ROOT = new URL('../', import.meta.url).pathname

type StatusResult = { dns: string } | { error: string }

/** Read what this script needs out of `tailscale status --json`. */
export function parseStatus(status: unknown): StatusResult {
  const value = (status ?? {}) as {
    BackendState?: string
    CertDomains?: string[] | null
    Self?: { DNSName?: string }
  }
  if (value.BackendState !== 'Running') {
    return { error: `Tailscale is "${value.BackendState ?? 'unknown'}", not Running. Turn Tailscale on.` }
  }
  if (!value.CertDomains || value.CertDomains.length === 0) {
    return { error: 'The tailnet has no HTTPS certificates. Enable HTTPS in the Tailscale admin console (DNS page).' }
  }
  const dns = value.Self?.DNSName?.replace(/\.$/, '')
  if (!dns) return { error: 'Tailscale reports no DNS name for this machine. Enable MagicDNS.' }
  return { dns }
}

/** Whether `tailscale serve status --json` already has a handler on `port`. */
export function portInUse(serveStatus: unknown, port: number): boolean {
  const tcp = (serveStatus as { TCP?: Record<string, unknown> } | null)?.TCP
  return tcp !== undefined && String(port) in tcp
}

/**
 * Headers that make a request from the tailnet look like one from localhost.
 * Bun's dev server answers 403 to any other `Host` on its HTML routes and to
 * any other `Origin` on the HMR socket, and has no option to allow one.
 * Dropping `host` lets `fetch` set the target's own.
 */
export function localHeaders(incoming: Headers, target: URL): Headers {
  const headers = new Headers(incoming)
  headers.delete('host')
  if (headers.has('origin')) headers.set('origin', target.origin)
  const referer = headers.get('referer')
  if (referer) {
    const url = new URL(referer)
    headers.set('referer', new URL(url.pathname + url.search, target).href)
  }
  return headers
}

/**
 * A proxy on 127.0.0.1 that forwards to the dev server with local-looking
 * headers, WebSockets included, so hot reload works on the phone too. It
 * deliberately undoes Bun's DNS-rebinding guard, which is why it only listens
 * on loopback and is only reachable through `tailscale serve`, on the tailnet.
 */
function startProxy(): ReturnType<typeof Bun.serve> {
  const target = new URL(`http://localhost:${DEV_PORT}`)
  type Link = { url: string; headers: Headers; upstream?: WebSocket; pending: (string | Buffer)[] }
  return Bun.serve<Link>({
    hostname: '127.0.0.1',
    port: PROXY_PORT,
    async fetch(req, server) {
      const url = new URL(req.url)
      const upstreamUrl = new URL(url.pathname + url.search, target)
      const headers = localHeaders(req.headers, target)
      if (req.headers.get('upgrade')?.toLowerCase() === 'websocket') {
        upstreamUrl.protocol = 'ws:'
        const upgraded = server.upgrade(req, {
          data: { url: upstreamUrl.href, headers, pending: [] },
        })
        return upgraded ? undefined : new Response('upgrade failed', { status: 400 })
      }
      return fetch(upstreamUrl, {
        method: req.method,
        headers,
        body: req.body,
        redirect: 'manual',
        // Pass compressed bodies through untouched, so `content-encoding`
        // stays true.
        decompress: false,
      })
    },
    websocket: {
      open(client) {
        const headers: Record<string, string> = {}
        client.data.headers.forEach((value, key) => {
          if (!key.startsWith('sec-websocket') && key !== 'upgrade' && key !== 'connection') {
            headers[key] = value
          }
        })
        const upstream = new WebSocket(client.data.url, { headers } as unknown as string[])
        upstream.binaryType = 'nodebuffer'
        upstream.onopen = () => {
          for (const message of client.data.pending) upstream.send(message)
          client.data.pending = []
        }
        upstream.onmessage = (event) => client.send(event.data as string | Buffer)
        upstream.onclose = () => client.close()
        client.data.upstream = upstream
      },
      message(client, message) {
        const upstream = client.data.upstream
        if (upstream?.readyState === WebSocket.OPEN) upstream.send(message)
        else client.data.pending.push(message)
      },
      close(client) {
        client.data.upstream?.close()
      },
    },
  })
}

async function tailscaleBinary(): Promise<string> {
  const onPath = Bun.which('tailscale')
  if (onPath) return onPath
  const app = '/Applications/Tailscale.app/Contents/MacOS/Tailscale'
  if (await Bun.file(app).exists()) return app
  fail('Tailscale is not installed.')
}

function fail(message: string): never {
  console.error(`dev:phone: ${message}`)
  process.exit(1)
}

async function waitForDevServer(): Promise<void> {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    try {
      if ((await fetch(`http://localhost:${DEV_PORT}/app`)).ok) return
    } catch {
      // Not listening yet.
    }
    await Bun.sleep(200)
  }
  fail(`the dev server did not answer on :${DEV_PORT} within 30 s`)
}

async function main(): Promise<void> {
  const dir = process.argv[2] ?? DEFAULT_DIR
  const tailscale = await tailscaleBinary()

  const status = parseStatus(await $`${tailscale} status --json`.nothrow().quiet().json().catch(() => null))
  if ('error' in status) fail(status.error)
  const serveStatus = await $`${tailscale} serve status --json`.nothrow().quiet().json().catch(() => ({}))
  if (portInUse(serveStatus, HTTPS_PORT)) {
    fail(`port ${HTTPS_PORT} is already served (see \`tailscale serve status\`); not touching it`)
  }
  const origin = `https://${status.dns}:${HTTPS_PORT}`

  const dev = Bun.spawn(['bun', '--hot', 'scripts/dev.ts'], {
    cwd: REPO_ROOT,
    env: { ...process.env, PORT: String(DEV_PORT) },
    // The dev server echoes the browser's console here, which is the only
    // way to see what the phone's page logs.
    stdout: 'inherit',
    stderr: 'inherit',
  })
  let published = false
  let session: ReturnType<typeof Bun.spawn> | null = null
  // One teardown, shared: SIGINT kills the session, whose exit calls `stop`
  // again, and that second call must wait for the first to finish removing
  // the serve entry rather than exit under it.
  let teardown: Promise<void> | null = null
  const stop = async (code: number): Promise<never> => {
    teardown ??= (async () => {
      session?.kill('SIGINT')
      dev.kill()
      if (published) await $`${tailscale} serve --https=${HTTPS_PORT} off`.nothrow().quiet()
      // agent-inject closes live connections gracefully, which can take a few
      // seconds; wait so the prompt does not come back while it still runs.
      if (session) await Promise.race([session.exited, Bun.sleep(10_000)])
    })()
    await teardown
    process.exit(code)
  }
  process.on('SIGINT', () => void stop(0))
  process.on('SIGTERM', () => void stop(0))
  void dev.exited.then(() => stop(1))

  await waitForDevServer()
  const proxy = startProxy()
  const serve = await $`${tailscale} serve --bg --https=${HTTPS_PORT} http://127.0.0.1:${proxy.port}`.nothrow().quiet()
  if (serve.exitCode !== 0) {
    console.error(`dev:phone: tailscale serve failed:\n${serve.stderr.toString().trim()}`)
    await stop(1)
  }
  published = true
  console.log(`dev:phone: web app at ${origin}/app (tailnet only)`)

  session = Bun.spawn(['cargo', 'run', '-q', '-p', 'agent-inject', '--', dir], {
    cwd: REPO_ROOT,
    env: { ...process.env, AGENT_INJECT_WEB_ORIGIN: origin },
    stdio: ['inherit', 'inherit', 'inherit'],
  })
  const exit = await session.exited
  await stop(exit)
}

if (import.meta.main) await main()
