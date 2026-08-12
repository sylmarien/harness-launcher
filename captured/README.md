# Captured output

The app reads what other programs print — tmux, `ps`, the harness, and now
whatever a spawn draws. The tests read the recordings here rather than strings
written from memory, because a format remembered wrongly makes a parser that
passes its tests and fails in front of a user, and not one of these formats is
one this project controls.

Each file says how it was made, so it can be made again on a machine where the
answer might differ.

## `tmux-list-panes.txt`

**tmux 3.4**, Linux. One `list-panes` covering every pane on the server, in the
format a tick uses:

```
tmux new-session -d -s harness-launcher-add-retry-logic-a7f3 -x 120 -y 30 -- sleep 300
tmux set-option -w -t harness-launcher-add-retry-logic-a7f3: remain-on-exit on
tmux split-window -h -t harness-launcher-add-retry-logic-a7f3: -- sleep 300
tmux split-window -h -t harness-launcher-add-retry-logic-a7f3: -- sh -c 'exit 3'
tmux new-session -d -s other -x 80 -y 24 -- sleep 300
tmux list-panes -a -F '#{pane_id} #{pane_dead} #{pane_pid} #{pane_tty}'
```

`%2` is the pane whose command exited: `remain-on-exit` kept it, and tmux
reports `pane_dead` as `1` rather than dropping the pane. Worth noticing that
its `pane_tty` is `/dev/pts/3` — **the same terminal `%3` now holds**, because a
dead pane's terminal is released and handed to the next pane that asks. Nothing
may probe a dead pane by its terminal; the ladder never does, because death
resolves before any probe is considered.

## `tmux-list-panes-in-session.txt`

**tmux 3.4**, Linux. The listing both reports are built from — `list-panes`
scoped to the holding session, printing each pane's *window* name and whether
what ran in it has stopped. Named for the command that made it rather than for
the field it prints, so that it cannot be mistaken for a `list-windows`:

```
tmux new-session -d -s spawns -x 61 -y 17 -n holding -- sh -c 'while :; do sleep 3600; done'
tmux set-option -g -w remain-on-exit on
tmux new-window -d -t spawns -n add-retry-logic-a7f3 -- sh -c 'while :; do sleep 3600; done'
tmux new-window -d -t spawns -n fix-the-flake-b2c9   -- sh -c 'while :; do sleep 3600; done'
tmux new-window -d -t spawns -n drop-the-cache-d4e1  -- sh -c 'exit 3'
tmux new-session -d -s other -x 80 -y 24 -- sleep 300
tmux list-panes -s -t spawns -F '#{window_name} #{pane_dead}'
```

Three things this pins down that a remembered format would not.

**A window is named after the spawn in it**, which is what lets a report say
which spawns are still running without the app having to remember them — and is
the whole reason the reports can be taken by an app that has just started and
knows nothing.

**The holding window is in the listing too.** It is the furniture that keeps the
session alive before the first spawn and after the last one stops, and it is
named `holding` precisely so it can be told apart from a spawn. A report that
counted it would say one more spawn than there is, for ever.

**`-s -t spawns` is scoped to the session, not the server.** The `other` session
above was created before this listing was taken and does not appear in it —
which is what makes the report about the app's own spawns rather than about
whatever else happens to be on the socket. `drop-the-cache-d4e1` is the window
whose command exited: `remain-on-exit` kept it, and it reports `1`.

## `tmux-control-mode.txt`

**tmux 3.4** and **vim 9.1**, Linux. The `%output` notifications a control-mode
client received while a real program drew a real screen — which is the whole of
what the app's own terminal emulation is fed:

```
tmux -L rec new-session -d -s spawns -x 20 -y 5 -- sh -c 'while :; do sleep 3600; done'
tmux -L rec set-option -g -w remain-on-exit on
script -q -f -c "tmux -L rec -CC attach -t spawns" raw.txt &   # a control client needs a tty
printf 'refresh-client -C 60x16\n' > <the client's input>
P=$(tmux -L rec new-window -d -t spawns -n s1 -P -F '#{pane_id}' -- sh -c 'while :; do sleep 3600; done')
tmux -L rec respawn-pane -k -t $P -e TERM=xterm-256color -- vim -u NONE -N --cmd 'set encoding=utf-8' note.txt
grep '^%output' raw.txt > tmux-control-mode.txt
```

