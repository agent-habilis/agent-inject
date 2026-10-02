---
name: inject-photo
description: Receive photos from your phone into this session.
allowed-tools: Bash, Read
---

# /inject-photo

The user sends photos from a phone into this session. `agent-inject` runs in
the background and shows a link and a QR code. The user opens the link on the
phone, adds photos, and taps **Done**. Then `agent-inject` stops, and its exit
wakes you with the list of saved files.

This skill needs a Bash command that runs in the background and wakes you when
it exits (in Claude Code, `run_in_background: true`).

## 1. Make the session folder

```bash
S=$(mktemp -d "${TMPDIR:-/tmp}/inject-photo.XXXXXX") && mkdir "$S/photos" && echo "$S"
```

Keep the printed path. The next steps call it `<S>`.

## 2. Start the receiver in the background

Run this command with `run_in_background: true`. Do not wait for it.

```bash
agent-inject "<S>/photos" --accept images --output json > "<S>/out.jsonl" 2> "<S>/err.log"
```

`--accept images` hides the file picker on the phone. The receiver also
refuses any file that is not an image.

## 3. Show the link and the QR code

Run this command in the foreground. It waits up to 10 s for the link.

```bash
S="<S>"
for _ in $(seq 100); do grep -q '"url"' "$S/out.jsonl" && break; sleep 0.1; done
sed -n 's/.*"url":"\([^"]*\)".*/\1/p' "$S/out.jsonl"
printf '%b\n' "$(sed -n '1s/.*"qr":"\([^"]*\)".*/\1/p' "$S/out.jsonl")"
cat "$S/err.log"
```

- If the output has no link, the receiver failed to start. Show the text of
  `err.log` to the user exactly as it is, for example `could not reach a
  relay`. Then stop. Do not try again without a change.
- If the output has a link, the user cannot see the tool output. Copy the QR
  code into your reply, in a code block, with no change to any character. Put
  the link below it.

Tell the user to open the link on the phone, add the photos, and tap **Done**.
Then end your turn.

## 4. When the receiver exits

The exit of the background command wakes you. Read the last line:

```bash
tail -n 1 "<S>/out.jsonl"; cat "<S>/err.log"
```

- `{"done":{"dir":…,"files":[…]}}`: the user tapped Done. `files` holds the
  absolute path of each photo, in the order that it arrived.
  - If `files` is empty, tell the user that no photos arrived. Then stop.
  - By default, give the paths to the user.
  - If the current task needs what the photos show, read each photo with the
    Read tool.
- Any other last line: the session stopped before Done. Show `err.log` to the
  user exactly as it is. The photos that arrived are in `<S>/photos`.

## Rules

- If the user cancels before Done, stop the background task.
- The photos stay in `<S>/photos`. Move or copy them only when the task needs
  it.
- One session per run. To get more photos, run the skill again.
