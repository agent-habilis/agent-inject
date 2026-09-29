import { expect, test } from 'bun:test'

import { parseOverrides } from './index.ts'

test('no query means the defaults: webrtc then relay, the ticket\'s relays', () => {
  expect(parseOverrides('')).toEqual({ webrtc: true, relay: true, relayUrls: [] })
})

test('?transport= names the paths the uploads may ride', () => {
  expect(parseOverrides('?transport=relay')).toEqual({ webrtc: false, relay: true, relayUrls: [] })
  expect(parseOverrides('?transport=webrtc')).toEqual({ webrtc: true, relay: false, relayUrls: [] })
  expect(parseOverrides('?transport=webrtc,relay')).toEqual({
    webrtc: true,
    relay: true,
    relayUrls: [],
  })
})

test('a browser has no UDP, and needs at least one path', () => {
  expect(parseOverrides('?transport=udp')).toHaveProperty('error')
  expect(parseOverrides('?transport=')).toHaveProperty('error')
  expect(parseOverrides('?transport=carrier-pigeon')).toHaveProperty('error')
})

test('?lookup= accepts only relay in a browser', () => {
  expect(parseOverrides('?lookup=relay')).toEqual({ webrtc: true, relay: true, relayUrls: [] })
  expect(parseOverrides('?lookup=relay,mdns')).toHaveProperty('error')
  expect(parseOverrides('?lookup=dht')).toHaveProperty('error')
  expect(parseOverrides('?lookup=')).toHaveProperty('error')
})

test('?relay= overrides which relays the page homes on', () => {
  expect(parseOverrides('?relay=https://relay.agent-habilis.com/,https://b.example')).toEqual({
    webrtc: true,
    relay: true,
    relayUrls: ['https://relay.agent-habilis.com/', 'https://b.example'],
  })
  expect(parseOverrides('?relay=not a url')).toHaveProperty('error')
  expect(parseOverrides('?relay=http://insecure.example')).toHaveProperty('error')
})
