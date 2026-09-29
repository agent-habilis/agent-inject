import { Box, Stack, Text } from 'moonspace-dom'

import { Centered } from '../centered/index.tsx'

export function FailedBody({ title, reason }: { title: string; reason: string }) {
  return (
    <Centered>
      <div style={{ padding: '0 2ch', maxWidth: '60ch' }}>
        <Box border="line" padX={2} padY={1}>
          <Stack direction="column" gap={1}>
            <Text weight="bold" color="danger">
              {title}
            </Text>
            {/* The raw error, which is exactly what gets pasted into a report. */}
            <Text color="fgSubtle" class="selectable">
              {reason}
            </Text>
          </Stack>
        </Box>
      </div>
    </Centered>
  )
}
