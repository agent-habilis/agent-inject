/**
 * The ways in, by what the session takes: photos from the library, the
 * camera, any file. A `files` session shows only **Add files**.
 *
 * Each handler opens its picker before anything else happens. The picker
 * spends the click's user activation, and an `await` in front of it turns the
 * dialog into a `NotAllowedError` in Safari.
 */

import { Button, Stack } from 'moonspace-dom'

import type { Accept } from '../../lib/client/index.ts'
import { isAbort, pickFiles } from '../../lib/pick-files/index.ts'

export interface AddButtonsProps {
  onFiles: (files: File[]) => void
  onCamera: () => void
  /**
   * What the session takes. `images` hides the any-file picker, and `files`
   * hides the photo picker and the camera.
   */
  mode?: Accept
}

function pick(options: Parameters<typeof pickFiles>[0], onFiles: (files: File[]) => void): void {
  pickFiles(options).then(onFiles, (error: unknown) => {
    if (!isAbort(error)) console.warn('[inject] picking files failed', error)
  })
}

export function AddButtons({ onFiles, onCamera, mode = 'any' }: AddButtonsProps) {
  if (mode === 'files') {
    return (
      <Stack direction="column" gap={1}>
        <Button
          variant="secondary"
          block
          data-testid="add-file"
          onclick={() => pick({ multiple: true }, onFiles)}
        >
          Add files
        </Button>
      </Stack>
    )
  }
  return (
    <Stack direction="column" gap={1}>
      <Button
        variant="secondary"
        block
        data-testid="add-photo"
        onclick={() => pick({ accept: 'image/*', multiple: true }, onFiles)}
      >
        Add photo
      </Button>
      <Button variant="secondary" block data-testid="camera" onclick={() => onCamera()}>
        Camera
      </Button>
      {mode === 'images' ? null : (
        <Button
          variant="secondary"
          block
          data-testid="add-file"
          onclick={() => pick({ multiple: true }, onFiles)}
        >
          Add file
        </Button>
      )}
    </Stack>
  )
}
