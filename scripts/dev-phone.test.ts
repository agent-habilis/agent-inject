import { expect, test } from 'bun:test'

import { localHeaders, parseStatus, portInUse } from './dev-phone.ts'

const running = {
  BackendState: 'Running',
  CertDomains: ['cg-macbook.tetra-ostrich.ts.net'],
  Self: { DNSName: 'cg-macbook.tetra-ostrich.ts.net.' },
}

test('a running node with certs yields its DNS name without the trailing dot', () => {
  expect(parseStatus(running)).toEqual({ dns: 'cg-macbook.tetra-ostrich.ts.net' })
})

test('a stopped node, missing certs, or no DNS name is refused with a reason', () => {
  expect(parseStatus({ ...running, BackendState: 'Stopped' })).toEqual({
    error: 'Tailscale is "Stopped", not Running. Turn Tailscale on.',
  })
  expect(parseStatus({ ...running, CertDomains: [] })).toHaveProperty('error')
  expect(parseStatus({ ...running, CertDomains: null })).toHaveProperty('error')
  expect(parseStatus({ ...running, Self: { DNSName: '' } })).toHaveProperty('error')
  expect(parseStatus(null)).toHaveProperty('error')
})

test('a port is in use when the serve config has a TCP handler for it', () => {
  expect(portInUse({}, 8443)).toBe(false)
  expect(portInUse({ TCP: { '443': { HTTPS: true } } }, 8443)).toBe(false)
  expect(portInUse({ TCP: { '8443': { HTTPS: true } } }, 8443)).toBe(true)
})

test('proxied requests look local to the dev server', () => {
  const incoming = new Headers({
    host: 'cg-macbook.tetra-ostrich.ts.net:8443',
    origin: 'https://cg-macbook.tetra-ostrich.ts.net:8443',
    referer: 'https://cg-macbook.tetra-ostrich.ts.net:8443/app/inject/abc',
    'accept-encoding': 'br',
  })
  const out = localHeaders(incoming, new URL('http://localhost:3000'))
  expect(out.get('host')).toBeNull()
  expect(out.get('origin')).toBe('http://localhost:3000')
  expect(out.get('referer')).toBe('http://localhost:3000/app/inject/abc')
  expect(out.get('accept-encoding')).toBe('br')
})
