# agent-inject

Send photos and files from a phone to a directory on your computer, peer to
peer. The agent on the computer reads the files from that directory.

The phone needs no app. It opens a web page. The bytes go over a WebRTC data
channel directly to the computer. The relay only helps the two ends find each
other. It never carries file data.

## Usage

```sh
agent-inject [dir]
```

With no `dir`, each session saves into a fresh folder,
`/tmp/agent-inject/<session-id>/`. The session id is the UTC start time and
4 random hex digits, for example `2026-10-02T23-21-26-51d3`.
`AGENT_INJECT_DIR` replaces `/tmp/agent-inject` as the base.

1. Run the command. It prints a link and a QR code.
2. Scan the QR code with the phone.
3. On the page, use one of the three buttons:
   - **Add photo** — pick photos from the library.
   - **Camera** — take many photos in a row. Each photo uploads as soon as you
     take it.
   - **Add file** — pick any file.
4. Each saved file prints as one absolute path on stdout.
5. When the phone has sent everything, press **Done** on the page. The
   command prints a last line and stops. Press ctrl-c to stop from the
   computer instead.

If a file with the same name exists, the new file gets a suffix (`a-2.jpg`).
agent-inject never overwrites a file.

### Output

stdout carries the link, the QR code, and one line per saved file. stderr
carries errors only.

For a script or an agent, use `--output json`. Then stdout has one JSON object
per line: `{"url": …, "qr": …, "dir": …}` first, then `{"path": …}` for each
file. `dir` is the folder that the session saves into.
`qr` is the QR code as text, for an agent that runs the command in the
background and shows the code itself. `--no-qr` removes it. When the phone
presses Done, the last line is `{"done": {"dir": …, "files": [ … ]}}`, with
every file of the session, and the command exits 0. After ctrl-c there is no
`done` line, so a caller can tell a finished session from a stopped one.

Done is available only when no file is still uploading. A file that failed
stays out of the result.

| Flag | Effect |
|---|---|
| `--output human\|json` | Output format. Default: `human`. |
| `--no-qr` | Do not print the QR code. |
| `--accept any\|images\|files` | What the phone can send. `images` removes **Add file** from the page, and the command refuses other files. `files` shows only **Add files** and **Done**, with no photo picker and no camera. Default: `any`. |

The link opens `https://inject.agent-habilis.com`. `AGENT_INJECT_WEB_ORIGIN`
sets another origin, for a dev server or a tunnel.

## Agent skills

```sh
agent-inject plug     # install the skills into ~/.claude/skills and ~/.agents/skills
agent-inject unplug   # remove them
```

`--agent claude-code` or `--agent generic` selects one agent. With no flag,
`plug` installs into each agent that it finds on this machine.

- `/inject-photo` receives photos from the phone into the session. The agent
  starts `agent-inject --accept images` in the background and shows the link
  and the QR code. When you press **Done**, the command stops and the agent
  gets the paths of the photos.
- `/inject-files` receives files of any type. The agent starts
  `agent-inject --accept files`. The phone page shows only **Add files** and
  **Done**, and the agent gets the paths of the files.

To try a skill against a local page, run `bun run dev:phone`, and set
`AGENT_INJECT_WEB_ORIGIN=https://<this machine>:8443` in the shell that starts
the agent.

## Development

Prerequisites: Rust (the toolchain in `rust-toolchain.toml`), the
`wasm32-unknown-unknown` target, the `wasm-bindgen` CLI, Bun, and on macOS
`brew install llvm` (Apple clang cannot build `ring` for wasm).

```sh
cargo task ci         # the full gate: names, fmt, clippy, tests, web, wasm
cargo task web-wasm   # build the browser client into packages/agent-inject-wasm
cargo task publish-web-image   # build the web app image (linux/arm64) and push it to the Gitea registry
bun run dev           # web app with hot reload on :3000/app (next free port up to :3009)
bun run build         # production bundle into dist/
bun run start         # serve dist/
```

To try the whole path on one machine:

```sh
bun run build && PORT=3417 bun run start
AGENT_INJECT_WEB_ORIGIN=http://localhost:3417 cargo run -- /tmp/inbox
```

The camera needs a secure context. `localhost` is one. A phone needs HTTPS.

### Deploy the web app

`cargo task publish-web-image` builds the web app as a container image and
pushes it to a container registry, tagged with the short commit sha (`-dirty`
when the image inputs have uncommitted changes) and `latest`. The build is
hermetic: the `Dockerfile` rebuilds the wasm from source. It needs a running
Docker daemon, and a `docker login` to the registry.

```sh
export AGENT_INJECT_REGISTRY=registry.example.com        # or pass --registry
export AGENT_INJECT_REGISTRY_OWNER=my-org                # or pass --owner
cargo task publish-web-image                             # --no-push builds only
```

`--platform` picks `linux/arm64` (the default) or `linux/amd64`. On the host,
copy `deploy/compose.example.yaml`, set the image to the one you pushed, and run
`docker compose up -d`. The file explains the loopback port and the TLS step.

### Test on your phone

Prerequisites:

- Tailscale runs on this machine and on the phone, in the same tailnet.
- HTTPS certificates are enabled for the tailnet (Tailscale admin console, DNS
  page).

```sh
bun run dev:phone [dir]    # no dir: /tmp/agent-inject/<session-id>/
```

1. The command starts the dev server and publishes it on the tailnet at
   `https://<this machine>:8443`. Only devices in the tailnet can open it.
2. Then it runs `agent-inject [dir]` and prints the link and the QR code.
3. Scan the QR code with the phone.
4. Press ctrl-c to stop. The command stops everything and removes the
   `tailscale serve` entry.

Hot reload works on the phone. Only the page goes through `tailscale serve`.
The files go over WebRTC.

If port 8443 is already served, the command stops and changes nothing. Look at
`tailscale serve status`.

### Transports and lookups

The files go over a WebRTC data channel. If WebRTC cannot connect, they go
over the relay. The page shows which one it uses: `connected (webrtc)` or
`connected (relay)`. The page finds the receiver through the relay that the
link names.

To test one path, add a query to the link:

| Query | Effect |
|---|---|
| `?transport=webrtc,relay` | The paths that can carry files. Default: both, WebRTC first. `udp` is not valid, because a browser has no UDP. |
| `?lookup=relay` | How the page finds the receiver. `relay` is the only valid value, because a browser has no mDNS and no DHT. |
| `?relay=<url>[,<url>]` | The relays the page uses, instead of the relays in the link. Each URL must use `https`. |

An option that is not valid stops the page with the reason.

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

The transport is iroh QUIC over a WebRTC data channel, through
[habilis-network](https://github.com/agent-habilis/habilis-network).

## License

MIT
