# Measurements: winget, and the four design claims it overturns

Measured 2026-08-09 on a14 (`100.83.225.100`), over `ssh`, at medium
integrity. This document is the raw record behind
`docs/specs/2026-08-09-phase4-backend-winget-design.md`.

```
winget            v1.29.280   (Package: Microsoft.DesktopAppInstaller v1.29.280.0)
Windows           Windows.Desktop v10.0.26200.8973  ([Environment]::OSVersion = 10.0.26200.0)
Architecture      Arm64
PowerShell        5.1.26100.8972
Culture           en-US
winget.exe        C:\Users\kln\AppData\Local\Microsoft\WindowsApps\winget.exe
Sources           msstore  https://storeedgefd.dsx.mp.microsoft.com/v9.0   explicit=false
                  winget   https://cdn.winget.microsoft.com/cache          explicit=false
                  winget-font  https://cdn.winget.microsoft.com/fonts      explicit=true
```

Four rounds, in this order:

| Round | What it is | Streams |
|---|---|---|
| `probe1` | Read-only reconnaissance: formats, exit codes, timings, help text | merged (`2>&1 \| Out-String`) |
| `probe2` | Positive **and** negative controls, determinism, console width, export | **separated**, `Start-Process -RedirectStandard*` |
| `probe3` | Resolve path, pin liveness, the `>` prefix, id case, `source update` | separated |
| `probe4` | Whether `--exact` is what makes `--id` case-sensitive | separated |

**Exit codes in `probe2`–`probe4` were read from
`Start-Process -PassThru`'s `.ExitCode` after `-Wait`**, which
`docs/measurements-2026-08-08-scoop-exit-codes.md` records as unreliable when
read too early. Here it is read after the process has been waited on, and every
value is a plausible winget or Win32 code rather than blank. `probe1` used
`$LASTEXITCODE`, and where the two rounds overlap they agree
(`list -e --id <absent>` = `-1978335212` in both).

---

## The headline

**winget reports failure through its exit code — the opposite of scoop — but
the code is a function of the *filter shape*, not of the output.**

| argv | exit | stdout |
|---|---|---|
| `list -e --id ajeetdsouza.zoxide` | `0` | one row |
| `list -e --id Xyzzy.NoSuch.Dotpkg` | `-1978335212` = `0x8A150014` | `No installed package found matching input criteria.` |
| `list -s msstore` | **`0`** | `No installed package found matching input criteria.` |
| `show -e --id Xyzzy.NoSuch.Dotpkg` | `0x8A150014` | `No package found matching input criteria.` |
| `show -e --id Git.Git -v 2.30.0` | `-1978335209` = `0x8A150017` | `No version found matching: 2.30.0` |
| `export -o Z:\nope\dp.json` | `-2147024893` = `0x80070003` | 6548 bytes, ending `The system cannot find the path specified.` |

The `list -s msstore` row is the trap. **Byte-for-byte the same 53-byte
sentence as the `0x8A150014` row above it, and it exits `0`.** So the rule is
not "trust winget's exit code"; it is **"trust winget's exit code for the exact
argv shapes dotpkg pins"**, and those shapes have to be pinned by tests.

**Everything goes to stdout. stderr was 0 bytes in every one of the ~45
invocations across `probe2`–`probe4`, including every failing one** — including
the `0x80070003` export failure, whose error text is on stdout. A backend that
reads stderr for errors reads nothing.

---

## 1. `Id` is not unique, and two duplicates carry different versions

`winget list --disable-interactivity`, parsed by column offsets read from its
own header row:

```
rows                140
distinct Id         125
duplicated Ids        8   (up to x4)
by Source           winget: 57      (none): 83
non-winget shapes   MSIX\...: 75    ARP\...: 8
```

The eight, with what each carries:

| Id | rows | versions |
|---|---|---|
| `7zip.7zip` | 2 | **`26.01.00.0`** and **`26.02`** |
| `Microsoft.WindowsAppRuntime.2` | 4 | **`2.3.1.0` x3** and **`2.2.0.0`** |
| `Microsoft.UI.Xaml.2.8` | 2 | **`8.2511.26001.0`** and **`8.2501.31001.0`** |
| `Microsoft.DotNet.Native.Runtime` | 3 | `2.2.28604.0` x3 |
| `Microsoft.VCLibs.Desktop.14` | 3 | `14.0.33728.0` x3 |
| `Microsoft.VCLibs.14` | 3 | `14.0.33519.0` x3 |
| `Microsoft.WindowsAppRuntime.1.7` | 3 | `1.7.9` x3 |
| `Microsoft.WindowsAppRuntime.1.8` | 3 | `> 1.8.9` x3 |

