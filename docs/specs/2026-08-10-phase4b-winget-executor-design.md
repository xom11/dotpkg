# dotpkg Phase 4b — the winget executor

**Status:** design proposed 2026-08-10, not yet implemented.
**Depends on:** Phase 4, with `main` at `98f3d33`.
**Gated on:** `docs/measurements-2026-08-10-winget-write-path.md`, which is the
round `docs/specs/2026-08-09-phase4-backend-winget-design.md` named as the
precondition for this phase and which
`docs/measurements-2026-08-09-winget.md` closes by demanding.

Phase 4 stopped at scan / plan / lock / report with `Capability::ReportsOnly`,
for a reason it stated plainly: winget has no `$env:SCOOP` equivalent, so there
was no throwaway root, so the write path had never been measured, and *"an
executor written against an unmeasured mutation surface would repeat the mistake
Phase 2b-2 exists to record."* That round has now been run — on a purpose-picked
disposable package, with the machine proven byte-identical before and after — and
it overturns enough that this document is mostly about what the measurement
changed.

## Scope

**Phase 4b makes `apply` install, upgrade and remove winget packages. It does
not downgrade them.**

The `add` command stays out, unchanged from Phase 4's recommendation: no third
resolver, `add` is `pkg.toml` plus `update <pkg>` plus `apply`, composed.

## Corrections to earlier documents

Recorded here rather than edited in place, matching the precedent set by the 2a,
2b-2, Phase 3 and Phase 4 designs.

### `Running::covers` is not "weaker" for winget. It is empty.

Three documents state the same thing and all three are wrong in the same
direction:

- `docs/phase4-notes.md`, "Read this first" — *"of `Running::covers`'s three
  signals (package directory, process name, declared executables), only the
  first two can ever fire for a winget package."*
- `src/backend/winget.rs:244-249` — *"with `bins` empty only the first two can
  ever fire."*
- `docs/specs/2026-08-09-phase4-backend-winget-design.md:336` — *"`bins` stays
  empty and `Running::covers` therefore falls back to its name and directory
  halves for winget."*

The package-directory half cannot fire. `Running.dirs` is populated by exactly
one function, `Scoop::running_apps` (`src/backend/scoop.rs:186-209`), which only
ever inserts a path segment found under `$SCOOP/apps/` or `$SCOOP/persist/`. A
winget package's executable is never there. Measured on a14: `Running.dirs`
contained exactly one entry, `kanata` — a scoop app.

So one signal remains, `names.contains(inst.name.key())`, and `key()` is the
whole dotted id. Measured against the live process list:

```
source-backed installed winget ids           36
  caught by today's guard (key or dirs)       0
    - by key()  = the whole dotted id         0
    - by dirs   = under the scoop root        0
  would be caught by the id's last segment    4
  would be caught by the display Name column  2
```

**Zero of thirty-six.** `Brave.Brave` was running at the time and was missed.

**And a green test stands in front of it.** `tests/planner.rs:404` — the one test
proving a running winget package is skipped — builds
`Running::new(BTreeSet::from(["brave.brave"]), Default::default())`. No machine
produces a process named `brave.brave`; Brave's is `brave.exe`. This is
`docs/phase4-notes.md`'s own "test that cannot fail" class, one step worse than
the `resolve_root` case: there the assertion was vacuous on one platform, here
the **fixture encodes a false premise about the world**, so it is green
everywhere, forever.

This is the `kanata` scenario the whole project exists to avoid, and it is why
A1 below is the first task and not a nice-to-have.

### `tests/fixtures/winget/PROVENANCE.md`'s "numerically identical" has expired

| | fixture | a14, 2026-08-10 |
|---|---|---|
| rows / ids | 141 / 126 | **140 / 125** |
| `installed` / `opaque` | 37 / 89 | **36** / 89 |

`wez.wezterm` uninstalled, `tailscale.tailscale` `1.98.2` → `1.102.2`, winget's
own source MSIX row rotated. True when written, false now. A dogfood that reuses
141/126/84/42/57/15 as expected values goes red for the wrong reason.

### scoop's "Falsified: `depends`" does not transfer to winget

`docs/measurements-2026-08-08-scoop-exit-codes.md` measured 0 of 30 installed
scoop manifests and 0 of 25 bucket-HEAD manifests declaring any dependency, and
recorded *"a pinned manifest pulling a dependency at latest, over the network,
inside the mutation window"* as a **falsified** concern.

