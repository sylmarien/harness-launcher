# What Claude Code offers for programmatic control and supervision

> **Status: research notes.** Fact-finding for [issue #5](https://github.com/sylmarien/harness-launcher/issues/5).
> This document does **not** recommend an architecture — that decision is issue #9, worked with the
> human. Everything here is either (a) quoted or paraphrased from official Claude Code documentation
> with a URL, or (b) explicitly labelled as an inference or as **not documented**.
>
> Sources read on 2026-08-03, all from `code.claude.com/docs` (the `docs.claude.com/en/docs/claude-code/*`
> URLs 301-redirect there). Claude Code version references in the docs run to ~v2.1.219.

## The question, and the constraint that decides it

> What are the actual, documented ways to start, drive, observe and stop a Claude Code session from
> another program — and what does each one give up?

The constraint that ranks the answers comes from the product vision, not from Claude Code:
[reimplementing a harness's own interface is out of scope](../product-vision.md#where-the-app-stops),
and tranche 1 promises "[the full view of the agent, exactly as if I had started it by
hand](../tranches/01-the-core-loop.md#whats-in)". So for each option the decisive column is
**real interface or reconstruction**.

## Summary table

| # | Option | Supervisor can **see** | Supervisor can **send** | Real interface? |
|---|---|---|---|---|
| A | [Interactive TUI under a pty](#a-the-interactive-tui-under-a-pty) | Rendered terminal bytes only | Synthesised keystrokes | **Real** (it *is* the TUI) |
| B | [Headless `-p` one-shot](#b-headless-print-mode--p) | Final result (`text`/`json`) | One prompt per process | **Reconstruction** |
| C | [`stream-json` in + out](#c-bidirectional-stream-json) | Full structured event stream | User messages, interrupts | **Reconstruction** |
| D | [Agent SDK](#d-the-agent-sdk) | Same stream, as typed objects + control methods | Messages, interrupts, mode/model changes, permission answers | **Reconstruction** (and Python/TS only) |
| E | [Background sessions + supervisor](#e-background-sessions-the-supervisor-and-agent-view) | `claude agents --json` status; `claude logs` | `claude attach` / `stop` / `respawn`; replies | **Real**, via `claude attach` under a pty |
| F | [Hooks](#f-hooks) | 31 lifecycle events, incl. over HTTP | Allow/deny/block decisions, injected context | Side-channel — composes with any option |
| G | [Remote Control](#g-remote-control) | Everything | Everything | **Real**, but no third-party API |
| H | [Channels (MCP)](#h-channels-mcp-push-into-a-live-session) | Almost nothing | Events into a live session's context; permission verdicts | Side-channel into a **real** session |
| I | [Status line](#i-status-line) | Session metadata on a timer | Nothing | Side-channel |
| J | [Transcript files](#j-session-transcripts-on-disk) | Full history | Nothing | Side-channel — **explicitly unsupported for parsing** |

---

## A. The interactive TUI under a pty

**There is no documented API for driving the interactive TUI from another program.** The docs
describe the TUI exclusively as a human-facing terminal interface. Nothing documents keystroke
injection, a machine-readable event stream from an interactive process, or a supported "embed me"
mode. Treat any pty-driving design as unsupported-by-documentation, and see the note at the end of
this section for the one place Claude Code does it to itself.

What *is* documented, and bears directly on feasibility:

**Two renderers.** The classic renderer writes into the terminal's native scrollback. The
**fullscreen renderer** "draws the interface on the terminal's alternate screen buffer, like `vim`
or `htop`, and only renders messages that are currently visible."
([fullscreen](https://code.claude.com/docs/en/fullscreen)) It is the default for anyone who first
used Claude Code on or after 2026-05-06; `/tui default` switches back, `/tui` with no argument
prints which is active.

Selection is possible from outside the process:

| Lever | Effect |
|---|---|
| `tui` setting in `settings.json` | Persisted renderer choice |
| `CLAUDE_CODE_NO_FLICKER=1` | Equivalent to the `tui` setting; forces fullscreen |
| `CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1` | "force the classic renderer regardless of the saved `tui` setting" |
| `CLAUDE_CODE_DISABLE_MOUSE=1` | Opt out of mouse capture, keep flicker-free rendering |
| `CLAUDE_CODE_DISABLE_MOUSE_CLICKS=1` | Keep wheel scrolling, drop click/drag/hover (v2.1.195+) |
| `CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT=1` | Repaint every cell every frame instead of sending incremental updates |
| `CLAUDE_CODE_SCROLL_SPEED` | Wheel multiplier |

All from [fullscreen](https://code.claude.com/docs/en/fullscreen) and
[env-vars](https://code.claude.com/docs/en/env-vars).

**`CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT` exists because embedding hosts get this wrong.** The docs
say: "Fullscreen rendering sends only the cells that changed between frames. Some terminals, most
commonly Windows Terminal and other ConPTY-backed hosts, coalesce these positioned writes
incorrectly and leave fragments of earlier output on screen until you resize the window."
(*Inference:* a hand-rolled pty + terminal-grid emulator in the host app is exactly the class of
consumer that hits this, and the escape hatch exists.)

**It expects a capable terminal.** Documented dependencies include mouse reporting forwarded to the
application, synchronized output (probed at startup), OSC 52 for clipboard over SSH, the kitty
keyboard protocol for some chords, and `COLUMNS`/`LINES` being set correctly. Fullscreen rendering
is "incompatible with iTerm2's tmux integration mode" (`tmux -CC`). Under tmux ≤3.6 there is extra
flicker because those versions lack synchronized output.

**Input is keystrokes, not commands.** Everything a supervisor would want to do — send a prompt,
interrupt, answer a permission dialog, detach — is a key sequence documented for humans in
[interactive-mode](https://code.claude.com/docs/en/interactive-mode) (`Enter`, `Esc`, `Ctrl+C`,
`Ctrl+O`, `←` on an empty prompt, …). There is no documented stable contract for these as an API.

**Observation is rendered bytes.** A pty host sees a terminal grid. It does not see tool calls,
turn boundaries, or status as data. Structured observation has to come from a side-channel —
[hooks](#f-hooks), the [status line](#i-status-line), or `claude agents --json` — not from the pty.

| | |
|---|---|
| **See** | The rendered interface, exactly as a human would. Nothing structured. |
| **Send** | Keystrokes. |
| **Real interface?** | **Yes — it is the real interface, byte for byte.** Zero reconstruction. |
| **Gives up** | Any structured knowledge of what the agent is doing; documentation support; terminal-emulation correctness is the host's problem. |

> **The one documented precedent.** Claude Code does exactly this to itself. `claude attach <id>`
> opens a background session in your terminal, and "The attaching terminal enters the alternate
> screen buffer to show the session"; attached sessions "always render in fullscreen mode,
> regardless of your `tui` setting, because a background session has no terminal scrollback to
> append to." ([fullscreen](https://code.claude.com/docs/en/fullscreen),
> [agent-view](https://code.claude.com/docs/en/agent-view)) See [option E](#e-background-sessions-the-supervisor-and-agent-view).

---

## B. Headless print mode (`-p`)

`-p` / `--print`: "Print response without interactive mode".
([cli-reference](https://code.claude.com/docs/en/cli-reference))

Output formats, via `--output-format`
([headless](https://code.claude.com/docs/en/headless)):

- `text` (default) — plain text
- `json` — "structured JSON with result, session ID, and metadata"; the payload "includes
  `total_cost_usd` and a per-model cost breakdown". With `--json-schema <schema>` the constrained
  value lands in `structured_output`.
- `stream-json` — "newline-delimited JSON for real-time streaming"

`--bare` "reduce[s] startup time by skipping auto-discovery of hooks, skills, plugins, MCP servers,
auto memory, and CLAUDE.md" and "skips OAuth and the system keychain". It "is the recommended mode
for scripted and SDK calls, and will become the default for `-p` in a future release." Note the
trade: `--bare` also disables the hook discovery that [option F](#f-hooks) depends on, unless hooks
are passed explicitly via `--settings`.

Other documented behaviour:

- stdin is read, so data can be piped in; "piped stdin is capped at 10MB" as of v2.1.128.
- Background Bash tasks started during a `-p` run "[are] terminated about five seconds after Claude
  has returned its final result and stdin has closed." Background subagents/workflows are waited
  for, capped at ten minutes by default (`CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS`).
- User-invoked skills and custom commands work in `-p` ("include `/skill-name` in the prompt
  string"). Terminal-only commands such as `/login` do not. `/model`, `/effort`, `/fast`, `/color`,
  `/rename` accept the value as an argument (v2.1.205+); `/config key=value` works (v2.1.181+).
- `--no-session-persistence` suppresses transcript writes for one run
  ([sessions](https://code.claude.com/docs/en/sessions)).

| | |
|---|---|
| **See** | For `text`/`json`: the final answer plus metadata. Nothing during the run. |
| **Send** | One prompt, at process start. |
| **Real interface?** | **No.** There is no interface — the app would render everything itself. |
| **Gives up** | Everything the vision protects: the user gets a reconstruction, not Claude Code. |

---

## C. Bidirectional `stream-json`

This is print mode used as a long-lived, two-way process. The flags
([cli-reference](https://code.claude.com/docs/en/cli-reference)):

| Flag | Meaning |
|---|---|
| `--output-format stream-json` | Newline-delimited JSON events out |
| `--input-format stream-json` | Newline-delimited JSON messages in ("options: `text`, `stream-json`") |
| `--verbose` | Required alongside `stream-json` in the documented examples |
| `--include-partial-messages` | "Include partial streaming events in output. Requires `--print` and `--output-format stream-json`" |
| `--replay-user-messages` | "Re-emit user messages from stdin back on stdout for acknowledgment. Requires `--input-format stream-json` and `--output-format stream-json`" |
| `--include-hook-events` | "Include hook lifecycle events from every hook event in the output stream" |
| `--forward-subagent-text` | "Emit subagent text and thinking blocks in the output stream as `assistant` and `user` messages with `parent_tool_use_id` set" (v2.1.211+) |
| `--prompt-suggestions` | Emit a `prompt_suggestion` message with a predicted next user prompt |

### What comes out

- **`system` / `init`** — first event unless startup events precede it. "reports session metadata
  including the model, tools, MCP servers, and loaded plugins." Also `plugins` / `plugin_errors`,
  `mcp_servers` / `mcp_server_errors` (v2.1.219+), and an optional **`capabilities` array** of
  protocol behaviour names such as `interrupt_receipt_v1` or `interrupt_cancel_queued_v1` —
  "Check it to feature-detect instead of comparing version strings" (v2.1.205+).
- **`assistant` / `user` messages** — including subagent messages, distinguished by
  `parent_tool_use_id` (`null` for the main conversation). Nesting is reconstructible by following
  those IDs (v2.1.219+ for nested subagents).
- **`stream_event`** — token-level deltas when `--include-partial-messages` is on. The documented
  `jq` filter is `select(.type == "stream_event" and .event.delta.type? == "text_delta")`.
- **`system` / `api_retry`** — `attempt`, `max_retries`, `retry_delay_ms`, `error_status`, `error`
  (one of `authentication_failed`, `oauth_org_not_allowed`, `billing_error`, `rate_limit`,
  `overloaded`, `invalid_request`, `model_not_found`, `server_error`, `max_output_tokens`,
  `unknown`), `uuid`, `session_id`.
- **`system` / `plugin_install`**, and `hook_started` / `hook_progress` / `hook_response` events.
- **`result`** — "The last line of the stream is a `result` message with the final response text,
  cost, and session metadata."

Backpressure is handled: "If your consumer reads the stream slowly, Claude Code waits for the queued
output to drain before exiting, scaling the wait with how much is still queued, capped at 30
seconds." (v2.1.214+)

### What goes in

The CLI reference documents the `--input-format stream-json` flag but **not the input message
schema**. The schema is documented on the SDK side as `SDKUserMessage`
([streaming-vs-single-mode](https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode)):

```jsonc
{
  "type": "user",
  "message": { "role": "user", "content": "Analyze this codebase for security issues" },
  "parent_tool_use_id": null
}
```

Content may be a block array including `{"type": "image", "source": {"type": "base64", ...}}`.
*Inference:* the SDKs drive the CLI as a subprocess over exactly this channel, so the CLI accepts
this shape — but the CLI reference does not state it, so a Rust implementation would be relying on
an interface documented only for the SDK.

Streaming input mode is described as letting "the agent operate as a long lived process that takes
in user input, handles interruptions, surfaces permission requests, and handles session
management", with queued messages, image uploads, real-time feedback and context persistence.
Single-message mode explicitly "does **not** support: Direct image attachments in messages, Dynamic
message queueing, Real-time interruption, Natural multi-turn conversations."

| | |
|---|---|
| **See** | Everything, as structured JSON: turns, tool calls, tool results, thinking, subagents, retries, cost, hook lifecycle. Richest observation of any option. |
| **Send** | User messages at any time; interrupts (via the SDK control protocol — see D). |
| **Real interface?** | **No.** The stream is data; the app draws the UI. |
| **Gives up** | The real interface. This is the maximal-observability, maximal-reconstruction end of the spectrum — the direct opposite trade from [option A](#a-the-interactive-tui-under-a-pty). |

---

## D. The Agent SDK

**Language support is the first fact.** "The SDK is available as a library for Python and
TypeScript only. To drive the same agent loop from another language, [run the CLI as a
subprocess](https://code.claude.com/docs/en/headless) with the `-p` flag and `--output-format
json`." ([agent-sdk/overview](https://code.claude.com/docs/en/agent-sdk/overview)) For a Rust app
this means: no SDK, or a Node/Python sidecar.

The SDK is positioned as "Claude Code as a library" — "the same tools, agent loop, and context
management that power Claude Code". Its comparison table says the CLI is for "Doing interactive
development or running one-off tasks from a terminal", and the SDK for "Building an agent without
implementing the tool loop yourself". Neither row is "surfacing Claude Code's own UI".

### Control surface

The TypeScript `Query` object extends `AsyncGenerator<SDKMessage, void>`
([agent-sdk/typescript](https://code.claude.com/docs/en/agent-sdk/typescript)):

```typescript
interface Query extends AsyncGenerator<SDKMessage, void> {
  interrupt(): Promise<SDKControlInterruptResponse | undefined>;
  rewindFiles(userMessageId: string, options?: { dryRun?: boolean }): Promise<RewindFilesResult>;
  setPermissionMode(mode: PermissionMode): Promise<void>;
  setModel(model?: string): Promise<void>;
  setMaxThinkingTokens(maxThinkingTokens: number | null): Promise<void>;
  applyFlagSettings(settings: { [K in keyof Settings]?: Settings[K] | null }): Promise<void>;
  initializationResult(): Promise<SDKControlInitializeResponse>;
  reinitialize(): Promise<SDKControlInitializeResponse>;
  supportedCommands(): Promise<SlashCommand[]>;
  supportedModels(): Promise<ModelInfo[]>;
  supportedAgents(): Promise<AgentInfo[]>;
  mcpServerStatus(): Promise<McpServerStatus[]>;
  getContextUsage(): Promise<SDKControlGetContextUsageResponse>;
  accountInfo(): Promise<AccountInfo>;
  reconnectMcpServer(serverName: string): Promise<void>;
  toggleMcpServer(serverName: string, enabled: boolean): Promise<void>;
  setMcpServers(servers: Record<string, McpServerConfig>): Promise<McpSetServersResult>;
  streamInput(stream: AsyncIterable<SDKUserMessage>): Promise<void>;
  stopTask(taskId: string): Promise<void>;
  close(): void;
}
```

`interrupt()` is "only in streaming input mode" and returns a response "listing queued messages that
survive, or `undefined` on CLIs before v2.1.205". `setPermissionMode` is likewise streaming-only.

### Approvals and questions

`canUseTool` is the host's hook for both permission prompts and Claude's clarifying questions
([agent-sdk/user-input](https://code.claude.com/docs/en/agent-sdk/user-input)):

```typescript
canUseTool: (request: ElicitationRequest, options: { signal: AbortSignal }) => Promise<ElicitationResult>
```

Return `{ behavior: "allow", updatedInput }` or `{ behavior: "deny", message }`. It "can stay pending
indefinitely. Execution remains paused until your callback returns". Critically: "**The callback
never fires for auto-approved tools.**" Anything resolved by an allow rule or a permission mode
never reaches it — "For logic that must apply to every tool call, use a `PreToolUse` hook".

`AskUserQuestion` arrives through the same callback with `toolName === "AskUserQuestion"`; the input
carries a `questions` array (1–4 questions, 2–4 options each, `multiSelect`), and the host returns an
`answers` map. **`AskUserQuestion` is not available in subagents.**

### Sessions

| Option | Meaning |
|---|---|
| `resume` | Session ID to resume |
| `resumeSessionAt` | Resume at a specific message UUID |
| `forkSession` | Fork to a new session ID instead of continuing the original |
| `continue` | Continue the most recent conversation |
| `sessionId` | Use a specific UUID instead of auto-generating |
| `persistSession` | `false` disables disk persistence (TypeScript only; "Python always persists to disk") |

Session ID is read from `session_id` on the result message, "present on every result regardless of
success or error"; in TypeScript it is also on the init `SystemMessage`.
`listSessions()` / `getSessionMessages()` / `getSessionInfo()` / `renameSession()` / `tagSession()`
(and Python equivalents) exist for "custom session pickers, cleanup logic, or transcript viewers"
([agent-sdk/sessions](https://code.claude.com/docs/en/agent-sdk/sessions)).

| | |
|---|---|
| **See** | The same stream as C, as typed objects, plus on-demand queries (context usage, MCP status, supported commands/models/agents). |
| **Send** | Messages, interrupts, permission verdicts, permission-mode and model changes, MCP reconfiguration, file rewinds, task stops. **The richest send surface of any option.** |
| **Real interface?** | **No.** The SDK is explicitly a library for building *your own* agent; branding guidance even forbids calling the result "Claude Code" or mimicking its visual elements. |
| **Gives up** | The real interface; and for a Rust host, the SDK itself — leaving the CLI-as-subprocess path, i.e. option C. |

---

## E. Background sessions, the supervisor, and agent view

This is the option that most closely matches what harness-launcher is trying to build, and it is
already shipped in Claude Code (as a research preview).
Source: [agent-view](https://code.claude.com/docs/en/agent-view).

### Starting

```bash
claude --bg "investigate the flaky SettingsChangeDetector test"
claude --name "flaky-test-fix" --bg "..."
claude --agent code-reviewer --bg "address review comments on PR 1234"
claude --bg --exec 'pytest -x'
```

From inside a session: `/background` (`/bg`) moves the conversation to the background; `/fork` copies
it into a new background session while the original keeps running.

On success Claude prints:

```text
backgrounded · 7c5dcf5d · flaky-test-fix
  claude agents             list sessions
  claude attach 7c5dcf5d    open in this terminal
  claude logs 7c5dcf5d      show recent output
  claude stop 7c5dcf5d      stop this session
```

### Observing — `claude agents --json`

> `claude agents --json` prints active sessions as a JSON array and exits: every live session, plus
> background sessions that are still working or blocked even when their process has exited. Add
> `--all` to also include completed background sessions, and `--cwd <path>` to limit the list to
> sessions started under that directory.

| Field | Present | Description |
|---|---|---|
| `cwd`, `kind`, `startedAt` | Always | Working directory, `interactive` or `background`, start time in Unix ms |
| `id` | Background sessions | Short ID, usable with `claude attach`/`logs`/`stop` |
| `state` | Background sessions | `working`, `blocked`, `done`, `failed`, or `stopped` |
| `pid`, `status` | While the process is alive | Process ID and current status |
| `waitingFor` | When `status` is `waiting` | `permission prompt`, `input needed`, `sandbox request`, `worker request`, or `dialog open` |
| `sessionId`, `name` | When set | Full session UUID (for `claude --resume`) and display name |

Session states, as documented: **Working** ("Claude is actively running tools or generating a
response"), **Blocked** ("waiting on something only you can provide"), **Idle** ("nothing to do and
ready for your next prompt"), **Done**, **Failed**, **Stopped**.

> **Caveat that matters.** "Interactive sessions you have open in other terminals don't appear in
> agent view until you background them." So `claude agents --json` is not a global registry of every
> running Claude Code — it covers the supervisor's own sessions plus live sessions it knows about.
> Note also that the docs enumerate the values of `state` and `waitingFor` but **not** the values of
> `status`; only `waiting` is implied.

### Driving and stopping

| Command | Purpose |
|---|---|
| `claude agents` | Open agent view |
| `claude agents --cwd <path>` | Scope to sessions under `<path>` |
| `claude attach <id>` | "Attach to a session in this terminal" |
| `claude logs <id>` | "Print the session's recent output" (in memory, not written to disk) |
| `claude stop <id>` / `claude kill <id>` | Stop a session |
| `claude respawn <id>` / `--all` | Restart with conversation intact, e.g. onto an updated binary |
| `claude rm <id>` | Remove from the list, plus a Claude-created worktree "when that's safe to delete" |
| `claude daemon status` | Supervisor state, version, socket directory, worker count |
| `claude daemon stop --any` | Stop the supervisor (`--keep-workers` leaves sessions running) |

Detaching never stops a session: `←`, `Ctrl+Z`, `/exit`, double `Ctrl+C`/`Ctrl+D` all leave it
running. `/stop` from inside ends it.

### The supervisor

- Per-user process, "separate from your terminal and from agent view", auto-started on first
  background/agent-view use.
- Keeps one pre-warmed worker so dispatch avoids a cold launch.
- Authenticates with the same stored credentials; the dispatching shell's `PATH` and provider
  variables are applied to the worker. `ANTHROPIC_BASE_URL` is **not** inherited from the shell that
  started the supervisor (gateway users must put it in the project's `.claude/settings.json` `env`).
- Lifecycle: a finished, unattached session's process is stopped after ~an hour; state stays on disk
  and restarts on next attach/peek/reply. When every session is finished and no terminal is
  connected, the supervisor exits.
- Restarts a session whose process exits unexpectedly, with three documented safeguards (don't
  restart something already done/failed/stopped; a user-backgrounded session killed externally is
  marked stopped, not restarted; a restarted session is told it was restarted).
- Watches the installed binary and restarts into new versions.

State paths (`~/.claude`, or `$CLAUDE_CONFIG_DIR`):

| Path | Contents |
|---|---|
| `~/.claude/daemon.log` | Supervisor log |
| `~/.claude/daemon/roster.json` | Running background sessions, used to reconnect after restart |
| `~/.claude/jobs/<id>/state.json` | Per-session state shown in agent view |
| `~/.claude/jobs/<id>/tmp/` | Per-session scratch; `$CLAUDE_JOB_DIR` points at `~/.claude/jobs/<id>` |

*Inference (not documented as a supported interface):* these files are implementation detail of the
supervisor. Nothing says they are stable, and by analogy with the explicit transcript warning
([option J](#j-session-transcripts-on-disk)) it would be unwise to parse them.

Limitations, verbatim in spirit: research preview; background sessions consume subscription usage
the same as interactive ones; sessions are local and "stop if the machine shuts down"; Claude-created
worktrees are deleted with the session in agent view (though "A worktree with commits that aren't
pushed anywhere is kept along with the session", and `claude rm` "keeps a worktree that has
uncommitted changes together with its session, and a worktree you created yourself is left in
place").

| | |
|---|---|
| **See** | Structured per-session status (`working`/`blocked`/`done`/`failed`/`stopped` + `waitingFor`) via a documented CLI-to-JSON command, plus recent output via `claude logs`. |
| **Send** | Start (`--bg`), stop, respawn, remove — and full interaction by attaching. |
| **Real interface?** | **Yes**, via `claude attach <id>` — which is itself a pty-hosted real TUI, exactly [option A](#a-the-interactive-tui-under-a-pty), but with the session lifecycle owned by Claude Code's own supervisor rather than by the host app. |
| **Gives up** | Polling (`agents --json` prints and exits; no documented watch/subscribe). No documented status for interactive sessions in other terminals. Two supervisors in the system (Claude Code's daemon and harness-launcher's own) with overlapping responsibility for worktrees and lifecycle. Research preview. |

---

## F. Hooks

Hooks are the richest *structured* observation channel that composes with the *real* interface.
Source: [hooks](https://code.claude.com/docs/en/hooks).

**31 documented events**, in lifecycle order: `SessionStart`, `Setup`, `UserPromptSubmit`,
`UserPromptExpansion`, `PreToolUse`, `PermissionRequest`, `PermissionDenied`, `PostToolUse`,
`PostToolUseFailure`, `PostToolBatch`, `Notification`, `MessageDisplay`, `SubagentStart`,
`SubagentStop`, `TaskCreated`, `TaskCompleted`, `Stop`, `StopFailure`, `TeammateIdle`,
`InstructionsLoaded`, `ConfigChange`, `CwdChanged`, `DirectoryAdded`, `FileChanged`,
`WorktreeCreate`, `WorktreeRemove`, `PreCompact`, `PostCompact`, `Elicitation`, `ElicitationResult`,
`SessionEnd`.

**Cadence** — the docs group them explicitly:

- Once per session: `SessionStart`, `SessionEnd`
- Once per turn: `UserPromptSubmit`, `Stop`, `StopFailure`
- Every tool call in the agentic loop: `PreToolUse`, `PostToolUse`

**Common input fields**, delivered as JSON on stdin (command hooks) or as a POST body (HTTP hooks):
`session_id`, `prompt_id` (v2.1.196+), `transcript_path`, `cwd`, `permission_mode`, `effort`,
`hook_event_name`, and inside a subagent `agent_id` / `agent_type`.

**Five handler types**: `command` (shell), **`http` (POST JSON to a URL)**, `mcp_tool`, `prompt`
(a model yes/no decision), `agent` (experimental subagent verification). The HTTP handler is the one
that matters for an external supervisor:

```json
{
  "type": "http",
  "url": "http://localhost:8080/hooks/pre-tool-use",
  "headers": { "Authorization": "Bearer $MY_TOKEN" },
  "allowedEnvVars": ["MY_TOKEN"]
}
```

**Configuration locations**: `~/.claude/settings.json` (all projects), `.claude/settings.json`
(project, shareable), `.claude/settings.local.json` (gitignored), managed policy settings
(organization), plugin `hooks/hooks.json`, and skill/agent frontmatter. From the CLI they can also
be injected per-invocation with `--settings <file-or-json>`
([headless](https://code.claude.com/docs/en/headless)). Managed settings can restrict this:
`allowManagedHooksOnly` blocks user/project/plugin hooks, `allowedHttpHookUrls` and
`httpHookAllowedEnvVars` apply globally.

**Exit codes** (command hooks):

| Exit code | Meaning |
|---|---|
| `0` | Success. stdout is parsed as JSON output. For most events stdout goes to the debug log; `UserPromptSubmit`, `UserPromptExpansion`, and `SessionStart` show stdout to Claude. |
| `2` | Blocking error — stdout ignored, stderr fed back to Claude. Blocks for `PreToolUse`, `PermissionRequest`, `UserPromptSubmit`, `UserPromptExpansion`, `Stop`, `SubagentStop`, `TeammateIdle`, `TaskCreated`, `TaskCompleted`, `ConfigChange`, `PreCompact`, `Elicitation`, `ElicitationResult`, `PostToolBatch`, `WorktreeCreate`. Non-blocking for the rest. |
| Any other | Non-blocking error; execution continues, first line of stderr shown in the transcript. |

**JSON output** — universal fields `continue` (`false` stops Claude entirely), `stopReason`,
`suppressOutput`, `systemMessage`, `terminalSequence` (allowlisted OSC 0/1/2/9/99/777 or BEL, for
desktop notifications; v2.1.141+). Event-specific: `PreToolUse` returns
`hookSpecificOutput.permissionDecision` of `allow`/`deny`/`ask`/**`defer`**;
`PermissionRequest` returns `decision.behavior` plus optional `updatedInput`; `SessionStart` can
return `additionalContext`, `initialUserMessage`, `watchPaths`, `sessionTitle`, `reloadSkills`.

`additionalContext` strings are capped at 10,000 characters per output. Handlers with identical
command/args (or URL) are deduplicated and run in parallel. `disableAllHooks: true` turns them all
off, except managed hooks.

| | |
|---|---|
| **See** | Turn start (`UserPromptSubmit`), turn end (`Stop` / `StopFailure`), every tool call, permission requests, notifications, session start/end, compaction, subagent lifecycle, worktree lifecycle — pushed, not polled, and deliverable straight to an HTTP endpoint the app owns. |
| **Send** | Permission verdicts, blocking decisions, injected context, `continue: false` to stop the agent. Not free-form prompts. |
| **Real interface?** | **Untouched.** Hooks are additive; the user still gets whatever interface the session is running. |
| **Gives up** | Requires configuration injection per spawn (`--settings`, or writing project settings). Incompatible with `--bare` unless hooks are passed explicitly. Managed policy can forbid non-managed and HTTP hooks. |

---

## G. Remote Control

Source: [remote-control](https://code.claude.com/docs/en/remote-control).

Remote Control "connects claude.ai/code or the Claude app for iOS/Android to a Claude Code session
running on your machine". Started with `claude remote-control` (server mode),
`claude --remote-control` / `--rc` (interactive session with it enabled), `/remote-control` from
inside a session, or `/remote-control` in VS Code.

It is unambiguously a **real interface + external steering** model — the remote device sends
messages, approves tool calls, sees subagent and workflow progress, and "the conversation and the
progress of subagents and dynamic workflows stay in sync across all connected devices, so you can
send messages from your terminal, browser, and phone interchangeably."

Server mode even has options that rhyme with harness-launcher's design: `--spawn worktree` ("each
on-demand session gets its own git worktree"), `--spawn session` (single-session mode),
`--capacity <N>` (default 32 concurrent sessions), `--sandbox` / `--no-sandbox`.

**But it is not a mechanism a third-party app can use.** Documented constraints:

- Subscription only (Pro/Max/Team/Enterprise); "API keys are not supported"; long-lived
  `setup-token` / `CLAUDE_CODE_OAUTH_TOKEN` tokens are rejected too.
- Only against `api.anthropic.com`; disabled on Bedrock, Google Cloud's Agent Platform, Foundry, or
  any custom `ANTHROPIC_BASE_URL`.
- All routing goes through Anthropic servers, and "While Remote Control is connected, the session
  transcript … is stored on Anthropic servers."
- The client is claude.ai/code or the Claude mobile app. **No public client API or protocol is
  documented.** Research preview; org-gated; ZDR orgs cannot enable it.
- "Local process must keep running"; a network outage over ~10 minutes times the session out.

| | |
|---|---|
| **See** / **Send** | Everything a human can, from the first-party clients. |
| **Real interface?** | **Yes** — and it is the strongest documented evidence that "real UI locally, steered from elsewhere" is a shape Anthropic supports. |
| **Gives up** | Usability by a third-party program: there is no documented API. Also cloud dependency, subscription-only auth, and transcripts on Anthropic servers. |

---

## H. Channels (MCP push into a live session)

Source: [channels-reference](https://code.claude.com/docs/en/channels-reference).

"A channel is an MCP server that pushes events into a Claude Code session so Claude can react to
things happening outside the terminal." Claude Code spawns it as a stdio subprocess. It declares
`capabilities.experimental['claude/channel'] = {}` and emits `notifications/claude/channel` with
`content` and optional `meta`. The event reaches Claude's context as:

```text
<channel source="webhook" severity="high" run_id="1234">
build failed on main: https://ci.example.com/run/1234
</channel>
```

Two-way channels expose an ordinary MCP tool that Claude calls to reply. A channel that
authenticates senders can also declare `capabilities.experimental['claude/channel/permission']` and
receive `notifications/claude/channel/permission_request` (`request_id` — five lowercase letters
from `a`–`z` excluding `l`; `tool_name`; `description`; `input_preview`), answering with
`notifications/claude/channel/permission` carrying `request_id` and `behavior: 'allow' | 'deny'`.
"The local terminal dialog stays open through all of this" and whichever answer arrives first wins.

Important limits:

- "Claude Code doesn't acknowledge notifications." If the session didn't load the channel or policy
  blocks it, "Claude Code drops the events silently and returns no error to your server."
- Events queue and are delivered as a group on the next turn if Claude is busy.
- Research preview with an Anthropic-curated allowlist; anything else needs
  `--dangerously-load-development-channels`, which shows a full-screen warning dialog.
- "An ungated channel is a prompt injection vector."

| | |
|---|---|
| **See** | Effectively nothing about the agent's state — only permission requests, if opted in. |
| **Send** | Text into a live session's context (as a `<channel>` tag, not as a user prompt), and permission verdicts. |
| **Real interface?** | **Untouched** — this works against a normal interactive session. |
| **Gives up** | Observation; delivery guarantees; and it is allowlist-gated. Useful as an *input* side-channel to a real session, not as a supervision mechanism. |

---

## I. Status line

Source: [statusline](https://code.claude.com/docs/en/statusline).

Configured as `statusLine: { type: "command", command: "...", refreshInterval: N }`. Claude Code
"runs your script and pipes JSON session data to it via stdin".

Runs "once when a session starts, including when you resume one", then again when: a new assistant
message arrives; `/compact` finishes; the permission mode changes; vim mode toggles; or a
`refreshInterval` timer elapses (minimum 1 second). Updates are debounced at 300ms and an in-flight
script is cancelled if a new update triggers.

Fields include `session_id`, `session_name`, `prompt_id`, `transcript_path`, `version`, `cwd`,
`workspace.{current_dir,project_dir,added_dirs,git_worktree,repo.*}`,
`cost.{total_cost_usd,total_duration_ms,total_api_duration_ms,total_lines_added,total_lines_removed}`,
`context_window.*`, `model.{id,display_name}`, `effort.level`, `thinking.enabled`, `fast_mode`,
`rate_limits.{five_hour,seven_day}.*`, `output_style.name`, `vim.mode`, `agent.name`,
`pr.{number,url,review_state}`, and `worktree.{name,path,branch,original_cwd,original_branch}`
("Present only during `--worktree` sessions").

**There is no busy/idle field.** *Inference:* "a new assistant message arrives" is a turn-activity
signal and `refreshInterval` gives a heartbeat, so a status-line script could report liveness — but
it cannot distinguish "working" from "waiting for you" from the documented fields.

| | |
|---|---|
| **See** | Rich session metadata (cost, context, model, worktree, PR) on assistant messages and on a timer. |
| **Send** | Nothing. |
| **Real interface?** | Untouched; the status line renders inside the real TUI. |
| **Gives up** | No state field, no tool-level events, event-driven triggers "can go quiet when the main session is idle". |

---

## J. Session transcripts on disk

"By default, transcripts are stored as JSONL at `~/.claude/projects/<project>/<session-id>.jsonl`,
where `<project>` is your working directory path with non-alphanumeric characters replaced by `-`."
([sessions](https://code.claude.com/docs/en/sessions))

The docs then close the door on parsing them:

> Each line is a JSON object for a message, tool use, or metadata entry. **The entry format is
> internal to Claude Code and changes between versions, so scripts that parse these files directly
> can break on any release.** To build on session data, use `/export` or the script interfaces
> instead.

The blessed "script interfaces" are: `claude -p` with `--output-format json`/`stream-json`;
`claude -p --resume <session-id>` to ask an existing session a question; the `transcript_path`
that hooks and status-line commands receive; and the Agent SDK.

Storage is configurable: `CLAUDE_CONFIG_DIR` moves it, `cleanupPeriodDays` changes the 30-day
retention, `CLAUDE_CODE_SKIP_PROMPT_HISTORY` suppresses writes, `--no-session-persistence`
suppresses writes for one `-p` run.

---

## Cross-cutting: session resumption

From [cli-reference](https://code.claude.com/docs/en/cli-reference) and
[sessions](https://code.claude.com/docs/en/sessions):

| Flag / command | Meaning |
|---|---|
| `--continue` / `-c` | "Load the most recent conversation in the current directory" |
| `--resume` / `-r` | "Resume a specific session by ID or name, or show an interactive picker" |
| `--session-id <uuid>` | "Use a specific session ID for the conversation (must be a valid UUID)" |
| `--fork-session` | "When resuming, create a new session ID instead of reusing the original" |
| `--from-pr <number>` | Session picker filtered to sessions linked to that PR |
| `-n <name>` / `/rename` | Name a session; `--resume <name>` then works |
| `/branch [name]` | Copy the conversation and switch into it, in-process |

Key facts for a supervisor:

- **Scope.** "session ID lookup is scoped to the current project directory and its git worktrees",
  so a resume must run from the directory the session was started in. This interacts directly with
  harness-launcher's worktree-per-spawn model.
- **`-p` sessions are resumable but invisible to the picker.** "Sessions created with `claude -p` or
  the Agent SDK do not appear in the session picker, but you can still resume one by passing its
  session ID to `claude --resume <session-id>`."
- **What a resume restores**: conversation history including tool calls and results, model, agent,
  permission mode (never `plan` or `bypassPermissions`; `auto` only if still eligible), active goal,
  unexpired scheduled tasks.
- **What it does not restore**: `--mcp-config`, `--settings`, `--plugin-dir`, `--fallback-model`,
  `--add-dir` directories. Standard settings files are re-read.
- **Concurrent resume is unsafe.** "If you resume the same session in two terminals without forking,
  messages from both interleave into one transcript."
- Interactive sessions get a **default display name** from v2.1.196 (`my-app-3f` style) that
  "identifies the session in listings of running sessions, such as agent view and
  `claude agents --json` output" — but "The default isn't a resume handle."

---

## Cross-cutting: exit codes and signals

**There is no documented exit-code table.** The
[errors reference](https://code.claude.com/docs/en/errors) enumerates error messages, not exit
codes; the only exit code it names is `137` for a killed installation. What is documented elsewhere:

| Situation | Code | Source |
|---|---|---|
| `claude -p` success | `0` | [headless](https://code.claude.com/docs/en/headless) |
| `claude -p` failure | non-zero | headless |
| `claude -p` receiving SIGTERM | `143` | headless |
| `claude auth status` | `0` logged in, `1` not | [cli-reference](https://code.claude.com/docs/en/cli-reference) |
| `claude ultrareview` | `0` success, `1` failure | cli-reference |
| `Failed to resume the conversation` from the picker | `1` | [sessions](https://code.claude.com/docs/en/sessions) |
| Piped stdin over 10MB | non-zero, "with a clear error" | headless |

**Error surfaces differ by failure class**, which matters for a supervisor deciding whether a spawn
died or merely failed: "If you pass an invalid flag, Claude Code reports the error to stderr before
the run starts. When a failure happens inside the run, such as missing authentication, Claude Code
prints the failure as the result on stdout."

**SIGTERM is the only documented signal contract**, and only for `-p`:

> If you stop a `claude -p` run with SIGTERM, for example from `kill`, a process supervisor, or an
> SDK host closing the session, Claude Code aborts the in-progress turn, terminates the process tree
> of any running Bash command, runs `SessionEnd` hooks, and exits with code 143.

Nothing is documented about SIGINT, SIGHUP, or signal behaviour in interactive mode. In the TUI,
`Ctrl+C` is documented as a *keystroke* ("cancels a running response or `!` shell command"), and
double `Ctrl+C` on an empty prompt detaches — not as signal semantics.

---

## Cross-cutting: what reports busy vs idle

This is the tranche-1 promise ("two statuses, and only two: the agent is working, or the agent has
stopped"), so it is worth isolating.

| Mechanism | Signal | Granularity | Notes |
|---|---|---|---|
| `claude agents --json` | `state`: `working` / `blocked` / `done` / `failed` / `stopped`; plus `status` and `waitingFor` | Per session, on demand | The only documented **first-class status field**. Poll-only. Does not cover interactive sessions in other terminals until they're backgrounded. `status` values are not enumerated in the docs. |
| Hooks | `UserPromptSubmit` (turn starts) → `Stop` / `StopFailure` (turn ends); `Notification`; `PermissionRequest`; `SessionStart` / `SessionEnd` | Per turn / per event, **pushed** | Can be delivered to an HTTP endpoint. Composes with any interface. Needs config injection. |
| `stream-json` | Turn boundaries and the terminal `result` message are explicit in the stream | Per event, pushed | Only in `-p` mode — costs the real interface. |
| Agent SDK | Same stream as typed messages; `canUseTool` pending = blocked on a human | Per event, pushed | Python/TS only. |
| Status line | Runs on "a new assistant message arrives" and on `refreshInterval` | Coarse | **No busy/idle field.** *Inference:* usable as a liveness heartbeat only. |
| `claude logs <id>` | Recent output text | Coarse | Background sessions only; in-memory, not on disk. |
| Transcript JSONL | Everything | Fine | Format explicitly unstable — do not parse. |
| pty output | Whatever the TUI draws | Visual | Would require screen-scraping the spinner/footer. Not documented, and the footer layout is not a contract. |

**The two mechanisms that give a documented, structured busy/idle signal without giving up the real
interface are `claude agents --json` (poll) and hooks (push).** Neither of those requires headless
mode. That is the load-bearing observation for issue #9.

---

## What the documentation does not say

Stated as "not documented" rather than guessed:

1. **Any supported way to run the interactive TUI under a pty and drive it from another program.**
   No keystroke API, no documented machine-readable event stream from an interactive process, no
   stability guarantee on the rendered layout. Claude Code does this internally for
   `claude attach`, but does not expose it as an integration point.
2. **A comprehensive exit-code table.** Only the fragments listed above.
3. **Signal behaviour beyond SIGTERM in `-p`.** SIGINT, SIGHUP and interactive-mode signal handling
   are unspecified.
4. **The wire schema of `--input-format stream-json` on the CLI.** The flag is documented; the
   message shape is documented only on the SDK side.
5. **The values of the `status` field in `claude agents --json`.** Only `state` and `waitingFor`
   values are enumerated; `waiting` is implied for `status`.
6. **A way to enumerate or attach to interactive sessions running in other terminals.** They are
   explicitly absent from agent view until backgrounded.
7. **Any programmatic client protocol for Remote Control.** The clients are first-party only.
8. **Stability of `~/.claude/jobs/*`, `~/.claude/daemon/roster.json`, or the transcript JSONL.** The
   transcript is explicitly called out as unstable; the daemon files carry no statement either way
   (*inference:* treat them as equally unstable).
9. **Whether the supervisor/agent-view surface is intended for third-party orchestration.** It is
   documented as a Claude Code feature in research preview, with no stated integration contract.

## Adjacent finding, flagged not chased

Claude Code has its own worktree support: a `--worktree` mode (the status line documents
`worktree.name` as "Present only during `--worktree` sessions"), a `--spawn worktree` mode in Remote
Control server mode ("each on-demand session gets its own git worktree"), `WorktreeCreate` and
`WorktreeRemove` hooks, and worktree-aware deletion in `claude rm`. This overlaps with what tranche
1 has the app doing itself. It is out of scope for issue #5 and deserves its own ticket; the
[worktrees page](https://code.claude.com/docs/en/worktrees) was not read for this document.

## Sources

All read 2026-08-03.

- [CLI reference](https://code.claude.com/docs/en/cli-reference)
- [Run Claude Code programmatically (headless)](https://code.claude.com/docs/en/headless)
- [Agent SDK overview](https://code.claude.com/docs/en/agent-sdk/overview)
- [Agent SDK — TypeScript reference](https://code.claude.com/docs/en/agent-sdk/typescript)
- [Agent SDK — Streaming input](https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode)
- [Agent SDK — Work with sessions](https://code.claude.com/docs/en/agent-sdk/sessions)
- [Agent SDK — Handle approvals and user input](https://code.claude.com/docs/en/agent-sdk/user-input)
- [Hooks reference](https://code.claude.com/docs/en/hooks)
- [Manage sessions](https://code.claude.com/docs/en/sessions)
- [Agent view](https://code.claude.com/docs/en/agent-view)
- [Remote Control](https://code.claude.com/docs/en/remote-control)
- [Channels reference](https://code.claude.com/docs/en/channels-reference)
- [Customize your status line](https://code.claude.com/docs/en/statusline)
- [Fullscreen rendering](https://code.claude.com/docs/en/fullscreen)
- [Interactive mode](https://code.claude.com/docs/en/interactive-mode)
- [Errors](https://code.claude.com/docs/en/errors)
- [Documentation index](https://code.claude.com/docs/llms.txt)
