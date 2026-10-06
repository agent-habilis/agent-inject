import { component } from 'visage-dom'
import { Header } from '../../components/header/index.tsx'
import { Centered } from '../../components/centered/index.tsx'

/** Any URL that is not an inject link. */
export const HelpPage = component(function* () {
  yield () => (
    <div>
      <Header trailing={null} />
      <Centered>
        <div class="notice stack">
          <strong>agent-inject</strong>
          <p class="muted">
            Run `agent-inject &lt;dir&gt;` on the computer, then open the link or scan the QR
            code it prints.
          </p>
        </div>
      </Centered>
    </div>
  )
})
