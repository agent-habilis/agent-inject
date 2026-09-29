/**
 * The page's view of one connection: the wasm client behind the `Uploader`
 * the queue drives.
 */

import { loadWasm } from 'agent-inject-wasm'

import type { Uploader } from '../upload-queue/index.ts'

export interface Connection extends Uploader {
  /** `webrtc` or `relay`: the path the uploads ride. */
  readonly dataPath: string
  /** Tell the receiver the sender is finished. Resolves with how many files it saved. */
  finish(): Promise<number>
  /** Resolves with the reason when the connection ends. */
  closed(): Promise<string>
  close(): Promise<void>
}

/** Which paths may carry uploads, and which relays the page homes on. */
export interface Overrides {
  webrtc: boolean
  relay: boolean
  /** Empty means the ladder the ticket names. */
  relayUrls: string[]
}

/**
 * Read the debugging overrides from the query string, in fofoca's words:
 *
 * - `?transport=webrtc,relay` — what may carry uploads. Default both, WebRTC
 *   first. A browser has no UDP.
 * - `?lookup=relay` — how the page finds the receiver. A browser has no mDNS
 *   and no DHT, so `relay` is the only valid entry.
 * - `?relay=<url>[,<url>]` — the relays to home on instead of the ticket's.
 *
 * A bad value is an error rather than a silent default, so a typo cannot
 * look like a network problem.
 */
export function parseOverrides(search: string = window.location.search): Overrides | { error: string } {
  const query = new URLSearchParams(search)
  const overrides: Overrides = { webrtc: true, relay: true, relayUrls: [] }

  const transport = query.get('transport')
  if (transport !== null) {
    const names = list(transport)
    for (const name of names) {
      if (name === 'udp') return { error: '?transport=udp: a browser has no UDP' }
      if (name !== 'webrtc' && name !== 'relay') {
        return { error: `?transport=${name}: expected webrtc or relay` }
      }
    }
    overrides.webrtc = names.includes('webrtc')
    overrides.relay = names.includes('relay')
    if (!overrides.webrtc && !overrides.relay) {
      return { error: '?transport= needs webrtc, relay, or both' }
    }
  }

  const lookup = query.get('lookup')
  if (lookup !== null) {
    const names = list(lookup)
    for (const name of names) {
      if (name === 'mdns' || name === 'dht') {
        return { error: `?lookup=${name}: a browser has no ${name === 'mdns' ? 'mDNS' : 'DHT'}` }
      }
      if (name !== 'relay') return { error: `?lookup=${name}: expected relay` }
    }
    if (!names.includes('relay')) return { error: '?lookup= needs relay' }
  }

  const relay = query.get('relay')
  if (relay !== null) {
    for (const raw of list(relay)) {
      let url: URL
      try {
        url = new URL(raw)
      } catch {
        return { error: `?relay=${raw}: not a URL` }
      }
      // A page on HTTPS cannot open a plain-HTTP relay socket.
      if (url.protocol !== 'https:') return { error: `?relay=${raw}: must be https` }
      overrides.relayUrls.push(raw)
    }
  }
  return overrides
}

function list(value: string): string[] {
  return value
    .split(',')
    .map((item) => item.trim())
    .filter((item) => item.length > 0)
}

export async function connect(ticket: string): Promise<Connection> {
  const overrides = parseOverrides()
  if ('error' in overrides) throw new Error(overrides.error)
  const wasm = await loadWasm()
  const client = await wasm.InjectClient.connect(
    ticket,
    overrides.webrtc,
    overrides.relay,
    overrides.relayUrls,
  )
  return {
    dataPath: client.dataPath(),
    upload: (name, blob, uploadId, onProgress) => client.upload(name, blob, uploadId, onProgress),
    finish: () => client.finish(),
    closed: () => client.closed(),
    close: () => client.close(),
  }
}

/** Retry delay for reconnect attempt `attempt` (0-based): 1 s doubling to 30 s,
 * jittered so tabs that lost the receiver together do not redial together. */
export function reconnectDelayMs(attempt: number): number {
  const base = Math.min(30_000, 1_000 * 2 ** attempt)
  return base * (0.5 + Math.random())
}
