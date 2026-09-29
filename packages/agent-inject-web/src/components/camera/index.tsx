/**
 * Take many photos in a row. Each shutter press is handed to `onShot` at once,
 * so it starts uploading while the next one is framed.
 *
 * The live preview needs `getUserMedia`. Where that is missing or refused, the
 * sheet offers the phone's own camera app instead, one shot per press.
 */

import { Button, Stack, Text } from 'moonspace-dom'
import { component, signal } from 'visage-dom'

import {
  cameraFileName,
  canUseLiveCamera,
  captureFrame,
  startStream,
  stopStream,
} from '../../lib/camera/index.ts'
import { isAbort, pickFiles } from '../../lib/pick-files/index.ts'

export interface CameraSheetProps {
  onShot: (file: File) => void
  onClose: () => void
}

type Mode = 'starting' | 'live' | 'fallback'

export const CameraSheet = component(function* (props: CameraSheetProps) {
  const ctx = this
  const mode = signal<Mode>(canUseLiveCamera() ? 'starting' : 'fallback')
  const shots = signal(0)
  const note = signal<string | null>(null)
  let stream: MediaStream | null = null
  let video: HTMLVideoElement | null = null

  function attach(): void {
    if (!video || !stream) return
    video.srcObject = stream
    void video.play().catch(() => {})
  }

  if (mode.peek() === 'starting') {
    startStream().then(
      (started) => {
        if (ctx.aborted.aborted) {
          stopStream(started)
          return
        }
        stream = started
        mode.value = 'live'
        attach()
      },
      (error: unknown) => {
        if (ctx.aborted.aborted) return
        note.value = `Live camera unavailable (${String(error)}). Using the camera app.`
        mode.value = 'fallback'
      },
    )
  }
  ctx.aborted.addEventListener('abort', () => {
    if (stream) stopStream(stream)
  })

  function hand(file: File): void {
    shots.value += 1
    props.onShot(file)
  }

  function shutter(): void {
    if (!video) return
    const name = cameraFileName(new Date(), shots.peek() + 1)
    captureFrame(video).then(
      (blob) => hand(new File([blob], name, { type: 'image/jpeg' })),
      (error: unknown) => {
        note.value = String(error)
      },
    )
  }

  function cameraApp(): void {
    pickFiles({ accept: 'image/*', capture: 'environment', multiple: false }).then(
      (files) => {
        for (const file of files) hand(file)
      },
      (error: unknown) => {
        if (!isAbort(error)) note.value = String(error)
      },
    )
  }

  yield () => (
    <Stack direction="column" gap={1} data-testid="camera-sheet">
      {mode.value === 'fallback' ? null : (
        <video
          ref={(element: HTMLVideoElement) => {
            video = element
            element.muted = true
            element.playsInline = true
            element.autoplay = true
            attach()
          }}
          style={{ width: '100%', maxHeight: '60vh', background: 'black' }}
        />
      )}
      {note.value ? <Text color="warning">{note.value}</Text> : null}
      <Stack direction="row" gap={1}>
        {mode.value === 'fallback' ? (
          <Button variant="primary" data-testid="camera-app" onclick={() => cameraApp()}>
            Take photo
          </Button>
        ) : (
          <Button
            variant="primary"
            data-testid="shutter"
            disabled={mode.value !== 'live'}
            onclick={() => shutter()}
          >
            Shutter
          </Button>
        )}
        <Button variant="secondary" data-testid="camera-done" onclick={() => props.onClose()}>
          Done
        </Button>
        <Text color="fgMuted">{shots.value === 1 ? '1 photo' : `${shots.value} photos`}</Text>
      </Stack>
    </Stack>
  )
})