Live for winget: 5 of 12 candidate packages surveyed declare
`Microsoft.VCRedist.2015+.x64`. A `winget install` that also installs a second
package hands dotpkg an installed package with no lock entry, no `state.json`
ownership and no declaration — reported as `Unmanaged` on the very next
`status`. Unmeasured (see Non-goals), not benign.

### `Divergence::describe()`'s four sentences expire the day this lands

All four hardcode *"dotpkg cannot install or remove winget packages yet"*
(`src/plan.rs:99-113`). `docs/phase4-notes.md` files this as a
backend-name-generalisation minor; it is not. It is
`docs/phase4-notes.md`'s own pattern 2 — *a sentence that was true when written
and stopped being true* — pre-scheduled. Every one must be rewritten in the same
task that changes the capability, and the grep for justifying comments must run.

## The measurements that set the shape

Full record: [`docs/measurements-2026-08-10-winget-write-path.md`](../measurements-2026-08-10-winget-write-path.md).

1. **`--version` is a target on a fresh install** — `install --version 0.24.1`
   on an absent package installed exactly 0.24.1, not the newest 0.26.2, and
   verified the installer hash.
2. **On an installed package, `install` silently becomes `upgrade`, and
   `--version` degrades to a floor.** Asking for an older version does nothing
   and exits `0x8A15002B` *"No available upgrade found."* — a message about
   upgrades in answer to a request that was not one. Observed against real
   EXE-installer packages too (`Obsidian.Obsidian`, `Brave.Brave`), so it is not
   portable-specific.
3. **`0x8A15002B` is returned both for "already exactly where you asked"
   (success) and for "I declined" (failure).** Nonzero cannot be read as failed.
4. **`install` succeeds elevated; `uninstall` of that same package is refused**
   with `0x8A15007D`, *"The package installed for user scope cannot be
   uninstalled when running with administrator privileges."* Paired control at
   medium integrity: exit 0, removed.
5. **`0x8A150014` does not distinguish "not in the index" from "not
   installed"** — two different sentences, one exit code.
6. **`uninstall --version` is a guard**: it resolves against what is installed
   and returns `0x8A150017` rather than removing a different version.
7. **`--exact` case-sensitivity governs the write verbs too**, and the
   package-level failure outranks the version-level one.
8. **`--id` never fuzzy-matches**, with or without `--exact`.
9. **stderr was 0 bytes across all 27 write-verb invocations**, every failure
   included.

## The central rule

**winget's exit code is never the verdict. The verdict is always the re-scan.**

For scoop the exit code is worthless and the disk is truth — `verify::verdict`
compares the installed manifest's bytes against the staged file, a content
address. For winget the exit code carries real information and is still
ambiguous (measurement 3), so the same rule applies for a different reason:
after every step, ask `winget list -e --id <the canonical id from pkg.lock>` and
compare. `-e` is safe there and only there, because `pkg.lock`'s key is the
spelling winget itself echoed back in `Found <name> [<Id>]` (Task 15), and
measurement 7 is what makes that a requirement rather than a preference.

**The asymmetry this leaves must be stated, not papered over.** There is no
independent oracle for winget. scoop's check reads a file winget's equivalent
does not have; winget's check asks the same tool that just performed the
mutation. `src/execute.rs`'s own module doc names exactly this hazard — *"a fake
that both performs and reports the mutation proves only that it is
self-consistent"* — and for winget there is no way around it. Recorded as a
known structural weakness of this backend, the same way `bins` being empty was.

## Half A — the preconditions, which must land before anything acts

`docs/phase4-notes.md` names three of these as conditions rather than
suggestions. The measurement adds four more, all of the "no task owns it" shape
that phase's own pattern 3 warns about.

### A1. `Running::covers` learns to see a winget package

`rows_to_scan` already parses everything needed and throws it away. Fill
`Installed.bins` for winget from two sources, lowercased and suffix-stripped to
match what `sys::running_processes` produces:

- **the id's last dotted segment** — `Brave.Brave` → `brave`,
  `Google.Chrome` → `chrome`, `OpenAI.Codex` → `codex`. Measured to catch 4 of
  36 on a14, strictly dominating the alternative below.
- **the display `Name` column** — `Brave.Brave`'s is `Brave`. Free (already
  parsed), a genuinely different signal, and 2 of 36 on its own.

