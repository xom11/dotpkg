<#
    idle-gate.ps1 -- refuse to start a long measurement on a machine that is not idle.

    This project's standing rule says cargo mutants runs "on an idle machine with
    nothing editing the tree". The 2026-08-12 residual round proved the second
    half (a hashed whole-tree manifest) and never checked the first: a separate
    session was compiling and taking screenshots on the same 8 cores for part of
    the run, and that was found during cleanup rather than during the round. A
    precondition that is only ever asserted in prose is not a precondition.

    This samples the process table twice, decides on the CPU-seconds burned
    BETWEEN the samples, prints every number the verdict rests on so it can be
    recorded beside the result, and exits non-zero when the machine is not quiet.
    Exit 0 means the caller may proceed; exit 1 means it must not.

    WHY THE THRESHOLDS ARE THESE NUMBERS. They were measured on a14, not chosen.
    Three consecutive 6 s windows on the idle machine (2026-08-12, 8 logical
    cores) gave machine-wide busy of 3.26 / 3.02 / 2.85 percent, with the largest
    single process dwm at 7.5 / 5.5 / 7.0 percent of one core and nothing else
    above 2. The defaults below sit at roughly three times that noise floor:
    10 percent machine-wide, 35 percent of one core per process. One rustc on
    eight cores is 12.5 percent machine-wide at minimum, so the gap is real in
    both directions. Re-derive them with scripts/idle-baseline.ps1 on any other
    machine before trusting them there.

    WHAT IS DELIBERATELY NOT USED. Win32_Processor.LoadPercentage is an
    instantaneous sample, and on the same idle machine the three rounds above read
    20, 16 and 6 percent. A gate keyed to it would refuse or admit the same
    machine depending on which second it looked. It is printed, because it is what
    a human would have looked at, and it is not part of the verdict.

    NOTE ON STYLE: no backtick appears anywhere in this file, including in
    comments. A backtick inside a comment is not a parse error, so a parse-check
    passes a file a backtick-check would fail; both gates exist and both must run.
#>

[CmdletBinding()]
param(
    # Seconds between the two process-table samples. The CPU-seconds delta across
    # this window is what separates a process that is running from one that is
    # merely resident.
    [int]$WindowSeconds = 6,

    # Machine-wide busy percentage, above which the machine is not idle.
    # Idle baseline measured at 2.85-3.26 on a14.
    [double]$MaxMachineBusyPercent = 10.0,

    # Per-process share of ONE core, above which that process is working rather
    # than ticking over. Idle maximum measured at 7.5 (dwm) on a14.
    [double]$MaxProcessCorePercent = 35.0,

    # Process names allowed to be alive and to burn CPU. Matched case-insensitively
    # as a prefix. On a14 this is kanata, which this project must never stop.
    [string[]]$Expect = @(),

    # Print the sample and the verdict but always exit 0. For recording the state
    # of a machine beside a result that is not gated on it.
    [switch]$ReportOnly
)

$ErrorActionPreference = 'Stop'

# Names that ONLY exist while something is being compiled or tested. Presence
# alone is disqualifying for these, whether or not the process is burning CPU
# this instant: a linker between two translation units is idle for a moment and
# the machine is still in use.
#
# node, python and their kin were in this list and have been REMOVED, for a
# measured reason: on a developer machine the editor session itself runs node,
# and the Unix half of this gate refused an otherwise-quiet machine over it. A
# long-lived runtime that happens to be resident is not a build. Those are left
# to the CPU threshold, which is the signal that actually distinguishes them.
$BuilderNames = @(
    'cargo', 'rustc', 'rustdoc', 'cc1', 'cc1plus', 'cl', 'link', 'lld', 'ld',
    'msbuild', 'ninja', 'make', 'cmake', 'clang', 'gcc', 'devenv', 'vctip',
    'mspdbsrv', 'cargo-mutants'
)

function Test-Allowed {
    param([string]$Name)
    foreach ($e in $Expect) {
        if ($Name -like ($e + '*')) { return $true }
    }
    return $false
}

$cores = [int](Get-CimInstance -ClassName Win32_ComputerSystem).NumberOfLogicalProcessors
if ($cores -lt 1) { $cores = 1 }

# --- sample 1 -------------------------------------------------------------
$t0 = Get-Date
$snap1 = @{}
foreach ($p in Get-Process) {
    if ($null -ne $p.CPU) { $snap1[$p.Id] = $p.CPU }
}

Start-Sleep -Seconds $WindowSeconds

