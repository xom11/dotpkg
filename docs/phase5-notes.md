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
measured on, and the Verification section says plainly what each run did and did
**not** establish rather than implying it went well.

**All three of the runs that section used to list as absent have now happened**,
and each names its own tree rather than borrowing one — this paragraph used to say
"all on `4bbe3be`", which was true of the dogfood and of the Windows suite's first
run, and never of the mutation run:

- the **Windows suite**, on `4bbe3be`, and then a second time on `765e091` (see
  "And then the tree moved twice more" two paragraphs below);
- the **dogfood**, on `4bbe3be`;
- the **mutation run**, on neither — it completed **twice** while the branch was
  open, on `4673517` (70 mutants) and on `ee46172` (72), both of them commits
  *before* `4bbe3be` existed, and then a **third time after the merge**, scoped
  by file rather than by diff: **618 mutants on `8ed3de0`** (see "The file-scoped
  mutation run" under Verification). That third run corrected four numbers this
  file had inherited and never re-derived, and closed none of them.

Why the two branch-time mutation runs' numbers still describe the code that merged
is an argument about *attributability* rather than a measurement taken on a later
tree, and it lives where the runs are recorded ("The mutation run" under
Verification) instead of being restated here — collapsing it into "measured on the
shipping tree" is exactly the move the promise two paragraphs above forbids. The
third run needs no such argument: it ran on the merge commit itself.

**Two of the questions the dogfood was meant to settle** came back
**inconclusive**, and those are recorded as inconclusive and given
numbers in the still-open list (items **20** and **21**) rather than filed as
passes: the retry has never been observed to fire, and the winget path signal could
not be isolated from `guard_names` on the one live subject the machine offered. A
run that happened is not the same as a question that was answered, and this file
tries not to spend the first to claim the second.

**And then the tree moved twice more, and one of those three runs had to happen a
second time.** `6b2211e` (the commit that first wrote the claim the bullets above
now correct) is docs only;
`765e091` changes one comment line in `src/backend/winget_exec.rs` in addition to
documentation. Both land before this file merges, so `4bbe3be` stopped being "the
tree that ships this file" the moment `765e091` was committed. The Windows suite
is the one of the three runs whose validity is tied to a specific tree by a sha
carried inside its own tarball, so it is the one that had to run again rather than
merely be relabelled — see "The Windows suite: it ran twice" below, which is also
where this file's own now-false claim that only one Windows run would be needed is
corrected. The dogfood and the mutation run are not retracted: neither of the two
commits touches anything either of them observed, which is checked rather than
assumed at each of those sections below.

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
  what `PROVENANCE.md` records. **The `90` is the count of ids that are `opaque`,
  measured on the live a14 capture** — not the count of *sourceless* ids, and not a
  figure from the fixture. Both distinctions matter, because
  `src/backend/mod.rs:348` cites a different number for what looks like the same
  thing; the two are reconciled below under Corrections.
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
  (`winget.rs:1601-1608`). The six exit-code constants are pinned across **three**
  such tests, not one — two in `winget.rs:1583-1598`, this one, and three in
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
- **The `file:line` citation convention failed the same way three times on this
  one branch, and every time the citation was correct when written.** *Structural*,
  not a carelessness finding: a citation of this shape is a claim about the
  Nth line of a file, and any later commit that inserts a line above the target
  changes what that claim points at without touching the sentence making it —
  nothing in the build can notice, because the sentence is still grammatical
  and the line number is still a number.
  - **Instance 1** — *measured* (`git log -S`, `git diff --numstat`). Task 5's
    `5c4894c` added `Plan::unmanaged_count`, shifting `src/plan.rs` by +17 lines
    and `tests/cli.rs` by +20. That falsified five citations that were correct
    when written: `backend::mod.rs`'s `running_set` and `apply_guard_overrides`,
    each citing `plan.rs:414`/`:462` and `plan.rs:345` (`running_set` written by
    Task 2's `db1c50f`; `apply_guard_overrides` by Task 4's `6cbdfa6`), and
    `apply::sample_fence`'s citation of `tests/cli.rs:980` (also Task 2's
    `01df082`). Caught by the whole-branch review's fix wave (`2a35df2`); see
    "Sixteen `file:line` citation numbers corrected inside the shipped `.rs`
    files" above for the full table.
  - **Instance 2** — *measured* (content read against `6b2211e`). Task 9b's
    `ee46172` (the `package_roots` split) added +90/-2 lines to
    `src/backend/winget.rs` — **two commits after** that same whole-branch
    review's own re-sweep had just certified all 26 live citation numbers as
    resolving correctly. It falsified two of those 26:
    `src/backend/winget_exec.rs:154`'s citation of `resolve_latest`'s and
    `resolve_installed`'s "no `--exact`" reasoning moved from `:867`/`:956` to
    `:899`/`:988`. Task 9c found and recorded the drift (see "And then the class
    recurred" above) but left it unfixed by design, since that task's scope was
    documentation only; corrected in the `.rs` file by Task 9d, which also
    re-swept all 21 `file:line` citing sites in `src/` against this tree by
    content and found no third instance **in `src/`**. That qualifier is the
    whole of Instance 3, and this bullet shipped without it: Task 9c's survey and
    Task 9d's re-sweep were both scoped to `src/`, and neither said so, so
    "no third instance" read as a statement about the tree.
  - **Instance 3** — *measured* (target content read at `01bdd16`, `1d633c6`,
    `c8c7f0d`, `ee46172` and this tree). **The same commit, `ee46172`, and the
    same +32, falsified a citation in `tests/` as well**, and both sweeps were
    blind to it by scope rather than by luck. `tests/winget_resolve.rs:232` cited
    `src/backend/winget.rs:870` for `resolve_installed`'s `versions_out.code == 0`
    guard — correct at `01bdd16` where it was written and still correct at
    `1d633c6`, then moved by this phase to `:1054` (+184, by `c8c7f0d`) and to
    `:1086` (+32, by `ee46172`). Found only by this phase's post-merge audit and
    fixed there; see "The same class in `tests/`" below, which also carries the
    seven inherited `tests/` citations found alongside it.
  - **What a reader should conclude.** A citation sweep is evidence about the
    tree it ran on, about the directories it covered, and about nothing else —
    Instance 2 is the proof of the first half, since it falsified two citations a
    sweep had certified two commits earlier, and Instance 3 is the proof of the
    second, since it was the same insertion one directory over. Given the
    convention as it stands, there are exactly two ways to keep a `file:line`
    citation from going stale unseen: run the sweep as the last edit before the
    file ships, or do not put a line number in the citation at all. **A third
    rule falls out of Instance 3 and is cheaper than either: a sweep must state
    its scope**, because one that does not is read as covering everything.
    This branch did none of the three consistently and paid for it three times —
    twice on citations *into* `src/backend/winget.rs`, from two different
    directories.
  - **A second failure mode, distinct from drift and invisible to every sweep
    that looks for it: a citation can be wrong the moment it is written, by
    anchoring one line off the thing it names.** *Structural.* Drift is a
    citation that was right and stopped being right; this one is never right, and
    it survives indefinitely because the cited line **exists** and reads
    plausibly — a sweep that asks "does this line resolve" gets yes, and only a
    reader asking "does it hold what the sentence claims" gets no. Both of the
    post-merge audit's own proposed targets failed this way, in the same
    direction: each pointed at the *explanation* of the thing rather than at the
    thing.
    - `tests/cli.rs:1109`'s target was proposed as `src/render.rs:493`, the start
      of the comment block that explains the column width. The citation is about
      the width itself, and at its origin `6683dd3` it pointed at
      `format!("  {marker:<8}{backend:<6} {name:<14} {rest}\n")`, which is
      **`:505`** here. `:493` resolves, reads like the right neighbourhood, and is
      twelve lines above the claim.
    - `tests/execute.rs:1534`'s target was proposed as `execute.rs:221`,
      `order`'s `sort_by_key` line. The citation is about the *group assignment*
      for `WingetStep::Set`, and at its origin `4ebd831` it pointed at the group-0
      match arm, which is **`:223`** here.
    - **The anchor that resolves it, and it is not judgement:** re-point to
      whatever line the citation pointed at **in the commit that wrote it**. That
      is the only definition of "the target" that does not require re-deciding
      the author's intent years later, and `git show <origin>:<file>` settles it
      in one command. Both corrections above were made that way.

- **A self-referential version claim is a `file:line` citation one level up.**
  *Structural*, and the same mechanism exactly: a sentence that names which commit
  contains **this document** is a claim about the document's own future, made
  inside the document, and the next commit to the document falsifies it while
  leaving the sentence grammatical. Nothing in the build can notice, for the
  identical reason. **This file wrote that claim three times and was wrong three
  times**, and a reviewer caught it every time — no sweep, no gate and no test
  ever will:
  - `137fc35` (Task 8) said the suite total was "measured on the tree this file
    describes" while `05023fd` and `c8c7f0d` had already landed. Caught by the
    whole-branch review (Important 2).
  - `6b2211e` (Task 9c) said the three runs happened "all on `4bbe3be`, the tree
    that ships this file", and labelled a table row the same way. **`6b2211e`'s
    parent is `4bbe3be`**, so both sentences were false at the instant they were
    committed, before any later commit existed. Task 9e (`3dda7e9`) hedged the
    table row and retracted the phrase in a correction paragraph but left the
    summary sentence standing; the post-merge audit caught that residue
    (Important 1).
  - `3dda7e9` (Task 9e) then said the suite was "measured on `765e091`, the tree
    that ships this file". Two commits later that was stale again, which is where
    the pattern stopped being a recurrence and became a property of the form.
  - **Not fixed a fourth time — removed.** Correcting the sha buys exactly one
    commit, and three corrections is enough evidence that the next one would too.
    What a reader actually needs is the per-run attribution this file already
    carries, and it now carries it alone: see the standing claim opening
    Verification. The general lesson, worth more than the three instances: **a
    document must not assert its own version.** It can name the sha of every
    measurement inside it, which is a claim about the past and stays true, and it
    can state a rule about what later commits are allowed to be — but the moment
    it says "this is the tree you are reading", it has written a `file:line`
    citation whose file is itself.

### Five in Task 9's own verification work, all the controller's

The runs in the Verification section below are the phase's most expensive
evidence, and the process that produced them failed five times first. Recorded on
the same footing as the measurement round's four, because a verification step that
reports a wrong answer confidently is worse than one that was never run.

1. **The dogfood's first `pkg.lock` was malformed, so stages A1 and A2 observed
   nothing and had to be redone.** The shape was **guessed instead of read**. A
   winget lock entry needs `[winget."<id>"]` carrying **both** a `version` **and**
   the literal sentinel `pin = "version-only"` — `src/lock.rs:84` rejects anything
   else with `only "version-only" is defined`. Caught only because the run printed
   a TOML parse error pointing at line 2 instead of a plan; a guess that had parsed
   into something *plausible* would have produced a stage that looked like it
   observed the fence and did not. Re-run corrected.
2. **The Windows suite script shipped with a backtick in a comment** — the one
   character the phase's own rules forbid absolutely, in exactly the place those
   rules warn about (comments included). Fixed before the script ever ran.
3. **And the check that was supposed to catch it lied.** The gate ran `grep -c`,
   got **1**, and then printed `(0 = no backticks, good)` **unconditionally** beside
   it. So the output read as passing while displaying the failure on the same line.
   **This is the worst of the five**, and it is this project's named recurring
   defect wearing a new hat: not a test that cannot fail, but a *report* that cannot
   say "failed". The remedy was to make the gate branch on the count rather than
   narrate it.
4. **The dogfood script first shipped with nine backticks** — the same forbidden
   character, nine times, because it is PowerShell's escape for a quote inside a
   string and reaching for it is the obvious move. The upload gate refused the
   script; rewritten using `[char]34` instead.
5. **The rewrite then introduced a quieter defect than the one it fixed.**
   PowerShell parses `'a' + $nl + 'b'` **after a command name** as three separate
   arguments, not as one concatenated expression, so **six file writes would have
   written garbage** — and written it successfully, with no error. Fixed by
   parenthesising each expression. **The standing rule adopted from it:** every
   script is parse-checked on the target machine with `[Parser]::ParseFile` before
   it is run, which is the only one of these five failures that a rule can actually
   prevent recurring.

The pattern across all five is worth naming: **four of them were caught by a gate
and one by a gate that had to be fixed first**, and none by re-reading the script.
Gates on the artefact caught what review of the artefact did not.

## Corrections to earlier documents

Recorded here rather than edited in place, matching the precedent set by the 2a,
2b-2, Phase 3, Phase 4 and Phase 4b designs. **Three** exceptions, all because
leaving them would ship a falsehood rather than a superseded sentence: this phase's
own measurement document is corrected **in place** in two sections and says so at
each (see the end of this section); the two `.rs` comments this phase falsified
were **fixed in the code**, since a reader of the tree cannot be warned off a
false comment by a document; and `README.md`'s `dotpkg apply` transcript was
corrected **in place** by the whole-branch fix wave, for the same reason one
directory over.

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

### And one `README.md` transcript, the third in-place exception

*Structural, provable by reading.* Recorded because it was missing entirely: the
word "README" appeared exactly **once** in this whole file, on an unrelated point,
so a reader counting the exceptions named at the top of this section found two
where the branch made three.

The `dotpkg apply` block read as **verbatim terminal output** — a `$ dotpkg apply`
shell prompt, two plan lines, a blank line, then the confirmation question — and
the binary **cannot produce it**. Two things always land between the plan and the
question: `render(plan)` emits an `N change(s), M skipped` summary line for every
non-empty plan (this branch is what made that clause mandatory), and `main` calls
`render::render_preparation` on the full `apply` path and not only under
`--prepare`, so a real run prints an entire second table there too. The same branch
then contradicted the block inside its own file, by adding a `--show-unmanaged`
bullet that says the flag reaches *both* tables `apply` prints.

Found by the whole-branch review (Minor 4) and fixed **in place**, in `2a35df2`,
rather than recorded here — a fabricated transcript in the project's front door is
the stale sentence a reader is least likely to check and least able to be warned
off by a phase-notes document, which is the identical argument the two `.rs`
comments above got, one directory over. The block is now labelled **"That block is
abridged, not verbatim"** and enumerates what it omits (`README.md:78-91`). **No
line numbers were added to it** — it names `src/render.rs` and `src/main.rs` as
files, deliberately, for the reason the next section is entirely about.

### Sixteen `file:line` citation numbers corrected inside the shipped `.rs` files

The same defect class as the two comments above, but caused by *line drift*
rather than by a wrong sentence.

**The unit is one `file:line` number, and it is named here because this heading
used to read "Thirteen" and thirteen counted nothing.** It was **9 rows of the
first table below plus 4 numbers of the second** — two units added together, a
total matching neither. Caught by the independent citation sweep that followed the
fix wave (ledger, `progress.md:132`), and it survived into this document anyway.
Recounted against the two tables, which are themselves unchanged:

- **16 citation numbers** — a number being one `:NNN` or one `:NNN-MMM` range;
- sitting on **13 lines** of `.rs` comment, because one line can carry two numbers
  (`backend/mod.rs:274` cites `plan.rs:431` *and* `:479`, and
  `winget_exec.rs:154` cites two `winget.rs` lines);
- presented as **12 rows** — 9 in the first table, 3 in the second. A row groups
  what one citing comment says about one target; in the first table each row
  happens to be one line as well, and in the second table's first row two lines of
  one test's comment are grouped, which is why 3 rows hold 4 lines.

**Every citing line counted here is in `src/`**, which the heading does not say and
which is not incidental — see "The same class in `tests/`" below for the eight
numbers that scope hid.

**Every count in this section is in numbers.** **Twelve** of the sixteen were
correct when written and were falsified by a later commit on this branch: **seven**
found by the whole-branch review (Important 1, across five citing comments),
**five** more by the fix wave's own re-sweep of every `<file>.rs:<line>` citation in
`src/`, comparing each target's line *content* at `1d633c6` against this tree.
**Four** were already wrong at `1d633c6` and are inherited rather than this
branch's; they are listed separately below. All sixteen are corrected in the
shipped tree; the tables are here because the *cause* is worth naming, not the
numbers.

Task 5's `5c4894c` added `Plan::unmanaged_count` and shifted `src/plan.rs` by
exactly **17** lines and `tests/cli.rs` by **20**; Task 2 removed
`Scoop::running_set` and shifted `src/backend/scoop.rs` by **-16**.

**`src/backend/winget.rs`'s own growth needs a tree named against it, because this
file used to give two different figures for it in two places and label neither.**
One figure was **535** and the other **611**, both written by the same commit, and
a reader got two numbers for one quantity with nothing to choose between them. Both
were correct measurements of trees that are not the one shipping. **Re-derived by
Task 9c with `git diff --numstat 1d633c6 -- src/backend/winget.rs` at each point:**

| tree | added | deleted | what it is |
|---|---|---|---|
| `c8c7f0d` | 535 | 15 | before the fix wave |
| `4673517` | 611 | 15 | after the fix wave; the tree the citations below were re-pointed against |
| **`4bbe3be`** | **699** | **15** | **the tree the figure was measured against when Task 9c wrote this row** |

**The single figure for this phase is therefore 699 added against 15 deleted, and
it still is**: `git diff --numstat 4bbe3be 765e091 -- src/backend/winget.rs` is
empty, so neither `6b2211e` nor `765e091` moves this number. `4bbe3be` is kept as
the row's label rather than replaced with `765e091` because the row is a
historical measurement, not a running total. **No row and no sentence in this file
claims to name the tree that ships it** — see the standing claim opening
Verification below for what replaced that, and why replacing it was cheaper than
keeping it correct.

The other two are kept only because the citation table below needs them: the
**+171/+184** shift the table's two `winget.rs` rows record was caused by the 535,
and re-pointing them was done against `4673517`. The 611 → 699 step is `ee46172`'s
`package_roots` split (`+90/-2` in that one file), and it moved those two targets
**again** — see the subsection immediately after the table.

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

**Two of the twelve were already imprecise before this branch** — `render.rs`'s
`:303` was two lines off its target function and `:286` two lines off its
expression at `1d633c6` — and this branch's own edits then moved both into
*different functions*, which is what turned an off-by-two into a false pointer.
Corrected on the same footing as the other ten.

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

**Not claimed: that sixteen is all of them — and there are two boundaries, not
one.** The first is what a sweep can see: it catches *drift* — a citation whose
target line changed content between `1d633c6` and this tree — and cannot catch one
that was wrong from the start and whose target never moved. The four above were
found only because a human read the comments around them, after the sweep had
flagged their *neighbours*. A mechanical guarantee would need the citations to be
checkable, which line numbers in prose are not.

**The second boundary is `src/`, and no sweep on this branch ever said so.** The
fix wave's sweep, the independent 26-number re-sweep after it and Task 9d's
21-site re-sweep all took `src/` as their scope, because that is where this
phase's own comments were. `tests/` uses the identical `file.rs:NNN` convention,
carries citations *into* `src/`, and was swept for the first time after the merge.
**A sweep whose scope is not stated reads as complete**, which is the whole
mechanism by which the eight numbers in "The same class in `tests/`" below stayed
invisible — including one that is this branch's own third instance of the drift
class.

### And then the class recurred, on this branch, after the sweep that fixed it

**Found by Task 9c, measured, and not fixed — the class this document has a whole
section about happened once more, inside the very commit that was closing it.**
`ee46172` (Task 9b's `package_roots` split) added **+90/-2** lines to
`src/backend/winget.rs`: about 32 of them at the function near `:250`, the rest as
four tests in the `#[cfg(test)]` module. Every citation whose target sits after
those two insertion points moved — **+32** for targets before the test module,
**+88** for targets inside it. The independent citation sweep that certified "26
live citation numbers, all 26 resolving" ran on the tree **before** `ee46172`, so
it is not evidence about any tree after it.

**One citing site in `src/` was stale on `4bbe3be`, and Task 9c did not fix it** —
this task's scope was documentation only, so the `.rs` files were left untouched
deliberately rather than by oversight. **Task 9d then fixed it, in `765e091`** —
see "The `file:line` citation convention failed the same way three times on this
one branch" above for the fix itself and the 21-site re-sweep that followed it. The
table below is kept as Task 9c wrote it, describing the citation's state on
`4bbe3be`, because the point of this subsection is the drift that tree recorded,
not the value it holds now:

**`src/` is the qualifier that matters in the sentence above, and it was not there
until after the merge.** `ee46172` falsified a citation in `tests/` by the same
+32, in the same insertion, and neither Task 9c's survey nor Task 9d's re-sweep
could see it, because both were scoped to `src/` and neither said so — see "The
same class in `tests/`" below.

| citing site | said | drifted target on `4bbe3be` |
|---|---|---|
| `src/backend/winget_exec.rs:154`, in `list_one_argv`'s doc comment | `src/backend/winget.rs:867`, `:956` | `:899`, `:988` |

Both targets are the *"no `--exact`"* comment lines in `resolve_latest` and
`resolve_installed`. They were correct at `1d633c6` as `:696`/`:772`, re-pointed to
`:867`/`:956` against `4673517`, and were `:899`/`:988` on `4bbe3be` — so the total
drift from base reached **+203/+216**, not the +171/+184 the table above records
for the earlier tree, before Task 9d's fix brought the `.rs` comment itself back
into agreement. **This is the same defect the table exists to document, in the
same file, one commit later — and, unlike the sixteen in that table, it shipped
broken for one more commit before anyone caught it.**

**Six citations in *this* document had drifted the same way, and those are
re-pointed in place** — they are documentation, and the whole premise of the table
above is that a wrong number is worth fixing:

| what it cites | said (correct at `4673517`) | now (`4bbe3be`) | shift |
|---|---|---|---|
| the two exit-code pin tests | `winget.rs:1495-1510` | `:1583-1598` | +88 |
| `the_internal_error_codes_decimal_and_hex_forms_still_agree` | `winget.rs:1513-1520` | `:1601-1608` | +88 |
| the reader's generic `INTERNAL_ERROR` arm | `winget.rs:1077-1092` | `:1109-1124` | +32 |
| shorthand citation 1 | `:907` | `:939` | +32 |
| shorthand citation 2 | `:1338` | `:1370` | +32 |
| shorthand citation 3 | `:1588` | `:1676` | +88 |

**What this actually demonstrates, and it is not that people are careless.** The
fix wave re-pointed sixteen citation numbers and a reviewer verified all
twenty-six live ones in `src/`; **two commits later eight numbers on seven lines
were wrong again** — the six in the table above, one number each, plus
`winget_exec.rs:154`'s two on one line — because a later commit inserted lines
above them. (Counted in numbers, for the reason the heading above now gives; the
earlier draft of this sentence said "seven of them", which was six documentation
rows plus one `.rs` citing line, the same mixed unit one more time.)

The failure is not attention, it is the format: `file:line` in prose
is invalidated by any edit above the target and nothing in the build can notice.
`src/backend/winget_exec.rs:154`'s citation has now been wrong, fixed, wrong
again, and fixed again within one branch — the second fix, Task 9d's in
`765e091`, corrected the numbers but did not switch the citation to a symbol
name, so it is exactly as fragile against the next insertion as it was after the
first fix. The durable answer is to cite by **symbol name** — which no line edit
can break, and which `winget.rs`'s own `INTERNAL_ERROR` doc comment already does
— and that is a production change no documentation task in this phase made.
Recorded here so the next phase inherits the diagnosis rather than the fifteenth
instance.

### The same class in `tests/`, swept only after the merge

**Found by this phase's post-merge audit and fixed in the same commit that records
it.** *Measured*: each target's content read at the commit that wrote the citation,
at `1d633c6`, and on this tree. Every citation sweep on this branch was scoped to
`src/`; `tests/` uses the identical convention and cites *into* `src/`, so the
convention's failure mode applies there unchanged. Swept for the first time after
the merge: **9 citation numbers on 8 lines in `tests/`, of which 8 were wrong.**

**One of the eight is this branch's own — the third instance of the drift class,
not the second.**

| citing line | said | now | correct when written? |
|---|---|---|---|
| `tests/winget_resolve.rs:232`, in `a_failed_depth_lookup_is_not_trusted…` | `src/backend/winget.rs:870` | `:1086` | **yes** |

The target is `resolve_installed`'s `versions_out.code == 0` guard. It was correct
at the commit that wrote it (`01bdd16`, Phase 4b) and still correct at `1d633c6`,
and **this phase moved it twice**: to `:1054` by `c8c7f0d` (+184, the tasks' own
additions to that file) and to `:1086` by `ee46172`'s `package_roots` split (+32) —
**the same +32, from the same insertion, that falsified `winget_exec.rs:154`'s two
numbers one file over.** That one was caught by Task 9c and fixed by Task 9d; this
one was not, and the only difference between them is the directory. So the durable
count for this branch is **three** instances of "a citation correct when written,
falsified by a later commit here", not the two the bullet above recorded: Task 5's
`5c4894c`, and `ee46172` twice over — once inside the sweeps' scope and once
outside it.

**Seven more, all inherited, all fixed in the same pass.** None is this branch's
doing — every one was already wrong at `1d633c6` — and they are corrected anyway,
which is the reasoning this document already gave for the four inherited `apply.rs`
citations above, applied across the directory boundary it stopped at. Leaving
known-false pointers in a tree whose review rounds were about false pointers is the
wrong call in `tests/` for exactly the reason it was the wrong call in `src/`; the
only thing the boundary changed was who looked.

| citing line | said | now | at `1d633c6` the target was |
|---|---|---|---|
| `tests/cli.rs:726`, `a_ready_prune_with_nothing_held_back…` | `main.rs:602` | `:670` | `:642` |
| `tests/cli.rs:1109`, the preparation-table column pin | `src/render.rs:229` | `:505` | `:350` |
| `tests/cli.rs:1698`, the Task 14 section header | `main.rs:438` | `:882` | `:840` |
| `tests/cli.rs:1698`, same line | `main.rs:459` | `:943` | `:901` |
| `tests/cli.rs:1699`, same comment | `main.rs:470` | `:955` | `:913` |
| `tests/cli.rs:1700`, same comment | `main.rs:496` | `:1004` | `:962` |
| `tests/execute.rs:1534`, `a_winget_set_sorts_before_every_removal…` | `execute.rs:190` | `:223` | `:223` |

Three details worth keeping rather than flattening:

- **The four `main.rs` numbers were correct at *their* origin** (`58c8e29`, Phase
  3, where they named the five `cargo mutants` survivors that section exists to
  close) and had already drifted by ~400 lines before this branch started. The
  comment now says which tree its four numbers describe, and keeps the Phase 3
  figures beside them so a reader can still find the survivor report
  (`docs/phase3-notes.md:320-322`, which records the same four).
  **Those four old numbers are labelled `HISTORICAL, DO NOT RE-POINT` in the
  comment itself**, not merely mentioned here, because the next sweep will see them
  and "correcting" them to this tree would turn a true statement about Phase 3 into
  a duplicate of the line above it. That is the trap Task 1's `package_roots`
  comment fell into twice — a comment that quotes numbers a sweep will then
  match — and the label is the cheapest thing that closes it, since the
  alternative is a sweep smart enough to read tense.
- **`tests/execute.rs:1534`'s target never moved on this branch at all** — it is
  `:223` at `1d633c6` and `:223` now. The citation says `:190`, correct at
  `4ebd831` where it was written. This is the case the drift sweep is structurally
  blind to and the "Not claimed" paragraph above names: wrong from the start of
  this branch, target stationary, so no content diff between two trees can surface
  it. Found by reading.
- **The one `tests/` citation that was already correct is the one naming a whole
  function** — `tests/execute.rs:1675` cites `tests/cli.rs:55-64` for
  `path_without_winget`, and `src/apply.rs:1090` cites the same range; both
  resolve. Stated as the single observation it is, and **not** as "ranges survive
  and lines rot": `src/`'s re-pointed set includes two ranges
  (`plan.rs:345` → `:362-369`, `scoop.rs:299-303` → `:283-287`), so a range is no
  more durable than a line. What made this one hold is that
  `path_without_winget` is short and nothing was inserted above it — luck, on the
  evidence available.

**The sweep, stated with its scope so this one does not read as complete either.**
After the fixes: **26 citation numbers on 21 lines in `src/`, all 26 resolving**,
and **9 numbers on 8 lines in `tests/`, all 9 resolving** — measured on the tree of
the commit that records this, by reading each target's current content. Both halves
are evidence about that tree and about `src/` and `tests/` only; nothing later, and
nothing under `docs/`, which carries its own line-number citations and has never
been swept.

**And a cost this section creates rather than removes, said out loud because the
alternative is discovering it later.** The two tables above are themselves new
`file:line` citations — into `tests/cli.rs:726`, `:1109`, `:1698`–`:1700`,
`tests/execute.rs:1534` and `tests/winget_resolve.rs:232` — so the record of the
drift class has just added surface for the drift class. That is the same trade this
document already accepted for its `src/` tables, and it is no better here: any
insertion above those lines falsifies the left-hand column and nothing in the build
can notice. The durable answer named at the end of the previous section — cite by
**symbol name** — would fix both, and it is **still not implemented**, because it
is a production change no task in this phase was given and inventing one here would
be precisely the unscoped widening this phase kept declining. Recorded as a known,
priced cost, not as an oversight.

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

### `src/backend/mod.rs`'s 84 and this file's 90 are both right — reconciled, not corrected

`src/backend/mod.rs:348` says *"84 of 126 ids on a14 were sourceless"*. This file
records **90 opaque** for the same machine and the same 126. Nothing reconciled
them, so a reader met two numbers for what reads like one quantity — the same shape
as the 535/611 defect above. Parked during the whole-branch review and closed here.

**They are different predicates measured on different captures.** *Sourceless* is
one of three ways a row fails to reach `installed`; *opaque* is the union of all
three. Computed from both captures:

| capture | sourceless | `"> "`-prefixed | disagreeing-version | = opaque |
|---|---|---|---|---|
| checked-in fixture (`tests/fixtures/winget/list-full.txt`) | **84** | 2 | 3 | **89** |
| live a14 capture | **85** | 2 | 3 | **90** |

So `mod.rs`' **84** is the **fixture-era sourceless** count, inherited from Phase
4b, and this file's **90** is **Phase 5's live opaque** count. Neither is wrong for
what it cites, and the fixture's own opaque total — **89** — is exactly the `89`
in the `141 / 126 / 37 / 89` replication above, which is the independent check that
the reconciliation is arithmetic rather than a story.

**`src/backend/mod.rs` is deliberately not edited.** Its 84 is correct for the
predicate and capture it names, so there is nothing there to fix; the gap was that
neither document said which was which.

**Confirmed live, by the machine, in the dogfood.** The shipped binary printed
`85 installed entries have no winget Source` on a14 — the **85** this table needs
for the live row, produced by production code (`src/backend/winget.rs:537`) rather
than derived by hand. The reconciliation was worked out before the dogfood ran and
the dogfood then confirmed it independently, which is the only reason this entry is
labelled **measured** rather than *reasoned*.

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

### Five corrections made in place, across four sections of this phase's own measurement document

**This heading read "Four corrections" while the body enumerated five**, and the
count is fixed rather than the body, because five is what happened. The **4** that
was true of it is the number of *sections* touched — §1, §3, §4 and §5 — which is
not what "four corrections" says. Two of the five are the §3 and §1 entries below;
the other three are the three drifted `file:line` citations in the table at the end,
which land in §4, §5 and §5. Parked during the whole-branch review and folded in
here, since this document had to be reopened for the Windows and dogfood runs
anyway. A miscounted heading over an enumerated list is the smallest possible
version of this project's recurring defect, and it is still that defect.

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

**Every measurement below names the tree it ran on, and none of them claims to
name "the tree that ships this file". That omission is deliberate.** *Structural.*
A sentence inside this document about which commit contains this document is
falsified by the next commit to it, and nothing in the writing can notice —
identical in form to a `file:line` citation, one level up, and this file has made
the claim three times and been wrong three times (`137fc35`, `6b2211e`,
`3dda7e9`; see "A self-referential version claim is a `file:line` citation one
level up" under Method failures). Fixing the sha buys exactly one commit, so the
claim is gone rather than corrected again.

**How a tree gets named, since one case cannot use a sha.** Every run that
happened before the commit writing it up is named by **sha**, which is a claim
about the past and stays true. The one measurement taken *in* the commit that
records it is named as **"the commit that records this"** — a commit cannot contain
its own sha, and inventing a label for it is how the retired claim got written in
the first place. `git log --oneline -- docs/phase5-notes.md` resolves it in one
command.

**What replaces it, and it is a standing claim rather than a sha.** *Structural.*
**Every commit to this repository after a measurement recorded below is
documentation or comment text, unless this record says otherwise.** Where a commit
did touch code a measurement had observed, the section for that measurement says
so and either re-runs it or argues attributability at that spot — the mutation
run's own paragraph and the Windows suite's second run are the two worked examples,
and they are the only two. A reader who wants to check rather than trust should run
`git log --numstat <measured-sha>..HEAD -- src/ tests/` and read the diff; prose
cannot notice a commit, and a sha embedded in prose is a claim about the future.

### macOS suite

`cargo test --no-fail-fast`, **measured on `765e091`** — named as a sha rather
than as "the tree above" or "the tree that ships this file", both of which this
section has been caught by, and named as `765e091` rather than `4bbe3be` because
two more commits landed after Task 9c measured that one (below): **exit 0, 638
passed, 0 failed, 0 ignored**, across **14**
`test result:` lines (`unittests src/lib.rs` 311, `unittests src/main.rs` 14, the
eleven `tests/*.rs` binaries totalling 313, and `Doc-tests dotpkg` 0). Base `main`
at `1d633c6` was 588, so the phase adds **50** tests. **Measured directly by Task
9e on `765e091`** — not inherited from Task 9c's `4bbe3be` figure, because
`765e091` also touches a `.rs` file (one comment line in
`src/backend/winget_exec.rs`), and this section's own history two entries below is
already two-for-two on what happens when a number is carried past a tree move
instead of re-run.

**The history of this one number, because it has been wrong here twice before and
the shape of the error was the same both times** — a count that was true of some
tree sitting under a sentence that named a different one:

- **631** was Task 8's total at `137fc35`. The sentence then read "measured on
  the tree this file describes" while two commits (`05023fd`, `c8c7f0d`) had
  landed without the file moving — the whole-branch review's Important 2. The 631
  was not wrong, it was unowned.
- **634** was the fix wave's, `+3`: two in `src/render.rs` for the
  `render_preparation` summary clause and one in `src/backend/winget.rs` for the
  retry's `attempt == 0` guard. It held from `2a35df2` through `4673517`.
- **638** is Task 9b's, `+4`: the four `package_roots_with` tests `ee46172`
  added, all four in `src/lib.rs`'s unit tests, which is exactly where the 307 →
  311 move comes from.
- **638 held unchanged through `4bbe3be`, `6b2211e` and `765e091`, and this is the
  first of those three it was actually re-run on rather than carried forward.**
  `4bbe3be` was a docs-only commit on top of `ee46172`, so inheriting 638 there was
  correct. `6b2211e` is docs-only too. `765e091` is not: it changes one comment
  line in `src/backend/winget_exec.rs`, which makes inheriting 638 across it
  without re-running exactly the mistake the 631 entry above already names once.
  Re-run instead, by Task 9e: same **638 / 0 / 0**, same **14** lines. A comment
  cannot change a test count, but this section does not get to assume that — it
  gets to measure it, which is what makes **638 a figure `765e091` actually
  earns**, rather than one it merely inherited.
- **638 again after the post-merge audit's remediation, which changed comment text
  in `src/backend/winget.rs` and three `tests/*.rs` files and nothing else** — the
  same 638 / 0 / 0 across the same 14 lines, re-measured rather than inherited,
  on the same principle the entry above states. Named as *the commit that says
  this* rather than as a sha, because a commit cannot contain its own sha; a
  reader locates it with `git log --oneline -- docs/phase5-notes.md`.
- **There is no fourth sha entry, by design.** The standing claim at the top of
  this section is what covers everything after these runs: a commit that changes
  code a measurement observed has to say so at that measurement. Three entries
  above are three re-measurements chasing a sentence that could not stay true, and
  the rule is what the list was reaching for the whole time.

Also **measured directly by Task 9e on `765e091`**, for the same reason — not
carried forward from Task 9c's `4bbe3be` run:

- `cargo fmt --check` — exit 0, no output.
- `cargo clippy --all-targets -- -D warnings` — exit 0, zero warnings.

#### The suite's one wall-clock assertion, and it was measured under load

**Recorded here because it existed only in the git-ignored ledger
(`progress.md:131`), and this is the one branch that lost a merge gate to a
timing-dependent test.** The 634 entry above names three tests without saying that
one of them asserts on a clock:
`the_retry_delay_is_not_slept_after_the_final_failed_attempt`
(`src/backend/winget.rs`), the fix wave's answer to whole-branch
Minor 6. It pins `attempt == 0` by timing rather than by injecting a sleeper, so no
production seam is added for a test's benefit, and its `DELAY` is 200 ms with a
100 ms threshold on either side of the split.

**Its author labelled that margin *reasoned*, not measured. The scoped re-review of
the fix wave then measured it**, which is why the label in the code now says both.
**Measured on the fix-wave tree** — `2a35df2`, the commit that added the test;
`4673517` on top of it is the record only — and **it carries to `765e091`** because
the test's code has not changed since: stripping comments, its 32 lines are
identical at `2a35df2` and here, so nothing about the timing it asserts on moved.

- **20 consecutive runs of the test under 20 busy loops on 10 CPUs — 0 flakes.**
- The margin is roughly **100 ms of headroom against a microsecond-scale span**:
  everything between the second `run` and the function's return is one `expect`,
  one integer comparison and one `format!`.
- **`total >= DELAY` cannot fail at all** — *structural* — because `sleep` never
  returns early. Only the `tail` assertion is timing-sensitive, so the surface
  that could flake is half of what the test's two assertions suggest.

**Not a merge-gate risk**, and the reason that verdict is worth the space: the
gate this branch actually lost was lost to a *different* timing-dependent failure
(git's background maintenance writing a lock file inside a fixture repo, below), so
"a test that asserts on wall-clock time" is not a hypothetical worry here. A margin
left *reasoned* on this branch, in the ledger only, would have been the cheapest
possible place for the next lost gate to hide.

### Windows target, cross-checked from macOS

`cargo check --target aarch64-pc-windows-msvc --all-targets` — exit 0, zero
warnings, **measured on `765e091`** (re-run by Task 9e; the `4bbe3be` result this
section previously cited was not carried forward, on the same standing as the
macOS suite above, even though the only intervening `.rs` change is a comment).
This type-checks every `#[cfg(windows)]` path from macOS and is explicitly
**not** a substitute for running the suite on Windows: it catches compile errors
on the Windows target, not behavioural differences. That distinction is no
longer hypothetical here — the suite *has* now run on Windows, below, twice, and
this check is what made both runs cheap rather than what replaced them.

### Fixture integrity

**Checked by sha, not only by the two counts**, on both platforms.
`tests/fixtures/winget/list-full.txt` is **30958 bytes, 143 CRLF pairs, sha256
`c71284a393f87686…`** — measured on `4bbe3be` on macOS, and measured again
**inside the tarball before upload** and on a14 after unpacking, all three values
identical. The byte count and the CRLF count are exactly what `PROVENANCE.md` and
`docs/phase4b-notes.md` record, and `.gitattributes` pins the path `-text`.

**Checked a second time for the Windows suite's second run, on `765e091`** —
same three numbers, same sha, again inside the tarball before upload and on a14
after unpacking. Re-confirmed on macOS too, directly against the checked-in file
by Task 9e: **30958 bytes, 143 CRLF pairs, sha256 `c71284a393f87686…`**,
unchanged. Expected, since `git diff --name-only 4bbe3be 765e091 --
tests/fixtures/` is empty — but this section's whole point is that expected is
not measured, so it was checked rather than assumed.

**Why the sha is the check that matters and the two counts are not enough**, which
this phase learned from its own mistake rather than from a rule: this round's
corrupted probe capture `p1-list.txt` also measured **30958 bytes with 143 CRLF
pairs** while having a different sha256 (method failure 2 above). Two files whose
size and line-ending counts agree can differ in content, so a Windows run
validated on those two numbers alone would have been validated by a check its own
notes already record as insufficient.

**No fixture bytes changed in this phase**: the branch diff touches no file under
`tests/fixtures/`. Checked before any Windows result is trusted, because a fixture
normalised by a checkout makes every downstream assertion meaningless.

### The mutation run: it refused to start, and then it completed

Two runs, and both belong here. The first aborted before testing a single mutant;
the second, on the settled tree, completed. The abort is recorded first because
it is the finding — a gate that fails is worth more of this section than a gate
that passes.

#### The first attempt refused to start

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

#### And then it completed — twice, either side of the `package_roots` split

**Measured**, `cargo mutants --in-diff -j 2 --timeout 600`, on an idle machine
with nothing editing the tree:

| run | tree | mutants | caught | missed | unviable | TIMEOUT |
|---|---|---|---|---|---|---|
| first authoritative | `4673517` | **70** | 57 | 2 | 11 | **0** |
| after the split | `ee46172` | **72** | 59 | 2 | 11 | **0** |

Both in **3 minutes** at `-j 2`. The second run's artifacts are the complete ones
still on disk and are what the counts above were re-derived from for this section:
`mutants.out/outcomes.json` carries **73** outcomes — 59 `CaughtMutant`, 11
`Unviable`, 2 `MissedMutant`, and the baseline's own `Success` — with both
`start_time` and `end_time` present (`12:44:55Z` → `12:48:05Z`), `missed.txt`
holding exactly two lines, `caught.txt` 59, `unviable.txt` 11, and `timeout.txt`
**empty**. Only one `mutants.out` exists on disk and it is the second run's, so the
first run's numbers come from the ledger and are **not** independently
re-derivable from artifacts today. Said rather than glossed: the 70/57 row is a
ledger record, the 72/59 row is a re-derivation.

**The baseline passing at all is the direct proof the flake fix above worked** —
the same step that had aborted now produced a result.

**Attributable to the shipping tree**, which is worth stating rather than
assuming, and restated here because the shipping tree has moved twice since this
paragraph was first written: `4bbe3be` is a docs-only commit on top of `ee46172`
(`1 file changed`, `docs/phase5-notes.md`). Two more commits land after it —
`6b2211e` (also docs-only) and `765e091`, which changes exactly one line of
`src/backend/winget.rs`'s companion file, `src/backend/winget_exec.rs`, and
nothing else under `src/`. That one line is a doc comment, not code `cargo
mutants` can mutate, so it changes nothing this run could have tested
differently — verified here rather than assumed: `git diff ee46172 765e091 --
src/` touches exactly `src/backend/winget_exec.rs`, one insertion and one
deletion, both inside a `///` comment. **`765e091`'s code is functionally
identical to `ee46172`'s for mutation-testing purposes**, so this run's numbers are
still the ones that describe the code as it merged. This is one of the two worked
examples the standing claim opening this section points at: the argument is made
here, at the measurement, rather than by relabelling the run onto a later sha.

**Zero timeouts at `-j 2`, matching Phase 4b** — the second consecutive phase to
measure that, which is what turns Phase 4's 69 unresolved `timeout` mutants
(still-open item 14) further into a settled question rather than a live one.

**On disk, and labelled honestly because the record does not support the stronger
claim.** The only disk figure on the record for any of these runs is **14 GiB free
throughout the aborted attempt**, recorded above. **No disk measurement was
captured during either completed run.** So "nothing resembling Phase 4's disk
starvation occurred" is **reasoned**, not measured, for the two completed runs:
what supports it is that both finished in 3 minutes with 0 `TIMEOUT` and a
complete `outcomes.json`, on the same machine that had 14 GiB free earlier in the
same session — not a reading taken while they ran. Phase 4's starvation showed up as
timeouts and truncated runs, and neither appeared; that is evidence, but it is
inference from the run's own shape, not a disk gauge.

**What the run actually bought, which is not the count.** Both survivors are
`package_roots`, and they are **exactly the gap a reviewer had already found by
reading, at Task 1**, and that this file recorded as a deferred minor before any
mutant was built: *"`package_roots()` has no direct test anywhere. A swap of its
two environment-variable names, or an extra `Microsoft` segment on the
machine-scope branch, would pass every test in the file."* The machine confirmed,
independently and mechanically, a defect a person had established by argument.
That is the interesting result — a mutation run agreeing with a careful reader is
evidence about *both*, and it is the one thing a count of 57 or 59 caught cannot
tell you. The two survivors' current status, their move from `:241` to `:251`
across the split, and why neither is closed, are still-open item 19.

### The file-scoped mutation run: 618 mutants, and it corrected four inherited numbers

**Measured**, after the merge, on `8ed3de0`, and recorded here because it settles
questions this file had left open by name.

Both earlier runs were scoped **by diff**, so they could only mutate lines this
phase changed. This one was scoped **by file** — `-f` for each of the eleven
`src/*.rs` files the phase touched — which is a superset covering Phase 4 and 4b
code the phase depends on but never edited:

```
618 mutants tested in 39m: 27 missed, 516 caught, 75 unviable
```

**0 `TIMEOUT`, at `-j 4`.** That parallelism is the point, and it was not
available before. Phase 4 lost a whole run to disk starvation at `-j 4` and
identified the fix as moving `TMPDIR` to the machine's 1.8 TB volume; Phase 4b
recorded that the fix *"is NOT available here: `/Volumes/ssd` is TCC-blocked from
this sandbox. Lowering parallelism is the substitute."* The volume became
reachable, so this run used it: `TMPDIR=/Volumes/ssd/...`, temp trees peaking
around 4.3 GiB there while the internal disk never fell below 18 GiB. The
substitute was no longer needed, and Phase 4's own diagnosis is now confirmed by
demonstration rather than carried as inference.

**What it corrected.** Four inherited numbers, all in the still-open list above,
none of them a defect in code and none of them closable here:

| what the record said | measured | where |
|---|---|---|
| `sys.rs`: **3** `#[cfg(windows)]` mutants | **4** | `sys.rs:139` ×3, `:163` ×1 |
| `floor_char_boundary`: **6** | **7** | `winget.rs:43`, `:46`, `:47` |
| `parse_list`: **6** | **7** | `winget.rs:78`, `:92`, `:108`, `:114`, `:150` |
| inherited `winget.rs` total: **14** | **16** | 7 + 7 + 1 + 1 |

And it found two survivors **no previous phase recorded at all** — `main.rs:627`
and `scoop.rs:222` — both in untouched code, both invisible to any diff-scoped
run by construction.

**Why the corrections were possible and had not been made.** Every one of these
numbers was inherited and quoted forward without being re-derived, and the reason
is mechanical rather than careless: no completed run had ever covered the files
they describe. Phase 4b's own record says so for `sys.rs` — *"NOT COVERED BY
THIS RUN AT ALL: `src/sys.rs` (I did not pass `-f src/sys.rs`)"* — and its
`winget.rs` figures came from a diff-scoped pass. **A number nobody can
re-derive is a number nobody can check**, which is the same lesson this file
records twice elsewhere, once for `file:line` citations and once for a document
asserting its own version.

**What it did not do.** It closed nothing. All 27 survivors are either inherited,
accepted as equivalent, or blocked on the Windows mutation run that has still
never happened. `package_roots()`'s two are unchanged (item 19), and the four
`sys.rs` mutants remain a platform gap — now measured to be one rather than
reasoned to be one.

### The Windows suite: it ran twice, because the tree moved after the first run

**Measured** on a14 (`zenbook-a14`). Two runs belong here, on the same footing as
the mutation run's abort belonging above its completion earlier in this section:
what the second run establishes is the finding, not a rerun taken for luck. This
section used to close by saying the first run was the only one that would be
needed. That sentence was true of Task 9's own sequencing, below, and false about
the branch: two more commits landed afterward, the second one touching a `.rs`
file, so the tree these notes described moved out from under them and a second
run followed, on `765e091` — the tree that actually merges.

#### First run, on `4bbe3be`

**The sha was carried inside the artefact, not asserted alongside it.** The
shipping sha travelled **in** the tarball as `SHIPPING-SHA.txt` and was echoed
back by the runner on the machine, so this run cannot be attributed to a tree it
did not test. That is a deliberate answer to Phase 4b's problem, where a run and a
tree had to be matched up afterwards by hand. The tarball was also verified to
contain **no `target/` and no `.git/`**, and the fixture's sha was checked inside
the tarball before upload (see Fixture integrity above, including why the sha
rather than the two counts).

- **`cargo test --no-fail-fast`: exit 0, 636 passed / 0 failed / 1 ignored**,
  across **14** `test result:` lines — the same 14 binaries as macOS.
- **The one `#[ignore]`d test passed when invoked by name**: `1 passed, 51
  filtered out` from `cargo test --test cli -- --ignored
  on_a_real_elevated_windows_session`. **It cannot pass vacuously**: its first
  statement asserts `sys::elevated() == Some(true)` and fails with instructions if
  the session is not elevated, so a run in the wrong kind of shell goes red rather
  than green (`tests/cli.rs:2600-2607`). The `1 + 51 = 52` also reconciles: macOS
  lists 51 tests in the `cli` binary, Windows 52, the difference being this
  `#[cfg(windows)]` test.
- **This measures `elevated()`'s `Some(true)` direction only**, which still-open
  item 15 already records as the measured one. Item 15's unmeasured half — an
  ordinary, non-elevated Windows session with no `runas` at all, where `elevated()`
  should answer `Some(false)` from `TokenIsElevated` alone — **is untouched by this
  run and stays open.**

**The cross-reference was name by name, never by subtracting totals**, and the
difference set came out to exactly the three predicted `cfg` exclusions:

| | count |
|---|---|
| macOS names | **638** |
| Windows names | **637** |
| common | **636** |

- **macOS only (2)**, both `#[cfg(unix)]`, both pre-existing and both named in the
  plan's Global Constraints: `a_failed_last_write_leaves_a_prefix_that_plan_does_nothing_about`
  (`tests/adopt.rs`) and `a_root_reached_through_a_symlink_still_matches_running_processes`
  (`tests/scoop_scan.rs`).
- **Windows only (1)**: `on_a_real_elevated_windows_session_the_pre_check_refuses_a_user_scope_removal`,
  the `#[cfg(windows)] #[ignore]` test above.
- **Zero discrepancies beyond those three**, and the difference set is
  **byte-identical to every Phase 4b run** — so the cross-reference is not just
  internally consistent, it matches an independent earlier baseline.

**Two method points worth recording because they are reusable, and each one is a
trap this project has already fallen into once:**

- **The name set came from `cargo test -- --list`, not from parsing run output.**
  A `#[should_panic]` test prints as `test <name> - should panic ... ok`, so a
  regex keyed on `... ok` drops it **silently** — a name-by-name cross-reference
  built on run output can therefore under-count on one platform only, which is the
  precise failure the cross-reference exists to detect.
- **The captured file was decoded by BOM, not by assumption.** `Tee-Object` writes
  UTF-16LE, and that once made this exact check read **0** names where `grep` read
  **565**. The capture came back UTF-8 and the detector confirmed it; the point is
  that the detector ran.

**The machine was verified unchanged by the run**, before and after: kanata
`kanata_windows_tty_winIOv2_arm64` **PID 13676** both times, **31** scoop apps,
`pkg.toml` sha `32A238FF…` unchanged, no `pkg.toml.bak`, `dotpkg-build` intact.

**Why this section used to say only one run would be needed, and why that turned
out wrong.** The tree did **not** move between this suite and the dogfood below —
both ran on `4bbe3be`, with the sha echoed back from inside the tarball both
times — because the whole-branch review, the fix wave, the mutation run and the
`package_roots` split were all **deliberately sequenced before** the Windows
work, precisely so that the expensive run would land on a settled tree. That
sequencing decision was Phase 4b's own recorded lesson being spent, and it is
still true as a claim about *this run's own two halves*. It was never a claim
about the rest of the branch, and the rest of the branch is what falsified it:
two more commits landed after `4bbe3be` — `6b2211e` (docs only) and `765e091`
(one comment line in `src/backend/winget_exec.rs`, see "And then the class
recurred" above, plus documentation) — so the tree this section described stopped
being the tree that would merge. A run whose validity is proven by a sha carried inside
its own tarball cannot be relabelled onto a later commit by arguing the code did
not meaningfully change; it has to run again on the sha that now matters. It did,
below.

#### Second run, on `765e091` — the tree that merges

**Measured** on a14, the same machine, the same convention. The sha was carried
inside the tarball as `SHIPPING-SHA.txt` again and echoed back by the runner,
this time matching `765e091`.

- **Fixture checked by sha again, inside the tarball**: 30958 bytes, 143 CRLF
  pairs, sha256 `c71284a393f87686…` — identical to the first run and to the
  checked-in file (re-confirmed on macOS too by Task 9e; see Fixture integrity
  above).
- **`cargo test --no-fail-fast`: exit 0, 636 passed / 0 failed / 1 ignored**,
  across **14** `test result:` lines — **identical to the first run.**
- **The `#[ignore]`d elevated-only test passed again when invoked by name.**
- **The name-by-name cross-reference was run again, not assumed from the first
  run**: macOS **638**, Windows **637**, common **636**, and the difference set
  **byte-identical** to the first run's and to every Phase 4b run — the same two
  `#[cfg(unix)]` tests and the same one `#[cfg(windows)] #[ignore]` test.
- **Machine verified unchanged again**: kanata `kanata_windows_tty_winIOv2_arm64`
  **PID 13676** before and after, **31** scoop apps, `pkg.toml` sha unchanged, no
  `pkg.toml.bak`.

**Two method points from this run that belong in the record on their own, because
both are about trusting a run's provenance rather than about its numbers:**

1. **The extract step partly failed, and the run was still valid — but only
   because that was proven, not assumed.** `Remove-Item` could not delete the
   previous build tree: macOS `tar` had written AppleDouble `._*` entries into it
   when the tarball was created, and one of them could not be removed. `New-Item`
   then reported the target directory already existed, and `tar` extracted the
   new tree **over** the old one rather than into a clean directory. **A run on a
   tree that was overwritten rather than replaced is not self-evidently a run on
   the shipping tree** — an overwrite that silently failed to touch even one file
   would leave a stale file from `4bbe3be` sitting inside what the runner reports
   as `765e091`. This was established **by content**, not assumed from the sha
   alone: five files — `src/backend/winget_exec.rs`, `src/backend/winget.rs`,
   `src/render.rs`, `src/backend/mod.rs`, `Cargo.lock` — were hashed on both sides
   (the known-good `765e091` checkout and the extracted a14 tree) and all five
   matched, on top of the echoed `SHIPPING-SHA.txt`. **The sha-in-tarball
   convention alone would not have caught a stale-file case**: `SHIPPING-SHA.txt`
   proves what sha the tarball *carried*, not what ended up on disk after an
   extraction wrote over an already-populated directory — only the five-file
   content check does that. **Structural, verified independently here too**: of
   the five, `src/backend/winget.rs`, `src/render.rs`, `src/backend/mod.rs` and
   `Cargo.lock` are byte-identical between `4bbe3be` and `765e091` (`git diff
   --numstat`, empty for all four) — only `src/backend/winget_exec.rs` differs.
   So `winget_exec.rs` was the one file in the set actually capable of telling the
   two trees apart, and it is the one Task 9d had just edited — the check worked
   because it happened to include the one file where a stale copy would have been
   distinguishable from the real thing.
2. **80 AppleDouble `._*` files are litter in the build directory on a14.**
   Inert — none is a `.rs` file and nothing compiles them — but they are what made
   the removal above fail, and they will still be there next time. A future
   Windows run should either create the tarball with `--exclude='._*'` or clear
   the target directory before extracting, rather than relying on `Remove-Item` to
   succeed against a directory macOS itself has littered.

### The dogfood: read-only except one stage

**Measured** on a14, on **`4bbe3be`**, sha echoed back from inside the tarball as
above. **Not re-run on `765e091`, and that is stated rather than assumed to be
fine**: unlike the Windows suite, the dogfood's evidence is not tied to a sha
inside its own tarball, so the question is whether anything it observed could
have changed between the two trees. Nothing did — `git diff --numstat 4bbe3be
765e091 -- src/` touches only `src/backend/winget_exec.rs`, one comment line, and
every stage below (`status`, `[winget.guard]`, the collapsed line, `dotpkg
update`) exercises code the dogfood ran through paths this diff does not touch.
**Structural**, not measured a second time.

**Why it could be read-only at all, which is the scoping insight and not a lucky
break:** everything this phase changed is observable through **`status`**, and
`status` performs no mutation. The path signal, `[winget.guard]` holding a running
package, the collapsed line and `--show-unmanaged` are all visible in a plan that
is never applied. Only the retry needed `dotpkg update`, and that writes only
inside a dogfood directory. **Nothing was installed, uninstalled or pruned, and
kanata was never touched** — this phase changed the fence and the reporting, not
the mutation path.

**Machine restored and proven afterwards:** kanata
`kanata_windows_tty_winIOv2_arm64`/**13676** unchanged, **31** scoop apps,
`pkg.toml` `32A238FF…` unchanged, no `pkg.toml.bak`, `WinGet\Links` still **5**
entries.

#### Stage B — the collapsed lines, live, and they confirm the offline computation

**Measured** — the two collapsed lines, one per backend, exactly as the design
intended and for the first time from a real machine (the column padding below is
the renderer's, reproduced here for shape rather than quoted for byte-exactness):

```
  ? scoop    24 installed outside dotpkg -- no action
  ? winget   36 installed outside dotpkg -- no action
```

- The hint printed **exactly once** in collapsed form and **zero times** under
  `--show-unmanaged`.
- `0 change(s), 0 skipped, 60 unmanaged` in **both** forms — which is the
  `render`-side half of the clause discussed under Corrections, observed on a real
  machine rather than on a fixture.
- `--show-unmanaged` printed **exactly 60** individual lines.

**The winget 36 closes a loop this file opened.** §4's `36` was computed on macOS
by running production code over a *captured* `winget list`; the live run drove the
same code through the *real* winget binary on the real machine. **Offline 36, live
36.** The one number this phase's headline behaviour change rests on is now
measured twice by two different routes.

**A second consistency check nobody asked for, and it is the better one.** The
later stages report **59** unmanaged, not 60, because one of the 36 winget
packages is by then **declared** and so leaves the unmanaged set: 36 − 1 + 24 =
**59**. So the count does not merely reproduce; it **tracks declaration**, which
is the property a reader actually cares about and which a single matching number
cannot demonstrate.

**The scoop 24 is measured, and it is measured against the dogfood's own
`pkg.toml` — not against a14's.** This file's §4 example carried an illustrative
scoop `6` from a fixture and said so, explicitly refusing to imply the measured
machine's scoop half had been counted. **A number now exists — 24 — but it is not
that number, and this file will not let it stand in for it.** The dogfood ran from
its own directory with its own `pkg.toml`; a14's real `pkg.toml` was verified
**unchanged** (`32A238FF…`) and was never the input. So the 24 counts scoop apps
undeclared *by the dogfood config*, and it is not comparable either to §4's
fixture-illustrative 6 or to the 25 scoop packages a14's real `pkg.toml` declares.
What the 24 does establish, and it is worth having: **the collapse fires for
scoop on real hardware**, with a real scoop install and a real count, so the
"this changes scoop's output too" claim above is no longer structural-only.

#### Stage A1 — the fence holds, but it does not isolate the new path signal

**Measured, and the caveat is the point of the entry.** `VKey` (**pid 9076**)
really does run from
`…\WinGet\Packages\PhatMT97.VKey_Microsoft.Winget.Source_8wekyb3d8bbwe\VKey.exe`
— a live process under a real `Packages\<id>_<sourceIdentifier>\` directory, which
is the shape `running_ids` was built for. Declared and locked at `0.0.1-dogfood`
against installed `4.2.0`, it became:

```
  ! winget PhatMT97.VKey  running -- stop it first
```

with **1 skipped**.

**But this does not prove the path signal fired**, and the notes say so plainly
rather than banking the result: `guard_names("PhatMT97.VKey", "VKey")` yields
`["vkey"]`, and the live process folds to `vkey`, so the **`names` half catches it
too**. Nothing in the observable output distinguishes which of the two signals
produced the refusal. This is exactly the caveat the measurement round already
recorded when it noted the path signal adds **zero** new catches on a14 — the one
live process under `Packages` is the one `guard_names` already caught.

**So: A1 confirms the fence. It does not confirm the path signal.** The path
signal remains **structurally verified and live-unverified**, and it is on the
still-open list as such.

#### Stage A2 — `[winget.guard]` proven live, with a counterweight

**Measured.** Three runs, one machine, one lock, **one variable changed** between
them:

| run | `[winget.guard]` entry | skipped |
|---|---|---|
| A2a | *none* | **0** |
| A2b | `["tailscaled", "tailscale-ipn"]` | **1** |
| A2c | `["no-such-process-xyz"]` | **0** |

The two outputs, verbatim:

```
  A2a and A2c:
  ! winget Tailscale.Tailscale 1.102.2 -> 0.0.1-dogfood (dotpkg will not downgrade a winget package -- run `dotpkg update`)

  A2b:
  ! winget Tailscale.Tailscale  running -- stop it first
```

A2a's line is the refused-downgrade path, so the fence demonstrably did **not**
catch the package without a guard entry.

**What the third run buys, stated because without it the second proves much
less.** A2b alone is consistent with a fence that holds *everything* — a guard
table that is read but whose contents are never actually compared, a fence stuck
on, a mis-plumbed default. A2c changes only the *contents* of the guard entry, on
the same package and the same lock, and the refusal disappears. **That is what
makes the guard entry provably the cause** rather than merely present when the
refusal happened.

This also closes, live, the half of the guard-merge path that had **unit coverage
only**: a value from `[winget.guard]` reached `Installed.bins` through
`backend::apply_guard_overrides` and changed a real plan on real hardware. The
**`opaque` warning branch** of that same function is a different branch and was
**not** exercised — `Tailscale.Tailscale` is source-backed on a14, so nothing in
the dogfood put a guard entry on a sourceless id. It stays open.

#### Stage C — inconclusive, and recorded as inconclusive rather than as a pass

**Measured, and it measures nothing about the retry.** Six `dotpkg update` rounds
with a concurrent `winget list` produced **zero** index-refresh warnings.

**That cannot distinguish the two explanations**, and both remain live:

1. the contention never reproduced under `dotpkg update` at all; or
2. it reproduced and **the retry absorbed it invisibly** — a successful retry
   produces no output, by design.

A run with zero warnings is the *expected* output of both. Distinguishing them
needs instrumentation the shipped binary does not have — a counter, a debug line,
something that says "retried once and succeeded" — and adding it is a production
change, not a dogfood step.

**So the retry ships structurally verified and live-unverified.** Its mechanism is
provable by reading (`update_source_with(Duration)`, one retry, only on
`INTERNAL_ERROR`, `Duration::ZERO` in tests) and its trigger was measured on real
hardware in the measurement round (3 of 10). What has **never** been observed is
the retry itself firing. That is on the still-open list, and it is deliberately
**not** written up as "the retry works".

#### One more live confirmation, of an item parked during the review

The live binary printed **`85 installed entries have no winget Source`**. That is
the scan's own warning loop (`src/backend/winget.rs:537`), and **85** is exactly
the figure derived by hand when reconciling the 84-versus-90 discrepancy recorded
under Corrections below. The reconciliation was arithmetic; the machine confirmed
it independently.

#### Environmental, unrelated, pre-existing — one line so a future reader is not alarmed

Three scoop manifests cannot be read on a14 — `actionlint`, `antigravity`,
`zellij` — failing with *"The path cannot be traversed because it contains an
untrusted mount point"* (**os error 448**). dotpkg warns per package and
continues, **which is the designed behaviour**. Not caused by this phase, not
caused by the dogfood, and not a dotpkg defect.

### What Windows work still cannot reach

The three items below were listed here as "reachable only from the Windows work"
before that work ran. **Two of the three are still open after it**, and saying so
is the point of keeping the list:

- `backend::winget::package_roots()` returns an empty vector on macOS, so the
  winget path signal is exercised on that platform only through
  `sample_fence_with_roots`' fabricated roots. **The Windows suite did not close
  this.** Nothing in the suite calls `package_roots()` on either platform, which
  is precisely why its two mutants survive (still-open item 19); closing it needs
  a **mutation run on Windows**, which has not happened.
- The `opaque` warning branch in `apply_guard_overrides` still has **unit coverage
  only**. `tests/cli.rs` strips winget from `PATH`, so no `cli.rs` test can produce
  a sourceless row. **The guard-merge half of the same function is now covered
  live** by Stage A2; the `opaque` arm is not, because no dogfood stage put a
  guard entry on a sourceless id. `opaque` is the majority case on real hardware —
  **90 of 126** ids in this round's live capture.
- The retry's real 1 s delay, and `INTERNAL_ERROR` arriving from a real winget
  rather than from `ScriptedWinget`. **Stage C above is inconclusive on exactly
  this**, so it stays open with a measurement attached rather than with nothing.

The standing rules that governed the runs, carried forward unchanged for the next
phase: the Windows suite runs on the tree that will merge, with the sha carried
inside the artefact; the cross-reference is **name by name**, never by subtracting
totals, and its name set comes from `--list` rather than from run output;
`cargo mutants -j 2` on an idle machine with nothing editing the tree; fixture
bytes checked **by sha** first.

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
- **The `opaque` warning branch and the guard-merge half had unit coverage
  only**, for the structural reason above (`tests/cli.rs` strips winget from
  `PATH`). **Half of this closed.** Task 9's dogfood ran, and its stage A2
  exercised the **guard-merge half live** on real hardware: a `[winget.guard]`
  value reached `Installed.bins` through `backend::apply_guard_overrides` and
  changed a real plan, with A2c as the counterweight proving the entry's *contents*
  were the cause. **The `opaque` warning branch is still not covered live** —
  `Tailscale.Tailscale` is source-backed on a14, so no dogfood stage put a guard
  entry on a sourceless id, and `mod.rs:409`'s arm has never run outside a unit
  test.
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
  `measurements-2026-08-11 §N` with no filename** — `:939`, `:1370`, `:1676`,
  unchanged from `4bbe3be` through `765e091` since neither later commit touches
  `src/backend/winget.rs`, re-pointed by Task 9c from `:907`, `:1338`,
  `:1588`, which were correct at `4673517` before `ee46172` shifted the file. Not
  ellipses and not factual errors, but inconsistent with the
  now-fully-named citations in the same file. A fourth, at the retry test, was
  named in full by this task's own comment fix. (The ledger recorded one of them at
  `:1477`; that line number had already drifted by the end of the branch — and
  these three have now drifted twice, which is its own small instance of the class
  this phase kept fixing and is why "cite by symbol, not by line" is the
  recommendation above.)
- ~~**`winget.rs:1109-1124` (`:1077-1088`, then `:1077-1092` after the fix, on earlier trees) read "P2 S2's 40, P2 S4's 15, and P7's 30 against a
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
is the one it deliberately did not close, and the rest are unchanged. Items 16-21
are new in this phase — 16 from Task 8, 17 and 18 from the whole-branch review
and the fix wave that answered it, 19 from Task 9b, and **20 and 21 from Task 9's
own Windows and dogfood runs**.

**20 and 21 exist because two of this phase's three headline changes ship
*structurally verified and live-unverified*, and the runs that were meant to settle
them did not.** Neither is a suspicion that the code is wrong; both are the absence
of an observation. They are numbered rather than buried in the Verification section
because the previous phase's post-merge audit found exactly this — a gate recorded
as met when it was not — and the fix is that an unobserved path gets a number in
this list.

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
   package, and dotpkg cannot tell them which entry they are missing. **And the
   `portable` half that `running_ids` does answer is now known to be
   live-unverified**: Task 9's dogfood could not separate the path signal from
   `guard_names` on the only live subject a14 offered — see item 21. The
   `[winget.guard]` half **is** verified live, by stage A2 with its counterweight.
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
    chosen, not measured to be sufficient on a slower machine. **And nothing has
    yet observed the retry fire at all**: Task 9's dogfood stage C is inconclusive
    on exactly that, which is item 20.
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
    gap in the numbering. **Strengthened, not reopened, by this phase:** both of
    Task 9's completed `--in-diff` runs measured **0 `TIMEOUT`** at `-j 2` (70 and
    72 mutants), so Phase 4b and Phase 5 are now **two consecutive phases** with no
    timeout at that concurrency — Phase 4, the one that produced the 69, is the last
    phase that saw any.
15. **`sys::elevated()`'s runtime behaviour is measured in one direction only.**
    **Still true, and this phase measured only the direction that was already
    measured.** Task 9's Windows suite ran the `#[cfg(windows)] #[ignore]` test by
    name on a real elevated session and it passed, which observes `elevated() ==
    Some(true)` — the direction the item already records as covered. **The
    unmeasured half is untouched:** an ordinary, non-elevated Windows
    session with **no `runas` at all** is still unmeasured. `elevated()` should
    answer `Some(false)` from `TokenIsElevated` alone and never consult
    `CheckTokenMembership`, and nobody has watched it do so. An elevated run cannot
    produce that observation no matter how carefully it is done, which is why
    running the suite on Windows did not move this item.
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
      bucket as this file's own four `#[cfg(windows)]` `sys.rs` mutants below:
      inert on macOS, a platform gap rather than a test gap, resolvable only by a
      mutation run *on* Windows — and for those four, that bucket is now
      **measured** rather than reasoned, by the file-scoped run.
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
20. **The retry ships structurally verified and live-unverified — nothing has ever
    observed it fire.** New in this phase, from Task 9's dogfood stage C.
    **Structural:** the mechanism is provable by reading — `update_source_with(Duration)`
    retries once, after 1 s, and only on `INTERNAL_ERROR`. **Measured:** the trigger
    is real, `3 of 10` on a14 in the measurement round. **What is missing is the
    middle:** six `dotpkg update` rounds against a concurrent `winget list` produced
    **zero** index-refresh warnings, and that outcome **cannot distinguish** "the
    contention never reproduced under `dotpkg update`" from "it reproduced and the
    retry absorbed it invisibly" — a successful retry prints nothing, by design, so
    zero warnings is the expected output of both. **Recorded as inconclusive, not as
    a pass.** Distinguishing the two needs instrumentation the shipped binary does
    not have (a counter, or one line saying the retry fired), which is a production
    change rather than another dogfood round; a second dogfood run under heavier
    contention would produce the same undistinguishable output. Also still open from
    item 11: whether `show` or `list` ever return `0x8A150001` under the same
    contention, and whether 1 s is sufficient on a slower machine.
21. **The winget path signal ships structurally verified and live-unverified — the
    one stage that could have isolated it could not.** New in this phase, from Task
    9's dogfood stage A1. **Structural:** `running_ids` inserts a scanned id into
    `Running.dirs` when a live process's `exe` lies under `<root>/<id>_…`, and
    `covers` is `dirs || names || bins`, so one `dirs` entry serves both fences.
    **Measured, and it is the problem:** the only live process on a14 under a
    `Packages\<id>_<sourceIdentifier>\` directory is `VKey.exe` (pid 9076), and
    `guard_names("PhatMT97.VKey", "VKey")` yields `["vkey"]` while the live process
    folds to `vkey` — so the **`names` half catches it too**, and A1's `! winget
    PhatMT97.VKey  running -- stop it first` is produced by either signal
    indistinguishably. The stage confirms **the fence**, not the path signal.
    **This is not a new discovery, it is a predicted one:** the measurement round
    already recorded that the path signal adds **zero** new catches on a14, and this
    is what that costs at verification time — a signal whose only live subject is
    already covered by a different signal cannot be isolated on that machine.
    **What would close it:** a `portable` winget package on the machine under test
    whose process name `guard_names` does **not** derive from its id — measured to
    exist as a class (`rg` from `BurntSushi.ripgrep.MSVC`, `codex-command-runner`),
    but none of them was running during the dogfood. Not closable by re-running the
    same dogfood on the same machine.

### Inherited verification debt, carried unchanged

None of these is in this phase's scope, and none of them moved. They are listed
so the next phase inherits a named list rather than a surprise.

- **An ordinary non-elevated Windows session with no `runas`** has never been
  measured (item 15 above).
- **Four `#[cfg(windows)]` mutants in `sys.rs`, not three — and this is the
  first time any run has actually observed them.** **Measured** by the
  file-scoped run described in "The file-scoped mutation run" below: three
  function replacements at `src/sys.rs:139` (`elevated` → `None`,
  → `Some(false)`, → `Some(true)`) and one `!=`→`==` at `:163`, all inside the
  `#[cfg(windows)] pub fn elevated()` body. `docs/phase4b-notes.md:593` records
  **three**, and this file repeated it; the count was never wrong in kind, only
  in number, because no run had ever covered the file. Phase 4b says so itself:
  *"NOT COVERED BY THIS RUN AT ALL: `src/sys.rs` (I did not pass `-f
  src/sys.rs`)"*. Task 9's own diff-scoped run did not reach them either, since
  this phase changed no line in that function.

  The characterisation survives the correction and is now **measured rather than
  reasoned**: they are a platform gap, not a test gap. On macOS the
  `cfg(not(windows))` arm returns `None` unconditionally, so the `None`
  replacement is an *equivalent* mutant there; and nothing on macOS asserts a
  value, so `Some(false)` and `Some(true)` survive for the same reason. Only a
  mutation run *on* Windows can distinguish any of the four. **No `cargo
  mutants` invocation has ever happened on a Windows machine in this project** —
  the suite ran there twice, but a *suite* run is not a *mutation* run.
  `package_roots()`'s two survivors (item 19) are blocked on the same missing
  run, so the Windows mutation run is a single gate holding **six** mutants.
- **The accepted equivalent mutant on the `outstanding_skips` check** that Phase
  4b recorded at `main.rs:773`; the call is at `main.rs:856` on this tree.
  Closing it needs a fake scoop binary (which a standing test policy forbids) or
  a production change.
- **16 mutants in `src/backend/winget.rs`, in Phase 4 code, not 14** —
  **measured**: `floor_char_boundary` (**7**), `parse_list` (**7**),
  `parse_versions` (1), `RealWinget::run` (1). `docs/phase4b-notes.md:606`
  records 6 and 6 for the first two, giving 14; the completed file-scoped run
  described below lists seven each. The **functions** Phase 4b named were right;
  two of its four counts were not. This phase added **699** lines to that file (against 15
  deleted) and closed none of them — `git diff --numstat 1d633c6 --
  src/backend/winget.rs`, measured by Task 9c on `4bbe3be` and unchanged since:
  `git diff --numstat 4bbe3be 765e091 -- src/backend/winget.rs` is empty, so the
  figure still describes **`765e091`**, the tree that actually ships.
  **This is the one figure for that quantity**; the table under
  "Sixteen `file:line` citation numbers" above records the two earlier trees (535
  at `c8c7f0d`, 611 at `4673517`) and why they are kept. The number
  read 534 until the whole-branch review (Minor 2) re-derived **535** against
  `c8c7f0d`; the fix wave that answered the review then added 76 more — the new
  retry-delay test, its fake's instant recorder, and the reworded
  `INTERNAL_ERROR` arm — and Task 9b's `ee46172` then added a further **88 net**
  (`+90/-2`) for the `package_roots` split.
  - **Settled by measurement, and the settlement corrects the inherited record.**
    This item previously said that the question needed "a completed file-scoped
    run that no task in this phase produced". That run has since been produced —
    see "The file-scoped mutation run" in Verification — and it resolves every
    open thread here:
    - **The two-function claim was wrong**, as its own arithmetic said it must
      be. `parse_versions` and `RealWinget::run` each contribute one survivor, so
      the survivors sit in **four** functions, exactly the four Phase 4b named.
      The claim came from a run that never finished, and the ledger carried the
      same error; both are withdrawn.
    - **Two of Phase 4b's four counts were low.** `floor_char_boundary` and
      `parse_list` have **seven** survivors each, not six, so the inherited total
      is **16, not 14**. Nothing closed them and nothing reopened them; the
      earlier number was simply never re-derived, because no completed run had
      covered the file since Phase 4b's own, which was scoped by diff.
    - **What that says about the earlier disagreement:** it was not between the
      ledger and Phase 4b's breakdown, as this file previously concluded. Both
      were wrong, in different ways and for the same underlying reason — an
      inherited number that no one re-measured, quoted forward twice.
- **Two mutants in `winget_exec.rs`, inside `RealWingetMutator::run`** — a
  `NotFound == -> !=` and an `unwrap_or(-1) -> unwrap_or(1)`. Covering them means
  spawning a real `winget.exe` from the test suite, which this project does not
  do. **Measured** at `winget_exec.rs:376` and `:385` by the file-scoped run,
  confirming Phase 4b's record for this pair unchanged.
- **Two survivors no previous phase recorded at all**, both in code this phase
  did not touch, both surfaced only because the file-scoped run covered whole
  files rather than a diff. **Measured:**
  - `src/main.rs:627` — `replace > with <`. Distinct from the accepted
    equivalent mutant above, which is at `:856`.
  - `src/backend/scoop.rs:222` — `replace match guard e.kind() ==
    std::io::ErrorKind::NotFound with true` in `<impl Backend for Scoop>::scan`.
    This is the `NotFound`-idiom shape `docs/phase3-notes.md`'s still-open item 4
    named as a family; a mutation survivor is a stronger statement about it than
    the reading that named it, and it belongs to the same untouched-code bucket
    as the rest of this list.

  Neither is in this phase's scope. They are here because a list that omits what
  a run found reads as a list of everything there is.
