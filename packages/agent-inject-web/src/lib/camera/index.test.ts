import { afterEach, expect, test } from 'bun:test'

import { cameraFileName, canUseLiveCamera } from './index.ts'

const secure = Object.getOwnPropertyDescriptor(globalThis, 'isSecureContext')
const devices = Object.getOwnPropertyDescriptor(navigator, 'mediaDevices')

afterEach(() => {
  if (secure) Object.defineProperty(globalThis, 'isSecureContext', secure)
  else delete (globalThis as { isSecureContext?: boolean }).isSecureContext
  if (devices) Object.defineProperty(navigator, 'mediaDevices', devices)
  else delete (navigator as { mediaDevices?: unknown }).mediaDevices
})

function setup(isSecure: boolean, mediaDevices: unknown): void {
  Object.defineProperty(globalThis, 'isSecureContext', { value: isSecure, configurable: true })
  Object.defineProperty(navigator, 'mediaDevices', { value: mediaDevices, configurable: true })
}

test('names a shot by local time and batch position', () => {
  const date = new Date(2026, 8, 28, 14, 15, 3)
  expect(cameraFileName(date, 1)).toBe('photo-20260928-141503-001.jpg')
  expect(cameraFileName(date, 12)).toBe('photo-20260928-141503-012.jpg')
})

test('the live camera needs a secure context and getUserMedia', () => {
  setup(true, { getUserMedia: () => Promise.resolve() })
  expect(canUseLiveCamera()).toBe(true)
  setup(false, { getUserMedia: () => Promise.resolve() })
  expect(canUseLiveCamera()).toBe(false)
  setup(true, undefined)
  expect(canUseLiveCamera()).toBe(false)
})