Over-matching is correct here and the type says so: *"A false positive costs one
`!` line the user clears by closing an app; a false negative costs the app."*

**`Installed.bins`'s own doc comment has to change with it**, and this is a real
semantic widening rather than a free reuse. It currently reads *"Lowercased,
extension-stripped basenames of every executable this package's manifest names"*
(`src/model.rs:190-192`). For winget there is no manifest to name executables,
and neither the id's last segment nor the display name **is** an executable name
— they are guesses that happen to match one often. The field's contract becomes
*"names a live process might plausibly report for this package"*, which is what
`Running::covers` actually uses it for and what scoop's values already satisfy.
Widening the comment is part of A1, not a follow-up: a field whose doc says
"executables the manifest names" and whose winget values are neither is exactly
the kind of sentence `docs/phase4-notes.md`'s pattern 2 is about.

Measured caveat, stated so nobody reads more into the numbers than is there:
`xh`'s install announced **two** aliases, `xh` and `xhs`, and `xhs` is neither
the id, the display name, nor the last segment of either — so even after A1 a
package's *second* alias is invisible to the guard. `winget list` does not expose
aliases at all; they appear only in `install`'s stdout, at install time.

**The mid-run re-sampler needs its own fix, and it is not the same fix.**
`execute::execute` calls `running().covers_name(&app)` (`src/execute.rs:513`),
which is deliberately the weaker two-signal form because a `Step` carries only a
`Name`. After A1 that call is *still* 0-of-36 for winget, because `covers_name`
has no `bins` half at all. So `Step` must carry the guard names alongside the
backend (A5), and the sampler must use the full three-signal check. Fixing only
`rows_to_scan` would close the plan-time hole and leave the during-the-run hole
exactly as wide — and the during-the-run one is the case the sampler exists for.

`Running`'s own doc comment gains the sentence the three documents above got
wrong: `dirs` is scoop-only by construction.

### A2. "winget could not be scanned" stops being spelled the same as "winget found nothing"

Two defects, one family, both currently safe only because winget cannot act.

`Winget::scan` treats **every** `WingetCmd::run` error as "winget is absent",
because the trait's `anyhow::Result` erases `io::ErrorKind` before `scan` sees
it. Fix at the type level, the move this project has used four times: `WingetCmd`
returns a typed error distinguishing `NotFound` from everything else, so a broken
or permission-denied `winget.exe` cannot reach the same arm as a genuinely
missing one.

`scan_or_warn`'s doc comment justifies safety only in the prune direction. In the
other direction a declared, locked, installed, converged winget package renders
as `Divergence::Install` after any empty scan. Today a wrong report line; with
`Capability::Acts` it is dotpkg installing a package that is already there. Fix
at the type level again: `plan()` receives a `ScanOutcome` per backend —
`Scanned(Scan)` or `Unscannable(reason)` — and `plan_backend` emits a skip for
every declared package of an unscannable backend rather than an `Install`.
"Empty" and "failed" become two values instead of one.

### A3. The round-trip guard gets a text-level half

`verify_round_trip`/`verify_round_trip_winget` compare the parsed `Config`, which
has no field for comments, so `pkg.toml`'s "byte-identical except the added line"
promise is unguarded and the comment-loss bug fixed in Phase 4's Task 16 was
invisible to it by construction.

Add a text-level check beside the semantic one, not instead of it: the new text's
lines must be the old text's lines with **exactly one insertion** and no other
difference. Cheap, and it catches the whole comment-shaped class rather than the
one instance that was found.

### A4. `apply` refuses a winget removal it has measured it cannot perform

From measurement 4. Before any step runs, and only when the plan contains a
winget removal:

- If the process is elevated **and** the package is user-scope, refuse, naming
  the measured reason and that re-running unelevated is the fix. Same shape and
  same reasoning as `execute::root_looks_like_scoop` — defence at the point of
  use, fail closed.
- Package scope is queryable: `list -e --id <id> --scope user` versus
  `--scope machine` was measured to discriminate — but **on exactly one package**
  (`Microsoft.VisualStudio.2022.BuildTools`, machine-scoped: `--scope machine`
  returned its row, `--scope user` returned `0x8A150014`;
  `docs/measurements-2026-08-09-winget.md` §2). One data point on the read side
  and none on a user-scope package. **Confirming `--scope` discriminates in both
  directions is the first task of the plan**, before A4's refusal depends on the
  answer — the same sequencing Phase 4's design used for
  `winget source update --name winget`.
