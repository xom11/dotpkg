<#
    idle-baseline.ps1 -- measure what "idle" costs on THIS machine, so that
    scripts/idle-gate.ps1's thresholds are derived rather than guessed.

    The gate's defaults were set from this script's output on a14 on 2026-08-12
    (8 logical cores): machine-wide busy 3.26 / 3.02 / 2.85 percent across three
    consecutive 6 s windows, largest single process dwm at 7.5 / 5.5 / 7.0 percent
    of one core. Run this on any other machine before trusting those numbers
    there, and record what it printed next to whatever the gate then admitted.

    It also prints Win32_Processor.LoadPercentage per round, which is how the
    first version of the gate was miscalibrated: on the idle machine above it read
    20, 16 and 6 across the three rounds. Seeing that swing next to a steady 3
    percent is the whole reason the gate decides on the delta and not on the load.

    NOTE ON STYLE: no backtick appears anywhere in this file, including in
    comments -- see the same note in idle-gate.ps1.
#>

[CmdletBinding()]
param(
    [int]$Rounds = 3,
    [int]$WindowSeconds = 6,
    [int]$Top = 6
)

$ErrorActionPreference = 'Stop'

$cores = [int](Get-CimInstance -ClassName Win32_ComputerSystem).NumberOfLogicalProcessors
if ($cores -lt 1) { $cores = 1 }
Write-Host ('host           : ' + $env:COMPUTERNAME)
Write-Host ('logical_cores  : ' + $cores)
Write-Host ('rounds         : ' + $Rounds + ' x ' + $WindowSeconds + ' s')

$busies = @()

foreach ($round in 1..$Rounds) {
    $t0 = Get-Date
    $snap = @{}
    foreach ($p in Get-Process) {
        if ($null -ne $p.CPU) { $snap[$p.Id] = $p.CPU }
    }

    Start-Sleep -Seconds $WindowSeconds

    $t1 = Get-Date
    $elapsed = ($t1 - $t0).TotalSeconds
    if ($elapsed -le 0) { $elapsed = 1 }

    $rows = @()
    $total = 0.0
    foreach ($p in Get-Process) {
        if ($null -eq $p.CPU) { continue }
        $before = 0.0
        if ($snap.ContainsKey($p.Id)) { $before = $snap[$p.Id] }
        $delta = $p.CPU - $before
        if ($delta -le 0) { continue }
        $total += $delta
        $rows += [pscustomobject]@{
            Name    = $p.ProcessName
            CpuSec  = [math]::Round($delta, 3)
            CorePct = [math]::Round(100.0 * $delta / $elapsed, 1)
        }
    }

    $busy = [math]::Round(100.0 * $total / ($elapsed * $cores), 2)
    $busies += $busy

    $loads = @()
    foreach ($cpu in (Get-CimInstance -ClassName Win32_Processor)) {
        if ($null -ne $cpu.LoadPercentage) { $loads += [int]$cpu.LoadPercentage }
    }

    Write-Host ('--- round ' + $round + '  window=' + [math]::Round($elapsed, 2) + 's ---')
    Write-Host ('  machine_busy_pct     : ' + $busy + '   (total ' + [math]::Round($total, 2) + ' cpu-s over ' + $cores + ' cores)')
    Write-Host ('  loadpercentage_seen  : ' + ($loads -join ','))
    foreach ($r in ($rows | Sort-Object -Property CpuSec -Descending | Select-Object -First $Top)) {
        Write-Host ('  ' + $r.Name + ' ' + $r.CpuSec + ' cpu-s = ' + $r.CorePct + '% of one core')
    }
}

$min = ($busies | Measure-Object -Minimum).Minimum
$max = ($busies | Measure-Object -Maximum).Maximum
Write-Host ''
Write-Host ('machine_busy_pct range : ' + $min + ' .. ' + $max)
Write-Host 'Set idle-gate.ps1 -MaxMachineBusyPercent to roughly three times the max above.'
