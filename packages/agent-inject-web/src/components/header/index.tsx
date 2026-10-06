/** The top bar: the app's name on the left, the link state on the right. */
import type { Child } from 'visage-dom'

export function Header({ trailing }: { trailing: Child }) {
  return (
    <header class="bar">
      <strong data-testid="inject-brand">agent-inject 💉</strong>
      <div>{trailing}</div>
    </header>
  )
}
