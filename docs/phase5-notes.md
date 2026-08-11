# Carried forward out of Phase 5

Findings from building Phase 5 (the running-process fence learns to see a winget
package; `Unmanaged` stops flooding `status`; the one winget transient that was
actually measured gets one retry), plus everything the execution ledger recorded
along the way.

Same discipline as `docs/phase4-notes.md` and `docs/phase4b-notes.md`: every item
says whether it was **measured**, **structural** (true by the shape of the code,
provable by reading, not by running a machine), or **reasoned only**. Where a
claim is reasoned only, that is stated rather than dressed up.

That promise is cheap to make and this project has already broken it once:
`docs/phase4b-notes.md`'s Verification section shipped stale by its last five
commits, and its post-merge audit's sharpest finding was that the notes did not
say a pre-merge gate had gone unmet. So every number below names the tree it was
measured on, and the Verification section says plainly which runs have **not**
happened yet rather than implying they went well.

`docs/phase2-notes.md`, `docs/phase2b-notes.md`, `docs/phase3-notes.md`,
`docs/phase4-notes.md` and `docs/phase4b-notes.md` still hold the earlier items;
this file does not repeat them except where a Phase 4b "still open" item was in
this phase's scope and its status changed.

- Full measurement record:
  [`docs/measurements-2026-08-11-phase5-guard-unmanaged-retry.md`](measurements-2026-08-11-phase5-guard-unmanaged-retry.md)
  — a14 (`zenbook-a14`, winget `v1.29.280`, PowerShell 5.1), 2026-08-11, every
  probe read-only.
- Design:
  [`docs/specs/2026-08-11-phase5-guard-unmanaged-retry-design.md`](specs/2026-08-11-phase5-guard-unmanaged-retry-design.md).
  Its Scope table records that the measurement round changed all three of the
  phase brief's targets before any of them reached a design.
- Base: `main` at `1d633c6`, 588 tests on macOS, clean.

The full execution record — every task, every review finding, every ruling,
including several corrections made to the controller — is
`.superpowers/sdd/2026-08-11-phase5-guard-unmanaged-retry/progress.md`. That file
is git-ignored, so the parts of it that outlive the branch are carried here.

## Read this first

### The two user-visible behaviour changes that are not additions

Both change what a user sees on a machine whose configuration did not change.
Neither changes what dotpkg *does*.

The second is listed second because a `pkg.toml` that does not use it behaves
identically — but it is a schema change, not a new command, and a reader
scanning for "what moved" needs both.

The precedent: Phase 4b's **ledger** parked a finding that
`docs/phase4b-notes.md`'s heading then read "The one user-visible behaviour change
that is not an addition" while the same wave also changed the consent prompt's
wording for every scoop run with a replacement, and changed scoop's ghost
reconciliation. The heading was fixed — that file has said "two" since `24ba0d6`,
"Say what status actually tolerates, and stop calling two changes one", the commit
that closed the finding — but the scoop-side change the finding named is still
absent from the two it lists, so what was hidden is the same shape as what this
phase had to disclose: the scoop half of a change measured on winget.

#### 1. `status` and `apply` collapse `Unmanaged` to one line per backend — for BOTH backends

**Measured** (measurement document §4): on a14, `status` printed **36**
`? winget` lines, every run, computed with production code (`parse_list` →
`rows_to_scan` → `config::load` → `plan::plan` → `render::render`) against a14's
live `winget list` capture and a14's real `pkg.toml`. Now it prints one line per
backend and a hint:

```
  ? scoop    6 installed outside dotpkg -- no action
  ? winget   36 installed outside dotpkg -- no action
      pass --show-unmanaged to list them

  0 change(s), 0 skipped, 42 unmanaged
```

The winget `36` is measured. **The scoop `6` above is illustrative** — it comes
from the fixture that pins this output in `src/render.rs`; the measured
machine's scoop half was never counted in this round, and this file will not
imply it was.

**This changes scoop's output too, for every user with an undeclared scoop app**,
and that was a decision rather than an oversight: the flood is measured for
winget only, but collapsing winget alone means a per-backend special case with no
measurement behind the asymmetry either. So a scoop user with undeclared apps
sees different output on a machine whose configuration did not change.

`--show-unmanaged`, on **both** `status` and `apply`, brings every per-package
line back and drops the hint. **It does not restore Phase 4b's output byte for
byte, and the design claimed it would** — see Corrections below: the summary line
carries a new `, N unmanaged` clause on both paths.

Structural, provable by reading:

- **Nothing dotpkg does changed.** `Action::Unmanaged` was report-only before
  this phase and still is; `plan.rs` still concatenates `reports` into `actions`,
  so `Plan` keeps all 36 facts and only the two renderers collapse them.
- **Two tables, not one.** A full `apply` prints `render(plan)` and then
  `render_preparation`, and `apply::prepare` turns every `Action::Unmanaged` into
  an `Outcome::Report` that `render_preparation` printed one line each for. Both
  now call one shared `render::unmanaged_collapse_lines`, so the two tables
  cannot drift apart. This was a real defect in the first implementation of this
  task, found by **measuring the real binary**: a default `apply --prepare` run
  printed the collapsed line, then the hint, then every individual line below it
  — the measured flood surviving on the command that matters most, under a hint
  that advised a flag whose output was already on screen.
- **The summary clause is mandatory**, and pinned by an assertion on the literal
  text `0 change(s), 0 skipped, 42 unmanaged`. `Plan::change_count` counts
  Install / Upgrade / Prune / non-winget Downgrade; an `Unmanaged` is counted by
  nothing. Collapsing removes the very lines that used to carry the fact, so
  without the clause 42 printed facts sit under `0 change(s), 0 skipped` — the
  shape `refused_downgrade_count` already earned its own clause to avoid.
  **`render_preparation`'s summary now carries the same clause**, on the same
  gating — the count, never the flag. This file used to say it "has no such
  clause and needs none: its numbers never counted a report, and the collapsed
  line it prints carries its own count". That was true on the collapse path and
  false under `--show-unmanaged`, where no collapsed line is printed at all:
  `apply --show-unmanaged` printed N individual `?` lines and then `0 of 0
  changes ready, 0 failed, 0 skipped, 0 not locked.`, while `render(plan)`'s
  summary for the same run *did* count them, its clause being gated on the count
  rather than on the flag. The two tables of one `apply` run disagreeing about
  whether a printed fact is counted is the exact defect this task was already
  fixed for once, one level up. Found by the whole-branch review (Minor 1), and
  pinned by
  `render_preparation_counts_unmanaged_reports_on_both_paths_not_only_the_collapsed_one`,
  which asserts the literal summary text for **both** flag values from one
  `Preparation`, plus a second test asserting the clause is absent when there are
  no reports. **Structural:** none of the four counts in that line can see an
  `Outcome::Report`, so a report is counted by nothing else there.

#### 2. `[winget.guard]` is a new `pkg.toml` table

```toml
[winget.guard]
"Tailscale.Tailscale"   = ["tailscaled", "tailscale-ipn"]
"AutoHotkey.AutoHotkey" = ["autohotkey64"]
"Microsoft.WSL"         = ["wslservice"]
```

A `pkg.toml` that does not use it behaves identically: an absent table parses to
an empty map and an empty map merges nothing (structural).

Those three entries are not examples invented for a README. **Measured** (§2):
they are exactly the three live misses on a14's process table — the installed id,
what `guard_names` guesses from it, and the process actually running:

| installed id | `guard_names` produces | live process |
|---|---|---|
| `Tailscale.Tailscale` | `["tailscale"]` | `tailscaled`, `tailscale-ipn` |
| `AutoHotkey.AutoHotkey` | `["autohotkey"]` | `autohotkey64` |
| `Microsoft.WSL` | `["wsl"]` | `wslservice` |

