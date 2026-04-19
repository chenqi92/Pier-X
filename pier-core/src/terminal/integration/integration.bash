# Pier-X shell integration — bash v2
#
# Sourced by `bash --rcfile <this_file> -i` on every Pier-X SSH tab
# (M2.7) and, when the user opts in, by the user's own `~/.bashrc`
# / `~/.zshrc` on the local machine (M4). Jobs:
#
#   1. Emit OSC 7 (`file://HOST/PATH`) before every prompt so the
#      Pier-X left file panel can follow `cd` in real time. This
#      works on both local and remote shells — the sequence is just
#      a VT control string; the parser lives in pier-core's
#      emulator.
#
#   2. Wrap the `ssh` command with a shell function that
#      best-effort uploads this very rc to the next host and then
#      launches the remote shell with `bash --rcfile …`. That way a
#      manual `ssh user@box` from inside a Pier-X terminal stays
#      observed — nested `cd` on the other end also flows back via
#      OSC 7. If the upload fails (agent isn't forwarded, sftp
#      denied, whatever) we silently drop back to plain `ssh` so the
#      user's command still works.

# ── 1. Source the user's normal bash rc first ─────────────────
# Run the user's environment before we install our hooks —
# otherwise a user rc that sets PROMPT_COMMAND or defines its own
# `ssh` function would clobber ours instead of layering cleanly.
if [ -f "$HOME/.bashrc" ]; then
    # shellcheck source=/dev/null
    . "$HOME/.bashrc"
fi

# ── 2. OSC 7 cwd + OSC 133 command boundaries ────────────────
# OSC 7  :  ESC ] 7   ; file:// HOST PATH         ESC \
# OSC 133:  ESC ] 133 ; A|B|C|D [ ; <exit-code> ] ESC \
#   A = prompt start, B/C = command start,
#   D = command finished (optional exit code).
# The Pier-X emulator consumes these to (a) track the remote cwd
# and (b) expose a reliable "command in-flight" signal so panels
# like Git can auto-refresh right after a command finishes.
__pier_prompt_start() {
    # Capture $? BEFORE anything else so our OSC doesn't overwrite
    # the user's exit code. `$?` is the last real command's status.
    local __pier_last_exit=$?
    local __pier_host
    __pier_host=$(hostname 2>/dev/null || echo "")
    # OSC 133 D with the exit from the command just completed.
    printf '\033]133;D;%d\033\\' "$__pier_last_exit"
    # OSC 7 cwd.
    printf '\033]7;file://%s%s\033\\' "$__pier_host" "$PWD"
    # OSC 133 A — "prompt is about to paint".
    printf '\033]133;A\033\\'
    # Preserve the user's $? for their own PS1 / PROMPT_COMMAND.
    return $__pier_last_exit
}

# `B` fires on command start — bash's DEBUG trap runs right before
# each simple command. We only want to emit once per prompt cycle,
# so we gate on `PROMPT_COMMAND` being the previous caller: if we
# just painted a prompt (PS0 phase), the next DEBUG trap is the
# first real command.
__pier_command_start() {
    # Skip the trap frames that fire during PROMPT_COMMAND itself.
    [ "$BASH_COMMAND" = "__pier_prompt_start" ] && return 0
    printf '\033]133;B\033\\'
}

case "$PROMPT_COMMAND" in
    *__pier_prompt_start*) ;;
    "") PROMPT_COMMAND="__pier_prompt_start" ;;
    *)  PROMPT_COMMAND="__pier_prompt_start; $PROMPT_COMMAND" ;;
esac
trap '__pier_command_start' DEBUG

# Emit OSC 7 + OSC 133 A once on startup so the panel catches up
# immediately instead of waiting for the first Enter.
printf '\033]7;file://%s%s\033\\' "$(hostname 2>/dev/null || echo "")" "$PWD"
printf '\033]133;A\033\\'

