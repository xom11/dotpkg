# Measurements: winget's write path, and the five things it overturns

Measured 2026-08-10 on a14 (`100.83.225.100`), over `ssh`, **at high integrity**
— which turned out to be load-bearing, see §5.

This is the round `docs/specs/2026-08-09-phase4-backend-winget-design.md` names
as the precondition for a winget executor, and that
`docs/measurements-2026-08-09-winget.md` closes with:

> **Every mutating command.** No `install`, `uninstall`, or `upgrade` was run.
> […] the measurement that rewrote Phase 2b-2 has **no counterpart here, and
> the winget executor must not be written until it does.**

```
winget            v1.29.280   (unchanged from the 2026-08-09 round)
Windows           10.0.26200.0, Arm64
winget.exe        C:\Users\kln\AppData\Local\Microsoft\WindowsApps\winget.exe
session identity  ZENBOOK-A14\kln, elevated = True
```

Three rounds:

| Round | What it is | Can it write? |
|---|---|---|
| `W0` | Read-only: pick a guinea pig from data, capture the live process list, probe `--id` without `--exact` | **No** — `show`/`list` only |
| `W1` | Write **verbs**, failure paths only, streams separated | **No** — enforced mechanically, see below |
| `W2` | Write path, positive controls, one guinea pig | **Yes** |

**W1's inability to write was enforced by the script, not promised by a
comment.** Every argv naming `install`/`uninstall`/`upgrade`/`pin` had to also
name a package that does not exist (`Xyzzy.NoSuch.Dotpkg`) or a version that
does not exist; anything else aborted the round. The guard never had to fire,
and `winget list`'s SHA256 was byte-identical before and after all 12 probes.

**W2 was allowlisted to one id.** Every write verb had to name `ducaale.xh` or
the round aborted.

**The machine was returned to its exact starting state**, proven by SHA256 of
`winget list` (`ADAB03E6…` at the start of W2, `ADAB03E6…` after cleanup), 0
leftover `WinGet\Packages` directories, 0 leftover `WinGet\Links` entries, 31
scoop app directories throughout, and `kanata_windows_tty_winIOv2_arm64` at PID
7972 untouched from first probe to last. `C:\Users\kln\pkg.toml` was never
opened for writing: sha256 `32A238FF…`, 449 bytes, identical to the value
Phase 3 and Phase 4 both recorded.

---

## The headline

**winget's write verbs report through the exit code, like its read verbs and
unlike scoop. But a nonzero exit does not mean the machine is wrong, and
`install --version` can only ever move a package *up*.**

Two facts, and the second is the one that decides the executor's shape:

1. `install --version <older>` on an installed package **does nothing** and
   exits `0x8A15002B` with the words *"No available upgrade found."* — a
   message about upgrades, in answer to a request that was not one.
2. The same code `0x8A15002B` is returned when the package is **already at
   exactly the version asked for**. So nonzero cannot be read as "failed".

That is the same class of defect as scoop's, pointing the other way. Scoop
exits `0` and says `WARN … is already installed` while changing nothing; winget
exits nonzero and says "no upgrade available" while changing nothing. **Neither
tool's exit code answers the question dotpkg asks it**, and for winget the
reason is that `install` silently reinterprets itself as `upgrade`.

---

## 1. `--version` is honest on a fresh install

The central question — does `winget install --version X` install X, or silently
the latest? Measured on a package that was **absent** first (verified: `list -e
--id ducaale.xh` → `0x8A150014`):

```
$ winget install -e --id ducaale.xh --version 0.24.1 --silent \
      --accept-package-agreements --accept-source-agreements --disable-interactivity
EXIT 0    8154 ms    stdout 499 bytes    stderr 0 bytes
  Found xh [ducaale.xh] Version 0.24.1
  Downloading .../v0.24.1/xh-v0.24.1-x86_64-pc-windows-msvc.zip
  Successfully verified installer hash
  Extracting archive...
  Command line alias added: "xh"
  Command line alias added: "xhs"
  Successfully installed

$ winget list -e --id ducaale.xh --disable-interactivity
EXIT 0    version 0.24.1    available 0.26.2
```