None of the three is reachable from disk, because none is a `portable` install —
which is what bounds the path signal below, and why this table exists at all.
winget exposes no way for dotpkg to discover a package's process names: `winget
list` does not report aliases, and **measured** (§3), 0 winget-shaped ARP keys
exist in HKLM or WOW6432Node, so an EXE/MSI package's uninstall entry is named by
its publisher and mapping it back to a winget id is the guesswork this crate
refuses.

Structural, and each half was a deliberate choice:

- Values fold through `sys::normalize` — now `pub(crate)` rather than private —
  which is the same function `sys::running_processes` applies to what it reports.
  A second implementation would be the "two copies can drift" class, and
  `guard_names`' own doc comment already records why unfolded text silently never
  matches.
- A value that is empty after folding is a **parse error**; it would otherwise
  sit in the comparison set matching nothing while reading as protection.
- `RawWingetSection` carries `deny_unknown_fields`, so a typo like `guards` is
  refused rather than read as "you declared nothing".
- Merged into `Installed.bins` at **one** point (`backend::apply_guard_overrides`,
  after the scan and before `plan`), never inside `rows_to_scan`, which stays a
  pure function of winget's output with no `Config`. Because `apply::guard_for`
  copies `inst.bins` into the `Step`, that one merge point serves the plan-time
  fence and the mid-run re-sampler both.
- A key that matches nothing at all gets one warning per run; a key naming an id
  winget reported with **no source** gets a different warning, because "nothing
  installed by that name" would be false of a package winget just reported. That
  second warning cannot be silenced except by deleting the entry — ruled: keep
  it. See "Left open".

### `kanata` is protected by a path, and winget had no path signal at all

The single most important thing this round measured is not about winget.

**Measured** (§1): `p4-paths.ps1` captured every live process's `Path` — 223
process entries, **22 with an unreadable path**. Two live processes under
`C:\Users\kln\scoop\apps`, one under `…\Microsoft\WinGet\Packages`. And
`kanata`'s process is named `kanata_windows_tty_winIOv2_arm64`, which is not the
package name, not a prefix of it and not a suffix of it. `Scoop::running_apps`
catches it purely because it strips `$SCOOP/apps/` off the executable's path and
takes the first segment. **Nothing else in the fence could catch it** — and
winget had no equivalent signal at all.

It has one now, for the subset winget gives it to: `backend::winget::running_ids`
inserts a scanned winget id into `Running.dirs` when a live process's `exe` lies
under `<root>/<id>_…`, case-folded with `/` separators, the same `fold` shape
`Scoop::running_apps` uses.

Structural, provable by reading `src/model.rs`: `covers` is
`dirs || names || bins` and `covers_name` is `dirs || names`, and `covers_any`
calls `covers_name`. So **one `dirs` entry closes the plan-time hole and the
mid-run re-sampler hole in one place**, with no change to `Step` — the opposite
of Phase 4b's `bins` fix, which had to thread guard names through `Step` because
`covers_name` has no `bins` half.

**What it does not do, stated as a class and not as a number.** On the measured
machine it adds **zero** new catches: the one live process under `Packages` is
`VKey.exe`, and `guard_names` already catches `PhatMT97.VKey` by name. Coverage
is **4 of 36** installed ids, all `portable`, and it reaches **none** of the
three live misses `[winget.guard]` exists for. Its value is the class —
`kanata`'s shape, a process name derivable from nothing about the package — not a
number, and this file does not dress it as one.

Two further measured bounds on it, both from §3:

- The five real basenames under `%LOCALAPPDATA%\Microsoft\WinGet\Links` (all five
  entries are `SymbolicLink`s, every target resolving into
  `Packages\<id>_<sourceIdentifier>\`) are caught by `guard_names` **2 of 5**:
  `codex` and `zoxide` yes; `codex-command-runner`,
  `codex-windows-sandbox-setup` and `rg` no.
- A directory-existence check would be worse than useless as an oracle:
  `PhatMT97.VKey.Classic_…\` still exists on a14 holding only a `config.toml`,
  while `PhatMT97.VKey.Classic` is **absent** from `winget list`. Harmless for a
  fence, which only consults entries for ids that are in `installed`; fatal for
  the oracle still-open item 10 asks for. The `_` boundary in `running_ids` is
  load-bearing for exactly this reason — a bare `starts_with` would match
  installed `phatmt97.vkey` against that dead directory.

### Three wiring sites, and the third is the one the sampler exists for

`backend::running_set` — now the one fence producer, with `Scoop::running_set`
deleted — is reached from three places: `status`, `apply`'s plan-time fence, and
the mid-run re-sampler. A fix reaching the first two and not the third closes the
plan-time hole and leaves the during-the-run hole exactly as wide, which is the
case the sampler exists for, and is the mistake `docs/phase4b-notes.md` names
about itself.

The sampling was therefore hoisted out of `main.rs` into
`apply::sample_fence` / `sample_fence_with_roots`, so the *inputs* live in one
tested place. Two residuals, both recorded in `sample_fence`'s own doc comment
and both real:

- **`main.rs` still holds two `sample_fence` calls no test observes** (the
  `status` arm and the re-sampler closure). Nothing goes red if someone deletes
  the closure's call or swaps it for `Running::default()`.
- **Measured, and it is a loss:** with `main.rs`'s closure passing `&[]` the full
  suite is green *and emits no compiler warning at all*, where the pre-hoist
  shape at least produced `unused variable: fence_ids`. The hoist closed the
  inputs hole and removed an incidental compiler guard on site 3 in the same
  move.

And the bound on any black-box pin, structural: `tests/cli.rs:55-64` strips every
`PATH` directory containing `winget`/`winget.exe` from the environment it hands
the spawned binary, so every `cli.rs` test sees an absent winget, `Winget::scan`
returns an empty `Scan`, and the winget half of the fence is unreachable from the
only harness that observes `main.rs` at all. Overriding `LOCALAPPDATA` changes
which roots are reported, not whether there is anything to match.

### The transient lives in the writer, not in the reader

Still-open item 11 said a momentarily unhappy winget "fails the whole run, scoop
included". **Measured** (§5), that is falsified in the direction that matters and
true in a direction that was already handled:

- The two reader argvs — `show --id <id> -v <ver>`, which is what
  `version_liveness` runs, and `list -e --id <id>` — returned **0 nonzero exits
  in 105 invocations combined** (not 105 of either one alone; the split is
  below), including 30 fired against a continuously running `source update`
  loop.
- The writer, `source update --name winget`, failed **0 of 10** alone and **3 of
  10** with another winget process alive. All three failures: exit
  `-1978335231` = `0x8A150001`, in **60 / 69 / 72 ms**, with **empty stdout**,
  against successes of 348–623 ms whose stdout begins `Updating source:
  winget...`. Distinguishable on exit code, duration and output presence
  independently.

**The consequence is not the one item 11 predicted.** `update_source`'s `Err` has
been a warning, not a refusal, since `src/update.rs:410`. What actually happens
is that **3 of 10 times `dotpkg update` resolves `latest` against an index it
failed to refresh, and only warns.** So the retry went where the transient was
measured: `update_source_with(retry_delay)` retries **once**, after **1 s**, and
**only** on `INTERNAL_ERROR`. Any other nonzero exit keeps the previous behaviour
exactly, because retrying a definitive answer only slows a certain failure down.

**The 1 s is chosen, not measured**, and the code says so at the call site: the
failure itself returns in 60–72 ms and a successful `source update` takes
348–623 ms, so 1 s clears the measured success range — but the process it lost to
in that probe was a full `winget list` whose own duration was never timed, and
the margin is not measured to be sufficient on a slower machine. Both gaps are
named in `update_source`'s comment rather than presented as a derived number.

**No retry for the reader**, and the arm added there is labelled with the
provenance it does not have: `INTERNAL_ERROR` was measured from `source update`,
**never** from `show` or `list`. In every contention probe the reader was the
winner — 105 of 105 — so "readers share the index, the updater needs it
exclusively" is a **mechanism inferred from the numbers, not a measured property
of the reader**. `version_liveness`'s generic arm now names that likely cause and
the action (re-run) instead of printing a bare `exited <n>`, and says at the arm
that it may never fire.

Also measured, and written down here because three code comments now depend on
it: the 105 are **two** argvs, not one — **85** calls of `show --id <id> -v
<ver> --disable-interactivity` (P2 S2's 40, P2 S4's 15, P7's 30) and **20** of
`list -e --id <id> --disable-interactivity`, which is `list_one_argv`'s shape,
the post-mutation verify rescan's, **not** the elevation pre-check's (that one
inserts `--scope`). None of the 105 used `resolve_latest`'s flagless form, which
is what makes leaving `resolve_latest` alone a measured decision rather than a
guess.

## What was measured versus what was only reasoned

Labels copied from the measurement document rather than re-derived.

**Measured on a real Windows machine (a14) on 2026-08-11, every probe read-only**
— only `show`, `list`, `source update --name winget`, `Get-Process`,
`Get-ChildItem` and registry reads; no winget write verb was invoked at any
point. Machine verified left as found afterwards: `winget list` sha
`55DD6D135C3F0FCA` identical to the pre-round capture, `pkg.toml` `32A238FF…`
unchanged, no `pkg.toml.bak`, 31 scoop apps, and kanata still at **PID 13676**,
the same PID the phase brief recorded.

- 223 live process entries, **22 with an unreadable path** — the known blind spot
  `Scoop::running_apps`' own doc comment already states
  (`src/backend/scoop.rs:181-182`: "a process at a higher integrity level reports
  no path at all"), measured here rather than assumed. 86 unique live process
  names.
- `kanata`'s executable path is the only thing protecting it (§1, above).
- **36** source-backed installed winget ids; **3** caught by the fence as it stood
  when this round ran, i.e. before this phase (`Brave.Brave`,
  `Microsoft.PowerShell`, `PhatMT97.VKey`, all by name);
  **0** by Phase 4's `key()`-only fence. The 0 → 3 move is Phase 4b's
  `guard_names` fix, confirmed live for the first time against a real process
  table rather than against design-time counts.
- The three live misses and their processes (`Tailscale.Tailscale`,
  `AutoHotkey.AutoHotkey`, `Microsoft.WSL`), none of them a `portable` install.
- `%LOCALAPPDATA%\Microsoft\WinGet\Links`: **5 entries, every one a
  `SymbolicLink`**, every target resolving into
  `Packages\<id>_<sourceIdentifier>\`. `C:\Program Files\WinGet\Links` and its
  `(x86)` sibling **absent**; `C:\Program Files\WinGet\Packages` **absent** too
  (that last line was added to §3 by this task — see Corrections).
- Path-signal coverage is **4 of 36 ids (11%), all `portable`**. Brave, Chrome,
  Discord, Telegram, Obsidian, Vivaldi, Warp, Edge — every EXE/MSI application,
  the class the fence exists for — appear in neither `Links` nor `Packages`.
- `guard_names` against the five real `Links` basenames: **2 of 5** caught.
- `Unmanaged`, whole chain run with production code against a14's live capture and
  real `pkg.toml`: **141** rows / **126** ids, **36** `installed` / **90**
  `opaque` (10 scan warnings), `pkg.toml` declaring **25 scoop packages and 0
  winget packages**, `pkg.lock` **absent**, `state.json` **absent**, **36**
  `Action::Unmanaged` for winget, **36** rendered `? winget` lines. The
  replication was validated first: the same computation over the checked-in
  `tests/fixtures/winget/list-full.txt` reproduces `141 / 126 / 37 / 89`, exactly
  what `PROVENANCE.md` records.
- The three `Microsoft.VCRedist.2015+.{x64,x86,arm64}` rows all carry `Source:
  winget` (checked against the checked-in fixture before any machine was
  touched), so they *do* land in `installed` and *are* reported — 3 of 36.
- Reader path: **105 invocations, 0 nonzero** (40 + 20 + 15 + 30). Timings
  407 / 778 / 1117 ms (min / median / max) over P2's 75 timed calls.
- Writer path: **0 of 10** alone, **3 of 10** under contention, every failure
  `0x8A150001` in 60–72 ms with empty stdout.
- Registry, for completeness: HKCU `Uninstall` has 16 keys, **4 winget-shaped**
  (`<id>_<sourceIdentifier>`), each carrying `InstallLocation`. HKLM has 35 keys
  and WOW6432Node 101, **0 winget-shaped in either**.
- File sizes only, contents **not opened**:
  `LocalState\Microsoft.Winget.Source_8wekyb3d8bbwe\installed.db` **262144
  bytes**, `StoreEdgeFD\installed.db` 225280, `pinning.db` 16384.

**Structural, provable by reading, not by running a machine:**

- `Running::covers` is `dirs || names || bins`; `covers_name` is `dirs || names`;
  `covers_any` calls `covers_name`. One `dirs` entry therefore serves both
  fences with no `Step` change.
- `backend::running_set` is the only non-test producer of a `Running`, and
  `apply::sample_fence` its only non-test caller with the winget roots — so every
  winget `dirs` entry that ever reaches a fence comes from `running_ids`.
- `Plan` keeps every `Action::Unmanaged`; only `render` and `render_preparation`
  collapse, through one shared function. `Plan::is_empty` still reads
  `plan.actions`, so a plan whose only actions are reports is still not "nothing
  to do".
- `apply_guard_overrides`' backend half **cannot fire today**: every production
  `ScanOutcome` reaching it comes from `scan_or_warn(&winget)`, so every row
  already carries `backend == WINGET`. It is a structural guard against a future
  caller handing it a merged both-backend list — a shape that is live one file
  over in `apply::guard_for`.
- `package_roots()` returns an empty vector wherever `LOCALAPPDATA` /
  `ProgramFiles` are unset, which is every non-Windows platform, and
  `running_ids` is a no-op on an empty root list — so no `cfg` is needed.
- The retry lives behind `update_source_with(Duration)`, so the test passes
  `Duration::ZERO` and the suite does not sleep; `update_source()` supplies the
  real delay.
- `INTERNAL_ERROR`'s decimal value is cross-checked against the hex form recorded
  beside its definition (`assert_eq!(INTERNAL_ERROR as u32, 0x8A150001)`), in a
  test of its own: `the_internal_error_codes_decimal_and_hex_forms_still_agree`
  (`winget.rs:1513-1520`). The six exit-code constants are pinned across **three**
  such tests, not one — two in `winget.rs:1495-1510`, this one, and three in
  `winget_exec.rs:468-470` — so the value is pinned but the constants are not
  checked together. That matters because the defect class here is a sign flip on a
  constant's own definition, which every test that builds its `CmdOut` from the
  constant flips along with it; only the hex cross-check catches it, and only for
  the constants a cross-check actually names.
- **Neither `pkg.toml`-editing round-trip guard compares the new field, and the
  design said one of them did.** `verify_round_trip` (`src/config_edit.rs:117-133`)
  compares `after.scoop.buckets`, `after.scoop.opts`, `after.winget.packages` and
  scoop's packages; `verify_round_trip_winget` (`:195-209`) compares
  `after.scoop == before.scoop` and winget's packages. **Neither reads
  `after.winget.guard`**, so a `[winget.guard]` table one of those editors dropped
  or mangled would pass both silently. What makes that harmless *today* is not the
  guard — it is the second half of the claim: nothing in the tool ever writes
  `[winget.guard]`. The `Config`-comparing guards are the **semantic** halves
  (`config_edit.rs:110-116` says so); the text-level guard is
  `no_comment_was_lost`, which compares comment multisets and never looks at a
  field at all. Extending one of the semantic guards to cover `guard` is on the
  still-open list below.

**Reasoned only, and labelled as such at the code that depends on it:**

- **`%ProgramFiles%\WinGet\Packages` as a machine-scope root.** Measured
  **absent** on a14; included anyway for a machine-scope portable install that
  has never been observed. The two roots are one pair in the code and must not
  read as one measurement.
- **A bare `<root>/<id>/` directory with no `_<sourceIdentifier>` suffix is
  accepted** by `running_ids`. All 5 measured directories carry the suffix.
- **Whether a winget id can contain `_` is unmeasured.** This is why the code
  tests each scanned id against the directory segment rather than splitting the
  segment on `_`: the prefix test assumes nothing about winget's naming, and its
  only failure direction is "no match when the package is genuinely not running",
  where a truncated split fails toward "the fence misses and a running package
  gets replaced".
- **The 1 s retry delay** (above): chosen, not measured sufficient.
- **"Readers share the index, the updater needs it exclusively"**: a mechanism
  inferred from 105 of 105, not a measured property of the reader.
- **The path signal's value.** Zero new catches on the measured machine; the
  claim is about the class, and there is no number behind it.
- **Whether `winget list`'s `Id` and `winget show`'s `Id` are ever different
  spellings for one package.** The mid-run fence's `dirs` half compares them
  against each other: `Running.dirs` is filled from the *scan*'s ids
  (`backend::winget_fence_ids` → `winget::running_ids`, which inserts the scanned
  id itself, and `rows_to_scan` builds those from `winget list`'s `Id` column),
  while the `Name` the re-sampler asks about is `Step::app()` — for a
  `WingetStep::Set`, `Outcome::ReadyToSet`'s `id`, which is `winget show`'s `Id`
  as `parse_show` read it back. `Name` folds case, so a case-only difference is
  absorbed; nothing measures whether a non-case difference exists. Recorded as a
  residual, not as a bug — see still-open item 17.

## Method failures: the measurement round, the design, and the controller

Recorded because a probe — or a brief — that reports a wrong answer confidently
is the failure mode this project keeps paying for.

### Four in the measurement round (§6, in substance)

1. **`p1-inventory.ps1` reported `kanata not running`. It was wrong.** The check
   was `Get-Process -Name kanata`; the process is
   `kanata_windows_tty_winIOv2_arm64`, PID 13676, alive throughout. Still-open
   item 9's exact defect class, reproduced *inside the probe sent to measure it*,
   and caught only by `p4-paths.ps1`'s full listing.
2. **`p1-list.txt` is not byte-faithful and must never become a fixture.**
   PowerShell 5.1 decoded winget's UTF-8 as the OEM code page, so `®` became
   `┬«`. The file is **30958 bytes with 143 CRLF pairs** — the checked-in
   fixture's exact numbers — with a **different sha256**. The previous round's
   lesson was "compare counts, not sizes, across methods"; this is the same
   lesson from the other side. The id, `Version` and `Source` columns are ASCII
   and unaffected, which is why §4's counts stand.
3. **P2's S5 measured nothing.** Its five rounds of 4 parallel `winget show`
   processes used `Start-Process -PassThru` with `-RedirectStandardOutput`, and
   `$p.ExitCode` came back **empty** for all 20. Those 20 calls are **excluded
   from every total in §5** — recorded as inconclusive rather than quietly folded
   into a "0 nonzero" claim. P6 avoided the API and captured exit codes by direct
   invocation.
4. **`Get-ChildItem -Recurse -Include '*.exe'` did not filter.** `-Include` needs
   a wildcard `-Path`, so `p3-pkgexes.txt` is a full recursive listing. A
   superset, so the analysis stands, but the file's name and the probe's stated
   intent were both wrong.

### And one about the designer rather than the measurer

While proposing the aggregate line for §4, the design's own mock-up read
**`36 change(s), 0 skipped`**. `Plan::change_count` counts Install / Upgrade /
Prune / non-winget Downgrade only; an `Unmanaged` is counted by nothing, so the
real line is `0 change(s), 0 skipped`. A false number in the one line a user
consents to — this project's own thrice-fixed defect — invented inside a design
whose subject is that line. The same mock-up also carried
`(11 are runtimes: VCRedist, …)`, a classification made by reading the ids that
no code in this crate can derive; implementing it would have required the
hardcoded `WINGET_HELPERS` list the mock-up never named as a decision (and which
the design then rejected explicitly).

### The controller's own defects during execution

Kept because a controller that is never wrong on the record is a controller
nobody checks.

- **The same brief defect, made three times, always the same shape: scoping the
  change by the function the controller had in mind rather than by the surface a
  user sees.**
  - Task 2 Step 8 specified a red-state experiment in `tests/execute.rs` and
    Task 4 Step 6 one in `tests/planner.rs`, both watching `main.rs` wiring.
    Neither can fire: `main.rs` is the binary crate and those harnesses link only
    the library. Task 2's brief had even preserved the reasoning that says so
    ("the union lives in the library because `main.rs` is not reachable from a
    test at all") two steps earlier. Both implementers diagnosed it instead of
    faking the experiment; the remedies were the `sample_fence` hoist and two
    `tests/cli.rs` tests that spawn the real binary.
  - Task 5 scoped `--show-unmanaged` to `render(plan)` and never mentioned
    `render_preparation`, i.e. to one of the two tables a single command prints.
    The reviewer measured the consequence on the real binary (above).
  - The standing rule the plan gained from it: a red-state experiment on
    `main.rs` wiring must live in `tests/cli.rs`, and a change must be scoped by
    asking which surfaces a user sees it on.
- **Three fixes that replaced one false sentence with another** — Task 1's
  `package_roots` dead-code comment twice, Task 7's argv attribution once. All
  three were sentences about *which thing does what*.
  - Task 1 round 1 replaced "unreachable from anything but their own tests" with
    a claim about a literal `grep` pattern's output — a claim that is
    self-defeating, because a comment quoting the pattern always matches it, and
    re-staled by anyone who mentions the function anywhere. Round 2's instruction
    was not "correct the count" but "stop citing a grep": state reachability as a
    call-graph claim, which is what `Structural:` is supposed to mean.
  - Task 7's round-1 fix corrected the 105-invocation split and, in the same
    sentence, attributed the 20 `list -e --id` calls to `is_user_scope` when they
    belong to `list_one_argv` (verified against both argv builders and the probe
    script: the probe ran the form without `--scope`). Caught at the point of
    substitution rather than by a re-review, which is cheaper than the
    alternative and is why round 2 was worth spending.
- **A review dispatched against a commit the implementer then amended.** The
  controller's wait loop exited when a new commit *appeared* rather than when the
  agent finished, so a review package was generated for `bb588e3` while the
  implementer was still working; it was amended into `01df082` and the package
  then described a commit no longer on the branch. The review survived only
  because the reviewer read the live tree rather than the diff file, and all
  seven findings were re-verified by hand against `01df082`. Fix adopted for the
  rest of the plan: wait for the task notification, never for a commit to
  appear.
- **Factual claims written into briefs that shipped into code comments before
  being caught.** The implementer's default of keeping brief text verbatim is
  right for a design decision, which is the controller's to own; a *factual*
  claim in a brief is still a factual claim, and shipping it does not transfer
  the error.
  - A retry delay justified with the **wrong probe's timings** (the reader's
    407–1117 ms latency, where the retry is about the writer probe, whose
    competing process was a full `winget list` never timed at all).
  - **A constructor count of four where there are five** (`apply.rs`'s
    `FakeWinget`: `live`, `live_echoing`, `version_gone`, `absent`,
    `never_called`).
  - **A citation with a literal ellipsis** in the document filename, at two sites
    in `winget.rs` — plus a third at `src/config.rs:84` introduced by an earlier
    task, so the phase was on course to ship two broken paths to the document it
    cites as its own authority. The re-review then checked *section* accuracy as
    well as filename accuracy, on the grounds that a correct filename with a
    wrong section number is the same defect one layer down.
  - **An unverifiable "the seam this crate has extracted six times before"**,
    reduced to the one precedent that is genuinely the same `_with(param)` shape.
  - Five brief-stated expected test counts were **low** (596 vs 597, 600 vs 602,
    605 vs 614, 611 vs 622, 615 vs 630), always in the same direction, because
    each brief was written against an older tree. Harmless only because the
    standing instruction is to trust the tree over the brief — the same finding
    Phase 4b recorded about three of its own.
  - Three citations in briefs and reports pointed at the wrong line or the wrong
    source: a generic-arm citation that landed on `resolve_latest`'s
    `NO_APPLICATIONS_FOUND` arm instead of `version_liveness`; a report crediting
    the brief with a test-count note that came from the dispatch message, and
    with a citation of *this* file, which no brief contained because Task 8
    creates it; and a report citing `main.rs:610` for the `plan_to_steps` call,
    which is at `main.rs:638` on this tree (the ledger recorded `:621`, and by the
    end of the branch that too had drifted — the fifth instance of the class in
    one phase). Each was resolved by content rather than by line number.
  - Task 3's carry-over note named two **files** where the real subject was two
    **sentences**, and the plan had baked that wording in at Task 1 with no task
    slated to revisit it. The result was an orphaned inaccuracy: at Task 3's commit
    `winget.rs:270` said an EXE/MSI winget package is "reachable only through
    `names` or `[winget.guard]`", which was false while the guard was parsed and
    merged into nothing, and no document scheduled its fix. Pre-scheduled into
    Task 4 once a reviewer raised it.
  - Task 4's dispatch omitted one of the three sentences the controller's own
    ledger had already listed as needing a rewrite (`rows_to_scan`'s `bins` doc
    comment, `winget.rs:416-421` at the time). The implementer found it anyway. The
    omission was the controller's, the catch was not.
- **A reviewer touched the real working tree.** One re-reviewer disclosed running
  `git checkout <sha> -- src tests` with a stray `--work-tree` flag, which
  briefly modified the working tree before it restored it. Verified
  independently afterwards: `git status --porcelain` empty, `git diff <task
  commit>` empty, and the full suite re-run at 598 passed / 0 failed / 14 lines.
  No lasting effect — recorded because a reviewer that modifies the tree is
  exactly what must not be taken on faith.

## Corrections to earlier documents

Recorded here rather than edited in place, matching the precedent set by the 2a,
2b-2, Phase 3, Phase 4 and Phase 4b designs. Two exceptions, both because leaving
them would ship a falsehood rather than a superseded sentence: this phase's own
measurement document is corrected **in place** in two sections and says so at each
(see the end of this section), and the two `.rs` comments this phase falsified
were **fixed in the code**, since a reader of the tree cannot be warned off a
false comment by a document.

### From the design's own corrections section

- **The phase brief's `leftover Links: 2` is not in the record.** The brief
  reported that a dogfood cleanup script "counted `leftover Links: 2` for pattern
  `xh*`". The record says **0 leftover Links**, in three places — Phase 4b's own
  ledger at `progress.md:214` and `:396`, and
  `docs/measurements-2026-08-10-winget-write-path.md:40`, which is the one of the
  three that is checked in and reads "0 leftover `WinGet\Links` entries". The `2`
  is that same document's §11, observed **while `xh` was installed**. The clue survives the
  correction and is strengthened by it — `Links` went 2 → 0 across install →
  uninstall, so it does track state — but no leftover ever existed.
- **`Links` is not still-open item 10's oracle. It is item 9's missing signal.**
  Measured: **4 of 36** installed ids, all `portable`, and every EXE/MSI
  application appears in neither `Links` nor `Packages`;
  `C:\Program Files\WinGet\Links` does not exist on a14 at all. What `Links`
  really shows is that `Packages\<id>_<sourceIdentifier>\` is structurally the
  same thing as `$SCOOP/apps/<name>/`, so it can fill `Running.dirs`.
- **Still-open item 9's class is wider than "a second alias".** The item is
  written from `xh` / `xhs`. Measured: `rg` is ripgrep's **only** command and it
  is invisible, because `BurntSushi.ripgrep.MSVC`'s last dotted segment is
  `MSVC`. The class is "the process name is not derivable from the id", and a
  second alias is one member of it. `winget::guard_names`' doc comment carries
  the corrected framing.
- **Still-open item 2's mechanism holds; its framing does not.**
  `Microsoft.VCRedist.2015+.{x64,x86,arm64}` all carry `Source: winget`, so they
  do reach `installed` and are reported — item 2 is right about that. They are 3
  of **36**, about 11 of which are runtimes nobody declares, including
  `Microsoft.AppInstaller`, which is winget itself. With **0** winget packages
  declared in the measured `pkg.toml`, a dependency-aware fix has no manifest to
  read a `Dependencies` list from and suppresses **0 of 36 lines**. The defect is
  the volume of `Unmanaged`, not a missing dependency vocabulary.
- **Still-open item 11 is falsified in the direction that matters** (§5, above:
  0 of 105 on the reader, 3 of 10 on the writer, whose failure has been a warning
  since `src/update.rs:410`). Two things item 11 does not say and should:
  - **`--keep-going` is not the documented escape hatch it is called.**
    `gate_removals` holds **every** removal step whenever `is_ok()` is false,
    scoop's included.
  - **`status` is already resilient.** It never calls `version_liveness`; it uses
    `backend::scan_or_warn` (`main.rs:481` on this tree; the measurement document
    records `:468`, from the base tree this phase then edited), which exists so a
    winget hiccup cannot abort scoop's half. Item 11 is an `apply` problem only.

### `--show-unmanaged` does not restore the previous output byte for byte

`docs/specs/2026-08-11-phase5-guard-unmanaged-retry-design.md:251-252` says
`--show-unmanaged` "restores the previous output byte for byte". **Measured
against the shipped code:** it restores every per-package `? backend` line and
drops the hint, but the summary line carries the new `, N unmanaged` clause on
**both** paths — `render`'s clause is gated on `plan.unmanaged_count() > 0`, not
on `show_unmanaged`, and the base tree at `1d633c6` had no such clause at all.
The code is right and the test that pins it says why ("The clause stays: the count
is true in both forms"); it is the design sentence, and the same sentence in this
phase's brief, that overstate. The accurate claim is: **`--show-unmanaged`
restores every line the collapse removed, and the summary gains one clause that
no run had before.**

### Two `.rs` comments this phase falsified, both corrected here

Recorded rather than left, because a false comment in the shipped tree is the one
kind of stale sentence a reader cannot be warned about from a document.

- **`src/model.rs`, in `covers_any_sees_a_guard_name_that_covers_name_cannot`:**
  *"For a winget package `dirs` can never contain the id (it is filled from the
  scoop root alone)."* Written in Phase 4b, true then, and **falsified by this
  phase's own Task 2** — `backend::winget::running_ids` inserts winget ids into
  `dirs` for the `portable` subset, and the same file's type-level doc comment,
  which this phase rewrote, now says so. The file contradicted itself. Rewritten
  to state what the test actually relies on: `covers_name` has no `bins` half,
  this fixture's `dirs` is empty, and `Brave.Brave` could not be in it in
  production either, because it is measured (§3) to be an EXE/MSI package with no
  package directory.
- **`src/backend/winget.rs`, in
  `update_source_retries_once_on_the_measured_contention_failure`:** *"`source
  update --name winget` exited 0 of 10 times alone and 3 of 10 with another winget
  process alive."* Read literally that inverts the measurement — `0` is winget's
  success code, and what was measured is **nonzero** 0 of 10 and 3 of 10. Written
  by this phase (Task 6). Rewritten to the same file's authoritative phrasing at
  `INTERNAL_ERROR`'s own doc comment ("exited nonzero 0 of 10 times run alone and
  3 of 10 with one other winget process alive"), so the three copies of this one
  measurement cannot drift again, and with the full document filename, which also
  closes one of the shorthand citations listed under "Left open".

### Thirteen `file:line` citations corrected inside the shipped `.rs` files

The same defect class as the two comments above, but caused by *line drift*
rather than by a wrong sentence. **Nine** were correct when written and were
falsified by a later commit on this branch: five found by the whole-branch review
(Important 1), four more by the fix wave's own re-sweep of every
`<file>.rs:<line>` citation in `src/`, comparing each target's line *content* at
`1d633c6` against this tree. **Four** more were already wrong at `1d633c6` and
are inherited rather than this branch's; they are listed separately below. All
thirteen are corrected in the shipped tree; the table is here because the *cause*
is worth naming, not the numbers.

Task 5's `5c4894c` added `Plan::unmanaged_count` and shifted `src/plan.rs` by
exactly **17** lines and `tests/cli.rs` by **20**; Task 2 removed
`Scoop::running_set` and shifted `src/backend/scoop.rs` by **-16**; the phase's
535 added lines shifted `src/backend/winget.rs` by **+171/+184** at the two
points cited into it.

| site | said | now |
|---|---|---|
| `src/backend/mod.rs`, `running_set` | `src/plan.rs:414`, `:462` | `:431`, `:479` |
| `src/backend/mod.rs`, `running_set` | `src/plan.rs:345` | `:362-369` |
| `src/backend/mod.rs`, `apply_guard_overrides` | `src/plan.rs:414`, `:462` | `:431`, `:479` |
| `src/backend/mod.rs`, `apply_guard_overrides` | `src/plan.rs:345` | `:362-369` |
| `src/apply.rs`, `sample_fence` | `tests/cli.rs:980` | `:1000` |
| `src/main.rs`, `reconcile_ghosts` test | `src/backend/scoop.rs:299-303` | `:283-287` |
| `src/backend/winget_exec.rs`, `list_one_argv` | `src/backend/winget.rs:696`, `:772` | `:867`, `:956` |
| `src/render.rs`, `render_execution` | `src/render.rs:303` | `:493` |
| `src/render.rs`, a mutation-reasoning test | `src/render.rs:286` | `:443` |

And the four inherited ones, described below — all four off by **exactly 44
lines**, which is one un-repointed historical shift, not four independent
mistakes:

| site | said | now |
|---|---|---|
| `src/apply.rs`, `a_prune_from_a_backend_that_is_neither…` | `apply.rs:849`, `:814` | `:912`, `:858` |
| `src/apply.rs`, `a_scoop_action_carrying_a_readytoset…` | `apply.rs:837` | `:900` |
| `src/apply.rs`, `guard_for_needs_both_the_right_backend…` | `apply.rs:911` | `:974` |

**Why this is not bookkeeping.** The four `backend/mod.rs` citations carry the
structural argument that an `opaque` id can never reach either fence, which is
the entire reason `winget_fence_ids` may return `installed` only. A maintainer
who opens `plan.rs:414` finds a comment about scoop's `install.json`, cannot
verify the claim, and is left to either weaken the claim or "fix" code that is
correct. `docs/phase5-notes.md` already cited `plan.rs:362-369` correctly, so the
notes and the code disagreed about one fact in the same tree.

**Two of the nine were already imprecise before this branch** — `render.rs`'s
`:303` was two lines off its target function and `:286` two lines off its
expression at `1d633c6` — and this branch's own edits then moved both into
*different functions*, which is what turned an off-by-two into a false pointer.
Corrected on the same footing as the other seven.

**The four inherited ones, fixed in the same pass and labelled so the blame is
not misplaced.** `src/apply.rs`'s `plan_to_steps` routing tests cited
`apply.rs:814`, `:837` and `:849` for the scoop-`Prune`, winget-`Set` and
winget-`Remove` guards, and its `guard_for` mutation test cited `:911` for that
function's `.find`. On this tree those four are at `:858`, `:900`, `:912` and
`:974`. They were **equally wrong at `1d633c6`** — the citing comments and their
targets are byte-identical there, and all four were off by the same 44 lines, so
one edit long before this branch moved the code and left every pointer behind at
once. This branch did not break them; it did move three of them further out of
date, and leaving four known-false pointers in a tree whose whole review round
was about false pointers would have been the wrong call. The two sibling
citations in the same comments (`:834`, `:853`) were correct and are unchanged.

**Not claimed: that thirteen is all of them.** The sweep catches *drift* — a
citation whose target line changed content between `1d633c6` and this tree — and
cannot catch one that was wrong from the start and whose target never moved. The
four above were found only because a human read the comments around them, after
the sweep had flagged their *neighbours*. A mechanical guarantee would need the
citations to be checkable, which line numbers in prose are not.

### The design attributes 105 invocations to one argv

`docs/specs/2026-08-11-phase5-guard-unmanaged-retry-design.md:85-87` reads *"the
reader argv `version_liveness` uses returned **0 nonzero exits in 105
invocations**"*, and the Scope table at `:20` compresses it further to "0 of 105".
**Twenty of those 105 are `list -e --id <id>`**, which is `list_one_argv`'s shape,
not `version_liveness`'s. This is the same defect the Task 7 review caught in two
code comments — the code now states the 85 / 20 split correctly at all three sites
that mention it — and it went unrecorded against the design that seeded it. The
measurement document itself is not wrong: §5's table lists the argvs separately
and only totals them in a `| | | **105** | **0** |` row.

### Ten historical `docs/` sentences this phase falsifies

Found by Task 2's sweep and carried here rather than edited: these documents keep
their stale sentences by design, and this is where a reader learns they are
stale. The sweep also confirmed the line-based-grep trap it was warned about —
`grep -rn "only the first two can ever fire" src/ docs/` returns **zero** content
hits purely because every real occurrence is line-wrapped. The list below came
from reading, not from a count.

**What the sweep covered, stated instead of a completeness claim.** This list
said the nine were "listed only so the sweep is provably complete". It was not
provable and it was not complete: the whole-branch review (Minor 5) found a
tenth, item 10 below, which the sweep had missed because it was reading for the
`scoop-only` / `covers` wording rather than for every name this phase deleted.
What the sweep actually did was read `docs/` for the sentences about
`Running::covers`, `dirs` being scoop-only, and `resolve`; it was never a
mechanical enumeration of every identifier this phase removed, and a
line-wrapped-text corpus does not admit one by grep. So: **ten found by reading,
no claim that ten is all of them.**

1. **`docs/phase4-notes.md:19-27`** ("Read this first"): *"of `Running::covers`'s
   three signals (package directory, process name, declared executables), only
   the first two can ever fire for a winget package"*, and its closing claim that
   a winget process whose package directory is not name-matchable *"would be
   invisible to the running-process guard"*. **Stale in two directions at once:**
   Phase 4b filled `bins` via `guard_names`, so the third signal fires; and this
   phase gave winget the *first* signal for the `portable` subset. A reader
   correcting only the directory half would leave the `bins` half wrong.
2. **`docs/specs/2026-08-10-phase4b-winget-executor-design.md:37-39`** — quotes
   the `phase4-notes.md` sentence above.
3. **…:40-41** — quotes `src/backend/winget.rs:244-249` as *"with `bins` empty
   only the first two can ever fire."* That source comment no longer exists in
   that form.
4. **…:42-44** — quotes
   `docs/specs/2026-08-09-phase4-backend-winget-design.md:336` as *"`bins` stays
   empty and `Running::covers` therefore falls back to its name and directory
   halves for winget."*
5. **…:209-210** — *"`Running`'s own doc comment gains the sentence the three
   documents above got wrong: `dirs` is scoop-only by construction."* That
   sentence is exactly what this phase deleted from `src/model.rs`; `dirs` now
   carries both backends. Directly superseded.
6. **`docs/specs/2026-08-09-phase4-backend-winget-design.md:336-339`** —
   *"`bins` stays empty and `Running::covers` therefore falls back to its name
   and directory halves for winget … the running-process guard is **weaker for
   winget than for scoop**."* The weakness is real and narrower now: winget's
   fence is `dirs` for `portable` ids, `names` for everything a user declares or
   `guard_names` happens to guess.
7. **…:13** — *"`resolve` is a free scoop-only function"*. Stale since Phase 4
   Task 13 moved it onto `Backend`; not this phase's doing, but it is part of the
   same `scoop-only` sweep and belongs in one place.
8. **`docs/phase4b-notes.md:447-449`** — *"`Running::covers` is not 'weaker' for
   winget. It was empty. Three documents said 'falls back to its name and
   directory halves'; the directory half cannot fire."* **The most pointed of the
   nine: it is itself a corrections entry that now needs correcting.** The
   directory half *does* fire, for the 4-of-36 `portable` subset. Its own
   supporting measurement ("`Running.dirs` contained exactly one entry on a14, a
   scoop app") remains true of the tree it was written against.
9. **`docs/specs/2026-08-11-phase5-guard-unmanaged-retry-design.md:153`** and
   **`docs/superpowers/plans/2026-08-11-phase5-guard-unmanaged-retry.md:68, 367,
   481, 573, 575, 597, 598, 617`** — this phase's own spec and plan, describing
   the change as pending. Correct as forward-looking documents; listed so a
   reader who greps for the old wording finds them accounted for.
10. **`docs/dogfood-phase2a-2026-08-08.md:467`** — names `Scoop::running_set`
    inside a live, present-tense description of what the `status` path does
    ("Nothing in `dotpkg`'s `status` path (`config::load`, … `Scoop::running_set`,
    `plan::plan`, `render::render`) performs a write, a subprocess spawn, or a
    network call"). Task 2 deleted that function when `backend::running_set`
    became the one fence producer; `apply::sample_fence` occupies that slot
    today. **And the sentence around the name is stale in a second, larger way
    this entry should not hide:** `status` now calls
    `backend::scan_or_warn(&winget)`, whose `RealWinget::run` is a
    `Command::new("winget")` — so "performs no subprocess spawn" stopped being
    true of the `status` path when winget got a backend, two phases before this
    one. **Structural**, provable by reading `src/main.rs`'s `status` arm. Left
    in place like items 1-9, and for the same reason: those documents keep their
    sentences and this is where a reader learns they are stale. Found by the
    whole-branch review (Minor 5), not by Task 2's sweep — which is the evidence
    for the paragraph above about what that sweep could and could not prove.

### Four corrections made in place, in this phase's own measurement document

**§3 recorded the two machine-scope `Links` directories as absent but never said
`%ProgramFiles%\WinGet\Packages` had been probed.** It was, and it is absent —
raw probe output `--- dir: C:\Program Files\WinGet\Packages` followed by
`ABSENT`. That line is now in §3. It matters because
`src/backend/winget.rs`'s `package_roots` cites §3 as measured for exactly that
absence, and a reviewer read the section and correctly found it did not support
the claim. The code comment was true and the document was incomplete, so the
document was fixed rather than the comment weakened. Corrected in place, not by
an entry above, because it is this phase's own document and the gap is the
measurer's.

**§1 cited a sentence this phase then deleted.** It pointed at
`src/backend/scoop.rs:212-214` for "an elevated process reports no `exe` and is
caught only by name", which was **correct against `1d633c6`** — that sentence lived
in `Scoop::running_set`'s doc comment, and Task 2 deleted that function when
`backend::running_set` became the one fence producer. So the phase broke its own
document's pointer. §1 now cites the surviving statement of the same claim,
`Scoop::running_apps`' doc comment at `scoop.rs:181-182`, with the same one-line
justification recorded beside it. Same treatment as §3, for the same reason: a
citation that resolves to nothing is not a superseded sentence, it is a dead one.

**Three of its `file:line` citations had drifted onto this phase's own code, and
all three are re-pointed in place.** Same defect class as the `.rs` citations
below, same cause — the phase edited the files it was citing:

| section | said | now | what moved it |
|---|---|---|---|
| §4 | `plan.rs:532` (winget's `helpers: &[]`) | `plan.rs:549` | Task 5's `5c4894c`, +17 lines |
| §5 | `main.rs:613` (`!preparation.is_ok() && !keep_going`) | `main.rs:641` | this phase's `apply` arm |
| §5 | `main.rs:468` (`backend::scan_or_warn`) | `main.rs:481` | this phase's `status` arm |

Each was correct when written — verified against `1d633c6`, where all three
resolve to exactly the cited code. Found by the fix wave's own sweep, not by the
whole-branch review. Corrected in place with the old number and its cause
recorded beside it; **no measurement changed**, and this is the same treatment §1
and §3 got, for the reason given there: a citation that resolves to the wrong
line is a dead pointer, not a superseded one.

## Verification

### macOS suite

`cargo test --no-fail-fast`, **re-measured on the tree that ships this file**
— the whole-branch fix wave's tree, three commits past the `b2fdc58`/`137fc35`
pair this section was originally written against: **634 passed, 0 failed, 0
ignored**, across **14** `test result:` lines (`unittests src/lib.rs` 307,
`unittests src/main.rs` 14, the eleven `tests/*.rs` binaries, and
`Doc-tests dotpkg` 0). Base `main` at `1d633c6` was 588, so the phase adds 46
tests.

**This sentence used to read "measured on the tree this file describes" while
naming 631**, which was Task 8's total at `137fc35`. Two commits (`05023fd`,
`c8c7f0d`) then landed and the file did not move, so the sentence named a tree
that was no longer the one it shipped on — the whole-branch review's Important 2.
The 631 was not wrong, it was unowned: `05023fd` and `c8c7f0d` added no tests, so
631 held through `c8c7f0d`, and the +3 to 634 is this fix wave's own — two in
`src/render.rs` for the `render_preparation` summary clause and one in
`src/backend/winget.rs` for the retry's `attempt == 0` guard.

Also measured now, on the same tree:

- `cargo fmt --check` — exit 0, no output.
- `cargo clippy --all-targets -- -D warnings` — exit 0, zero warnings.

### Windows target, cross-checked from macOS

`cargo check --target aarch64-pc-windows-msvc --all-targets` — exit 0, zero
warnings, re-measured on the same tree as the suite above. This type-checks every
`#[cfg(windows)]` path from macOS
and is explicitly **not** a substitute for running the suite on Windows: it
catches compile errors on the Windows target, not behavioural differences.

### Fixture integrity

`tests/fixtures/winget/list-full.txt` is **30958 bytes with 143 CRLF pairs**,
re-measured on the same tree — exactly the values `PROVENANCE.md` and `docs/phase4b-notes.md`
record, and `.gitattributes` pins the path `-text`. **No fixture bytes changed in
this phase**: the branch diff touches no file under `tests/fixtures/`. Checked
before any Windows result is trusted, because a fixture normalised by a checkout
makes every downstream assertion meaningless.

### The mutation run was attempted and refused to start

**Not "has not run yet". It ran, and the gate failed before testing a single
mutant.** `cargo mutants --in-diff -j 2 --timeout 600` against the branch diff
reported:

```
cargo test failed in an unmutated tree, so no mutants were tested
```

The baseline — the unmutated tree, the run's own control — went red, so zero
mutants were built and zero were killed. This section previously said only that
the mutation run "ha[s] not run", which reads as a scheduling fact when what
actually happened was a gate failing. Recorded as a failure, because the
difference is the whole point of this section.

**The cause was a pre-existing instance of a defect class this project already
names.** The failing test was
`apply_prepare_also_sees_the_winget_scan_and_stays_quiet_about_it`
(`tests/cli.rs`), at `assert_nothing_was_touched`'s `assert_eq!`. **Measured**,
by parsing both `Snapshot` values out of the panic dump in `mutants.log` rather
than taking the failure on report: the two differ in **exactly one of 58
entries**, and it is `buckets/main/.git/objects/maintenance.lock` — written by
**git's own background maintenance** inside the fixture's temp bucket repo,
between the `before` snapshot and the `after` one.

So a helper whose entire job is to prove *dotpkg* wrote nothing reported *git's*
housekeeping as dotpkg's write. That is the inverse of this project's named
recurring defect: not a test that cannot fail, but one that **fails for a reason
unrelated to what it asserts**. Both the helper and the fixture predate this
phase, and `assert_nothing_was_touched` has **14 call sites** in `tests/cli.rs`,
every one of which carried the same exposure. It had never surfaced under a plain
`cargo test`: only the load and the relocation `cargo mutants` imposes moved the
timing far enough for git's maintenance to land inside the window. Disk was never
the constraint — 14 GiB free throughout, no starvation.

> The count is **14**, not the 16 first recorded in `05023fd`'s own commit
> message and repeated into the whole-branch review. 16 was a `grep -c` over
> lines *mentioning* the helper — which at `137fc35` were the 14 calls, the `fn`
> definition and one comment. Re-derived here by counting
> `f.assert_nothing_was_touched(before);` (14 at `137fc35` and 14 today). A
> line-count standing in for a call-count is the same measurement error this
> phase's own sweep was warned about.

**The fix removed the cause, not the symptom.** Every `git init` in this repo's
test infrastructure now sets `gc.auto 0` and `maintenance.auto 0` immediately
afterwards — **six sites**, which is every `git init` in the tree: one in
`tests/cli.rs` (`write_lock_and_bucket_for`), one in `tests/common/mod.rs`
(`Fixture::bucket`), **three** in `tests/prepare.rs`, and one in `src/apply.rs`'s
`#[cfg(test)]` module. The last of those is stated at its own site as
**prophylactic and never observed failing** — that test asserts no disk snapshot,
so it is not currently exposed. `maintenance.auto` is the trigger measured to
create that exact lock file; `gc.auto` is the older, separate auto-gc trigger
whose own lock is `gc.pid` and which was *not* observed here, disabled alongside
it as *reasoned*, and labelled that way at the site.

Filtering `*.lock` out of `Snapshot::of` was considered and **rejected**: it
would also hide a real lock file dotpkg leaked, which is precisely the kind of
write this assertion exists to catch. `Snapshot::of` and
`assert_nothing_was_touched` are untouched — the fixture's environment was made
deterministic, the assertion was not weakened.

**The structural finding underneath it, which would otherwise have stayed in the
git-ignored ledger:** `tests/prepare.rs` carries its **own private copy** of the
`git()` helper instead of importing `common::git` (`tests/prepare.rs:8` versus
`tests/common/mod.rs:14`). That duplication is why the fix needed six sites
rather than two, and it is the mechanism by which the defect returns — the next
person adding a temp-repo test copies whichever helper is nearest, and the new
`git init` arrives without the two `config` calls. **Structural**, provable by
reading. Not fixed here (collapsing the two helpers is a test-infrastructure
change with no behaviour attached, and this fix wave is a correction pass); it is
on the still-open list below.

**Proven against the failure mode, not against a lucky re-run:** `cargo mutants
--file src/sys.rs -j 2` afterwards reported `ok Unmutated baseline in 9s build +
5s test`, where the same step had aborted before. The config was verified to land
on a real fixture repo by temporary instrumentation inside
`write_lock_and_bucket_for`, then reverted.

### The Windows suite, the dogfood and the full mutation run have not run

They are Task 9's, and until Task 9 records them in this section, this phase has
**no** Windows-run, dogfood or completed-mutation evidence at all. Read that as
absent, not as pending-and-probably-fine — this is the exact place
`docs/phase4b-notes.md` shipped a stale claim, and its post-merge audit's best
finding was that its own pre-merge watch-list item had gone unmet without the
notes saying so. The `src/sys.rs`-only baseline above is evidence that the gate
can now *start*; it is not a mutation result.

The standing rules carry unchanged: the Windows suite runs on the tree that
ships and is cross-referenced **name by name**, never by subtracting totals;
`cargo mutants -j 2` on an idle machine with nothing editing the tree
concurrently; fixture bytes checked first.

Three things in this phase are reachable **only** from that Windows work, and
they are the reason it is a gate rather than a formality:

- `backend::winget::package_roots()` returns an empty vector on macOS, so the
  entire winget path signal is exercised on this platform only through
  `sample_fence_with_roots`' fabricated roots. No macOS run has ever seen it read
  a real `LOCALAPPDATA`.
- The `opaque` warning branch in `apply_guard_overrides` and the guard-merge half
  have **unit coverage only**. `tests/cli.rs` strips winget from `PATH`, so no
  `cli.rs` test can produce a sourceless row or an installed winget package —
  and `opaque` is the majority case on real hardware (90 of 126 ids in this
  round's own capture).
- The retry's real 1 s delay, and `INTERNAL_ERROR` arriving from a real winget
  rather than from `ScriptedWinget`.

## Deferred minors, by originating task

Closed inside execution, at each task's own fix rounds: the trailing-separator
root that made `running_ids` silently return empty; two dead-code comments that
each claimed the wrong reachability; `mod.rs`'s "three sites call `running_set`"
after the hoist; a `§4` citation for a measurement in `§3`; two "measured"
labels on a one-test transcript; a comment ruling out a state that never existed;
a comment presenting a structural guard as a live hazard; a test name that
overstated what its fixture could prove; a `bins`-writers list missing scoop's
`declared_executables`; a dropped hedge ("is meant to cover" → "covers"); a
warning that said "nothing installed by that name" about an installed-but-opaque
package (which became a code change rather than a rewording: a separate `opaque`
arm with its own message and its own test); the unpinned `[winget.guard]`
value-dedup; the unpinned
`declared` argument at both production sites; the hint that could have printed
once per backend; `apply --show-unmanaged` being exercised by no test at all; the
retry delay's wrong-probe justification; a constructor count; two ellipsis
citations plus a third inherited from an earlier task; the "six times before"
count; and Task 7's argv attribution, twice. Task 9's whole-branch review
triages whatever remains.

**Left open, each with the reason it can stay:**

- **`main.rs` holds two `sample_fence` calls no test observes**, and the hoist
  **removed** the incidental `unused variable: fence_ids` warning that used to
  guard one of them (measured; see "Three wiring sites" above). Parked with a
  ruling: closing the two remaining doors — making `sample_fence_with_roots` and
  `backend::running_set` `pub(crate)` — would narrow *argument passing* without
  pinning site 3's call, which is the actual residual, and it would require
  relocating three `tests/scoop_scan.rs` integration tests into the library. The
  hole is irreducible in this plan: `cli.rs` observes `main.rs` but strips winget
  from `PATH`, so the winget half is unreachable from the only black-box harness
  that exists.
- **A `[winget.guard]` entry on a genuinely opaque winget id warns on every run,
  with no way to silence it short of deleting the entry.** Ruled: keep it. The
  entry really does protect nothing — `plan.rs:362-369` skips an opaque package
  before either fence check — and the silence it replaced was the actively
  misleading option. If it proves noisy the fix is to say it once per run, not to
  drop the branch.
- **The `opaque` warning branch and the guard-merge half have unit coverage
  only**, for the structural reason above (`tests/cli.rs` strips winget from
  `PATH`). Task 9's dogfood is the first thing that would exercise either on real
  hardware; nothing has yet.
- **`package_roots()` has no direct test anywhere.** A swap of its two
  environment-variable names, or an extra `Microsoft` segment on the
  machine-scope branch, would pass every test in the file; the asymmetry between
  the two branches was verified by reading against the measurement document
  instead. Likely the right call — `std::env::var` mutation across parallel Rust
  tests is its own hazard — but recorded rather than silently accepted.
  **Partially closed by Task 9b** (commit `ee46172`): the path construction was
  extracted into `package_roots_with(local_appdata: Option<&str>, program_files:
  Option<&str>)`, a pure function taking the two variables as plain values
  instead of reading `std::env` itself, with four new tests asserting the exact
  paths — including the `"Microsoft"` segment on the user-scope branch and its
  absence on the machine-scope one — with no environment mutation anywhere.
  `package_roots()` itself now just reads the two variables and delegates, and
  stays genuinely untested; that residual moved, it did not close. See "Still
  open" item 19 for what a mutation run confirmed about it.
- **`ScriptedWinget` is a second `WingetCmd` fake in the crate**;
  `src/apply.rs`'s test module already has `FakeWinget`. A reviewer may fairly
  call that duplication. It stands because `FakeWinget` lives in a different
  module's `#[cfg(test)]` and is unreachable from `winget.rs`, and the
  alternative is making it `pub(crate)` and dragging its five canned constructors
  along.
- **`prepared_line`'s `ArchDrift` arm has no literal space before its
  parenthesis** where `render(plan)`'s equivalent arm does, so a value that fills
  the 18-column pad runs into it: `64bit, declared arm64(architecture drift --
  reported, not fixed)`, observed live. **Pre-existing** — verified byte-identical
  at `main` (`1d633c6`), untouched by this phase, and in the `~` line rather than
  the `?` collapse this phase changed.
- **Three citations in `winget.rs` still use the shorthand
  `measurements-2026-08-11 §N` with no filename** — `:907`, `:1338`, `:1588` on
  this tree. Not ellipses and not factual errors, but inconsistent with the
  now-fully-named citations in the same file. A fourth, at the retry test, was
  named in full by this task's own comment fix. (The ledger recorded one of them at
  `:1477`; that line number had already drifted by the end of the branch, which is
  its own small instance of the class this phase kept fixing.)
- ~~**`winget.rs:1077-1088` (`:1077-1092` after the fix) read "P2 S2's 40, P2 S4's 15, and P7's 30 against a
  continuously running `source update`"**, which can be read as putting S4's 15
  under the continuous loop too.~~ **Fixed** by the whole-branch fix wave, which
  triaged it fix-before-merge. Only P7's 30 were continuous; S4's 15 were
  concurrent with one `source update`, a distinct condition, and S2's 40 state no
  concurrency at all (§5's table lists the three separately). The arm now names
  all three conditions rather than listing three counts under one of them —
  the same care the top-level `INTERNAL_ERROR` doc comment in the same file
  already took ("0 nonzero exits in 105 invocations, including 30 fired against a
  continuously running `source update` loop").
- **`Phase 4b`'s `main.rs:773` citation has drifted.** The accepted equivalent
  mutant is on the `outstanding_skips` argument to `floor_exit_code`, which is at
  `main.rs:856` on this tree (verified by reading). The mutant itself is
  unchanged and still open — see below.

## Still open

Items 1-15 are `docs/phase4b-notes.md`'s list renumbered one for one, with each
item's status stated against it: 2, 9 and 11 are the three this phase rewrote, 10
is the one it deliberately did not close, and the rest are unchanged. Items 16-19
are new in this phase — 16 from Task 8, 17 and 18 from the whole-branch review
and the fix wave that answered it, and 19 from Task 9b.

1. **Downgrading a winget package.** *Decided, not deferred.* Unchanged from
   Phase 4b: measured, `install --version <older>` cannot do it, and the
   alternative would reintroduce a nightly uninstall-and-reinstall loop on every
   self-updating application.
2. **Dependency handling — rewritten. Rejected on the measurement, not
   deferred.** The framing is gone, and what remains is smaller and named: the
   VCRedist rows *do* reach `installed` and *are* reported (3 of 36), and with 0
   winget packages declared a dependency-aware fix suppresses 0 of 36 lines on
   the measured machine. The reporting symptom — an undeclared package appearing
   after an install — is now one collapsed line per backend rather than a flood.
   **Still genuinely open:** dotpkg has no vocabulary for a package it did not
   declare (no lock entry, no ownership record, no declaration), and the
   5-of-12-manifests survey is the only measurement of how often winget creates
   one. A hardcoded `WINGET_HELPERS` list was rejected explicitly: it would
   *exclude* those packages from `Unmanaged` rather than count them, which is a
   different and less honest thing than collapsing a line.
3. **`--location`, `--all-versions`, and side-by-side versions of one id.** All
   three unmeasured. Unchanged.
4. **Removing a machine-scope package while elevated.** Unmeasured; the Phase 4b
   refusal stays narrowed to user-scope rather than guessing. Unchanged.
5. **Any installer type other than `portable`, for the success paths.**
   Unchanged, and this phase gives the item a second edge: `portable` is also the
   only installer type the new path signal can ever see.
6. **`--force` and `--purge` against the elevation refusal.** Unmeasured.
   Unchanged.
7. **`winget pin`.** Unchanged: two sources of truth about permitted versions is
   how a tool starts lying.
8. **`add`, architecture drift, same-version re-pin, locking against two
   concurrent dotpkg runs, and Chocolatey.** All unchanged.
9. **The process name is not derivable from the id — rewritten, and answered
   only where a user says so.** Phase 4b wrote this as "a package's *second*
   alias is invisible" from `xh` / `xhs`; measured, `rg` is ripgrep's *only*
   command and is invisible too, so the class is wider. `[winget.guard]` lets
   `pkg.toml` name what winget will not, and `running_ids` catches the
   `portable` 4 of 36 by path. **What stays open:** there is still no scan-time
   source for a package's process names — `winget list` does not expose aliases
   at all; they appear only in `install`'s stdout, at install time — so a user
   who does not write the guard entry gets no protection for a non-portable
   package, and dotpkg cannot tell them which entry they are missing.
10. **There is no independent oracle for a winget mutation — NOT closed.** The
    `Links` lead was measured and is not it: `portable`-only, 4 of 36, and every
    EXE/MSI application absent from both `Links` and `Packages`. A
    directory-existence check is worse than nothing for this purpose
    (`PhatMT97.VKey.Classic_…\` still exists for an uninstalled package). The
    registry is not it either: 0 winget-shaped ARP keys in HKLM or WOW6432Node.
    **The strongest lead found, and deliberately not opened:**
    `%LOCALAPPDATA%\Packages\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\LocalState\Microsoft.Winget.Source_8wekyb3d8bbwe\installed.db`,
    **262144 bytes** — a winget-written catalog that is not portable-only.
    Reading it means bundling SQLite and depending on an undocumented internal
    schema. Recorded as a lead, not as a finding; nothing in this phase opened
    the file.
11. **A transient winget failure — rewritten. Now a decision with numbers behind
    it, not an open gap.** 0 nonzero exits across 105 reader invocations of two
    argvs, so no retry there; 3 of 10 on the writer, so one retry there, once,
    after 1 s, only on `INTERNAL_ERROR`.
    Item 11's own text was wrong twice over: `--keep-going` is not a full escape
    hatch (`gate_removals` holds every removal whenever `is_ok()` is false), and
    `status` never calls `version_liveness` at all, so this was only ever an
    `apply` problem. **What stays open:** whether `show` or `list` ever return
    `0x8A150001` under the same contention is unmeasured — the reader won 105 of
    105 races, and the asymmetry is inferred, not measured — and the 1 s delay is
    chosen, not measured to be sufficient on a slower machine.
12. **The `--prepare` loop is unmeasured as a loop.** Nothing is parallelised or
    cached, and the per-call ~1 s figure is still the only number there is.
    Unchanged.
13. **Every Phase 4 "still open" item not in this phase's scope** stays open,
    notably `plan_backend`'s unconditional `Arch::as_scoop()`; the design's "a
    new backend slots in without touching the planner" promise being half true;
    `verify.rs:146`'s `NotFound`-idiom guard; `floor_char_boundary` and the
    missing-`Version` refusal branch being untested defensive code; no fixture
    pairing a plain `show` with `show --versions` for the same package; and
    `resolve_installed`'s `fell_back_to_tip` warning path being untested.
14. ~~The 69 unresolved `timeout` mutants from Phase 4's final mutation run~~ —
    **settled by Phase 4b**, which measured 0 `TIMEOUT` over 253 mutants at
    `-j 2`. Left numbered so an old reference finds the resolution rather than a
    gap in the numbering. Unchanged by this phase.
15. **`sys::elevated()`'s runtime behaviour is measured in one direction only.**
    Unchanged, and untouched by this phase: an ordinary, non-elevated Windows
    session with **no `runas` at all** is still unmeasured. `elevated()` should
    answer `Some(false)` from `TokenIsElevated` alone and never consult
    `CheckTokenMembership`, and nobody has watched it do so.
16. **Neither `pkg.toml`-editing round-trip guard covers `[winget.guard]`** — new
    in this phase, and a **future-only** risk. `verify_round_trip` and
    `verify_round_trip_winget` compare the sections `adopt`'s two editors are
    allowed to touch and never read `after.winget.guard`, so a dropped or mangled
    guard table would pass both. Harmless today for one reason only, and it is not
    the guard: nothing in the tool writes `[winget.guard]` at all. It becomes real
    the moment any editor does — an `add` that writes the table, or a rewrite of
    `add_*_package` that touches more of the document than it means to. Structural,
    and the cheap fix is one clause in each guard rather than a new test.
17. **The mid-run fence's `dirs` half compares two different winget spellings,
    and nothing measures whether they ever differ** — new in this phase, found by
    the whole-branch review (Minor 7), **reasoned only**. `Running.dirs` holds
    `winget list`'s `Id` (`backend::winget_fence_ids` → `winget::running_ids`,
    which inserts the scanned id; `rows_to_scan` builds those from the `Id`
    column). The `Name` the per-step re-sampler asks `Running::covers_any` about
    is `Step::app()` — for a `WingetStep::Set`, the `id` out of
    `Outcome::ReadyToSet`, which is `winget show`'s `Id` as `parse_show` read it
    back. `Name` folds case, so only a difference that is **not** case would make
    the `dirs` half silently answer "not running" mid-run for a package the
    plan-time fence could see, and `guard_for`'s `bins` half would not
    compensate: it supplies plausible *process* names, a different signal, empty
    unless `[winget.guard]` names the package or `guard_names` guesses right.
    **No measurement either way** — nothing in this phase's measurement document
    compares the two spellings for one package — so this is an uncovered
    residual, **not a claim that the two ever differ**. `src/apply.rs`'s
    `plan_to_steps` winget `Set` arm now states it at the site, next to the
    `set_argv` two-spellings discussion that had covered only the argv half.
18. **`tests/prepare.rs` duplicates `common::git` instead of importing it** —
    `tests/prepare.rs:8` versus `tests/common/mod.rs:14`. **Structural.** Surfaced
    by the background-maintenance fix above, which needed six `git init` sites
    instead of two because of it, and it is the mechanism by which that defect
    returns: the next temp-repo test copies whichever helper is nearest and
    arrives without the two `config` calls. Left open deliberately — collapsing
    the two helpers is a pure test-infrastructure change with no behaviour
    attached, and the fix wave that found it was a correction pass. The cheap
    version is one `mod common;` in `tests/prepare.rs`; the durable version is a
    single fixture-repo constructor that no caller can bypass.
19. **Both survivors of Task 9's `--in-diff` mutation run are now on
    `package_roots()` itself, not on the logic Task 9b extracted from it, and
    neither is closed.** Task 9b (commit `ee46172`) split `package_roots()` into
    that thin, still-untested wrapper and a new pure function,
    `package_roots_with`, that takes the two environment values as plain
    `Option<&str>` parameters and does the actual path construction. The same
    `--in-diff` mutation run, re-run against `ee46172`, reports **72 mutants
    tested in 3m: 2 missed, 59 caught, 11 unviable** (`mutants.out`, complete —
    `start_time`/`end_time` both present, `missed.txt` holding exactly these two
    lines, `caught.txt` 59 lines, `unviable.txt` 11 lines):
    ```
    MISSED src/backend/winget.rs:251:5: replace package_roots -> Vec<std::path::PathBuf> with vec![]
    MISSED src/backend/winget.rs:251:5: replace package_roots -> Vec<std::path::PathBuf> with vec![Default::default()]
    ```
    The two survivors are the same pair as before, moved from `winget.rs:241`
    (the old, undivided function) to `winget.rs:251` (the new delegating
    `package_roots()`) — 57 caught became 59 (the two new `package_roots_with`
    mutants, both now caught), and the 2 missed stayed 2, on the wrapper. Nothing
    in the suite calls `package_roots()`: every test that reaches the winget path
    signal goes through `apply::sample_fence_with_roots` with fabricated roots
    instead (see "The Windows suite..." above), so no test observes either
    mutation. The two mutants are not the same *kind* of gap and are
    characterised separately for that reason:
    - **`vec![]` is an equivalent mutant on macOS, not merely uncovered.**
      `LOCALAPPDATA` and `ProgramFiles` are both unset on every macOS run of this
      suite, so `package_roots()`'s real, correct output on this platform is
      already `vec![]` — identical, byte for byte, to what the mutant returns.
      No test on this platform can distinguish two functions whose real outputs
      are the same value; that is not a coverage gap, it is arithmetic. Same
      bucket as this file's own three `#[cfg(windows)]` `sys.rs` mutants above:
      "inert on macOS ... a platform gap, not a test gap, only resolvable by a
      mutation run *on* Windows."
    - **`vec![Default::default()]` is not equivalent, only unreached.** It
      returns `vec![PathBuf::new()]`, one element whose folded prefix is `"/"`.
      On a Unix-like machine every absolute path folds to a `/`-prefixed string
      too, so this mutant is not distinguishable *by that property* on macOS
      either — but the real reason no test catches it is simpler and platform-
      independent: nothing calls `package_roots()` at all, so nothing is ever in
      a position to notice its return value is wrong.
    - **Both are resolvable the same way: a mutation run *on* Windows**, where
      `LOCALAPPDATA`/`ProgramFiles` are genuinely set and `main.rs`'s production
      call path (`apply::sample_fence` → `package_roots()`) is live, so a test
      exercising that path would see a real, non-empty, correctly-shaped answer
      that either mutant would visibly break.
    **What the split did close:** the part of the original gap that was about
    *logic*, not *plumbing*. `package_roots_with` is now pinned by four tests —
    both values present, each absent alone, both absent — each asserting the
    exact resulting paths. The `"Microsoft"` segment the user-scope root carries
    and the machine-scope one does not is one of the things they pin: swapping
    which branch gets it, or swapping the two parameters, turns a test red, with
    no `std::env` mutation anywhere in any of them.

### Inherited verification debt, carried unchanged

None of these is in this phase's scope, and none of them moved. They are listed
so the next phase inherits a named list rather than a surprise.

- **An ordinary non-elevated Windows session with no `runas`** has never been
  measured (item 15 above).
- **Three `#[cfg(windows)]` mutants in `sys.rs`** — two in `elevated()`'s
  `Some(true)` / `Some(false)` returns, one in a `!=`/`==` inside it. Not test
  gaps: that body is not compiled off Windows, so no macOS mutation run can
  exercise them. Resolvable only by a mutation run *on* Windows.
- **The accepted equivalent mutant on the `outstanding_skips` check** that Phase
  4b recorded at `main.rs:773`; the call is at `main.rs:856` on this tree.
  Closing it needs a fake scoop binary (which a standing test policy forbids) or
  a production change.
- **14 mutants in `src/backend/winget.rs`, in Phase 4 code** —
  `floor_char_boundary` (6), `parse_list` (6), `parse_versions` (1),
  `RealWinget::run` (1). This phase added **611** lines to that file (against 15
  deleted) and closed none of them — `git diff --numstat 1d633c6 --
  src/backend/winget.rs`, measured on the tree this file describes. The number
  read 534 until the whole-branch review (Minor 2) re-derived **535** against
  `c8c7f0d`; the fix wave that answered the review then added 76 more — the new
  retry-delay test, its fake's instant recorder, and the reworded
  `INTERNAL_ERROR` arm. **Re-observed, not re-measured, by Task 9b:** a
  `cargo mutants --file src/backend/winget.rs` run (dispatched to check Task 9b's
  own fix, not this list) found the same 14 mutants missed, all in these same
  two functions, before the run was interrupted partway through and never
  printed its own summary line — so this is a re-observation consistent with
  the count above, not a fresh, complete, authoritative one.
- **Two mutants in `winget_exec.rs`, inside `RealWingetMutator::run`** — a
  `NotFound == -> !=` and an `unwrap_or(-1) -> unwrap_or(1)`. Covering them means
  spawning a real `winget.exe` from the test suite, which this project does not
  do.
