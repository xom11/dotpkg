# Open items

The live list. Everything here is either unmeasured, measured-and-still-open, or
a decision recorded so it is not rediscovered as a gap.

Item numbers **1–21** are the numbers `docs/phase5-notes.md` used, and they are
kept even where an item is closed, so a reference written against the old
numbering resolves to its answer rather than to a hole in the sequence. Items
**22 and up** are new here.

## How to read a reference to a record that is no longer in the tree

Twenty documents, about **28,200 lines**, were removed on 2026-08-12 in two
waves. **None of them is gone; none of them is in the working tree.** Each wave
names the commit immediately before it, which is where the files still are:

```console
git show 07dd86b:docs/phase5-notes.md                        # wave 1
git show 3bf1584:docs/plans/2026-08-09-phase4-backend-winget.md   # wave 2
```

**Wave 1, at `07dd86b`** — six dogfood records and six phase notes, ~8,700
lines: `dogfood-2026-08-08.md`, `dogfood-phase2a-2026-08-08.md`,
`dogfood-phase2b1-2026-08-08.md`, `dogfood-phase2b2-2026-08-09.md`,
`dogfood-phase3-2026-08-09.md`, `dogfood-phase4-2026-08-10.md`,
`phase2-notes.md`, `phase2b-notes.md`, `phase3-notes.md`, `phase4-notes.md`,
`phase4b-notes.md`, `phase5-notes.md`.

**Wave 2, at `3bf1584`** — all eight task-breakdown plans, **19,482 lines**, the
single largest thing in `docs/` and 63% of it, being everything that was under
`docs/plans/` and `docs/superpowers/plans/`; both directories are now gone
entirely: `plans/2026-08-08-phase1-status-scoop.md`,
`plans/2026-08-08-phase2a-truthful-plan.md`,
`plans/2026-08-08-phase2b1-prepare.md`,
`plans/2026-08-08-phase2b2-executor.md`,
`plans/2026-08-09-phase3-update-adopt.md`,
`plans/2026-08-09-phase4-backend-winget.md`,
`superpowers/plans/2026-08-10-phase4b-winget-executor.md`,
`superpowers/plans/2026-08-11-phase5-guard-unmanaged-retry.md`. They were the
step lists the phases were built from, every phase they belong to is closed, and
what they established that is still true is either in `docs/specs/` (the
decisions), in `docs/measurements-*` (the numbers), or in this file (what is
still open). A plan is the most perishable document a project produces: it is a
description of work not yet done, and it stops being read the day the work lands.

**What survived, and why those and not these.** Two directories: all of
`docs/specs/`, which holds the designs — why a lock file, why the two backends
pin different things, what dotpkg refuses to do — and every
`docs/measurements-*` document, which holds the raw commands and output that
every claim rests on. Both are what the README and the code cite as evidence;
the plans were cited by nothing. The phase notes were the narrative around the
measurements, and this file is the part of that narrative that is still live.

**What was deliberately not done:** the surviving `docs/specs/` and
`docs/measurements-*` documents still name those files in their prose, and those
mentions were **left exactly as written**. They are frozen records; a sentence
that was true about the tree it was written against stays true, and re-pointing
it would falsify it. This section is how such a mention resolves. The only
reference that was rewritten is one `file:line` citation that would otherwise
have failed `scripts/check-citations.py`, and it is marked at its site.

**The same rule covers a symbol that was renamed: `plan::is_older` is now
`plan::version_order`.** Seven mentions across four frozen documents —
`docs/measurements-2026-08-09-winget.md` and three designs under `docs/specs/` —
name it under the old name, and they were left as written. The rename came with
the change that matters: it returns `std::cmp::Ordering` rather than `bool`, and
a `bool` has no way to answer *the same version, spelt differently*, so a
trailing-zero version — `30.6.4.0` against a pin of `30.6.4` — had to come back
as a downgrade. `Ordering::Equal` is what closed that case, and it deliberately
leaves the prerelease case open: `1.0.0-rc1` against a pin of `1.0.0` still
answers `Greater`. A frozen sentence about `is_older` returning `true` resolves
to `version_order` returning `Less`.

---

## A. Decisions, not gaps

Recorded so they are not reopened as if nobody had looked.

