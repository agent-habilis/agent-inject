/**
 * The three ways in: photos from the library, the camera, any file.
 *
 * Each handler opens its picker before anything else happens. The picker
 * spends the click's user activation, and an `await` in front of it turns the
 * dialog into a `NotAllowedError` in Safari.
 */

import { Button, Stack } from 'moonspace-dom'

import { isAbort, pickFiles } from '../../lib/pick-files/index.ts'

export interface AddButtonsProps {
  onFiles: (files: File[]) => void
  onCamera: () => void
  /** Hide the any-file picker: the session takes photos only. */
  photosOnly?: boolean
}

function pick(options: Parameters<typeof pickFiles>[0], onFiles: (files: File[]) => void): void {
  pickFiles(options).then(onFiles, (error: unknown) => {
    if (!isAbort(error)) console.warn('[inject] picking files failed', error)
  })
}

export function AddButtons({ onFiles, onCamera, photosOnly = false }: AddButtonsProps) {
  return (
    <Stack direction="column" gap={1}>
      <Button
        variant="primary"
        data-testid="add-photo"
        onclick={() => pick({ accept: 'image/*', multiple: true }, onFiles)}
      >
        Add photo
      </Button>
      <Button variant="secondary" data-testid="camera" onclick={() => onCamera()}>
        Camera
      </Button>
      {photosOnly ? null : (
        <Button
          variant="secondary"
          data-testid="add-file"
          onclick={() => pick({ multiple: true }, onFiles)}
        >
          Add file
        </Button>
      )}
    </Stack>
  )
}
