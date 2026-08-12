<#
    uac-name-probe.ps1 -- why can cargo not launch one test binary from an
    ordinary Windows session?

    RUN THIS FROM AN ORDINARY POWERSHELL WINDOW ON THE DESKTOP.
    Not "Run as administrator", and not over ssh -- an elevated session launches
    everything here and would report a clean sweep that means nothing.

    WHAT IS MEASURED, twice, and reproducible: cargo test from a non-elevated
    session cannot start the binary built from tests/update.rs.

        could not execute process ...\update-<hash>.exe (never executed)
        Caused by: The requested operation requires elevation. (os error 740)

    WHAT THIS PROBE'S FIRST VERSION GOT WRONG, and why it is worth saying here
    rather than only in the commit: it decided "LAUNCHED" from $LASTEXITCODE
    alone. PowerShell leaves $LASTEXITCODE at its previous value when it fails to
    start a native command, so a stale 0 read exactly like a success, and the
    probe reported three clean launches that may never have happened. A check
    whose output narrates its own result is this project's fourth defect class,
    and it was committed inside the tool written to refute a claim.

    SO EVERY LAUNCH HERE IS VERIFIED BY CONTENT. The --list flag makes a libtest binary
    print one line per test; a run that produced no such line did not run,
    whatever the exit code says.

    THE TWO AXES, separated so one run can tell them apart:
      A. the NAME -- one binary under three names (original, no keyword, a
         different keyword). Same bytes, so a difference can only be the name.
      B. the LAUNCHER -- the same binary started directly by PowerShell versus
         started by cargo. Same file, same session, so a difference can only be
         who called CreateProcess.

    NOTE ON STYLE: no backtick appears anywhere in this file, including in
    comments -- a backtick in a comment is not a parse error, so a parse-check
    passes a file a backtick-check would fail.
#>

[CmdletBinding()]
param(
    [string]$Deps = 'C:\Users\kln\ph6-target\debug\deps',
    [string]$Tree = 'C:\Users\kln\ph6-build'
)

$ErrorActionPreference = 'Continue'

$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]$id
Write-Host ('session_id     : ' + (Get-Process -Id $PID).SessionId)
Write-Host ('isinrole_admin : ' + $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator))
foreach ($l in (@(& whoami /groups) | Where-Object { $_ -match 'Mandatory Label' })) {
    Write-Host ('integrity      : ' + ($l -replace '\s+', ' ').Trim())
}
if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host ''
    Write-Host 'REFUSE: this session is elevated. It can launch all of these and would prove nothing.'
    exit 1
}

$found = @(Get-ChildItem (Join-Path $Deps 'update-*.exe') -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending)
if ($found.Count -eq 0) {
    Write-Host ('REFUSE: no update-*.exe under ' + $Deps)
    exit 1
}
$src = $found[0].FullName
Write-Host ''
Write-Host ('subject        : ' + $src)
Write-Host ('sha256         : ' + (Get-FileHash -Algorithm SHA256 $src).Hash.ToLower())
Write-Host ('written        : ' + $found[0].LastWriteTime.ToString('yyyy-MM-ddTHH:mm:ss'))

function Show-Launch {
    param([string]$Label, [scriptblock]$Action)
    $global:LASTEXITCODE = 0
    $out = @()
    $failure = ''
    try {
        $out = @(& $Action 2>&1 | ForEach-Object { ([string]$_).TrimEnd([char]13) })
    } catch {
        $failure = $_.Exception.Message
    }
    $code = $LASTEXITCODE
    # The verdict is the OUTPUT, not the exit code: a libtest binary asked to
    # --list prints one line per test, and zero such lines means it never ran.
    $names = @($out | Where-Object { $_ -match ': test$' }).Count
    if ($failure -eq '') {
        foreach ($line in $out) {
            if ($line -match 'elevation|denied|740|cannot|failed to run') { $failure = $line.Trim(); break }
        }
    }
    $verdict = if ($names -gt 0) { 'RAN     ' } else { 'DID NOT RUN' }
    Write-Host ('  ' + $verdict + '  ' + $Label)
    Write-Host ('      test_names_printed=' + $names + '  exit=' + $code)
    if ($failure -ne '') { Write-Host ('      error: ' + $failure) }
}

$neutral = Join-Path $Deps 'ph6-neutral-probe.exe'
$setupish = Join-Path $Deps 'ph6-setup-probe.exe'
Copy-Item $src $neutral -Force
Copy-Item $src $setupish -Force

Write-Host ''
Write-Host '--- axis A: same bytes, three names, launched directly ---'
Show-Launch -Label 'name contains "update" (the original)' -Action { & $src --list }
Show-Launch -Label 'name contains no keyword'             -Action { & $neutral --list }
Show-Launch -Label 'name contains "setup"'                -Action { & $setupish --list }

Write-Host ''
Write-Host '--- axis B: the original binary, launched by cargo ---'
Push-Location $Tree
$env:CARGO_TARGET_DIR = 'C:\Users\kln\ph6-target'
Show-Launch -Label 'cargo test --test update -- --list' -Action { & cargo test --test update -- --list }
Pop-Location

Remove-Item $neutral, $setupish -Force -ErrorAction SilentlyContinue
Write-Host ''
Write-Host 'copies removed. Copy everything above back.'