- **1. Downgrading a winget package.** Measured: `winget install --version
  <older>` only ever moves a package up. The alternative — uninstall then
  install — would put a nightly uninstall-and-reinstall loop on every
  self-updating application. dotpkg prints the refusal and tells you to run
  `dotpkg update`.

  **The refusal is unchanged; what closed 2026-08-13 is the other half of it,
  which had never been built.** This item recorded a decision and left no way to
  say the thing a user of a self-updating application actually means. Measured
  on a14 on 2026-08-12, five GUI packages — `Brave.Brave`, `Vivaldi.Vivaldi`,
  `Google.Chrome`, `Discord.Discord`, `Warp.Warp` — had to be deleted from a
  real dotfiles repository's declaration outright, because each moves past its
  pin within days and the correct refusal then floored that module to non-zero
  on every invocation. `[winget.opts] pin = "none"` is the answer: install if
  absent, never manage the version, no `pkg.lock` entry at all. Design in
  `docs/specs/2026-08-13-winget-unpinned-design.md`.

  **What that does not cover, stated so it is not read as wider than it is:** a
  *pinned* package that has moved ahead still refuses, still exits non-zero, and
  `dotpkg update` is still the fix. Nothing about the downgrade decision moved.
- **7. `winget pin`.** Not used. Two sources of truth about permitted versions is
  how a tool starts lying.
- **8. `add`, architecture drift, same-version re-pin, locking against two
  concurrent dotpkg runs, Chocolatey.** All unbuilt. `add` composes today from
  `pkg.toml` + `dotpkg update <pkg>` + `dotpkg apply`.

  **Narrowed 2026-08-13, not closed.** For a package declared
  `[winget.opts] pin = "none"`, `add` composes from `pkg.toml` + `dotpkg apply`
  — with no `update` step at all, because there is nothing to resolve and
  nothing to record. The two-step composition above is now the *pinned* case
  specifically. Everything else in this item stands, and the architecture-drift
  cost recorded below is untouched: this design does nothing about drift.

  **Architecture drift has since cost somebody something, and the cost is
  recorded here for prioritisation — not as a request, and not as a promise to
  build it.** Moving a real dotfiles repository (`github.com/xom11/nix`) onto
  dotpkg 0.1.0 on zenbook-a14 on 2026-08-12 **lost that repository the ability
  to fix drift**. The hand-written PowerShell dotpkg replaced did fix it: that
  is what cleaned **17 emulated x64 packages** off that ARM64 machine on
  2026-08-03. dotpkg is the better tool on every other axis in that repository
  and is strictly worse on this one, which is the first time an unbuilt item on
  this list has been paid for by a user rather than argued about.
- **2 (the rejected fixes, both of them).** A hardcoded `WINGET_HELPERS` list was
  rejected explicitly: it would *exclude* dependency-installed packages from
  `Unmanaged` rather than count them, which is less honest than collapsing a
  line. **`[winget] ignore` was rejected in the same breath and for a different
  reason** — it makes the user maintain 36 entries to silence noise dotpkg
  created. Both refusals live only in
  `docs/specs/2026-08-11-phase5-guard-unmanaged-retry-design.md` §B3, which
  names them together. What remains open about item 2 is below.
- **10 (the rejected oracle).** winget's `installed.db` was opened and priced
  rather than adopted: SQLite, 262144 bytes, a populated `commands` table that
  would raise fence coverage from 4 to 10 of 41 — while disagreeing with
  `winget list` on 31 names, at least 11 of them packages **scoop** installed. A
  catalog that attributes winget ids to scoop's packages is not an oracle for a
  winget mutation. `winget show` prints no Commands field, and `WinGet\Links`'
  shims are portable-only; both measured dead in the same round. See
  `docs/measurements-2026-08-12-phase7-fence-coverage.md`.

## B. Unmeasured winget surface

Nothing here is known to be wrong. Nothing here has been watched.

