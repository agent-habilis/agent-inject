import { Stack, Text } from 'moonspace-dom'
import { component } from 'visage-dom'

import { Centered } from '../../components/centered/index.tsx'

/** Any URL that is not an inject link. */
export const HelpPage = component(function* () {
  yield () => (
    <Centered>
      <div style={{ padding: '0 2ch', maxWidth: '60ch' }}>
        <Stack direction="column" gap={1}>
          <Text weight="bold">agent-inject</Text>
          <Text color="fgMuted">
            Run `agent-inject &lt;dir&gt;` on the computer, then open the link or scan the QR
            code it prints.
          </Text>
        </Stack>
      </div>
    </Centered>
  )
})
