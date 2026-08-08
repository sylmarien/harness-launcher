#!/usr/bin/env python3
"""
PROTOTYPE — throwaway. Not production code. See issue #13.

Layout C, chosen from three variants: the list groups by repository, and status
is carried by an icon plus a colour.

Two things this version exists to show:

  * Nothing is a fixed size. The list pane and the slot are computed from the
    real terminal every frame — maximise the window and it uses the space.
  * The right-hand pane is a SLOT. It shows either a spawn or the new-spawn
    draft, and moving between them is the same gesture. A draft you walk away
    from is still there when you come back.

    python3 prototypes/list-pane-layouts.py

    j / k       move down / up the list
    n           start a new spawn (creates a draft, selects it)
    t           type another line into the draft (shows it persists)
    e           toggle "this harness offers no effort levels"
    q           quit

Deliberately not answered: keybindings proper, scrolling past a screenful, and
what the redraw looks like when switching spawns for real. That one needs live
tmux, not a mock.
"""

import os
import shutil
import sys
import termios
import tty

RESET = "\033[0m"
BOLD = "\033[1m"
WHITE = "\033[97m"
GREY = "\033[38;5;244m"
FAINT = "\033[38;5;238m"
AMBER = "\033[38;5;214m"
CYAN = "\033[38;5;80m"

MARK = {
    "working": f"{FAINT}·{RESET}",
    "stopped": f"{WHITE}{BOLD}●{RESET}",
    "unknown": f"{AMBER}{BOLD}?{RESET}",
}
DOT = {
    "working": f"{FAINT}▪{RESET}",
    "stopped": f"{WHITE}{BOLD}▪{RESET}",
    "unknown": f"{AMBER}{BOLD}▪{RESET}",
}
ORDER = {"stopped": 0, "unknown": 1, "working": 2}

SPAWNS = [
    ("harness-launcher", "add-retry-logic",        "working", "4m"),
    ("harness-launcher", "fix-worktree-cleanup",   "stopped", "31m"),
    ("harness-launcher", "status-ladder",          "working", "2m"),
    ("harness-launcher", "tmux-park-unpark",       "working", "11m"),
    ("harness-launcher", "spawn-form-choices",     "unknown", "1h4m"),
    ("acme-api",         "rate-limit-headers",     "working", "7m"),
    ("acme-api",         "drop-legacy-auth",       "stopped", "22m"),
    ("acme-api",         "openapi-drift-check",    "working", "3m"),
    ("acme-api",         "flaky-integration-test", "working", "18m"),
    ("acme-api",         "pagination-cursors",     "working", "9m"),
    ("acme-api",         "audit-log-schema",       "working", "44m"),
    ("dotfiles",         "nvim-lsp-rework",        "working", "6m"),
    ("dotfiles",         "zsh-startup-time",       "stopped", "2h1m"),
    ("infra",            "terraform-state-move",   "working", "13m"),
    ("infra",            "bump-node-runners",      "working", "27m"),
    ("infra",            "cache-docker-layers",    "working", "5m"),
    ("infra",            "prune-old-artifacts",    "working", "51m"),
    ("infra",            "rotate-deploy-keys",     "working", "38m"),
]

DRAFT_LINES = [
    "fix the worktree cleanup so retiring a dirty spawn",
    "refuses instead of deleting. the check currently",
]
MORE_LINES = [
    "inherits status.showUntrackedFiles, which goes blind",
    "to untracked files when the user has it off.",
]


def vlen(s):
    out, i = 0, 0
    while i < len(s):
        if s[i] == "\033":
            while i < len(s) and s[i] != "m":
                i += 1
            i += 1
        else:
            out += 1
            i += 1
    return out


def pad(s, w):
    return s + " " * max(0, w - vlen(s))


def trunc(s, w):
    return s if len(s) <= w else s[: max(1, w - 1)] + "…"


def build_rows(list_w, draft, typed):
    """The list pane. Returns (rows, selectable) where selectable is a list of
    ('spawn', idx) / ('draft', None) in the order they appear."""
    rows, sel = [], []
    name_w = max(10, list_w - 12)

    if draft:
        rows.append(f"{CYAN}{BOLD}DRAFT{RESET}")
        n = len(DRAFT_LINES) + (len(MORE_LINES) if typed else 0)
        rows.append(("draft", f"{CYAN}✎{RESET} {trunc('new spawn — unfinished', name_w)}", f"{n}L"))
        sel.append(("draft", None))
        rows.append("")

    repos = []
    for repo, *_ in SPAWNS:
        if repo not in repos:
            repos.append(repo)

    for repo in repos:
        members = [(i, s) for i, s in enumerate(SPAWNS) if s[0] == repo]
        members.sort(key=lambda p: ORDER[p[1][2]])
        bar = "".join(DOT[s[2]] for _, s in members)
        rows.append(f"{BOLD}{trunc(repo, list_w - len(members) - 2)}{RESET} {bar}")
        for idx, (_, name, status, age) in members:
            nm = trunc(name, name_w)
            nm = f"{WHITE}{nm}{RESET}" if status != "working" else f"{GREY}{nm}{RESET}"
            rows.append((idx, f"{MARK[status]} {nm}", age))
            sel.append(("spawn", idx))
        rows.append("")
    return rows, sel


def render_list(rows, list_w, cursor, sel):
    out = []
    key = sel[cursor] if sel else None
    for r in rows:
        if isinstance(r, tuple):
            ident, label, right = r
            is_sel = (key == ("draft", None) and ident == "draft") or (
                key is not None and key[0] == "spawn" and ident == key[1]
            )
            bar = f"{CYAN}▍{RESET}" if is_sel else " "
            out.append(f"{bar}{pad(label, list_w - 7)}{FAINT}{right:>5}{RESET}")
        else:
            out.append(r)
    return out


