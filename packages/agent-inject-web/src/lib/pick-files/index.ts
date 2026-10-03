/**
 * Ask for files with `<input type="file">`, the picker every browser has.
 */

export interface PickOptions {
  /** `accept` attribute, e.g. `image/*`. Absent means any file. */
  accept?: string
  /** `capture` attribute: open the camera app instead of the library. */
  capture?: 'environment' | 'user'
  multiple: boolean
}

/**
 * How long to wait after the window regains focus before calling it a cancel.
 * The safety net for the `cancel` event, which Safari only got in 16.4.
 */
const CANCEL_GRACE_MS = 500

/**
 * Open the picker and resolve with what was chosen.
 *
 * **Call this synchronously from a click handler.** `input.click()` spends the
 * transient user activation, and a single `await` above it turns the dialog
 * into a `NotAllowedError` — in Safari only. Everything before the `.click()`
 * below is synchronous; keep it that way.
 *
 * Rejects with an `AbortError` when the pick is cancelled.
 */
export function pickFiles(options: PickOptions): Promise<File[]> {
  const input = document.createElement('input')
  input.type = 'file'
  input.multiple = options.multiple
  if (options.accept) input.accept = options.accept
  if (options.capture) input.setAttribute('capture', options.capture)
  // Off-screen rather than hidden: WebKit has a long history of never firing
  // `change` on an input that is `display: none` or not in the document.
  input.style.cssText = 'position:fixed;left:-9999px;top:0;opacity:0;width:1px;height:1px'
  document.body.appendChild(input)

  const abort = new AbortController()
  const { signal } = abort

  const picked = new Promise<File[]>((resolve, reject) => {
    const settle = (): void => {
      abort.abort()
      input.remove()
    }
    const cancel = (): void => {
      settle()
      reject(new DOMException('The user aborted a request.', 'AbortError'))
    }

    input.addEventListener(
      'change',
      () => {
        const files = [...(input.files ?? [])]
        if (files.length === 0) {
          cancel()
          return
        }
        settle()
        resolve(files)
      },
      { signal },
    )
    input.addEventListener('cancel', cancel, { signal })

    // For browsers older than the `cancel` event: focus coming back with no
    // `change` after it is a dismissal. Armed by the blur, so a window that
    // never blurred cannot cancel its own live pick.
    let dialogTookFocus = false
    window.addEventListener(
      'blur',
      () => {
        dialogTookFocus = true
      },
      { signal },
    )
    window.addEventListener(
      'focus',
      () => {
        if (!dialogTookFocus) return
        setTimeout(() => {
          if (!signal.aborted) cancel()
        }, CANCEL_GRACE_MS)
      },
      { signal },
    )
  })

  input.click()
  return picked
}

/** Whether `error` is a dismissed picker rather than a failure. */
export function isAbort(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'AbortError'
}
