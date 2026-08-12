<#
    nonelevated-mutants.ps1 -- the run that closes the last mutant sys.rs holds.

    RUN THIS FROM AN ORDINARY POWERSHELL WINDOW ON THE DESKTOP.
    Not "Run as administrator", and not over ssh: an OpenSSH session on the
    machine this was written for is elevated and lives in session 0, and three
    ways of de-elevating from it have been measured not to work (runas
    /trustlevel:0x20000, schtasks /RL LIMITED, and Shell.Application's
    ShellExecute, which comes back at High integrity in session 0).

    WHY A NON-ELEVATED SESSION IS THE ONLY THING THAT CAN DO THIS. On Windows,
    cargo mutants kills elevated -> None, elevated -> Some(false) and the != ->
    == inversion, and cannot kill elevated -> Some(true): in an elevated session
    Some(true) IS the correct answer, so that mutant is genuinely equivalent
    there. From an ordinary session the mirror holds -- Some(true) dies and
    Some(false) becomes the equivalent one -- so the UNION of the two runs kills
    all four and neither run alone can.

    WHY THE LIBTEST FILTER IS NOT OPTIONAL. The repository now has two #[ignore]d
    tests that contradict each other by design: one asserts elevated() ==
    Some(true) and one asserts Some(false). A bare --include-ignored therefore
    fails one of them in EVERY session, and cargo mutants reports "cargo test
    failed in an unmutated tree" and aborts at the baseline. Naming the test is
    what keeps the baseline honest here.

    WHY --test cli IS HERE, and what is NOT known about why it is needed.
    On its first run from an ordinary session the baseline failed because cargo
    could not start one test binary at all:

        could not execute process ...\update-<hash>.exe (never executed)
        Caused by: The requested operation requires elevation. (os error 740)

    That is measured. The explanation first offered for it -- Windows' UAC
    installer detection keying on a filename containing "update" -- is
    MEASURED FALSE: the same binary, copied to a name with no keyword and to a
    name carrying a different keyword ("setup"), launches under all three names
    from a non-elevated session (scripts/uac-name-probe.ps1). Nor is there a
    RUNASADMIN compatibility layer for it, nor a zone-identifier stream.

    And the symptom does not reproduce: that exact file, unchanged since before
    the failure -- its mtime predates it -- now launches from the same kind of
    session. So the cause is UNKNOWN, and this flag is a workaround for a
    failure observed once, not a fix for a mechanism anybody has identified.
    Scoping to the one binary that holds the test under measurement is enough
    for this run either way.

    NOTE ON STYLE: no backtick appears anywhere in this file, including in
    comments. A backtick inside a comment is not a parse error, so a parse-check
    passes a file a backtick-check would fail; both gates exist and both run.
#>

[CmdletBinding()]
param(
    [string]$Tree = 'C:\Users\kln\ph6-build',
    [string]$Out  = 'C:\Users\kln\ph6-mut-nonelev'
)

$ErrorActionPreference = 'Stop'

# --- independent evidence that this session is NOT elevated ---------------
# The test asserts what sys::elevated() returned, which is the thing under
# measurement; taking the session's elevation from that same function would be
# circular. IsInRole and the token's mandatory label are two different APIs from
# the TokenIsElevated call sys::elevated() makes.
$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]$id
Write-Host ('user             : ' + $id.Name)
Write-Host ('session_id       : ' + (Get-Process -Id $PID).SessionId)
Write-Host ('isinrole_admin   : ' + $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator))
$label = @(& whoami /groups) | Where-Object { $_ -match 'Mandatory Label' }
foreach ($l in $label) { Write-Host ('integrity        : ' + ($l -replace '\s+', ' ').Trim()) }

if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host ''
    Write-Host 'REFUSE: this session IS elevated. It cannot produce the observation this run exists for.'
    Write-Host 'Open an ordinary PowerShell window (not "Run as administrator") and try again.'
    exit 1
}

# --- the machine must be idle --------------------------------------------
$gate = 'C:\Users\kln\ph6-idle-gate.ps1'
if (Test-Path $gate) {
    & powershell -NoProfile -NonInteractive -File $gate -Expect kanata | ForEach-Object {
        $s = ([string]$_).TrimEnd([char]13)
        if ($s -match 'machine_busy_pct' -or $s -match '^VERDICT') { Write-Host ('gate             : ' + $s.Trim()) }
    }
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'REFUSE: machine is not idle. Close what is running and try again.'
        exit 1
    }
}

# --- the run --------------------------------------------------------------
Push-Location $Tree
# cargo-mutants manages its own build directories; pointing it at the shared one
# the suite uses makes every mutant contend for a single target dir.
Remove-Item Env:\CARGO_TARGET_DIR -ErrorAction SilentlyContinue
Write-Host ''
Write-Host 'running cargo mutants -- this takes a few minutes, leave the window open'
$sw = [System.Diagnostics.Stopwatch]::StartNew()
# Two separate double-dash boundaries: cargo-test args, then libtest args. The
# single-dash form the help describes does not reach libtest at all.
$mutantArgs = @('-f', 'src/sys.rs', '--re', 'elevated', '-j', '2', '--timeout', '600', '-o', $Out,
    '--', '--test', 'cli',
    '--', '--include-ignored', 'on_an_ordinary_windows_session')
$result = & cargo mutants @mutantArgs 2>&1 | ForEach-Object { ([string]$_).TrimEnd([char]13) }
$code = $LASTEXITCODE
$sw.Stop()
Pop-Location

Write-Host ''
Write-Host ('mutants_exit     : ' + $code + '   seconds: ' + [math]::Round($sw.Elapsed.TotalSeconds, 0))
foreach ($line in $result) {
    if ($line -match 'MISSED|CAUGHT|TIMEOUT|UNVIABLE|mutants tested|Found \d+ mutants|baseline|FAILED') {
        Write-Host ('  ' + $line.Trim())
    }
}
Write-Host ''
Write-Host 'Copy everything above back. A TIMEOUT is the ABSENCE of a verdict, not a verdict.'
