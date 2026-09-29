/**
 * The camera, for taking many photos in one go.
 *
 * The live preview needs `getUserMedia`, which only exists in a secure
 * context. Where it is missing or refused, the page falls back to an
 * `<input capture>`, which opens the phone's own camera app one shot at a
 * time.
 */

/** JPEG quality for a captured frame. */
const JPEG_QUALITY = 0.92

/** Whether a live preview can be tried at all on this page. */
export function canUseLiveCamera(): boolean {
  return (
    globalThis.isSecureContext === true &&
    typeof navigator !== 'undefined' &&
    typeof navigator.mediaDevices?.getUserMedia === 'function'
  )
}

/** The rear camera, if the phone has one. */
export function startStream(): Promise<MediaStream> {
  return navigator.mediaDevices.getUserMedia({
    video: { facingMode: { ideal: 'environment' } },
    audio: false,
  })
}

export function stopStream(stream: MediaStream): void {
  for (const track of stream.getTracks()) track.stop()
}

/** The current frame of `video`, as a JPEG. */
export function captureFrame(video: HTMLVideoElement): Promise<Blob> {
  const canvas = document.createElement('canvas')
  canvas.width = video.videoWidth
  canvas.height = video.videoHeight
  const context = canvas.getContext('2d')
  if (!context) return Promise.reject(new Error('no 2d canvas context'))
  context.drawImage(video, 0, 0)
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => (blob ? resolve(blob) : reject(new Error('could not encode the frame'))),
      'image/jpeg',
      JPEG_QUALITY,
    )
  })
}

/**
 * `photo-20260928-141503-001.jpg`: local time, then the shot's place in the
 * batch, so a batch sorts in the order it was taken.
 */
export function cameraFileName(date: Date, index: number): string {
  const pad = (value: number, width = 2): string => String(value).padStart(width, '0')
  const day = `${date.getFullYear()}${pad(date.getMonth() + 1)}${pad(date.getDate())}`
  const time = `${pad(date.getHours())}${pad(date.getMinutes())}${pad(date.getSeconds())}`
  return `photo-${day}-${time}-${pad(index, 3)}.jpg`
}
