#!/usr/bin/env bash
# PROTOTYPE — throwaway. See issue #22.
#
# Does the park/unpark redraw make switching spawns unpleasant?
#
# Reproduces the real mechanism: a visible window holding a list pane beside a
# slot, and N children parked in a DETACHED holding session, moved in and out
# with break-pane / join-pane. Every switch resizes the child and SIGWINCHes it,
# which is the thing to look at.
#
# Runs on its own tmux socket (-L hlredraw). It cannot touch your real sessions,
# and `down` removes every trace.
#
#   ./redraw.sh up [N]      build it and attach   (default N=3)
#   ./redraw.sh down        tear it all down
#
# Inside, from the attached session:
#   C-b then 1..9   put that spawn in the slot   <- watch this
#   C-b b           bench: 20 switches, print timings
#   C-b d           tear down and detach
#
#   Alt+1..9 / Alt+b / Alt+d do the same, but only in terminals that send Meta.
#   macOS Terminal and iTerm2 do NOT by default, which is why the prefix keys
#   above are the ones documented.
#
# Knobs:
#   CMD=...      what to run as a child. Default: claude
#                Rehearse without tokens:  CMD='htop' ./redraw.sh up 3
#   CLASSIC=1    force Claude Code's classic renderer instead of fullscreen,
#                to compare (tranche 1 chose fullscreen)
#   PROMPT=...   initial prompt for claude, so you can watch a MID-TURN switch
#
# What to judge:
#   * a switch while the agent is mid-turn vs idle at the composer
#   * a small window vs a maximised one
#   * rapid back-and-forth vs an occasional jump
#   * your tmux version — 3.6 and older flicker more, lacking synchronised output

set -uo pipefail

SOCK=hlredraw
TMUX_="tmux -L $SOCK"
CMD=${CMD:-claude}
PROMPT=${PROMPT:-}
LOG=${TMPDIR:-/tmp}/hl-redraw-timings.log

self() { cd "$(dirname "${BASH_SOURCE[0]}")" && pwd; }

child_cmd() {
  local env=""
  [ -n "${CLASSIC:-}" ] && env="CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1 "
  if [ "$CMD" = claude ] && [ -n "$PROMPT" ]; then
    printf '%s' "${env}claude -n spawn-$1 \"$PROMPT\""
  elif [ "$CMD" = claude ]; then
    printf '%s' "${env}claude -n spawn-$1"
  else
    printf '%s' "$CMD"
  fi
}

up() {
  local n=${1:-3}
  local here_early; here_early=$(self)
  if $TMUX_ has-session -t view 2>/dev/null; then
    echo "already up — run './redraw.sh down' first"; exit 1
  fi
  if [ "$CMD" = claude ] && ! command -v claude >/dev/null; then
    echo "claude not on PATH. Rehearse with: CMD='htop' $0 up $n"; exit 1
  fi

  # children, each its own window in a DETACHED holding session
  $TMUX_ new-session -d -s hold -n s1 "$(child_cmd 1)"
  for i in $(seq 2 "$n"); do
    $TMUX_ new-window -d -t hold -n "s$i" "$(child_cmd "$i")"
  done
  $TMUX_ set-option -g remain-on-exit on

  # the visible window: list pane on the left, slot on the right
  $TMUX_ new-session -d -s view -n main -x 200 -y 50 "$here_early/list.sh"
  # keep a hint bar: these keys are the whole interface
  $TMUX_ set-option -g status on
  $TMUX_ set-option -g status-left-length 200
  $TMUX_ set-option -g status-right ''
  $TMUX_ set-option -g status-style 'bg=colour236 fg=colour250'
  $TMUX_ set-option -g status-left ' switch spawn: C-b then 1..9   ·   bench: C-b b   ·   down: C-b d   ·   (Alt+1..9 also works if your terminal sends Meta) '

  local here=$here_early
  for i in $(seq 1 9); do
    # prefix table: works in every terminal
    $TMUX_ bind-key "$i" run-shell "$here/redraw.sh swap $i"
    # root table: convenient, but needs a terminal that sends Meta
    $TMUX_ bind-key -n "M-$i" run-shell "$here/redraw.sh swap $i"
  done
  $TMUX_ bind-key b run-shell "$here/redraw.sh bench"
  $TMUX_ bind-key d run-shell "$here/redraw.sh down"
  $TMUX_ bind-key -n M-b run-shell "$here/redraw.sh bench"
  $TMUX_ bind-key -n M-d run-shell "$here/redraw.sh down"

  "$here/redraw.sh" swap 1
  : > "$LOG"
  echo "up with $n children (CMD=$CMD)."
  echo "switch spawn: press C-b then a digit 1..$n   (Alt+digit also works if your terminal sends Meta)"
  echo "bench: C-b b    tear down: C-b d"
  sleep 1
  $TMUX_ attach -t view
}

swap() {
  local want="s$1"
  $TMUX_ has-session -t view 2>/dev/null || exit 0
  $TMUX_ list-windows -t hold -F '#{window_name}' 2>/dev/null | grep -qx "$want" || exit 0

  local t0 t1 cur curname
  t0=$(date +%s%N)
  cur=$($TMUX_ show-options -wqv -t view:main @slotpane)
  curname=$($TMUX_ show-options -wqv -t view:main @slotname)
  if [ -n "$cur" ] && [ "$curname" != "$want" ]; then
    $TMUX_ break-pane -d -s "$cur" -n "$curname" -t hold: 2>/dev/null
  elif [ "$curname" = "$want" ]; then
    exit 0
  fi
  $TMUX_ join-pane -h -l 68% -s "hold:$want" -t view:main 2>/dev/null || exit 0
  t1=$(date +%s%N)

  local newpane
  newpane=$($TMUX_ list-panes -t view:main -F '#{pane_id}' | tail -1)
  $TMUX_ set-option -w -t view:main @slotpane "$newpane"
  $TMUX_ set-option -w -t view:main @slotname "$want"
  echo "$(( (t1 - t0) / 1000000 ))" >> "$LOG"
}

bench() {
  local here; here=$(self)
  : > "$LOG"
  for _ in $(seq 1 10); do
    "$here/redraw.sh" swap 1; "$here/redraw.sh" swap 2
  done
  local n min max avg
  n=$(wc -l < "$LOG"); min=$(sort -n "$LOG" | head -1); max=$(sort -n "$LOG" | tail -1)
  avg=$(awk '{s+=$1} END {printf "%.0f", s/NR}' "$LOG")
  $TMUX_ display-message -d 4000 "$n switches — min ${min}ms  avg ${avg}ms  max ${max}ms (timings are tmux's, not the repaint you see)"
}

down() {
  $TMUX_ kill-server 2>/dev/null
  echo "down."
}

case "${1:-}" in
  up)    shift; up "$@" ;;
  swap)  shift; swap "$@" ;;
  bench) bench ;;
  down)  down ;;
  *)     sed -n '2,40p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' ;;
esac
