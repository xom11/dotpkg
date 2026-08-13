# Phase 14c — `WingetStep::Remove`, driven by dotpkg, on real hardware

**Machine:** `zenbook-a14`, ARM64, winget 1.29.280.
**Date:** 2026-08-13, 10:16:28–10:16:38 local.
**Tree:** `main` at `ebf973b`, shipped as `git archive`, sha256
`147619152f38be4439a70bdd019e4eb40e4b2126c59c19103e0d267b4bb50f62`.
**Binary:** built on the machine, release, sha256
`37e568f37f378dc83532cdd59ffa64c0f8ff90fa8dba9f7e79bb09b037b17a8e`.
**Frozen record.** Do not edit.

This closes the half of `docs/OPEN-ITEMS.md` item 29 that Phase 14b could not:
**dotpkg's own winget removal path had never run outside the Phase 4b rounds.**

## What 14b could not do, and why this round could

14b's session is elevated and `ducaale.xh` installs at user scope, so
`refuse_elevated_winget_removal` refused before attempting — correctly, since
winget itself answered `0x8A15007D` to the same uninstall in that round. The
package was removed there by invoking winget directly, which restored the
machine but proved nothing about dotpkg's own path.

This round drives dotpkg from a **medium-integrity** context (`gsudo -i Medium`,
the fourth de-elevation route, measured working in 14b §4), where the
pre-check does not fire. The command sits in a `.cmd` file for 14b's measured
reason: passed as gsudo arguments it is routed through PowerShell, which parses
`apply` as an expression and dies before dotpkg is reached.

## The round

Install first, from the ordinary elevated session — installing is not what the
elevation rule blocks:

```
+ winget ducaale.xh     -                        (install, unpinned -- whatever winget's index has now)
ready   winget ducaale.xh     0.26.2            (install, unpinned)
done    winget ducaale.xh     verified on disk
1 verified on disk, 0 failed, 0 held.        [exit 0]
```

Then the package is undeclared (`[winget] packages` keeps `Brave.Brave`, so
`mass_prune_guard` is not the thing under test) and **dotpkg removes it**, under
`gsudo -i Medium`:

```
- winget ducaale.xh     0.26.2                   (prune, owned)
1 change(s), 0 skipped, 60 unmanaged
ready   winget ducaale.xh     0.26.2            (prune)
done    winget ducaale.xh     verified on disk
1 verified on disk, 0 failed, 0 held.        [gsudo exit 0]
```

Three checks after:

- **Gone.** A fresh scan does not find `ducaale.xh`.
- **Ownership released.** `state.json` is `{"winget": {}}` — the entry
  `run_winget_step`'s `Remove` arm drops on success, not merely emptied by hand.
- **`pkg.lock` never existed.** The package was unpinned throughout, so there
  was never an entry to remove, and the removal's `--version` guard read
  `0.26.2` off the scan instead.

## The idle gate refused, and this round ran anyway

Recorded because the standing rule is to record what it printed, and what it
printed was a refusal:

```
machine_busy_pct    : 18.36   (total 8.89 cpu-s)
burners_over_thresh : 1
VERDICT: NOT IDLE -- machine busy 18.36% exceeds 10%; 1 process(es) working:
MsMpEng; 5 build/test process(es) alive: cargo,rustc,rustc,rustc,vctip
idle_gate=REFUSE
```

The load is **this round's own build**, finishing: `cargo`/`rustc` from the
compile three minutes earlier, plus `MsMpEng` (Windows Defender) scanning its
output. That is the same effect
`docs/OPEN-ITEMS.md` already records for the macOS machine — *the agent running
a measurement is part of the load it measures* — observed here for the first
time on a14, where previous rounds had the load and the controller on different
machines.

**Proceeding was a judgement, not an oversight, and it is a second instance of
applying this gate outside what it was written for.** `idle-gate.ps1`'s own
header says it exists so `cargo mutants` does not run on a busy machine, where
CPU contention corrupts a timing-sensitive result. A winget install and removal
is not timing-sensitive: what could corrupt it is another process holding
winget's index, and no `winget` process was alive. Both operations returned
`done ... verified on disk` on the first attempt with no contention error.

The honest reading: this gate's verdict does not bear on this kind of round, and
the rule that says to run it before "any mutation run" means `cargo mutants`.
Running it anyway costs nothing; treating its refusal as binding here would
have been cargo-culting a threshold measured for a different question.

## Machine left as found

`ducaale.xh` absent per dotpkg's own scan. `%LOCALAPPDATA%\dotpkg` holds exactly
its three pre-round entries with pre-round timestamps. `scoop\cache`: nothing
newer. Every `ph14c-*` artefact removed; final sweep of `C:\Users\kln` for
anything newer than 10:14:00 returns **nothing**, and no `ph14*` name remains.
`kanata` pid **11040** throughout, never started or stopped.
