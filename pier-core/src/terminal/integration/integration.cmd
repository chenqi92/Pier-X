@echo off
REM Pier-X shell integration — cmd.exe fallback
REM
REM Used when a Pier-X SSH tab opens against a Windows OpenSSH
REM host that only exposes cmd.exe — no bash, no PowerShell.
REM Launched via `cmd.exe /K "call %USERPROFILE%\.pier-x\integration.cmd"`.
REM
REM cmd has no prompt hook akin to bash's `PROMPT_COMMAND` or
REM PowerShell's `prompt` function, so this integration is
REM deliberately minimal: it only emits OSC 7 (current working
REM directory) every time the prompt repaints. OSC 133 (command
REM boundary) isn't expressible in cmd — downstream panels that
REM rely on `last_command_exit` simply don't fire, which is fine;
REM they gracefully fall back to polling.
REM
REM cmd's PROMPT variable supports these escapes in the templated
REM string — re-evaluated per prompt paint:
REM   $E = ESC (0x1b)
REM   $P = current drive + path  (e.g. C:\Users\me)
REM   $G = >
REM   $S = space
REM
REM We build: `ESC ] 7 ; file:// %COMPUTERNAME% $P ESC \ $P > `
REM so each prompt paint emits the OSC 7 control string (invisible
REM to the eye; consumed by the Pier-X emulator) followed by the
REM usual `C:\path>` prompt text.

SET "PROMPT=$E]7;file://%COMPUTERNAME%$P$E\$P$G$S"

REM The next prompt paint emits OSC 7 automatically, so the UI
REM catches the initial cwd without any further action.