- Whether a **machine-scope** package can be removed while elevated is
  **unmeasured**, so the refusal is scoped to user-scope packages only rather
  than to elevation alone.

**And `0x8A15007D` is translated into a named failure regardless**, because the
pre-check cannot be perfect. A pre-check plus a translation, not either alone.

Elevation detection: a `sys` seam returning `Option<bool>` — `None` meaning
"could not tell", which must **not** trigger the refusal, only the translation.
Implementation proposed as a direct `windows` dependency (`TOKEN_ELEVATION` via
`GetTokenInformation`); the crate is already in the tree transitively through
`sysinfo`. This is the one place in Half A that adds a dependency, and it is
called out here so the spec review can object.

### A5. `Step` carries its backend, so a winget step cannot reach scoop's `Mutator`

`Step` (`src/execute.rs:59-83`) names only `app`, `staged` and `arch`, and
`plan_to_steps` (`src/apply.rs:564-599`) matches on `Action::Install { name,
arch, .. }` — **ignoring `backend`**. Nothing but `stage_and_fetch`'s
`backend != SCOOP` check at the *staging* layer (`src/apply.rs:509`) keeps a
winget action out of scoop's executor today, and that is the wrong layer for the
guard.

`Step` gains the backend and the guard names from A1. A winget step routed to
scoop's `Mutator` becomes a compile error rather than a test that has to
remember to exist — the move `Resolution` carrying a `Pin`,
`is_outstanding`'s wildcard-free match, and the `rewritten` seam all already
make in this crate.

### A6. `execute` stops demanding a scoop root on a run that has no scoop steps

`root_looks_like_scoop(root)?` is the unconditional first line of `execute`
(`src/execute.rs:496`), and `main.rs` passes `d.scoop.root()`. A Windows machine
with winget and no scoop would have its entire run refused, winget steps
included. Masked today only by the "Nothing to do" early return
(`src/main.rs:378`), which no longer fires once winget produces steps.

The check becomes conditional on the step list containing a scoop step. It must
stay unconditional *for* scoop: it exists because a wrong `$SCOOP` makes every
uninstall verify as successful.

### A7. `state.reconcile` runs per backend

`main.rs:469-471` scans scoop and reconciles `SCOOP` only. Winget ghosts — an
entry whose package was removed outside dotpkg — would accumulate forever and
inflate `owned_count(WINGET)`, the number `mass_prune_guard` reads. The winget
scan already exists in the same function; the reconcile pass does not.

## Half B — the executor

### B1. `WingetMutator`, behind a seam, mirroring `Mutator`

Every mutating winget invocation goes through one trait so no test spawns
`winget.exe`, the sibling rule to `Mutator` and to the standing prohibition on
creating a file at `Scoop::scoop_exe()`'s path.

Argv, each line measured:

| operation | argv |
|---|---|
| install / upgrade | `install -e --id <canonical> --version <pin> --silent --accept-package-agreements --accept-source-agreements --disable-interactivity` |
| remove | `uninstall -e --id <canonical> --version <installed> --disable-interactivity --accept-source-agreements` |

**`winget upgrade` is deliberately not used.** Measured: it goes to the *newest*
version in the index, not to a requested one — it took the guinea pig from
0.26.1 to 0.26.2 while the pin was neither. A pinning tool cannot use a verb
whose target is "latest". `install --version <pin>` is measured to perform the
upgrade instead (0.24.1 → 0.26.1), which is exactly what a pin asks for.

**`--version` on the removal is not optional.** Measurement 6: it resolves
against what is installed and refuses with `0x8A150017` rather than removing a
different version. Passing the version dotpkg believes is installed makes the
removal fail closed.

**A winget version change is ONE call, not an uninstall-and-install pair.** This
is the sharpest divergence from the scoop executor and it must not be left
implied. `Step::Replace` exists because scoop *cannot* change a version any other
way — `install` over an installed app is a measured no-op, so scoop needs an
uninstall half and therefore a window in which the package is absent. winget's
`install --version <pin>` performs the upgrade directly (measured: 0.24.1 →
0.26.1, exit 0), so a winget version change opens **no such window**: the package
is never absent, and `run_step`'s `touched` bookkeeping has no uninstall half to
reason about.

