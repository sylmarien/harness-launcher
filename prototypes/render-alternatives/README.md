# Prototypes: alternatives to two settled decisions

Throwaway. These exist so the decisions in
[`docs/design/tranche-01-the-core-loop.md`](../../docs/design/tranche-01-the-core-loop.md)
can be re-tested against reality rather than re-argued.

Under test:

- **§4.1** the app *drives* tmux and does not embed terminals
- **§4.8** plain per-command `tmux`, **no control mode**

## Two backends, one binary

```bash
cargo run -- --backend tmux-cc     # tmux owns the processes, in CONTROL MODE; we render
cargo run -- --backend pty         # no tmux at all; we own the ptys and render
cargo run -- --backend pty --n 5   # how many children (1..9, default 3)
cargo run -- --backend pty --cmd htop        # rehearse without spending tokens
cargo run -- --backend pty -- --model opus   # anything after `--` goes to the children
```

| key | |
| --- | --- |
| `F1` … `F9` | switch child |
| `F10` | quit |
| anything else | goes to the selected child |

Function keys rather than digits, because a digit is exactly what you need to
send when Claude Code asks you to pick an option.

**This covers all three questions.** "Does Claude Code render properly under tmux
control mode" is answered by `--backend tmux-cc` — that binary *is* a control-mode
client, so what you see is our rendering of a control-mode stream. No third-party
terminal is involved.

## What is actually being compared

Both backends **share one renderer**, so the difference between them is purely
*who owns the process and how the bytes reach us* — not how they are drawn. If
they look identical, that is the finding: control mode buys nothing over owning
the pty, because either way we are the ones doing the emulation.

The renderer is deliberately the **real Rust stack the design rejected**: `vt100`
for terminal emulation, drawn by hand into ratatui cells. That matters. `vt100`
cannot write back to the child, so terminal queries such as cursor-position
reports go unanswered — and Claude Code is reported to issue them. Prototyping
this with a friendlier emulator from another ecosystem would give a **falsely
optimistic** answer about the option actually on the table.

`tui-term` is deliberately not used; drawing the grid by hand keeps the
experiment about `vt100`'s fidelity rather than a wrapper's version compatibility.

## The contrast worth noticing

**Switching costs nothing here.** Every child keeps its own grid in memory, so
selecting another one is a re-render, not a resize — no `SIGWINCH`, no repaint.

That is the substantive argument *for* these alternatives. The current design
parks and unparks tmux panes, which resizes the child and forces a full repaint
on every switch. That was measured and judged acceptable — but "acceptable" is
not "free", and this is what free looks like.

## Three things learned while building this

All found by testing, and each would otherwise have looked like "control mode
cannot render" when the truth was "the client was wrong".

**A control-mode client needs a tty.** With piped stdio tmux refuses outright:
`tcgetattr failed: Inappropriate ioctl for device`. The control client here runs
inside a pty of our own.

**Control mode streams only what is produced while a client is attached.**
Attaching *after* a child has drawn itself leaves a permanently blank screen,
with no catch-up short of priming from `capture-pane`. So the tmux-cc backend
starts every pane with a holder process, attaches, and only then respawns each
with the real command.

That one is a genuine strike against control mode independent of fidelity: any
supervisor that restarts, or attaches to something already running, has to solve
screen priming. Plain per-command tmux never poses the problem.

**One control client carries every pane in the session**, including windows that
are not visible — verified on tmux 3.4, with pane ids `%0`…`%(n-1)` in creation
order on a fresh server. That is why the backend kills the server first, and it
is what makes routing by pane id viable at all.

## What to look for

- **Fidelity** — colours, box drawing, the spinner, the prompt box, wide
  characters, the status line.
- **Queries** — anything that hangs or misbehaves in a way suggesting the child
  asked the terminal a question nobody answered.
- **Resize** — resize your terminal; does the child reflow sanely? (The children
  are sized once at startup here, so this is a known rough edge.)
- **Input** — typing, `Enter`, `Esc`, arrows, `Ctrl-C` on a running turn.
- **Switching** — flip between children repeatedly, mid-turn and idle.
- **Mouse and scrollback** — known weak points of this approach, and not
  implemented here at all.

## What this cannot tell you

Three or five panes is not twenty. The scrollback-memory arithmetic that helped
kill embedding — roughly 1.3 GB across twenty panes — is untested here, and a
handful of panes is exactly where this approach looks cheapest.
