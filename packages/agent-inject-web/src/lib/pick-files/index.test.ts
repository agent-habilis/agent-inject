/**
 * The picker has to settle exactly once, leave nothing behind, and open the
 * dialog inside the click that asked for it.
 */

import { beforeEach, expect, test } from 'bun:test'

import { isAbort, pickFiles } from './index.ts'

beforeEach(() => {
  document.body.innerHTML = ''
})

function openInput(): HTMLInputElement {
  const input = document.body.querySelector('input[type=file]')
  if (!input) throw new Error('the picker put no input in the document')
  return input as HTMLInputElement
}

/** `input.files` is read-only, so a test supplies its own list. */
function choose(input: HTMLInputElement, files: File[]): void {
  Object.defineProperty(input, 'files', { value: files, configurable: true })
  input.dispatchEvent(new Event('change'))
}

test('sets accept, capture and multiple as asked', async () => {
  const picked = pickFiles({ accept: 'image/*', capture: 'environment', multiple: true })
  const input = openInput()
  expect(input.accept).toBe('image/*')
  expect(input.getAttribute('capture')).toBe('environment')
  expect(input.multiple).toBe(true)
  input.dispatchEvent(new Event('cancel'))
  await picked.catch(() => {})

  const plain = pickFiles({ multiple: false })
  const second = openInput()
  expect(second.accept).toBe('')
  expect(second.hasAttribute('capture')).toBe(false)
  expect(second.multiple).toBe(false)
  second.dispatchEvent(new Event('cancel'))
  await plain.catch(() => {})
})

test('opens the dialog synchronously', () => {
  let clicked = false
  const original = HTMLInputElement.prototype.click
  HTMLInputElement.prototype.click = function () {
    clicked = true
  }
  try {
    const picked = pickFiles({ multiple: true })
    expect(clicked).toBe(true)
    openInput().dispatchEvent(new Event('cancel'))
    void picked.catch(() => {})
  } finally {
    HTMLInputElement.prototype.click = original
  }
})

test('resolves with what was chosen and removes the input', async () => {
  const picked = pickFiles({ multiple: true })
  const input = openInput()
  const file = new File(['hi'], 'a.txt')
  choose(input, [file])
  expect(await picked).toEqual([file])
  expect(input.isConnected).toBe(false)
})

test('a cancel rejects with an AbortError', async () => {
  const picked = pickFiles({ multiple: true })
  openInput().dispatchEvent(new Event('cancel'))
  const error = await picked.catch((caught: unknown) => caught)
  expect(isAbort(error)).toBe(true)
  expect(document.body.querySelector('input')).toBeNull()
})

test('an empty selection is a cancel', async () => {
  const picked = pickFiles({ multiple: true })
  choose(openInput(), [])
  expect(isAbort(await picked.catch((caught: unknown) => caught))).toBe(true)
})