Confirmed against a filtered query, so it is not an artifact of the full-table
render:

```
$ winget list -e --id 7zip.7zip --disable-interactivity          EXIT 0
Name                      Id        Version    Source
7-Zip 26.01 (x64 edition) 7zip.7zip 26.01.00.0 winget
7-Zip 26.02 (x64)         7zip.7zip 26.02      winget
```

`src/plan.rs` reads `installed` with `.find(...)` for the declared loop (takes
the first, silently) and with a full iteration for the undeclared loop (would
emit **two** `Prune` or two `Unmanaged` lines for one package). The
"at most one `Installed` per (backend, name)" invariant has never been written
down and is false for the second backend.

### The 83 sourceless rows are a second cause of the Phase 3 scan-integrity hole

`ARP\User\Arm64\efad722a6fc0ee06b7d8ab418af717ec` (name `Claude`),
`ARP\User\Arm64\Look`, `MSIX\B9ECED6F.Glidex_4.2.1.0_x64__qmba6cd70vzyy`. The
`ARP\` id format is not stable across entries — one is a hash, one is a name.

These are installed and **not resolvable against any source**. A declared
package in that state is installed-but-not-comparable, which `plan()` today
would read as *not installed* and turn into `Install` — the identical shape of
`docs/phase3-notes.md` "Still open" item 11, reached by a different route.

---

## 2. `Version` is not always a version, and the cause is not what it looks like

Three shapes that are not dotted numerals:

```
Microsoft.VisualStudio.2022.BuildTools    > 17.14.37
Microsoft.WindowsAppRuntime.1.8           > 1.8.9
Warp.Warp                                 v0.2026.07.15.08.55.stable_01
```

The `> ` prefix survives into JSON, so it is not a table-rendering artifact:

```json
{ "PackageIdentifier" : "Microsoft.VisualStudio.2022.BuildTools",
  "Version" : "> 17.14.37" }
