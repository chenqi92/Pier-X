# Pier-X shell integration — PowerShell v1
#
# Loaded by `pwsh -NoLogo -NoExit -Command ". <this path>"` when a
# Pier-X SSH tab opens against a Windows host, and by the user's
# PowerShell profile on local Windows when they opt in through
# Settings → Shell integration.
#
# Jobs (mirror of the bash integration):
#
#   1. Emit OSC 7 (`file://HOST/PATH`) before every prompt so the
#      Pier-X emulator can follow `cd` / `Set-Location` in real
#      time. Windows paths are lowercased + backslashes flipped to
#      forward slashes so the URI parses correctly.
#
#   2. Emit OSC 133 `A` / `D;<exit>` so the UI can tell prompt-
#      paints from command-finishes and auto-refresh right after a
#      command returns.
#
#   3. Hijack `ssh` so manual `ssh user@host` in a PowerShell tab
#      still flows through our integration on the next hop. Uses
#      scp best-effort to upload the bash rc; falls back to plain
#      `ssh.exe` on any failure so the user's command still works.

# ── 1. Chain-load the user's own profile ──────────────────────
# Run their profile first so our hooks wrap (rather than stomp)
# any prompt / aliases / module imports they already have.
if ($PROFILE -and (Test-Path $PROFILE -ErrorAction SilentlyContinue)) {
    . $PROFILE
}

# ── 2. OSC 7 cwd + OSC 133 prompt boundaries ─────────────────
$Global:__PierPrevPrompt = $Function:prompt

function Global:__Pier-Write-Osc7 {
    $Script:__pier_host = [System.Net.Dns]::GetHostName()
    # Build a file:// path. Windows paths use `\` and sometimes
    # `C:\…`; flip slashes and ensure a leading `/` so the URI is
    # well-formed for terminal OSC 7 consumers.
    $Script:__pier_path = ($PWD.ProviderPath -replace '\\', '/')
    if (-not $Script:__pier_path.StartsWith('/')) {
        $Script:__pier_path = '/' + $Script:__pier_path
    }
    [System.Console]::Write(
        "`e]7;file://$Script:__pier_host$Script:__pier_path`e\"
    )
}

function Global:prompt {
    # Capture the previous command's exit info BEFORE we clobber it
    # with our own helper calls. `$?` is a bool; `$LASTEXITCODE`
    # is the numeric exit of the last external process. Fall back
    # to 0 / 1 when neither is populated (e.g. first prompt).
    $__pier_lastok = $?
    $__pier_code = if ($null -ne $global:LASTEXITCODE) {
        $global:LASTEXITCODE
    } elseif ($__pier_lastok) {
        0
    } else {
        1
    }
    [System.Console]::Write("`e]133;D;$__pier_code`e\")

    __Pier-Write-Osc7
    [System.Console]::Write("`e]133;A`e\")

    # Delegate to whatever prompt the user's profile defined; if
    # nothing did, fall back to a sensible default.
    if ($Global:__PierPrevPrompt) {
        & $Global:__PierPrevPrompt
    } else {
        "PS $($PWD.ProviderPath)> "
    }
}

# Emit once on startup so the panel catches the initial cwd
# without waiting for the first prompt repaint.
__Pier-Write-Osc7
[System.Console]::Write("`e]133;A`e\")

# ── 3. Nested ssh hijacker ────────────────────────────────────
# Parse the target, detect whether it's a POSIX host (bash) or a
# Windows OpenSSH host (pwsh), upload the matching rc, and launch
# the remote interactively with that rc sourced. Any failure on
# any step silently falls back to plain `ssh.exe` so the user's
# command line always works.
$Global:__PierBashRc = Join-Path $HOME '.pier-x' 'integration.sh'
$Global:__PierPwshRc = Join-Path $HOME '.pier-x' 'integration.ps1'

function Global:__Pier-Parse-SshTarget {
    param([Parameter(ValueFromRemainingArguments = $true)] $ArgList)
    $port = 22
    $host_ = ''
    $sawCommand = $false
    $i = 0
    while ($i -lt $ArgList.Count) {
        $a = $ArgList[$i]
        if ($a -eq '-p') {
            $i++
            if ($i -ge $ArgList.Count) { return $null }
            $port = [int]$ArgList[$i]
        } elseif ('-l', '-i', '-F', '-o', '-J', '-L', '-R', '-D',
                   '-W', '-B', '-b', '-c', '-E', '-e', '-I', '-m',
                   '-O', '-Q', '-S', '-w' -contains $a) {
            $i++ # skip the flag value
        } elseif ($a.StartsWith('-')) {
            # boolean flag — leave alone
        } else {
            if ($host_ -eq '') {
                $host_ = $a
            } else {
                $sawCommand = $true
            }
        }
        $i++
    }
    if ($sawCommand -or $host_ -eq '') { return $null }
    return [PSCustomObject]@{ Host = $host_; Port = $port }
}

function Global:__Pier-Detect-RemoteShell {
    param($Parsed)
    # Try bash first (most common). BatchMode + accept-new means
    # the probe never blocks on password or host-key prompts.
    & ssh.exe -B -o BatchMode=yes -o StrictHostKeyChecking=accept-new `
        -p $Parsed.Port $Parsed.Host 'bash --version' 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) { return 'bash' }
    & ssh.exe -B -o BatchMode=yes -o StrictHostKeyChecking=accept-new `
        -p $Parsed.Port $Parsed.Host 'pwsh -Version' 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) { return 'pwsh' }
    return $null
}

function Global:ssh {
    $parsed = __Pier-Parse-SshTarget @Args
    if ($null -eq $parsed) { & ssh.exe @Args; return }

    $kind = __Pier-Detect-RemoteShell $parsed
    if ($null -eq $kind) { & ssh.exe @Args; return }

    # Pick local rc + remote name + launch command for the
    # detected target flavour.
    if ($kind -eq 'bash') {
        $localRc      = $Global:__PierBashRc
        $remoteName   = '.pier-x/integration.sh'
        $mkdirCmd     = 'mkdir -p ~/.pier-x'
        $launchCmd    = 'bash --rcfile ~/.pier-x/integration.sh -i'
    } else {
        $localRc      = $Global:__PierPwshRc
        $remoteName   = '.pier-x/integration.ps1'
        # PowerShell `mkdir` alias works but we spell it out for
        # clarity on hosts where the alias was removed.
        $mkdirCmd     = 'New-Item -ItemType Directory -Force -Path "$HOME/.pier-x" | Out-Null'
        $launchCmd    = 'pwsh -NoLogo -NoExit -Command ". $HOME/.pier-x/integration.ps1"'
    }

    if (-not (Test-Path $localRc)) { & ssh.exe @Args; return }

    & ssh.exe -B -o BatchMode=yes -p $parsed.Port $parsed.Host $mkdirCmd 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) { & ssh.exe @Args; return }

    & scp.exe -q -B -P $parsed.Port `
        $localRc `
        "$($parsed.Host):$remoteName" 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) { & ssh.exe @Args; return }

    & ssh.exe -t -p $parsed.Port $parsed.Host $launchCmd
}
