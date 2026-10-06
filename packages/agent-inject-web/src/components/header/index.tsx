/** The top bar: the app's name on the left, the link state on the right. */

import { Box, Stack, Text } from 'moonspace-dom'
import type { Child } from 'visage-dom'

export function Header({ trailing }: { trailing: Child }) {
  return (
    <Box background="bgSunken" padX={2} padY={1}>
      <Stack direction="row" justify="between" align="center" gap={2}>
        {/* Never shrinks, so the name never breaks at its hyphen. */}
        <Text weight="bold" data-testid="inject-brand" style={{ flexShrink: 0 }}>
          agent-inject 💉
        </Text>
        {/* minWidth 0 lets a long status truncate instead of pushing the name off the row. */}
        <div style={{ minWidth: 0 }}>{trailing}</div>
      </Stack>
    </Box>
  )
}