```

**It is not "several versions installed".** Measured:

```
list -e --id Microsoft.VisualStudio.2022.BuildTools                 EXIT 0, ONE row, "> 17.14.37"
list -e --id Microsoft.VisualStudio.2022.BuildTools --scope machine EXIT 0, ONE row, "> 17.14.37"
list -e --id Microsoft.VisualStudio.2022.BuildTools --scope user    EXIT 0x8A150014
show -e --id Microsoft.VisualStudio.2022.BuildTools --versions      newest = 17.14.37
```

One machine-scoped install, one row. `> ` is winget saying *at least*, for a
package whose exact installed version it cannot determine.

**Consequence, derived from `src/plan.rs` rather than from winget** — the
reliable direction per `docs/phase3-notes.md`'s first pattern. `cur.version ==
want` is `"> 17.14.37" == "17.14.37"`, false. `is_older` splits on
non-digits, so both sides reduce to `[17, 14, 37]` and `pa < pb` is false.
The remaining arm is `Downgrade`. So a declared `Microsoft.VisualStudio.2022
.BuildTools` pinned at the version winget itself reports as newest would print
a **`↓` arrow on every run, forever**, and `apply --yes` would act on it.

---

## 3. `--exact` is what makes `--id` case-sensitive

`src/model.rs`'s doc comment states *"Scoop and winget both resolve names
case-insensitively."* For `--exact`, that is false.

| argv | exit | stdout |
|---|---|---|
| `show -e --id Git.Git` | `0` | `Found Git [Git.Git]` |
| `show -e --id git.git` | **`0x8A150014`** | `No package found matching input criteria.` |
| `show -e git.git` (positional) | **`0x8A150014`** | `No package found matching input criteria.` |
| `show --id git.git` (**no** `-e`) | `0` | **`Found Git [Git.Git]`** |
| `show --id Git.Git` (no `-e`) | `0` | `Found Git [Git.Git]` |
| `show git.git` (positional, no `-e`) | `0` | `Found Git [Git.Git]` |
| `list -e --id 7zip.7zip` | `0` | two rows |
| `list -e --id 7ZIP.7ZIP` | **`0x8A150014`** | not found |
| `list --id 7ZIP.7ZIP` (no `-e`) | `0` | two rows |

Two facts fall out:

1. **Any code path that puts `Name::key()` (the folded form) into
   `winget --exact --id` gets `0x8A150014` for a package that exists.** Phase 3
   settled the opposite convention for scoop — the lock records
   `bucket_name.key()`, because scoop opens a directory and Windows folds case.
   winget needs the display spelling, and `Name` is built on the doc comment
   above.
2. **Dropping `--exact` both folds case and hands back the canonical Id** in
   `Found <name> [<Id>]`. That is a single self-verifying call: ask with
   whatever the user wrote, read back what winget matched, record *that*.

This is the same defect Phase 3 closed for buckets ("the lock recorded a
bucket's display spelling while `choose_bucket` opened its folded key"),
pointing the other way.

**Not measured:** what `--id <spelling>` without `--exact` does when the
spelling matches more than one package. winget is documented to print a
disambiguation table; no such case exists on a14.

---

## 4. Version retention is a publisher policy, not a winget guarantee

`docs/specs/2026-08-08-design.md:78` says a winget version is pinnable *"only while that version's
manifest still exists upstream"*. True, and the size of the window was never
measured. `show -e --id <id> --versions`, row counts computed programmatically:

| package | versions in the index | note |
|---|---|---|
| `JanDeDobbeleer.OhMyPosh` | **828** | |
| `Brave.Brave` | **150** | |
| `Git.Git` | **73** | oldest `2.24.1.2` |
| `Obsidian.Obsidian` | **65** | |
| `ajeetdsouza.zoxide` | **11** | `0.9.0` … `0.10.0` |
| `BurntSushi.ripgrep.MSVC` | **8** | |

Three orders of magnitude apart. zoxide and ripgrep have shipped far more than
11 and 8 releases; the index keeps that many.

A version can also be missing from the middle of a run that looks continuous:
`2.30.2`, `2.30.1` and `2.30.0.2` are present, **`2.30.0` is not**.

### An old manifest comes back complete, hash included

```
$ winget show -e --id ajeetdsouza.zoxide -v 0.9.0 --disable-interactivity   EXIT 0
Found zoxide [ajeetdsouza.zoxide]
Version: 0.9.0
Installer:
  Installer Type: portable (zip)
  Installer Url: https://github.com/.../v0.9.0/zoxide-0.9.0-aarch64-pc-windows-msvc.zip
  Installer SHA256: 674b98ef20400d02d1ce5950c83a0fdfa96ea9d720b39d86d828a2111f54e4c5
  Release Date: 2023-01-08
