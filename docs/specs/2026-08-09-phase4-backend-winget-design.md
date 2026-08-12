# dotpkg Phase 4 — generalise the pipeline, then add winget

**Status:** design proposed 2026-08-09, not yet implemented.
**Depends on:** Phase 3 (`update`, `adopt`), with `main` at `9338b80`.
**Carries:** five of the eleven items in `docs/phase3-notes.md` "Still open" —
1 (`mass_prune_guard`), 2 (the exit-code floor), 3 (15 mutants in
`backend/scoop.rs`), 8 (`State::names`) and 11 (scan integrity). They are
carried because each one gets *harder* once a second backend exists, not
because Phase 4 is a convenient place to put them.

The approved design promised at `docs/specs/2026-08-08-design.md:95` that *"the backend trait exists
from v1 so choco slots in without touching the planner."* That was never built.
`Backend` has `name` and `scan`; `resolve` is a free scoop-only function called
directly from `update::run`; and `SCOOP` appears on **141 lines of `src/`**
(198 counting `tests/`). Phase 4 is where the promise is either made true or
withdrawn.

## Scope, decided before this document was written

**Phase 4 stops at scan / plan / lock / report. It does not execute winget.**

Phase 2b-2 was rewritten end to end by one measurement round — that scoop
never reports operational failure through its exit code. The equivalent round
for winget cannot be run: winget has no `$env:SCOOP` equivalent, so there is no
throwaway root, and every install/uninstall experiment touches the real
machine. `docs/measurements-2026-08-09-winget.md` therefore contains **no
mutating measurement at all**, and an executor written against an unmeasured
mutation surface would repeat the mistake Phase 2b-2 exists to record.

So after this phase: `status`, `update` and `adopt` tell the truth about winget;
`apply` reports what it will not do and says why. The executor is Phase 4b, and
it is gated on a mutation measurement, not on a calendar.

## The measurements that set the shape

Full record: [`docs/measurements-2026-08-09-winget.md`](../measurements-2026-08-09-winget.md).
Four rounds on a14, winget `v1.29.280`, Arm64, `en-US`. The five results this
design is built on:

1. **`Id` is not unique.** 140 rows, 125 distinct ids, 8 duplicated up to x4 —
   and `7zip.7zip`, `Microsoft.WindowsAppRuntime.2` and
   `Microsoft.UI.Xaml.2.8` each carry **two different versions** at once.
2. **83 of 140 installed rows have no source**, so they are installed and not
   comparable against anything.
3. **`--exact` makes `--id` case-sensitive.** `show -e --id git.git` exits
   `0x8A150014`; `show --id git.git` exits `0` and answers
   `Found Git [Git.Git]`.
4. **winget's exit code is honest for the argv shapes tested, with one
   measured exception**: `list -s msstore` returns the identical 53-byte
   "not found" sentence as `list -e --id <absent>` and exits `0` where the
   other exits `0x8A150014`.
5. **`show -v <version>` is the pin-liveness check**, with `0x8A150017` for a
   version that is gone and `0x8A150014` for a package that is gone.

## Corrections to the approved design

Recorded here rather than edited in place, matching the precedent set by the
2a, 2b-2 and Phase 3 designs.

**`docs/specs/2026-08-08-design.md:78` — "winget pins a version, not a hash" — is wrong about
winget.** `winget show` prints `Installer Url` and `Installer SHA256`, and
winget verifies the hash; `show -e --id ajeetdsouza.zoxide -v 0.9.0` returns a
complete manifest with its SHA256 for a release dated 2023-01-08. winget
manifests are the same "URL + hash" shape `docs/specs/2026-08-08-design.md:60` credits scoop with.

What winget lacks is a **local content handle**. A scoop bucket is a git clone
on the user's own machine, so `git show <commit>:bucket/<app>.json` recovers a
historical manifest offline and forever. winget's source is a pre-indexed cache
served from a CDN: no object database, no commit to name, nothing to address
content by. The corrected sentence:

> A winget pin is a **request**, re-resolved against an index dotpkg does not
> hold, at the moment of install. A scoop pin is a **content address**,
> resolvable from the user's own disk.

`pin = "version-only"` in the lock stays exactly as it is. It was always the
right field; only the explanation was wrong.

