import { afterEach, beforeEach, expect, test } from 'bun:test'
import { flushSync, render } from 'visage-dom'
import type { Root } from 'visage-dom'

import type { Connection } from '../../lib/client/index.ts'
import { InjectSession } from './index.tsx'

let host: HTMLElement
let root: Root | null = null
const originalClick = HTMLInputElement.prototype.click

beforeEach(() => {
  document.body.innerHTML = ''
  host = document.createElement('div')
  document.body.appendChild(host)
  HTMLInputElement.prototype.click = function () {}
})

afterEach(() => {
  HTMLInputElement.prototype.click = originalClick
  root?.unmount()
  root = null
})

const settle = async (): Promise<void> => {
  for (let round = 0; round < 5; round += 1) await Promise.resolve()
  flushSync()
}

interface FakeConnection extends Connection {
  uploads: string[]
  drop(reason: string): void
}

function fakeConnection(): FakeConnection {
  let drop: (reason: string) => void = () => {}
  const closed = new Promise<string>((resolve) => {
    drop = resolve
  })
  const uploads: string[] = []
  return {
    dataPath: 'relay',
    uploads,
    drop: (reason) => drop(reason),
    upload: (name) => {
      uploads.push(name)
      return Promise.resolve(name)
    },
    closed: () => closed,
    close: () => Promise.resolve(),
  }
}

test('an invalid link fails without dialling', async () => {
  let dials = 0
  root = render(
    InjectSession({
      ticket: 'not/base58!',
      connect: () => {
        dials += 1
        return Promise.reject(new Error('unreachable'))
      },
    }),
    host,
  )
  await settle()
  expect(dials).toBe(0)
  expect(host.querySelector('[data-testid="inject-failed"]')?.textContent).toContain(
    'not an agent-inject link',
  )
})

test('picked files upload once connected, and a dropped link redials', async () => {
  const connections: FakeConnection[] = []
  root = render(
    InjectSession({
      ticket: '3xYz',
      connect: () => {
        const opened = fakeConnection()
        connections.push(opened)
        return Promise.resolve(opened)
      },
    }),
    host,
  )
  await settle()
  expect(host.textContent).toContain('connected (relay)')

  host.querySelector<HTMLElement>('[data-testid="add-file"]')?.click()
  const input = document.querySelector<HTMLInputElement>('input[type=file]')
  Object.defineProperty(input, 'files', {
    value: [new File(['a'], 'a.txt'), new File(['b'], 'b.txt')],
    configurable: true,
  })
  input?.dispatchEvent(new Event('change'))
  await settle()

  expect(connections[0]?.uploads).toEqual(['a.txt', 'b.txt'])
  const statuses = [...host.querySelectorAll<HTMLElement>('[data-testid="upload-row"]')].map(
    (row) => row.dataset['status'],
  )
  expect(statuses).toEqual(['saved', 'saved'])

  // A drop redials straight away; backoff only applies to failed dials.
  connections[0]?.drop('receiver went away')
  await settle()
  expect(connections).toHaveLength(2)
  expect(host.textContent).toContain('connected')
})