**0.24.1, not the newest 0.26.2.** `--version` is a target on a fresh install,
and winget verifies the installer hash while doing it (`Successfully verified
installer hash`), which is the "URL + hash" property
`docs/specs/2026-08-09-phase4-backend-winget-design.md` already corrected
`design.md:78` about.

## 2. On an already-installed package, `install` becomes `upgrade` and `--version` becomes a floor

Four measurements, one table. Installed version at the time is in the first
column.

| installed | argv | exit | stdout | result |
|---|---|---|---|---|
| 0.24.1 | `install --version 0.24.1` | `0x8A15002B` | `Found an existing package already installed. Trying to upgrade…` / `No available upgrade found.` | **unchanged** |
| 0.24.1 | `install --version 0.26.1` | `0` | full download + install | **0.26.1** |
| 0.26.1 | `install --version 0.22.0` | `0x8A15002B` | `…Trying to upgrade…` / `No available upgrade found.` | **unchanged** |
| 0.26.2 | `install --version 0.23.0` | `0x8A15002B` | same | **unchanged** |

The `Found an existing package already installed. Trying to upgrade the
installed package...` line appears in W1 too, against real installed packages
(`Obsidian.Obsidian`, `Brave.Brave`), so it is not specific to a portable or to
the guinea pig.

**So a downgrade is unreachable through `install --version`.** dotpkg's
`Action::Downgrade` for winget must be **uninstall then install** — the same
`Step::Replace` shape scoop needs, arrived at for an entirely different reason
(scoop: `install` over an installed app is a no-op; winget: `install` over an
installed app is an *upgrade-only* no-op).

**And `--version` cannot be trusted as a target in the general case.** On a
machine where the package is present at a *higher* version, asking for the
pinned one is silently declined. An executor that fires `install --version
<pin>` and reads exit `0x8A15002B` as "already fine" would be wrong exactly
when the machine is *ahead* of the pin — the measured `Brave.Brave` shape from
`docs/dogfood-phase4-2026-08-10.md`.

## 3. Six exit codes, three of them unknown to this crate

Across all 27 write-verb invocations (12 in W1, 15 in W2):

| hex | decimal | what it means | in `src/backend/winget.rs` today |
|---|---|---|---|
| `0x00000000` | `0` | it did the thing | — |
| `0x8A150014` | `-1978335212` | no package / no *installed* package — see §4 | `NO_APPLICATIONS_FOUND` |
| `0x8A150017` | `-1978335209` | no version matching | `NO_VERSION_FOUND` |
| `0x8A15002B` | `-1978335189` | "no available upgrade" — **including the converged case** | **absent** |
| `0x8A150061` | `-1978335135` | `A package version is already installed. Installation cancelled.` (from `--no-upgrade`) | **absent** |
| `0x8A15007D` | `-1978335107` | user-scope package cannot be uninstalled while elevated — §5 | **absent** |

`0x8A15002B` is the dangerous one, because it is returned both for "nothing to
do, you are already there" and for "I declined what you asked". Those are a
success and a failure sharing one code.

## 4. `0x8A150014` does not distinguish "not in the index" from "not installed"

| argv | exit | stdout | bytes |
|---|---|---|---|
| `install -e --id Xyzzy.NoSuch.Dotpkg` | `0x8A150014` | `No package found matching input criteria.` | 43 |
| `uninstall -e --id Xyzzy.NoSuch.Dotpkg` | `0x8A150014` | `No installed package found matching input criteria.` | 53 |
| `upgrade -e --id Xyzzy.NoSuch.Dotpkg` | `0x8A150014` | `No installed package found matching input criteria.` | 53 |

Two different sentences, **one exit code**. For a `Step::Remove` this matters:
"it was already gone" is the desired end state (a success) and "that id is
wrong" is a failure, and the exit code cannot tell them apart. Only the text
can — which is the same trap `list -s msstore` set on the read side
(`docs/measurements-2026-08-09-winget.md`, "The headline"), reached from the
other direction: there, one sentence with two codes; here, two sentences with
one code.

## 5. The one that has no scoop counterpart: **install succeeds elevated, uninstall then refuses**