```

**So `docs/specs/2026-08-08-design.md:78`'s "winget pins a version, not a hash" is wrong about
winget.** winget manifests carry a SHA256 and winget verifies it — the same
"URL + hash" shape `docs/specs/2026-08-08-design.md:60` credits scoop with. What winget lacks is a
*local content handle*: a scoop bucket is a git clone on the user's own
machine, so `git show <commit>:bucket/<app>.json` recovers a historical
manifest offline and forever; winget's source is a pre-indexed cache served
from a CDN, with no object database and no commit to name.

The honest statement is therefore about **who holds the manifest**, not about
hashes.

---

## 5. `show -v` is winget's `git cat-file -e`

The pin-liveness check, with two distinguishable failures:

| argv | exit | stdout |
|---|---|---|
| `show -e --id ajeetdsouza.zoxide -v 0.10.0` | `0` | full manifest |
| `show -e --id ajeetdsouza.zoxide -v 0.9.0` | `0` | full manifest (2023-01-08) |
| `show -e --id ajeetdsouza.zoxide -v 0.8.0` | `0x8A150017` | `No version found matching: 0.8.0` |
| `show -e --id ajeetdsouza.zoxide -v 99.99.99` | `0x8A150017` | `No version found matching: 99.99.99` |
| `show -e --id Xyzzy.NoSuch.Dotpkg -v 1.0` | `0x8A150014` | `No package found matching input criteria.` |

`0x8A150017` is exactly the signal `docs/specs/2026-08-08-design.md:311`'s row *"winget version
manifest gone upstream → Report, skip that package"* needs, and the package
-level failure takes precedence over the version-level one.

## 6. `show` agrees with `show --versions` row 1, on all six packages tried

| package | `show`'s `Version:` | `--versions` row 1 |
|---|---|---|
| `Git.Git` | `2.55.0.3` | `2.55.0.3` |
| `ajeetdsouza.zoxide` | `0.10.0` | `0.10.0` |
| `Brave.Brave` | `151.1.93.132` | `151.1.93.132` |
| `JanDeDobbeleer.OhMyPosh` | `30.6.3` | `30.6.3` |
| `Obsidian.Obsidian` | `1.13.4` | `1.13.4` |
| `BurntSushi.ripgrep.MSVC` | `15.2.0` | `15.2.0` |

Six for six. Either is a resolver; `--versions` costs the same and yields the
retention depth as a by-product.

---

## 7. Timings

All warm unless marked. `probe1` used in-process piping; `probe2`/`probe3` used
`Start-Process`, which is what dotpkg will do and is ~0.5 s dearer.

| command | ms |
|---|---|
| `winget list` — **first invocation of the session** | **8125** |
| `winget list` (probe1, in-process, warm) | 624, 607 |
| `winget list` (probe2, `Start-Process`, warm) | **1105, 1117, 1108** |
| `winget export -o … --include-versions` (probe2, warm) | **1073, 1106, 1109** |
| `winget show -e --id <pkg>` (probe3, five runs) | **1055, 1102, 1075, 1121, 1088** |
| `winget source update` | 2097, 2127 |
| `winget source list` | 1106 |
| `winget --info` / `list --help` (probe1, in-process) | 127 / 120 |

`docs/specs/2026-08-08-design.md:257`'s **1213 ms for `winget list`** is confirmed for the warm case.
What the table omits is the **8125 ms first invocation** — and "cached once per
run" does not help the first run, which is the one `dotpkg status` makes.

Resolution at ~1.09 s/package puts 17 declared winget packages at **~18.5 s**,
against the dogfooded 31.5 s for 25 scoop packages with a fetch.

---

## 8. The table is parseable; the JSON is lossy

### `winget list` under redirected stdout

```
Console.IsOutputRedirected  True
Console.WindowWidth         THROWS          RawUI.BufferSize  120,9001
lines 143   max line length 218   contains U+2026 ellipsis: False
```

- **No truncation and no ellipsis** at any width tried.
- **Three consecutive runs byte-identical** (30744 bytes, sha256
  `5c12c7a8945998082a7c0616272cbd3b8ec9580081eb7573383061ea633d4e46`).
- **Console width does not reach it.** `COLUMNS=40` → 30742 bytes; buffer set
  to 200 → 30742 bytes, 143 lines, max 218. *(PowerShell refused to set the
  buffer to 60 or 80 — "size specified is too large or too small" — so a
  genuinely narrow console is **untested**.)*
- Column offsets are recomputed per invocation, and **the column set is
  data-dependent**: the full list has
  `Name(0) Id(64) Version(152) Available(182) Source(212)`, while
  `list -e --id 7zip.7zip` has no `Available` column at all.
- The header row is English and therefore locale-dependent. a14 is `en-US`;
  **no other locale was tested.**

### `winget export --include-versions`

```json
{ "$schema" : "https://aka.ms/winget-packages.schema.2.0.json",
  "CreationDate" : "2026-08-09T19:40:52.703-00:00",
  "Sources" : [ { "Packages" : [ { "PackageIdentifier" : "7zip.7zip",
                                   "Version" : "26.02" }, … ] } ] }
```

Two losses, both measured, both exactly the facts dotpkg must not lose:

1. **It dedupes by Id, silently.** 57 rows with `Source=winget` become
   **42** `PackageIdentifier` entries. The arithmetic is exact:
   `57 − 23 duplicate rows + 8 ids = 42`. For `7zip.7zip` it keeps `26.02` and
   drops `26.01.00.0`.
2. **It drops every package with no source** — 83 of 140 here — printing one
   `Installed package is not available from any source: <Name>` line per drop,
   **by Name, not by Id**, and still exiting `0`.

So `list` strictly dominates `export` in information content. `export` is
useful as an **independent second implementation** to check a `list` parser
against, which is the role the 2b-1 rehearsal script played in Phase 3.

`export` is honest about write failure: `-o Z:\nope\dp.json` exits
`0x80070003` and `Test-Path` afterwards is `False` — no partial file.

---

## 9. `winget source update` is not `git fetch` — it installed something

Phase 3 established that "latest" means fetching. The winget analogue is
`winget source update`. It is **not** the read-only operation `git fetch` is.

`winget list` was captured immediately before the probe and immediately after
it, and the two were parsed and compared field by field:

```
rows before 140      rows after 141
(Name,Id,Version,Source) multiset identical:  False
  ONLY AFTER:  Windows Package Manager Source (winget-font) V2
               MSIX\Microsoft.Winget.Fonts.Source_2025.1016.311.49_neutral__8wekyb3d8bbwe
               2025.1016.311.49   (no source)