**`docs/specs/2026-08-08-design.md:78`'s "only while that version's manifest still exists upstream"
is right, and the window is three orders of magnitude wide.** Versions in the
index today: `JanDeDobbeleer.OhMyPosh` 828, `Brave.Brave` 150, `Git.Git` 73
(back to `2.24.1.2`), `Obsidian.Obsidian` 65, `ajeetdsouza.zoxide` **11**,
`BurntSushi.ripgrep.MSVC` **8**. Retention is a publisher policy, not a winget
guarantee, and a version can be missing from the middle of a run that looks
continuous (`2.30.2`, `2.30.1`, `2.30.0.2` exist; `2.30.0` does not).

This is a fact `status` should be able to state, not only a caveat in a README.

**`docs/specs/2026-08-08-design.md:257`'s scan-cost table omits the expensive case.** `winget list`
at 1213 ms is confirmed warm (measured 1105 / 1117 / 1108 ms via
`Start-Process`). The **first invocation of a session is 8125 ms**, and
"cached once per run" does not help the first run — which is the one
`dotpkg status` makes.

**`src/model.rs:10` is wrong, and `Name` is built on it.** The doc comment says
*"Scoop and winget both resolve names case-insensitively."* Measured, that
holds for winget only when `--exact` is absent. Since `Name::key()` is the
folded form and Phase 3 settled that the scoop lock records
`bucket_name.key()`, the obvious symmetry is a defect: **any path that puts a
folded name into `winget --exact --id` gets "not found" for a package that
exists.** This is the Phase 3 bucket-spelling defect pointing the other way.

**`docs/specs/2026-08-08-design.md:243`'s `Backend` sketch will not survive contact.**
`fn resolve(&self, pkg: &str) -> Result<Pin>` is too narrow in three ways: it
returns `Result` where the shipped code needs a per-package *outcome* that does
not abort the run (`update::Resolution`), it takes a `&str` where the crate has
`Name` precisely to stop `&str` comparisons, and it has no place for the bucket
/ offline / lock context `resolve_latest` already needs. The trait below
replaces it.

**`docs/specs/2026-08-08-design.md:245`'s `helpers()` does not generalise, and is not where it was
put.** It lives as `plan::SCOOP_HELPERS`, not on the backend. Its winget
counterpart is not a fixed list — it is the 83 sourceless rows, which change
per machine. The generalised idea is *"the backend decides which installed
entries are reportable"*, which is a method over the scan, not a constant.

## Half A — generalise, with no winget behaviour

Ordered so that each step is easier now than it would be after a second backend
exists. Nothing in Half A changes what dotpkg does on a machine.

### A1. `Backend::name` gets an assertion, before there are two names

`backend/scoop.rs:219` survives mutation to `""` and to `"xyzzy"`. Everything
keys on that string: `state.json` is a map keyed by backend name, `plan()`
compares against `model::SCOOP`, `owned_count(SCOOP)` is what
`mass_prune_guard` reads. Done first, and with the other 14 survivors in that
file (`docs/phase3-notes.md` lists them), because a second backend makes every
one of them ambiguous rather than merely untested.

`name()` becomes `-> &'static str` returning the `model` constant, so the two
backends cannot disagree with the two constants.

### A2. `Scan` carries what it could not establish, and `plan()` skips it

`docs/phase3-notes.md` item 11, generalised. The auditor's phrasing was *"the
names `scan` could not read"*; winget supplies a second cause with an identical
consequence — installed, but not resolvable against any source. One field, not
two:

```rust
pub struct Scan {
    pub installed: Vec<Installed>,
    /// Installed, but this backend could not establish its state. `plan()`
    /// must not read a name's absence from `installed` as "not installed":
    /// the scoop case is a manifest that cannot be traversed, the winget case
    /// is a row with no source, and both would otherwise become `Install` and
    /// then, under `--yes`, an uninstall-and-reinstall of a working package.
    pub opaque: Vec<Name>,
    pub warnings: Vec<String>,
}
```

`plan()` emits `Skip { reason: Opaque }` for a declared name in `opaque`, and
emits nothing at all for an undeclared one — an entry whose state is unknown is
not evidence of a stray.

**This is the item that most needs doing before B1**, because retrofitting the
field after two scanners exist is two migrations instead of one.

### A3. The exit-code floor becomes a pure function

`main.rs:411`. Two mutants (`&&`→`||`, `delete !`) are structurally unreachable
from `tests/cli.rs`: they diverge only on a fully successful non-empty `apply`,
and no fixture can build one without a real scoop. `docs/phase3-notes.md`
recommends the seam move, and this phase takes it:

