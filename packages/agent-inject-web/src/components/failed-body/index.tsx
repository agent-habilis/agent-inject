import { Centered } from '../centered/index.tsx'

export function FailedBody({ title, reason }: { title: string; reason: string }) {
  return (
    <Centered>
      <div class="notice boxed stack">
        <strong class="danger">{title}</strong>
        {/* The raw error, which is exactly what gets pasted into a report. */}
        <p class="muted selectable">{reason}</p>
      </div>
    </Centered>
  )
}
