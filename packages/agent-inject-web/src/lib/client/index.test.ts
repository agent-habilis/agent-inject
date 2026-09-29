import { expect, test } from 'bun:test'

import { relayOnly } from './index.ts'

test('?transport=relay asks for the relay path only', () => {
  expect(relayOnly('?transport=relay')).toBe(true)
  expect(relayOnly('?transport=webrtc')).toBe(false)
  expect(relayOnly('')).toBe(false)
})