- **30. `--id`'s substring behaviour is claimed two ways and measured one
  way, on the same verb.** Found 2026-08-13 while designing `pin = "none"`,
  putting two existing statements side by side for the first time.
  `docs/measurements-2026-08-10-winget-write-path.md` §7 probed five bare-word
  substrings of real installed ids — `show --id 7zip`, `Microsoft`, `ripgrep`,
  `git`, `zoxide` — and every one returned `0x8A150014`, concluding **"`--id`
  always requires the whole id; `--exact` only controls case."**
  `winget_exec::list_one_argv`'s doc comment cites that to argue dropping `-e`
  costs nothing. Against it, `CHANGELOG.md` and the same sentence in
  `update::run` and `adopt::run_winget` state that a declared `OhMyPosh` matched
  `JanDeDobbeleer.OhMyPosh` — **also `winget show --id` without `--exact`** —
  and a refusal was built on the strength of it. **That second claim carries no
  measurement**: it is in three source comments, the changelog and a commit
  message, and in no `docs/measurements-*` document.

  **Measured and settled the same day, against the unmeasured claim.** The probe
  this item asked for — `show --id <a trailing dotted segment of a real id>` —
  was run on a14 (winget 1.29.280) on 2026-08-13, through dotpkg's own spawn, on
  a machine where `JanDeDobbeleer.OhMyPosh 30.6.4.0` is installed:
  `show --id OhMyPosh` returns `NO_APPLICATIONS_FOUND`, *"No package found
  matching input criteria."* **§7 holds, and the four unmeasured restatements of
  the opposite are wrong.** Full round in
  `docs/measurements-2026-08-13-phase14-winget-unpinned.md`.

  **Two consequences, and neither is a code change.** The different-id refusals
  in `update`, `adopt` and `apply::resolve_for_ensure` **stay**: one machine and
  one winget version is not grounds for deleting defence at the point of use,
  and a refusal that never fires costs nothing where a missing one costs a
  package installed under a name the plan does not carry. They should be read as
  defending a shape **not observed here** rather than one that was. And
  **dropping `-e` from a write verb would probably have been safe** — the Phase
  14 design refused that option on the strength of this disagreement, and the
  disagreement resolved toward "it would have been fine". That design choice is
  still correct for the argument that never depended on this one: `-e` makes
  `--id` case-sensitive, and an unpinned package has no lock entry holding the
  canonical spelling.

  **The prose was corrected 2026-08-13, at all three live sites.**
  `CHANGELOG.md`, `update::run` and `adopt::run_winget` each stated the refuted
  sentence as the *reason* for a refusal. Each now says what was measured
  instead, and each points here. The refusals themselves are untouched.
  Commit `c3517e7`'s message still carries the original claim and cannot be
  edited; this item is where a reader who follows it lands.
- **3. `--location`, `--all-versions`, and side-by-side versions of one id.** All
  three unmeasured.
- **4. Removing a machine-scope package while elevated.** Unmeasured. The
  shipped refusal stays narrowed to user-scope rather than guessing.
- **5. Any installer type other than `portable`, for the success paths.** And
  `portable` is also the only installer type the path signal can ever see: a
  package directory exists for exactly the 4 `portable (zip)` ids of the 41
  installed on a14 and for none of the other 37, across eight installer types,
  no exception in either direction.
- **6. `--force` and `--purge` against the elevation refusal.** Unmeasured.
- **9 (first half). There is no scan-time source for a winget package's process
  names.** `winget list` does not expose aliases at all; they appear only in
  `install`'s stdout, at install time. A user who writes no `[winget.guard]`
  entry gets no protection for a non-portable package. *(The second half —
  telling the user which entry they are missing — closed 2026-08-12; see §E.)*
- **10. There is no independent oracle for a winget mutation.** Verification
  re-runs `winget list` for the one package, which is winget asked twice, not a
  second witness. Every cheaper lead has been measured dead; see §A.
- **11 (residual). Whether `show` or `list` ever return `0x8A150001` under
  contention is unmeasured** — the reader won 105 of 105 races, so the
  reader/writer asymmetry is inferred, not measured. And **the 1 s retry delay is
  chosen, not measured sufficient**: on a machine proven idle, successful
  `source update` calls run 294–621 ms, but the busy-machine range reached
  1.2–5.4 s, which 1 s does not clear.
- **12. The `--prepare` loop is unmeasured as a loop.** Nothing is parallelised
  or cached; the per-call ~1 s figure is the only number there is.
- **17. The mid-run fence's `dirs` half compares two different winget
  spellings.** `Running.dirs` holds `winget list`'s `Id`; the re-sampler asks
  about `winget show`'s `Id` as `parse_show` read it back. `Name` folds case, so
  only a non-case difference could make the `dirs` half answer "not running"
  mid-run for a package the plan-time fence could see. **Measured 36 of 36 on
  a14: byte-identical, 0 differing at all.** First evidence either way; still not
  a guarantee about the next package. Stays open as *measured, no difference
  observed*.

