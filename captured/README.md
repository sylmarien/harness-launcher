# Captured output

The status ladder is built on what three other programs print — tmux, `ps` and
the harness. Its tests read the recordings here rather than strings written from
memory, because a format remembered wrongly makes a parser that passes its tests
and fails in front of a user, and none of these three formats is one this project
controls.

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
