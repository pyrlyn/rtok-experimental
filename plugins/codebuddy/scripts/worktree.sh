#!/bin/sh
# Copyright (c) 2026 Ivan Tugay
# SPDX-License-Identifier: GPL-3.0-or-later
# Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

# Launcher for the WorktreeCreate / WorktreeRemove hooks of Claude Code (T159, D31) and CodeBuddy
# (T995.1), which share one contract; the second argument names the host (default claude). The
# same bytes ship in plugins/claude and plugins/codebuddy, each plugin being installed on its own.
# Unlike hook.sh it cannot fail open by printing nothing: the host replaces its own create and
# remove with these hooks, so a create that prints no path fails the session. Each failure path
# therefore ends in what the host itself would have done, never in a lost worktree.
event=$1
host=${2:-claude}
case "$host" in claude | codebuddy) ;; *)
  echo "rtok: unknown host $host" >&2
  exit 1
  ;;
esac
input=$(cat)
# The fetch behind `rtok worktree add` must never wait for a prompt the host cannot show.
GIT_TERMINAL_PROMPT=0
export GIT_TERMINAL_PROMPT

bin=
for b in "$(command -v rtok 2>/dev/null)" "$HOME/.ketch/bin/rtok" \
  /usr/local/bin/rtok /opt/homebrew/bin/rtok; do
  if [ -n "$b" ] && [ -x "$b" ]; then
    bin=$b
    break
  fi
done

# The payload is JSON, and a path may hold \" \\ or \uXXXX escapes that no regex undoes, so the
# fields go through a real parser: jq, else the perl every Git install ships (JSON::PP is core).
# Neither is a reason to guess: a payload that cannot be read fails the hook with the reason.
field() {
  if command -v jq >/dev/null 2>&1; then
    printf '%s' "$input" | jq -r --arg k "$1" '.[$k] | strings'
  elif command -v perl >/dev/null 2>&1; then
    printf '%s' "$input" | perl -MJSON::PP -e '
      local $/;
      my $d = eval { decode_json(<STDIN>) };
      die "rtok: hook payload is not a JSON object\n" unless ref $d eq "HASH";
      binmode STDOUT, ":utf8";
      my $v = $d->{$ARGV[0]};
      print "$v\n" if defined $v && !ref $v;
    ' "$1"
  else
    echo "rtok: no jq or perl found to read the hook payload" >&2
    return 1
  fi
}

case "$event" in
WorktreeCreate)
  if [ -n "$bin" ]; then
    # Anything but a path falls through: an rtok too old to know the event prints `{}`, and
    # `[worktree] enabled = false` prints nothing so the host gets its own worktree.
    if out=$(printf '%s' "$input" | "$bin" hook WorktreeCreate --host "$host"); then
      case "$out" in
      /* | [A-Za-z]:*)
        printf '%s\n' "$out"
        exit 0
        ;;
      esac
    else
      echo "rtok: rtok worktree add failed; creating a plain worktree" >&2
    fi
  fi
  # printf: a POSIX sed ends its output with a newline, and tr -c would turn it into a dash.
  name=$(field name) || exit 1
  cwd=$(field cwd) || exit 1
  name=$(printf '%s' "$name" | tr -c 'A-Za-z0-9._-' '-')
  case "$name" in '' | .* | -*) name="w-$name" ;; esac
  root=$(git -C "$cwd" rev-parse --show-toplevel) || exit 1
  # The host's own default: .<host>/worktrees/<name> on worktree-<name>, from the current HEAD.
  # CodeBuddy documents the folder; its branch name is not documented and taken from Claude Code.
  dir=$root/.$host/worktrees/$name
  if [ ! -e "$dir/.git" ]; then
    git -C "$root" worktree add --quiet -b "worktree-$name" "$dir" >&2 ||
      git -C "$root" worktree add --quiet "$dir" "worktree-$name" >&2 || exit 1
  fi
  printf '%s\n' "$dir"
  ;;
WorktreeRemove)
  if [ -n "$bin" ]; then
    printf '%s' "$input" | "$bin" hook WorktreeRemove --host "$host"
    exit $?
  fi
  path=$(field worktree_path) || exit 1
  cwd=$(field cwd) || exit 1
  [ -d "$path" ] || exit 0
  # Never --force: git refuses a dirty worktree, and the host keeps it and reports the reason.
  git -C "$cwd" worktree remove "$path"
  ;;
esac