- **31. `[winget.opts]` refuses an entry for an undeclared package;
  `[scoop.opts]` accepts one and ignores it.** Deliberate asymmetry, 2026-08-13.
  A `[winget.opts]` entry naming a package `[winget] packages` does not declare
  is a parse error, because the failure it prevents is silent and expensive: a
  typo in `packages` spelled correctly in `opts` yields a **pinned** package
  where the user asked for an unpinned one, and the only symptom is the
  refused-downgrade line the entry was added to remove. `[scoop.opts]` has the
  same inertness — a bogus name there silently loses an `arch` pin — and was
  deliberately left alone rather than have a winget decision widened over
  another backend's file in the same change. Whether scoop should gain the same
  rule is unexamined.

## C. Coverage and mutation debt

- **13. Inherited from Phase 4, none of it in a later phase's scope:**
  `plan_backend`'s unconditional `Arch::as_scoop()`; the design's "a new backend
  slots in without touching the planner" promise being half true (see item 24);
  `verify.rs`'s `NotFound`-idiom guard; `parse_list`'s missing-`Version` refusal
  branch being untested defensive code — the one test of that refusal feeds a
  French header, which trips the `Name` arm of the same loop first, so neither
  the `Id` arm nor the `Version` arm has ever been reached; no fixture pairing a
  plain `show` with `show --versions` for the same package; `resolve_installed`'s
  `fell_back_to_tip` warning path being untested.

  **One entry left that list on 2026-08-12 and the rest of it stands.**
  `floor_char_boundary` was carried here as untested defensive code beside the
  missing-`Version` branch, and it is not untested any more: it has its own test
  and its own control, and 3 of its mutants are killed — the four-kinds
  breakdown further down *Coverage and mutation debt* is where that is recorded,
  along with the 3 survivors it characterises as equivalent. The branch it was
  paired with is still open, which is why the item is still here.
- **A fourth de-elevation route from the a14 ssh session works, where three
  recorded ones do not.** `scripts/nonelevated-mutants.ps1` names `runas
  /trustlevel:0x20000`, `schtasks /RL LIMITED` and `Shell.Application` as
  measured failures. `gsudo -i Medium` (gsudo 2.6.1) succeeds — measured
  2026-08-13, used to perform a user-scope `winget uninstall` that the elevated
  session could not. The command must sit in a **`.cmd` file**: passed as gsudo
  arguments it is routed through PowerShell, which parses `uninstall` as an
  expression and fails before winget is reached. This is the route that can
  close item 29's removal half, and it may also be what `src/sys.rs`'s
  ordinary-Windows mutation run needs.
- **`src/sys.rs` needs three mutation runs, not one.** macOS, elevated Windows,
  and ordinary Windows — because each platform is blind to the other's `cfg`
  arm, and **no two of them together kill all six mutants**. A Windows-only run
  closes three and silently reopens the two that live in the
  `#[cfg(not(windows))]` arm. The documented invocation is also wrong in two
  ways: `cargo mutants -- --include-ignored` does not reach libtest (it needs the
  two-`--` form), and `cargo install cargo-mutants --locked` cannot build on a14
  at all — it pins `winapi` 0.3, which fails on `aarch64-pc-windows-msvc`.
- **`floor_char_boundary` no longer exists, so 6 of the counts below describe a
  function that is not in the tree.** Deleted 2026-08-13, and the reason is that
  it was defending the wrong thing. It walked a byte offset back to a character
  boundary so that slicing a data row at a *header byte offset* could not panic
  mid-character — but winget pads to **character** columns, measured: on
  `tests/fixtures/winget/list-full.txt` line 67 the `Id` column's content
  begins at character 64 and **byte 66**, while the header puts `Id` at 64. The
  byte offset was never the right place to cut, and flooring only chose which
  wrong place. `parse_list` now maps the header's character columns through
  each line's own `char_indices`, which cannot land mid-character, so there is
  nothing left to floor.

  What that does to the four kinds below: its **3 killed** and its **3
  characterised as equivalent** are both retired rather than resolved, leaving
  **9 genuinely open and unexamined** as the only figure still live. The
  function still reads at `git show 2fdcc69:src/backend/winget.rs`. Neither
  `docs/measurements-2026-08-12-phase10-mutation-debt.md` nor
  `docs/measurements-2026-08-12-phase5-residuals.md` was touched: both are
  frozen records and were true of the trees they were taken on.
