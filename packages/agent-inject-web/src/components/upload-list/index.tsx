/** One row per file: its name, its size, and where it is. */

import { FileName } from '../file-name/index.tsx'

import { type Item, isRefusedForGood } from '../../lib/upload-queue/index.ts'

export function humanBytes(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return unit === 0 ? `${bytes} B` : `${value.toFixed(1)} ${units[unit]}`
}

function status(item: Item, onRetry: (id: number) => void) {
  switch (item.status) {
    case 'queued':
      return <small class="muted">queued</small>
    case 'uploading':
      return (
        <progress
          value={item.size === 0 ? 0 : item.sent / item.size}
          max={1}
          aria-label={`uploading ${item.name}`}
        />
      )
    case 'saved':
      return (
        <small class="success">
          {item.savedAs && item.savedAs !== item.name ? `saved as ${item.savedAs}` : 'saved'}
        </small>
      )
    case 'failed':
      return (
        <div class="row">
          <small class="danger selectable">{item.error ?? 'failed'}</small>
          {isRefusedForGood(item) ? null : (
            <button
              class="p-button"
              data-variant="outline"
              data-size="small"
              onclick={() => onRetry(item.id)}
            >
              Retry
            </button>
          )}
        </div>
      )
  }
}

export function UploadList({
  items,
  onRetry,
}: {
  items: readonly Item[]
  onRetry: (id: number) => void
}) {
  return (
    <div class="stack" data-testid="upload-list">
      {items.map((item) => (
        <div data-testid="upload-row" data-status={item.status}>
          <div class="row">
            <FileName value={item.name} />
            <small class="muted">{humanBytes(item.size)}</small>
          </div>
          {status(item, onRetry)}
        </div>
      ))}
    </div>
  )
}
