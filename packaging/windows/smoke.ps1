<#
.SYNOPSIS
    Smoke test for the Windows zip produced by packaging/windows/build-zip.sh.

.DESCRIPTION
    Runs the two things that can only be proven on a real Windows box:

    1. chewvy-cli starts and exits with the codes it documents on a machine with
       no Plex account and no network use: usage error 2 with no arguments,
       success with --version and with status. Status prints a JSON readiness
       report, which also proves the exe's DLL imports all resolved.
    2. chewvy really starts in the *interactive* console session. A plain SSH
       session cannot create a visible window, so the app is started through an
       interactive-only scheduled task (schtasks /it) that runs as the
       console-logged-on user. It has to still be running a few seconds later.

    The script only touches its own task and its own chewvy processes.

    Exit codes: 0 ok, 1 CLI check failed, 2 GUI check failed.

.PARAMETER Root
    Directory holding chewvy.exe, chewvy-cli.exe and libmpv-2.dll.
    Defaults to the directory this script sits in.

.PARAMETER RunAs
    User the interactive task runs as; it must be the account logged on at the
    console, otherwise schtasks /it has no session to run in.

.PARAMETER TaskName
    Name of the scheduled task created and removed by this run.

.PARAMETER StartSeconds
    How long to wait for chewvy to appear after the task runs. A freshly
    unpacked exe is scanned by Defender first, which can take several seconds.

.PARAMETER SoftwareGL
    For a test VM whose display adapter only offers Windows' OpenGL 1.1: Mesa's
    opengl32.dll + libgallium_wgl.dll must have been copied into Root (they are
    never part of the zip), and the app is started with GALLIUM_DRIVER=llvmpipe,
    because Mesa's default d3d12 driver fails on such adapters.
#>
[CmdletBinding()]
param(
    [string]$Root = '',
    [string]$RunAs = 'ci',
    [string]$TaskName = 'chewvy-windows-smoke',
    [int]$AliveSeconds = 5,
    [int]$StartSeconds = 20,
    [switch]$SoftwareGL
)

$ErrorActionPreference = 'Stop'

$ExitCli = 1
$ExitGui = 2

function Write-Step {
    param([string]$Message)
    Write-Output ('smoke: ' + $Message)
}

function Stop-Smoke {
    param([int]$Code, [string]$Message)
    Write-Output ('smoke: FAILED ' + $Message)
    exit $Code
}

# Native commands do not raise PowerShell errors, so their exit code has to be
# collected explicitly; the merged output is kept only to print on failure.
function Invoke-Native {
    param([string]$FileName, [string[]]$Arguments)
    $text = & $FileName @Arguments 2>&1 | Out-String
    [pscustomobject]@{ Exit = $LASTEXITCODE; Text = ('' + $text).Trim() }
}

# System.Diagnostics instead of Start-Process: the argument string is then
# exactly what this script builds, and the streams are read without temp files.
function Invoke-ConsoleExe {
    param([string]$Exe, [string[]]$Arguments = @(), [int]$TimeoutSeconds = 60)
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $Exe
    # Only simple arguments are passed here, so joining with spaces is enough.
    $startInfo.Arguments = ($Arguments -join ' ')
    $startInfo.WorkingDirectory = $Root
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $startInfo.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    $process = [System.Diagnostics.Process]::Start($startInfo)
    try {
        # Reading before waiting keeps a chatty child from blocking on a full pipe.
        $outRead = $process.StandardOutput.ReadToEndAsync()
        $errRead = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            Stop-Smoke $ExitCli ('timed out after ' + $TimeoutSeconds + 's: ' + $Exe)
        }
        [pscustomobject]@{
            Exit = $process.ExitCode
            Out  = [string]$outRead.Result
            Err  = [string]$errRead.Result
        }
    }
    finally {
        $process.Dispose()
    }
}

function Get-ChewvyApp {
    Get-Process -Name 'chewvy' -ErrorAction SilentlyContinue | Select-Object -First 1
}

if (-not $Root) { $Root = $PSScriptRoot }
$appExe = Join-Path $Root 'chewvy.exe'
$cliExe = Join-Path $Root 'chewvy-cli.exe'
foreach ($needed in @($cliExe, $appExe, (Join-Path $Root 'libmpv-2.dll'))) {
    if (-not (Test-Path -LiteralPath $needed)) {
        Stop-Smoke $ExitCli ('no ' + $needed + ' - unpack the zip and run smoke.ps1 from that directory')
    }
}

Write-Output ('smoke: PowerShell ' + $PSVersionTable.PSVersion + ' root=' + $Root)

# A chewvy left over from an earlier run would make "still alive" meaningless.
$leftover = Get-ChewvyApp
if ($leftover) {
    Write-Step ('ending leftover chewvy pid ' + $leftover.Id)
    Stop-Process -Id $leftover.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
}