- **The 20 surviving mutants were sorted into four kinds on 2026-08-12 and are
  now 15**, plus one detected only by timeout. Full round in
  `docs/measurements-2026-08-12-phase10-mutation-debt.md`. Counting them as one
  number was hiding that only two of the four kinds are debt at all. **Read the
  entry above first**: two of the four kinds no longer have a function to be
  about.

  - **Accepted by design, 3 — not debt, and should stop being carried as it.**
    `RealWinget::run` (1) and `RealWingetMutator::run` (2) are the only
    functions that spawn `winget.exe`; every test reaches a fake through the
    `WingetCmd` / `WingetMutator` seam, which is why 662 tests run on macOS.
    Killing these needs a real winget spawned from the suite, which this
    project forbids. They are the shadow the seam casts.
  - **Characterised as equivalent, 3.** `floor_char_boundary`'s survivors after
    the new tests: both `:43` mutants are unreachable-true because both call
    sites clamp first (`end.min(line.len())`, and an early return when `start >=
    line.len()`), and `:46 > → >=` is equivalent outright since
    `is_char_boundary(0)` is always true. *Equivalent under current callers* is
    weaker than *equivalent* and is stated that way on purpose: a future caller
    passing an unclamped index would make one of them wrong.
  - **Killed, 4.** Three in `floor_char_boundary`, whose loop had never executed
    once, and one in `Scoop::scan`, which was the dangerous one: with its
    `NotFound` guard replaced by `true`, every read failure reads as "no scoop
    packages installed", and an empty scan is the one input that turns every
    owned package into a prune candidate.

    **The reason given for that dead loop was wrong, and correcting it moves no
    count.** It was recorded as *"all 15 fixtures are from an en-US machine and
    are pure ASCII"*, and they are not:
    `tests/fixtures/winget/list-full.txt` line 67 carries two `®` (U+00AE) in a
    package name, which are the only non-ASCII bytes in any captured fixture.
    What actually kept the loop dead is narrower — **no column offset in any
    fixture lands inside a multi-byte character.** Those two `®` occupy bytes
    16–17 and 30–31, well inside the `Name` field, while the offsets `parse_list`
    slices at on that table are 0, 64, 152, 182 and 212, every one of them an
    ASCII byte, so `is_char_boundary` answered true at each and the body never
    ran. Pure ASCII was sufficient for the loop to be unreachable and never
    necessary, and the fixture that separates the two was already in the tree
    when the claim was written.
    `docs/measurements-2026-08-12-phase10-mutation-debt.md` states it the old way
    and is left as written, as a frozen record.
  - **Genuinely open and unexamined, 9:** `parse_list` 7, `parse_versions` 1,
    and one in `src/main.rs` (`replace > with <`, distinct from the accepted
    equivalent mutant on the `outstanding_skips` check). The `parse_list` block
    is the largest and is the next thing worth measuring.

- **The idle gate's threshold was a14's, and had never been re-derived here.**
  Default 10%; this macOS machine's measured floor is **17.10–19.47%**, so the
  gate refused every run until it was calibrated (3× the maximum, the procedure
  `scripts/idle-baseline.sh` itself prescribes). **And the agent running a
  measurement is part of the load it measures** — three `claude` processes were
  among the largest burners. On a single machine "idle" cannot include the
  process asking the question; the a14 rounds never hit this because the load
  and the controller were on different machines.
- **The `opaque` arm of `apply_guard_overrides` has unit coverage only.**
  `tests/cli.rs` strips winget from `PATH`, so no test there can produce a
  sourceless row — and `opaque` is the **majority** case on real hardware, 90 of
  126 ids in the last live capture. The guard-merge half of the same function is
  covered live; this arm is not.

## D. Verification scope — what has never been observed

This is the section the README's "Verified on" table is the summary of.

- **20. The winget source-refresh retry has never been observed firing.**
  Structural (it retries once, after 1 s, only on `INTERNAL_ERROR`), and the
  trigger is real (measured 3 of 10 against one competitor, later 1 of 10 against
  four). Instrumented since 2026-08-12: `update_source` returns `FirstTry` or
  `AfterRetry`, and `AfterRetry` prints a warning naming `0x8A150001`. **70
  contended rounds in three configurations printed the line zero times.** The
  plumbing was proven rather than assumed — with winget off `PATH` the sibling
  arm of the same `match` printed — so the zero is a fact about the trigger.
  Decided: keep the retry, record 70 rounds as a lower bound, stop hunting a
  sharper trigger.
- **22. No x86_64 Windows machine has ever run dotpkg.** The published x64 binary
  has never been started on real hardware; a *different* build of it answered
  `--version` on a CI runner, and that is the whole of it.
