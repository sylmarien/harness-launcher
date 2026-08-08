#!/usr/bin/env bash
# PROTOTYPE — the left-hand "list" pane. Stands in for the app, so the slot has
# something to sit beside and the window has realistic proportions.
T="tmux -L hlredraw"
while :; do
  clear
  printf '\033[1mSPAWNS\033[0m\n\n'
  slot=$($T show-options -wqv -t view:main @slotname 2>/dev/null)
  for w in $($T list-windows -t hold -F '#{window_name}' 2>/dev/null); do
    printf '  \033[2m·  %s   parked\033[0m\n' "$w"
  done
  [ -n "$slot" ] && printf '  \033[97m\033[1m▍● %s   in the slot\033[0m\n' "$slot"
  printf '\n\033[2mC-b 1..9  switch spawn\033[0m\n'
  printf '\033[2mC-b b     bench 20 switches\033[0m\n'
  printf '\033[2mC-b d     tear down\033[0m\n'
  sleep 1
done