def spawn_pane(idx, w):
    repo, name, status, age = SPAWNS[idx]
    head = f"─ claude ─ {name} ─ {repo} "
    body = [
        f"{FAINT}┌{trunc(head, w - 2)}{'─' * max(0, w - 2 - len(head))}{RESET}",
        "",
        f"{GREY}> {trunc('fix the flaky integration test in the payments suite', w - 4)}{RESET}",
        "",
        f"  {CYAN}⏺{RESET} Reading how the suite sets up its fixtures.",
        "",
        f"  {FAINT}Read(tests/payments/conftest.py){RESET}",
        f"  {FAINT}└─ 96 lines{RESET}",
        "",
        f"  {CYAN}⏺{RESET} The fixture shares a client across tests, so ordering leaks.",
    ]
    if status == "stopped":
        body += ["", f"{AMBER}  Shall I give each test its own client?{RESET}",
                 f"{FAINT}  1. Yes   2. Yes, and add a regression test   3. Explain first{RESET}"]
    elif status == "unknown":
        body = [f"{FAINT}┌{trunc(head, w - 2)}{'─' * max(0, w - 2 - len(head))}{RESET}", "",
                f"{AMBER}  unknown — the app cannot determine this spawn's state{RESET}", "",
                f"{FAINT}  the session status file did not resolve for pid 48213{RESET}",
                f"{FAINT}  the pane is alive; last known status was working, 1h4m ago{RESET}"]
    return body


def draft_pane(w, typed, no_effort):
    lines = DRAFT_LINES + (MORE_LINES if typed else [])
    levels = [] if no_effort else ["low", "medium", "high", "xhigh", "max"]
    eff = (
        f"{FAINT}(this harness offers no effort levels — control omitted){RESET}"
        if no_effort
        else "  ".join(
            (f"{WHITE}{BOLD}[{e}]{RESET}" if e == "high" else f"{GREY} {e} {RESET}") for e in levels
        )
    )
    body = [
        f"{CYAN}┌─ new spawn {'─' * max(0, w - 13)}{RESET}",
        "",
        f"{GREY}Repository{RESET}",
        f"  {WHITE}~/code/harness-launcher{RESET}  {FAINT}main ← default branch{RESET}",
        "",
        f"{GREY}What should it do?{RESET}",
    ]
    for l in lines:
        body.append(f"  {WHITE}{trunc(l, w - 4)}{RESET}")
    body += [
        f"  {CYAN}▌{RESET}",
        "",
        f"  {FAINT}→ spawn/fix-worktree-cleanup-a7f3{RESET}",
        "",
        f"{GREY}Model{RESET}",
        "  " + "  ".join(
            (f"{WHITE}{BOLD}[{m}]{RESET}" if m == "opus" else f"{GREY} {m} {RESET}")
            for m in ["fable", "opus", "sonnet"]
        ),
        "",
        f"{GREY}Effort{RESET}",
        "  " + eff,
        "",
        f"{FAINT}The app does not know what these choices mean — the harness hands it{RESET}",
        f"{FAINT}{{id, label}} lists and the form asks \"which of these?\".{RESET}",
        "",
        f"{FAINT}You can walk away from this. Pick a spawn on the left, deal with it,{RESET}",
        f"{FAINT}come back — the draft is still here, still in the list.{RESET}",
    ]
    return body


def render(cursor, draft, typed, no_effort):
    cols, rows_n = shutil.get_terminal_size((100, 30))
    list_w = max(28, min(46, cols // 3))
    slot_w = max(20, cols - list_w - 4)

    rows, sel = build_rows(list_w, draft, typed)
    cursor = max(0, min(cursor, len(sel) - 1)) if sel else 0
    left = render_list(rows, list_w, cursor, sel)

    kind, idx = sel[cursor] if sel else ("spawn", 0)
    right = draft_pane(slot_w, typed, no_effort) if kind == "draft" else spawn_pane(idx, slot_w)

    sys.stdout.write("\033[H\033[J")
    print(f"{FAINT} tmux window {'─' * max(0, cols - 14)}{RESET}")
    body_h = max(0, rows_n - 5)
    for i in range(min(body_h, max(len(left), len(right)))):
        l = left[i] if i < len(left) else ""
        r = right[i] if i < len(right) else ""
        print(f" {pad(l, list_w)} {FAINT}│{RESET} {r}")
    print(f"{FAINT} {'─' * max(0, cols - 2)}{RESET}")
    print(f" {FAINT}{cols}×{rows_n} — list {list_w}, slot {slot_w}. Resize the window; nothing is fixed.{RESET}")
    print(f" {FAINT}j/k move   n new spawn   t type into draft   e empty-effort   q quit{RESET}")
    return cursor, len(sel)


def main():
    cursor, draft, typed, no_effort = 0, False, False, False
    fd = sys.stdin.fileno()
    old = termios.tcgetattr(fd)
    try:
        tty.setcbreak(fd)
        while True:
            cursor, n = render(cursor, draft, typed, no_effort)
            k = sys.stdin.read(1)
            if k == "q":
                break
            elif k == "j":
                cursor = min(cursor + 1, n - 1)
            elif k == "k":
                cursor = max(cursor - 1, 0)
            elif k == "n":
                draft, cursor = True, 0
            elif k == "t" and draft:
                typed = not typed
            elif k == "e":
                no_effort = not no_effort
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, old)
        sys.stdout.write("\033[H\033[J")


if __name__ == "__main__":
    main()