# --- sample 2 -------------------------------------------------------------
$t1 = Get-Date
$elapsed = ($t1 - $t0).TotalSeconds
if ($elapsed -le 0) { $elapsed = 1 }
$procs = @(Get-Process)

# --- what burned CPU across the window -----------------------------------
$totalCpu = 0.0
$burners = @()
$builders = @()

foreach ($p in $procs) {
    if ($p.Id -eq $PID) { continue }
    $name = $p.ProcessName

    foreach ($b in $BuilderNames) {
        if ($name -ieq $b) {
            if (-not (Test-Allowed -Name $name)) {
                $builders += [pscustomobject]@{ Name = $name; Id = $p.Id }
            }
            break
        }
    }

    if ($null -eq $p.CPU) { continue }
    $before = 0.0
    if ($snap1.ContainsKey($p.Id)) { $before = $snap1[$p.Id] }
    $delta = $p.CPU - $before
    if ($delta -le 0) { continue }
    $totalCpu += $delta

    $corePct = 100.0 * $delta / $elapsed
    if ($corePct -gt $MaxProcessCorePercent) {
        $burners += [pscustomobject]@{
            Name    = $name
            Id      = $p.Id
            CpuSec  = [math]::Round($delta, 3)
            CorePct = [math]::Round($corePct, 1)
            Allowed = (Test-Allowed -Name $name)
        }
    }
}

$machineBusy = [math]::Round(100.0 * $totalCpu / ($elapsed * $cores), 2)
$blocking = @($burners | Where-Object { -not $_.Allowed })

# Recorded, never decided on -- see the header.
$loads = @()
foreach ($cpu in (Get-CimInstance -ClassName Win32_Processor)) {
    if ($null -ne $cpu.LoadPercentage) { $loads += [int]$cpu.LoadPercentage }
}

# --- report ---------------------------------------------------------------
Write-Host '--- idle-gate ---'
Write-Host ('sampled_at          : ' + $t1.ToString('yyyy-MM-ddTHH:mm:sszzz'))
Write-Host ('window_seconds      : ' + [math]::Round($elapsed, 2))
Write-Host ('logical_cores       : ' + $cores)
Write-Host ('processes_total     : ' + $procs.Count)
Write-Host ('machine_busy_pct    : ' + $machineBusy + '   (total ' + [math]::Round($totalCpu, 2) + ' cpu-s)')
Write-Host ('loadpercentage_seen : ' + ($loads -join ',') + '   (recorded, not decided on)')
Write-Host ('expect_allowed      : ' + $(if ($Expect.Count) { $Expect -join ',' } else { '<none>' }))
Write-Host ('threshold_machine   : ' + $MaxMachineBusyPercent + ' %')
Write-Host ('threshold_process   : ' + $MaxProcessCorePercent + ' % of one core')
Write-Host ('burners_over_thresh : ' + $burners.Count)
foreach ($b in ($burners | Sort-Object -Property CorePct -Descending)) {
    $tag = if ($b.Allowed) { 'allowed ' } else { 'BLOCKING' }
    Write-Host ('  ' + $tag + ' ' + $b.Name + ' (pid ' + $b.Id + ') ' + $b.CpuSec + ' cpu-s = ' + $b.CorePct + '% of one core')
}
Write-Host ('builders_alive      : ' + $builders.Count)
foreach ($b in $builders) {
    Write-Host ('  BLOCKING ' + $b.Name + ' (pid ' + $b.Id + ')')
}

# --- verdict --------------------------------------------------------------
$reasons = @()
if ($machineBusy -gt $MaxMachineBusyPercent) {
    $reasons += ('machine busy ' + $machineBusy + '% exceeds ' + $MaxMachineBusyPercent + '%')
}
if ($blocking.Count -gt 0) {
    $names = ($blocking | ForEach-Object { $_.Name }) -join ','
    $reasons += ($blocking.Count.ToString() + ' process(es) working: ' + $names)
}
if ($builders.Count -gt 0) {
    $names = ($builders | ForEach-Object { $_.Name }) -join ','
    $reasons += ($builders.Count.ToString() + ' build/test process(es) alive: ' + $names)
}

Write-Host ''
if ($reasons.Count -gt 0) {
    Write-Host ('VERDICT: NOT IDLE -- ' + ($reasons -join '; '))
    if ($ReportOnly) {
        Write-Host 'idle_gate=REFUSE (report-only, not enforced)'
        exit 0
    }
    Write-Host 'idle_gate=REFUSE'
    exit 1
}

Write-Host 'VERDICT: IDLE'
Write-Host 'idle_gate=OK'
exit 0
