#!/bin/zsh
# Launches Buzz dev in the Aqua (GUI) session so the app gets keyboard focus.
# Claude Code / tmux shells run in a Background launchd session — apps launched
# there can never become the active app. See memory: gui-apps-need-aqua-session.
cd "$(dirname "$0")"
echo "session: $(launchctl managername)"   # should print Aqua
. ./bin/activate-hermit
exec just dev