```
$ winget uninstall -e --id ducaale.xh --disable-interactivity   [elevated]
EXIT -1978335107   (0x8A15007D)
  Found xh [ducaale.xh]
  The package installed for user scope cannot be uninstalled when running with
  administrator privileges.
```

The install that created it ran in the *same elevated session* and exited `0`.
Repeated: plain `uninstall`, `uninstall` again, and `uninstall --all-versions`
all returned `0x8A15007D`.

Paired positive control, same machine, same package, same argv, one variable
changed — the process's integrity level:

```
inner elevated: False
inner user    : ZENBOOK-A14\kln
uninstall exit: 0
  | Found xh [ducaale.xh]
  | Starting package uninstall...
  | Successfully uninstalled
```

**dotpkg's whole shape is a scheduled `apply`.** An `apply` running at high
integrity can install a user-scope winget package and then be *structurally
unable to remove it* — every prune failing forever, with a real distinct exit
code and a message that names the cause. This is not a transient failure the
next run clears; it is a property of the integrity level the run happens under.

`--force` and `--purge` against this refusal are **unmeasured**: the
de-elevated route succeeded first and the round stopped there.

### Method: how to de-elevate from a14's ssh session

Worth carrying forward, because the first attempt cost a round.

- **`runas /trustlevel:0x20000 "powershell -File <script>"` works.** Verified by
  the inner script printing its own `IsInRole(Administrator) = False`.
- **`schtasks /create … /RL LIMITED` does not**, with or without `/IT`. With
  `/IT` the task sits at `Status: Queued`, `Logon Mode: Interactive only`,
  `Last Result: 0`, and never runs when triggered from an ssh session. The first
  attempt also *deleted the task before querying it*, throwing away the
  diagnosis — don't.

## 6. `--exact` case-sensitivity governs the write verbs too

| argv | exit | stdout |
|---|---|---|
| `install -e --id sharkdp.hyperfine --version <absent>` | `0x8A150017` | `No version found matching: …` |
| `install --id sharkdp.hyperfine --version <absent>` (no `-e`) | `0x8A150017` | same |
| `install -e --id SHARKDP.HYPERFINE --version <absent>` | **`0x8A150014`** | `No package found matching input criteria.` |

The rule `docs/measurements-2026-08-09-winget.md` §3 measured for `show` holds
for `install`: `--exact` is what makes `--id` case-sensitive. And the
**package-level failure takes precedence over the version-level one**, the same
ordering as the read side.

Consequence: a mutating call may use `-e --id` **only** with a spelling winget
itself produced. `pkg.lock`'s key is exactly that (Task 15 writes the canonical
id from `Found <name> [<Id>]`), so the lock is the correct source for a write
argv, and `Name::key()` still must never reach one.

## 7. `--id` does **not** fuzzy-match, with or without `--exact`

`docs/measurements-2026-08-09-winget.md` lists *"`--id` without `--exact`
matching more than one package"* under "deliberately not measured". Measured now,
read-only:

```
show --id 7zip        EXIT 0x8A150014   No package found matching input criteria.
show --id Microsoft   EXIT 0x8A150014   No package found matching input criteria.
show --id ripgrep     EXIT 0x8A150014   No package found matching input criteria.
show --id git         EXIT 0x8A150014   No package found matching input criteria.
show --id zoxide      EXIT 0x8A150014   No package found matching input criteria.
```

Every one of those is a real substring of a real installed id (`7zip.7zip`,
`Git.Git`, `BurntSushi.ripgrep.MSVC`, `ajeetdsouza.zoxide`). **`--id` always
requires the whole id; `--exact` only controls case.** So the design's rule that
every resolution call drops `--exact` carries no ambiguity risk. What *is*
ambiguous for a write verb is an id with two versions installed at once
(`7zip.7zip`), and those already reach `Scan::opaque` and are never acted on.

## 8. `uninstall --version` is a guard, measured twice

