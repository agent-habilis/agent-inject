import { expect, test } from 'bun:test'

import { looksLikeTicket, ticketFromPath } from './index.ts'

test('reads the ticket out of an inject path', () => {
  expect(ticketFromPath('/app/inject/3xYz')).toBe('3xYz')
  expect(ticketFromPath('/app/inject/3xYz/')).toBe('3xYz')
})

test('rejects anything that is not one inject route', () => {
  for (const path of ['/', '/app', '/app/inject', '/app/files/3xYz', '/app/inject/3xYz/extra', '/x/inject/3xYz']) {
    expect(ticketFromPath(path)).toBeNull()
  }
})

test('Base58 excludes 0, O, I and l', () => {
  expect(looksLikeTicket('abc123')).toBe(true)
  for (const bad of ['', '0abc', 'Oabc', 'Iabc', 'labc', 'ab c', 'ab%20']) {
    expect(looksLikeTicket(bad)).toBe(false)
  }
})
