# Carried forward out of Phase 4b

Findings from building Phase 4b (the winget executor: `apply` installs,
upgrades and removes winget packages, and deliberately never downgrades one),
plus everything the whole-branch review found by reading the branch as one
change rather than as seventeen tasks.

Same discipline as `docs/phase4-notes.md`: every item says whether it was
**measured**, **structural** (true by the shape of the code, provable by
reading, not by running a machine), or **reasoned only**. Where a claim is
reasoned only, that is stated rather than dressed up. `docs/phase2-notes.md`,
`docs/phase2b-notes.md`, `docs/phase3-notes.md` and `docs/phase4-notes.md`
still hold the earlier items; this file does not repeat them except where a
Phase 4 "still open" item was in this phase's scope and its status changed.

The full execution record — every task, every review finding, every ruling,
including several corrections made to the controller — is
`.superpowers/sdd/2026-08-10-phase4b-winget-executor/progress.md`.

## Read this first

### The two user-visible behaviour changes that are not additions

The first is the only one that can make a previously-passing `apply` refuse.
The second only ever lets a run through that used to be stopped, so it is
listed second — but it is a behaviour change, not an addition, and a reader
scanning for "what moved" needs both.

#### 1. A declared winget package with no lock entry now fails the run

**A declared winget package with no `pkg.lock` entry now fails the whole
`apply` run (exit 2), where it used to be a benign report line.** Identical to
scoop's rule, and the same rule the whole tool has always applied: resolving a
version is `update`'s job, not `apply`'s. The old exemption existed for exactly
one reason — `apply` could not act on a winget package at all, so refusing the
run over a missing pin helped nobody — and that reason is gone. The fix is
`dotpkg update`, and `update` really can now resolve a winget package, which it
could not when the exemption was written.

Structural, and pinned by `tests/cli.rs`. It is the only change on this branch
that can make a previously-passing `apply` refuse.

#### 2. A winget-only run no longer needs a scoop root

`apply` used to require a discoverable scoop root before it would do anything.
A machine with winget packages and no scoop was therefore refused, even though
nothing in the run touched scoop. The root is now required only for runs that
actually contain a scoop step.

This can only turn a refusal into a run — never the reverse — so it cannot
break a working setup. It is here because it changes what `apply` does on a
machine whose configuration did not change.

### winget's exit code is never the verdict. The verdict is always the re-scan

The central rule of the whole executor, and it is **measured**, three separate
ways:

1. `0x8A15002B` ("No available upgrade found.") is returned both when the
   package is *already exactly* at the version asked for — a success — and when
   the request was a downgrade winget declined — a failure. One code, two
   opposite facts.
2. `0x8A150014` does not distinguish "not in the index" from "not installed".
   From a removal, "not installed" is the *desired end state*.
3. `install --version <pin>` against an installed package silently reinterprets
   itself as an upgrade toward `<pin>`.

So after every mutation the executor runs `winget list -e --id <canonical id>`
and judges from that, re-applying **the same three opaque rules the scanner
uses** rather than a second, looser executor-only check. An id that comes back
sourceless, `"> "`-prefixed or version-disagreeing after a mutation is "dotpkg
cannot confirm this", never "done".

**And the asymmetry this leaves is structural and must not be papered over.**
For scoop, `verify::verdict` compares the installed manifest's *bytes* against
the staged file — a content address, checked against something the mutation did
not write. Winget has no equivalent. **Not because winget has no hash** — it
has one and verifies it (`Successfully verified installer hash`; `winget show`
prints `Installer SHA256`). What winget lacks is an **on-disk manifest or hash
dotpkg can read back after the install**. So the winget verdict re-asks the very
tool that just performed the mutation, and a fake on that seam proves only that
it is self-consistent. That is the same kind of gap `Installed.bins` being empty
already was for this backend: real, and worth saying plainly rather than
dressing this check up as equal in strength to scoop's.

Two ends of that one cross-reference disagreed with each other for most of this
branch — `execute.rs`'s module doc said "winget has a hash, just not an on-disk
one", `WingetState`'s doc said "there is no manifest, no hash, nothing" — and
`execute.rs` pointed at `WingetState` as where the weakness was "said plainly".
Caught by the whole-branch review, not by any of the seventeen per-task reviews.

### `opaque` is the ordinary shape of a winget machine, not an edge case

**Measured on a14: 84 of 126 ids came back with no `Source` at all**, plus a
`"> "`-prefixed pair and three ids whose two rows disagreed on version. A
sourceless row cannot be compared against any index, so it is "installed, and I
cannot establish its state".

This matters far beyond the report line, and the whole-branch review found one
place that had it wrong: `reconcile_ghosts` built its "present on the machine"
list from `Scan::installed` alone. An owned package in `opaque` therefore had
its ownership record **deleted while the package sat installed**, and the run
printed "ownership record dropped: nothing by that name is installed" about a
package that is installed. Three costs, all silent: `owned_count(WINGET)`
shrinks, and that is the number `mass_prune_guard` reads; an
`Ownership::Adopted` record is unrecoverable, because a later dotpkg install
writes `Installed`; and an adopted-then-undeclared package becomes permanently
`Unmanaged`, unprunable by the tool that adopted it.

