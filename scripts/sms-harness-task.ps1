#Requires -Version 7
# Lifecycle CLI for the Windows Scheduled Task that durably hosts the SMS
# operator harness (buzz-acp with the sms-operator persona pack).
#
# WHY this exists at all: the harness used to run as an ad-hoc background
# process started from a Claude Code session, so it died with that session.
# It must run on THIS machine — it dispatches agents against local repo
# checkouts (--project-paths) and drives a locally-installed, locally
# authenticated Claude Code CLI — so Docker/Railway/any cloud host are out by
# construction. A Windows Scheduled Task is the durable host; nssm is not
# installed and must not become a dependency.
#
# WHY this script never launches buzz-acp.exe itself: the task points at
# scripts/sms-harness-supervisor.ps1, which owns the restart loop, the env
# scrub and the DPAPI secret read. This script only manages the task
# definition and the operator verbs around it.
#
# WHY the task is created from XML and never from `schtasks /Create` flags:
# the complete /Create flag list contains no way to set ExecutionTimeLimit, so
# a flag-created task silently gets PT72H and Task Scheduler kills the harness
# after three days. Verified on this machine: the pre-existing flag-created
# `agent-coord-watchdog` task reads ExecutionTimeLimit = PT72H, while the XML
# path below reads back PT0S. `/SC ONLOGON` additionally returned "Access is
# denied" non-elevated in all three forms tried, while `/Create /XML` succeeds
# non-elevated from the same shell.
#
# This is the first PowerShell script in scripts/. It transliterates the repo's
# bash conventions rather than inventing new ones: a WHY header block,
# Set-StrictMode + $ErrorActionPreference='Stop' as the analogue of
# `set -euo pipefail`, a usage guard exiting 2, diagnostics to stderr and
# machine-readable output to stdout, and -DryRun for --dry-run.
#
# Usage: pwsh -NoProfile -File ./scripts/sms-harness-task.ps1 -Status
#        just sms-harness -Status