| installed | argv | exit | still installed? |
|---|---|---|---|
| 0.26.2 | `uninstall --version 0.23.0` (real index version, not installed) | `0x8A150017` | **yes** |
| 0.26.2 | `uninstall --version <does not exist at all>` | `0x8A150017` | **yes** |
| — (absent) | `uninstall --version <absent>` | `0x8A150014` | — |
| 26.01/26.02 (`7zip.7zip`) | `uninstall --version <absent>` | `0x8A150017` | untouched |

`uninstall --version` resolves against **what is installed**, not against the
index, and refuses rather than removing something else. Passing the pinned
version is therefore a cheap way to make a removal fail closed.

## 9. `--force`, `--no-upgrade`, `upgrade`

```
install --version <current> --force   EXIT 0     2121 ms
    no "Downloading" line; "Successfully verified installer hash" then extract
install --no-upgrade (while installed)  EXIT 0x8A150061
    A package version is already installed. Installation cancelled.
upgrade (from 0.26.1)                 EXIT 0     7154 ms  -> 0.26.2
upgrade (already newest)              EXIT 0x8A15002B      unchanged
```

`--force` re-asserts a version from cache without re-downloading — the only
measured way to make an install idempotent rather than a no-op.

## 10. The `Available` column is winget's own upgrade signal, and dotpkg throws it away

| state | `list -e --id`'s `Available` |
|---|---|
| installed 0.24.1, index newest 0.26.2 | `0.26.2` |
| installed 0.26.1, index newest 0.26.2 | `0.26.2` |
| installed 0.26.2 (newest) | *(column absent)* |

`src/backend/winget.rs`'s `WingetRow.available` is parsed and then never read by
`rows_to_scan`. Measured, it is exactly "what winget itself would upgrade this
to" — one field, already in hand, that answers the question `resolve_latest`
currently spends a ~1.09 s `winget show` per package to answer.

## 11. A winget package can add more command-line aliases than its name

```
Command line alias added: "xh"
Command line alias added: "xhs"
```

`%LOCALAPPDATA%\Microsoft\WinGet\Links` held **`xh.exe` and `xhs.exe`** —
two entries for one package, one of which is not the id, the name, or the last
segment of either. `%LOCALAPPDATA%\Microsoft\WinGet\Packages` held one directory,
`ducaale.xh_Microsoft.Winget.Source_8wekyb3d8bbwe`.

This is the winget analogue of scoop's `bins`, the field
`backend::winget::rows_to_scan` leaves empty because "there is no winget-side
manifest to read executable names from". There is, in a sense — but only in
`install`'s stdout, at install time. **`winget list` does not expose it**, so a
scan still cannot recover it.

## 12. winget manifests declare dependencies, and scoop's never did

`docs/measurements-2026-08-08-scoop-exit-codes.md` has a section titled
**"Falsified: `depends`"**: 0 of 30 installed scoop manifests and 0 of 25
bucket-HEAD manifests declared any dependency, so "a pinned manifest pulling a
dependency at latest, over the network, inside the mutation window" was recorded
as a *falsified* concern.

It is not falsified for winget. Of the 12 candidate packages surveyed in W0,
**5 declare `Microsoft.VCRedist.2015+.x64`** (`sharkdp.hyperfine`, `sharkdp.fd`,
`dandavison.delta`, `chmln.sd`, `dbrgn.tealdeer`); 7 declare none.
`sharkdp.hyperfine`'s manifest:

```
Dependencies:
  - Package Dependencies:
      Microsoft.VCRedist.2015+.x64
```

**The dependency-install path is unmeasured, deliberately.** The guinea pig was
switched to a dependency-free package (`ducaale.xh`) precisely so a dependency
resolution would not run inside the command being measured. And on this machine
`Microsoft.VCRedist.2015+.x64` is already installed at `14.51.36247.0`, so it
would have been satisfied rather than fetched anyway. Recorded as unmeasured
rather than as benign: a `winget install` that also installs a second package
gives dotpkg an installed package with no lock entry, no `state.json` ownership,
and no declaration — reported as `Unmanaged` on the very next `status`.

## 13. Timings

| operation | ms |
|---|---|
| `install --version` with download (portable zip, ~2–4 MB) | **8154, 9176, 7154** |
| `install --force` at the same version, from cache | **2121** |
| `install`/`upgrade` no-op (`0x8A15002B`) | 1057, 1085, 2106, 3100, 3112 |
| failing write verb (package or version absent) | 1053–2131 |
| `list -e --id <one package>` (the verify call) | ~1000 |