Fixed, for both backends, with a non-empty-`opaque` test. **The reason nothing
caught it for the whole branch is worth more than the fix:** the test that
existed constructed `opaque: Vec::new()`. A guard whose fixture cannot express
the hazard is `docs/phase4-notes.md`'s "test that cannot fail" class again, one
step subtler than the two instances that phase named — the assertion is real and
the code is real, and the *input* is the thing that can never reach the branch.

### `pkg.toml`'s spelling and `pkg.lock`'s spelling are two different strings, and only one may go on a winget command line

**Measured** (`docs/measurements-2026-08-10-winget-write-path.md` §6): `--exact`
is what makes `winget --id` case-sensitive, on the **write** verbs as well as
the read ones. `install -e --id SHARKDP.HYPERFINE --version <x>` returns
`0x8A150014` "No package found matching input criteria." where the
correctly-cased call reaches `0x8A150017`. The package-level failure outranks the
version-level one, same ordering as the read side.

A case mismatch between the two files is a **supported state, not a typo the
tool rejects**: `update` warns about it and says "pkg.toml is left as you wrote
it", and `adopt` deliberately writes the user's own spelling. So both spellings
really do exist on a real machine, by design.

The whole-branch review found that the write argv carried the wrong one, and
that every one of the seventeen per-task reviews had missed it because each saw
only a piece: the planner builds the action from `pkg.toml`'s string, the
liveness check obtained winget's own canonical echo and **threw it away**, and
`plan_to_steps` copied the declared string into the step that `set_argv` puts
`-e` beside. The failure was total and silent in the worst possible way:
`--prepare` printed `ready` (the liveness check omits `--exact` on purpose, so
winget folds case), the install found nothing, and the rescan — the same wrong
spelling — missed too, so the run told the user **the package does not exist**.
It does. Every run, forever, exit 1.

The rule, stated once so it is not re-derived: **a mutating winget call may use
`-e --id` only with a spelling winget itself produced.** `pkg.lock`'s key is
exactly that. `Name::key()` — the ASCII-folded form every *internal* comparison
in this crate uses — must never reach a winget command line. Both directions of
that rule now have a test.

### dotpkg does not compute the downgrade direction, and that is deliberate

A winget package installed **ahead** of its pin is a real, measured shape
(`Brave.Brave`: installed `151.1.93.134`, index newest `151.1.93.132`). dotpkg
does not decide which version is newer. It fires `install --version <pin>` and
**winget's own measured refusal is the gate**, translated into a named failure
that says to run `dotpkg update`.

This is what keeps `plan::is_older` cosmetic, and that is load-bearing: its own
doc comment warns that whoever gates on it "owes it a real version comparison —
pre-release ordering, non-numeric suffixes, and the `pa.is_empty()` string
fallback all become answers a user can be hurt by", and winget versions include
`v0.2026.07.15.08.55.stable_01` and `26.01.00.0` against `26.02`. Delegating the
direction to winget avoids that debt entirely and is more honest: dotpkg finds
the answer out rather than guessing it.

**The design's plan-time reasoning about this expired inside the branch, and
nobody noticed until the whole-branch review.** The design said plan-time display
needed no change because `Divergence::Change { from, to }` carried one shape for
both directions and printed no arrow. Task 13 deleted `Divergence`. So an
ahead-of-pin package rendered as `v winget Brave.Brave 151.1.93.134 ->
151.1.93.132 (downgrade, from lock)`, **with** an arrow, was counted by
`change_count()`, and flowed into the prompt's "N installed" — announcing, in the
line a user consents to, a downgrade the tool had already decided never to
perform. Fixed at the render and the count, never at the planner.

**The residual, recorded rather than hidden:** because `is_older` is cosmetic, it
can pick `Downgrade` for a suffixed version pair where the machine is really
*behind* its pin, and the new line then predicts a refusal that does not happen
— the package is upgraded instead. That errs toward "we will not act" where the
tool then acts, which is the safe direction of the two; the opposite error
announces a change and then refuses it.

### A winget version change is ONE call. A scoop version change is not

**Measured**: `install --version <pin>` performs the upgrade directly
(`0.24.1 -> 0.26.1`, exit 0). scoop cannot change a version any other way than
uninstall-then-install, because `install` over an installed app is a measured
no-op — so a scoop version change opens a window in which the package is absent
and a winget one does not.

This is the sharpest divergence between the two executors and the design said
outright that it "must not be left implied". It was implied, for the whole
branch, in the one line a user reads before saying yes: *"Every version change is
an uninstall followed by an install, in both directions."*
`count_replaces_and_installs`'s own doc comment said that sentence is "measurably
false of `WingetStep::Set`" and then left it in the prompt. Now conditional on a
scoop replacement actually being in the run, scoped to scoop, and pinned by a
test — this project has fixed a false number in that exact line three times now,
and the third fix is the one that gave it an assertion.

**`winget upgrade` is deliberately never used.** Measured: it goes to the
*newest* version in the index, not to a requested one — it took the guinea pig
from 0.26.1 to 0.26.2 while the pin was neither. A pinning tool cannot use a verb
whose target is "latest".

### A high-integrity `apply` can install a package it is structurally unable to remove

