---
name: inject-files
description: Receive files from your phone into this session.
allowed-tools: Bash, Read
---

# /inject-files

The user sends files from a phone into this session. `agent-inject` runs in
the background and shows a link and a QR code. The user opens the link on the
phone, adds files, and taps **Done**. Then `agent-inject` stops, and its exit
wakes you with the list of saved files.

This skill needs a Bash command that runs in the background and wakes you when
it exits (in Claude Code, `run_in_background: true`).

## 1. Make a folder for the logs

```bash
S=$(mktemp -d "${TMPDIR:-/tmp}/inject-files.XXXXXX") && echo "$S"
```

Keep the printed path. The next steps call it `<S>`. It holds only the
output of the receiver. The files go to a session folder that the receiver
makes: `/tmp/agent-inject/<session-id>/`, or `$AGENT_INJECT_DIR/<session-id>/`
when that variable is set.

## 2. Start the receiver in the background

Run this command with `run_in_background: true`. Do not wait for it.

```bash
agent-inject --accept files --output json > "<S>/out.jsonl" 2> "<S>/err.log"
```

`--accept files` shows only an **Add files** button and **Done** on the
phone. The phone can send any type of file.

## 3. Show the link and the QR code

Run this command in the foreground. It waits up to 10 s for the link.

```bash
S="<S>"
for _ in $(seq 100); do grep -q '"url"' "$S/out.jsonl" && break; sleep 0.1; done
sed -n 's/.*"url":"\([^"]*\)".*/\1/p' "$S/out.jsonl"
sed -n '1s/.*"dir":"\([^"]*\)".*/\1/p' "$S/out.jsonl"
printf '%b\n' "$(sed -n '1s/.*"qr":"\([^"]*\)".*/\1/p' "$S/out.jsonl")"
cat "$S/err.log"
```

- If the output has no link, the receiver failed to start. Show the text of
  `err.log` to the user exactly as it is, for example `could not reach a
  relay`. Then stop. Do not try again without a change.
- If the output has a link, the user cannot see the tool output. Copy the QR
  code into your reply, in a code block, with no change to any character. Put
  the link below it. Keep the second line of the output: it is the session
  folder, called `<dir>` below.

Tell the user to open the link on the phone, add the files, and tap **Done**.
Then end your turn.

## 4. When the receiver exits

The exit of the background command wakes you. Read the last line:

```bash
tail -n 1 "<S>/out.jsonl"; cat "<S>/err.log"
```

- `{"done":{"dir":…,"files":[…]}}`: the user tapped Done. `files` holds the
  absolute path of each file, in the order that it arrived.
  - If `files` is empty, tell the user that no files arrived. Then stop.
  - By default, give the paths to the user.
  - If the current task needs what a file holds, read it with the Read tool.
    Skip binary files that Read cannot show.
- Any other last line: the session stopped before Done. Show `err.log` to the
  user exactly as it is. The files that arrived are in `<dir>`.

## Rules

- If the user cancels before Done, stop the background task.
- The files stay in `<dir>`. Move or copy them only when the task needs it.
- One session per run. To get more files, run the skill again.
