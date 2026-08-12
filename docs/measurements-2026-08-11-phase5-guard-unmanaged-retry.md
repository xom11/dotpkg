# Measurements: the running-process guard, the `Unmanaged` flood, and whether winget has a transient failure at all

Round run on a14 (`zenbook-a14`, winget `v1.29.280`, PowerShell 5.1) on
2026-08-11, against `main` at `1d633c6`. **Every probe is read-only**: only
`show`, `list`, `source update --name winget`, `Get-Process`, `Get-ChildItem`
and registry reads. No winget write verb was invoked at any point, and no
`.ps1` in the round contains a backtick character.

Machine left as found, verified after the last probe: `winget list` sha
identical to the value captured before the round (`55DD6D135C3F0FCA`),
`pkg.toml` `32A238FF...` unchanged, no `pkg.toml.bak`, 31 scoop apps, and
kanata still `kanata_windows_tty_winIOv2_arm64` **PID 13676** — the same PID
the phase brief recorded, so nothing in this round restarted it.

Probe scripts and raw captures: `C:\Users\kln\phase5-probe` (14 files,
107 KB). `p1-inventory.ps1`, `p2-flake.ps1`, `p3-procs.ps1`, `p4-paths.ps1`,
`p6-sourceupdate.ps1`, `p7-reader.ps1`, plus their reports.

## The headline

1. The `Links` clue in the brief is **not** the independent oracle
   `docs/phase4b-notes.md`'s still-open item 10 asks for. It is something else,
   and better for item 9: **the winget analogue of `Running.dirs`**, the signal
   three documents said cannot fire for winget at all.
2. `Unmanaged` for winget is **already saturated**: 36 `? winget` lines on
   every `status`, measured with production code. The dependency framing of
   still-open item 2 would suppress **0 of the 36** on this machine.
3. Still-open item 11's mechanism is **falsified in the direction that
   matters** and true in a direction that is already handled. The reader path
   never failed in 105 invocations; the writer path failed 3 of 10 under
   contention, and its failure has been non-fatal since Phase 4b.
4. The single most important thing this round measured is not about winget at
   all: **kanata is protected only by its executable's path**, and that is the
   protection winget has none of.

## 1. `kanata` is caught by a path, not by a name

`p4-paths.ps1` captured every live process's `Path`. 223 process entries, **22
with an unreadable path**.

```
under C:\Users\kln\scoop\apps : 2
  kanata_windows_tty_winIOv2_arm64 -> ...\scoop\apps\kanata\current\kanata_windows_tty_winIOv2_arm64.exe
  beckon-serve                     -> ...\scoop\apps\beckon\current\beckon-serve.exe
under ...\Microsoft\WinGet\Packages : 1
  VKey.exe -> ...\WinGet\Packages\PhatMT97.VKey_Microsoft.Winget.Source_8wekyb3d8bbwe\VKey.exe
```

`kanata_windows_tty_winIOv2_arm64` is not the package name, not a prefix of it,
and not any suffix of it. `Scoop::running_apps` catches it because it strips
`$SCOOP/apps/` off the executable's path and takes the first segment —
`kanata`. **Nothing else in the guard could catch it.** The scenario the whole
project exists to avoid is defended, today, by exactly one signal, and winget
does not have that signal.

The 22 unreadable paths are the known blind spot `Scoop::running_apps`' own doc
comment already states — `src/backend/scoop.rs:181-182`: "a process at a higher
integrity level reports no path at all, and that is exactly the case — an
elevated kanata — where names still work". Measured here rather than assumed.

*Pointer corrected in place by Phase 5 Task 8.* This section originally cited
`src/backend/scoop.rs:212-214` for "an elevated process reports no `exe` and is
caught only by name", which was **correct against `1d633c6`**: that sentence lived
in `Scoop::running_set`'s doc comment, and Phase 5 Task 2 deleted that function
when `backend::running_set` became the one fence producer. So this phase broke its
own document's pointer. The claim is unchanged and still stated in the same file;
only the surviving statement of it moved.

## 2. What the guard catches today: 3 of 36

Computed by running production `parse_list` -> `rows_to_scan` ->
`Running::covers` on a14's live `winget list` and a14's live process table (86
unique names), from macOS, because those functions are pure.

| | |
|---|---|
| source-backed installed winget ids | **36** |
| caught by today's three-signal guard | **3** — `Brave.Brave`, `Microsoft.PowerShell`, `PhatMT97.VKey` |
| caught by Phase 4's `key()`-only guard | **0** |

The 0 -> 3 move is Phase 4b's `guard_names` fix, confirmed live for the first
time against a real process table rather than against the counts recorded at
design time.

