/**
 * The files waiting to go, going, and gone. No DOM and no wasm: the page hands
 * it an `Uploader` while connected and `null` while not, and renders whatever
 * `onChange` reports.
 *
 * Every file gets a random upload id once, when it is added. A retry reuses
 * it, so if the receiver saved the file and only the answer was lost, the
 * retry is answered with the name already saved instead of making a copy.
 */

export interface Uploader {
  /** Resolves with the name the receiver saved the file under. */
  upload(
    name: string,
    blob: Blob,
    uploadId: Uint8Array,
    onProgress: (sent: number, total: number) => void,
  ): Promise<string>
}

export type Status = 'queued' | 'uploading' | 'saved' | 'failed'

export interface Item {
  readonly id: number
  readonly name: string
  readonly size: number
  readonly blob: Blob
  readonly uploadId: Uint8Array
  status: Status
  sent: number
  savedAs?: string
  error?: string
}

export interface QueueOptions {
  /** Uploads in flight at once. Each is its own QUIC stream. */
  concurrency: number
  onChange: (items: readonly Item[]) => void
  /** Injected by tests; the page uses the default. */
  newUploadId?: () => Uint8Array
}

function randomUploadId(): Uint8Array {
  return crypto.getRandomValues(new Uint8Array(16))
}

export class UploadQueue {
  readonly #items: Item[] = []
  readonly #options: QueueOptions
  #uploader: Uploader | null = null
  #active = 0
  #nextId = 1

  constructor(options: QueueOptions) {
    this.#options = options
  }

  get items(): readonly Item[] {
    return this.#items
  }

  add(files: readonly { name: string; blob: Blob }[]): void {
    const newId = this.#options.newUploadId ?? randomUploadId
    for (const { name, blob } of files) {
      this.#items.push({
        id: this.#nextId++,
        name,
        size: blob.size,
        blob,
        uploadId: newId(),
        status: 'queued',
        sent: 0,
      })
    }
    this.#changed()
    this.#pump()
  }

  /** Start sending on `uploader`, or hold everything while `null`. */
  setUploader(uploader: Uploader | null): void {
    this.#uploader = uploader
    this.#pump()
  }

  retry(id: number): void {
    const item = this.#items.find((candidate) => candidate.id === id)
    if (item?.status !== 'failed') return
    this.#requeue(item)
    this.#changed()
    this.#pump()
  }

  /** Put every failed file back in line, as after a reconnect. */
  retryFailed(): void {
    for (const item of this.#items) {
      if (item.status === 'failed') this.#requeue(item)
    }
    this.#changed()
    this.#pump()
  }

  #requeue(item: Item): void {
    item.status = 'queued'
    item.sent = 0
    delete item.error
  }

  #pump(): void {
    while (this.#uploader && this.#active < this.#options.concurrency) {
      const next = this.#items.find((item) => item.status === 'queued')
      if (!next) return
      void this.#send(next, this.#uploader)
    }
  }

  async #send(item: Item, uploader: Uploader): Promise<void> {
    this.#active += 1
    item.status = 'uploading'
    this.#changed()
    try {
      item.savedAs = await uploader.upload(item.name, item.blob, item.uploadId, (sent) => {
        item.sent = sent
        this.#changed()
      })
      item.status = 'saved'
      item.sent = item.size
    } catch (error) {
      item.status = 'failed'
      item.error = error instanceof Error ? error.message : String(error)
    } finally {
      this.#active -= 1
    }
    this.#changed()
    this.#pump()
  }

  #changed(): void {
    this.#options.onChange([...this.#items])
  }
}