`note.txt` is three lines of box drawing with `世界` inside it, so the recording
carries the two things a hand-written string would get wrong: **wide characters**
that must take two cells and push nothing along, and **box drawing** that must
line up column for column afterwards.

Four things this pins down that no remembered format would.

**The escaping.** Anything a line protocol could not survive arrives as three
octal digits behind a backslash — `\033`, `\015\012` — **and so does the
backslash itself**, as `\134`. That last one is what makes it unambiguous.
Everything else, UTF-8 included, is passed through as raw bytes: read the file
with `od -c` and the box-drawing characters are there untouched.

**Where the chunks fall.** One line per read from the pane, wherever that landed
— so `%output` boundaries fall in arbitrary places, and half a wide character at
the end of a line is ordinary. Escaping happens *after* chunking, so an escape is
never split; a UTF-8 sequence is, which is why the app reads this as bytes.

**The line terminator is `\r\n`.** The client writes into a terminal, and a
terminal turns every newline into both. Left on, that carriage return reaches the
emulator as something the spawn drew.

**A real terminal query, unanswered here.** Line 3 contains `\033[6n` — vim
asking where the cursor is. The app never answers it, and does not need to: the
pane's terminal is tmux, which replies before passing the bytes on. There is an
integration test for that in `src/control.rs`, because it is the one thing in the
design named as its sharpest risk.

The recording stops mid-life, as a spawn's output always does — the last
`%output` is a full redraw and the file simply ends.

## `ps-foreground.txt`

Linux `procps` `ps`, against the terminal of a pane running a job-control shell
with one job in the background and one in the foreground:

```
ps -t /dev/pts/N -o pgid=,tpgid=,comm=,args=
```

Two things this recording pins down. The **foreground process group** is the one
where `pgid` equals `tpgid`, which here is the third row and not the shell that
started it. And `comm` is **truncated to fifteen characters** by the kernel —
`stand-in-harnes` is the whole of what a program called `stand-in-harness`
reports — which is why a name is looked for in the full argument vector as well.

The program in the pane is a stand-in, exactly as in the tmux tests: no harness
is ever really started by a test, because the real one costs tokens and needs
credentials.

## `ps-harness.txt`

The same command, against a pane whose foreground process group **is** the
harness — a stand-in carrying the harness's name, since the point is what `ps`
prints rather than what the program does.

Two rows share the foreground group here, and only one of them names the
harness, which is why the probe reads every foreground row before concluding
that the agent has gone. The columns are also right-aligned, so the numbers
arrive with leading spaces: another reason to read this from a recording rather
than from an idea of what `ps` output looks like.

## `session-record.json`

**Claude Code 2.1.226**, from `<config dir>/sessions/<pid>.json`, copied
verbatim.

Read what is *not* there: this record carries **no `status` field**.

The field is real — it has since been confirmed present on an ordinary local
install, alongside the research on the prior art that first described it (`idle`
⇄ `busy` ⇄ `waiting`, plus `shell` since 2.1.197). What this recording shows is
that it is **not always** written: the machine it came from runs a session
started remotely, and its record has no status at all. That is the undocumented
internal detail the design says to treat as fallible, caught being fallible on
the first machine we looked at, and it is why the ladder has a rung for a record
that will not resolve rather than assuming one that always does.

The consequence for the tests is that the records *carrying* a status — the ones
needed to cover the mapping — are **this capture with the one field added**,
through the single helper `harness::recorded`. That is stated where it happens
rather than dressed up as a recording. Replacing this file with a capture that
does carry a status would make those tests read a real record end to end; until
then, the field's name and its four values are the one thing here taken on
trust rather than observed.
