/** One row per file: its name, its size, and where it is. */

import { Button, MiddleTruncate, ProgressBar, Stack, Text } from 'moonspace-dom'

import type { Item } from '../../lib/upload-queue/index.ts'

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
      return <Text color="fgMuted">queued</Text>
    case 'uploading':
      return (
        <ProgressBar
          fluid
          value={item.size === 0 ? 0 : item.sent / item.size}
          label={`uploading ${item.name}`}
        />
      )
    case 'saved':
      return (
        <Text color="success">
          {item.savedAs && item.savedAs !== item.name ? `saved as ${item.savedAs}` : 'saved'}
        </Text>
      )
    case 'failed':
      return (
        <Stack direction="row" gap={1}>
          <Text color="danger" class="selectable">
            {item.error ?? 'failed'}
          </Text>
          <Button variant="ghost" onclick={() => onRetry(item.id)}>
            Retry
          </Button>
        </Stack>
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
    <Stack direction="column" gap={1} data-testid="upload-list">
      {items.map((item) => (
        <Stack direction="column" data-testid="upload-row" data-status={item.status}>
          <Stack direction="row" gap={1} justify="between">
            <MiddleTruncate value={item.name} />
            <Text color="fgMuted">{humanBytes(item.size)}</Text>
          </Stack>
          {status(item, onRetry)}
        </Stack>
      ))}
    </Stack>
  )
}
