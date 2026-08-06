# Prior art: how comparable tools drive coding agents

Read directly from the source of three projects that solve some version of
harness-launcher's problem. Claims are tagged **[V]** verified by reading the code in
the repository, **[D]** taken from the project's own documentation or docstrings, and
**[I]** inferred.

Repositories read at clone time, `--depth 1`:

- [`kunchenguid/firstmate`](https://github.com/kunchenguid/firstmate) — shell, an
  "agent distro" rather than an app.
- [`kbwo/ccmanager`](https://github.com/kbwo/ccmanager) — TypeScript, Ink/React TUI.
- [`omnigent-ai/omnigent`](https://github.com/omnigent-ai/omnigent) — Python server
  plus multiple clients; a "meta-harness".

## The headline

The three split cleanly across the decision recorded in
[#18](https://github.com/sylmarien/harness-launcher/issues/18), and **none of them
provides simultaneity** — the guarantee that opening one spawn never costs sight of
the others.

| | terminal hosting | list visible while in a session |
| --- | --- | --- |
| firstmate | tmux (pluggable backends) | no — one window per crewmate |
| ccmanager | owns its own pty | no — modal view state machine |
| omnigent | tmux, for the native path | no — separate clients |

That reframes the prior art. These are not projects that judged embedding a mistake;
they are projects working without our constraint. **The park/unpark cost we
discovered is the price of simultaneity, not the price of tmux.** [I]

## firstmate — drives a multiplexer

**Backend seam.** `bin/backends/{tmux,zellij,cmux,orca,herdr}.sh`, one script per
backend, tmux by hard default. [V] The interface each backend implements is a useful
reference point for our own harness seam: [V]

```
capture(target, lines)            create_task(session, window, proj) -> window id
send_key(target, key)             current_path(target)
send_text_submit(target, text,    send_text_line(target, text)
                 retries,         send_literal(target, text)
                 enter-sleep,     kill(target)
                 settle)          current_command(target)
container_ensure()                classify_process_name(path, argv0) -> agent|shell|other
foreground_comms(target)          agent_state(target)
                                  agent_alive(target)
```

**Parent → child: `send-keys`, with a proof-carrying verdict.** [V]
`fm_tmux_submit_core` types the text literally (`send-keys -t T -l`), sleeps a settle
interval, then sends `Enter` — **retried, Enter only, never retyping the text**. After
each `Enter` it reads the pane back with `capture-pane` and decides whether the
composer cleared: `empty` (submitted), `pending` (proven still there — swallowed), or
`pending-unproven`.

If retries are exhausted on a proven `pending`, it checks whether the pane is busy. The
comment cites opencode 1.18.4: the harness accepts `Enter` mid-turn and *queues* it
while leaving the typed text visible. So busy → report `empty` so the caller does not
double-send; idle → `pending`, a genuine swallow. [D]

**Child → parent: hooks → a durable on-disk queue → a polling daemon.** [V]

- Hook configs are checked in per harness: `.claude/`, `.codex/hooks.json`,
  `.grok/hooks/*.json`, `.pi/extensions/*.ts`. Events used are `SessionStart`,
  `PreToolUse`, `Stop`.
- The hook commands are defensive one-liners: read the payload from stdin, bail unless
  `jq` exists, bail unless the cwd looks like a firstmate root, and **bail unless the
  hook finds itself registered in that very hooks file**. A stale config cannot fire
  against a directory that never opted in.
- The script appends to `$STATE/.wake-queue`, guarded by a lockfile with a stale-lock
  timeout, plus dedup and annotation helpers.
- `fm-supervise-daemon.sh` drains the queue and injects into the first mate's own pane
  through the same `send_text_submit` path — because their UI *is* an agent.
- `fm_pid_identity` reads `/proc/<pid>/stat` field 22 (starttime) plus a hex dump of
  `cmdline`, so a recycled pid cannot impersonate a live watcher.

Despite the "event-driven" framing, the daemon **polls** — a 1 s main loop with
0.1–0.2 s confirm polls. No inotify. [V]

**Liveness, and it is harness-agnostic.** [V] `fm_backend_tmux_agent_state`:

1. `tmux display-message -p -t <target> '#{pane_tty}'` → the pane's tty.
2. `ps -t <tty> -o pid=,pgid=,tpgid=,comm=`, keeping rows where `pgid == tpgid` → the
   **foreground process group**.
3. Classify `comm` *and* `argv0` — two independent sources — as agent / shell / other.
4. Verdict is tri-state plus `missing` and `unreadable`.

Two pieces of reasoning worth importing wholesale, both from the source comments: [D]

- A false `dead` is the only verdict that can launch a **duplicate agent onto a live
  worktree**, so any read failure yields `unreadable`, never `dead`.
- **tmux silently falls back to the active window when a named target is absent**, so
  the window must be confirmed present in `list-windows` before its foreground command
  is trusted — otherwise you are reading some other pane and do not know it.

## ccmanager — owns its pty

**Its own pty layer** (`services/bunTerminal.ts`, written to replace `bun-pty` and
avoid native-library problems), `@xterm/headless` as a shadow screen model, Ink/React
for the UI. [V] Explicitly anti-tmux in its README — "No tmux dependency… completely
self-contained" — positioning against Claude Squad as the tmux-based alternative. [D]

**State detection is screen-scraping, one detector per harness**:
`services/stateDetector/{claude,codex,gemini,cursor,cline,kimi,opencode,github-copilot}.ts`.
[V] The Claude detector matches a set of spinner glyphs, an `…ing…` regex, a
parenthesised token-stats line, and the prompt box's `─` borders — plus a 1500 ms idle
debounce whose comment concedes Claude Code "sometimes appears idle in terminal output
while still actively processing (busy)." [D]

So their harness seam is real, but each implementation is a pile of regexes over
someone else's UI, and it breaks whenever that UI changes. [I]

**The embedding tax is visible in the code.** `Session.tsx` resets kitty keyboard
protocol, `modifyOtherKeys` and focus tracking so they "don't leak into other
sessions", and normalises line endings against cursor drift with `ONLCR` disabled. [V]

**It is modal.** `App.tsx` is a view state machine including `'menu'` and `'session'`,
with an `onReturnToMenu` callback, and `Session.tsx` writes the pty straight through to
stdout rather than rendering it into an Ink pane. In a session, the list is gone. [V]

## omnigent — a meta-harness

**Three integration strategies per harness, all behind one `Executor` interface**: [V]

- `<h>_executor.py` — headless, one `-p … --output-format stream-json` subprocess per
  turn.
- `<h>_sdk_executor.py` — the vendor SDK.
- `<h>_native_executor.py` — types into a **resident TUI** (`kimi_native_executor.py`
  says so in as many words). [D]

This is a stronger answer to "where does the harness seam go" than choosing one
strategy: the seam is the executor, and the integration strategy becomes a property of
an implementation rather than of the architecture. [I]

**Parent → child on the native path is tmux `send-keys`.** From
`claude_native_bridge.py`: web UI messages "are delivered to Claude by typing them into
the same tmux pane the user is attached to; Claude treats them as ordinary user
input", with the runner advertising the pane's socket and target in a `tmux.json`. The
same docstring notes Claude's experimental Channels MCP capability was the *original*
input path. [D]

pty appears in only two places — `server/routes/terminal_attach.py` and
`terminals/ws_bridge.py` — i.e. for attaching a terminal, not for driving the
harness. [V]

**Child → parent runs over four channels:** [V]

1. **A status file** — see below.
2. **Hooks** (`claude_native_hook.py`), posting over HTTP.
3. **A transcript forwarder** (`claude_native_forwarder.py`), with a dead-letter queue.
4. **A statusLine wrapper** (`claude_native_status.py`) — `context_window` is only
   exposed to the `statusLine` command via stdin, so it captures that, writes
   `context.json`, then execs the user's original statusLine unchanged.

Plus an MCP stdio server that Claude Code launches as a child, advertising omnigent
tools. [D]

### The status file — this corrects our own research

`claude_native_status_file.py`: Claude Code writes a per-process JSON file at
`<config_dir>/sessions/<pid>.json` — its internal `concurrentSessions` registry,
present since **v2.1.139**, and **the same file that backs `claude agents`**. For an
interactive session it carries a `status` field that flips `idle` ⇄ `busy` ⇄
`waiting`. [D]

Omnigent resolves it primarily by pid, "which equals the tmux `#{pane_pid}` on the
omnigent launch path", with a `sessionId` cross-check and a bounded directory scan as
fallback. Its own docstring calls this "a cleaner running/idle signal than diffing the
tmux pane." [D]

**This contradicts what
[#5](https://github.com/sylmarien/harness-launcher/issues/5) concluded and
[#11](https://github.com/sylmarien/harness-launcher/issues/11) recorded.** Our research
found that `claude agents --json` does not report interactive sessions, and we
generalised that to "this mechanism cannot see our sessions." The *command* cannot; the
*file underneath it* can. [V]

Caveats, stated by omnigent itself and worth carrying over unsoftened: [D]

- The file is an **undocumented internal detail** of Claude Code.
- It is **version-gated** at v2.1.139+.
- Their watcher treats an unresolved or unreadable file as *"fall back to the PTY
  watcher"*, **never as an error** — 40 resolve attempts at a ~0.2 s cadence, then the
  PTY watcher stays authoritative for the session's lifetime.
- Status mapping is not one-to-one: `busy` and `waiting` both map to *running*, `idle`
  to *idle*, and a newer `shell` status (turn ended, background shell still alive, ≥
  v2.1.197) maps to *idle* — with a comment explaining that mapping it to *running*
  would strand the composer on a "(queued)" placeholder.
- The poller short-circuits on mtime and de-dupes to `running` ⇄ `idle` edges only.

## Patterns worth stealing

**Wrap, don't replace.** Omnigent's tmux integration discovers the user's existing
prefix-table `split-window` / `new-window` bindings and rewrites each with an
`if-shell -F` wrapper whose else-branch is *the user's exact original command* — so the
global mutation is "behaviorally invisible everywhere except inside an omnigent pane."
The statusLine wrapper chains to the user's pre-existing command the same way. [D] If
we ever touch a user's tmux configuration, this is the standard to meet.

**Layered signals with an explicit authority order**, rather than one mechanism that
must always work: status file first, PTY watcher as the standing fallback. [D]

**Verify the send.** Both firstmate and omnigent type into a tmux pane; only firstmate
verifies it landed. Given Claude Code's composer can swallow or queue an `Enter`,
verification is not optional. [I]

## The warning

Omnigent's tmux pane-split integration ships **disabled**:
`PANE_INTEGRATION_ENABLED = False`, with the comment that the feature "ships disabled
while the chooser UX is still being iterated on." [V] A funded team found tmux pane UX
hard enough to turn off by default. Our layout is simpler — one list pane, one spawn
pane, no chooser — but it belongs on the record for
[#13](https://github.com/sylmarien/harness-launcher/issues/13).

## What this does not tell us

- Nothing here bears on **Rust**. ccmanager is TypeScript because it wanted Ink and
  xterm; firstmate is shell; omnigent is Python. Since the only tmux crate is unstable,
  any implementation shells out to `tmux`, which is unremarkable in any language. [I]
- No project here implements simultaneity, so **none of them has ever paid the
  park/unpark redraw cost**. There is no prior art on whether it is tolerable. [I]