Available-column changes: 0
rows lost: 0
```

**No user-facing package was installed, upgraded, removed, or had its
`Available` column move.** The one new row is winget's own source-index MSIX
for `winget-font`, which `source list` shows as `explicit=true` and which bare
`winget source update` refreshed along with the other two.

`winget source update --name winget` exits `0`. **Whether scoping it that way
avoids the MSIX install was not verified** — the before/after diff was not
repeated for the scoped form.

**Repeated, scoped — 2026-08-10.** `winget list` was captured immediately
before and after `winget source update --name winget --disable-interactivity`,
parsed and compared the same way as the bare-form probe above. This run
happened after the bare-form probe had already run once on this machine, so
`winget-font`'s MSIX was already part of the installed set going in, and the
"before" row count reflects that:

```
exit=0
rows before 141      rows after 141
(Name,Id,Version,Source) multiset identical:  True
Available-column changes: 0
rows lost: 0        rows gained: 0
```

**`winget source update --name winget` changed nothing on this machine.** 141
rows in, 141 rows out, the multiset identical, zero `Available`-column moves,
stdout only `Updating source: winget...` / `Done`, stderr empty. The
`winget-font` MSIX row is present exactly once in both captures, unchanged
(`grep -c "Fonts.Source"` = 1 on each) — so the scoped form neither duplicated
it nor removed it.

**What that does and does not establish.** Measured: on a machine already
holding the `winget-font` MSIX, the scoped update touches no installed
package. **Inferred, not measured:** that the scoped form would also avoid
*installing* that MSIX on a machine lacking it. The inference is that
`--name winget` never processes the `winget-font` source at all, which is
consistent with the stdout naming only one source — but the discriminating
experiment (running the scoped form first, on a machine where the MSIX is
absent) cannot be run again here without uninstalling it, which this phase
forbids. Recorded as an inference on purpose: this document's own section on
method notes that reasoning about an external tool's behaviour has been wrong
about half the time in this project.

For Phase 4 that distinction does not bite — `dotpkg update` may run
`winget source update --name winget` unconditionally, because the measured
result is what a dotpkg run would produce on this machine and any machine
that has already been through one.

---

## What was deliberately not measured

- **Every mutating command.** No `install`, `uninstall`, or `upgrade` was run.
  winget has no `$env:SCOOP` equivalent, so there is no throwaway root; every
  write experiment touches the real machine. Scope for Phase 4 was set to
  scan/plan/lock/report, so the measurement that rewrote Phase 2b-2 has **no
  counterpart here, and the winget executor must not be written until it does.**
- **`msstore`-sourced installed packages.** `list -s msstore` returns nothing
  on a14, so every claim above is about `Source=winget` and about sourceless
  ARP/MSIX rows.
- **A non-`en-US` locale**, and a console narrower than 120 columns.
- **`--id` without `--exact` matching more than one package.**
- **`winget pin`.** Its shape was read (`add` / `remove` / `list` / `reset`;
  `pin add -v` accepts a trailing `*` wildcard) and `pin list` reports
  `There are no pins configured.` Nothing was added.

## A note on this document's own method

Round `probe1` measured `list --id` four times and **every one of the four was
the not-found path** — `Git.Git` is not installed by winget on a14, so what
looked like four data points was four samples of one branch. `probe2` exists
because of that: it pairs every negative with a positive using ids that are
actually installed. The exit-code table in "The headline" would otherwise have
recorded `0x8A150014` as winget's answer to `list --id` in general.

An earlier draft of this document also asserted that the `> ` prefix meant
"several versions installed". `probe3` measured it and it does not; the entry
in section 2 is what replaced that guess.
