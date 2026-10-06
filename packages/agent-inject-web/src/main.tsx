// First, before anything can build a Disposable. See the file for why.
import './compat.ts'

import { component, render } from 'visage-dom'
import { loadWasm } from 'agent-inject-wasm'

import './styles/primitive/primitive.css'
import './app.css'

import { App } from './pages/index.ts'

// Start the wasm fetch and compile now: it is the largest download on the
// connect path, and the promise memo makes the later real call free.
void loadWasm()

const root = document.getElementById('root')
if (!root) throw new Error('#root is missing from index.html')

const Root = component(function* () {
  yield () => (
    <App />
  )
})

render(<Root />, root)