```rust
fn floor_exit_code(code: i32, preparation_ok: bool, has_running_skips: bool) -> i32
```

Same move as `write_in_order` and `parse_batch`: the behaviour becomes
*observed* rather than inferred, on every platform.

### A4. `State::names` gets a caller or is deleted

Zero callers in `src/` or `tests/`. A6 gives it one, or it goes.

### A5. `plan()` becomes one pass per backend, and states its invariant

Today `plan()` runs a scoop declared-loop (`plan.rs:130`), a winget stub loop
(`:231`) and a scoop undeclared-loop (`:240`). It becomes one loop over
backends. Two things must be settled explicitly rather than inherited:

**The invariant, which has never been written down and is false for winget:**

> **At most one `Installed` per `(backend, name)`.** The declared loop's
> `installed.iter().find(...)` silently takes the first of several; the
> undeclared loop iterates all of them and would emit **two `Prune` actions for
> one package**.

Half B's `Winget::scan` upholds it, by the rule set out in B1: duplicates that
agree on a version collapse to one entry with a warning, and duplicates that
disagree go to `opaque`. `winget export` also collapses — 57 source-backed rows
become 42 entries — but does it silently, which dotpkg may not. The invariant is
enforced in `plan()` with a `debug_assert!` and tested at the seam, so a future
backend that breaks it fails loudly rather than pruning twice.

**`SkipReason` loses `Copy`** — it is `#[derive(Debug, Clone, Copy, PartialEq,
Eq)]` today — **and gains two variants:**

```rust
pub enum SkipReason {
    Running,
    NotLocked,
    /// Installed, but the backend could not establish its state (A2).
    Opaque,
    /// The backend can scan and resolve, but cannot act. Carries the diff so
    /// `status` still tells the truth about the machine.
    ReportedOnly(Divergence),
}

pub enum Divergence {
    Install { version: String },
    Change  { from: String, to: String },
    Prune   { version: String },
}
```

`BackendNotImplemented` is deleted — 13 sites across `src/` and `tests/`. After
Half B, winget *is* implemented for everything except acting, so a declared
winget package with no lock entry is `NotLocked` exactly like a scoop one.

**`ReportedOnly` is deliberately not an `Action`.** A winget upgrade is a true
fact about the machine and a non-event for this run, so it must appear in
`status` and must **not** be counted by `Plan::change_count()` — which is what
prints `4 changes, 1 skipped. Continue?`. Counting it would put a false number
in the one line the user reads before saying yes, which is the defect class
Phase 3 fixed twice in `render.rs`.

`main.rs` floors the exit code to 1 when any `ReportedOnly` is present, by the
same rule already applied to running skips: outstanding work the user asked for
and did not get.

### A6. `mass_prune_guard` grows a backend loop

`apply.rs:37`. The bug is the shape, not the constant: `if
!declared.scoop.packages.is_empty() { return Ok(()) }` returns from the whole
function, so **any** declared scoop package disables the check for every other
backend. The per-backend form turns that `return` into a `continue`:

```rust
for backend in BACKENDS {
    if declared.count(backend) > 0 { continue; }
    let owned = state.owned_count(backend);
    anyhow::ensure!(owned == 0, "pkg.toml declares no {backend} packages but \
        dotpkg owns {owned}. Refusing to prune everything. \
        If the file is right, pass --allow-empty-config.");
}
```

`tests/cli.rs`'s `an_empty_config_is_refused_before_the_machine_is_even_scanned`
covers the scoop half and keeps passing while the winget half is missing, so
the winget half needs its own test **and** a test with a non-empty `[scoop]`
and an empty `[winget]`, which is the case the current short-circuit lets
through.

### A7. `resolve` moves onto the trait, and `Resolution` carries a `Pin`

```rust
pub trait Backend {
    fn name(&self) -> &'static str;
    fn scan(&self) -> Result<Scan>;
    /// `update`: what is newest. The only method that reaches a network.
    fn resolve_latest(&self, name: &Name, ctx: &ResolveCtx) -> Resolution;
    /// `adopt`: a pin describing what is installed right now. Reaches no network.
    fn resolve_installed(&self, inst: &Installed, ctx: &ResolveCtx) -> Resolution;
}
```

