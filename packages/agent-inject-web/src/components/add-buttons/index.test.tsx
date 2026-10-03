import { afterEach, beforeEach, expect, test } from 'bun:test'
import { flushSync, render } from 'visage-dom'
import type { Root } from 'visage-dom'

import type { Accept } from '../../lib/client/index.ts'
import { AddButtons } from './index.tsx'

let host: HTMLElement
let root: Root | null = null
let clicked: HTMLInputElement[] = []
const originalClick = HTMLInputElement.prototype.click

beforeEach(() => {
  document.body.innerHTML = ''
  host = document.createElement('div')
  document.body.appendChild(host)
  clicked = []
  HTMLInputElement.prototype.click = function () {
    clicked.push(this)
  }
})

afterEach(() => {
  HTMLInputElement.prototype.click = originalClick
  for (const input of document.querySelectorAll('input[type=file]')) {
    input.dispatchEvent(new Event('cancel'))
  }
  root?.unmount()
  root = null
})

function mount(onCamera = (): void => {}, mode: Accept = 'any'): void {
  root = render(AddButtons({ onFiles: () => {}, onCamera, mode }), host)
  flushSync()
}

function button(testId: string): HTMLElement {
  const found = host.querySelector<HTMLElement>(`[data-testid="${testId}"]`)
  if (!found) throw new Error(`no ${testId} button`)
  return found
}

test('Add photo opens an image picker inside the click', () => {
  mount()
  button('add-photo').click()
  // Checked before any microtask runs: the picker must open synchronously.
  expect(clicked).toHaveLength(1)
  expect(clicked[0]?.accept).toBe('image/*')
  expect(clicked[0]?.multiple).toBe(true)
  expect(clicked[0]?.hasAttribute('capture')).toBe(false)
})

test('Add file opens a picker for any file inside the click', () => {
  mount()
  button('add-file').click()
  expect(clicked).toHaveLength(1)
  expect(clicked[0]?.accept).toBe('')
  expect(clicked[0]?.multiple).toBe(true)
})

test('Camera hands off to the camera sheet', () => {
  let opened = 0
  mount(() => {
    opened += 1
  })
  button('camera').click()
  expect(opened).toBe(1)
  expect(clicked).toHaveLength(0)
})

test('a photos-only session has no file picker', () => {
  mount(undefined, 'images')
  expect(host.querySelector('[data-testid="add-file"]')).toBeNull()
  expect(button('add-photo')).toBeTruthy()
  expect(button('camera')).toBeTruthy()
})

test('a files session has only Add files, for any file', () => {
  mount(undefined, 'files')
  expect(host.querySelector('[data-testid="add-photo"]')).toBeNull()
  expect(host.querySelector('[data-testid="camera"]')).toBeNull()
  expect(button('add-file').textContent).toContain('Add files')
  button('add-file').click()
  expect(clicked).toHaveLength(1)
  expect(clicked[0]?.accept).toBe('')
  expect(clicked[0]?.multiple).toBe(true)
})

test('Add photo and Camera share one look, so Done is the only call to action', () => {
  mount()
  expect(button('add-photo').dataset['variant']).toBe(button('camera').dataset['variant'])
})

test('the labels sit in the middle of the buttons', () => {
  mount()
  expect(button('add-photo').dataset['block']).toBe('true')
  expect(button('camera').dataset['block']).toBe('true')
})
