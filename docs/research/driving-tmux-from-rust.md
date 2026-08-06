# Driving tmux from Rust

> **Fact-finding for [issue #19](https://github.com/sylmarien/harness-launcher/issues/19).** This
> document does **not** pick a control surface, a parking mechanism or a minimum tmux version —
> those are decisions worked later with the human. Everything here is input to them.
>
> Premise: the [resolution on #18](https://github.com/sylmarien/harness-launcher/issues/18#issuecomment-5198632660)
> — the app drives tmux rather than embedding a terminal emulator; one visible window holds the
> app's list pane beside one spawn pane; inactive spawns park in a dedicated **detached holding
> session**, moved in and out with `join-pane` / `break-pane`. tmux only; zellij is out of scope.

## How to read this

Claims are tagged:

- **[V]** — **verified by experiment** on this machine, **`tmux 3.4`**, Linux 6.18, on 2026-08-05.
  Each experiment ran against a throwaway server on its own socket (`tmux -L <name>`), so nothing
  touched a real session.
- **[S]** — **read from primary source**: the tmux manual page source (`tmux.1`) at a pinned tag, or
  the tmux C source at a pinned tag. Not executed.
- **[I]** — **inferred**. Reasoning from **[V]** or **[S]**, not directly established.

Sources fetched 2026-08-05. `repology.org` and `packages.debian.org` refused automated fetch (HTTP
403); distro versions in §9 come from Launchpad and from a domain-restricted search of
`packages.debian.org`, and are flagged accordingly. GitHub's REST API is not reachable from this
environment, so `raw.githubusercontent.com` was used to read tmux's source and manual directly at
pinned tags.

**One bias runs through this document.** Parking is load-bearing: every spawn crosses it twice per
visit, twenty times over. Where a mechanism is *silently* lossy — where it works but degrades
something the user would notice — this document says so loudly, because that is the failure class
that will not show up in a smoke test.

---

## 0. The short version

Six things that will shape the design:

1. **Parking works, and works with no client attached anywhere on the server.** `break-pane` into a
   detached session succeeds, the pane keeps its id and its process keeps its pid. **[V]** (§3.1)
2. **Every park and every unpark resizes the pane and delivers a SIGWINCH to the child.** This is
   structural, not a setting: a pane alone in a window is the window's full height, and a pane
   sharing the app window is not. Neither `window-size manual` nor a same-sized holding session
   avoids it. Twenty parks is twenty full redraws of a Claude Code TUI. **[V]** (§3.3)
3. **Control mode never tells you a process died.** `pane-died` / `pane-exited` are hooks, not
   control-mode notifications. Everything the control stream gives you for a dying spawn is
   second-order — a `%layout-change`, or a `%unlinked-window-close`. Death detection has to be built
   out of hooks. **[V]** [S] (§5, §6)
4. **A control client attached to the app session receives no `%output` from parked panes** — output
   is filtered to windows linked into that client's session. Watching parked spawns live needs a
   *second* control client attached to the holding session. **[V]** [S] (§2.2)
5. **`refresh-client -A '%id:off'` stalls the child.** If every control client turns a pane off,
   tmux stops reading its pty and the process blocks on write once the buffer fills. Verified: a
   writer never finished and `history_size` stayed at 0. The `no-output` *client* flag does not have
   this problem. **[V]** (§2.4)
6. **20 panes with a full default scrollback cost ~216 MB of tmux server RSS.** ~10.5 MB per pane at
   200 columns × 2000 lines. The #18 resolution says the scrollback arithmetic "disappears" — it
   does not disappear, it moves into the tmux server, and it scales with the user's own
   `history-limit`. **[V]** (§10)

---

## 1. The three control surfaces

### 1.1 Shelling out to the `tmux` client, one process per command

`std::process::Command::new("tmux")` with arguments. The `tmux` binary connects to the server over
its unix socket, runs the command, prints, exits.

- **Sending**: complete. Every command in the manual is reachable. **[V]**
- **Receiving**: nothing asynchronous. You get the command's own stdout and exit status. Any event
  awareness has to be polled, or pushed to you by a hook (§6.3).
- **Cost**: one `fork`/`exec` and one socket round-trip per command. Measured: 20
  unpark/repark round trips (40 `tmux` invocations) in **0.19 s**, ≈5 ms each. **[V]** 40
  spawn-plus-exit-plus-hook cycles took **2.4 s** — ≈60 ms each, dominated by process spawn, since
  each cycle is three processes. **[V]**
- **Batching**: multiple commands can be sent in one invocation separated by `;` — but `;` is a
  command separator in tmux's own parser, so a `;` inside a shell command argument must be escaped
  or the tail becomes a second tmux command. **[V]**
- **Structured output**: `-F '<format>'` on the listing commands gives exactly the fields you ask
  for, so no scraping of human-readable output is required. **[V]**

### 1.2 Control mode (`tmux -C` / `tmux -CC`)

One long-lived `tmux -C attach -t <session>` child process. Commands go in on stdin, one per line;
output comes back on stdout as blocks, plus asynchronous notifications.

Framing, from the manual **[S]** and confirmed **[V]**:

```
%begin <unix-time> <command-number> <flags>
…command output…
%end <unix-time> <command-number> <flags>          (or %error … on failure)
```

Notifications start with `%` and "will never occur inside an output block". **[S]** Command numbers
correlate request to response.

`-C` versus `-CC`: `-C` keeps the terminal in canonical mode and is what you want when driving from
a program with pipes; `-CC` additionally disables echo and emits terminal-detection sequences, and
exists for terminal emulators taking over the user's tty (iTerm2's original use case). **[S] — tmux
wiki, [Control Mode](https://github.com/tmux/tmux/wiki/Control-Mode).**

**What it gives that shelling out does not**: asynchronous notification of state changes, and
`%output` — a live copy of every byte a pane produces, with non-printables and backslash escaped as
octal `\xxx`. Verified: `plain line\015\012\033[31mred\033[0m\011tab\134backslash\015\012`. **[V]**

**The complete notification list in 3.4** **[S]** (`tmux.1`, CONTROL MODE):
`%client-detached`, `%client-session-changed`, `%config-error`, `%continue`, `%exit`,
`%extended-output`, `%layout-change`, `%message`, `%output`, `%pane-mode-changed`,
`%paste-buffer-changed`, `%paste-buffer-deleted`, `%pause`, `%session-changed`, `%session-renamed`,
`%session-window-changed`, `%sessions-changed`, `%subscription-changed`, `%unlinked-window-add`,
`%unlinked-window-close`, `%unlinked-window-renamed`, `%window-add`, `%window-close`,
`%window-pane-changed`, `%window-renamed`.

**Note what is not on that list: anything about a process exiting.** See §5.

**Parsing hazard.** A hook that runs `display-message -p` emits its text into the control stream as
a **bare line with no `%` prefix and outside any `%begin`/`%end` block**. Observed: `EVT pane-died
%2 3` on its own line. The same hook using `display-message` *without* `-p` produces a well-formed
`%message EVT pane-died %2 3`. **[V]** A control-mode parser must tolerate unrecognised bare lines
regardless, since anything on the server can call `display-message -p`.

**A control client is a client.** It attaches to a session and it has a size, and by default that
size participates in window sizing (§3.4). `-f ignore-size` opts out. **[V]**

### 1.3 The server socket directly

The `tmux` client and server speak a private protocol over a unix domain socket (default
`/tmp/tmux-<uid>/default`; `#{socket_path}` reports it **[V]**). It is OpenBSD `imsg` framing with a
message-type enum and an explicit version:

```c
/* Protocol version. */
#define PROTOCOL_VERSION 8

enum msgtype {
	MSG_VERSION = 12,
	MSG_IDENTIFY_FLAGS = 100, MSG_IDENTIFY_TERM, MSG_IDENTIFY_TTYNAME, …
	MSG_COMMAND = 200, MSG_DETACH, MSG_DETACHKILL, MSG_EXIT, …
```

— [`tmux-protocol.h`](https://raw.githubusercontent.com/tmux/tmux/master/tmux-protocol.h) **[S]**.
`PROTOCOL_VERSION` is **8** in both 3.4 and master, so it has not moved recently **[S]**.

It is not documented in the manual page, has no compatibility promise, and carries file descriptors
(the client's tty) as ancillary data. Speaking it from Rust means reimplementing `imsg` framing and
the identify handshake against a moving private target. No Rust crate does this (§11). **[S] / [I]**

### 1.4 They compose

These are not exclusive. A long-lived control client for events plus ordinary `tmux` invocations for
commands is a legitimate shape — verified working side by side throughout the experiments here
**[V]**. Equally, a control client can issue every command itself on its stdin. The decision is out
of scope for this document.

---

## 2. What a control client can and cannot see

This section matters because the design puts most spawns in a session the app is not attached to.

### 2.1 Notifications are scoped by the client's attached session

From [`control-notify.c`](https://raw.githubusercontent.com/tmux/tmux/master/control-notify.c)
**[S]**: `%layout-change` is sent only to control clients whose session contains the window; window
link/unlink notifications are split by the same test —

```c
if (winlink_find_by_window_id(&cs->windows, w->id) != NULL)
        control_notify_write(c, "%%window-close @%u", w->id);
else
        control_notify_write(c, "%%unlinked-window-close @%u", w->id);
```

So a control client attached to the **app** session *does* learn about the holding session's
windows — as `%unlinked-window-add` / `%unlinked-window-close` / `%unlinked-window-renamed`.
Verified: parking a pane produced `%unlinked-window-renamed @2 sleep` then `%unlinked-window-add
@2`; the parked pane dying (with `remain-on-exit` off) produced `%unlinked-window-close @2`. **[V]**

Two caveats. The rename notification arrives **before** the add **[V]**. And these carry a *window*
id, not a pane id — the app must keep its own pane→window map, and the window vanishes at the same
moment the notification arrives, so the mapping cannot be recovered afterwards. **[V] / [I]**

### 2.2 `%output` is hard-filtered to the attached session

[`control.c`](https://raw.githubusercontent.com/tmux/tmux/3.4/control.c), `control_write_output`:

```c
if (winlink_find_by_window(&c->session->windows, wp->window) == NULL)
        return;
```

**[S]** Verified: a control client on the app session received nothing when a parked pane produced
output; a second control client attached to the holding session received
`%output %2 plain line\015\012…` for the same pane. **[V]**

So live output from parked spawns requires a **second control client attached to the holding
session**. Attaching it with `-f ignore-size` did not resize any parked pane. **[V]**

### 2.3 Format subscriptions are scoped the same way

`refresh-client -B <name>:<what>:<format>` asks tmux to push `%subscription-changed` when a format's
value changes, "at most once a second". `what` may be `%*` (all panes in the attached session), `@*`
(all windows in the attached session), a specific id, or empty. **[S]**

Verified: `refresh-client -B 'deadness:%*:#{pane_dead}'` reported
`%subscription-changed deadness $0 @0 0 %2 : 1` when a pane in the attached session died, and
reported **nothing** for a pane that had been parked in the holding session. **[V]**

Note the once-a-second ceiling: this is push-shaped polling, not an event. **[S]**

### 2.4 Per-pane output suppression stalls the child — do not use it

The manual: "If `off`, tmux will not send output from the pane to the client and **if all clients
have turned the pane off, will stop reading from the pane**." **[S]**

That is not a throttle, it is a stall. Verified with one control client, pane turned off via
`refresh-client -A '%0:off'`, then a process writing 5000 lines: the process **never finished**, and
`history_size` stayed at **0**. **[V]** With no `:off`, the same process finished and filled 1951
lines of history. **[V]**

Two things follow.

- The `no-output` **client** flag is a different mechanism and is safe. In `control_pane_offset`,
  `CLIENT_CONTROL_NOOUTPUT` returns with `*off = 0` ("this client is not asking the pane to stop"),
  whereas `CONTROL_PANE_OFF` returns `*off = 1`. **[S]** Verified: with
  `-f ignore-size,no-output` the writer finished, history filled to 1951 lines, and zero `%output`
  lines were delivered. **[V]**
- Turning a pane back **on** does not give a clean cut. `control_set_pane_on` resets the client's
  offset to `wp->offset` **[S]**, and `wp->offset` does not advance while nobody is reading, so
  resuming replays what accumulated. Verified: output written while off appeared in the stream
  immediately after `:on`. **[V]**

### 2.5 With no clients at all, tmux still drains every pane

A fully detached server keeps reading pane ptys into its own grid. Verified: a process writing 5000
lines in a session with zero clients ran to completion, and 20 parked panes were each filled to the
2000-line history limit with no client anywhere. **[V]** Parked spawns therefore keep running
normally — the risk in §2.4 comes only from explicitly turning a pane off for a control client.

### 2.6 Flow control exists for the firehose

Twenty Claude Code panes streaming into one control client is a lot of `%output`. tmux 3.2 added a
per-client `pause-after=<seconds>` flag: when a pane's buffered output falls that far behind, tmux
sends `%pause %<id>` and stops; the client resumes it with `refresh-client -A '%<id>:continue'`, and
in the meantime output arrives as `%extended-output %<id> <age-ms> … : <value>`. **[S]**
Not exercised here — flagged as the mechanism to reach for, not as verified behaviour.

---

## 3. Pane movement: the parking mechanism

### 3.1 It works across sessions, with no client attached anywhere

Verified end to end on a server with **zero attached clients**: `break-pane -d -s %2 -t hold:`
returned 0, and afterwards `%2` reported `session=hold window=@2:1 dead=0`. `join-pane -d -s %2 -t
app:main` brought it back. **[V]**

The manual's sentence "This command works only if at least one client is attached" sits in the
`find-window` entry, **not** `break-pane` — it is adjacent in `tmux.1` and easy to misattribute.
**[S]** (`tmux.1` at 3.4, lines 2545–2549.)

### 3.2 The process, the pane id and the scrollback all survive

- **Pane id**: unchanged across break and join. **[V]** Pane ids are documented as "unique and …
  unchanged for the life of the … pane in the server" **[S]**, and are **not reused** — after
  killing `%1` the next new pane was `%2`. **[V]**
- **Process**: same pid before, during and after the round trip (`pid=9228` throughout). **[V]** The
  pty is owned by the pane object, which is moved between windows by pointer, not recreated —
  [`cmd-join-pane.c`](https://raw.githubusercontent.com/tmux/tmux/3.4/cmd-join-pane.c) does
  `src_wp->window = dst_w;` and re-parents its options. **[S]**
- **Scrollback**: fully preserved. 300 marker lines written before parking were all still capturable
  with `capture-pane -p -S -` after parking, and again after unparking. **[V]** `capture-pane`
  works on a parked pane. **[V]**
- **`history_size` changes** — 277 before, 251 after — because the pane got taller and 26 lines
  moved out of history into the visible screen. Content is conserved; the split between screen and
  history is not. **[V]**

### 3.3 …but the pane is resized on every move, and that is structural

| Step | pane size | child sees |
| --- | --- | --- |
| created in app window (200×50, split) | 200×24 | — |
| `break-pane` into holding session | **200×50** | `SIGWINCH`, `50 200` |
| `join-pane` back into app window | **200×24** | `SIGWINCH`, `24 200` |

**[V]** — measured with a child that logs `stty size` on `SIGWINCH`.

The cause is in
[`cmd-break-pane.c`](https://raw.githubusercontent.com/tmux/tmux/3.4/cmd-break-pane.c):

```c
w = wp->window = window_create(w->sx, w->sy, w->xpixel, w->ypixel);
```

The new window is created at the **source window's** dimensions, and the pane is then the only pane
in it, so it grows to fill. **[S]** The holding session's own size is irrelevant — verified by
parking into an 80×24 holding session and watching the parked window come out at 200×50. **[V]**

Things that do **not** fix it:

- `window-size manual` + `default-size 200x50` — still 24 → 50 → 24. **[V]**
- Making the holding session the same size as the app session — the parked window still takes the
  source *window* size, and the pane still goes from "half a window" to "a whole window". **[V]**
- Parking with `join-pane` into a pre-existing holding window instead of `break-pane` — the pane
  then takes the *split* size of that holding window. Verified against a 100×30 holding window:
  200×12 → **100×14** → 200×24. Worse, not better, and the return trip did not restore the original
  height either. **[V]**

**Consequence [I]:** a park/unpark round trip means at least two `SIGWINCH`es and two full redraws of
whatever TUI the spawn is running, plus tmux reflowing the grid. Any design that parks on every
selection change pays this on every keystroke of list navigation. If the size must be restored
exactly, the app has to re-assert it (`resize-pane` / `select-layout`) after unparking — join-pane's
default split does not restore it.

### 3.4 A detached holding session is *safer* than an attached one

With the holding session detached, parked panes keep whatever size they were given at break time.
**[V]**

Attach a sized client to it and **every parked pane resizes at once**: verified, a parked pane at
200×50 became 100×30 the moment a control client attached to `hold` was given
`refresh-client -C 100x30`. **[V]** With `-f ignore-size` on that client, nothing resized. **[V]**

**This is a user-facing hazard [I].** The holding session is a normal session on a normal server. It
shows up in `tmux ls`, and `tmux attach` with no arguments can land on it. A user who does that
resizes twenty live Claude Code sessions in one go. Nothing in tmux hides a session from
`list-sessions`.

The relevant option semantics: `window-size` may be `largest` / `smallest` / `manual` / `latest`
(default `latest` in 3.4 **[V]**); `default-size` (default `80x24` **[V]**) applies "when the
`window-size` option is set to manual or when a session is created with `new-session -d`". **[S]**
`new-session -x/-y` sets `default-size` for that session. **[S]**

### 3.5 The holding session dies when its last window leaves

`cmd-join-pane.c`: `if (window_count_panes(src_w) == 0) server_kill_window(src_w, 1);` **[S]** — and
killing a session's last window destroys the session.

Verified: with the keepalive window removed and one parked pane remaining, joining that pane back to
the app window destroyed the `hold` session entirely; `list-sessions` afterwards showed only `app`.
**[V]**

So the holding session needs either a permanent keepalive window or logic to recreate it. Note the
keepalive window costs a real process. **[I]**

### 3.6 `break-pane` has a second code path when the source window has one pane

`cmd-break-pane.c` lines 76–89: if `window_count_panes(w) == 1`, break-pane does **not** create a new
window — it `server_link_window`s the existing window into the destination session and unlinks it
from the source. **[S]** Different identity semantics (the window id is preserved), different
sizing. The app's window is meant to hold list + spawn = 2 panes, so the ordinary path applies — but
this branch is a trap for any edge case where the spawn pane is briefly alone in a window. **[I]**

### 3.7 `join-pane` fails when the destination is too small

`layout_split_pane` returning NULL yields `create pane failed: pane too small`. **[S]** Verified: in
a 200×50 window, only 6 panes could be joined; the other 15 attempts failed with exactly that
message. **[V]** Not a concern for a one-spawn-pane layout, but it *is* the failure mode, and it is
a plain non-zero exit with a message on stderr, so it is detectable. **[V]**

### 3.8 Moving a pane re-parents its options

`options_set_parent(src_wp->options, dst_w->options)` in both commands. **[S]** Options set on the
*pane* (`set-option -p -t %id`) travel with it; anything relying on the source *window's* options
changes meaning on arrival. **[I]**

---

## 4. Creating a pane running a specific command, in a specific directory

### 4.1 The command

```
tmux split-window -d -P -F '#{pane_id}' -t <target> -c <dir> [-e K=V]… <command…>
```

- `-P -F '#{pane_id}'` prints the new pane's id on stdout. This is the reliable way to learn which
  pane you got — verified across every experiment here; it printed e.g. `%2`. **[V]** `new-window`
  takes the same flags. **[V]**
- `-c <dir>` sets the working directory; verified with a path containing spaces, which arrived
  intact as `/tmp/dir with spaces`. **[V]**
- `-d` leaves the new pane inactive.
- **Three argument forms all work** in 3.4 **[V]**: one shell-command string
  (`"/path/prog arg"`), argv style (`"/path/prog" "arg"`), and with a `--` separator. Quoting is
  tmux's own, then the shell's: a command whose argument contained `'`, `"` and `;` round-tripped
  correctly once escaped for both layers, producing `it's a "test"; ok`. **[V]** A bare `;` in a
  command line is a **tmux command separator**, not shell syntax. **[V]**
- `-e K=V` sets environment variables in the new pane's process; verified `FOO=bar BAZ=qux` reached
  the child. **[V]** Added in 3.0 — present in the 3.0a manual, absent in 2.9's. **[S]**
- Failure to exec is *not* a command failure. `split-window` returned a pane id happily for a
  non-executable script; the pane then died with `pane_dead_status=126`. **[V]** So "the command
  started" and "the command ran" are different questions.

### 4.2 What the child is told

- `TMUX_PANE` is set to the pane id — `environ_set(child, "TMUX_PANE", 0, "%%%u", new_wp->id);` in
  [`spawn.c`](https://raw.githubusercontent.com/tmux/tmux/3.4/spawn.c) **[S]**, verified as
  `TMUX_PANE=%1` **[V]**.
- `TMUX` is `<socket_path>,<server_pid>,<session_id_number>` —
  `environ_set(env, "TMUX", 0, "%s,%ld,%d", socket_path, (long)getpid(), idx);` in
  [`environ.c`](https://raw.githubusercontent.com/tmux/tmux/3.4/environ.c) **[S]**, verified as
  `TMUX=/tmp/tmux-0/e15,25714,0` **[V]**. Note the third field is the numeric session id (`0`), not
  `$0`.
- `TERM` is set to `screen`/`tmux` per the manual. **[S]**

`$TMUX` is the documented way for the app to know it is already inside tmux — this is the mode
detection #18 assumes, and it comes from the app's own process environment, not from tmux state
(`show-environment -g TMUX` reports `unknown variable`). **[V]**

### 4.3 Bootstrapping when the app is *not* in tmux

- A tmux **client needs a tty**: `tmux attach` with stdin redirected fails with
  `open terminal failed: not a terminal`. **[V]** So the app's own TUI can only be a tmux pane if
  something re-execs it under tmux; a headless process cannot "become" a client. **[I]**
- **`new-session -A` is not a headless idempotent create.** With `-A` and an existing session,
  `new-session` "behave[s] like `attach-session`" and `-D` maps to attach's `-d` (detach *others*),
  not to "do not attach" **[S]**; both `new-session -A -d` and `new-session -A -D` failed with
  `open terminal failed: not a terminal` from a non-tty process. **[V]** The reliable pattern is
  `has-session -t X` (exit 0/1) followed by `new-session -d -s X`. **[V]**

### 4.4 Server isolation

`tmux -L <name>` / `tmux -S <path>` selects a different server entirely. Verified: a second server on
another socket was completely invisible to the first. **[V]** This is the lever for "never touch the
user's tmux" — at the cost of giving up taking over the user's current window, which lives on the
user's server. **[I]**

---

## 5. Noticing that a pane's process has exited or died

### 5.1 What tmux offers

| Mechanism | Fires when | Available where |
| --- | --- | --- |
| `pane-exited` hook | the program in a pane exits (pane goes away) | hooks only |
| `pane-died` hook | the program exits **but `remain-on-exit` is on**, so the pane stays | hooks only |
| `#{pane_dead}` | 1 for a dead-but-retained pane | formats |
| `#{pane_dead_status}` | exit status | formats |
| `#{pane_dead_signal}` | terminating signal | formats (**3.3+**, §9) |
| `#{pane_dead_time}` | exit time | formats |

**[S]** (`tmux.1`, HOOKS and FORMATS.) `remain-on-exit` may be `on`, `off`, or `failed` — the last
retaining the pane only when the program's exit status is non-zero. The `failed` value arrived in
3.2: it is in the 3.2a manual and not in 3.1c's. **[S]**

Verified in 3.4 **[V]**:

```
remain-on-exit off, exit 0        -> pane vanishes; hook: pane-exited hook_pane=%2
remain-on-exit on,  exit 7        -> dead=1 status=7            hook: pane-died hook_pane=%3
remain-on-exit on,  SIGKILL       -> dead=1 signal=9 (status empty)
```

Note **status and signal are mutually exclusive** — a signalled process reports `pane_dead_signal=9`
and an *empty* `pane_dead_status`. **[V]** Code that reads only `pane_dead_status` will read empty
string and must not treat it as 0.

### 5.2 Hooks fire for parked panes

The important positive result: a pane parked in the **detached holding session** and then killed
fired `pane-died hook_pane=%5 sess=hold status= sig=9`. **[V]** Death detection therefore does not
depend on the spawn being visible.

### 5.3 `#{hook_pane}` identifies the pane

Inside a hook, `#{hook_pane}` expands to the pane the hook is about, and the other pane formats
resolve against it. **[V]** This is what makes a global hook usable — one `set-hook -g` covers every
spawn without per-pane registration.

### 5.4 `respawn-pane` revives a dead pane, keeping its id

`respawn-pane -t %3 'sleep 100'` on a dead pane returned 0 and produced `dead=0` with a **new pid**
and the **same pane id**. **[V]** `respawn-pane -k` kills a live process first. **[V]**

### 5.5 The gap: control mode says nothing about death

`pane-died` and `pane-exited` are in the manual's list of hooks that are *not* control-mode
notifications; the manual is explicit that "all the notifications listed in the CONTROL MODE section
are hooks … except `%exit`", and the additional hook list — which includes `pane-died`,
`pane-exited`, `client-attached`, `session-created`, … — goes the other way. **[S]**

Verified: a pane in the app window exiting produced only `%layout-change` on the control client, with
no pane id in it that identifies what left. **[V]** A parked pane vanishing produced
`%unlinked-window-close @3`, which identifies a window, not a pane. **[V]**

So: **a control-mode-only design still needs hooks** to learn that a spawn died with any precision.

---

## 6. Getting an event out of tmux and into Rust

Four working paths, all verified on 3.4. This is not an exhaustive list.

### 6.1 Hook → `display-message` → `%message` on a control client

```
set-hook -g pane-died 'display-message "EVT pane-died #{hook_pane} #{pane_dead_status}"'
```

Produces `%message EVT pane-died %3 7` on every attached control client. **[V]** No process spawned.
Use `display-message` **without** `-p`; with `-p` you get the bare-line hazard from §1.2. **[V]**
`display-message -c <client>` targets a specific client. **[V]**

### 6.2 `refresh-client -B` subscriptions

`refresh-client -B 'deadness:%*:#{pane_dead}'` → `%subscription-changed` on change, throttled to
once a second, scoped to the attached session (§2.3). **[V]** Good for state, not for edges.

### 6.3 Hook → `run-shell -b` → a FIFO or socket the app reads

```
set-hook -g pane-exited 'run-shell -b "echo pane-exited #{hook_pane} > /path/to.fifo"'
```

Verified delivering `pane-exited %1` into a FIFO the app was reading. **[V]** Works with no control
client at all — this is the event path for a pure shell-out design. Costs one process per event
(§1.1: ≈60 ms per spawn+exit+hook cycle, process-spawn dominated). **[V]**

### 6.4 `tmux wait-for` as a blocking channel

A Rust thread blocks on `tmux wait-for <channel>`; a hook runs `tmux wait-for -S <channel>` and the
waiter returns 0. Verified. **[V]** Poll-free and needs no control client, but the channel carries
**no payload** — the app wakes and must re-query state. And it costs a blocked process plus a spawned
one per event. **[I]**

### 6.5 The hook surface itself

Hooks are array options, settable globally (`-g`) or per session / window / pane, with the same
scoping rules as options. **[S]** Every control-mode notification is also a hook (without arguments),
plus: `alert-activity`, `alert-bell`, `alert-silence`, `client-active`, `client-attached`,
`client-detached`, `client-focus-in`, `client-focus-out`, `client-resized`,
`client-session-changed`, `pane-died`, `pane-exited`, `pane-focus-in`, `pane-focus-out`,
`pane-set-clipboard`, `session-created`, `session-closed`, `session-renamed`, `window-linked`,
`window-renamed`, `window-resized`, `window-unlinked`. **[S]** (`tmux.1` at 3.4, HOOKS.) Every
command additionally has an `after-<command>` hook. **[S]**

`window-unlinked` fired when a parked pane's window went away. **[V]**

---

## 7. Taking over a window the user is attached to

Verified against a simulated user server with two windows, three panes, and non-default global
options.

### 7.1 Non-destructive and recoverable

- **Adding the app's list pane by splitting** an existing window is non-destructive: the user's panes
  and processes are untouched, only their sizes change. **[V]**
- **`#{window_layout}` is a saveable, restorable string.** Captured before, restored afterwards with
  `select-layout -t <win> "<string>"` — byte-identical result. **[V]** The layout string encodes pane
  ids in its trailing numbers, so restoring requires the same panes still to exist. **[I]**
- **When the app's pane exits, the layout collapses back on its own** — after a temporary pane
  exited, the window's layout string was identical to the saved one without any restore call. **[V]**
- **Parking the user's own panes** to a holding window and giving them back works exactly as it does
  for spawns, layout restored. **[V]** Their window count changes while parked (a new window appears
  in whichever session receives them) — visible in the user's status bar. **[V]**
- **`tmux -L <socket>` isolation** sidesteps the whole question, at the cost of not being in the
  user's window at all. **[V]** (§4.4)

### 7.2 Destructive

- **`kill-pane`** on a user pane destroys the process and its scrollback. Irrecoverable.
- **`kill-server`** ends everything on that server, including sessions the app never created.
- **`kill-session`** on the wrong target, likewise.
- **Global option writes leak into every session on the server.** `set-option -g remain-on-exit on`
  changes the behaviour of *all* the user's panes; the app's own bookkeeping option becomes the
  user's surprise. **[V]** The scoped forms — `set-option -p -t %id` for a pane, `-w` for a window,
  `-t` for a session — do not leak, and `pane-died` / `pane-exited` / `remain-on-exit` are all
  settable per pane since 3.0. **[V] [S]** Restoring a global option means remembering its previous
  value and whether it was set at all (`set-option -gu` unsets). **[V]**
- **`detach-client`, `switch-client`, `select-window`** move the user somewhere they did not ask to
  go. Recoverable, but they are the actions a user would describe as rude. **[I]**

### 7.3 Things a user would notice even when nothing is destroyed

**[I]**, from the mechanics above:

- Every pane in the taken-over window is resized when the app's pane appears.
- The status line is the user's, showing the user's window list; new windows the app creates in the
  user's session appear in it.
- Window *indexes* shift when windows are added or removed; `break-pane -a/-b` explicitly
  renumbers ("existing windows are moved if necessary" **[S]**).
- The prefix key and every user binding remain live in the app's pane, so the user can `C-b`
  themselves out of the app's layout at any time. The app cannot assume its layout is stable.
- The holding session appears in the user's `tmux ls` (§3.4).

### 7.4 One thing that is free

The tmux server outlives the app. Verified: killing the app's control client left every spawn's
process running and every session intact. **[V]** The tranche-1 scope says "surviving the app
closing" is out — with tmux, a crude version of it arrives whether or not it is asked for. Note the
inverse: the app's spawns do **not** outlive the tmux *server*, so `kill-server` and a machine
reboot are still total losses. **[I]**

---

## 8. Ordering and identification notes

- **Use ids, not names.** The tmux wiki is explicit: using session, window and pane ids "is strongly
  recommended because they are unambiguous". **[S]** Window *indexes* shift (§7.3); window and pane
  *ids* do not, and pane ids are never reused. **[V]**
- **Window names are set automatically** from the running command unless `automatic-rename` is off —
  parked windows were auto-named `sleep`, which is how the `%unlinked-window-renamed` notification
  arrived. **[V]** `break-pane -n <name>` sets a name and turns `automatic-rename` off. **[S]**
- **`#{socket_path}`, `#{pid}` (server pid) and `#{version}` are all available as formats.** **[V]**
  `#{version}` exists since 2.4. **[S]**
- **`session_width` / `session_height` do not exist** in 3.4's format list — only `client_width`,
  `window_width` and so on. A format string naming them silently expands to empty. **[V] [S]**

---

## 9. Version sensitivity

Established by diffing `tmux.1` at pinned tags. **[S]**

| Capability | Earliest checked version that has it |
| --- | --- |
| `break-pane` / `join-pane` across sessions | ≤ 2.6 (join-pane 1.2, break-pane 0.9 per `CHANGES`) |
| `display-message -p` | 1.2 (`CHANGES`) |
| `pane-died` / `pane-exited` hooks | 2.2 (`CHANGES`, "internal for now"); in the manual by 2.6 |
| `set-hook` / `show-hooks` | ≤ 2.6 (manual) |
| `#{hook_pane}`, `#{pane_dead_status}`, `#{pane_dead_time}` | ≤ 2.6 (manual) |
| `%unlinked-window-add` / `-close` | ≤ 2.6 (manual) |
| `window-size` option | 2.9 |
| `default-size` option | 2.9 (`CHANGES`) |
| pane-level options (`set -p`), incl. per-pane `pane-died` | 3.0 (`CHANGES`) |
| `-e` env flag on `new-window` / `split-window` | 3.0 |
| hooks as array options, settable via `set-option` | 3.0 (`CHANGES`) |
| `refresh-client -B` format subscriptions | **3.2** |
| `ignore-size` / `no-output` client flags, `pause-after` | **3.2** |
| `remain-on-exit failed` | **3.2** |
| `#{pane_dead_signal}` | **3.3** |
| `refresh-client -C '@win:WxH'` (per-window size) | **3.3** |
| `set-hook -B` monitor hooks, `wait-for -E`, event payloads | **3.8** (`CHANGES`, current master) |

Versions given as "≤ 2.6" mean the feature is already in the 2.6 manual; earlier tags were not
checked, because 2.6 is well below any realistic floor.

The binding constraints for this design are **3.0** (`-e`, and per-pane options so the app can set
`remain-on-exit` without touching the user's globals — §7.2), **3.2** (`ignore-size`, which §3.4
shows is what keeps a control client from resizing parked panes; plus format subscriptions and
`remain-on-exit failed`), and **3.3** if `pane_dead_signal` is wanted to tell "killed" from "exited
non-zero".

What is actually installed **[S, from Launchpad and a `packages.debian.org` search; repology
refused automated fetch]**:

| Platform | tmux |
| --- | --- |
| Ubuntu 22.04 LTS | 3.2a |
| Ubuntu 24.04 LTS | 3.4 |
| Ubuntu 26.04 (resolute) | 3.6a |
| Debian 12 bookworm | 3.3a |
| Debian 13 trixie | 3.5a |
| Homebrew / current upstream | 3.7b–3.8 |

Current upstream head is **3.8** (from `CHANGES` on master). **[S]**

**3.8 is worth watching**: it reworks hooks and control-mode notifications onto an internal event
system with key/value payloads, adds `set-hook -B` "monitor" hooks that check a format every second
and fire when it changes, `wait-for -E` for waiting on hooks and events, and new hooks explicitly
covering "pane movement and resizing" and OSC 133 command start/finish. **[S]** (`CHANGES FROM 3.7b
TO 3.8`.) Several of the gaps this document records — no death notification in control mode, no pane
id on move events — look like they are being closed there, but on a version nobody has installed.
**[I]**

---

## 10. Behaviour at ~20 panes

Nothing in the manual or `CHANGES` documents a pane-count limit. **[S]** Measured instead:

| Measurement | Result |
| --- | --- |
| Create + park 20 spawn panes | **0.26 s** total |
| 20 unpark/repark round trips of one pane | **0.19 s** (≈5 ms each) |
| Holding session after parking 20 | 21 windows, 22 panes on the server |
| tmux server RSS, empty | 4.4 MB |
| tmux server RSS, 20 parked idle panes | 4.5 MB |
| **tmux server RSS, 20 panes at full 2000-line scrollback, 200 cols** | **216 MB** |
| `#{history_bytes}` for one such pane | 10,479,163 (≈10.5 MB) |

**[V]** All of it on tmux 3.4, one thread (`nlwp=1`).

Three things follow.

1. **Speed is a non-issue.** Parking costs milliseconds; the resize redraw it triggers (§3.3) costs
   far more than the tmux bookkeeping.
2. **Memory is the real cost, and it is the user's option that sets it.** `history-limit` defaults
   to 2000 **[V]**, but it is a *user* setting and 10k–50k is a common `.tmux.conf` value. Scaling
   the measured 10.5 MB per pane linearly, `history-limit 50000` × 20 panes extrapolates to
   **~5 GB**. **[I]** 216 MB is the *default-configuration* number, not the worst case. This directly
   qualifies the #18 claim that "the ~1.3 GB scrollback arithmetic disappears": it does not
   disappear, it moves into the tmux server and becomes governed by a setting the app does not own.
   The app can set `history-limit` per window (`set-option -w`) for windows it creates, which would
   bound it — untested here. **[I]**
3. **The measured figure is a near-worst case** — 190 printable characters on every line. tmux stores
   per-line only the cells actually used, so sparse output costs less. **[I]**

Not measured: 20 panes all producing output simultaneously into one control client (the §2.6
firehose), and whether `%output` fairness holds up. Flagged as the untested risk.

---

## 11. Rust crates

Searched crates.io (2026-08-05). Only one general-purpose library exists.

### `tmux_interface` — the only library, and it is CLI-only

- <https://github.com/AntonGepting/tmux-interface-rs> · <https://crates.io/crates/tmux_interface>
- Latest **0.4.0**, published **2026-03-10**; 148k downloads all-time, 63k recent. First published
  2019. Edition 2018. **[S]**
- **It is a typed builder over the `tmux` CLI**, not a control-mode or socket client. Its own
  description: "communication with TMUX via CLI". The example is `Tmux::new().add_command(NewSession::new()…).output()`
  — i.e. it assembles an argv and shells out. **[S]**
- **Nothing for control mode or notifications.** Nothing in the README or the crate description
  mentions `-C`, `%output`, or receiving events. **[S]**
- **Version handling is by Cargo feature**, one per tmux release: `tmux_3_6a`, `tmux_3_4`,
  `tmux_2_6`, … down to `tmux_1_8`. CI covers 1.8 through 3.6a on Linux only; Windows and macOS are
  unchecked boxes; `master` is an unchecked box. **[S]**
- **The README's own stability warning**: "The library is still in experimental development stage
  (unstable) — many features are unimplemented or not well tested, some APIs/structures/names can be
  changed in the future, almost all library documentation is missing at the moment." Versions below
  1.0.0 are "mostly for development and testing purposes (use them in your projects on your own
  risk)". **[S]**
- **Its README tells you to depend on `1.0.0`, which does not exist** — the highest published version
  is 0.4.0. **[S]** The documentation is ahead of the releases.

### `par-term-tmux` — a real Rust control-mode implementation, but not a general library

- <https://github.com/paulrobello/par-term> · <https://crates.io/crates/par-term-tmux>
- Published, 0.1.14 as of 2026-07-30, 436 downloads. **[S]**
- Its own doc comment: "This crate provides integration with tmux's control mode (`-CC`) … Attach to
  existing tmux sessions … Send input through tmux control protocol … Receive output and
  notifications", with modules `session.rs`, `commands.rs`, `sync.rs`, `types.rs`. It states that
  "the core library (par-term-emu-core-rust) provides the control mode parser". **[S]**
- It is a component of one terminal emulator, not a general-purpose library, and the parser it relies
  on lives in a sibling crate. Useful as **prior art for what a control-mode client in Rust looks
  like**; not a dependency to reach for. **[I]**

### Nothing speaks the socket protocol

No crate found implementing tmux's `imsg` client/server protocol. **[S]**

### Adjacent, for context

The search surfaced a large and growing set of "run coding agents in tmux panes" tools in Rust —
`workmux`, `bosun-tmux`, `botctl`, `triage-tui`, `claude-tmux`, `swimmers`, `cekanje`, `ilmari`,
among others, most first published in 2026. **[S]** Not examined here; several are in exactly this
problem space and may be worth a separate look for how they handle §3.3 and §5.5.

---

## 12. What this leaves for the decision, and what was not investigated

Open, for the human:

1. Control surface: control client, CLI shell-out, or both — given that events need hooks either way
   (§5.5) and watching parked spawns live needs a second control client (§2.2).
2. Whether the resize-on-park cost (§3.3) is acceptable, or whether the design should avoid parking
   on every selection change.
3. Whether to run on the user's server (takeover possible, holding session visible, global options
   shared) or on a private socket (§4.4, §7.1).
4. Minimum tmux version — 3.2 buys `ignore-size` and subscriptions, 3.3 buys `pane_dead_signal`
   (§9).
5. Whether the app should bound `history-limit` on windows it creates (§10).
6. How the app re-asserts the spawn pane's size and the window layout after unparking (§3.3).

Not investigated:

- `pause-after` / `%pause` / `%extended-output` flow control under real load (§2.6).
- Whether `set-hook -B` monitors in 3.8 close the death-notification gap (§9).
- `capture-pane -e` (escape sequences preserved) as a polling alternative to `%output`.
- macOS behaviour. Everything here is Linux.
- Nested tmux (the app's session running inside a user's outer tmux on the same server).
- The other Rust agent-in-tmux tools listed in §11.

---

## Sources

tmux manual and source, read at pinned tags (fetched 2026-08-05):

- `tmux.1` at 3.4 — <https://raw.githubusercontent.com/tmux/tmux/3.4/tmux.1>
- `tmux.1` at master — <https://raw.githubusercontent.com/tmux/tmux/master/tmux.1>
- `tmux.1` at 2.6, 2.8, 2.9, 3.0a, 3.1c, 3.2a, 3.3a, 3.5a — same URL pattern, used for §9
- `CHANGES` at master — <https://raw.githubusercontent.com/tmux/tmux/master/CHANGES>
- `cmd-break-pane.c` at 3.4 — <https://raw.githubusercontent.com/tmux/tmux/3.4/cmd-break-pane.c>
- `cmd-join-pane.c` at 3.4 — <https://raw.githubusercontent.com/tmux/tmux/3.4/cmd-join-pane.c>
- `control.c` at 3.4 — <https://raw.githubusercontent.com/tmux/tmux/3.4/control.c>
- `control-notify.c` at master — <https://raw.githubusercontent.com/tmux/tmux/master/control-notify.c>
- `spawn.c` at 3.4 — <https://raw.githubusercontent.com/tmux/tmux/3.4/spawn.c>
- `environ.c` at 3.4 — <https://raw.githubusercontent.com/tmux/tmux/3.4/environ.c>
- `tmux-protocol.h` at master and 3.4 — <https://raw.githubusercontent.com/tmux/tmux/master/tmux-protocol.h>
- tmux wiki, Control Mode — <https://github.com/tmux/tmux/wiki/Control-Mode>

Crates:

- `tmux_interface` — <https://github.com/AntonGepting/tmux-interface-rs> ·
  README read at <https://raw.githubusercontent.com/AntonGepting/tmux-interface-rs/master/README.md> ·
  <https://crates.io/api/v1/crates/tmux_interface>
- `par-term-tmux` — <https://crates.io/api/v1/crates/par-term-tmux> · `lib.rs` read at
  <https://raw.githubusercontent.com/paulrobello/par-term/main/par-term-tmux/src/lib.rs>
- crates.io search — <https://crates.io/api/v1/crates?q=tmux>

Packaged versions:

- Ubuntu — <https://launchpad.net/ubuntu/+source/tmux>
- Debian — <https://packages.debian.org/bookworm/tmux>, <https://packages.debian.org/search?keywords=tmux>
  (fetched via domain-restricted search; direct fetch returned HTTP 403)
- <https://repology.org/project/tmux/versions> returned HTTP 403 and was not used.

Experiments: all **[V]** claims were run on **`tmux 3.4`**, Linux 6.18 x86_64, on 2026-08-05, each
against a throwaway server on its own socket.