Two resolvers, not one, because Phase 3 measured that they need **opposite git
flags** (`update` uses `git log -1`; `adopt` uses `--full-history`) and each has
a measurement justifying it. Collapsing them would break one of the two.

`Resolution::Resolved { bucket, commit, version }` becomes
`Resolution::Resolved { pin: Pin }`. `Pin` is already the asymmetric type, so
**a winget resolution carrying a commit stops being a bug to be caught and
becomes a program that cannot be written** — the same move `Name` and
`WriteLock`/`WritePkgToml`/`WriteState` already make in this crate.

`ResolveCtx`'s exact contents are a plan-level decision. The binding constraint
is that `update::run` must no longer name `bucket::resolve_latest`.

### A8. `Scoop::stage` names the command that fixes it

Settled before this document. When `git cat-file -e` fails, the message says
`git -C <dir> fetch`, in the same shape `src/adopt.rs` already uses for a
shallow clone. Cheap, honest, no behaviour change.
`apply --fetch-missing-commits` waits until the gap is seen to bite.

## Half B — the winget backend

### B1. `scan` parses `winget list`, and `export` is its control

`winget list --disable-interactivity`, one invocation, cached per run.

Chosen over `winget export --include-versions` even though export is JSON with
a published schema and costs the same (1073–1109 ms against 1105–1117 ms),
because **export loses exactly the two facts dotpkg must not lose**: it dedupes
ids silently (57 rows → 42 entries) and drops all 83 sourceless rows, naming
them by `Name` rather than by `Id`. `list` strictly dominates it in information
content.

The measured properties the parser may rely on, under redirected stdout: no
truncation and no `U+2026` at any width tried; three consecutive runs
byte-identical; `COLUMNS` and console buffer size do not reach the output.

The properties it may **not** rely on, also measured:

- Column offsets are recomputed per invocation → read them from the header row
  every time, never hardcode.
- **The column set is data-dependent** — `Available` is absent whenever no row
  has an upgrade → key on header names, not on column count.
- The header is English → **if the header row is not the expected shape, `scan`
  refuses and says so**, rather than guessing offsets. A backend that guesses
  here reports an empty machine, and an empty machine is what
  `mass_prune_guard` exists to catch too late.

Each row becomes an `Installed { backend: WINGET, name, version, arch: None,
bucket: None, bins: vec![] }`, subject to:

- **`Source` is empty → the row goes to `Scan::opaque`, not to `installed`.**
  It is installed and not comparable. The `MSIX\…` / `ARP\…` id is kept as the
  `Name` so a declared package in that state can be matched and skipped.
- **Duplicate ids collapse to one entry, with a warning naming both versions.**
  Which one survives is not a coin flip: the entry is kept only if every
  duplicate agrees on the version, and otherwise the id goes to `opaque` — two
  installed versions is precisely the state dotpkg has no vocabulary for, and
  picking one would be inventing a fact.
- **A version starting `> ` goes to `opaque` too.** Measured, `> 17.14.37` is
  winget saying *at least*, for a single install whose exact version it cannot
  determine. Left in `installed` it makes `cur.version == want` false forever
  and `is_older` choose `Downgrade`, so `status` would print a `↓` on every
  run for a package that is fine.

`bins` stays empty and `Running::covers` therefore falls back to its name and
directory halves for winget. Recorded as a known weakness rather than papered
over: the running-process guard is **weaker for winget than for scoop**, and
since Phase 4 does not act on winget packages, nothing depends on it yet.

`winget.exe` absent is a valid empty state, exactly as a missing `~/scoop/apps`
is for `Scoop::scan` — `Scan::default()` plus one warning.

### B2. `resolve`, and the canonical-id rule

**Every winget invocation uses `--id <spelling>` without `--exact`, and reads
the canonical id back from `Found <name> [<Id>]`.** Measured: `--exact` makes
`--id` case-sensitive, `--exact`'s absence folds case, and the bracketed id is
what winget actually matched. Asking with what the user wrote and recording
what winget answered is one self-verifying call, and it is the same rule Phase 3
settled for buckets — *record the thing that was actually opened*.

So `update` and `adopt` write the **canonical** id into `pkg.lock`, and a
`pkg.toml` whose spelling differs in case is reported, not silently rewritten.