A verify-after-each-step costs about **1 s per step**. A 17-package winget
`apply` that verifies every step pays roughly 17 s of verification on top of the
installs themselves.

## 14. Fixture drift, measured against the checked-in fixtures

| | `tests/fixtures/winget/list-full.txt` | a14, 2026-08-10 |
|---|---|---|
| rows | 141 | **140** |
| distinct ids | 126 | **125** |
| `installed` after `rows_to_scan` | 37 | **36** |
| `opaque` | 89 | 89 |

`wez.wezterm` has been uninstalled; `tailscale.tailscale` moved `1.98.2` →
`1.102.2`; winget's own source MSIX row rotated
(`…Source_2026.809.1424.23…` → `…Source_2026.810.756.39…`).

`tests/fixtures/winget/PROVENANCE.md`'s statement that the machine is
"numerically identical" to the fixtures was true when written and is not now. A
Phase 4b dogfood that reuses 141/126/84/42/57/15 as expected values will go red
for the wrong reason.

**A byte-count comparison across capture methods is not valid**, and one was
nearly recorded here: the same `winget list` is **30744 bytes** captured via
`& winget … | Out-String` + `WriteAllText` and **30738 bytes** captured via
`Start-Process -RedirectStandardOutput`. Row, id, `installed` and `opaque`
counts are identical under both, so the structural drift above is real and the
6-byte difference is an artifact of how the bytes were captured.

---

## What was deliberately not measured

- **`--location`.** The probe was confounded: it asked to install 0.26.1 while
  0.26.2 was installed, so §2's upgrade short-circuit returned `0x8A15002B`
  before `--location` was ever consulted. Needs a re-run from an *absent* state.
- **`--all-versions`, and `uninstall --version <the installed version>`.** Both
  were blocked by §5's elevation refusal.
- **Whether `--force` or `--purge` overcomes §5's refusal.**
- **Any non-portable installer type.** Every candidate on this machine was
  `portable`. §2's `Found an existing package already installed` line *was*
  observed against two real EXE-installer packages (`Obsidian.Obsidian`,
  `Brave.Brave`) via a version-absent probe, so the upgrade-reinterpretation is
  not portable-specific — but a *successful* downgrade, upgrade or uninstall of
  an MSI/EXE package is unmeasured, and `--silent`'s behaviour for an installer
  that has a GUI is unmeasured.
- **The dependency-install path** — see §12.
- **Two versions of one id installed side by side.** Every `install --version`
  on an installed package either upgraded it or did nothing, so the
  `7zip.7zip`-shaped state was never constructed and `--all-versions` had no
  multi-version target even before §5 blocked it.
- **A machine that is not a14, and a winget other than v1.29.280.**

## A note on this document's own method

Two mistakes, both caught, both recorded because the second one nearly produced
a transcript that read like a completed round.

**W0's first run measured nothing and looked like it had.** The helper function
was named `Winget`, so `& winget @argv` resolved to the *function* — PowerShell
function names are case-insensitive — and recursed instead of running
`winget.exe`. Every `exit=` field came back blank and `winget list` came back as
369 bytes of a formatted PowerShell object. It was caught by reading the byte
count, not by the script. Both defences added afterwards: the executable is
resolved to an absolute path via `Get-Command -CommandType Application` up
front, and a **sanity check refuses to continue** unless the first `winget list`
exits 0 and exceeds 10 KB. This is the `ok. 0 passed` class
`docs/phase4-notes.md` names, reproduced in a measurement script.

**One claim in this round's own working notes was falsified by §7.** Before
measuring, the design's rule that every winget call drops `--exact` was read as
a hazard for `uninstall` — a fuzzy match removing the wrong package. §7 shows
`--id` never fuzzy-matches at all. The claim was reasoned, not measured, and it
was wrong; §7 is what replaced it. Fifth round in a row in this project where a
measurement overturned an assumption, and the first where the assumption was the
measurer's own.
