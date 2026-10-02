import { afterEach, beforeEach, expect, test } from 'bun:test'
import { flushSync, render } from 'visage-dom'
import type { Root } from 'visage-dom'

import type { Item } from '../../lib/upload-queue/index.ts'
import { UploadList, humanBytes } from './index.tsx'

let host: HTMLElement
let root: Root | null = null

beforeEach(() => {
  document.body.innerHTML = ''
  host = document.createElement('div')
  document.body.appendChild(host)
})

afterEach(() => {
  root?.unmount()
  root = null
})

function item(id: number, overrides: Partial<Item>): Item {
  return {
    id,
    name: `file-${id}.jpg`,
    size: 2048,
    blob: new Blob(),
    uploadId: new Uint8Array(16),
    status: 'queued',
    sent: 0,
    ...overrides,
  }
}

test('shows each status, and a retry for a failure', () => {
  const retried: number[] = []
  const items = [
    item(1, { status: 'queued' }),
    item(2, { status: 'uploading', sent: 1024 }),
    item(3, { status: 'saved', savedAs: 'file-3-2.jpg' }),
    item(4, { status: 'failed', error: 'truncated: body did not match' }),
  ]
  root = render(UploadList({ items, onRetry: (id) => retried.push(id) }), host)
  flushSync()

  const rows = [...host.querySelectorAll<HTMLElement>('[data-testid="upload-row"]')]
  expect(rows.map((row) => row.dataset['status'])).toEqual(['queued', 'uploading', 'saved', 'failed'])
  expect(rows[2]?.textContent).toContain('saved as file-3-2.jpg')
  expect(rows[3]?.textContent).toContain('truncated: body did not match')

  rows[3]?.querySelector('button')?.click()
  expect(retried).toEqual([4])
})

test('a file the session does not accept gets no retry, since it would fail again', () => {
  const items = [item(1, { status: 'failed', error: 'not_accepted: this session takes photos only' })]
  root = render(UploadList({ items, onRetry: () => {} }), host)
  flushSync()
  const row = host.querySelector<HTMLElement>('[data-testid="upload-row"]')
  expect(row?.textContent).toContain('this session takes photos only')
  expect(row?.querySelector('button')).toBeNull()
})

test('sizes read in binary units', () => {
  expect(humanBytes(0)).toBe('0 B')
  expect(humanBytes(1536)).toBe('1.5 KB')
  expect(humanBytes(5 * 1024 * 1024)).toBe('5.0 MB')
})