- `resolve_latest`: `winget show --id <name> --disable-interactivity`, read the
  `Version:` line. Measured to agree with `show --versions` row 1 on 6 of 6
  packages; `show` is chosen because it is one line to parse rather than a
  table. ~1.09 s/package → ~18.5 s for a14's 17 declared packages.
- `resolve_installed`: the installed version *is* the pin, but only if it still
  exists in the index — so it is confirmed with
  `winget show --id <name> -v <version>`, and `0x8A150017` is a refusal naming
  the version and how many versions the index still holds. A package in
  `Scan::opaque` has no installed version dotpkg can vouch for, so **`adopt`
  refuses it** rather than pinning a version it could not read.

**Exit codes are trusted only for the argv shapes above, and those shapes are
pinned by tests.** `list -s msstore` proves the code is a function of the
filter, not of the output; a blanket "winget exit codes are reliable" would be
false and is not what this design claims.

### B3. Reaching the network, and what `update` is allowed to do

Phase 3's rule — *"latest in a bucket nobody has fetched is latest as of the
last time something else pulled it"* — applies unchanged. The winget analogue of
`git fetch` is `winget source update`.

**It is not read-only, and this is the one place winget is worse than git.**
Measured: bare `winget source update` left the installed set otherwise
untouched (0 `Available` changes, 0 rows lost) but **added one row** — winget's
own source-index MSIX for the `winget-font` source, which `source list` marks
`explicit=true`. `git fetch` cannot install anything; this can.

So `update` runs **`winget source update --name winget`**, scoped to the one
source dotpkg reads. It exits `0`; whether the scoped form avoids the MSIX
install was **not verified**, and verifying it is the first task of the plan,
before any code depends on the answer. `--offline` skips it, as for scoop.

### B4. Lock coherence for winget entries

`apply::incoherent_entries` iterates `lock.scoop` only, and `entry_coherence`
bails on any non-`ScoopCommit` pin. Once winget pins are real that map needs its
own rules — a non-empty version, no path separators, and the id spelled as
winget spells it. Written as a second arm of `entry_coherence`, not as a second
function, so `update`'s "which entries do I repair" message covers both
backends from one place.

## What the user sees

```
$ dotpkg status
  + scoop  ripgrep       15.2.0                     (install)
  ! winget Brave.Brave   151.1.93.132 -> 151.1.93.134
                                     reported only -- dotpkg cannot change
                                     winget packages yet
  ! winget OpenAI.Codex  0.145.0     would prune -- reported only
  ! winget Microsoft.VisualStudio.2022.BuildTools
                                     installed, version not determinable
                                     (winget reports "> 17.14.37")
  ? scoop  antigravity   2.0.6                      (unmanaged, not adopted)

  1 change, 3 skipped.
```

```
$ dotpkg update
  + winget Brave.Brave   151.1.93.134               (new pin)
  ! winget ripgrep.MSVC  kept the previous pin: version 15.1.0 is no longer
                         in the winget index (8 versions retained, oldest …)
```

That last line is why the retention depth is worth carrying out of `resolve`:
"the manifest is gone" and "this publisher keeps eight releases" are different
amounts of help.

## Testing

Layers 1 and 2 from the approved design, unchanged: everything below runs on
macOS and Linux.

**Phase 3 could build real git repositories because git is on every machine.
Phase 4 cannot: winget exists only on Windows.** So the winget backend is split
at a seam that takes text, the way `Mutator` and `parse_batch` already are:

```rust
fn parse_list(stdout: &str) -> Result<Scan>
fn parse_show(stdout: &str) -> Result<Found>
```

and the fixtures are **the captured bytes from a14**, checked in — the real
140-row table with its duplicate ids, its 83 sourceless rows, its `> 17.14.37`
and its missing `Available` column, plus the filtered single-package outputs and
every failure sentence. A fixture invented by hand would be self-consistent with
a winget nobody ran.

The subprocess layer above those functions is behind the existing `Mutator`-style
seam so no test spawns `winget.exe`, and **no test may create a file at
`winget.exe`'s resolved path** — the standing rule inherited from
`Scoop::scoop_exe()`.

The coverage this plan requires by name, following
`docs/phase3-notes.md`'s third pattern — *ask what each module produces that
something downstream consumes, and require that thing to be asserted*:

| Producer | Consumer | Must be asserted |
|---|---|---|
| `Winget::scan` | `plan()` | duplicate ids collapse; a sourceless row lands in `opaque` and **not** in `installed`; `> ` lands in `opaque` |
| `plan()` | `render` and `$?` | a `ReportedOnly` is **not** counted by `change_count()`; a `ReportedOnly` floors the exit code to 1 |
| `mass_prune_guard` | the machine | empty `[winget]` + non-empty `[scoop]` + owned winget packages **refuses** |
| `render_status` | a human | every `ReportedOnly` line, with an absence counterweight |
| `resolve_installed` | `pkg.lock` | the canonical id is written, not the user's spelling |
| `parse_list` | everything | a header that is not the expected shape **refuses** rather than returning an empty scan |

**Negative controls.** `docs/phase3-notes.md` records that ten controls across
Phase 3's plan were un-fireable or mis-aimed, and that **every one that failed
was aimed at an external tool's behaviour while every one that held was aimed at
this crate's own logic**. Phase 4 has more external surface than any phase so
far, so:

- Controls are written against **the checked-in a14 fixtures**, not against
  reasoning about what winget would do. The fixtures are the recording; the
  control asserts against the recording.
- Every refusal assertion is paired with a count of files written (which must be
  zero) or with a positive sibling that must stay green.
- No control may consume a `Result` with `unwrap_err()` before its other
  assertions run.

**The rule that outranks this brief:** if a negative control cannot be made to
go red, that is a **failure of this plan**, not of the implementer. Fix the
test, say so in the notes, and do not ask first. Phase 3 lost a round to an
implementer who diagnosed exactly this correctly, verified the fix, and did not
dare apply it.

**Standing rules kept:** `--no-fail-fast` on every run; the suite runs on
Windows **before the dogfood and again at the end of the change**; a
whole-branch review plus `cargo mutants` before merge; an independent audit
after merge.

## Dogfood

a14, 17 declared winget packages alongside the 25 scoop ones. As in Phase 3, no
command in this phase changes installed software — so the risk is not the run,
it is that what the run *records* is wrong.

Framed so it can fail:

1. Does `Winget::scan` agree with `winget export --include-versions` on the
   42 source-backed ids, and does it account for every one of the 15 rows
   export drops as a duplicate? A disagreement is a finding either way.
2. Does `scan` put exactly the 83 sourceless rows into `opaque` and none of
   them into `installed`?
3. Does `update` produce a lock for all 17 that `apply` then reports rather
   than acts on — and does `apply`'s exit code become 1 for that reason and
   name it?
4. Does any declared winget package fail `show -v <pinned>` today? If none
   does, the `0x8A150017` path is documentation rather than a working path,
   and that must be said rather than implied. It can be induced with a pin
   edited to a version the index does not hold.
5. Is `pkg.toml` byte-identical after `adopt` except for the added line?
6. **Does `winget source update --name winget` leave the installed set
   unchanged?** Captured, diffed field by field, before and after — the same
   check that caught the `winget-font` MSIX.
7. Does a `pkg.toml` declaring a winget id in the wrong case get reported
   rather than silently rewritten?

`kanata` is never started or stopped. `C:\Users\kln\dotpkg-build` and
`C:\Users\kln\pkg.toml` are reused. No `winget install`, `winget uninstall`,
`winget upgrade` or `winget pin add` is run at any point.

## Non-goals

Unchanged from the approved design. Additionally, Phase 4:

- **does not execute winget** — see "Scope" above. `apply` reports and refuses.
- **does not use `winget pin`.** Its shape was measured (`add` / `remove` /
  `list` / `reset`; `pin add -v` takes a trailing `*` wildcard; `pin list` is
  empty on a14) and it is deliberately unused: it is winget's own record of
  which versions are permitted, and two sources of truth about that is how a
  tool starts lying. `pkg.lock` is dotpkg's answer.
- **does not implement `add`.** The Phase 3 design filed it as a question, and
  the answer this document proposes is **no third resolver**: `add` is
  `pkg.toml` plus `update <pkg>` plus `apply`, composed. Settled in Phase 5, not
  here.
- **does not touch chocolatey**, which `docs/specs/2026-08-08-design.md:91` defers to v2 for a reason
  that still holds.
- **does not act on architecture drift or on a same-version commit re-pin**, and
  does not add locking against two concurrent dotpkg runs.
- **does not close `docs/phase3-notes.md` items 4, 6, 7, 9 or 10.** They are
  real and none of them gets harder because a second backend exists, which is
  the test this phase used to decide what to carry.
