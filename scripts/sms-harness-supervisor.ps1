#Requires -Version 7.4
<#
    SMS operator harness supervisor.

    The scheduled task launches THIS, never buzz-acp.exe directly. Everything
    here exists because of a failure that actually happened, not for symmetry.

    WHY A SUPERVISOR AT ALL: the harness was previously started by hand from a
    terminal. It died with exit code 1 and *nothing fatal in its log* — no
    panic, no error, just absence. From the outside that is indistinguishable
    from a working system nobody happens to be texting, which is the worst
    property a message bridge can have. So this process restarts the harness
    and, just as importantly, publishes a heartbeat that makes "alive" an
    observable fact rather than an assumption.

    LOAD-BEARING FLAGS — do not "tidy" these away:
      --kinds 9         `--subscribe all` with no kinds subscribes to NOTHING
                        (kinds default to an empty vec), so the harness
                        connects, reports healthy, and receives no events.
      --respond-to anyone  the default is owner-only, which silently DROPS
                        every event when no owner is configured.
      CLAUDECODE unset  claude-code-acp refuses to start when it is inherited
                        ("cannot be launched inside another Claude Code
                        session") and fails as a -32603 that reads like an ACP
                        protocol fault rather than an environment guard.

    THE ENVIRONMENT SCRUB IS NOT PARANOIA. buzz-acp's clap parser is built with
    the `env` feature and 48 of its flags have environment twins
    (crates/buzz-acp/src/config.rs). Three of them — BUZZ_ACP_SUBSCRIBE,
    BUZZ_ACP_KINDS, BUZZ_ACP_RESPOND_TO — silently override the very arguments
    listed above. An inherited BUZZ_ACP_RESPOND_TO=owner-only would produce a
    harness that connects, subscribes, logs nothing unusual, and drops every
    message. Scrubbing the whole BUZZ_ACP_* namespace is the only way to make
    the argument list below mean what it says.

    THE KEY: BUZZ_PRIVATE_KEY is a live Nostr private key. It is stored as a
    DPAPI blob (decryptable only by this user on this machine), read here at
    runtime, and handed to the child through the process environment — never
    on a command line, which any user on the box can read via
    Win32_Process.CommandLine.
#>
[CmdletBinding()]
param(
    [string] $InstallRoot = "$env:USERPROFILE\.buzz-sms-harness",
    [string] $ConfigPath,
    # Launch exactly one child and return its exit code. For interactive
    # debugging — the scheduled task never uses this.
    [switch] $Once,
    # Mirror supervisor lines to the console as well as the log.
    [switch] $Foreground
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# --------------------------------------------------------------------------
# Layout
# --------------------------------------------------------------------------
$Layout = @{
    Root      = $InstallRoot
    Logs      = Join-Path $InstallRoot 'logs'
    KeyFile   = Join-Path $InstallRoot 'buzz-private-key.dpapi'
    PidFile   = Join-Path $InstallRoot 'harness.pid'
    Heartbeat = Join-Path $InstallRoot 'heartbeat.txt'
    StopFlag  = Join-Path $InstallRoot 'stop.flag'
    OutLog    = Join-Path $InstallRoot 'logs\harness.out.log'
    ErrLog    = Join-Path $InstallRoot 'logs\harness.err.log'
}
if (-not $ConfigPath) { $ConfigPath = Join-Path $InstallRoot 'harness.config.json' }

New-Item -ItemType Directory -Force -Path $Layout.Root, $Layout.Logs | Out-Null

# --------------------------------------------------------------------------
# Logging. Supervisor lines are timestamped; child output passes through raw
# via redirection. Any 64-hex run or nsec1 bech32 string is masked on the way
# out — cheap insurance against a future log line, a panic, or an error chain
# echoing the key.
# --------------------------------------------------------------------------
$script:SecretPattern = '(?i)\b[0-9a-f]{64}\b|nsec1[02-9ac-hj-np-z]{20,}'

function Protect-Secret {
    param([string] $Text)
    if ([string]::IsNullOrEmpty($Text)) { return $Text }
    return [regex]::Replace($Text, $script:SecretPattern, '<redacted>')
}

function Write-Log {
    param([string] $Level, [string] $Message)
    $line = '[{0}] {1} {2}' -f (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'),
                                $Level, (Protect-Secret $Message)
    Add-Content -Path $Layout.OutLog -Value $line -Encoding utf8
    if ($Foreground) { Write-Host $line }
}

# --------------------------------------------------------------------------
# Environment scrub — must run before anything reads configuration, so that a
# poisoned inherited value cannot influence a decision made below.
# --------------------------------------------------------------------------
function Clear-InheritedEnvironment {
    $removed = [System.Collections.Generic.List[string]]::new()

    foreach ($name in @('CLAUDECODE', 'BUZZ_AUTH_TAG', 'BUZZ_API_TOKEN', 'BUZZ_PRIVATE_KEY')) {
        if (Test-Path "Env:$name") { Remove-Item "Env:$name" -Force; $removed.Add($name) }
    }
    # The whole BUZZ_ACP_* namespace: 48 flags have env twins and three of
    # them silently invert the harness's core behaviour.
    foreach ($item in (Get-ChildItem Env: | Where-Object Name -like 'BUZZ_ACP_*')) {
        Remove-Item "Env:$($item.Name)" -Force
        $removed.Add($item.Name)
    }

    # Names only. A value here could be the key itself.
    if ($removed.Count -gt 0) { Write-Log 'INFO' "scrubbed inherited env: $($removed -join ', ')" }
}

# --------------------------------------------------------------------------
# Log rotation, performed only at a launch boundary when no child holds the
# handle. Rotating under a live child would silently truncate to a file the
# child still has open at its old offset.
# --------------------------------------------------------------------------
function Invoke-LogRotation {
    param([int] $MaxBytes = 10MB, [int] $Generations = 4)
    foreach ($path in @($Layout.OutLog, $Layout.ErrLog)) {
        if (-not (Test-Path $path)) { continue }
        if ((Get-Item $path).Length -lt $MaxBytes) { continue }
        Remove-Item "$path.$Generations" -Force -ErrorAction SilentlyContinue
        for ($i = $Generations - 1; $i -ge 1; $i--) {
            if (Test-Path "$path.$i") { Move-Item "$path.$i" "$path.$($i + 1)" -Force }
        }
        Move-Item $path "$path.1" -Force
    }
}

# --------------------------------------------------------------------------
# Singleton. Local\ rather than Global\ deliberately: Global needs
# SeCreateGlobalPrivilege, which a LeastPrivilege scheduled task does not have,
# so a Global mutex would throw rather than guard.
#
# A mutex is used instead of a pid-file lock because the OS releases it when
# the process dies, however it dies. A stale lock file that permanently
# prevents restart is a worse failure than the duplicate it prevents.
# --------------------------------------------------------------------------
$script:Mutex = $null
function Enter-Singleton {
    $created = $false
    $script:Mutex = [System.Threading.Mutex]::new($true, 'Local\BuzzSmsHarnessSupervisor', [ref] $created)
    if (-not $created) {
        # Another supervisor holds it. Not an error — the task firing twice is
        # normal (logon + a manual start), and exiting 0 keeps Task Scheduler
        # from recording a spurious failure.
        Write-Log 'INFO' 'another supervisor instance is already running — exiting'
        return $false
    }
    return $true
}

# --------------------------------------------------------------------------
# Reap an orphan from a previous supervisor that died without cleaning up.
# The Path check is mandatory: PIDs are reused across reboots, and this must
# never kill a developer's own buzz-acp running from a different checkout.
# --------------------------------------------------------------------------
function Remove-OrphanedChild {
    param([string] $BuzzAcpExe)
    if (-not (Test-Path $Layout.PidFile)) { return }
    $recorded = (Get-Content $Layout.PidFile -Raw).Trim()
    Remove-Item $Layout.PidFile -Force -ErrorAction SilentlyContinue
    if ($recorded -notmatch '^\d+$') { return }

    $proc = Get-Process -Id ([int] $recorded) -ErrorAction SilentlyContinue
    if (-not $proc) { return }
    $path = try { $proc.Path } catch { $null }
    if ($path -and ($path -eq $BuzzAcpExe)) {
        Write-Log 'WARN' "reaping orphaned harness pid $recorded from a previous supervisor"
        & taskkill.exe /PID $recorded /T /F 2>&1 | Out-Null
    }
}

function Write-Heartbeat {
    param([int] $ChildPid, [int] $Restarts)
    $payload = [ordered]@{
        utc           = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
        supervisorPid = $PID
        childPid      = $ChildPid
        restarts      = $Restarts
    } | ConvertTo-Json -Compress
    Set-Content -Path $Layout.Heartbeat -Value $payload -Encoding utf8
}

# --------------------------------------------------------------------------
# Main
# --------------------------------------------------------------------------
Clear-InheritedEnvironment

if (-not (Enter-Singleton)) { exit 0 }

try {
    if (Test-Path $Layout.StopFlag) {
        Write-Log 'INFO' 'stop.flag present — not starting'
        exit 0
    }

    # Config: committed defaults, optionally overlaid by a non-secret json file.
    $cfg = @{
        buzzAcpExe   = 'E:\Projects\buzz\.claude\worktrees\external-integration-design\target\debug\buzz-acp.exe'
        workingDir   = 'E:\Projects\buzz\.claude\worktrees\external-integration-design'
        pathPrepend  = 'E:\Projects\buzz\.claude\worktrees\external-integration-design\target\release'
        relayUrl     = 'wss://buzz-relay-sms-production.up.railway.app'
        packPath     = 'crates/buzz-persona/packs/sms-operator'
        agentCommand = 'C:\Users\mfeth\AppData\Roaming\npm\claude-code-acp.cmd'
        channels     = '707fefeb-53ec-4e1e-8195-ace864cdbafe'
        projectPaths = 'bidcraft=E:/Projects/buildbid/bidcraft-repo'
    }
    if (Test-Path $ConfigPath) {
        $overlay = Get-Content $ConfigPath -Raw | ConvertFrom-Json
        foreach ($k in $cfg.Keys.Clone()) {
            if ($overlay.PSObject.Properties.Name -contains $k) { $cfg[$k] = $overlay.$k }
        }
        Write-Log 'INFO' "config overlay applied from $ConfigPath"
    }

    # Refuse to start rather than spin: a missing binary is an operator error
    # that a restart loop would only obscure behind a wall of identical lines.
    foreach ($p in @($cfg.buzzAcpExe, $cfg.workingDir, $cfg.agentCommand)) {
        if (-not (Test-Path $p)) {
            Write-Log 'FATAL' "required path does not exist: $p"
            exit 1
        }
    }
    Write-Log 'INFO' "harness binary: $($cfg.buzzAcpExe)"

    if (-not (Test-Path $Layout.KeyFile)) {
        Write-Log 'FATAL' "key store missing at $($Layout.KeyFile) — run sms-harness-task.ps1 -ProvisionSecret"
        exit 1
    }
    try {
        $secure = Get-Content $Layout.KeyFile -Raw | ConvertTo-SecureString
        $plainKey = [System.Net.NetworkCredential]::new('', $secure).Password
    } catch {
        # DPAPI blobs are user- and machine-bound; this is what a copied
        # install root or a different user account looks like.
        Write-Log 'FATAL' 'key store could not be decrypted by this user on this machine'
        exit 1
    }
    if ([string]::IsNullOrWhiteSpace($plainKey)) {
        Write-Log 'FATAL' 'key store decrypted to an empty value — refusing to start'
        exit 1
    }

    Remove-OrphanedChild -BuzzAcpExe $cfg.buzzAcpExe

    $argList = @(
        '--pack', $cfg.packPath,
        '--agent-command', $cfg.agentCommand,
        '--channels', $cfg.channels,
        '--subscribe', 'all',
        '--kinds', '9',              # load-bearing — see header
        '--respond-to', 'anyone',    # load-bearing — see header
        '--project-paths', $cfg.projectPaths
    )

    # $null UNSETS an inherited variable rather than setting it empty — the
    # native equivalent of `env -u`. Verified present: Start-Process
    # -Environment requires PowerShell 7.4+, which the #Requires enforces.
    $childEnv = @{
        BUZZ_PRIVATE_KEY = $plainKey
        BUZZ_RELAY_URL   = $cfg.relayUrl
        PATH             = "$($cfg.pathPrepend);$env:PATH"
        CLAUDECODE       = $null
    }

    $restarts = 0
    $consecutiveFailures = 0

    while ($true) {
        if (Test-Path $Layout.StopFlag) {
            Write-Log 'INFO' 'stop.flag appeared — shutting down'
            break
        }

        Invoke-LogRotation

        $startedAt = Get-Date
        $proc = Start-Process -FilePath $cfg.buzzAcpExe -ArgumentList $argList `
            -WorkingDirectory $cfg.workingDir -Environment $childEnv `
            -RedirectStandardOutput $Layout.OutLog -RedirectStandardError $Layout.ErrLog `
            -NoNewWindow -PassThru
        Set-Content -Path $Layout.PidFile -Value $proc.Id -Encoding ascii
        Write-Log 'INFO' "harness started (pid $($proc.Id), restart #$restarts)"
        # Publish immediately, not just on the 60s tick. Without this the
        # heartbeat still advertises the PREVIOUS child's pid and restart count
        # for up to a minute after a restart — i.e. it is least trustworthy in
        # exactly the window where someone is asking "did it come back?".
        # Caught by killing the child and watching the heartbeat not move.
        Write-Heartbeat -ChildPid $proc.Id -Restarts $restarts

        # Poll rather than block, so the heartbeat keeps publishing and the
        # stop flag stays responsive while the child runs.
        $tick = 0
        while (-not $proc.WaitForExit(1000)) {
            $tick++
            if ($tick % 60 -eq 0) { Write-Heartbeat -ChildPid $proc.Id -Restarts $restarts }
            if (Test-Path $Layout.StopFlag) {
                Write-Log 'INFO' 'stop.flag appeared — stopping harness'
                & taskkill.exe /PID $proc.Id /T /F 2>&1 | Out-Null
                break
            }
        }

        $lifetime = ((Get-Date) - $startedAt).TotalSeconds
        $code = try { $proc.ExitCode } catch { 'unknown' }
        Remove-Item $Layout.PidFile -Force -ErrorAction SilentlyContinue

        if (Test-Path $Layout.StopFlag) { break }
        if ($Once) {
            Write-Log 'INFO' "-Once: harness exited ($code) after $([int]$lifetime)s"
            exit ($(if ($code -is [int]) { $code } else { 1 }))
        }

        $restarts++
        # A long-lived child that dies is a transient; a child that dies
        # immediately is a misconfiguration, and hammering it would produce
        # gigabytes of identical logs rather than a legible signal.
        if ($lifetime -ge 60) {
            $consecutiveFailures = 0
            $delay = 5
        } else {
            $consecutiveFailures++
            $delay = [Math]::Min(5 * [Math]::Pow(2, $consecutiveFailures), 300)
        }
        Write-Log 'WARN' ("harness exited code=$code after $([int]$lifetime)s; " +
                          "restarting in $([int]$delay)s (consecutive short-lived: $consecutiveFailures)")
        Start-Sleep -Seconds $delay
    }
} finally {
    # Never leak an orphan on a supervisor shutdown.
    if (Test-Path $Layout.PidFile) {
        $last = (Get-Content $Layout.PidFile -Raw).Trim()
        if ($last -match '^\d+$') { & taskkill.exe /PID $last /T /F 2>&1 | Out-Null }
        Remove-Item $Layout.PidFile -Force -ErrorAction SilentlyContinue
    }
    if ($script:Mutex) {
        try { $script:Mutex.ReleaseMutex() } catch { }
        $script:Mutex.Dispose()
    }
    Write-Log 'INFO' 'supervisor stopped'
}