**Measured** (§4/§5): `winget install` succeeds elevated; `winget uninstall` of
that same user-scope package, from the same elevated session, is refused with
`0x8A15007D` — *"The package installed for user scope cannot be uninstalled when
running with administrator privileges."* Paired positive control, same machine,
same package, same argv, one variable changed (the process's integrity level):
exit 0, removed.

dotpkg's whole shape is a scheduled `apply`. This is not a transient failure the
next run clears; it is a property of the integrity level the run happens under,
and every prune would fail forever. So: **a pre-check plus a translation, never
either alone.** The pre-check refuses before anything runs when the process is
elevated *and* the package is user-scope; the translation catches whatever the
pre-check lets through.

Three deliberate narrowings, each because the measurement did not reach further:

- Scoped to **user-scope** packages only. Whether a machine-scope package can be
  removed while elevated is **unmeasured**, so the refusal does not key on
  elevation alone.
- `sys::elevated()` returning `None` ("could not tell") must **not** refuse.
  A refusal has to be caused by a measured hazard, never by a missing answer.
- The scope query returning "could not tell" likewise does not refuse; it warns,
  names the package, and lets the translation catch it.

`--force` and `--purge` against this refusal are **unmeasured** — the
de-elevated route succeeded first and the round stopped there.

### `sys::elevated()` reads two signals, not one

**Not mentioned anywhere else in this file, or in
`docs/specs/2026-08-10-phase4b-winget-executor-design.md`, before the
post-merge audit found the omission.** Both documents still describe a
single Win32 signal (`TOKEN_ELEVATION.TokenIsElevated`). A reader of only the
shipped documents would have concluded `sys::elevated()` reads one signal --
wrong, and the omission ran in the direction of hiding this branch's single
most safety-relevant change.

`elevated()` is now built on `verdict(is_elevated: Option<bool>, in_admins:
Option<bool>) -> Option<bool>`, a pure function extracted so macOS can test
the rule a Windows-only body implements: `Some(true)` only from
`(Some(true), Some(true))` -- `TokenIsElevated` says elevated **and**
`CheckTokenMembership` says the token is still an enabled member of the
built-in Administrators group. Any Win32 failure, on either signal, still
yields `None`, and both callers still treat `None` as "do not refuse".

**Why:** a `runas /trustlevel:0x20000` shell -- a restricted token, the exact
shell this branch's own refusal message sends an operator to re-run `apply`
from -- reports `TokenIsElevated = 1` while `IsInRole(Administrators)`
reports `false`, and a real `winget uninstall` from that shell **succeeds**.
The single-signal version refused it anyway: the over-refusal this whole
elevation pre-check exists to prevent, reproduced by the pre-check itself.

**Measured, on a14, in the direction the fix exists for:** from that same
restricted-token shell, `dotpkg apply --yes --allow-prune` of a user-scope
`ducaale.xh` exited 0, printed `done winget ducaale.xh verified on disk`, and
the package was really gone -- where the single-signal version returned exit
2 and refused that same shape. `CheckTokenMembership` honours DENY_ONLY for
this shape, and `verdict` has now been seen answering `Some(false)` where
`TokenIsElevated` alone says otherwise.

**Still unmeasured: an ordinary non-elevated session with no `runas` at
all.** It should answer on the first signal alone and never consult the
second, but nobody has watched it do so. This is the exact condition this
file's own pre-merge watch-list named ("both halves of the elevation
pre-check, at both integrity levels -- a pass on only the elevated half is a
refusal that might be refusing everything") and it was not met by merge: the
two Windows suite runs that exercised `CheckTokenMembership` at all did so
only through the `#[ignore]`d elevated-only test, which asserts `Some(true)`
-- a function hardcoded to `Some(true)` would pass that test too, so neither
run is evidence the second signal does anything.

`src/sys.rs`'s own doc comment on `elevated()` and `verdict` states this
precisely, measured against reasoned; nothing above changes what it says,
only makes sure this file says it too.

### `Running::covers` could not see a single winget package, and a green test stood in front of it

Phase 4's own carried-forward item, cleared here, and the numbers are worth
keeping because the guard is what stops dotpkg replacing a running application.

**Measured against a14's live process list**, before the fix:

```
source-backed installed winget ids           36
  caught by the old guard (key or dirs)       0
    - by key()  = the whole dotted id         0
    - by dirs   = under the scoop root        0
  would be caught by the id's last segment    4
  would be caught by the display Name column  2
```

Zero of thirty-six. `Brave.Brave` was running at the time and was missed. Both
of the two signals that do work are now filled in, lowercased. Over-matching is
correct here and the type says so: a false positive costs one `!` line the user
clears by closing an app; a false negative costs the app.

Three documents had all said the guard "falls back to its name and directory
halves for winget". The directory half **cannot fire at all**: `Running.dirs` is
populated by exactly one function, which only ever inserts a path segment found
under `$SCOOP/apps/` or `$SCOOP/persist/`. Measured: `Running.dirs` contained
exactly one entry on a14, a scoop app.

And the test that proved a running winget package was skipped built
`Running::new(BTreeSet::from(["brave.brave"]))`. **No machine produces a process
named `brave.brave`** — Brave's is `brave.exe`. The fixture encoded a false
premise about the world, so it was green everywhere, forever. This is the
strongest member of `docs/phase4-notes.md`'s "test that cannot fail" class found
so far: worse than a vacuous assertion, because the assertion was real.

**Measured caveat, so nobody reads more into the numbers than is there:** `xh`'s
install announced **two** aliases, `xh` and `xhs`, and `xhs` is neither the id,
the display name, nor the last segment of either — so even now a package's
*second* alias is invisible to the guard. `winget list` does not expose aliases
at all; they appear only in `install`'s stdout, at install time.

The mid-run re-sampler needed a **different** fix from the plan-time guard, and
that distinction is the part most likely to be lost: a `Step` carries only a
`Name`, so the sampler used the weaker two-signal `covers_name`, which has no
`bins` half at all and would have stayed 0-of-36 after the plan-time fix. Steps
now carry their guard names and the sampler uses the full three-signal check.
Fixing only the scanner would have closed the plan-time hole and left the
during-the-run hole exactly as wide — and during-the-run is the case the sampler
exists for.

## What was measured versus what was only reasoned

**Measured on a real Windows machine (a14), 27 write-verb invocations plus 16
verify captures, every byte checked in under `tests/fixtures/winget/`:**

- `--version` is a target on a fresh install (installed exactly 0.24.1, not the
  newest 0.26.2, hash verified).
- On an installed package, `install` silently becomes an upgrade and `--version`
  degrades to a floor. Confirmed against real EXE-installer packages
  (`Obsidian.Obsidian`, `Brave.Brave`), so it is not portable-specific.
- `0x8A15002B` covers a success and a failure at once.
- `install` succeeds elevated; `uninstall` of that same user-scope package is
  refused, with a paired de-elevated control that succeeds.
- `0x8A150014` does not distinguish "not in the index" from "not installed".
- `uninstall --version` is a guard: it resolves against what is *installed* and
  refuses rather than removing a different version.
- `--exact` case-sensitivity governs the write verbs, and the package-level
  failure outranks the version-level one.
- `--id` never fuzzy-matches, with or without `--exact`. Probed
  `7zip`/`Microsoft`/`ripgrep`/`git`/`zoxide`, each a real substring of a real
  installed id, all without `--exact`: every one came back "no package found".
- **stderr was 0 bytes across all 27 write-verb invocations**, every failure
  included.
- `--scope user` versus `--scope machine` discriminates in **both** directions
  (19 ids user-scope, `Microsoft.VisualStudio.2022.BuildTools` the exact
  reverse). This was made the *first* task of the plan, before the refusal that
  depends on it was built, because the prior round had one data point on the read
  side and none on a user-scope package.
- `winget source update --name winget` is inert: exits 0 and changes nothing on
  the machine it was run against twice — 141 rows before and after, identical
  `(Name, Id, Version, Source)` multiset, zero `Available`-column moves. That
  measurement is what makes it safe to call unconditionally, unlike the **bare**
  `winget source update`, which installed winget's own `winget-font` source MSIX
  and is never used by this crate.
- Cost: a `winget` invocation is ~1 s. A `--prepare` pays one `show` per winget
  action (two when the pin has fallen out of the index), and every step pays one
  rescan. A 17-package winget apply pays ~17 s of verification.
- Version retention is a **publisher policy, not a winget guarantee** — measured
  from 8 versions (`BurntSushi.ripgrep.MSVC`) to 828
  (`JanDeDobbeleer.OhMyPosh`). This is what makes a `recover.cmd` winget line a
  weaker promise than a scoop one, and the file says so.

**Structural, provable by reading, not by running a machine** — and labelled as
such because a review caught this being over-claimed as "measured":

- `touched: false` on the two paths where the *mutator itself* could not be
  spawned. `RealWingetMutator::run` errors only when `Command::output` fails, so
  nothing ran. That is the shape of the code, not a measurement.
- A winget step cannot reach scoop's `Mutator`: `Step`/`ScoopStep`/`WingetStep`
  make it a compile error rather than a test somebody has to remember to write.
- On a machine with **no** `winget.exe`, a declared-and-locked winget package
  still plans as an `Install` (an empty scan and an absent binary are spelled the
  same way in `installed`). It never becomes a mutation, and the reason is a
  mechanism rather than a sentence: the liveness check runs `winget show` first,
  gets `CmdError::NotFound`, and the package becomes `Outcome::Failed`, which
  fails the whole preparation.

**Reasoned only, and still open** — see "Still open" below. Chief among them:
nothing about dependency handling, and nothing about any installer type other
than `portable` on the *success* paths.

## Method findings from the measurement round

Three of these cost real time and all three generalise beyond winget.

1. **A PowerShell helper function named `Winget` shadows `winget.exe`, and the
   round looked like it had succeeded.** PowerShell function names are
   case-insensitive, so `& winget @argv` resolved to the *function* and recursed.
   Every `exit=` field came back blank and `winget list` came back as 369 bytes
   of a formatted PowerShell object. **It was caught by reading the byte count,
   not by the script.** This is `docs/phase4-notes.md`'s `ok. 0 passed` class
   reproduced inside a measurement script — a run that produces output shaped
   like a result and contains none. Two defences added afterwards, both of which
   belong in any future probe script: resolve the executable to an absolute path
   via `Get-Command -CommandType Application` up front, and **refuse to
   continue** unless the first `winget list` exits 0 and exceeds 10 KB.
2. **`runas /trustlevel:0x20000 "powershell -File <script>"` de-elevates from
   a14's ssh session; `schtasks /create … /RL LIMITED` does not**, with or
   without `/IT`. With `/IT` the task sits at `Status: Queued`, `Logon Mode:
   Interactive only`, `Last Result: 0`, and never runs when triggered from ssh.
   This matters because Phase 4b is the first phase where the integrity level
   decides whether a code path can execute at all, so the dogfood must run at
   medium integrity — every prior dogfood used the elevated ssh. Also: the first
   attempt **deleted the task before querying it**, throwing away the diagnosis.
   Don't.
3. **A byte-count comparison across capture methods is not valid, and one was
   nearly recorded as a finding.** The same `winget list` is **30744 bytes**
   captured via `& winget … | Out-String` + `WriteAllText` and **30738 bytes**
   captured via `Start-Process -RedirectStandardOutput`. Row, id, `installed` and
   `opaque` counts are identical under both — so the structural drift recorded
   alongside it is real, and the 6-byte difference is an artifact of the capture.
   Compare counts, not sizes, across methods.

**And a fourth, about the measurer rather than the tool:** one claim in the
round's own working notes was falsified by its own §7. Before measuring, the
rule that every winget call drops `--exact` had been read as a hazard for
`uninstall` — a fuzzy match removing the wrong package. `--id` never
fuzzy-matches at all. Fifth round in a row in this project where a measurement
overturned an assumption, and **the first where the assumption was the
measurer's own**.

## Corrections to earlier documents and to this phase's own controller

Recorded here rather than edited in place, matching the precedent set by the 2a,
2b-2, Phase 3 and Phase 4 designs.

- **`PROVENANCE.md`'s "numerically identical" has expired.** The fixtures say
  141 rows / 126 ids / 37 installed / 89 opaque; a14 on 2026-08-10 said 140 /
  125 / 36 / 89. `wez.wezterm` was uninstalled, `tailscale.tailscale` moved
  `1.98.2` → `1.102.2`, and winget's own source MSIX row rotated. **A dogfood
  that reuses 141/126/84/42/57/15 as expected values will go red for the wrong
  reason.** Re-derive the machine's numbers; do not reuse the fixtures'.
- **scoop's "Falsified: `depends`" does not transfer to winget.** Phase 2b
  measured 0 of 30 installed scoop manifests and 0 of 25 bucket-HEAD manifests
  declaring any dependency, and filed the concern as falsified. Live for winget:
  **5 of 12 candidate packages surveyed declare
  `Microsoft.VCRedist.2015+.x64`.** Unmeasured, not benign.
- **`Running::covers` is not "weaker" for winget. It was empty.** Three
  documents said "falls back to its name and directory halves"; the directory
  half cannot fire. Covered in full above.
- **`install-no-upgrade-available.txt` was named after the wrong file** and is
  now `install-no-upgrade-flag.txt`. It holds `--no-upgrade`'s `0x8A150061`
  result, while `install-already-installed-no-upgrade.txt` holds "No available
  upgrade found." / `0x8A15002B`. The two names described each other's contents,
  and no test referenced the first — so a later test reaching for it by name for
  the `NO_AVAILABLE_UPGRADE` case would have paired the wrong bytes with the
  wrong code, silently. Bytes unchanged.
- **`docs/measurements-2026-08-10-winget-write-path.md` §3's "three of them
  unknown to this crate" has expired.** Its table still marks `0x8A15002B`,
  `0x8A150061` and `0x8A15007D` **absent** from `src/backend/winget.rs` under
  a present-tense column header. All three are now named constants --
  `NO_AVAILABLE_UPGRADE`, `ALREADY_INSTALLED`, `CANNOT_UNINSTALL_ELEVATED` --
  in `src/backend/winget_exec.rs:56/67/79`, added by the executor this
  measurement document is cited as the authority for. Post-merge audit M4.
- **`docs/specs/2026-08-10-phase4b-winget-executor-design.md:270-273` still
  specifies a single-signal `TOKEN_ELEVATION` implementation for
  `sys::elevated()`.** It is now two signals; see "`sys::elevated()` reads
  two signals, not one" above for what changed and why. Post-merge audit I3.
- **Corrections made to this phase's own controller, by implementers and
  reviewers**, kept because a controller that is never wrong on the record is a
  controller nobody checks:
  - Three brief-stated expected test counts were stale, every time in the same
    direction (too low), because the brief was written against an older tree.
  - One brief specified a test in a location where the property **could not be
    observed at all**: `tests/cli.rs` strips `winget` off the `PATH` it hands the
    spawned process, and an absent binary makes `Winget::scan` return an empty
    `Scan` rather than an error, so the ghost could never be seen dropped. The
    implementer refused to work around it and was right.
  - One brief specified a text-level round-trip guard as "exactly one line was
    added", which is **false of the function's real behaviour** (creating a
    section adds three lines; appending to a single-line array adds none). 16
    tests failed and the guard's invariant, not the code, was what changed.
  - One controller amendment cited the wrong document for a quotation; the
    implementer cited the real source instead of copying the wrong reference.
  - One brief told an agent to check line-wrapped prose with a **line-based
    grep**, whose prediction could therefore never fire, and nobody noticed the
    prediction had failed.

