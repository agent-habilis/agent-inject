import { afterEach, beforeEach, expect, test } from 'bun:test'
import { flushSync, render } from 'visage-dom'
import type { Root } from 'visage-dom'

import { CameraSheet } from './index.tsx'

let host: HTMLElement
let root: Root | null = null

beforeEach(() => {
  document.body.innerHTML = ''
  host = document.createElement('div')
  document.body.appendChild(host)
  // No secure context in happy-dom, so the sheet takes the camera-app path.
  Object.defineProperty(globalThis, 'isSecureContext', { value: false, configurable: true })
})

afterEach(() => {
  for (const input of document.querySelectorAll('input[type=file]')) {
    input.dispatchEvent(new Event('cancel'))
  }
  root?.unmount()
  root = null
})

function testId(id: string): HTMLElement {
  const found = host.querySelector<HTMLElement>(`[data-testid="${id}"]`)
  if (!found) throw new Error(`no ${id}`)
  return found
}

test('without a live camera, each press opens the camera app and hands the shot on', async () => {
  const shots: File[] = []
  root = render(CameraSheet({ onShot: (file) => shots.push(file), onClose: () => {} }), host)
  flushSync()

  expect(host.querySelector('video')).toBeNull()
  testId('camera-app').click()
  const input = document.querySelector<HTMLInputElement>('input[type=file]')
  expect(input?.getAttribute('capture')).toBe('environment')
  expect(input?.accept).toBe('image/*')

  const photo = new File(['jpeg'], 'IMG_0001.JPG')
  Object.defineProperty(input, 'files', { value: [photo], configurable: true })
  input?.dispatchEvent(new Event('change'))
  await Promise.resolve()
  flushSync()

  expect(shots).toEqual([photo])
  expect(testId('camera-sheet').textContent).toContain('1 photo')
})

test('Done closes the sheet', () => {
  let closed = 0
  root = render(CameraSheet({ onShot: () => {}, onClose: () => (closed += 1) }), host)
  flushSync()
  testId('camera-done').click()
  expect(closed).toBe(1)
})