It also means dotpkg does not need to tell the two directions apart for winget at
all. `Action::Upgrade` and `Action::Downgrade` both become the same single
`install --version <pin>` attempt; the upgrade succeeds and the downgrade comes
back `0x8A15002B` with the version unchanged, which B4 turns into the refusal.
The planner may keep emitting whichever of the two variants `is_older` picks —
its answer is still only cosmetic, because both variants route to one call.

**`--force` is not used by default.** It is the only measured way to re-assert a
version idempotently (exit 0, from cache, no re-download) and is recorded for
whoever needs that; a nightly `apply` does not.

### B2. `winget_verdict`, the analogue of `verify::verdict`

`list -e --id <canonical> --disable-interactivity`, parsed with the existing
`parse_list`, then run through **the same three opaque rules `rows_to_scan`
already applies**. That last part is the point: an id that comes back
sourceless, `> `-prefixed or version-disagreeing after a mutation is *"dotpkg
cannot confirm this"*, not *"done"*. Reusing the rules rather than writing a
second, looser check is what keeps the executor from being more credulous than
the scanner.

Returns an enum mirroring `Disagreement`, so the executor's `touched` reasoning
keeps its shape: absent when present was expected, present-at-the-wrong-version,
present when absent was expected, and unreadable.

Cost: ~1 s per step, measured. A 17-package winget apply pays ~17 s of
verification.

### B3. Exit codes: diagnostic text, never a verdict

Three new constants, alongside the two the crate already has:

```rust
pub const NO_AVAILABLE_UPGRADE: i32 = -1978335189; // 0x8A15002B
pub const ALREADY_INSTALLED:    i32 = -1978335135; // 0x8A150061, from --no-upgrade
pub const CANNOT_UNINSTALL_ELEVATED: i32 = -1978335107; // 0x8A15007D
```

Each carries a doc comment naming the measured argv it was observed under and,
for `NO_AVAILABLE_UPGRADE`, the fact that it covers a success and a failure at
once — the same warning `NO_APPLICATIONS_FOUND`'s doc comment already carries
about `list -s msstore`.

`0x8A150014` from a removal needs its own note: it means "no *installed* package
matching", which for a `Remove` step is the **desired end state**, and it is
indistinguishable by code from "that id is wrong". The re-scan is what settles
it, which is the central rule doing its job.

### B4. The downgrade refusal, delegated to winget rather than decided by dotpkg

A winget package installed **ahead** of its pin is the measured `Brave.Brave`
shape: installed `151.1.93.134`, index newest `151.1.93.132`. Today it is
`EXIT = 1` forever. With a naive executor it is worse — `plan_backend` would
emit `Action::Downgrade`, `apply --yes` would remove and reinstall the browser,
and Brave would self-update back before the next run. **A nightly
uninstall-and-reinstall loop on a running application, permanently.**

dotpkg does not decide the direction. It fires `install --version <pin>`, and
**winget's own measured refusal is the gate**: `0x8A15002B` with the version
unchanged in the re-scan means the machine is ahead of the pin. That is
translated into a named refusal whose advice is actionable — *"installed
`<x>`, pinned `<y>`; dotpkg will not downgrade a winget package. Run `dotpkg
update` to move the pin forward."*

**This is why `is_older` stays cosmetic.** Its own doc comment warns that
whoever gates on it *"is promoting this function from cosmetic to load-bearing
and owes it a real version comparison — pre-release ordering, non-numeric
suffixes, and the `pa.is_empty()` string fallback all become answers a user can
be hurt by."* winget versions include `v0.2026.07.15.08.55.stable_01` and
`26.01.00.0` against `26.02`. Delegating the direction to winget avoids that
debt entirely, and it is more honest: dotpkg finds the answer out rather than
guessing it.

Cost, stated plainly: one wasted ~1–3 s winget call per ahead-of-pin package per
run. Measured to change nothing on disk. The running-process guard (A1) fires
first, so a running app is skipped before this call is made at all.

Plan-time display needs no change: `Divergence::Change { from, to }` already
carries one shape for both directions and prints no arrow, a Phase 4 decision
that turns out to have been the right one for a reason Phase 4 did not have.

### B5. `prepare` gains a winget arm that cannot carry a manifest path

