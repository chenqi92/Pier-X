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
# detect whether the target is a POSIX host (bash) or a Windows
# OpenSSH host (pwsh), upload the matching rc, and launch the
# remote interactively with that rc sourced. Any step failing
# falls through to `command ssh "$@"` so the user's command line
# still works exactly as typed.
#
# Flag parser is deliberately minimal: recognise `-p PORT`, pass
# everything else through. A remote command (non-flag arg after
# the host) disables hijacking — the user wants `ssh host 'cmd'`
# one-liner semantics, not an interactive shell.

__pier_bash_rc="$HOME/.pier-x/integration.sh"
__pier_pwsh_rc="$HOME/.pier-x/integration.ps1"

__pier_parse_ssh_target() {
    local port=22
    local host=""
    local saw_command=0
    while [ $# -gt 0 ]; do
        case "$1" in
            -p)
                shift; [ $# -gt 0 ] || return 1; port="$1"
                ;;
            -l|-i|-F|-o|-J|-L|-R|-D|-W|-B|-b|-c|-E|-e|-I|-m|-O|-Q|-S|-w)
                # Flags that take an argument — skip both.
                shift; [ $# -gt 0 ] || return 1
                ;;
            -*)
                # Boolean flag (e.g. -v, -4, -A). Leave it alone.
                ;;
            *)
                if [ -z "$host" ]; then
                    host="$1"
                else
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

__pier_detect_remote_shell() {
    local host="$1"
    local port="$2"
    if command ssh -B -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
        -p "$port" "$host" 'bash --version' > /dev/null 2>&1; then
        printf 'bash'
        return 0
    fi
    if command ssh -B -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
        -p "$port" "$host" 'pwsh -Version' > /dev/null 2>&1; then
        printf 'pwsh'
        return 0
    fi
    return 1
}

ssh() {
    local parsed
    if ! parsed=$(__pier_parse_ssh_target "$@"); then
        command ssh "$@"; return $?
    fi
    local host="${parsed% *}"
    local port="${parsed##* }"

    local kind
    if ! kind=$(__pier_detect_remote_shell "$host" "$port"); then
        command ssh "$@"; return $?
    fi

    # Choose local rc, remote name, mkdir cmd, and launch command
    # per detected target flavour.
    local local_rc remote_name mkdir_cmd launch_cmd
    if [ "$kind" = "bash" ]; then
        local_rc="$__pier_bash_rc"
        remote_name=".pier-x/integration.sh"
        mkdir_cmd='mkdir -p ~/.pier-x'
        launch_cmd='bash --rcfile ~/.pier-x/integration.sh -i'
    else
        local_rc="$__pier_pwsh_rc"
        remote_name=".pier-x/integration.ps1"
        mkdir_cmd='New-Item -ItemType Directory -Force -Path "$HOME/.pier-x" | Out-Null'
        launch_cmd='pwsh -NoLogo -NoExit -Command ". $HOME/.pier-x/integration.ps1"'
    fi

    if ! [ -f "$local_rc" ]; then
        command ssh "$@"; return $?
    fi

    command ssh -B -o BatchMode=yes \
        -p "$port" "$host" "$mkdir_cmd" > /dev/null 2>&1 || {
        command ssh "$@"; return $?
    }

    scp -q -B -P "$port" \
        -o ControlMaster=auto \
        -o ControlPath="$HOME/.ssh/pier-x-%C" \
        -o ControlPersist=60 \
        "$local_rc" \
        "$host:$remote_name" > /dev/null 2>&1 || {
        command ssh "$@"; return $?
    }

    command ssh -t -p "$port" "$host" "$launch_cmd"
}