- **29. The version-change path.** Split out of 23 on 2026-08-12 so that closing
  23 would not read as covering it. **Half closed the same day, and the halves
  are not interchangeable.**

  **Closed for scoop, in CI.** The `scoop-integration` job now publishes a second
  bucket commit at 1.0.1 — a different archive, hash and url, so scoop cannot
  satisfy the install from cache and skip the download half — and applies it.
  Read from the run's own output: `installed_before: 1.0.0`, the plan presenting
  it as `^ scoop ci-payload 1.0.0 -> 1.0.1 (upgrade, 64bit)` rather than as an
  install, `done scoop ci-payload verified on disk`, `installed_after: 1.0.1`,
  and `shims_after: ci-payload, ci-payload.cmd` — so a shim survived the window
  in which the package is absent. Ownership survived it too. This is the first
  time anything has watched scoop's irreducible gap happen.

  ~~**Still open:** the published binary has never performed a version change on
  real hardware~~ — **closed for scoop 2026-08-12**
  (`docs/measurements-2026-08-12-phase11-release-version-change.md`), and in
  **both directions**: the published artifact drove `jq` 1.8.2 → 1.8.1
  (`v … downgrade, from lock, arm64`) and 1.8.1 → 1.8.2 (`^ … upgrade, 64bit`),
  each `verified on disk`, with the shims and the ownership surviving the window
  in which the package is absent. Both bucket commits were resolved from the
  bucket by the script rather than typed.

  Two things came out of it that nobody was looking for: the **architecture
  changed underneath the version** (arm64 at 1.8.1, 64bit at 1.8.2 on an ARM64
  machine) and dotpkg named it on the plan line before the user consented, which
  is the `arch` design working; and **`ScanOutcome::Unscannable` fired on real
  hardware for the first time** when winget became unreachable on that machine
  between rounds — the unmanaged count dropped to scoop's 24 and the winget
  backend was reported unreadable rather than empty.

  ~~**Still open: no winget mutation has run anywhere** outside the Phase 4b
  rounds and their own trees.~~ — **the install half closed 2026-08-13; the
  removal half did not, and the two are not interchangeable.**

  **Closed for install.** `dotpkg apply --yes` performed a real winget install
  on a14 — `ducaale.xh` 0.26.2, declared `pin = "none"`, `done winget
  ducaale.xh verified on disk`, confirmed present by a fresh scan afterwards,
  with **no `pkg.lock` file written at all** and ownership recorded in
  `state.json`. Idle gate `VERDICT: IDLE` (3.06% machine-wide) recorded before
  it, as the standing rule requires. Full round in
  `docs/measurements-2026-08-13-phase14b-winget-mutation.md`.

  **Still open for removal, and refused rather than skipped.** The prune was
  planned and prepared and then refused at exit 2 by
  `refuse_elevated_winget_removal`: the ssh session is elevated and the package
  is user-scope, both measured directly. `WingetStep::Remove` has therefore
  still never run outside Phase 4b. The refusal was **vindicated in the same
  round**: winget itself, asked to perform that uninstall from that session,
  returned `0x8A15007D` and *"The package installed for user scope cannot be
  uninstalled when running with administrator privileges."* — the exact code
  `winget_exec::CANNOT_UNINSTALL_ELEVATED` names. The pre-check and the
  behaviour it predicts were observed agreeing.

  **How to close the remaining half:** run `dotpkg apply --yes --allow-prune`
  under `gsudo -i Medium`, which §4 of that round measured to de-elevate
  successfully where `scripts/nonelevated-mutants.ps1`'s three recorded routes
  do not. The elevation pre-check will not fire there.
- **One machine, one architecture, one winget version, one scoop layout, and one
  elevated session.** `zenbook-a14`, aarch64, winget v1.29.280. Nothing has
  observed `apply` from an ordinary non-elevated session.

## E. Structural debt found by the 2026-08-12 design review