`stage_and_fetch` returns `Outcome::Failed` for any `backend != SCOOP`
(`src/apply.rs:509`), so with `Capability::Acts` every winget install would fail
preparation. But winget has nothing to stage — there is no local manifest, which
is the whole content of Phase 4's own correction about *who holds the manifest*.

What `--prepare` **can** do for winget is a liveness check: `show -e --id
<canonical> -v <pin>`, whose `0x8A150017` says the pinned version has fallen out
of the index. That code path already exists in `Winget::resolve_installed`.

`Outcome` gains a variant for "ready, nothing to fetch, liveness confirmed",
distinct from `ReadyToFetch { manifest }` and from `ReadyToRemove` — for exactly
the reason those two were split from one another in Phase 2b-2: as one variant,
"no manifest" would mean "this is a winget action" only for values `prepare`
itself produced.

### B6. `recover.cmd` gains winget lines, and says what they are worth

Built from `WingetMutator`'s own argv builder, never typed out a second time —
the measured reason `write_recovery` already builds its scoop lines from
`install_argv`: hand-duplicating it left a mutation that deleted `-u` from only
the recovery line green across the whole suite.

And the file gains one honest sentence. A scoop recovery line names a manifest
dotpkg staged and hash-verified on local disk. **A winget recovery line is a
request re-resolved against an index dotpkg does not hold**, and if that version
has fallen out of the index — measured retention spans 8 to 828 versions,
publisher policy, not a winget guarantee — the line fails. Two different
promises in one file, and the file must not imply they are the same one.

## What the user sees

```
$ dotpkg apply
  + winget BurntSushi.ripgrep.MSVC  15.2.0                    (install)
  ~ winget Obsidian.Obsidian        1.13.4 -> 1.14.0          (upgrade)
  - winget Vivaldi.Vivaldi          8.1.4087.62               (remove)
  ! winget Brave.Brave              151.1.93.134 -> 151.1.93.132

  3 changes, 1 skipped. Continue? [y/N] y

  + winget BurntSushi.ripgrep.MSVC  installed 15.2.0, confirmed by rescan
  ~ winget Obsidian.Obsidian        1.14.0, confirmed by rescan
  ! winget Brave.Brave              installed 151.1.93.134, pinned 151.1.93.132 --
                                    dotpkg will not downgrade a winget package.
                                    Run `dotpkg update` to move the pin forward.
  - winget Vivaldi.Vivaldi          refused: this package is installed for user
                                    scope and dotpkg is running elevated, which
                                    winget will not let it uninstall. Re-run
                                    without elevation.
```

The last two lines are the whole product of this measurement round. Neither
could have been written before it, and the first of them is a run that would
otherwise have uninstalled and reinstalled a browser every night.

## Testing

Layers unchanged: everything runs on macOS and Linux, behind the seam.

**The fixtures are the captured bytes from W1 and W2** — the real stdout and the
real exit codes for every shape above, including all three new constants,
including the elevation refusal, including
`Found an existing package already installed. Trying to upgrade the installed
package...`. A fixture invented by hand would be self-consistent with a winget
nobody ran.

Coverage this plan requires by name, following `docs/phase3-notes.md`'s third
pattern — *ask what each module produces that something downstream consumes, and
require that thing to be asserted*:

| Producer | Consumer | Must be asserted |
|---|---|---|
| `rows_to_scan` | `Running::covers` | a winget `Installed`'s `bins` contains the id's last segment **and** the folded display name |
| `Step` | `execute` | a winget step cannot be constructed against scoop's `Mutator` (compile-time) |
| the re-sampler | a running app | a winget package whose process is named after the id's **last segment** is held mid-run — the case `covers_name` misses |
| `winget_verdict` | `run_step` | an id that comes back `opaque` after a mutation is "cannot confirm", **not** `Done` |
| `0x8A15002B` + unchanged rescan | the user | becomes the named downgrade refusal, never `Done` and never a bare failure |
| `0x8A15002B` + rescan **at the pin** | the user | becomes `Done` — the converged case sharing the code |
| `0x8A150014` from `uninstall` | `run_step` | "already absent" is `Done` for a `Remove`, decided by the rescan, not by the code |
| A4's pre-check | the machine | an elevated run with a user-scope winget removal **refuses**, and writes nothing |
| `ScanOutcome::Unscannable` | `plan()` | a declared+locked winget package does **not** become `Install` |
| `write_recovery` | a human | a winget line is present, built from the mutator's own argv, and the file says a winget line is a request |

