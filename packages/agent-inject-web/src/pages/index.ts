/**
 * The route table. Paths are relative to `APP_BASE` (`/app`), so
 * `inject/<ticket>` here is `/app/inject/<ticket>` in the address bar.
 * Anything else shows how to get a link.
 */

import { browserHistory, createRouter } from 'visage-router'

import { APP_BASE } from '../lib/ticket/index.ts'
import { HelpPage } from './help/index.tsx'
import { InjectPage } from './inject/index.tsx'

export const App = createRouter({
  routes: [{ path: 'inject/:ticket', component: InjectPage }],
  history: browserHistory({ base: APP_BASE }),
  fallback: HelpPage,
  // The page never scrolls as a whole, so there is no scroll to restore.
  scroll: false,
})