# ── 3. Nested ssh hijacker ────────────────────────────────────
# Goal: when the user types `ssh [flags] [user@]host [command]`,
# try to upload this rc to the target first, then launch the
# remote shell with `bash --rcfile <that path>`. If anything
# fails, fall through to a plain ssh so the command still works.
#
# We keep the flag parser deliberately minimal: recognise `-p
# PORT`, pass everything else through. If the user supplied a
# remote command (i.e. a non-flag arg after the host), we do NOT
# hijack — they likely want `ssh host 'one-liner'` semantics, not
# an interactive shell.

__pier_ssh_integration_path="$HOME/.pier-x/integration.sh"

__pier_parse_ssh_target() {
    # Walk argv; on success echoes "host port" to stdout, returns 0.
    # On any shape we don't understand (remote command, flags we
    # don't know what to do with), returns 1 and echoes nothing.
    local port=22
    local host=""
    local saw_command=0
    while [ $# -gt 0 ]; do
        case "$1" in
            -p)
                shift
                [ $# -gt 0 ] || return 1
                port="$1"
                ;;
            -l)
                # user override — we don't need to split it out
                # because the remote shell will already know who
                # it is; drop the flag.
                shift
                [ $# -gt 0 ] || return 1
                ;;
            -i|-F|-o|-J|-L|-R|-D|-W|-B|-b|-c|-E|-e|-I|-m|-O|-Q|-S|-w)
                # Flags that take an argument — skip both.
                shift
                [ $# -gt 0 ] || return 1
                ;;
            -*)
                # Boolean flag (e.g. -v, -4, -A). Leave it alone.
                ;;
            *)
                if [ -z "$host" ]; then
                    host="$1"
                else
                    # A second non-flag arg means "ssh host cmd …"
                    # — bail out, the user wants a one-shot exec.
                    saw_command=1
                fi
                ;;
        esac
        shift
    done
    if [ $saw_command -eq 1 ] || [ -z "$host" ]; then
        return 1
    fi
    printf '%s %s\n' "$host" "$port"
    return 0
}

__pier_ssh_upload() {
    # Try to place our rc at ~/.pier-x/integration.sh on $1 (the
    # ssh target, possibly `user@host`). Uses scp in batch mode so
    # it either succeeds via an already-available key / agent or
    # fails instantly — it never hangs asking for a password.
    local target="$1"
    local port="$2"
    # Use BatchMode to avoid password prompts; propagate the port.
    scp -q -B -P "$port" \
        -o ControlMaster=auto \
        -o ControlPath="$HOME/.ssh/pier-x-%C" \
        -o ControlPersist=60 \
        "$__pier_ssh_integration_path" \
        "$target:.pier-x/integration.sh" \
        > /dev/null 2>&1
}

ssh() {
    # Only engage the hijack when the target is parseable and the
    # integration script exists on this side — otherwise we have
    # nothing to upload. For all other shapes, pass through.
    if ! [ -f "$__pier_ssh_integration_path" ]; then
        command ssh "$@"; return $?
    fi

    local parsed
    if ! parsed=$(__pier_parse_ssh_target "$@"); then
        command ssh "$@"; return $?
    fi
    local host="${parsed% *}"
    local port="${parsed##* }"

    # Pre-create ~/.pier-x on the target so scp can drop into it.
    # BatchMode again — never block.
    command ssh -B -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
        -p "$port" "$host" 'mkdir -p ~/.pier-x' > /dev/null 2>&1 || {
        command ssh "$@"; return $?
    }

    if ! __pier_ssh_upload "$host" "$port"; then
        command ssh "$@"; return $?
    fi

    # Relaunch with rcfile — forces an interactive bash on the
    # remote regardless of the user's login shell. `-t` keeps the
    # pty allocated since we're running a non-default command.
    command ssh -t -p "$port" "$host" \
        "bash --rcfile ~/.pier-x/integration.sh -i"
}