**Negative controls.** `docs/phase3-notes.md` records that every control aimed at
an external tool's behaviour failed and every one aimed at this crate's own logic
held. So: controls assert against the checked-in W1/W2 fixtures, never against
reasoning about what winget would do. Every refusal assertion is paired with a
count of winget invocations (which must be zero) or with a positive sibling that
must stay green. No control consumes a `Result` with `unwrap_err()` before its
other assertions run.

**And the rule that outranks this brief:** if a negative control cannot be made
to go red, that is a failure of this plan, not of the implementer. Fix the test,
say so in the notes, and do not ask first.

**Standing rules kept:** `--no-fail-fast` on every run; the suite runs on Windows
before the dogfood **and again on the tree that ships**; every filtered `cargo
test` states its expected test count, because four filters in Phase 4 selected
zero tests and printed `ok. 0 passed`; a whole-branch review plus `cargo mutants`
before merge; an independent audit after merge. The unresolved `timeout` column
from Phase 4's final mutation run (69 of 71) is re-run as its own set with
`--timeout 600` on an idle machine, per `docs/phase4-notes.md` item 9.

## Dogfood

**Run at medium integrity.** Every prior dogfood used the elevated ssh; this is
the first phase where that choice decides whether a code path can execute at
all. `runas /trustlevel:0x20000` is the measured working route
(`docs/measurements-2026-08-10-winget-write-path.md` §5); `schtasks /RL LIMITED`
is not, with or without `/IT`.

**Re-derive the machine's numbers; do not reuse the fixtures'.** They have
already drifted once.

Framed so it can fail:

1. Does a real `apply` install a declared winget package, and does the rescan
   confirm the pinned version rather than the newest?
2. Does an ahead-of-pin package produce the named downgrade refusal, exit 1, and
   **no change on disk**? `Brave.Brave` supplies this shape today.
3. Does a running winget application get held — mid-run, by the re-sampler, not
   only at plan time? Constructible: start the guinea pig's own process during
   the run.
4. Both halves of A4, run deliberately at both integrity levels. **Elevated:**
   does the pre-check refuse a user-scope removal before anything runs, and write
   nothing? **Medium integrity:** does the same removal succeed? A pass on only
   the first half is a refusal that might be refusing everything.
5. Is `pkg.toml` byte-identical after every command, comments included?
6. Does `recover.cmd` contain a winget line that actually reinstalls what the run
   removed?
7. Does `state.json` end with no winget ghosts, after a package is removed
   outside dotpkg between two runs?

`kanata` is never started or stopped. `C:\Users\kln\dotpkg-build` and
`C:\Users\kln\pkg.toml` are reused. The guinea pig is a purpose-picked
disposable package, not one of the 36 real installed ids.

## Non-goals

- **Downgrading a winget package.** Decided, not deferred: measured, `install
  --version <older>` cannot do it, and the alternative — uninstall then install —
  depends on the one step measured to be fragile (A4) and would reintroduce a
  nightly loop on every self-updating application.
- **`add`.** Unchanged from Phase 4: `pkg.toml` plus `update <pkg>` plus `apply`.
- **Dependency handling.** winget manifests declare dependencies and dotpkg has
  no vocabulary for a package it did not declare appearing after an install.
  Unmeasured and out of scope; recorded as a real gap, not a closed question.
- **`--location`, `--all-versions`, and side-by-side versions of one id.** All
  three unmeasured — the first confounded by measurement 2, the second blocked by
  measurement 4, the third never constructible because every `install --version`
  either upgraded or did nothing.
- **Removing a machine-scope package while elevated.** Unmeasured; A4 refuses
  narrowly (user-scope only) rather than guessing.
- **Any installer type other than `portable`, for the success paths.** The
  upgrade-reinterpretation was confirmed against real EXE-installer packages, but
  a successful downgrade, upgrade or uninstall of an MSI/EXE package is
  unmeasured, and `--silent`'s behaviour for an installer with a GUI is
  unmeasured.
- **`winget pin`.** Unchanged: two sources of truth about permitted versions is
  how a tool starts lying. `pkg.lock` is dotpkg's answer.
- **Chocolatey**, architecture drift, same-version re-pin, and locking against
  two concurrent dotpkg runs. Unchanged.
