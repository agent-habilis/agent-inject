import { expect, test } from 'bun:test'

import { type Item, type Uploader, UploadQueue } from './index.ts'

interface Call {
  name: string
  uploadId: Uint8Array
  progress: (sent: number, total: number) => void
  resolve: (saved: string) => void
  reject: (error: Error) => void
}

/** An uploader whose calls the test settles by hand. */
function manualUploader(): { uploader: Uploader; calls: Call[] } {
  const calls: Call[] = []
  const uploader: Uploader = {
    upload: (name, _blob, uploadId, progress) =>
      new Promise<string>((resolve, reject) => {
        calls.push({ name, uploadId, progress, resolve, reject })
      }),
  }
  return { uploader, calls }
}

function queue(concurrency = 2): { queue: UploadQueue; latest: () => readonly Item[] } {
  let latest: readonly Item[] = []
  let counter = 0
  const created = new UploadQueue({
    concurrency,
    onChange: (items) => {
      latest = items
    },
    newUploadId: () => new Uint8Array(16).fill(++counter),
  })
  return { queue: created, latest: () => latest }
}

function files(...names: string[]): { name: string; blob: Blob }[] {
  return names.map((name) => ({ name, blob: new Blob([name]) }))
}

const tick = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0))

test('holds files until connected, then sends at most `concurrency`', () => {
  const { queue: q, latest } = queue(2)
  const { uploader, calls } = manualUploader()
  q.add(files('a', 'b', 'c'))
  expect(calls).toHaveLength(0)
  expect(latest().map((item) => item.status)).toEqual(['queued', 'queued', 'queued'])

  q.setUploader(uploader)
  expect(calls.map((call) => call.name)).toEqual(['a', 'b'])
  expect(latest().map((item) => item.status)).toEqual(['uploading', 'uploading', 'queued'])
})

test('a finished upload frees a slot for the next', async () => {
  const { queue: q, latest } = queue(1)
  const { uploader, calls } = manualUploader()
  q.setUploader(uploader)
  q.add(files('a', 'b'))
  calls[0]?.resolve('a-2')
  await tick()
  expect(latest()[0]).toMatchObject({ status: 'saved', savedAs: 'a-2', sent: 1 })
  expect(calls.map((call) => call.name)).toEqual(['a', 'b'])
})

test('progress is reported per item', () => {
  const { queue: q, latest } = queue(1)
  const { uploader, calls } = manualUploader()
  q.setUploader(uploader)
  q.add(files('abcdef'))
  calls[0]?.progress(3, 6)
  expect(latest()[0]?.sent).toBe(3)
})

test('one failure does not stop the others', async () => {
  const { queue: q, latest } = queue(2)
  const { uploader, calls } = manualUploader()
  q.setUploader(uploader)
  q.add(files('a', 'b'))
  calls[0]?.reject(new Error('truncated: body did not match'))
  calls[1]?.resolve('b')
  await tick()
  expect(latest()[0]).toMatchObject({ status: 'failed', error: 'truncated: body did not match' })
  expect(latest()[1]?.status).toBe('saved')
})

test('a retry reuses the upload id, so a lost ack cannot duplicate', async () => {
  const { queue: q } = queue(1)
  const { uploader, calls } = manualUploader()
  q.setUploader(uploader)
  q.add(files('a'))
  calls[0]?.reject(new Error('connection lost'))
  await tick()
  q.retry(q.items[0]?.id ?? -1)
  expect(calls).toHaveLength(2)
  expect(calls[1]?.uploadId).toEqual(calls[0]?.uploadId)
})

test('retryFailed puts every failed file back in line after a reconnect', async () => {
  const { queue: q, latest } = queue(2)
  const first = manualUploader()
  q.setUploader(first.uploader)
  q.add(files('a', 'b'))
  first.calls[0]?.reject(new Error('lost'))
  first.calls[1]?.reject(new Error('lost'))
  await tick()
  q.setUploader(null)
  q.retryFailed()
  expect(latest().map((item) => item.status)).toEqual(['queued', 'queued'])

  const second = manualUploader()
  q.setUploader(second.uploader)
  expect(second.calls.map((call) => call.name)).toEqual(['a', 'b'])
})

test('retryFailed leaves a file the session does not accept', async () => {
  const { queue: q, latest } = queue(2)
  const first = manualUploader()
  q.setUploader(first.uploader)
  q.add(files('a.pdf', 'b.jpg'))
  first.calls[0]?.reject(new Error('not_accepted: this session takes photos only'))
  first.calls[1]?.reject(new Error('lost'))
  await tick()
  q.setUploader(null)
  q.retryFailed()
  expect(latest().map((item) => item.status)).toEqual(['failed', 'queued'])
})

test('idle means nothing is queued or uploading; failures do not count', async () => {
  const { queue: q } = queue(1)
  expect(q.idle).toBe(true)
  const { uploader, calls } = manualUploader()
  q.add(files('a', 'b'))
  expect(q.idle).toBe(false)
  q.setUploader(uploader)
  calls[0]?.resolve('a')
  await tick()
  expect(q.idle).toBe(false)
  calls[1]?.reject(new Error('lost'))
  await tick()
  expect(q.idle).toBe(true)
})
