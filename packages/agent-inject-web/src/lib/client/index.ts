/**
 * The page's view of one connection: the wasm client behind the `Uploader`
 * the queue drives.
 */

import { loadWasm } from 'agent-inject-wasm'

import type { Uploader } from '../upload-queue/index.ts'

export interface Connection extends Uploader {
  /** `webrtc` or `relay`: the path the uploads ride. */
  readonly dataPath: string
  /** Resolves with the reason when the connection ends. */
  closed(): Promise<string>
  close(): Promise<void>
}

/**
 * `?transport=relay` skips WebRTC, to test the relay fallback from a network
 * where ICE works. A debugging switch, so it rides the query string.
 */
export function relayOnly(search: string = window.location.search): boolean {
  return new URLSearchParams(search).get('transport') === 'relay'
}

export async function connect(ticket: string): Promise<Connection> {
  const wasm = await loadWasm()
  const client = await wasm.InjectClient.connect(ticket, relayOnly())
  return {
    dataPath: client.dataPath(),
    upload: (name, blob, uploadId, onProgress) => client.upload(name, blob, uploadId, onProgress),
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
