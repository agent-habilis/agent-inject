/**
 * The inject ticket in the page URL: `/app/inject/<ticket>`.
 *
 * The ticket is a bearer capability in one path segment. It is bare ASCII
 * Base58, so it survives a URL path verbatim. Only the shape is checked here;
 * the wasm client decodes it for real.
 */

/** Where the web app is mounted. The router strips and adds it. */
export const APP_BASE = '/app'

const BASE58 = /^[1-9A-HJ-NP-Za-km-z]+$/

/** Whether `ticket` could be a ticket at all: non-empty Base58. */
export function looksLikeTicket(ticket: string): boolean {
  return BASE58.test(ticket)
}

/** The ticket in a `/app/inject/<ticket>` pathname, or `null`. */
export function ticketFromPath(pathname: string): string | null {
  const parts = pathname.replace(/\/+$/, '').split('/').filter(Boolean)
  const [base, view, ticket, ...rest] = parts
  if (`/${base}` !== APP_BASE || view !== 'inject' || !ticket || rest.length > 0) return null
  return looksLikeTicket(ticket) ? ticket : null
}