**A plan improvement found mid-execution, now mandatory:** `cargo check --target
aarch64-pc-windows-msvc --all-targets` type-checks every `#[cfg(windows)]` path
**from macOS**, and works on this tree today (`check` does not link, so no MSVC
toolchain is needed). The plan was written believing such code could only be
verified on the real machine; that was wrong. It is explicitly **not** a
replacement for running the suite on Windows — it catches compile errors on the
Windows target, not behavioural differences, and Phase 4's `resolve_root` defect
compiled fine on both platforms.

## Verification

### macOS suite

`cargo test --no-fail-fast`, on the tree that ships: **588 passed, 0 failed, 0
ignored**, across **14** `test result:` lines. `cargo fmt --check` clean;
`cargo clippy --all-targets -- -D warnings` clean; `cargo build --all-targets`
zero warnings. (Was 585 at merge, `834589e`; the post-merge audit's
remediation added three tests -- one each to `src/apply.rs`, `src/render.rs`
and `src/main.rs` -- pinning the fix for its I2. The 566 this section
previously reported was already stale by five commits before that
remediation even started; see the post-merge audit for the full account.)

Windows collects one more: `on_a_real_elevated_windows_session_the_pre_check_
refuses_a_user_scope_removal` is `#[cfg(windows)]` and `#[ignore]`d, so it is
not even a test item off Windows. It asserts `sys::elevated() == Some(true)`
first, so it cannot pass vacuously, and it is the dogfood's check on the one
runtime behaviour no macOS run can reach:
`cargo test --test cli -- --ignored on_a_real_elevated_windows_session`.