- ~~**24. `trait Backend` does not cover the write path.**~~ — **the accidental
  half is closed 2026-08-12; the deliberate half is not, and the distinction is
  the finding.** The design specified `scan / resolve / install / uninstall /
  helpers` in one trait, to buy *"the backend trait exists from v1 so choco
  slots in without touching the planner."* The shipped `Backend` is read-only,
  and the write path was **two unrelated per-backend seams** (`execute::Mutator`
  for scoop, `winget_exec::WingetMutator` for winget) threaded through `execute`
  and `run_step` as a hand-written pair of parameters — so a third backend meant
  a third parameter at 27 call sites rather than a third implementation of
  anything.

  **What closed:** `execute::Mutates` now names the write contract at the *step*
  level, with `Step` as an associated type so one backend's executor cannot be
  handed another's step. `ScoopSide` and `WingetSide` implement it, and
  `execute::Backends` carries them in one value, so a backend is now a field
  rather than a parameter everywhere. Behaviour is unchanged, measured name by
  name: 658 tests before and after, **identical name sets, 0 lost, 0 added**,
  and the new indirection was confirmed live by a negative control (routing the
  scoop side at a bogus root turns the suite red).

  **What is deliberately still open, and must not be counted as debt:** a third
  backend still owns a `Step` variant, an arm in `run_step`'s wildcard-free
  match, its own process seam, and a `plan::Capability` decision. Those are
  decision points a new backend should be *made* to face — a compile error is
  the only reliable way to ask the question — and merging the two process seams
  into one argv-shaped trait would flatten a real difference (scoop installs a
  staged manifest path with an architecture; winget sets a version by id) or lie
  about it. The design's promise is now true for reading, true for the plumbing
  of writing, and honestly false for the four decisions, which is as close to
  true as it should get.
- ~~**25. The design's third test layer has never existed.**~~ — **built and
  green 2026-08-12.** It specified *"Real scoop in a throwaway `$env:SCOOP` on a
  Windows runner, gated"*, and `tests/cli.rs` was never it: that suite is
  hermetic on purpose, with `SCOOP` and `LOCALAPPDATA` pointed at temporary
  directories and winget stripped from `PATH`, so nothing in it had ever run the
  scoop binary.

  The `scoop-integration` job in `.github/workflows/ci.yml` installs **scoop
  0.5.3** into a throwaway root, builds a bucket that is a real git repository,
  and serves a real archive over HTTP from `127.0.0.1` so that **scoop performs
  its own download and its own hash check** — the part that must not be faked,
  since dotpkg never verifies a hash itself and must never pass
  `--skip-hash-check`. Hermetic despite being real: the only outbound call is
  scoop's own installer.

  First green run, read from its own output rather than from a job status:
  payload sha256 `f0d25d4f…`, bucket head `0ad53670…`, `+ scoop ci-payload 1.0.0
  (new pin)` → `ready` under `--prepare` with no app directory and no ownership
  → `done scoop ci-payload verified on disk`, `1 verified on disk, 0 failed, 0
  held` → **`refusal_exit: 2`** with the package still installed → `- scoop
  ci-payload 1.0.0 (dropped, no longer declared)` → `(prune, owned)` → `done …
  verified on disk`.

  ~~**What it still does not cover:** a version change. The job installs and
  removes; a scoop `Replace` needs a second manifest in the bucket and is the
  obvious next step for it.~~ — **closed 2026-08-12.**
  `.github/workflows/ci.yml` gained the step that was called obvious here:
  *publish 1.0.1 into the bucket, so a version change has somewhere to go*. The
  job now installs, changes a version and removes, and it asserts the plan
  presented the middle one as a version change rather than as an install. What
  that run measured is under item 29.
- **26. `dotpkg` cannot install itself.** The design's phase 5 said release
  *"through the existing scoop bucket"*. The release is GitHub binaries plus
  `SHA256SUMS`; there is no scoop manifest, so the tool is not distributed by the
  mechanism it advocates.

## F. Found on real hardware 2026-08-12, not looked for

All three came from `docs/measurements-2026-08-12-phase8-release-apply.md` §7.
The third of them, item 27, closed the same day and is in the table below.

- **28. Three installed scoop packages cannot be read at all on a14** —
  `actionlint`, `antigravity`, `zellij`, each `cannot read manifest.json: The
  path cannot be traversed because it contains an untrusted mount point. (os
  error 448)`. dotpkg behaves correctly and visibly: it names the package it
  could not read rather than counting it absent. What is open is that **nothing
  in `docs/` records os error 448 before this round**, the condition is silent to
  scoop itself, and its cause is unmeasured. It is also the whole explanation of
  a number that would otherwise look wrong: 31 app directories, 24 reported
  unmanaged (31 − 3 unreadable − `scoop` − 3 helpers).
- **A reading correction worth keeping, since it was nearly carried into a
  conclusion.** `& winget --version` fails with `Access is denied` from the ssh
  session, because the alias is a 0-byte execution stub — but dotpkg's own scan
  read **36** source-backed ids in that same session. The limitation is
  PowerShell's `&`, **not** winget being unavailable, and the first probe of the
  round read it the other way round.

## Closed, with what closed it