try {
    $noArgs = Invoke-ConsoleExe -Exe $cliExe -Arguments @()
    if ($noArgs.Exit -ne 2) {
        Stop-Smoke $ExitCli ('chewvy-cli without arguments exited ' + $noArgs.Exit +
            ', expected 2 (usage). stderr: ' + $noArgs.Err.Trim())
    }
    Write-Step 'chewvy-cli without arguments exits 2 (usage), so every DLL it imports resolved'

    $version = Invoke-ConsoleExe -Exe $cliExe -Arguments @('--version')
    if ($version.Exit -ne 0) {
        Stop-Smoke $ExitCli ('chewvy-cli --version exited ' + $version.Exit +
            '. stderr: ' + $version.Err.Trim())
    }
    Write-Step ('chewvy-cli --version ok: ' + $version.Out.Trim())

    $status = Invoke-ConsoleExe -Exe $cliExe -Arguments @('status')
    if ($status.Exit -ne 0) {
        Stop-Smoke $ExitCli ('chewvy-cli status exited ' + $status.Exit +
            '. stderr: ' + $status.Err.Trim())
    }
    try {
        $report = $status.Out | ConvertFrom-Json
    }
    catch {
        Stop-Smoke $ExitCli ('chewvy-cli status did not print JSON: ' + $_.Exception.Message)
    }
    if ($report.tool -ne 'chewvy-cli') {
        Stop-Smoke $ExitCli ('unexpected status report: ' + $status.Out.Trim())
    }
    Write-Step ('chewvy-cli status ok without an account or network; native_missing=' +
        ($report.native_missing -join ','))

    # An interactive-only task is the only way to reach the console session from
    # a non-interactive SSH connection, and a "once" schedule needs a start time.
    $taskExe = $appExe
    if ($SoftwareGL) {
        foreach ($dll in @('opengl32.dll', 'libgallium_wgl.dll')) {
            if (-not (Test-Path -LiteralPath (Join-Path $Root $dll))) {
                Stop-Smoke $ExitGui ('-SoftwareGL needs Mesa ' + $dll + ' in ' + $Root)
            }
        }
        # The task cannot carry environment variables, so a wrapper sets one.
        $taskExe = Join-Path $Root 'run-llvmpipe.cmd'
        $wrapper = @('@echo off', 'set GALLIUM_DRIVER=llvmpipe', ('cd /d "' + $Root + '"'), ('"' + $appExe + '"'))
        Set-Content -LiteralPath $taskExe -Value $wrapper -Encoding ASCII
        Write-Step 'software OpenGL: Mesa llvmpipe via run-llvmpipe.cmd'
    }
    $startAt = (Get-Date).AddMinutes(1)
    $startTime = '{0:D2}:{1:D2}' -f $startAt.Hour, $startAt.Minute
    $created = Invoke-Native -FileName 'schtasks' -Arguments @(
        '/create', '/tn', $TaskName, '/tr', $taskExe,
        '/sc', 'once', '/st', $startTime, '/it', '/ru', $RunAs, '/f')
    if ($created.Exit -ne 0) {
        Stop-Smoke $ExitGui ('schtasks /create failed with ' + $created.Exit +
            ': ' + $created.Text)
    }
    Write-Step ('created interactive-only task ' + $TaskName + ' as ' + $RunAs +
        ' for ' + $startTime)

    $ran = Invoke-Native -FileName 'schtasks' -Arguments @('/run', '/tn', $TaskName)
    if ($ran.Exit -ne 0) {
        Stop-Smoke $ExitGui ('schtasks /run failed with ' + $ran.Exit + ': ' + $ran.Text)
    }

    $process = $null
    foreach ($second in 1..$StartSeconds) {
        Start-Sleep -Seconds 1
        $process = Get-ChewvyApp
        if ($process) { break }
    }
    if (-not $process) {
        $state = Invoke-Native -FileName 'schtasks' -Arguments @(
            '/query', '/tn', $TaskName, '/v', '/fo', 'LIST')
        Stop-Smoke $ExitGui ('chewvy did not appear within ' + $StartSeconds +
            's of the task starting. Task state:' +
            [Environment]::NewLine + $state.Text)
    }
    Write-Step ('chewvy started as pid ' + $process.Id + ', watching it for ' +
        $AliveSeconds + 's')

    Start-Sleep -Seconds $AliveSeconds
    $alive = Get-Process -Id $process.Id -ErrorAction SilentlyContinue
    if (-not $alive) {
        Stop-Smoke $ExitGui ('chewvy exited within ' + $AliveSeconds +
            's of starting in the interactive session')
    }
    # Cross-session window handles are best effort: report them, do not fail on them.
    $windowState = if ($alive.MainWindowHandle -ne 0) { 'has a top-level window' } else { 'no window handle seen' }
    Write-Step ('chewvy still alive after ' + $AliveSeconds + 's and ' + $windowState)
    Write-Output 'smoke: OK - chewvy-cli runs offline, chewvy starts in the console session'
    exit 0
}
finally {
    $running = Get-ChewvyApp
    if ($running) {
        Stop-Process -Id $running.Id -Force -ErrorAction SilentlyContinue
        Write-Step ('ended chewvy pid ' + $running.Id)
    }
    $deleted = Invoke-Native -FileName 'schtasks' -Arguments @('/delete', '/tn', $TaskName, '/f')
    Write-Step ('deleted task ' + $TaskName + ' (schtasks /delete exited ' + $deleted.Exit + ')')
}