### Windows target, cross-checked from macOS

`cargo check --target aarch64-pc-windows-msvc --all-targets` and
`cargo clippy --target aarch64-pc-windows-msvc --all-targets -- -D warnings`:
both clean on the tree that ships.

### Fixture integrity

Verified **before** trusting any Windows run, because a fixture whose bytes have
been normalised by a checkout makes every downstream assertion meaningless:
`tests/fixtures/winget/list-full.txt` is 30958 bytes with 143 CRLF pairs,
exactly the expected values. `.gitattributes` pins these paths `-text`.

### The Windows suite and the dogfood: done, not still outstanding

This section used to say these were still ahead. They happened before merge,
as the plan's own standing rule requires; recorded here because the version
of this file that shipped did not carry any of it.

Three real Windows runs on a14, sequenced **after** the whole-branch review
rather than before it, deliberately: the tree changed twice after the first
run, and absorbing the review's fixes first means later runs happen on a tree
that already ships.

- **Run #1**, tree `e1bb95e`: 564 passed, 0 failed, 1 ignored.
- **Run #2**, on the tree the whole-branch review shipped (`24ba0d6`): 568
  passed, 0 failed, 1 ignored, against a macOS count of 570.
- **Run #3, final**, on the tree the branch actually merged at (`cc7452f`):
  **583 passed, 0 failed, 1 ignored**, against a macOS count of **585**.

