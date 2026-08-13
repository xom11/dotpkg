# Phase 14b — the first winget mutation outside Phase 4b, and a refusal that held

**Machine:** `zenbook-a14`, ARM64, winget **1.29.280**, elevated ssh session
(`IsInRole(Administrator)` = **True**).
**Date:** 2026-08-13, round 09:45:41–09:45:52 plus follow-up probes.
**Tree:** `main` at `589c14c`, shipped as `git archive`, sha256
`3088f12629d17f665b1957f9ff7a1c5cdb5b684dca327f28a074b9ac23a64993`.
**Binary:** built on the machine, release, 85.5 s, sha256
`e4c739a36fc133cfb8c0d7e3c902652d004875b21d92c82b2bd10298b8e1d275`.
**Idle gate before the mutation, as the standing rule requires:** `VERDICT:
IDLE`, `machine_busy_pct 3.06`, 0 burners over threshold, 0 builders alive,
8 logical cores, 6.07 s window.
**Package:** `ducaale.xh`, Phase 4b's disposable, confirmed absent at round
start by dotpkg's own scan.
**Frozen record.** Do not edit.

## 1. A real winget install, performed by dotpkg, for an unpinned package

`dotpkg apply --yes` with `ducaale.xh` declared `pin = "none"` and an empty
lock:

```
  + winget ducaale.xh     -                        (install, unpinned -- whatever winget's index has now)
  1 change(s), 0 skipped, 61 unmanaged
  ready   winget ducaale.xh     0.26.2            (install, unpinned)
  1 of 1 changes ready, 0 failed, 0 skipped, 0 not locked, 61 unmanaged.
  done    winget ducaale.xh     verified on disk
  1 verified on disk, 0 failed, 0 held.
  [exit 0]
```

Three things checked immediately after, each because it is a distinct claim:

- **`pkg.lock`: no file written at all.** Not an empty entry, not a stub — the
  path does not exist. An unpinned declaration resolves to nothing.
- **`state.json`: `{"winget": {"ducaale.xh": "installed"}}`.** Ownership *is*
  recorded, which is what makes the package prunable later.
- **A fresh scan finds it**: `? winget ducaale.xh 0.26.2`.

Then, declared and unpinned and now present, `status` prints **no line for it**
and `apply --prepare` reports `0 change(s)` / `0 of 0 changes ready`, exit 0.
Converged, silently, as designed.

**This closes the install half of `docs/OPEN-ITEMS.md` item 29.** Before this
round, no winget mutation had run anywhere outside the Phase 4b rounds and
their own trees.

## 2. The removal was refused, correctly, and winget agrees

`dotpkg apply --yes --allow-prune` planned and prepared the prune
(`ready winget ducaale.xh 0.26.2 (prune)`) and then refused at **exit 2**.

The refusal is `refuse_elevated_winget_removal`: the session is elevated and
the package is user-scope. Both inputs measured directly, through the real
`winget.exe` at
`C:\Program Files\WindowsApps\Microsoft.DesktopAppInstaller_1.29.280.0_arm64__8wekyb3d8bbwe\winget.exe`
(the `WindowsApps` alias is the 0-byte stub this project already records):

```
winget list -e --id ducaale.xh --scope user      -> exit 0, row present
winget list -e --id ducaale.xh --scope machine   -> "No installed package found", 0x8A150014
```

**And winget itself refuses the same uninstall**, from the same elevated
session:

```
Found xh [ducaale.xh]
The package installed for user scope cannot be uninstalled when running with
administrator privileges.
exit -1978335107   (0x8A15007D)
```

That is `winget_exec::CANNOT_UNINSTALL_ELEVATED`, the constant dotpkg encodes.
**The pre-check and the behaviour it predicts were observed agreeing on real
hardware** — dotpkg refused before attempting, and the attempt would have
failed with exactly the code it names.

**So the removal half of item 29 is still open.** `WingetStep::Remove` has
still never run outside Phase 4b. It was refused by design, not skipped.

## 3. A defect that was not one — recorded because it was nearly written down

The refused run was first read as *"exit 2 and no message at all"*, and that was
about to be recorded as a defect: a refusal with no explanation. It is false.
`Select-String` against the run's own output file found every distinctive
substring of the refusal sentence:

```
bytes: 3208
contains "elevated"          : True
contains "8A15007D"          : True
contains "user scope"        : True
contains "without elevation" : True
```

The message was always there. What produced the wrong reading was the
*display*: stderr is unbuffered and stdout is block-buffered when redirected, so
the two streams do not interleave in source order, and a `Get-Content |
ForEach-Object` dump plus a `tail` on the ssh side did not show the line where a
reader would look for it. The control that settled it — omitting
`--allow-prune`, whose refusal has its own sentence — printed cleanly, proving
`refuse()` works in this environment before any conclusion was drawn about the
other path.

This is the project's own rule earning itself again: **a refutation needs
measuring as carefully as an assertion.** Withdrawing a correct claim about
`refuse()` on a broken probe would have been worse than never probing it.

## 4. A fourth de-elevation route, and this one works

`scripts/nonelevated-mutants.ps1` records three routes measured **not** to
de-elevate from this session: `runas /trustlevel:0x20000`, `schtasks /RL
LIMITED`, and `Shell.Application`. A fourth does work:

```
gsudo -i Medium <a .cmd file>     (gsudo 2.6.1, C:\Program Files\gsudo\Current\gsudo.exe)
```

Run that way, `winget uninstall -e --id ducaale.xh` printed `Found xh
[ducaale.xh]` / `Starting package uninstall...` / `Successfully uninstalled`,
and the package is gone.

Two caveats. The command must be in a **`.cmd` file**: passing it as gsudo
arguments routes it through PowerShell, which parses `uninstall` as an
expression and fails with `UnexpectedToken` before winget is reached. And this
removal was performed by **winget directly, not by dotpkg**, so it does not
close item 29's removal half — it only restored the machine.

It does mean a future round can close that half: run `dotpkg apply --yes
--allow-prune` itself under `gsudo -i Medium`, where the elevation pre-check
will not fire.

## 5. Machine left as found

Round-start state re-established and verified by dotpkg's own scan: `ducaale.xh`
absent. `%LOCALAPPDATA%\dotpkg` holds exactly the three entries it held before
(`manifests`, `state.json`, `state.json.bak`), all with pre-round timestamps —
the `recover.cmd` the install wrote was removed by that run's own successful
completion. `scoop\cache`: nothing newer than round start. Fifteen `ph14b-*`
artefacts removed; sweep of `C:\Users\kln` for anything newer than 09:45:00
returns **none**, and no `ph14*` name remains.

The real `pkg.toml` — a symlink into the nix repo — was never read or written;
every run used its own `--config`, `--lock` and `--state`.

`kanata` was pid **11040** before and pid **11040** after. Never started or
stopped.
