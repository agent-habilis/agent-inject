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
  finishes: number
  drop(reason: string): void
}

function fakeConnection(): FakeConnection {
  let drop: (reason: string) => void = () => {}
  const closed = new Promise<string>((resolve) => {
    drop = resolve
  })
  const uploads: string[] = []
  const connection: FakeConnection = {
    dataPath: 'relay',
    uploads,
    finishes: 0,
    drop: (reason) => drop(reason),
    finish: () => {
      connection.finishes += 1
      return Promise.resolve(uploads.length)
    },
    upload: (name) => {
      uploads.push(name)
      return Promise.resolve(name)
    },
    closed: () => closed,
    close: () => Promise.resolve(),
  }
  return connection
}

const anyAccept = (): Promise<'any'> => Promise.resolve('any')

/** The visible text of `testId`, without the scoped `<style>` a component renders inside itself. */
function text(testId: string): string | undefined {
  const el = host.querySelector(`[data-testid="${testId}"]`)
  if (!el) return undefined
  const copy = el.cloneNode(true) as Element
  for (const style of copy.querySelectorAll('style')) style.remove()
  return copy.textContent?.trim()
}

function status(): string | undefined {
  return text('inject-status')
}

function doneButton(): HTMLButtonElement {
  const button = host.querySelector<HTMLButtonElement>('[data-testid="done"]')
  if (!button) throw new Error('no done button')
  return button
}

function pick(names: string[]): void {
  host.querySelector<HTMLElement>('[data-testid="add-file"]')?.click()
  const input = [...document.querySelectorAll<HTMLInputElement>('input[type=file]')].pop()
  Object.defineProperty(input, 'files', {
    value: names.map((name) => new File([name], name)),
    configurable: true,
  })
  input?.dispatchEvent(new Event('change'))
}

test('an invalid link fails without dialling', async () => {
  let dials = 0
  root = render(
    InjectSession({
      ticket: 'not/base58!',
      readAccept: anyAccept,
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
      readAccept: anyAccept,
      connect: () => {
        const opened = fakeConnection()
        connections.push(opened)
        return Promise.resolve(opened)
      },
    }),
    host,
  )
  await settle()
  expect(status()).toBe('connected')

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

test('Done waits for uploads, finishes the session, and stops redialling', async () => {
  const connections: FakeConnection[] = []
  let release: () => void = () => {}
  const held = new Promise<void>((resolve) => {
    release = resolve
  })
  root = render(
    InjectSession({
      ticket: '3xYz',
      readAccept: anyAccept,
      connect: () => {
        const opened = fakeConnection()
        const upload = opened.upload
        opened.upload = async (...args) => {
          await held
          return upload(...args)
        }
        connections.push(opened)
        return Promise.resolve(opened)
      },
    }),
    host,
  )
  await settle()
  expect(doneButton().disabled).toBe(true)

  pick(['a.txt'])
  await settle()
  expect(doneButton().disabled).toBe(true)

  release()
  await settle()
  expect(doneButton().disabled).toBe(false)

  doneButton().click()
  await settle()
  expect(connections[0]?.finishes).toBe(1)
  expect(host.textContent).toContain('Finished')
  expect(host.textContent).toContain('1 file sent')
  expect(host.querySelector('[data-testid="add-file"]')).toBeNull()

  // The receiver closes the connection after done; that is not a drop.
  connections[0]?.drop('closed by peer')
  await settle()
  expect(connections).toHaveLength(1)
})

test('a photos-only ticket hides the file picker, also before the mode is known', async () => {
  let known: (accept: 'images') => void = () => {}
  root = render(
    InjectSession({
      ticket: '3xYz',
      readAccept: () =>
        new Promise((resolve) => {
          known = resolve
        }),
      connect: () => Promise.resolve(fakeConnection()),
    }),
    host,
  )
  await settle()
  expect(host.querySelector('[data-testid="add-file"]')).toBeNull()
  known('images')
  await settle()
  expect(host.querySelector('[data-testid="add-file"]')).toBeNull()
  expect(host.querySelector('[data-testid="add-photo"]')).not.toBeNull()
})

test('a files ticket shows only Add files, also before the mode is known', async () => {
  let known: (accept: 'files') => void = () => {}
  root = render(
    InjectSession({
      ticket: '3xYz',
      readAccept: () =>
        new Promise((resolve) => {
          known = resolve
        }),
      connect: () => Promise.resolve(fakeConnection()),
    }),
    host,
  )
  await settle()
  expect(host.querySelector('[data-testid="add-photo"]')).toBeNull()
  expect(host.querySelector('[data-testid="add-file"]')).toBeNull()
  known('files')
  await settle()
  expect(host.querySelector('[data-testid="add-photo"]')).toBeNull()
  expect(host.querySelector('[data-testid="camera"]')).toBeNull()
  expect(host.querySelector('[data-testid="add-file"]')).not.toBeNull()
})

test('the header names the app on the left and the link state on the right', async () => {
  root = render(
    InjectSession({
      ticket: '3xYz',
      readAccept: anyAccept,
      connect: () => Promise.resolve(fakeConnection()),
    }),
    host,
  )
  expect(status()).toBe('connecting…')
  await settle()
  expect(text('inject-brand')).toBe('agent-inject 💉')
  // Which path carries the bytes is a debugging detail; it lives on the title.
  expect(status()).toBe('connected')
  expect(host.querySelector('[data-testid="inject-status"]')?.getAttribute('title')).toBe('over relay')
})

test('Done starts disabled and turns primary once a file is saved', async () => {
  root = render(
    InjectSession({
      ticket: '3xYz',
      readAccept: anyAccept,
      connect: () => Promise.resolve(fakeConnection()),
    }),
    host,
  )
  await settle()
  expect(doneButton().disabled).toBe(true)
  expect(doneButton().dataset['variant']).toBe('secondary')
  expect(doneButton().closest('.buttons')).not.toBeNull()

  pick(['a.jpg'])
  await settle()
  expect(doneButton().disabled).toBe(false)
  // Primitive's primary look is the default, so it carries no variant.
  expect(doneButton().dataset['variant']).toBeUndefined()
})
