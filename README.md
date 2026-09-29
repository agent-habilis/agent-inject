# agent-inject

Send photos and files from a phone to a directory on your computer, peer to
peer. The agent on the computer reads the files from that directory.

The phone needs no app. It opens a web page. The bytes go over a WebRTC data
channel directly to the computer. The relay only helps the two ends find each
other. It never carries file data.

## Usage

```sh
agent-inject <dir>
```

1. Run the command. It prints a link and a QR code.
2. Scan the QR code with the phone.
3. On the page, use one of the three buttons:
   - **Add photo** — pick photos from the library.
   - **Camera** — take many photos in a row. Each photo uploads as soon as you
     take it.
   - **Add file** — pick any file.
4. Each saved file prints as one absolute path on stdout.
5. Press ctrl-c to stop.

If a file with the same name exists, the new file gets a suffix (`a-2.jpg`).
agent-inject never overwrites a file.

### Output

stdout carries the link, the QR code, and one line per saved file. stderr
carries errors only.

For a script or an agent, use `--output json`. Then stdout has one JSON object
per line: `{"url": …}` first, then `{"path": …}` for each file.

| Flag | Effect |
|---|---|
| `--output human\|json` | Output format. Default: `human`. |
| `--no-qr` | Do not print the QR code. |

`AGENT_INJECT_WEB_ORIGIN` sets the origin of the web page in the link, for a
dev server or a tunnel.

## Development

Prerequisites: Rust (the toolchain in `rust-toolchain.toml`), the
`wasm32-unknown-unknown` target, the `wasm-bindgen` CLI, Bun, and on macOS
`brew install llvm` (Apple clang cannot build `ring` for wasm).

```sh
cargo task ci         # the full gate: names, fmt, clippy, tests, web, wasm
cargo task web-wasm   # build the browser client into packages/agent-inject-wasm
bun run dev           # web app with hot reload on http://localhost:3000/app
bun run build         # production bundle into dist/
bun run start         # serve dist/
```

To try the whole path on one machine:

```sh
bun run build && PORT=3417 bun run start
AGENT_INJECT_WEB_ORIGIN=http://localhost:3417 cargo run -- /tmp/inbox
```

The camera needs a secure context. `localhost` is one. A phone needs HTTPS.

### Layout

| Path | What it is |
|---|---|
| `crates/agent-inject` | The CLI receiver. |
| `crates/agent-inject-proto` | The wire format: ticket and upload framing. Builds for wasm32. |
| `crates/agent-inject-wasm-client` | The browser client. A separate workspace, wasm32 only. |
| `packages/agent-inject-web` | The phone page. |
| `packages/agent-inject-wasm` | Loads the wasm client. |
| `packages/visage-*`, `packages/moonspace*` | Vendored UI kit. See `docs/vendoring.md`. |
| `tasks/` | `cargo task`. |

The transport comes from [agent-share](https://github.com/agent-habilis/agent-share):
iroh QUIC over a WebRTC data channel, through
[fofoca](https://github.com/fofoca-network/fofoca).

## License

MIT
