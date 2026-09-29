/**
 * The page a phone opens from the link agent-inject printed: connect to that
 * one session, then send it photos and files.
 *
 * Files can be added before the connection is up; the queue holds them. When
 * the connection drops, the page redials with backoff and puts anything that
 * failed back in line.
 */

import { Box, Button, Stack, Text } from 'moonspace-dom'
import { component, signal } from 'visage-dom'
import { useParams } from 'visage-router'

import { AddButtons } from '../../components/add-buttons/index.tsx'
import { Centered } from '../../components/centered/index.tsx'
import { CameraSheet } from '../../components/camera/index.tsx'
import { FailedBody } from '../../components/failed-body/index.tsx'
import { UploadList } from '../../components/upload-list/index.tsx'
import {
  type Connection,
  connect,
  parseOverrides,
  reconnectDelayMs,
} from '../../lib/client/index.ts'
import { looksLikeTicket } from '../../lib/ticket/index.ts'
import { type Item, UploadQueue, isIdle } from '../../lib/upload-queue/index.ts'

/** Dials in a row that may fail before the page gives up and asks. */
const MAX_FAILED_DIALS = 4

type Phase =
  | { kind: 'connecting' }
  | { kind: 'connected'; dataPath: string }
  | { kind: 'reconnecting'; reason: string }
  | { kind: 'failed'; reason: string }
  | { kind: 'finishing' }
  | { kind: 'finished'; count: number }

export interface InjectSessionProps {
  ticket: string
  connect: (ticket: string) => Promise<Connection>
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms)
    signal.addEventListener(
      'abort',
      () => {
        clearTimeout(timer)
        resolve()
      },
      { once: true },
    )
  })
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

export const InjectSession = component(function* (props: InjectSessionProps) {
  const ctx = this
  const ticket = props.ticket
  const dial = props.connect
  const phase = signal<Phase>(
    looksLikeTicket(ticket)
      ? { kind: 'connecting' }
      : { kind: 'failed', reason: 'This is not an agent-inject link.' },
  )
  const items = signal<readonly Item[]>([])
  const cameraOpen = signal(false)
  const queue = new UploadQueue({
    concurrency: 2,
    onChange: (next) => {
      items.value = next
    },
  })
  let connection: Connection | null = null

  async function run(): Promise<void> {
    let failures = 0
    while (!ctx.aborted.aborted) {
      let opened: Connection
      try {
        opened = await dial(ticket)
      } catch (error) {
        if (ctx.aborted.aborted) return
        failures += 1
        if (failures >= MAX_FAILED_DIALS) {
          phase.value = { kind: 'failed', reason: message(error) }
          return
        }
        phase.value = { kind: 'reconnecting', reason: message(error) }
        await sleep(reconnectDelayMs(failures - 1), ctx.aborted)
        continue
      }
      if (ctx.aborted.aborted) {
        void opened.close()
        return
      }
      failures = 0
      connection = opened
      phase.value = { kind: 'connected', dataPath: opened.dataPath }
      queue.retryFailed()
      queue.setUploader(opened)
      const reason = await opened.closed()
      queue.setUploader(null)
      connection = null
      // After Done the receiver closes the connection on purpose.
      if (ctx.aborted.aborted || isOver()) return
      phase.value = { kind: 'reconnecting', reason }
    }
  }

  function isOver(): boolean {
    const kind = phase.peek().kind
    return kind === 'finishing' || kind === 'finished'
  }

  async function finish(): Promise<void> {
    const current = connection
    if (!current) return
    phase.value = { kind: 'finishing' }
    try {
      phase.value = { kind: 'finished', count: await current.finish() }
    } catch (error) {
      phase.value = { kind: 'failed', reason: message(error) }
    }
  }

  function start(): void {
    phase.value = { kind: 'connecting' }
    void run()
  }

  if (phase.peek().kind === 'connecting') void run()
  ctx.aborted.addEventListener('abort', () => {
    void connection?.close()
  })

  function add(files: readonly File[]): void {
    queue.add(files.map((file) => ({ name: file.name, blob: file })))
  }

  function statusLine() {
    const current = phase.value
    switch (current.kind) {
      case 'connecting':
        return <Text color="fgMuted">connecting…</Text>
      case 'connected':
        return <Text color="success">connected ({current.dataPath})</Text>
      case 'reconnecting':
        return (
          <Text color="warning" class="selectable">
            reconnecting — {current.reason}
          </Text>
        )
      case 'finishing':
        return <Text color="fgMuted">finishing…</Text>
      case 'failed':
      case 'finished':
        return null
    }
  }

  yield () => {
    const current = phase.value
    if (current.kind === 'finished') {
      return (
        <Centered>
          <Stack direction="column" gap={1} data-testid="inject-finished">
            <Text weight="bold" color="success">
              Finished — {current.count === 1 ? '1 file' : `${current.count} files`} sent.
            </Text>
            <Text color="fgMuted">You can close this page.</Text>
          </Stack>
        </Centered>
      )
    }
    if (current.kind === 'failed' && items.value.length === 0) {
      return (
        <Stack direction="column" gap={1} data-testid="inject-failed">
          <FailedBody title="Could not reach agent-inject" reason={current.reason} />
          {looksLikeTicket(ticket) ? (
            <Button variant="secondary" onclick={() => start()}>
              Try again
            </Button>
          ) : null}
        </Stack>
      )
    }
    return (
      <div style={{ padding: '1em 2ch', maxWidth: '72ch', margin: '0 auto' }}>
        <Stack direction="column" gap={1} data-testid="inject-page">
          <Stack direction="row" gap={1}>
            <Text weight="bold">agent-inject</Text>
            {statusLine()}
            {current.kind === 'failed' ? (
              <Button variant="ghost" onclick={() => start()}>
                Reconnect
              </Button>
            ) : null}
          </Stack>
          {cameraOpen.value ? (
            <Box border="line" padX={1} padY={1}>
              <CameraSheet
                onShot={(file: File) => add([file])}
                onClose={() => {
                  cameraOpen.value = false
                }}
              />
            </Box>
          ) : (
            <AddButtons
              onFiles={add}
              onCamera={() => {
                cameraOpen.value = true
              }}
            />
          )}
          <Button
            variant="secondary"
            data-testid="done"
            disabled={current.kind !== 'connected' || !isIdle(items.value)}
            onclick={() => void finish()}
          >
            Done
          </Button>
          <UploadList items={items.value} onRetry={(id) => queue.retry(id)} />
        </Stack>
      </div>
    )
  }
})

/**
 * The route: the ticket from the URL, re-keyed so a new ticket is a new
 * session. A bad override is a mistake in the link, not a network problem, so
 * it fails at once instead of spending the redial budget.
 */
export const InjectPage = component(function* () {
  const params = useParams(this)
  yield () => {
    const ticket = params.value['ticket'] ?? ''
    const overrides = parseOverrides()
    if ('error' in overrides) {
      return <FailedBody title="This link has a bad option" reason={overrides.error} />
    }
    return <InjectSession key={ticket} ticket={ticket} connect={connect} />
  }
})
