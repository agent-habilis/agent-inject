import type { Child } from 'visage-dom'

export function Centered({ children }: { children: Child }) {
  return <div class="centered">{children}</div>
}
