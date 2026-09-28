#!/bin/sh
# A fake `claude` that replays a recording (docs/CHAT-PANE-DESIGN.md §9):
#
#   replay.sh RECORDING LOGDIR [the runtime's argv...]
#
# RECORDING is one of this directory's .jsonl files. Its "out" lines are
# printed in order; at each "in" line the fake reads one line of its stdin
# and checks it is the same KIND — an initialize, a user message, an
# interrupt, or an answer (allow / deny / error) to the SAME request id —
# never the same bytes: the cockpit's own ids are fresh, so each recorded id
# of the cockpit's (a request id, a message uuid) is replaced by the one it
# read in every line printed after. A mismatch prints "replay: …" on stderr
# and exits 3. At the end it reads stdin to EOF (as the runtime waits for
# its next message) and exits 0.
#
# LOGDIR receives: argv, env, cwd, and every stdin line (stdin).
#
# Also honoured, for the end-routine tests: REPLAY_IGNORE_TERM=1 (trap TERM),
# REPLAY_GRANDCHILD=1 (a `sleep` in the group, its pid in LOGDIR/grandchild),
# REPLAY_TTY=1 (read /dev/tty once the script is played — a stopped runtime),
# REPLAY_HOLD=1 (sleep instead of reading stdin to EOF).
rec=$1
log=$2
shift 2
pwd > "$log/cwd"
env > "$log/env"
: > "$log/stdin"
for a in "$@"; do printf '%s\n' "$a"; done > "$log/argv"
[ "$REPLAY_IGNORE_TERM" = 1 ] && trap '' TERM
if [ "$REPLAY_GRANDCHILD" = 1 ]; then
  sleep 300 < /dev/null > /dev/null 2>&1 &
  echo $! > "$log/grandchild"
fi
subst=''
n=0
fail() {
  echo "replay: step $n: $1" >&2
  exit 3
}
while IFS= read -r rec_line <&4; do
  n=$((n + 1))
  case "$rec_line" in
    '{"dir": "out", "msg": '*)
      msg=$(printf '%s\n' "$rec_line" | sed -e 's/^{"dir": "out", "msg": //' -e 's/, "t": [0-9.]*}$//')
      if [ -n "$subst" ]; then
        printf '%s\n' "$msg" | sed $subst
      else
        printf '%s\n' "$msg"
      fi
      ;;
    '{"dir": "in", "msg": '*)
      IFS= read -r line || fail "stdin closed"
      printf '%s\n' "$line" >> "$log/stdin"
      case "$rec_line" in
        *'"subtype": "initialize"'*) want='"subtype":"initialize"'; field=request_id ;;
        *'"subtype": "interrupt"'*) want='"subtype":"interrupt"'; field=request_id ;;
        *'"type": "user"'*) want='"type":"user"'; field=uuid ;;
        *'"behavior": "allow"'*) want='"behavior":"allow"'; field='' ;;
        *'"behavior": "deny"'*) want='"behavior":"deny"'; field='' ;;
        *'"type": "control_response"'*) want='"type":"control_response"'; field='' ;;
        *) want=''; field='' ;;
      esac
      case "$line" in
        *"$want"*) ;;
        *) fail "wanted $want, read: $line" ;;
      esac
      if [ -z "$field" ]; then
        # An answer: to the request the runtime asked (its id is its own).
        id=$(printf '%s\n' "$rec_line" | sed -n -e 's/.*"request_id": "\([^"]*\)".*/\1/p')
        case "$line" in
          *"\"request_id\":\"$id\""*) ;;
          *) fail "wanted an answer to $id, read: $line" ;;
        esac
      else
        old=$(printf '%s\n' "$rec_line" | sed -n -e "s/.*\"$field\": \"\\([^\"]*\\)\".*/\\1/p")
        new=$(printf '%s\n' "$line" | sed -n -e "s/.*\"$field\":\"\\([^\"]*\\)\".*/\\1/p")
        [ -n "$new" ] || fail "no $field in: $line"
        [ -n "$old" ] && subst="$subst -e s/$old/$new/g"
      fi
      ;;
  esac
done 4< "$rec"
[ "$REPLAY_TTY" = 1 ] && read -r _ < /dev/tty
# REPLAY_HOLD=1: hold on past the recording, deaf to stdin's EOF (and to
# TERM when the caller ignored it: a disposition exec keeps).
[ "$REPLAY_HOLD" = 1 ] && exec sleep 300
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log/stdin"
done
exit 0
