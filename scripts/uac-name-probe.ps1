<#
    uac-name-probe.ps1 -- is the filename really why a test binary cannot be
    launched from an ordinary Windows session?

    RUN THIS FROM AN ORDINARY POWERSHELL WINDOW ON THE DESKTOP.
    Not "Run as administrator", and not over ssh -- an elevated session can
    launch all of these and would report a clean sweep that means nothing.

    WHAT IS ALREADY MEASURED. From a non-elevated session, cargo could not start
    the test binary built from tests/update.rs:

        could not execute process ...\update-<hash>.exe (never executed)
        Caused by: The requested operation requires elevation. (os error 740)

    WHAT IT SETTLED, on 2026-08-12: the explanation was WRONG. All three names
    launched, exit 0, from a session proven non-elevated. UAC installer
    detection -- which flags an executable whose FILENAME contains install,
    setup, update or patch -- is a real behaviour, it fitted the observation
    exactly, and it is not what happened here. Nor is a RUNASADMIN compatibility
    layer (there is none for this binary) nor a zone identifier (0 alternate
    streams). The file that failed launches now, unchanged since before it
    failed. The cause is unknown.

    The script is kept rather than deleted because the question can recur, and
    because a refutation nobody can re-run is just another assertion.

    THE EXPERIMENT. Copy ONE binary to three names and try to launch each. Same
    bytes, same signature, same manifest state -- only the name differs, so the
    name is the only thing a difference in outcome can be about:

      update-<hash>.exe      the original, expected to fail
      ph6-neutral-<hash>.exe a name with no keyword, expected to succeed
      ph6-setup-<hash>.exe   a DIFFERENT keyword, which tests the CLASS rather
                             than the one filename -- if this fails too, the
                             finding generalises; if it succeeds, the
                             explanation is wrong and something specific to
                             "update" is going on

    --list is used as the argument: it prints test names and runs nothing.

    NOTE ON STYLE: no backtick appears anywhere in this file, including in
    comments -- a backtick in a comment is not a parse error, so a parse-check
    passes a file a backtick-check would fail.
#>

[CmdletBinding()]
param([string]$Deps = 'C:\Users\kln\ph6-target\debug\deps')

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
    Write-Host 'REFUSE: this session is elevated. It can launch all three and would prove nothing.'
    exit 1
}

$original = @(Get-ChildItem (Join-Path $Deps 'update-*.exe') -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending)
if ($original.Count -eq 0) {
    Write-Host ('REFUSE: no update-*.exe under ' + $Deps + ' -- build the tests first')
    exit 1
}
$src = $original[0].FullName
Write-Host ''
Write-Host ('subject        : ' + $src)
Write-Host ('sha256         : ' + (Get-FileHash -Algorithm SHA256 $src).Hash.ToLower())

$copies = @(
    @{ Label = 'no keyword     '; Path = (Join-Path $Deps 'ph6-neutral-probe.exe') },
    @{ Label = 'keyword setup  '; Path = (Join-Path $Deps 'ph6-setup-probe.exe') }
)
foreach ($c in $copies) { Copy-Item $src $c.Path -Force }

function Test-Launch {
    param([string]$Label, [string]$Path)
    $out = & $Path --list 2>&1
    $code = $LASTEXITCODE
    $err = ''
    foreach ($line in $out) {
        $s = ([string]$line).TrimEnd([char]13)
        if ($s -match 'elevation|denied|740|Exception') { $err = $s.Trim(); break }
    }
    $verdict = if ($code -eq 0) { 'LAUNCHED' } else { 'REFUSED ' }
    Write-Host ('  ' + $verdict + '  ' + $Label + '  exit=' + $code + '  ' + $err)
}

Write-Host ''
Write-Host '--- same bytes, three names ---'
Test-Launch -Label ('keyword update  (' + (Split-Path $src -Leaf) + ')') -Path $src
foreach ($c in $copies) { Test-Launch -Label $c.Label -Path $c.Path }

foreach ($c in $copies) { Remove-Item $c.Path -Force -ErrorAction SilentlyContinue }
Write-Host ''
Write-Host 'copies removed. Copy everything above back.'