Kept so a reference to one of these numbers finds its resolution.

| # | What it was | Closed by |
|---|---|---|
| 14 | 69 unresolved `timeout` mutants from Phase 4 | Phase 4b: 0 `TIMEOUT` over 253 mutants at `-j 2`, then two more consecutive phases with none |
| 15 | `sys::elevated()` measured in one direction only | 2026-08-12 citation round: a non-elevated session obtained by hand, `elevated -> Some(true)` **CAUGHT** |
| 16 | Neither `pkg.toml` round-trip guard covered `[winget.guard]` | 2026-08-12: both guards pinned by tests carrying positive controls, both confirmed able to fail |
| 18 | `tests/prepare.rs` duplicated `common::git` | 2026-08-12: `common::init_repo` does the `git init` and both `config` calls together; the only `git init` in `tests/` is inside it |
| 19 | Two `package_roots()` mutants surviving | 2026-08-12: one assertion tying `package_roots()` to the environment it reads; both die, on macOS and on Windows |
| 21 | The winget path signal shipped live-unverified | 2026-08-12: observed firing on `BurntSushi.ripgrep.MSVC` with both name signals measured dark, plus a counterweight run with scoop's `rg` that correctly did **not** skip |
| 23 | `apply` had never been exercised from a release binary | 2026-08-12: the published artifact, sha256 `9daeae0c…` and no rebuild of it, installed and then pruned a real package on a14, verified both on disk, and left the other 31 untouched — preceded by a counterweight run that refused at exit 2 with the flags withheld. `Install` and `Remove` only; the version change is item 29. Full round in `docs/measurements-2026-08-12-phase8-release-apply.md` |
| 27 | `update` and `apply` left a `.bak` beside what they rewrote, and nothing said so | 2026-08-12: **a `.bak` is for a file nothing else can recover.** `state.json.bak` stays — `state.json` is deliberately not committed; `lock::save` and `config_edit::save` no longer write one, because `pkg.lock` and `pkg.toml` are committed and `git checkout` recovers strictly more than a copy of the last version. Two corrections outlive the decision: **nothing ever accumulated** — each path is fixed (`with_extension("toml.bak")`), so there was at most one `.bak` per file and it was overwritten in place, and the original accumulation claim was wrong; and production **wrote** a `.bak` in three places and **read** one in none, every read in the crate sitting behind the `#[cfg(test)]` boundary — the displaced file was an artefact for a human, never an input to the program |
| 9 (second half) | dotpkg could not tell you which `[winget.guard]` entry was missing | 2026-08-12 fence-coverage round: 27 of 30 pending changes warn; the 3 silent ones are exactly the `portable` ids the path signal already covers |
| 2 (the flood) | An undeclared package appearing after an install produced a wall of `?` lines | Collapsed to one line per backend, with `--show-unmanaged` to expand |

**Two numbers that are confirmed rather than corrected**, because both have been
mistaken for errors: `winget export -s winget` reports **41** ids on a14 and
dotpkg's own scan reports **36**; the common set is 36 and the five extra are
exactly the ids dotpkg refuses to read a version for. 41 = 36 + 5, and each
denominator answers a different question — so **"32 of 36"** remains correct for
how many packages dotpkg could act on and could not see, and "4 of 41" is correct
for how many ids winget reports that have a package directory.

## The rules these items were measured under

Carried forward because every one of them is the scar of a defect that recurred.

- **Verify by output, never by exit code.** PowerShell leaves `$LASTEXITCODE`
  stale when it cannot *start* a native command, so a stale `0` is
  indistinguishable from success. A refutation needs measuring as carefully as an
  assertion: withdrawing a correct claim on a broken probe is worse than never
  testing it.
- **Compare name by name; never subtract totals.** Name sets come from
  `cargo test -- --list`, not from run output.
- **Citations into code name a symbol, never a line.** `tests/citations.rs`
  enforces it in the suite; `scripts/check-citations.py` gates `docs/`. A
  citation true about a past tree is marked historical with that tree's sha
  rather than re-pointed.
- **`.ps1` files carry no backtick at all, including in comments** — a backtick
  in a comment still parses, so parse-check and backtick-check are two separate
  gates. `scripts/check-ps1-style.py` is the second one.
- **Run the idle gate before any mutation run and record what it printed.**
  Thresholds differ per machine; one default is not right for both.
- **A test that has never been watched fail has not been shown to test
  anything.** Every assertion here that says "pinned" means a negative control
  was run.