[CmdletBinding()]
param(
    # ── Verbs (mutually exclusive; exactly one required) ────────────────────
    [switch]$ProvisionSecret,
    [switch]$Install,
    [switch]$Start,
    [switch]$Stop,
    [switch]$Status,
    [switch]$Verify,
    [switch]$Disable,
    [switch]$Enable,
    [switch]$Uninstall,

    # ── Modifiers ───────────────────────────────────────────────────────────
    [string]$TaskName = 'BuzzSmsHarness',
    [string]$InstallRoot = (Join-Path $env:USERPROFILE '.buzz-sms-harness'),
    [string]$SourceRoot,
    [switch]$DryRun,
    [switch]$PurgeSecrets,
    [switch]$RegisterEventSource
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ─────────────────────────────────────────────────────────────────────────────
# Constants
# ─────────────────────────────────────────────────────────────────────────────

# The worktree the harness was verified working from. Baked in as a DEFAULT
# only — the supervisor reads these from harness.config.json so promoting the
# binaries out of the worktree is a config edit, not a code change.
$script:DefaultWorktree = 'E:\Projects\buzz\.claude\worktrees\external-integration-design'

$script:SupervisorFileName = 'sms-harness-supervisor.ps1'
$script:PurgeConfirmation = 'PURGE BUZZ SMS SECRETS'
$script:EventSourceName = 'BuzzSmsHarness'

# 0x800710E0 = ERROR_ALREADY_RUNNING / "the operator or administrator has
# refused the request". This is the HEALTHY steady state for this task: the
# 5-minute watchdog TimeTrigger fires, MultipleInstancesPolicy=IgnoreNew
# refuses it because the supervisor is already running, and Task Scheduler
# records the refusal as LastTaskResult. Anyone who does not know this files a
# phantom bug or, far worse, "fixes" it by relaxing MultipleInstancesPolicy —
# which would let two supervisors run and double every agent turn.
$script:HealthyRefusalCode = 2147946720

$script:ExitOk = 0
$script:ExitFail = 1
$script:ExitUsage = 2

# ─────────────────────────────────────────────────────────────────────────────
# Output helpers — diagnostics to stderr, machine-readable results to stdout
# ─────────────────────────────────────────────────────────────────────────────

function Write-Diag {
    param([Parameter(Mandatory)][string]$Message, [string]$Level = 'INFO')
    [Console]::Error.WriteLine("$Level $Message")
}

function Write-Fail {
    param([Parameter(Mandatory)][string]$Message)
    Write-Diag -Level 'ERROR' -Message $Message
}

function Write-Warn {
    param([Parameter(Mandatory)][string]$Message)
    Write-Diag -Level 'WARN' -Message $Message
}

# Status output must survive redirection to a file, so severity is carried by
# literal text markers ([OK]/[RED]) and colour is only ever additive.
#
# WHY [Console]::Out and not Write-Output/Write-Host: the verb functions signal
# their exit code through their RETURN VALUE, and Write-Output would merge every
# printed line into that return value — `$exit = Invoke-Status` would become an
# array of status text and the real exit code would be lost. (This is not
# hypothetical: the first version of this script used Write-Output and -Status
# printed nothing at all while still exiting 0.) Writing straight to the console
# handle keeps stdout genuinely stdout and keeps return values clean. Write-Diag
# does the same on stderr.
function Write-Line {
    param([string]$Message = '', [ValidateSet('none', 'ok', 'red', 'yellow')][string]$Colour = 'none')
    if ($Colour -eq 'none' -or [Console]::IsOutputRedirected) {
        [Console]::Out.WriteLine($Message)
        return
    }
    $code = switch ($Colour) { 'ok' { '32' } 'red' { '31' } 'yellow' { '33' } }
    [Console]::Out.WriteLine("$([char]27)[${code}m$Message$([char]27)[0m")
}

function Show-Usage {
    $usage = @'
Manage the durable Windows Scheduled Task hosting the SMS operator harness.

Usage:
  sms-harness-task.ps1 <verb> [modifiers]

Verbs (exactly one required):
  -ProvisionSecret   One-time interactive DPAPI provisioning of BUZZ_PRIVATE_KEY.
  -Install           Deploy the supervisor and register the task (does not start it).
  -Start             Clear stop.flag, run the task, then prove liveness via heartbeat.
  -Stop              stop.flag -> schtasks /End -> taskkill -> orphan sweep -> assert.
  -Status            Task view + real liveness view (heartbeat, processes, logs).
  -Verify            Assert every registered-task and supervisor-argument invariant.
  -Disable           Keep the definition, stop it running on triggers.
  -Enable            Re-enable a disabled task.
  -Uninstall         Stop, delete the task, remove the deployed supervisor.

Modifiers:
  -TaskName <string>       Default: BuzzSmsHarness
  -InstallRoot <string>    Default: %USERPROFILE%\.buzz-sms-harness
  -SourceRoot <string>     Default: the repo root containing scripts/
  -DryRun                  Print the schtasks/icacls/taskkill commands, change nothing.
  -PurgeSecrets            Only with -Uninstall. Deletes the DPAPI key store.
  -RegisterEventSource     Only with -Install. Optional, needs one elevated call.

Exit codes: 0 ok, 1 operational failure, 2 usage error.
'@
    [Console]::Error.WriteLine($usage)
}

# ─────────────────────────────────────────────────────────────────────────────
# Path / process helpers
# ─────────────────────────────────────────────────────────────────────────────

function Get-NormalizedPath {
    param([string]$Path)
    if ([string]::IsNullOrWhiteSpace($Path)) { return $null }
    try { return [System.IO.Path]::GetFullPath($Path) } catch { return $Path }
}

function Test-SamePath {
    param([string]$A, [string]$B)
    $na = Get-NormalizedPath $A
    $nb = Get-NormalizedPath $B
    if (-not $na -or -not $nb) { return $false }
    return $na.TrimEnd('\') -ieq $nb.TrimEnd('\')
}

# Reading .Path on a process owned by another user (or one that exited between
# enumeration and access) throws. Under $ErrorActionPreference='Stop' that would
# abort the whole verb, so every read goes through here.
function Get-ProcessPathSafe {
    param([Parameter(Mandatory)]$Process)
    try { return $Process.Path } catch { return $null }
}

function Get-ProcessStartTimeSafe {
    param([Parameter(Mandatory)]$Process)
    try { return $Process.StartTime } catch { return $null }
}

# Every buzz-acp on the box, not just ours. -Status shows all of them because a
# SECOND instance running with the same key is invisible from the relay side
# (nothing arbitrates two connections with the same pubkey) and produces
# duplicate agent turns against the same working copy.
function Get-AllBuzzAcpProcesses {
    return @(Get-Process -Name 'buzz-acp' -ErrorAction SilentlyContinue)
}

# Path-matched subset — the ONLY set this script is ever allowed to kill.
# Without the .Path match the sweep would murder a developer's `just goose`
# harness running from another worktree, and a stale harness.pid reused after a
# reboot would kill an unrelated process.
function Get-OurBuzzAcpProcesses {
    param([Parameter(Mandatory)][string]$BuzzAcpExe)
    return @(Get-AllBuzzAcpProcesses | Where-Object { Test-SamePath (Get-ProcessPathSafe $_) $BuzzAcpExe })
}

# ─────────────────────────────────────────────────────────────────────────────
# Native command execution (honours -DryRun)
# ─────────────────────────────────────────────────────────────────────────────

function Format-CommandLine {
    param([string]$FilePath, [string[]]$Arguments)
    $quoted = foreach ($a in $Arguments) {
        if ($a -match '[\s"]') { '"' + ($a -replace '"', '\"') + '"' } else { $a }
    }
    return (@($FilePath) + @($quoted)) -join ' '
}

# Returns the native exit code. Callers must decide what a non-zero means —
# and note that for schtasks /Run and /End a ZERO means nothing either (see
# Assert-HeartbeatFresh and the -Stop sweep).
function Invoke-Native {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [Parameter(Mandatory)][string[]]$Arguments,
        [switch]$Quiet
    )
    $display = Format-CommandLine -FilePath $FilePath -Arguments $Arguments
    if ($DryRun) {
        Write-Line "DRYRUN would run: $display"
        return 0
    }
    Write-Diag "run: $display"
    $output = & $FilePath @Arguments 2>&1
    $code = $LASTEXITCODE
    if (-not $Quiet) {
        foreach ($line in @($output)) {
            if ("$line".Trim()) { Write-Diag "     $line" }
        }
    }
    return $code
}

# ─────────────────────────────────────────────────────────────────────────────
# InstallRoot layout
# ─────────────────────────────────────────────────────────────────────────────

function Assert-InstallRootOutsideRepo {
    # -InstallRoot is free-form operator input, and everything secret-bearing
    # lands under it. Pointed inside a git working tree, `git add -A` would
    # commit the DPAPI key blob — unrecoverable in an OSS repo. .gitignore
    # carries matching patterns as a second layer, but a path check is the one
    # that cannot be defeated by an operator renaming the file.
    param([Parameter(Mandatory)][string]$Root)
    $probe = if (Test-Path -LiteralPath $Root) { $Root } else { Split-Path -Parent $Root }
    if (-not $probe -or -not (Test-Path -LiteralPath $probe)) { return }
    $inside = & git -C $probe rev-parse --show-toplevel 2>$null
    if ($LASTEXITCODE -eq 0 -and $inside) {
        throw "InstallRoot '$Root' is inside the git working tree at '$inside'. " +
              "It holds the DPAPI key blob and must live outside any repo — " +
              "use a path such as `$env:USERPROFILE\.buzz-sms-harness."
    }
}

function Get-Layout {
    param([Parameter(Mandatory)][string]$Root)
    Assert-InstallRootOutsideRepo -Root $Root
    return [pscustomobject]@{
        Root       = $Root
        Logs       = Join-Path $Root 'logs'
        Supervisor = Join-Path $Root 'supervisor.ps1'
        Config     = Join-Path $Root 'harness.config.json'
        Manifest   = Join-Path $Root 'installed.manifest.json'
        KeyFile    = Join-Path $Root 'buzz-private-key.dpapi'
        StopFlag   = Join-Path $Root 'stop.flag'
        Pid        = Join-Path $Root 'harness.pid'
        Heartbeat  = Join-Path $Root 'heartbeat.txt'
        OutLog     = Join-Path $Root 'logs\harness.out.log'
        ErrLog     = Join-Path $Root 'logs\harness.err.log'
        TaskXml    = Join-Path $Root 'task.generated.xml'
    }
}

function Get-DefaultConfig {
    # These mirror the verified-working invocation. They are duplicated in the
    # supervisor by design (it must run standalone); harness.config.json is the
    # single place an operator overrides either copy.
    return [ordered]@{
        buzzAcpExe       = Join-Path $script:DefaultWorktree 'target\debug\buzz-acp.exe'
        workingDirectory = $script:DefaultWorktree
        pathPrepend      = Join-Path $script:DefaultWorktree 'target\release'
        relayUrl         = 'wss://buzz-relay-sms-production.up.railway.app'
        channels         = '707fefeb-53ec-4e1e-8195-ace864cdbafe'
        packPath         = 'crates/buzz-persona/packs/sms-operator'
        agentCommand     = 'C:\Users\mfeth\AppData\Roaming\npm\claude-code-acp.cmd'
        projectPaths     = 'bidcraft=E:/Projects/buildbid/bidcraft-repo'
        relayObserver    = $false
        maxLogBytes      = 10485760
    }
}

function Get-ConfigValue {
    param([Parameter(Mandatory)]$Config, [Parameter(Mandatory)][string]$Name, $Default = $null)
    # StrictMode makes a missing property a terminating error, so probe first.
    if ($Config -is [System.Collections.IDictionary]) {
        if ($Config.Contains($Name)) { return $Config[$Name] }
        return $Default
    }
    $prop = $Config.PSObject.Properties[$Name]
    if ($prop) { return $prop.Value }
    return $Default
}

# Effective config = built-in defaults overlaid by harness.config.json. Used to
# learn which buzz-acp.exe is ours, which is what makes every kill Path-guarded.
function Get-EffectiveConfig {
    param([Parameter(Mandatory)]$Layout)
    $defaults = Get-DefaultConfig
    $effective = [ordered]@{}
    foreach ($k in $defaults.Keys) { $effective[$k] = $defaults[$k] }

    if (Test-Path -LiteralPath $Layout.Config) {
        try {
            $onDisk = Get-Content -LiteralPath $Layout.Config -Raw | ConvertFrom-Json
            foreach ($k in @($effective.Keys)) {
                $v = Get-ConfigValue -Config $onDisk -Name $k -Default $null
                if ($null -ne $v -and "$v" -ne '') { $effective[$k] = $v }
            }
        } catch {
            Write-Warn "harness.config.json is unreadable or malformed; using built-in defaults ($($_.Exception.Message))"
        }
    }
    return $effective
}

# ─────────────────────────────────────────────────────────────────────────────
# pwsh resolution
# ─────────────────────────────────────────────────────────────────────────────

function Resolve-PwshPath {
    # WHY not just (Get-Command pwsh).Source: on a Store/MSIX install that
    # resolves to the VERSION-PINNED
    # C:\Program Files\WindowsApps\Microsoft.PowerShell_7.6.4.0_x64__...\pwsh.exe,
    # which stops existing the next time PowerShell updates — silently breaking
    # a durable task. Prefer the stable, version-independent launchers.
    # Verified on this machine: a task whose Command is the WindowsApps
    # execution alias registers and RUNS correctly, non-elevated.
    $candidates = @(
        (Join-Path $env:ProgramFiles 'PowerShell\7\pwsh.exe'),
        (Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\pwsh.exe')
    )
    foreach ($c in $candidates) {
        if ($c -and (Test-Path -LiteralPath $c)) { return $c }
    }
    $cmd = Get-Command pwsh -ErrorAction SilentlyContinue
    if ($cmd -and $cmd.Source) {
        if ($cmd.Source -match 'WindowsApps\\Microsoft\.PowerShell_\d') {
            Write-Warn "only a version-pinned pwsh.exe was found ($($cmd.Source)); the task will break on the next PowerShell update"
        }
        return $cmd.Source
    }
    return $null
}

# ─────────────────────────────────────────────────────────────────────────────
# Task XML generation
# ─────────────────────────────────────────────────────────────────────────────

function ConvertTo-XmlText {
    param([string]$Value)
    return [System.Security.SecurityElement]::Escape($Value)
}

function New-TaskXml {
    param(
        [Parameter(Mandatory)][string]$TaskUri,
        [Parameter(Mandatory)][string]$UserId,
        [Parameter(Mandatory)][string]$PwshPath,
        [Parameter(Mandatory)][string]$SupervisorPath
    )

    # WHY a LITERAL here-string plus String.Replace, and not an expandable
    # here-string: in an expandable here-string every `$` is live, so a path or
    # username containing `$` would be silently mangled or would inject content
    # into the XML. Placeholders are substituted with ordinal String.Replace
    # (NOT -replace, whose replacement side also interprets `$1`/`$&`), and
    # every substituted value is XML-escaped first.
    #
    # NOTHING SECRET MAY EVER ENTER THIS TEMPLATE. Everything on a task action
    # command line is world-readable via `schtasks /Query /XML`,
    # `Get-ScheduledTask | Select -Expand Actions`, the Task Scheduler MMC, and
    # C:\Windows\System32\Tasks\. BUZZ_PRIVATE_KEY reaches the child only via
    # the supervisor's DPAPI read at runtime.
    $template = @'
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Durable host for the Buzz SMS operator harness (buzz-acp + sms-operator pack). Managed by scripts/sms-harness-task.ps1 — edit there, not here.</Description>
    <URI>{{URI}}</URI>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{{USER}}</UserId>
      <Delay>PT1M</Delay>
    </LogonTrigger>
    <TimeTrigger>
      <Repetition>
        <Interval>PT5M</Interval>
        <StopAtDurationEnd>false</StopAtDurationEnd>
      </Repetition>
      <StartBoundary>2026-01-01T00:00:00</StartBoundary>
      <Enabled>true</Enabled>
    </TimeTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{{USER}}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>3</Count>
    </RestartOnFailure>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{{PWSH}}</Command>
      <Arguments>-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "{{SUPERVISOR}}"</Arguments>
    </Exec>
  </Actions>
</Task>
'@

    $xml = $template
    $xml = $xml.Replace('{{URI}}', (ConvertTo-XmlText $TaskUri))
    $xml = $xml.Replace('{{USER}}', (ConvertTo-XmlText $UserId))
    $xml = $xml.Replace('{{PWSH}}', (ConvertTo-XmlText $PwshPath))
    $xml = $xml.Replace('{{SUPERVISOR}}', (ConvertTo-XmlText $SupervisorPath))

    if ($xml -match '\{\{[A-Z]+\}\}') {
        throw "internal error: unsubstituted placeholder left in the generated task XML"
    }
    return $xml
}

function Write-TaskXmlFile {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$Xml)
    # Encoding matrix verified on this machine:
    #   UTF-16LE + BOM with prolog encoding="UTF-16"  -> SUCCESS
    #   utf8NoBOM  with prolog encoding="UTF-8"       -> "unable to switch the encoding"
    #   UTF-8 + BOM with prolog encoding="UTF-8"      -> "incorrect document syntax"
    # [Text.Encoding]::Unicode is UTF-16LE and WriteAllText emits its preamble
    # (the BOM), which is exactly the combination schtasks accepts. Do not
    # "modernise" this to Set-Content -Encoding utf8.
    [System.IO.File]::WriteAllText($Path, $Xml, [System.Text.Encoding]::Unicode)
}

# ─────────────────────────────────────────────────────────────────────────────
# Task assertions
# ─────────────────────────────────────────────────────────────────────────────

function Get-HarnessTask {
    param([Parameter(Mandatory)][string]$Name)
    return Get-ScheduledTask -TaskName $Name -TaskPath '\' -ErrorAction SilentlyContinue
}

function Get-TriggerOfType {
    param([Parameter(Mandatory)]$Task, [Parameter(Mandatory)][string]$CimClassName)
    foreach ($t in @($Task.Triggers)) {
        if ($t.CimClass.CimClassName -eq $CimClassName) { return $t }
    }
    return $null
}

# Returns a list of failure strings. EMPTY means every invariant held.
#
# The expected values below are the values Get-ScheduledTask reads BACK, which
# are not always the values written into the XML — verified empirically:
#   XML LogonType InteractiveToken -> readback Interactive
#   XML RunLevel  LeastPrivilege   -> readback Limited
#   XML Principal UserId DOMAIN\u  -> readback u  (bare, domain stripped)
# Asserting the XML spellings here would fail a perfectly correct install.
function Test-TaskInvariants {
    param([Parameter(Mandatory)]$Task, [Parameter(Mandatory)][string]$ExpectedUser)

    $failures = [System.Collections.Generic.List[string]]::new()
    $s = $Task.Settings

    # THE single most important assertion. PT0S means "no time limit". A
    # flag-created task gets PT72H and Task Scheduler kills the harness after
    # three days — a death that looks exactly like the harness silently dying
    # on its own. This must never be downgraded to a warning.
    if ($s.ExecutionTimeLimit -ne 'PT0S') {
        $failures.Add("ExecutionTimeLimit is '$($s.ExecutionTimeLimit)', expected 'PT0S' (a non-PT0S limit makes Task Scheduler kill the harness — a flag-created task gets PT72H)")
    }
    if ($s.MultipleInstances -ne 'IgnoreNew') {
        $failures.Add("MultipleInstances is '$($s.MultipleInstances)', expected 'IgnoreNew' (anything else lets the 5-minute watchdog start a SECOND supervisor and double every agent turn)")
    }
    if ($s.DisallowStartIfOnBatteries) {
        $failures.Add("DisallowStartIfOnBatteries is True, expected False (the harness must start on battery)")
    }
    if ($s.StopIfGoingOnBatteries) {
        $failures.Add("StopIfGoingOnBatteries is True, expected False (unplugging must not kill the harness)")
    }
    if ($s.RunOnlyIfNetworkAvailable) {
        $failures.Add("RunOnlyIfNetworkAvailable is True, expected False (the supervisor's own retry loop handles a missing network better than Task Scheduler refusing to start)")
    }
    if (-not $s.StartWhenAvailable) {
        $failures.Add("StartWhenAvailable is False, expected True (a missed watchdog run must be caught up)")
    }
    if ($s.RestartInterval -ne 'PT1M') {
        $failures.Add("RestartOnFailure Interval is '$($s.RestartInterval)', expected 'PT1M'")
    }
    if ($s.RestartCount -ne 3) {
        $failures.Add("RestartOnFailure Count is '$($s.RestartCount)', expected 3")
    }

    $p = $Task.Principal
    if ($p.LogonType -notin @('Interactive', 'InteractiveToken')) {
        $failures.Add("Principal LogonType is '$($p.LogonType)', expected Interactive/InteractiveToken (Password stores a second long-lived secret; S4U breaks DPAPI and network credentials)")
    }
    if ($p.RunLevel -notin @('Limited', 'LeastPrivilege')) {
        $failures.Add("Principal RunLevel is '$($p.RunLevel)', expected Limited/LeastPrivilege")
    }
    $bareUser = ($ExpectedUser -split '\\')[-1]
    if (($p.UserId -split '\\')[-1] -ine $bareUser) {
        $failures.Add("Principal UserId is '$($p.UserId)', expected '$ExpectedUser' (DPAPI CurrentUser binds the key blob to this user)")
    }

    # No BootTrigger, ever: SYSTEM gets C:\Windows\System32\config\systemprofile,
    # which breaks Claude Code CLI auth, DPAPI CurrentUser, and the per-user
    # gitconfig for the bidcraft checkout — all silently.
    if (Get-TriggerOfType -Task $Task -CimClassName 'MSFT_TaskBootTrigger') {
        $failures.Add("a BootTrigger is registered; it must not be (a pre-logon SYSTEM context breaks Claude Code auth, DPAPI, and the per-user gitconfig)")
    }

    $logon = Get-TriggerOfType -Task $Task -CimClassName 'MSFT_TaskLogonTrigger'
    if (-not $logon) {
        $failures.Add("no LogonTrigger is registered (the harness would never start at logon)")
    } else {
        if (-not $logon.Enabled) { $failures.Add("the LogonTrigger is disabled") }
        if ($logon.Delay -ne 'PT1M') { $failures.Add("LogonTrigger Delay is '$($logon.Delay)', expected 'PT1M'") }
        if (($logon.UserId -split '\\')[-1] -ine $bareUser) {
            $failures.Add("LogonTrigger UserId is '$($logon.UserId)', expected '$ExpectedUser'")
        }
    }

    $time = Get-TriggerOfType -Task $Task -CimClassName 'MSFT_TaskTimeTrigger'
    if (-not $time) {
        $failures.Add("no repeating TimeTrigger is registered (the 5-minute watchdog is missing)")
    } else {
        if (-not $time.Enabled) { $failures.Add("the watchdog TimeTrigger is disabled") }
        $rep = $time.Repetition
        if (-not $rep -or $rep.Interval -ne 'PT5M') {
            $interval = if ($rep) { $rep.Interval } else { '<none>' }
            $failures.Add("watchdog Repetition Interval is '$interval', expected 'PT5M'")
        }
        # A Duration would make the repetition STOP after that window, quietly
        # retiring the watchdog. It must be absent/empty = repeat indefinitely.
        if ($rep -and -not [string]::IsNullOrWhiteSpace("$($rep.Duration)")) {
            $failures.Add("watchdog Repetition Duration is '$($rep.Duration)', expected empty (a Duration retires the watchdog after that window)")
        }
    }

    $actions = @($Task.Actions)
    if ($actions.Count -ne 1) {
        $failures.Add("the task has $($actions.Count) actions, expected exactly 1")
    } else {
        $a = $actions[0]
        $exec = Get-ConfigValue -Config $a -Name 'Execute' -Default ''
        $argLine = Get-ConfigValue -Config $a -Name 'Arguments' -Default ''
        # The action must point at the SUPERVISOR, never at buzz-acp.exe — the
        # supervisor is what scrubs CLAUDECODE, reads the DPAPI key, and owns
        # the restart loop.
        if ($exec -notmatch 'pwsh\.exe$') {
            $failures.Add("the task action runs '$exec', expected pwsh.exe (it must launch the supervisor, never buzz-acp.exe directly)")
        }
        if ($argLine -notmatch 'supervisor\.ps1') {
            $failures.Add("the task action arguments do not reference supervisor.ps1: '$argLine'")
        }
        if ($argLine -match 'buzz-acp') {
            $failures.Add("the task action references buzz-acp directly; it must launch the supervisor instead")
        }
        # Belt-and-braces: nothing secret may ever sit on the action line.
        if ($argLine -match 'BUZZ_PRIVATE_KEY' -or $argLine -match '(?<![0-9a-fA-F])[0-9a-fA-F]{64}(?![0-9a-fA-F])') {
            $failures.Add("the task action command line appears to carry a secret; it is world-readable and must never contain one")
        }
    }

    return $failures
}

# ─────────────────────────────────────────────────────────────────────────────
# Heartbeat
# ─────────────────────────────────────────────────────────────────────────────

# Age is taken from the file's LastWriteTime, NOT by parsing its contents. The
# supervisor owns the file format; coupling to it here would make a cosmetic
# change over there silently break liveness checking on this side.
function Get-HeartbeatAgeSeconds {
    param([Parameter(Mandatory)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    $item = Get-Item -LiteralPath $Path
    return [int]((Get-Date) - $item.LastWriteTime).TotalSeconds
}

# schtasks /Run exits 0 even when MultipleInstancesPolicy refuses the start, so
# its exit code proves nothing. A fresh heartbeat is the only real proof.
function Assert-HeartbeatFresh {
    param(
        [Parameter(Mandatory)][string]$Path,
        [int]$MaxAgeSeconds = 90,
        [int]$TimeoutSeconds = 150
    )
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $age = Get-HeartbeatAgeSeconds -Path $Path
        if ($null -ne $age -and $age -le $MaxAgeSeconds) { return $age }
        Start-Sleep -Seconds 3
    }
    return $null
}

# ─────────────────────────────────────────────────────────────────────────────
# Secret store helpers
# ─────────────────────────────────────────────────────────────────────────────

# Set-Content appends a trailing newline, and ConvertTo-SecureString rejects the
# resulting string with "The input string was not in a correct format" when it
# is read with -Raw. Verified. Stripping whitespace makes the read correct under
# both -Raw and line-mode; do not "simplify" this back to a bare Get-Content -Raw.
function Read-KeyCiphertext {
    param([Parameter(Mandatory)][string]$Path)
    return ((Get-Content -LiteralPath $Path -Raw) -replace '\s', '')
}

# icacls can exit 0 while reporting "Successfully processed 0 files", which
# would leave the secret store with inherited permissions. Check both.
function Protect-SecretText {
    # Masks a 64-hex Nostr private key or an nsec1 bech32 string. Used on every
    # path that echoes file content the harness child produced.
    param([string]$Text)
    if ([string]::IsNullOrEmpty($Text)) { return $Text }
    return [regex]::Replace($Text, '(?i)\b[0-9a-f]{64}\b|nsec1[02-9ac-hj-np-z]{20,}', '<redacted>')
}

function Test-AclIsRestricted {
    # Confirms /inheritance:r is still in effect and no principal beyond the
    # current user holds access. Permissions were previously set once by
    # -ProvisionSecret and then assumed forever; a restore-from-backup, a
    # manual copy, or a re-created install root silently re-inherits the
    # parent's ACL, and nothing noticed.
    param([Parameter(Mandatory)][string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) { return $false }
    $acl = Get-Acl -LiteralPath $Path
    if ($acl.AreAccessRulesProtected -ne $true) { return $false }
    $me = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
    foreach ($rule in $acl.Access) {
        if ($rule.IdentityReference.Value -ne $me) { return $false }
    }
    return $true
}

function New-RestrictedDirectory {
    # Create-and-lock in one step. Previously Invoke-Install and Invoke-Stop
    # created the install root with a bare New-Item and no icacls, so with a
    # non-default -InstallRoot (a shared or synced folder) the tree — and every
    # artifact later written into it — inherited that parent's ACL.
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$Grant)
    $existed = Test-Path -LiteralPath $Path
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
    if (-not $existed -or -not (Test-AclIsRestricted -Path $Path)) {
        [void] (Set-RestrictedAcl -Path $Path -Grant $Grant)
    }
}

function Set-RestrictedAcl {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$Grant)
    $display = Format-CommandLine -FilePath 'icacls.exe' -Arguments @($Path, '/inheritance:r', '/grant:r', $Grant)
    Write-Diag "run: $display"
    $output = & icacls.exe $Path /inheritance:r /grant:r $Grant 2>&1
    $code = $LASTEXITCODE
    $text = ($output | Out-String)
    if ($code -ne 0) {
        Write-Fail "icacls failed (exit $code) on ${Path}: $text"
        return $false
    }
    if ($text -notmatch 'Successfully processed\s+([1-9]\d*)\s+files') {
        Write-Fail "icacls exited 0 but did not report processing any file for ${Path}; permissions were NOT restricted: $text"
        return $false
    }
    return $true
}

# ─────────────────────────────────────────────────────────────────────────────
# Verb: -ProvisionSecret
# ─────────────────────────────────────────────────────────────────────────────

function Invoke-ProvisionSecret {
    param([Parameter(Mandatory)]$Layout)

    # DPAPI CurrentUser scope binds the ciphertext to THIS user on THIS machine.
    # That is precisely why the task principal must be an InteractiveToken logon
    # of the same user — and why this must be typed by a human, not automated.
    if (-not [Environment]::UserInteractive) {
        Write-Fail "-ProvisionSecret requires an interactive session (the key is typed at a masked prompt, never passed as a parameter)"
        return $script:ExitFail
    }
    if ($DryRun) {
        Write-Line "DRYRUN would create: $($Layout.Root), $($Layout.Logs)"
        Write-Line "DRYRUN would run: $(Format-CommandLine -FilePath 'icacls.exe' -Arguments @($Layout.Root, '/inheritance:r', '/grant:r', "${env:USERNAME}:(OI)(CI)F"))"
        Write-Line "DRYRUN would prompt (masked) for BUZZ_PRIVATE_KEY and write the DPAPI blob to: $($Layout.KeyFile)"
        Write-Line "DRYRUN would run: $(Format-CommandLine -FilePath 'icacls.exe' -Arguments @($Layout.KeyFile, '/inheritance:r', '/grant:r', "${env:USERNAME}:R"))"
        return $script:ExitOk
    }

    New-Item -ItemType Directory -Force -Path $Layout.Root | Out-Null
    New-Item -ItemType Directory -Force -Path $Layout.Logs | Out-Null

    if (-not (Set-RestrictedAcl -Path $Layout.Root -Grant "${env:USERNAME}:(OI)(CI)F")) {
        return $script:ExitFail
    }

    Write-Diag "the key is read at a masked prompt so it never enters PSReadLine history, and is never echoed"
    $first = Read-Host -AsSecureString -Prompt 'BUZZ_PRIVATE_KEY (input hidden)'
    $second = Read-Host -AsSecureString -Prompt 'Re-enter to confirm'

    $plainFirst = $null
    $plainSecond = $null
    try {
        # Both plaintexts live only inside this try block and are overwritten in
        # the finally. The shape check catches a truncated paste, which would
        # otherwise present as a harness that starts, shows online, fires the
        # eyes reaction and answers nothing.
        $plainFirst = [System.Net.NetworkCredential]::new('', $first).Password
        $plainSecond = [System.Net.NetworkCredential]::new('', $second).Password

        if ($plainFirst -ne $plainSecond) {
            Write-Fail "the two entries did not match; nothing was written"
            return $script:ExitFail
        }
        if ([string]::IsNullOrWhiteSpace($plainFirst)) {
            Write-Fail "an empty key was entered; nothing was written"
            return $script:ExitFail
        }

        # Classification only — the value itself is never printed.
        $shape = if ($plainFirst -match '^[0-9a-fA-F]{64}$') { '64-char hex' }
        elseif ($plainFirst -match '^nsec1[0-9a-z]{20,}$') { 'nsec1 bech32' }
        else { 'UNRECOGNISED' }
        if ($shape -eq 'UNRECOGNISED') {
            Write-Warn "the entered key matches neither 64-char hex nor nsec1 bech32 (length $($plainFirst.Length)); writing it anyway, but check it if the harness never replies"
        } else {
            Write-Diag "key shape looks like: $shape"
        }

        $cipher = ConvertFrom-SecureString $first
        Set-Content -LiteralPath $Layout.KeyFile -Value $cipher -Encoding ascii
    } finally {
        if ($plainFirst) { $plainFirst = ('0' * $plainFirst.Length) }
        if ($plainSecond) { $plainSecond = ('0' * $plainSecond.Length) }
        $plainFirst = $null
        $plainSecond = $null
        if ($first) { $first.Dispose() }
        if ($second) { $second.Dispose() }
        [GC]::Collect()
    }

    if (-not (Set-RestrictedAcl -Path $Layout.KeyFile -Grant "${env:USERNAME}:R")) {
        return $script:ExitFail
    }

    # Prove the blob decrypts NOW rather than discovering at 3am that it does not.
    try {
        $probe = Read-KeyCiphertext -Path $Layout.KeyFile | ConvertTo-SecureString
        $probe.Dispose()
        Write-Diag "verified: the DPAPI blob decrypts as $env:USERNAME on this machine"
    } catch {
        Write-Fail "the DPAPI blob was written but does NOT decrypt back: $($_.Exception.Message)"
        return $script:ExitFail
    }

    Write-Line "provisioned: $($Layout.KeyFile)"
    Write-Diag "DPAPI CurrentUser: this blob is bound to $env:USERNAME on this machine. A machine rebuild or user change makes it undecryptable and there is no backup by design — re-run -ProvisionSecret."
    return $script:ExitOk
}

# ─────────────────────────────────────────────────────────────────────────────
# Verb: -Install
# ─────────────────────────────────────────────────────────────────────────────

function Invoke-Install {
    param([Parameter(Mandatory)]$Layout, [Parameter(Mandatory)][string]$SourceScriptRoot)

    $sourceSupervisor = Join-Path $SourceScriptRoot $script:SupervisorFileName
    if (-not (Test-Path -LiteralPath $sourceSupervisor)) {
        Write-Fail "supervisor source not found: $sourceSupervisor (pass -SourceRoot <repo root> if you are not running from the repo)"
        return $script:ExitFail
    }

    $pwshPath = Resolve-PwshPath
    if (-not $pwshPath) {
        Write-Fail "pwsh.exe (PowerShell 7) could not be resolved; the task action would be unrunnable"
        return $script:ExitFail
    }
    Write-Diag "task action will run: $pwshPath"

    $user = "$env:USERDOMAIN\$env:USERNAME"
    $xml = New-TaskXml -TaskUri "\$TaskName" -UserId $user -PwshPath $pwshPath -SupervisorPath $Layout.Supervisor

    if ($DryRun) {
        Write-Line "DRYRUN would create: $($Layout.Root), $($Layout.Logs)"
        Write-Line "DRYRUN would copy:   $sourceSupervisor -> $($Layout.Supervisor)"
        Write-Line "DRYRUN would write:  $($Layout.Manifest)"
        if (Test-Path -LiteralPath $Layout.Config) {
            Write-Line "DRYRUN would LEAVE existing config untouched: $($Layout.Config)"
        } else {
            Write-Line "DRYRUN would write default config: $($Layout.Config)"
        }
        Write-Line "DRYRUN would write UTF-16LE task XML: $($Layout.TaskXml)"
        Write-Line "DRYRUN would run: $(Format-CommandLine -FilePath 'schtasks.exe' -Arguments @('/Create', '/TN', $TaskName, '/XML', $Layout.TaskXml, '/F'))"
        Write-Line "DRYRUN would then assert the registered task invariants (ExecutionTimeLimit=PT0S, MultipleInstances=IgnoreNew, triggers, principal)"
        return $script:ExitOk
    }

    # Create-and-lock, in case -Install runs before -ProvisionSecret. Verb
    # ordering is not enforced anywhere, so neither verb may assume the other
    # already restricted the tree.
    $grant = "$([System.Security.Principal.WindowsIdentity]::GetCurrent().Name):(OI)(CI)(F)"
    New-RestrictedDirectory -Path $Layout.Root -Grant $grant
    New-RestrictedDirectory -Path $Layout.Logs -Grant $grant

    Copy-Item -LiteralPath $sourceSupervisor -Destination $Layout.Supervisor -Force
    $hash = (Get-FileHash -LiteralPath $Layout.Supervisor -Algorithm SHA256).Hash
    Write-Diag "deployed supervisor.ps1 (sha256 $hash)"

    $manifest = [ordered]@{
        taskName     = $TaskName
        installedUtc = (Get-Date).ToUniversalTime().ToString('o')
        sourcePath   = (Get-NormalizedPath $sourceSupervisor)
        deployedPath = (Get-NormalizedPath $Layout.Supervisor)
        sourceSha256 = $hash
        pwshPath     = $pwshPath
        principal    = $user
        installedBy  = "$env:USERNAME@$env:COMPUTERNAME"
    }
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $Layout.Manifest -Encoding utf8

    # Never clobber operator edits. A re-install must not silently reset a
    # promoted binary path back to the worktree default.
    if (Test-Path -LiteralPath $Layout.Config) {
        Write-Diag "harness.config.json already exists — left untouched"
    } else {
        (Get-DefaultConfig) | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $Layout.Config -Encoding utf8
        Write-Diag "wrote default harness.config.json"
    }

    Write-TaskXmlFile -Path $Layout.TaskXml -Xml $xml
    Write-Diag "wrote UTF-16LE task XML: $($Layout.TaskXml)"

    $code = Invoke-Native -FilePath 'schtasks.exe' -Arguments @('/Create', '/TN', $TaskName, '/XML', $Layout.TaskXml, '/F')
    if ($code -ne 0) {
        Write-Fail "schtasks /Create failed (exit $code)"
        return $script:ExitFail
    }

    # POST-INSTALL ASSERT. A failure here is a HARD failure, never a warning:
    # the whole point is that a task can register successfully and still be
    # configured to kill the harness after 72 hours.
    $task = Get-HarnessTask -Name $TaskName
    if (-not $task) {
        Write-Fail "schtasks reported success but '$TaskName' cannot be read back"
        return $script:ExitFail
    }
    # @() is load-bearing: PowerShell UNROLLS a returned collection, so an
    # all-clear result comes back as $null and a single failure comes back as a
    # bare string — both of which make a naive .Count throw under StrictMode.
    $failures = @(Test-TaskInvariants -Task $task -ExpectedUser $user)
    if ($failures.Count -gt 0) {
        Write-Fail "the task registered but does NOT satisfy its invariants, so it must not be trusted:"
        foreach ($f in $failures) { Write-Fail "  - $f" }
        Write-Fail "fix the cause and re-run -Install (it uses /F and will overwrite), or -Uninstall to remove it."
        return $script:ExitFail
    }
    Write-Line "installed: task '$TaskName' registered and all invariants asserted (ExecutionTimeLimit=PT0S)"

    if ($RegisterEventSource) {
        # Strictly optional and never a hard install dependency: New-EventLog
        # needs elevation, and the harness is perfectly observable without it.
        try {
            if (-not [System.Diagnostics.EventLog]::SourceExists($script:EventSourceName)) {
                New-EventLog -LogName Application -Source $script:EventSourceName
                Write-Line "registered Windows event source '$($script:EventSourceName)'"
            } else {
                Write-Diag "event source '$($script:EventSourceName)' already exists"
            }
        } catch {
            Write-Warn "could not register the event source (this needs one elevated run and is optional): $($_.Exception.Message)"
        }
    }

    Write-Diag "the task is registered but NOT started. Run -Start when the secret is provisioned."
    return $script:ExitOk
}

# ─────────────────────────────────────────────────────────────────────────────
# Verb: -Start
# ─────────────────────────────────────────────────────────────────────────────

function Invoke-Start {
    param([Parameter(Mandatory)]$Layout)

    $task = Get-HarnessTask -Name $TaskName
    if (-not $task -and -not $DryRun) {
        Write-Fail "task '$TaskName' is not registered; run -Install first"
        return $script:ExitFail
    }
    if (-not (Test-Path -LiteralPath $Layout.KeyFile) -and -not $DryRun) {
        Write-Fail "no DPAPI key store at $($Layout.KeyFile); run -ProvisionSecret first (the supervisor refuses to start without it)"
        return $script:ExitFail
    }

    if ($DryRun) {
        Write-Line "DRYRUN would delete: $($Layout.StopFlag)"
        Write-Line "DRYRUN would run: $(Format-CommandLine -FilePath 'schtasks.exe' -Arguments @('/Run', '/TN', $TaskName))"
        Write-Line "DRYRUN would then poll $($Layout.Heartbeat) for an age under 90s (schtasks /Run exit 0 proves nothing)"
        return $script:ExitOk
    }

    # stop.flag FIRST — the supervisor checks it at startup and before every
    # relaunch, so leaving it in place means the task starts and immediately
    # exits 0, which reads exactly like a broken install.
    if (Test-Path -LiteralPath $Layout.StopFlag) {
        Remove-Item -LiteralPath $Layout.StopFlag -Force
        Write-Diag "cleared stop.flag"
    }

    $code = Invoke-Native -FilePath 'schtasks.exe' -Arguments @('/Run', '/TN', $TaskName)
    if ($code -ne 0) {
        Write-Fail "schtasks /Run failed (exit $code)"
        return $script:ExitFail
    }

    Write-Diag "waiting for a fresh heartbeat (schtasks /Run exits 0 even when MultipleInstancesPolicy refuses the start, so its exit code proves nothing)"
    $age = Assert-HeartbeatFresh -Path $Layout.Heartbeat
    if ($null -eq $age) {
        Write-Fail "no fresh heartbeat appeared at $($Layout.Heartbeat)."
        Write-Fail "the task may have started and exited immediately. Check, in order:"
        Write-Fail "  1. $($Layout.ErrLog) and $($Layout.OutLog)"
        Write-Fail "  2. that the configured buzz-acp.exe still exists (a rebuild or branch switch can remove it)"
        Write-Fail "  3. that the DPAPI key still decrypts as this user"
        return $script:ExitFail
    }
    Write-Line "started: heartbeat is ${age}s old"
    return $script:ExitOk
}

# ─────────────────────────────────────────────────────────────────────────────
# Verb: -Stop
# ─────────────────────────────────────────────────────────────────────────────

# Order is NOT negotiable:
#   1. stop.flag   — so a live supervisor cannot instantly relaunch the child
#                    we are about to kill, and so it exits rather than looping.
#   2. schtasks /End — kills ONLY the supervisor (verified). On its own this
#                    ORPHANS buzz-acp, which keeps its relay subscription and
#                    keeps posting reactions while the task reads "Ready".
#   3. taskkill the recorded child PID, Path-guarded.
#   4. sweep any remaining buzz-acp whose .Path matches ours, Path-guarded.
#   5. assert none of ours remain, and report.
function Invoke-Stop {
    param([Parameter(Mandatory)]$Layout, [switch]$Quiet)

    $config = Get-EffectiveConfig -Layout $Layout
    $buzzAcpExe = Get-NormalizedPath ([string]$config['buzzAcpExe'])

    if ($DryRun) {
        Write-Line "DRYRUN would write: $($Layout.StopFlag)"
        Write-Line "DRYRUN would run: $(Format-CommandLine -FilePath 'schtasks.exe' -Arguments @('/End', '/TN', $TaskName))"
        $recorded = Get-RecordedChildPid -Layout $Layout
        if ($recorded) {
            Write-Line "DRYRUN would run: $(Format-CommandLine -FilePath 'taskkill.exe' -Arguments @('/PID', "$recorded", '/T', '/F')) (only if its .Path matches $buzzAcpExe)"
        } else {
            Write-Line "DRYRUN no child PID recorded in $($Layout.Pid)"
        }
        foreach ($p in (Get-OurBuzzAcpProcesses -BuzzAcpExe $buzzAcpExe)) {
            Write-Line "DRYRUN would run: $(Format-CommandLine -FilePath 'taskkill.exe' -Arguments @('/PID', "$($p.Id)", '/T', '/F'))"
        }
        return $script:ExitOk
    }

    # ── 1. stop.flag ────────────────────────────────────────────────────────
    New-Item -ItemType Directory -Force -Path $Layout.Root | Out-Null
    Set-Content -LiteralPath $Layout.StopFlag -Encoding utf8 -Value @"
Written by sms-harness-task.ps1 -Stop at $((Get-Date).ToUniversalTime().ToString('o')) by $env:USERNAME.
While this file exists the supervisor refuses to start and refuses to relaunch its child.
Remove it (or run -Start, which removes it) to allow the harness to run again.
"@
    Write-Diag "wrote stop.flag"

    # ── 2. end the task (supervisor only) ───────────────────────────────────
    $endCode = Invoke-Native -FilePath 'schtasks.exe' -Arguments @('/End', '/TN', $TaskName)
    if ($endCode -ne 0) {
        # Not fatal: the task may not be registered, or may already be idle.
        # And a zero would not have proved anything either.
        Write-Warn "schtasks /End returned $endCode (continuing — its exit code proves nothing about the child)"
    }

    # ── 3. the recorded child ───────────────────────────────────────────────
    $recordedPid = Get-RecordedChildPid -Layout $Layout
    if ($recordedPid) {
        Stop-GuardedProcess -ProcessId $recordedPid -ExpectedPath $buzzAcpExe -Reason 'recorded child from harness.pid'
    } else {
        Write-Diag "no child PID recorded in harness.pid"
    }

    # ── 4. sweep survivors, Path-guarded ────────────────────────────────────
    foreach ($p in (Get-OurBuzzAcpProcesses -BuzzAcpExe $buzzAcpExe)) {
        Stop-GuardedProcess -ProcessId $p.Id -ExpectedPath $buzzAcpExe -Reason 'orphan sweep'
    }

    # ── 5. assert ───────────────────────────────────────────────────────────
    Start-Sleep -Seconds 2
    # @() because a returned collection is unrolled: no survivors would arrive
    # as $null and .Count would throw under StrictMode — turning the "did the
    # stop actually work" assertion into a crash.
    $survivors = @(Get-OurBuzzAcpProcesses -BuzzAcpExe $buzzAcpExe)
    if ($survivors.Count -gt 0) {
        Write-Fail "$($survivors.Count) buzz-acp process(es) matching $buzzAcpExe survived the stop sequence:"
        foreach ($p in $survivors) { Write-Fail "  PID $($p.Id)  $(Get-ProcessPathSafe $p)" }
        return $script:ExitFail
    }

    if (Test-Path -LiteralPath $Layout.Pid) {
        Remove-Item -LiteralPath $Layout.Pid -Force -ErrorAction SilentlyContinue
    }

    if (-not $Quiet) {
        Write-Line "stopped: stop.flag set, task ended, no buzz-acp process matching $buzzAcpExe remains"
        # Other-path buzz-acp processes are deliberately left alone AND reported.
        $foreign = @(Get-AllBuzzAcpProcesses | Where-Object { -not (Test-SamePath (Get-ProcessPathSafe $_) $buzzAcpExe) })
        if ($foreign.Count -gt 0) {
            Write-Warn "$($foreign.Count) OTHER buzz-acp process(es) are running from a different path and were left untouched:"
            foreach ($p in $foreign) { Write-Warn "  PID $($p.Id)  $(Get-ProcessPathSafe $p)" }
        }
    }
    return $script:ExitOk
}

function Get-RecordedChildPid {
    param([Parameter(Mandatory)]$Layout)
    if (-not (Test-Path -LiteralPath $Layout.Pid)) { return $null }
    $raw = (Get-Content -LiteralPath $Layout.Pid -Raw).Trim()
    $parsed = 0
    if ([int]::TryParse($raw, [ref]$parsed) -and $parsed -gt 0) { return $parsed }
    Write-Warn "harness.pid does not contain a PID: '$raw'"
    return $null
}

# NEVER kill by PID alone. PIDs are reused across a reboot, and a developer's
# `just goose` harness in another worktree is also called buzz-acp.
function Stop-GuardedProcess {
    param(
        [Parameter(Mandatory)][int]$ProcessId,
        [Parameter(Mandatory)][string]$ExpectedPath,
        [Parameter(Mandatory)][string]$Reason
    )
    $proc = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if (-not $proc) {
        Write-Diag "PID $ProcessId ($Reason) is not running"
        return
    }
    $actual = Get-ProcessPathSafe $proc
    if (-not (Test-SamePath $actual $ExpectedPath)) {
        Write-Warn "PID $ProcessId ($Reason) runs '$actual', not '$ExpectedPath' — NOT killing it (PID reuse, or another worktree's harness)"
        return
    }
    # /T so the agent subprocess tree goes with it; /F because a graceful
    # shutdown request is not available to us here.
    $null = Invoke-Native -FilePath 'taskkill.exe' -Arguments @('/PID', "$ProcessId", '/T', '/F')
}

# ─────────────────────────────────────────────────────────────────────────────
# Verb: -Status
# ─────────────────────────────────────────────────────────────────────────────

function Invoke-Status {
    param([Parameter(Mandatory)]$Layout, [Parameter(Mandatory)][string]$SourceScriptRoot)

    $config = Get-EffectiveConfig -Layout $Layout
    $buzzAcpExe = Get-NormalizedPath ([string]$config['buzzAcpExe'])

    Write-Line "═══ SMS harness status ═══════════════════════════════════════"
    Write-Line "task name    : $TaskName"
    Write-Line "install root : $($Layout.Root)"
    Write-Line "buzz-acp exe : $buzzAcpExe"
    if (-not (Test-Path -LiteralPath $buzzAcpExe)) {
        Write-Line "               [RED] THIS BINARY DOES NOT EXIST — a rebuild, cargo clean or branch switch removed it" -Colour red
    }
    Write-Line ""

    # ── Task Scheduler's view ───────────────────────────────────────────────
    Write-Line "── Task Scheduler view ───────────────────────────────────────"
    $task = Get-HarnessTask -Name $TaskName
    if (-not $task) {
        Write-Line "[RED] task '$TaskName' is NOT registered" -Colour red
    } else {
        Write-Line "state        : $($task.State)"
        Write-Line "enabled      : $($task.Settings.Enabled)"
        $info = Get-ScheduledTaskInfo -TaskName $TaskName -TaskPath '\' -ErrorAction SilentlyContinue
        if ($info) {
            Write-Line "last run     : $($info.LastRunTime)"
            Write-Line "next run     : $($info.NextRunTime)"
            Write-Line "missed runs  : $($info.NumberOfMissedRuns)"
            $r = $info.LastTaskResult
            $u = [uint32]($r -band 0xFFFFFFFF)
            Write-Line "last result  : $r / 0x$('{0:X8}' -f $u)"
            if ($u -eq [uint32]$script:HealthyRefusalCode) {
                Write-Line "               2147946720 / 0x800710E0 = the 5-minute watchdog being refused" -Colour ok
                Write-Line "               because it is already running — EXPECTED AND HEALTHY." -Colour ok
                Write-Line "               Do NOT 'fix' this by relaxing MultipleInstancesPolicy." -Colour ok
            } elseif ($r -eq 0) {
                Write-Line "               0 = the last launch completed. For a supervisor meant to run forever," -Colour yellow
                Write-Line "               a recent 0 means it EXITED — check stop.flag and the logs below." -Colour yellow
            }
        }
    }
    Write-Line ""

    # ── What is actually true ───────────────────────────────────────────────
    Write-Line "── Real liveness view ────────────────────────────────────────"
    $age = Get-HeartbeatAgeSeconds -Path $Layout.Heartbeat
    if ($null -eq $age) {
        Write-Line "heartbeat    : [RED] MISSING ($($Layout.Heartbeat))" -Colour red
    } elseif ($age -gt 120) {
        Write-Line "heartbeat    : [RED] STALE — ${age}s old (the supervisor rewrites it every 60s)" -Colour red
    } else {
        Write-Line "heartbeat    : [OK] ${age}s old" -Colour ok
    }
    if (Test-Path -LiteralPath $Layout.Heartbeat) {
        foreach ($line in @(Get-Content -LiteralPath $Layout.Heartbeat -ErrorAction SilentlyContinue)) {
            Write-Line "               $line"
        }
    }

    if (Test-Path -LiteralPath $Layout.StopFlag) {
        Write-Line "stop.flag    : [RED] PRESENT — the supervisor will refuse to start. Run -Start to clear it." -Colour red
    } else {
        Write-Line "stop.flag    : absent"
    }

    $recorded = Get-RecordedChildPid -Layout $Layout
    Write-Line "harness.pid  : $(if ($recorded) { $recorded } else { '<none>' })"

    # ALL of them, not just ours — a second instance running with the same key
    # is the failure mode nothing else on this screen would reveal.
    Write-Line ""
    Write-Line "live buzz-acp processes (ALL on this machine):"
    $all = @(Get-AllBuzzAcpProcesses)   # @(): see the note in Invoke-Stop
    if ($all.Count -eq 0) {
        Write-Line "  [RED] none — the harness is NOT running" -Colour red
    } else {
        foreach ($p in $all) {
            $path = Get-ProcessPathSafe $p
            $tag = if (Test-SamePath $path $buzzAcpExe) { '[ours]   ' } else { '[FOREIGN]' }
            Write-Line "  $tag PID $($p.Id)  started $(Get-ProcessStartTimeSafe $p)  $path"
        }
        $ours = @($all | Where-Object { Test-SamePath (Get-ProcessPathSafe $_) $buzzAcpExe })
        if ($ours.Count -gt 1) {
            Write-Line "  [RED] $($ours.Count) instances match our binary. Two harnesses on the same key produce" -Colour red
            Write-Line "        DUPLICATE agent turns against the same working copy — nothing relay-side prevents it." -Colour red
        }
    }

    # ── Drift ───────────────────────────────────────────────────────────────
    Write-Line ""
    $sourceSupervisor = Join-Path $SourceScriptRoot $script:SupervisorFileName
    if ((Test-Path -LiteralPath $Layout.Supervisor) -and (Test-Path -LiteralPath $sourceSupervisor)) {
        $deployedHash = (Get-FileHash -LiteralPath $Layout.Supervisor -Algorithm SHA256).Hash
        $sourceHash = (Get-FileHash -LiteralPath $sourceSupervisor -Algorithm SHA256).Hash
        if ($deployedHash -eq $sourceHash) {
            Write-Line "supervisor   : [OK] deployed copy matches the repo copy" -Colour ok
        } else {
            Write-Line "supervisor   : [RED] DRIFT — the deployed copy differs from scripts/$($script:SupervisorFileName)." -Colour red
            Write-Line "               Re-run -Install, then -Stop, then -Start to deploy the repo version." -Colour red
        }
    } elseif (-not (Test-Path -LiteralPath $Layout.Supervisor)) {
        Write-Line "supervisor   : [RED] not deployed at $($Layout.Supervisor) — run -Install" -Colour red
    }

    # ── Log tails ───────────────────────────────────────────────────────────
    # Redacted on the way out. These files are the harness child's redirected
    # stdout/stderr, and that child holds the live key in its environment: a
    # Rust panic that dumps env, an error chain quoting a URL with an embedded
    # credential, or anything logged under RUST_LOG=debug could put the key in
    # here. -Status is an amplifier — `-Status > status.txt` would copy it into
    # a second, unprotected file — so mask before printing rather than trusting
    # that nothing upstream ever logs it.
    foreach ($log in @($Layout.OutLog, $Layout.ErrLog)) {
        Write-Line ""
        Write-Line "── last 20 lines of $log (redacted) ──"
        if (Test-Path -LiteralPath $log) {
            $tail = @(Get-Content -LiteralPath $log -Tail 20 -ErrorAction SilentlyContinue)
            if ($tail.Count -eq 0) { Write-Line "  <empty>" }
            foreach ($line in $tail) { Write-Line "  $(Protect-SecretText $line)" }
        } else {
            Write-Line "  <missing>"
        }
    }

    # ── The honesty line ────────────────────────────────────────────────────
    Write-Line ""
    Write-Line "══════════════════════════════════════════════════════════════"
    Write-Line "NONE OF THE ABOVE PROVES THE AGENT POOL IS HEALTHY." -Colour yellow
    Write-Line "A live process, a fresh heartbeat and relay presence all stay green while" -Colour yellow
    Write-Line "every agent turn fails: a circuit-breaker-open pool keeps publishing 'online'" -Colour yellow
    Write-Line "and keeps firing the eyes reaction (queued before any agent runs) for a full" -Colour yellow
    Write-Line "5-minute cooldown while replying to nothing. An expired Claude Code CLI login" -Colour yellow
    Write-Line "looks identical. The only real proof is an SMS that gets an answer." -Colour yellow
    return $script:ExitOk
}

# ─────────────────────────────────────────────────────────────────────────────
# Verb: -Verify
# ─────────────────────────────────────────────────────────────────────────────

# Stands in for the paired scripts/test-<name>.sh the repo convention would
# otherwise want: `just ci` lints no .ps1, so nothing else guards these.
# Re-run it after ANY edit to the supervisor.
function Invoke-Verify {
    param([Parameter(Mandatory)]$Layout, [Parameter(Mandatory)][string]$SourceScriptRoot)

    $failures = [System.Collections.Generic.List[string]]::new()
    $checked = 0

    # ── 1. registered task invariants ───────────────────────────────────────
    $task = Get-HarnessTask -Name $TaskName
    if (-not $task) {
        $failures.Add("task '$TaskName' is not registered")
    } else {
        $checked++
        # @() for the same unrolling reason as in Invoke-Install.
        foreach ($f in @(Test-TaskInvariants -Task $task -ExpectedUser "$env:USERDOMAIN\$env:USERNAME")) {
            $failures.Add("task: $f")
        }
    }

    # ── 2. deployed supervisor argument invariants ──────────────────────────
    # Each of these produces a harness that connects, shows online, posts
    # reactions and NEVER replies if a well-meaning refactor drops it.
    if (-not (Test-Path -LiteralPath $Layout.Supervisor)) {
        $failures.Add("supervisor is not deployed at $($Layout.Supervisor) — run -Install")
    } else {
        $checked++
        $text = Get-Content -LiteralPath $Layout.Supervisor -Raw
        $required = @(
            @{ Pattern = '--kinds[''"\s,]+9'; Why = "--kinds 9 is mandatory: --subscribe all with no kinds subscribes to NOTHING (kinds defaults to an empty vec)" },
            @{ Pattern = '--respond-to[''"\s,]+anyone'; Why = "--respond-to anyone is mandatory: the default is owner-only, which silently drops every inbound SMS event" },
            @{ Pattern = '--subscribe[''"\s,]+all'; Why = "--subscribe all is mandatory: the default only reacts to mentions" },
            @{ Pattern = 'target[\\/]+release'; Why = "the target\release PATH prepend is mandatory: the agent runs 'buzz sms set-route' and needs the buzz CLI on PATH" },
            @{ Pattern = 'CLAUDECODE\s*=\s*\$null'; Why = "CLAUDECODE = `$null in the child environment is mandatory: claude-code-acp refuses to start when it inherits CLAUDECODE, and fails as a -32603 that reads like an ACP protocol fault" },
            @{ Pattern = 'Env:\\?CLAUDECODE'; Why = "the supervisor must also scrub CLAUDECODE from its OWN environment (Task Scheduler inherits user-level variables)" },
            @{ Pattern = 'BUZZ_ACP_SETUP_PAYLOAD'; Why = "BUZZ_ACP_SETUP_PAYLOAD must be scrubbed: a stale value diverts buzz-acp into its setup listener and the agent pool never starts" },
            @{ Pattern = 'stop\.flag'; Why = "stop.flag is the only stop signal the supervisor honours (child exit 0 is ambiguous between a relay drop, Ctrl-C and an owner !shutdown)" }
        )
        foreach ($r in $required) {
            if ($text -notmatch $r.Pattern) {
                $failures.Add("supervisor: missing /$($r.Pattern)/ — $($r.Why)")
            }
        }
        # RUST_LOG=debug turns on ACP wire-frame logging, which would write a
        # live private key into the log the moment an MCP command is added.
        if ($text -match 'RUST_LOG\s*=\s*[''"]?debug') {
            $failures.Add("supervisor: sets RUST_LOG=debug, which enables ACP wire-frame logging and would write the live key into the log")
        }
        if ($text -match '--mcp-command') {
            $failures.Add("supervisor: passes --mcp-command, which is forbidden for the unattended instance")
        }
    }

    # ── 3. effective config still points somewhere real ─────────────────────
    $config = Get-EffectiveConfig -Layout $Layout
    $checked++
    $buzzAcpExe = Get-NormalizedPath ([string]$config['buzzAcpExe'])
    if (-not (Test-Path -LiteralPath $buzzAcpExe)) {
        $failures.Add("config: buzzAcpExe does not exist: $buzzAcpExe (a rebuild, cargo clean or branch switch can remove a worktree binary)")
    }
    $prepend = [string]$config['pathPrepend']
    if ($prepend -notmatch 'target[\\/]+release') {
        $failures.Add("config: pathPrepend is '$prepend', which is not a target\release directory — the agent needs the buzz CLI on PATH for 'buzz sms set-route'")
    } elseif (-not (Test-Path -LiteralPath $prepend)) {
        $failures.Add("config: pathPrepend does not exist: $prepend")
    } elseif (-not (Test-Path -LiteralPath (Join-Path $prepend 'buzz.exe'))) {
        $failures.Add("config: buzz.exe is not in $prepend — 'buzz sms set-route' will fail inside the agent")
    }

    # ── 4. no secret ever committed ─────────────────────────────────────────
    # Scans both committed scripts for a bare 64-hex run and for an nsec1
    # bech32 literal. Note the detectors do not match their own regex source:
    # the character after 'nsec1' in the pattern below is '[', which is not in
    # [0-9a-z], so the pattern cannot match itself.
    foreach ($name in @($script:SupervisorFileName, 'sms-harness-task.ps1')) {
        $path = Join-Path $SourceScriptRoot $name
        if (-not (Test-Path -LiteralPath $path)) {
            $failures.Add("committed script missing: $path")
            continue
        }
        $checked++
        $lineNo = 0
        foreach ($line in (Get-Content -LiteralPath $path)) {
            $lineNo++
            if ($line -match 'secret-scan:allow') { continue }
            if ($line -match '(?<![0-9a-fA-F])[0-9a-fA-F]{64}(?![0-9a-fA-F])') {
                $failures.Add("SECRET SCAN: ${name}:${lineNo} contains a bare 64-hex run")
            }
            if ($line -match 'nsec1[0-9a-z]{20,}') {
                $failures.Add("SECRET SCAN: ${name}:${lineNo} contains an nsec1 literal")
            }
        }
    }

    # ── 5. the key store must hold a DPAPI blob, not plaintext ──────────────
    if (Test-Path -LiteralPath $Layout.KeyFile) {
        $checked++
        try {
            $cipher = Read-KeyCiphertext -Path $Layout.KeyFile
            if ($cipher -match '^[0-9a-fA-F]{64}$' -or $cipher -match '^nsec1') {
                $failures.Add("SECRET STORE: $($Layout.KeyFile) holds a PLAINTEXT key, not a DPAPI blob — re-run -ProvisionSecret")
            }
            $sec = $cipher | ConvertTo-SecureString
            $sec.Dispose()
        } catch {
            $failures.Add("SECRET STORE: $($Layout.KeyFile) does not decrypt as $env:USERNAME on this machine — re-run -ProvisionSecret")
        }
    } else {
        $failures.Add("no DPAPI key store at $($Layout.KeyFile) — run -ProvisionSecret")
    }

    Write-Line "── verify ────────────────────────────────────────────────────"
    if ($failures.Count -eq 0) {
        Write-Line "[OK] all invariants hold across $checked check group(s)" -Colour ok
        return $script:ExitOk
    }
    Write-Line "[RED] $($failures.Count) invariant(s) failed:" -Colour red
    foreach ($f in $failures) { Write-Line "  - $f" -Colour red }
    Write-Fail "verify failed with $($failures.Count) failure(s)"
    return $script:ExitFail
}

# ─────────────────────────────────────────────────────────────────────────────
# Verbs: -Disable / -Enable / -Uninstall
# ─────────────────────────────────────────────────────────────────────────────

function Invoke-ChangeState {
    param([Parameter(Mandatory)][ValidateSet('/DISABLE', '/ENABLE')][string]$Flag)
    $code = Invoke-Native -FilePath 'schtasks.exe' -Arguments @('/Change', '/TN', $TaskName, $Flag)
    if ($code -ne 0) {
        Write-Fail "schtasks /Change $Flag failed (exit $code)"
        return $script:ExitFail
    }
    if (-not $DryRun) {
        Write-Line "task '$TaskName' $(if ($Flag -eq '/DISABLE') { 'disabled' } else { 'enabled' }) (the definition is kept)"
    }
    return $script:ExitOk
}

function Invoke-Uninstall {
    param([Parameter(Mandatory)]$Layout)

    if ($PurgeSecrets -and -not $DryRun) {
        if (-not [Environment]::UserInteractive) {
            Write-Fail "-PurgeSecrets requires an interactive session for its confirmation prompt"
            return $script:ExitFail
        }
        Write-Diag "-PurgeSecrets will DELETE $($Layout.Root), including the DPAPI key store. There is no backup by design."
        $typed = Read-Host -Prompt "Type exactly '$($script:PurgeConfirmation)' to confirm"
        if ($typed -cne $script:PurgeConfirmation) {
            Write-Fail "confirmation did not match; nothing was purged"
            return $script:ExitFail
        }
    }

    $stopCode = Invoke-Stop -Layout $Layout -Quiet
    if ($stopCode -ne $script:ExitOk) {
        Write-Fail "the stop sequence failed; refusing to uninstall while a harness process may still be running"
        return $script:ExitFail
    }

    $code = Invoke-Native -FilePath 'schtasks.exe' -Arguments @('/Delete', '/TN', $TaskName, '/F')
    if ($code -ne 0) {
        Write-Warn "schtasks /Delete returned $code (the task may already be absent)"
    }

    if ($DryRun) {
        Write-Line "DRYRUN would remove: $($Layout.Supervisor)"
        Write-Line "DRYRUN would remove: $($Layout.Manifest)"
        Write-Line "DRYRUN would remove: $($Layout.TaskXml)"
        if ($PurgeSecrets) {
            Write-Line "DRYRUN would remove the ENTIRE install root (including the key store): $($Layout.Root)"
        } else {
            Write-Line "DRYRUN would LEAVE the secret store, harness.config.json and logs in place"
        }
        return $script:ExitOk
    }

    foreach ($f in @($Layout.Supervisor, $Layout.Manifest, $Layout.TaskXml)) {
        if (Test-Path -LiteralPath $f) { Remove-Item -LiteralPath $f -Force }
    }

    if ($PurgeSecrets) {
        Remove-Item -LiteralPath $Layout.Root -Recurse -Force
        Write-Line "uninstalled: task deleted and $($Layout.Root) purged"
    } else {
        # The key store, config and logs deliberately survive: re-installing
        # must not require re-typing the key, and the logs are the only record
        # of why you uninstalled.
        Write-Line "uninstalled: task deleted, supervisor removed. Secret store, config and logs kept in $($Layout.Root)"
        Write-Diag "use -Uninstall -PurgeSecrets to remove the key store as well"
    }
    return $script:ExitOk
}

# ─────────────────────────────────────────────────────────────────────────────
# Dispatch
# ─────────────────────────────────────────────────────────────────────────────

$verbs = [ordered]@{
    ProvisionSecret = $ProvisionSecret
    Install         = $Install
    Start           = $Start
    Stop            = $Stop
    Status          = $Status
    Verify          = $Verify
    Disable         = $Disable
    Enable          = $Enable
    Uninstall       = $Uninstall
}
$selected = @($verbs.Keys | Where-Object { $verbs[$_] })

if ($selected.Count -eq 0) {
    Show-Usage
    exit $script:ExitUsage
}
if ($selected.Count -gt 1) {
    Write-Fail "the verbs are mutually exclusive; got: $($selected -join ', ')"
    Show-Usage
    exit $script:ExitUsage
}
if ($PurgeSecrets -and $selected[0] -ne 'Uninstall') {
    Write-Fail "-PurgeSecrets is only valid with -Uninstall"
    exit $script:ExitUsage
}
if ($RegisterEventSource -and $selected[0] -ne 'Install') {
    Write-Fail "-RegisterEventSource is only valid with -Install"
    exit $script:ExitUsage
}

if (-not $PSBoundParameters.ContainsKey('SourceRoot') -or [string]::IsNullOrWhiteSpace($SourceRoot)) {
    # $PSScriptRoot is <repo>\scripts, so the repo root is its parent.
    $SourceRoot = Split-Path -Parent $PSScriptRoot
}
$sourceScriptRoot = Join-Path $SourceRoot 'scripts'
$layout = Get-Layout -Root $InstallRoot

if ($DryRun) { Write-Diag "DRYRUN — no state will be changed" }

try {
    $exit = switch ($selected[0]) {
        'ProvisionSecret' { Invoke-ProvisionSecret -Layout $layout }
        'Install' { Invoke-Install -Layout $layout -SourceScriptRoot $sourceScriptRoot }
        'Start' { Invoke-Start -Layout $layout }
        'Stop' { Invoke-Stop -Layout $layout }
        'Status' { Invoke-Status -Layout $layout -SourceScriptRoot $sourceScriptRoot }
        'Verify' { Invoke-Verify -Layout $layout -SourceScriptRoot $sourceScriptRoot }
        'Disable' { Invoke-ChangeState -Flag '/DISABLE' }
        'Enable' { Invoke-ChangeState -Flag '/ENABLE' }
        'Uninstall' { Invoke-Uninstall -Layout $layout }
    }
    exit $exit
} catch {
    # Never let a raw exception escape: the message could contain a path a
    # caller would not expect on stdout, and the exit code must stay inside the
    # documented 0/1/2 contract.
    Write-Fail "$($selected[0]) failed: $($_.Exception.Message)"
    Write-Diag "$($_.ScriptStackTrace)"
    exit $script:ExitFail
}