Every run was cross-referenced **name by name**, never by subtracting totals
-- the discipline that caught Phase 4's own `resolve_root` defect. The
difference set was identical across all three: two `#[cfg(unix)]` tests
absent on Windows, and the one `#[cfg(windows)]` `#[ignore]`d elevated-only
test absent on macOS, invoked by name and passing every time. Zero
discrepancies beyond those three predicted `cfg` exclusions, on any run.
(This file's own count has since grown to 588 -- see "macOS suite" above --
from three tests the post-merge audit's remediation added after Run #3;
none is `#[cfg(windows)]`-gated, but none has been run on Windows either, so
the 583/585 pairing above is Run #3's tree, not this one.)

A dogfood on a14 completed in three stages, machine restored byte-identical
afterwards (`winget list`'s hash matched before and after): a fresh install
and a wrong-case upgrade that proved the canonical-id fix live; a genuinely
elevated removal attempt refused exactly as designed; and an opaque owned
package surviving reconciliation, followed by a real removal in cleanup. Its
own attempt to *also* cover a restricted, de-elevated token failed on a
script bug and never actually de-elevated -- the one gap a later, separate
dogfood closed, in one direction only. See "`sys::elevated()` reads two
signals, not one" above.

`cargo mutants` ran twice. First scoped to this branch's diff -- 253 mutants,
`-j 2`, 11 minutes, **0 `TIMEOUT`** -- which is the clean, idle-machine
re-measurement Phase 4's own notes said its 69 unresolved timeouts still
needed, settling that question by demonstration rather than inference (Phase
4 could only reason "very likely starvation"; this measured zero). Then
again, scoped by *file* rather than by diff -- a superset, redone clean after
an implementer's own verification run was found contaminated by a concurrent
edit: **419 mutants, 347 caught, 19 missed, 53 unviable** over the touched
files, after two independently hand-verified fix rounds closed twelve of the
thirteen real gaps the first pass found (`src/apply.rs` alone is now 105
mutants, 0 missed). See "Genuinely still open" below for what the
file-scoped pass's own 19 missed mutants leave.

### Genuinely still open

- **An ordinary, non-elevated Windows session -- no `runas` at all -- has
  never been measured.** `sys::elevated()` should answer `Some(false)` from
  `TokenIsElevated` alone and never consult `CheckTokenMembership`, but
  nobody has watched it do so. See "`sys::elevated()` reads two signals, not
  one" above.
- **Three `#[cfg(windows)]` mutants in `sys.rs`** -- two in `elevated()`'s
  `Some(true)`/`Some(false)` returns, one in a `!=`/`==` inside it -- **are
  inert on macOS and unresolved.** Not test gaps: that function's body is not
  even compiled off Windows, so no macOS mutation run can exercise them at
  all. A platform gap, only resolvable by a mutation run *on* Windows.
- **`main.rs:773`'s mutant (`delete !` on the `outstanding_skips` check) is
  accepted as an equivalent mutant, not closed.** It survives because
  whenever `outstanding_skips` is non-empty, those skips are already pushed
  as `Held`, which forces `code == 1` before `floor_exit_code` ever runs,
  regardless of the deleted `!`. Closing it needs a fake scoop binary (a
  standing test policy forbids one) or a production change -- out of scope
  for a test task.
- **14 mutants in `src/backend/winget.rs`, in Phase 4 code, not this
  branch's diff** -- `floor_char_boundary` (6), `parse_list` (6),
  `parse_versions` (1), `RealWinget::run` (1) -- surfaced only because the
  file-scoped mutation pass covered the whole file rather than the diff.
  Not this phase's scope to close; the next phase inherits a named list
  instead of a surprise.
- **Two mutants in `winget_exec.rs`, inside `RealWingetMutator::run`** -- a
  `NotFound == -> !=` and an `unwrap_or(-1) -> unwrap_or(1)` -- are the one
  seam every test in this crate replaces with a fake. Covering them means
  spawning a real `winget.exe` from the test suite, which this project does
  not do.

## Deferred minors, by originating task

Closed before merge, at the whole-branch review's triage: the canonical-id
write-argv defect (Critical) and its sibling byte-comparison in the scope query;
`reconcile_ghosts`' `opaque` blindness; four doc comments dating this branch's
own new behaviour to Phase 4 and crediting a fix for a defect that never existed;
the routing-bug-arm mechanism claim; `WingetState`'s no-hash over-claim; the
prompt's false uninstall-then-install sentence; the announced winget downgrade;
the misnamed `--no-upgrade` fixture; `scratch/` dirtying `git status`;
`plan.rs`'s colliding task number; `main.rs`'s "from all three" over two scans;
and this file plus the README.

**Left open, each with the reason it can stay:**

- **`winget.rs:206`'s `id.rsplit('.').next().unwrap_or(id)`** — `rsplit` never
  yields an empty iterator, so the fallback is unreachable. A harmless
  defensive no-op.
- **`plan_backend`'s "installed is empty for this backend by construction"
  comment** is true of the only production caller, but `plan()` is `pub` and
  pure. A `debug_assert!` would make it checkable. The violation direction is
  safe (lost report lines, never a fabricated action).
- **The `Unscannable` sentence literal is duplicated** in `apply.rs` and
  `render.rs`, which is the convention `Opaque` already set. The two copies can
  drift.
- **A second `list -e --id` argv exists**, hand-built in `winget.rs` with
  `--scope` inserted rather than sharing `list_one_argv`. Both are argv-pinned by
  tests, but they can drift apart.
- **Four generic winget failure messages offer no next action**, unlike the four
  that name `dotpkg update` or "re-run without elevation".
- **A measured gap in the downgrade translation**: a machine ahead of its pin
  *and* a pin that has fallen out of the index yields `0x8A150017`, which lands
  in the generic arm — so the operator gets neither "will not downgrade" nor
  `dotpkg update`, at exactly the moment the advice would help most. Widening the
  arm to that code is the **wrong** fix and that is why the gap is documented
  rather than closed: the code proves only "this version is not available", which
  is equally what a machine *behind* an unavailable pin gets, so keying on it
  would print "dotpkg will not downgrade" for runs that were never downgrades.
- **`line_comment`'s `"`-escape limit**: a comment whose own text contains a
  quote can be dropped from the multiset in *both* the before and after
  snapshots, so its loss is invisible to the round-trip guard. Worse than the
  disclosed "moved, not lost" weakness, and now disclosed too. `line_comment`
  also assumes package names and bucket URLs never contain a literal quote,
  which neither `Name::new` nor `parse_buckets` enforces.
- **The text-level round-trip guard cannot tell "moved" from "still attached to
  the right line"** — only "present" from "absent". Accepted deliberately: loss
  is the measured defect class, and position is not checkable without
  re-introducing the false line-count invariant.
- **`recover.cmd`'s winget REM line is one ~284-character line** where the scoop
  half wraps over two. Valid batch, less consistent.
- **The recovery-line test can catch a flag *added* to `set_argv` but not one
  *dropped*** — the golden list is computed from a live `set_argv` call inside
  the test. Coverage is complete across the suite (`winget_exec.rs` pins the
  whole vector with `assert_eq!`), not within that one test.
- **`Plan::Unreachable`'s panic text** says "this test declared no winget
  packages", which is untrue of one test that does. Pre-existing.
- **`{backend:<6}` is sized for `scoop`/`winget` exactly.** A third backend with
  a longer name shifts every column in both tables.
- **One adopted-package test pairs a fixture reading `Version 0.26.1` with a
  rescan reporting `0.24.1`** — harmless, because the fake ignores stdout on that
  path, but it describes a run winget could not produce.
- **A stale `:78` line citation** for "winget pins a version, not a hash",
  inherited verbatim from two earlier documents that both say `:78` where the
  sentence is at `:76`.
- **`Capability` is a one-variant enum matched one-armed at four points.** The
  four matches exist so a third, report-only backend re-earns them as four
  compile errors at the four points where a human must decide what such a backend
  does. A reviewer's fair nit: a one-armed match on a one-variant enum is a
  weaker version of the exhaustive-match idiom this crate uses elsewhere, not the
  same thing.
- **`main.rs`'s single `gate_the_run(...)` call has no red-able test.** Deleting
  the match, or substituting a constant for `sys::elevated()`, leaves the suite
  green. Down from four unpinned lines plus an unpinned ordering to one call, by
  hoisting the three guards into one tested function; the `#[ignore]`d Windows
  test is the dogfood's check on what remains.

## Still open

Every "deliberately not measured" item from the design's Non-goals, restated as
still open, plus what this phase added.

1. **Downgrading a winget package.** *Decided, not deferred* — the one entry
   here that is a closed question rather than a gap. Measured: `install
   --version <older>` cannot do it, and the alternative (uninstall then install)
   depends on the one step measured to be fragile and would reintroduce a nightly
   uninstall-and-reinstall loop on every self-updating application.
2. **Dependency handling.** winget manifests declare dependencies — 5 of 12
   surveyed declare `Microsoft.VCRedist.2015+.x64` — and dotpkg has no vocabulary
   for a package it did not declare appearing after an install. It will be
   reported as `Unmanaged` on the very next `status`. Unmeasured. A real gap.
3. **`--location`, `--all-versions`, and side-by-side versions of one id.** All
   three unmeasured: the first confounded by the upgrade-reinterpretation, the
   second blocked by the elevation refusal, the third never constructible because
   every `install --version` either upgraded or did nothing. Side-by-side
   versions are also one of the ways an owned package lands in `opaque`, which is
   the shape the ghost-reconciliation fix above exists for.
4. **Removing a machine-scope package while elevated.** Unmeasured; the refusal
   is narrowed to user-scope rather than guessing.
5. **Any installer type other than `portable`, for the success paths.** The
   upgrade-reinterpretation was confirmed against real EXE-installer packages,
   but a successful upgrade or uninstall of an MSI/EXE package is unmeasured, and
   `--silent`'s behaviour for an installer with a GUI is unmeasured.
6. **`--force` and `--purge` against the elevation refusal.** Unmeasured; the
   de-elevated route succeeded first and the round stopped there.
7. **`winget pin`.** Unchanged: two sources of truth about permitted versions is
   how a tool starts lying. `pkg.lock` is dotpkg's answer.
8. **`add`**, architecture drift, same-version re-pin, locking against two
   concurrent dotpkg runs, and Chocolatey. All unchanged.
9. **A package's second alias is invisible to the running-process guard.** `xh`
   announced `xh` and `xhs`; `xhs` is neither the id, the display name, nor the
   last segment of either. `winget list` does not expose aliases at all.
10. **There is no independent oracle for a winget mutation.** Structural, and
    the most important open item on this list, because it cannot be closed by
    adding a test — only by finding something on disk to read back. See "Read
    this first".
11. **No retry policy for a transient winget failure.** `version_liveness`
    returns `Err` for *any* nonzero exit code, so a momentarily unhappy winget —
    a locked index, a source mid-update — fails the whole run, scoop included.
    Fail-closed on purpose (`--keep-going` is the documented way to let the ready
    packages through), but it is a new failure mode this phase introduced and a
    retry policy is the obvious refinement.
12. **The `--prepare` loop is unmeasured as a loop.** Nothing is parallelised or
    cached, and the per-call ~1 s figure is the only number there is.
13. **Every Phase 4 "still open" item not in this phase's scope** stays open,
    notably: `plan_backend`'s unconditional `Arch::as_scoop()`; the design's "a
    new backend slots in without touching the planner" promise being half true;
    `verify.rs:146`'s `NotFound`-idiom guard; `floor_char_boundary` and the
    missing-`Version` refusal branch being untested defensive code; no fixture
    pairing a plain `show` with `show --versions` for the same package; and
    `resolve_installed`'s `fell_back_to_tip` warning path being untested.
14. ~~The 69 unresolved `timeout` mutants from Phase 4's final mutation run~~
    **-- settled, not still open.** This phase's own mutation run (253
    mutants, `-j 2`, 11 minutes) recorded **0 `TIMEOUT`**, which is the
    clean, idle-machine re-run this item asked for, by demonstration rather
    than inference: Phase 4 could only reason "very likely starvation"; this
    phase measured zero timeouts under the same resource-limited settings
    and that is the re-measurement. Left numbered rather than deleted, so a
    reader following an old reference here finds the resolution, not a gap
    in the numbering.
15. **`sys::elevated()`'s runtime behaviour is now measured in one direction,
    not both.** The dogfood found a real defect in the direction this item
    flagged as open (a restricted, DENY_ONLY token was refused when winget
    itself would have allowed the removal) and the code changed in response
    -- `verdict` now requires two signals, not one. What remains open is
    narrower than this item originally was: an ordinary, non-elevated
    session with no `runas` at all is still unmeasured. See
    "`sys::elevated()` reads two signals, not one" and "Genuinely still
    open", both above.