**Live misses, each with the running process that should have caught it:**

| installed id | `guard_names` produces | live process | reachable from disk? |
|---|---|---|---|
| `Tailscale.Tailscale` | `["tailscale"]` | `tailscaled`, `tailscale-ipn` | **no** — not portable |
| `AutoHotkey.AutoHotkey` | `["autohotkey"]` | `autohotkey64` | **no** — not portable |
| `Microsoft.WSL` | `["wsl"]` | `wslservice` | **no** — not portable |

None of the three is reachable by any on-disk winget artifact, because none is
a `portable` install. This is what bounds §3 below.

## 3. The `Links` directory: real, resolvable, and portable-only

`%LOCALAPPDATA%\Microsoft\WinGet\Links` holds **5 entries, every one a
`SymbolicLink`**, and every target resolves into
`Packages\<id>_<sourceIdentifier>\`:

```
codex.exe                       -> ...\OpenAI.Codex_...\codex-aarch64-pc-windows-msvc.exe
codex-command-runner.exe        -> ...\OpenAI.Codex_...\codex-command-runner.exe
codex-windows-sandbox-setup.exe -> ...\OpenAI.Codex_...\codex-windows-sandbox-setup.exe
rg.exe                          -> ...\BurntSushi.ripgrep.MSVC_...\ripgrep-15.2.0-.../rg.exe
zoxide.exe                      -> ...\ajeetdsouza.zoxide_...\zoxide.exe
```

`C:\Program Files\WinGet\Links` and its `(x86)` sibling are **absent**, so no
machine-scope portable exists on this machine.

`C:\Program Files\WinGet\Packages` — the machine-scope **package** root itself,
not its `Links` sibling — was probed in the same pass and is **absent** too. Raw
probe output, `p1-report.txt-78`:

```
--- dir: C:\Program Files\WinGet\Packages
ABSENT
```

*Added by Phase 5 Task 8, in place rather than as a correction entry elsewhere,
because the gap is this document's.* The section as originally written recorded
only the two machine-scope `Links` directories as absent, while
`src/backend/winget.rs`'s `package_roots` cites §3 as **measured** for exactly
this root's absence — a reviewer read the section and correctly found that it did
not support the claim. The probe did check it; the document was incomplete, so
the document was fixed rather than the code comment weakened.

**Coverage is 4 of 36 ids (11%), all `portable`.** Brave, Chrome, Discord,
Telegram, Obsidian, Vivaldi, Warp, Edge — every EXE/MSI application, which is
the class the running-process guard exists for — appears in neither `Links` nor
`Packages`.

**`guard_names` against the five real basenames: 2 of 5 caught.**

| basename | id | `guard_names` | |
|---|---|---|---|
| `codex` | `OpenAI.Codex` | `["codex", "codex cli"]` | CAUGHT |
| `codex-command-runner` | `OpenAI.Codex` | same | MISSED |
| `codex-windows-sandbox-setup` | `OpenAI.Codex` | same | MISSED |
| `rg` | `BurntSushi.ripgrep.MSVC` | `["msvc", "ripgrep msvc"]` | **MISSED** |
| `zoxide` | `ajeetdsouza.zoxide` | `["zoxide"]` | CAUGHT |

**`rg` reframes still-open item 9.** The item is written as "a package's
*second* alias is invisible". `rg` is ripgrep's *only* command, and it is
invisible: the id's last dotted segment is `MSVC`. The class is wider than a
second alias — it is "the process name is not derivable from the id".

### Corrections to the brief's account of this clue

- **"the cleanup script counted `leftover Links: 2` for pattern `xh*`" is not
  in the record.** The record says **0 leftover Links**, in three places:
  `.superpowers/.../progress.md`, `:396`, and
  `docs/measurements-2026-08-10-winget-write-path.md:40`. The `2` comes from
  that document's §11, observed **while `xh` was installed**. That makes the
  clue stronger, not weaker: `Links` went 2 -> 0 across install -> uninstall,
  so it does track state.
- **`Packages\` alone is not a faithful record; the `.db` and the ARP key
  are.** `PhatMT97.VKey.Classic_Microsoft.Winget.Source_8wekyb3d8bbwe\` still
  exists, holding only a `config.toml` — no `<id>_<hash>.db`, no ARP key, and
  `PhatMT97.VKey.Classic` is absent from `winget list`. A directory-existence
  check would report an uninstalled package as installed. (Harmless for a
  guard, which only consults entries for ids that are in `installed`; fatal for
  an oracle.)

### The oracle question (still-open item 10) — a different file, not measured

`%LOCALAPPDATA%\Packages\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\LocalState`
contains:

| file | bytes |
|---|---|
| `Microsoft.Winget.Source_8wekyb3d8bbwe\installed.db` | **262144** |
| `StoreEdgeFD\installed.db` | 225280 |
| `pinning.db` | 16384 |

A winget-written tracking catalog, **not** portable-only. This is the strongest
oracle candidate found, and it was **not opened**: reading it means bundling
SQLite and depending on an undocumented internal schema. Recorded as a lead,
not as a finding.

Registry, for completeness: HKCU `Uninstall` has 16 keys, **4 winget-shaped**
(`<id>_<sourceIdentifier>`), each carrying `InstallLocation` = its package
directory. HKLM has 35 keys and WOW6432Node 101, **0 winget-shaped in either**.
So an EXE/MSI winget package's ARP entry is named by its publisher, and mapping
it back to a winget id is exactly the guesswork this crate refuses.

## 4. `Unmanaged`: 36 lines, measured with production code

The whole chain — `parse_list`, `rows_to_scan`, `config::load`, `plan::plan`,
`render::render` — run on macOS against a14's live `winget list` capture and
a14's real `pkg.toml`:

| | |
|---|---|
| rows / ids | 141 / 126 |
| `installed` / `opaque` | **36 / 90** (10 scan warnings) |
| `pkg.toml` declares | 25 scoop packages, **0 winget packages** |
| `pkg.lock` | **absent** on this machine |
| `state.json` | **absent** on this machine |
| **`Action::Unmanaged` for winget** | **36** |
| **rendered `? winget` lines** | **36** |

The replication was validated first: the same computation over the checked-in
`tests/fixtures/winget/list-full.txt` reproduces `141 / 126 / 37 / 89`, exactly
what `PROVENANCE.md` records.

**Why still-open item 2's framing does not survive this.** Item 2 says winget
declares dependencies (5 of 12 surveyed declare
`Microsoft.VCRedist.2015+.x64`) and dotpkg has no vocabulary for a package it
did not declare appearing after an install. Both halves are true. But:

- The three `Microsoft.VCRedist.2015+.{x64,x86,arm64}` rows all carry
  `Source: winget` (checked against the checked-in fixture, before any machine
  was touched), so they do land in `installed` and are reported. Confirmed.
- They are **3 of 36**. Around 11 of the 36 are runtime or platform packages
  nobody declares: VCRedist x3, `Microsoft.VCLibs.*` x2,
  `Microsoft.WindowsAppRuntime.1.{6,7}`, `Microsoft.DotNet.Native.Runtime`,
  `Microsoft.OpenCLGLVulkanCompatibilityPack`, `Microsoft.WindowsSDK.*`, and
  `Microsoft.AppInstaller` — **winget itself**.
- A dependency-aware fix reads a declared package's manifest for its
  `Dependencies` list. With **0** winget packages declared, there is nothing to
  read one from, so such a fix suppresses **0 of the 36 lines on this
  machine**.

The defect is the **volume** of `Unmanaged`, not the absence of a dependency
vocabulary. And the argument for fixing it is already in the code:
`rows_to_scan` collapses 84 sourceless ids into **one** aggregate warning
because "84 lines for the ordinary shape of a winget machine is exactly the
false-positive flood that gets a feature silenced and never turned back on".
That reasoning is stated for `opaque` and not for `Unmanaged`; `plan.rs:549`
gives winget `helpers: &[]` where scoop gets `SCOOP_HELPERS`. (That citation
read `:532` when this section was written, which was correct then; Task 5's
`5c4894c` shifted `src/plan.rs` by 17 lines and the whole-branch fix wave
re-pointed it. The measurement itself is unchanged.)

## 5. Transient winget failures: the reader never lost, the writer did

### The reader — the path a failure kills the whole run

`version_liveness` returns `Err` for any nonzero exit, and `main.rs:641`'s
`!preparation.is_ok() && !keep_going` then refuses the whole run at exit 2,
scoop included. (Read `:613` when written, against `1d633c6`; the phase's own
`apply` arm then grew. Re-pointed by the whole-branch fix wave — see the notes'
citation-drift table. The measurement is unchanged.)

| probe | argv | calls | nonzero |
|---|---|---|---|
| P2 S2 | `show --id <id> -v <ver> --disable-interactivity` | 40 | **0** |
| P2 S3 | `list -e --id <id> --disable-interactivity` | 20 | **0** |
| P2 S4 | `show …` with a `source update` running concurrently | 15 | **0** |
| P7 | `show …` with a continuous `source update` loop running | 30 | **0** |
| | | **105** | **0** |

Timings: 407 / 778 / 1117 ms (min / median / max) over P2's 75 timed calls.
`show` runs ~0.6 s, `list -e --id` ~1.0 s, consistent with the ~1 s per
invocation figure already recorded.

### The writer — where the transient actually lives

`p6-sourceupdate.ps1`, run **because** the one nonzero exit P2 saw was the
number that agreed with the premise, and this project's own rule is to
re-measure that number first:

| condition | calls | nonzero |
|---|---|---|
| `source update --name winget` alone | 10 | **0** |
| the same, with one other winget process alive | 10 | **3** |

All three failures: exit `-1978335231` = **`0x8A150001`**, **60 / 69 / 72 ms**,
**empty stdout**. Successes: 348–623 ms, stdout beginning `Updating source:
winget...`. So the failure is distinguishable on three independent axes — exit
code, duration, and output presence — and its trigger is a concurrent winget
process, not the network.

**The consequence is not the one item 11 predicted.** `Winget::update_source`'s
`Err` is already downgraded to a warning at `src/update.rs:410`, so this never
fails a run. What it does instead: **3 of 10 times, `dotpkg update` resolves
`latest` against an index it failed to refresh, and only warns.**

**Not measured, and it is the generalisation the design must not make
silently:** whether `show` or `list` ever return `0x8A150001` under the same
contention. In P2 S4 and P7 the reader was always the winner of the race — 105
of 105 — so the asymmetry (readers share the index, the updater needs it
exclusively) is a **mechanism inferred from the numbers, not a measured
property of the reader**.

**Also true and not in item 11:** `--keep-going` is not a full escape hatch.
`gate_removals` holds **every** removal step whenever `is_ok()` is false,
scoop's included. And `status` never calls `version_liveness` at all — it uses
`backend::scan_or_warn` (`main.rs:481`; read `:468` when written, against
`1d633c6`, and re-pointed by the whole-branch fix wave for the same reason as
`:641` above), which deliberately does not abort on a winget hiccup. Item 11 is
an `apply` problem, not a `status` problem.

## 6. Four method failures of my own in this round

Recorded because a probe that reports a wrong answer confidently is the failure
mode this project keeps paying for.

1. **`p1-inventory.ps1` reported `kanata not running`. It was wrong.** The
   check was `Get-Process -Name kanata`, and the process is
   `kanata_windows_tty_winIOv2_arm64`. PID 13676 was alive throughout. I
   reproduced still-open item 9's exact defect class inside the probe sent to
   measure it, and only `p4-paths.ps1`'s full listing caught it.
2. **`p1-list.txt` is not byte-faithful and must never become a fixture.**
   PowerShell 5.1 decoded winget's UTF-8 output as the OEM code page, so `®`
   in `OpenCLTM, OpenGL®, and Vulkan® Compatibility Pack` was re-encoded as
   `┬«`. The file is **30958 bytes with 143 CRLF pairs** — the checked-in
   fixture's exact numbers — with a **different sha256**. The previous round's
   method finding says "compare counts, not sizes, across methods"; this is the
   same lesson from the other side: identical size and identical CRLF count
   over different bytes. The id, `Version` and `Source` columns are ASCII and
   unaffected, which is why the counts in §4 stand.
3. **P2's S5 measured nothing.** Its five rounds of 4 parallel `winget show`
   processes used `Start-Process -PassThru` with `-RedirectStandardOutput`, and
   `$p.ExitCode` came back **empty** for all 20. Those 20 calls are **excluded
   from every total in §5**; the only evidence they succeeded is that each
   wrote the same 2642 bytes a successful call writes. Inconclusive, and
   recorded rather than quietly folded into a "0 nonzero" claim. P6 avoided the
   API and captured exit codes by direct invocation.
4. **`Get-ChildItem -Recurse -Include '*.exe'` did not filter.** `-Include`
   needs a wildcard `-Path`, so `p3-pkgexes.txt` is a full recursive listing.
   A superset, so the analysis stands, but the file's name and the probe's
   stated intent were both wrong.

**And one about the designer rather than the measurer.** While proposing the
aggregate line for §4, I wrote a mock-up reading `36 change(s), 0 skipped`.
`Plan::change_count` (`plan.rs:129-131`) counts Install / Upgrade / Prune /
non-winget Downgrade only; an `Unmanaged` is counted by nothing, so the real
line is `0 change(s), 0 skipped`. A false number in the one line a user
consents to — the project's own thrice-fixed defect — invented inside a design
whose subject is that line. The same mock-up also carried
`(11 are runtimes: VCRedist, …)`, a classification **I** made by reading the
ids and that no code in this crate can derive; implementing it would have
required a hardcoded `WINGET_HELPERS` list that the mock-up never named as a
decision.
