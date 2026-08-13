# dotpkg Phase 14 — `[winget.opts] pin = "none"`

**Status:** design proposed 2026-08-13, not yet implemented.
**Depends on:** `main` at `d12e826`, with the eight Unreleased behaviour changes
already landed.
**Answers:** `docs/OPEN-ITEMS.md` §A item 1's other half — the half that was
decided against and never replaced with anything.

## What this is

A way to say *"this application belongs on this machine; do not manage its
version at all."*

```toml
[winget]
packages = ["Brave.Brave", "BurntSushi.ripgrep.MSVC"]

[winget.opts]
"Brave.Brave"     = { pin = "none" }
"Vivaldi.Vivaldi" = { pin = "none" }
```

Install if absent. Never upgrade. Never downgrade. No `pkg.lock` entry, ever.
Not counted as a change once present. Still pruned when it stops being declared
and dotpkg owns it.

## Why item 1 needs an other half

Item 1 records the refusal to downgrade a winget package, and the reasoning
holds: uninstall-then-reinstall would put a nightly loop on every self-updating
application. What it never said is what a user of such an application is
supposed to write instead.

Measured on `zenbook-a14` (ARM64, winget 1.29.280) on 2026-08-12, by the first
dotfiles repository to call dotpkg rather than be run by hand:
`Brave.Brave`, `Vivaldi.Vivaldi`, `Google.Chrome`, `Discord.Discord` and
`Warp.Warp` were **removed from the declaration entirely**. Each moves past its
pin within days, dotpkg refuses the downgrade correctly, the run exits non-zero,
and the calling module failed on every invocation. So for exactly the class of
package the winget backend exists to serve — GUI applications that update
themselves — dotpkg could not express the thing the hand-written PowerShell it
replaced could express, which was only ever *ensure presence*.

The pin is not the value here. Presence is. Today dotpkg can only offer the
first, and charges the second for it.

## The rule this rests on, and the one it must not break

`docs/OPEN-ITEMS.md` §A item 7 refuses `winget pin` because *"two sources of
truth about permitted versions is how a tool starts lying."*

**An unpinned package therefore gets no `pkg.lock` entry.** `pkg.lock` records
what a declaration resolved to; an unpinned declaration resolves to nothing, and
writing a version there would be the second source of truth item 7 rejects — one
that `apply` would then be obliged either to enforce (which is a pin) or to
ignore (which is a lie in a committed file).

Everything below follows from that one sentence.

---

## 1. `pin` is a closed enum, and `[winget.opts]` is its own struct

### The enum

```rust
/// How dotpkg manages a declared winget package's version.
///
/// A closed set for `config::Arch`'s reason, in its own words: `arch = "arm"`
/// used to parse and mean "installed wrong, forever". `pin = "latest"` would
/// parse and mean "pinned after all", which is the same failure pointing the
/// other way — the user asks for one behaviour, gets the opposite, and nothing
/// tells them.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Pinning {
    /// `pkg.lock` records a version and `apply` holds the package to it. The
    /// default, and spellable, so a reader of a `pkg.toml` that mixes the two
    /// can say which is which without knowing which way the default falls.
    #[default]
    Version,
    /// Install if absent; never manage the version.
    ///
    /// Spelled `Unpinned` in Rust and `"none"` in TOML on purpose. `Pinning::None`
    /// beside `Option::None` in the same match is a name collision in the one
    /// place this crate cannot afford one: a `match` arm that reads as the
    /// wrong thing to a human still compiles.
    #[serde(rename = "none")]
    Unpinned,
}
```

Serde's own message for a bad value lists the real ones (`expected one of
\`version\`, \`none\``), which is what
`a_misspelled_architecture_is_an_error_not_a_permanent_drift` already asserts
for `Arch`. The winget twin asserts the same.

### The struct — separate from `PkgOpts`, not a reuse

`PkgOpts` carries `arch` and `bucket`. Both are scoop concepts and both are
`deny_unknown_fields`. Reusing it would make three things legal that must not
be:

- `[winget.opts] "X" = { arch = "arm64" }` would parse and do **nothing**.
  winget exposes no architecture at all — `rows_to_scan` leaves
  `Installed::arch` `None` for every winget row, and `plan_backend`'s arch block
  reads `Installed::arch` — so the declaration would be inert. That is
  `arch = "arm"` exactly: parses, means nothing, forever.
- `[winget.opts] "X" = { bucket = "extras" }` would parse and name a scoop
  bucket for a package that has none.
- `[scoop.opts] fzf = { pin = "none" }` would parse. Scoop's pin is a **bucket
  commit**, not a version; "unpinned" for scoop would have to mean *install
  from whatever commit is at the tip on the day it is installed, and never look
  again*, which is a different feature with a different failure mode. Making it
  spellable before it is designed is how a value gets a meaning by accident.

So:

```rust
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WingetOpts {
    #[serde(default)]
    pub pin: Pinning,
}
```

`WingetSection` gains `opts: BTreeMap<Name, WingetOpts>`, folded through
`fold_map` with `"[winget.opts]"` as its label — the same collision refusal
`guard` and `[scoop.opts]` already get, for the same measured reason (a
`BTreeMap<Name, V>` built from string keys keeps the FIRST key and the LAST
value and says nothing).

### An opts entry for an undeclared package is a parse error

`[winget.opts] "Brave.Brave" = { pin = "none" }` with `Brave.Brave` absent from
`[winget] packages` is refused at parse time, naming both the entry and the
section it is missing from.

The reason is the whole point of the feature: a user who typos the id in
`packages` but spells it right in `opts` gets a **pinned** package where they
asked for an unpinned one, and the only symptom is the refused-downgrade line
they added the opts entry to remove. Silence there is the `arch = "arm"` class
again.

`[scoop.opts]` does **not** have this rule and this design does not add one.
Its entries are inert in the same way, and changing that is a separate decision
about a different backend's file; it goes on `docs/OPEN-ITEMS.md` as a noted
asymmetry rather than being smuggled in here.

### `verify_round_trip_winget` has a named obligation, and this triggers it

`verify_round_trip_winget`'s own doc comment says: *"every OTHER field of
`WingetSection` has to be named on the `ensure!` line, and today that is
exactly `guard`. … Adding a third field to `WingetSection` without adding it
here reopens that hole."*

`opts` is that third field. The `ensure!` becomes
`after.winget.opts == before.winget.opts` alongside `guard`, and the test that
pins it is the twin of
`the_round_trip_guard_rejects_a_scoop_edit_that_drops_the_winget_guard_table`,
written against `opts`. Without it, an `adopt` that dropped `[winget.opts]`
would pass silently — and dropping it turns five unpinned packages into five
pinned ones with no line anywhere saying so.

---

## 2. `Action::Ensure`, not `Action::Install { version: Option<String> }`

**Answering question 1 against the 55 sites.**

`Action::Install` has 55 references across `src/` and `tests/` (counted at
`d12e826`: `src/apply.rs` 27, `src/render.rs` 16, `tests/planner.rs` 6,
`src/plan.rs` 2, and one each in `src/main.rs`, `src/backend/mod.rs`,
`src/execute.rs`, `tests/prepare.rs`). Making `version` an `Option<String>`
touches every one of them.

It is refused, and not because 55 is a large number.

**It would not remove a single decision point — it would convert each one from a
compile-time arm into a runtime branch.** `check_pin_is_live` reads
`Install.version`; with `None` it needs a different code path. `render` prints
`{version:<24}`; with `None` it needs a different line. `ready_rest` prints
`{version:<18}(install)`; same. Every place that would have needed a new arm
still needs a new branch, and the other ~45 sites — most of them tests and
constructions — get an `Option` to thread through for nothing.

**It would make "no version" representable for scoop, where it means nothing.**
This is `Pin`'s argument, and `Pin`'s doc comment already makes it: *"Deliberately
asymmetric: only scoop can be pinned to content. Flattening these into one shape
would let a reader believe a winget entry carries the same guarantee as a scoop
one."* An `Install` with `version: None` says the two carry the same promise and
one of them merely forgot to fill it in. They do not. One promises a version;
the other promises presence.

The crate has made this call three times already and written down why each time:
`ReadyToFetch`/`ReadyToRemove` split rather than `Option<PathBuf>`
(*"an executor branching on `manifest.is_none()` to decide whether to uninstall
would then be right by luck"*), `ReadyToSet` split from both, and
`Step`/`ScoopStep`/`WingetStep` split rather than a backend flag carried
alongside.

So:

```rust
/// Declared with `pin = "none"` and not installed: put it on the machine, at
/// whatever version winget's index offers when the install runs, and record
/// nothing about which version that was.
///
/// **Carries no version, and that is the variant's whole content.** It is
/// emitted only when the package is ABSENT — an unpinned package that is
/// present produces no action at all, in either direction, forever.
Ensure {
    backend: String,
    name: Name,
},
```

### What that costs, exactly

`Action` is matched exhaustively in six places, and each gains one arm:
`apply::classify`, `apply::action_name`, `apply::action_backend`,
`render::render`, `render::action_backend_name`, and `plan::Plan::change_count`.
Three more (`render::ready_rest`, `render::refused_downgrade_rest`,
`render::report_marker_and_rest`) carry a `_` fallback; `ready_rest` gains a real
arm and the other two are correct to fall through.

`apply::plan_to_steps` matches `(action, outcome)` pairs with a trailing
`_ => {}`. An `Ensure` that reached it unrouted would land in the
`(a, Outcome::ReadyToSet { .. })`-shaped routing-bug arm rather than the silent
`_`, which is the loud behaviour that arm exists for. It gains a real arm anyway.

Nothing in `tests/` breaks: a new variant nothing constructs is invisible to
every existing test.

---

## 3. The planner: one branch, above the lock lookup

`BackendView` gains `unpinned: &BTreeSet<Name>` — winget's built from
`declared.winget.opts`, scoop's empty and stated to be empty because scoop has
no such concept rather than because nobody wired it up.

In `plan_backend`'s declared loop, **after** the `opaque` check and **before**
the lock lookup:

```rust
if view.unpinned.contains(name) {
    match view.capability {
        Capability::Acts => match current {
            // Present at any version. Not a change, not a skip, not a report:
            // there is nothing dotpkg has undertaken to do about it.
            Some(_) => {}
            None => actions.push(Action::Ensure {
                backend: view.backend.into(),
                name: name.clone(),
            }),
        },
    }
    continue;
}
```

Three placements, each load-bearing:

- **After `opaque`.** An opaque package's state could not be read, so "is it
  installed?" has no answer, and installing over an unknown state is the mistake
  `SkipReason::Opaque` exists to prevent. An unpinned package that comes back
  opaque is still `Skip { Opaque }`.
- **After the whole-backend `Unscannable` early return**, which already covers
  it: `installed` is empty by construction, so `current` would be `None` and
  every unpinned package would be fabricated into an `Ensure`. The existing
  return happens first.
- **Before the lock lookup.** This is the answer to question 4's
  81-reference problem, below.

Inside `match view.capability` for `Capability`'s stated reason: a backend that
can only report must be **made** to decide what it does about a declaration it
cannot act on, and a compile error is the only reliable way to ask. This is the
fifth such decision point; the doc comment counting them says four and becomes
five.

### `SkipReason::NotLocked` and its 81 references do not move

`NotLocked` has 81 references (counted at `d12e826`: `src/apply.rs` 43,
`src/render.rs` 10, `tests/planner.rs` 10, `src/plan.rs` 4, `tests/cli.rs` 4,
`src/update.rs` 3, `tests/adopt.rs` 3, and one each in `src/adopt.rs`,
`src/lock.rs`, `src/main.rs`, `tests/update.rs`).

**None of them changes.** An unpinned package never reaches
`let Some(pin) = view.lock.get(name) else { … NotLocked }`, because the branch
above returns first. So `NotLocked` keeps meaning exactly what it means today —
*declared, pinned, and `update` has not been run* — and keeps failing the whole
run at exit 2 for exactly that. The rule is not weakened for pinned packages by
one line, and it is not weakened for unpinned ones either: it simply never
applies to them, because they have nothing to be missing.

This is the design's central structural claim, and it is the reason `Ensure` is
a planner-level variant rather than an exemption bolted onto `NotLocked`. An
exemption would have had to be read correctly at all 81 sites. A branch that
returns before the lookup has to be read correctly at one.

---

## 4. The landmine: what argv an unpinned install uses

`set_argv` builds `install -e --id <id> --version <v> …`. Its own doc comment
says `-e` is safe *"because the lock holds the canonical spelling winget itself
echoed back"*. An unpinned package has no lock entry and therefore no canonical
spelling.

### Option (b) — drop `-e` — is refused, and the reason is a conflict nobody has noticed

The two statements this crate holds about `winget --id` **contradict each other,
on the same verb**:

- `docs/measurements-2026-08-10-winget-write-path.md` §7, measured on a14:
  `show --id 7zip`, `show --id Microsoft`, `show --id ripgrep`, `show --id git`
  and `show --id zoxide` — every one a real substring of a real installed id —
  all returned `0x8A150014` "No package found matching input criteria".
  Its conclusion: **"`--id` always requires the whole id; `--exact` only
  controls case."** `winget_exec::list_one_argv`'s doc comment cites this to
  argue that dropping `-e` costs nothing.
- `CHANGELOG.md`'s "A winget id that matches a *different* id is refused
  instead of pinned", and the same sentence in `update::run` and
  `adopt::run_winget`: *"`show` runs without `--exact` … which leaves `--id` a
  substring filter, so a declared `OhMyPosh` matches
  `JanDeDobbeleer.OhMyPosh`."*

Both are about `winget show --id <x>` without `--exact`. §7 says a substring
never matches; the 2026-08-13 fix says one did, and was built to refuse the
result. **The second claim carries no measurement.** It appears in three source
comments, the changelog and the commit message, and in no `docs/measurements-*`
document; §7 is the only measured statement either way, and it says the
opposite.

Dropping `-e` from a **write** verb is where a tool finds out which of those is
right, by installing something. That is not a place to find out. Option (b) is
refused on the strength of the disagreement, not on the strength of either
claim, and the disagreement itself goes on `docs/OPEN-ITEMS.md` as a new
unnumbered finding — it is a live inconsistency in the crate's own reasoning
about the flag every winget resolution depends on.

### Option (a), taken: resolve the canonical id at prepare time

An unpinned `Ensure` prepares by running the argv `Winget::resolve_latest`
already runs:

```
show --id <the declared spelling> --disable-interactivity
```

No `-e` — that is what folds case on the way in — and `parse_show` reads the
canonical id back out of the self-verifying `Found <name> [<Id>]` line, together
with the version winget's index currently offers. Both are needed and both come
from one call.

The install then reuses `set_argv` **verbatim**, with the canonical id and that
version:

```
install -e --id <the id winget echoed back> --version <what show reported> \
  --silent --accept-package-agreements --accept-source-agreements \
  --disable-interactivity
```

**Not one new argv, and that is the point.** Every flag keeps the measured
reason it already has (`docs/measurements-2026-08-10-winget-write-path.md`
§§1–2: this argv installs exactly `<version>` on a fresh install, hash-verified).
The alternative — omitting `--version` for a "just install it" call — would put
an argv on the wire that no measurement covers, in the module whose contract is
that §§1–9 are the only invocations winget's exit codes are trusted for.

`--version <what show reported one second ago>` is not a pin. Nothing records
it. It is the version the install happened at, which is exactly what "install
whatever winget's index has now" means, made explicit on the wire so
`run_winget_step`'s verdict has something to compare against.

### The different-id refusal is required for correctness, not only for consistency

If `parse_show` returns an id that is a **different id** from the declared one
(compared through `Name`, which folds case — so this is false for a mere case
difference), the package fails preparation, with `update`'s own sentence:

```
winget matched "JanDeDobbeleer.OhMyPosh", not the id pkg.toml declares
("OhMyPosh") -- declare it as "JanDeDobbeleer.OhMyPosh"
```

Without it, an unpinned declaration of `OhMyPosh` would install
`JanDeDobbeleer.OhMyPosh`, and then on the next run the scan reports
`JanDeDobbeleer.OhMyPosh` installed while `declared` says `OhMyPosh` — which
does not match, so `current` is `None`, so `Ensure` fires again. **A reinstall
every run, forever**, which is the shape item 1 exists to have prevented once
already. `update` and `adopt` refuse the same thing for the same reason.

A difference of **case alone** takes the canonical spelling onto the wire and
says nothing. There is no lock entry for it to disagree with — `update` warns
because `pkg.lock` and `pkg.toml` would then hold two spellings, and here there
is no `pkg.lock` entry at all. Stated rather than left to be noticed: for an
unpinned package the canonical spelling is used and recorded nowhere, which is
what "no lock entry" costs and what it buys.

### `version_liveness`'s move, made twice

`version_liveness` exists so that its two callers decide the argv and every
error sentence once. The `show --id <id>` argv now has two callers as well —
`Winget::resolve_latest` and this preparation — so it gets the same treatment:
a free function `index_latest(cmd: &dyn WingetCmd, id: &Name) -> Result<Found, String>`
beside `version_liveness`, taking `&dyn WingetCmd` for the same reason
(`prepare` holds the seam, not a backend). `Winget::resolve_latest` calls it and
keeps its own `ctx.canonical` write; `prepare` calls it and reads the `Found`
directly.

`resolve_latest`'s deliberately-absent `INTERNAL_ERROR` arm stays absent for
both callers, unchanged and for its own recorded reason: that argv has
**measurably zero** calls in the 105-call contention population, so borrowing
`version_liveness`'s wording would attribute someone else's measurement to it.

---

## 5. `--prepare`: a new `Intent`, a new `Outcome`, no new `Step`

**Answering question 3.**

### `Intent::NeedsIndexLookup`

`classify` sends `Action::Ensure` here. Its own variant rather than a second
reading of `NeedsLiveness`, on `NeedsLiveness`'s own argument — *"the two do
different work, produce different `Outcome`s, and a caller that confused them
would ask `Scoop::stage` for a manifest that does not exist"*. Here the
confusion would be worse in a specific way: `check_pin_is_live` reads a version
off the action, and an `Ensure` has none, so a confused caller reaches
`check_pin_is_live`'s `_ =>` arm and reports *"needs a liveness check but names
no version"* — a true sentence about an internal mismatch, printed at a user who
did nothing wrong.

There is nothing for `check_pin_is_live` to verify: **no pin exists, so pin
liveness is not a question that can be asked.** What is asked instead is *does
winget's index have this id at all, and under what spelling* — which is a
different question with a different failure sentence.

### `Outcome::ReadyToEnsure { id: Name, version: String }`

Split from `ReadyToSet` rather than reused, because `ReadyToSet`'s doc comment
makes a claim that would become false: *"The pinned version was confirmed still
present in winget's index."* For an unpinned package there is no pinned version
and nothing was confirmed against one — a version was **chosen** from the index.
Widening that sentence to cover both would leave it saying "some version, from
somewhere, is installable", which is the flattening this crate refuses
everywhere else.

It carries `id` for `ReadyToSet`'s exact reason, restated because it is the
whole landmine: the action's `name` is `pkg.toml`'s spelling, `id` is what
winget echoed back, and only the second may sit beside `-e`.

Counted by `ready_count` like every other ready shape.
`refused_winget_downgrade_count` is untouched: it matches
`(ReadyToSet, Action::Downgrade)`, and an `Ensure` is neither.

### No new `WingetStep`

`plan_to_steps` routes `(Action::Ensure { backend, name }, Outcome::ReadyToEnsure { id, version })`
with `backend == WINGET` into the **existing** `WingetStep::Set { id, version, guard: guard_for(name, installed) }`.

`Set` is reused because at the step level there is genuinely nothing to
distinguish: the same argv, the same `winget_verdict` rescan, the same
`At(v) if v == *version` → `Done`, the same `Ownership::Installed` claim. The
difference — where the version came from, and whether anything records it — is
settled upstream and irrelevant by the time a step exists. Splitting here would
produce two variants with byte-identical `run` bodies, which fails the
`Mutates` test in the other direction: *"flattening them into one signature
would either lose that difference or lie about it"* only bites when there is a
difference to lose.

That reuse is what makes the following need no changes at all: `execute::order`
(a `Set` groups with installs and opens no absent-window), `write_recovery` (a
winget line built from `set_argv`), `gate_removals` (`Set` is not a removal),
`refuse_elevated_winget_removal` (asks only about `Remove`), and
`count_replaces_and_installs` (already counts a `Set` as an install).

### What preparation costs, and when it costs nothing

One `winget show` per unpinned package **that is absent** — measured at ~1.09 s
on a14 (`docs/measurements-2026-08-09-winget.md`), the same call the pinned path
already pays, and paid on the same serial, uncached, unparallelised loop
`docs/OPEN-ITEMS.md` item 12 records as unmeasured.

**On a converged machine it is zero.** An unpinned package that is present
produces no `Action` at all, so it produces no `Prepared`, so it spawns nothing.
Five declared browsers, all installed, add nothing to a run. The cost is paid
once, on the run that installs each of them.

### Failure modes, all `Outcome::Failed`

Package not in the index; a different id matched; any other nonzero exit; winget
not on `PATH` (`CmdError::NotFound`, which `index_latest` turns into "winget
show could not be run"). Each fails `Preparation::is_ok`, so by default the run
refuses at exit 2 with nothing attempted, and `--keep-going` lets the rest
through — identical to a pinned winget package whose liveness check fails, and
identical for the same fail-closed reason. **No new exit code.**

---

## 6. What `status` and `apply` print

**Answering question 5, against `render.rs` rather than against the design.**

### Absent — one line, in both tables

`render(plan)`, whose `Action` arms are the seven above:

```
  + winget Brave.Brave    -                        (install, unpinned -- whatever winget's index has now)
```

The version column is `-`, not a version, because `render(plan)` is pure and
`status` runs no subprocess: at plan time nothing has asked winget anything and
there is no version to print. Printing `latest` there would be a claim about a
value dotpkg does not hold.

`render_preparation`, via a new `ready_rest` arm — by which point `show` has
answered, so the version is real:

```
  ready   winget Brave.Brave    151.1.93.134      (install, unpinned)
```

The two lines deliberately do not match, and the difference is the information:
the first says dotpkg does not know the version yet, the second says what it
turned out to be. Consistent with `--prepare`'s whole job.

### Present — nothing at all, and that is the existing rule

An installed unpinned package appears in **no line of either table**.

That is not new behaviour and not an omission: `plan_backend`'s
`Some(cur) if cur.version == want => {}` already prints nothing for a pinned
package sitting exactly where its lock says. "Converged produces no line" is the
existing rule, and an unpinned package is converged the moment it exists.

The honest cost, stated rather than left to be discovered: **a user cannot see
from `status` that dotpkg is managing an unpinned package's presence at all.**
The evidence that it is lives in `pkg.toml` and in `state.json`'s ownership
record, and surfaces the day the declaration is removed and the package is
pruned. This design accepts that rather than inventing an eighth `Action`
variant for "present and fine", which would print a line for every converged
package of both backends or would print one only for unpinned ones and be
inconsistent.

### What the summary counts

`Plan::change_count` gains `Action::Ensure { .. } => true`. An `Ensure` really
installs software, so it belongs in the `N change(s)` a user says yes to — the
same test `change_count`'s doc comment applies to the four shapes it already
counts, and the opposite of a winget `Downgrade`, which is excluded precisely
because it is measured never to happen.

It is counted by nothing else: not `skip_count`, not `drift_count`, not
`refused_downgrade_count`, not `unmanaged_count`. No new summary clause is
added, because unlike a refused downgrade or a collapsed unmanaged line there is
no printed fact left unaccounted for.

`apply`'s consent prompt is unchanged: `count_replaces_and_installs` counts the
`WingetStep::Set` as an install, and `installs_a_user_should_be_asked_about`
subtracts only refused downgrades.

The closing table prints the existing `done winget Brave.Brave` line, via
`ItemOutcome { backend: "winget", … }`.

---

## 7. `dotpkg update`: no entry, and no churn

**Answering question 4, including the churn half.**

### The resolution loop skips it

`update::run`'s winget loop skips any name whose `pin` is `Unpinned`: no
`winget show`, no `Resolution`. Nothing is resolved because nothing is recorded.

The `winget source update --name winget` call is gated today on
`!declared.winget.packages.is_empty()`. It narrows to *at least one **pinned**
winget package*: refreshing the index buys nothing for a run that resolves
nothing, and this removes a subprocess from a `pkg.toml` that declares only
unpinned packages.

### `fold_backend` handles it, after the scope check

Placed after `if !scope.covers(name)` and before `match resolutions.get(name)`:

```rust
if unpinned.contains(name) {
    changes.push(match previous {
        Some(p) => Change::Unpinned { backend, name: name.clone(),
                                      previous: Some(p.version().to_string()) },
        None    => Change::Unpinned { backend, name: name.clone(), previous: None },
    });
    // No `lock_map` insert. This is also what DROPS a stale entry: not
    // reinserting is the deletion.
    continue;
}
```

The placement decides three scopes, and each is correct:

| scope | what happens | why |
|---|---|---|
| `WholeRun` | line printed, any old entry dropped | the lock is being rebuilt; an unpinned package contributes nothing to it |
| `Named` naming it (`update Brave.Brave`) | line printed, any old entry dropped | the user asked about this package, and the answer is that there is nothing to pin |
| `Named` not naming it (`update fzf`) | old entry carried forward, no line | the existing `!scope.covers` branch fires first, and `update fzf` must not quietly rewrite anything else — the rule `fold_backend_keeps_the_canonical_key_*` already holds |

The second loop, which drops entries for undeclared packages, sees the name in
`declared` and skips it. So an unpinned package can never produce a
`Change::Dropped` line, and `Change::Dropped`'s existing text — `(dropped, no
longer declared)` — stays true wherever it is printed. **That sentence is why
`Dropped` is not reused here**: it would be a false line about a package that is
still declared.

### `Change::Unpinned`, and the churn bug it exists to close

```rust
Unpinned {
    backend: &'static str,
    name: Name,
    /// The pin this run removed, when there was one. `None` when there never
    /// was — the ordinary steady state.
    previous: Option<String>,
},
```

Two render lines, split on the `Option` for `Change::Kept`'s recorded reason —
*"Two different facts share this variant, and they must not read the same"*:

```
  - winget Brave.Brave    151.1.93.134               (pin dropped -- pkg.toml declares pin = "none")
  = winget Brave.Brave    unpinned                   (no pin -- pkg.toml declares pin = "none")
```

**`Update::wrote_anything` must treat only the `None` form as no-write.**

```rust
!matches!(c, Change::Unchanged { .. }
           | Change::Kept { .. }
           | Change::Unpinned { previous: None, .. })
```

Both halves are load-bearing and they fail in opposite directions:

- Without the exclusion, five declared unpinned browsers make **every**
  `dotpkg update` rewrite `pkg.lock` for a diff that is empty. That is the churn
  question 4 asks about, and it is real.
- With the exclusion applied to *both* forms, the run that removes a pin would
  report the removal and not write it, so the stale entry would **survive
  forever** and no `update` could ever clear it. `previous: Some(_)` is a real
  write and must count as one.

`render_update`'s summary counts `changes.len() - unchanged - failed_count()` as
"changed". `Unpinned { previous: None }` is not a change and gets its own
subtraction beside `unchanged`, or the line reports work that did not happen.

---

## 8. An existing `pkg.lock` entry when a package becomes unpinned

**Answering question 7: dropped by `update`, ignored by `apply`, never an
error, and warned about in between.**

- **Not an error.** A `Pin::WingetVersion` sitting in `lock.winget` is
  well-formed. `lock_coherence_guard` and `entry_coherence` check shape, not
  agreement with `pkg.toml`, and they must keep doing exactly that —
  `lock_coherence_guard`'s own doc comment records why a whole-run check against
  `pkg.toml` deadlocks: drop a package and its bucket line together, and the
  stale entry fails every later `apply` **including the prune that would clear
  it.** The same deadlock applies here.
- **Ignored by `apply`.** The planner's unpinned branch returns before the lock
  lookup, so the entry is read by nothing.
- **Dropped by the next whole-run `update`**, as §7 describes.

Between the `pkg.toml` edit and that `update` there is a committed file holding
a version nothing enforces. That is precisely the shape item 7 calls "how a tool
starts lying", even though nothing consults it — because a **user** reading
`pkg.lock` would consult it. So `status` and `apply` print one warning per such
entry, on stderr, beside `unprotected_winget_changes`' warnings and computed the
same way (purely, from `Config` and `Lock`, so it is testable without a
machine):

```
warning: pkg.lock still pins Brave.Brave at 151.1.93.134, but pkg.toml declares
it pin = "none" -- that entry is read by nothing. `dotpkg update` removes it.
```

---

## 9. `dotpkg adopt --backend winget`

**Answering question 6. It adopts, writes no lock entry, and spawns nothing —
and it is the only way an already-installed unpinned package can ever become
prunable.**

That last clause is the reason it must work rather than refuse. A user who
already has Brave installed and declares it unpinned gets: `apply` sees it
present → no action → dotpkg never installs it → dotpkg never **owns** it → no
prune can ever reach it. `adopt` is the only path to ownership for a package
dotpkg did not install, and closing it for unpinned packages would make
"still pruned when it stops being declared and dotpkg owns it" unreachable for
exactly the packages this feature exists for.

`adopt_one_winget` gains a branch, before `Backend::resolve_installed`:

- **No `resolve_installed` call, so no subprocess.** Its whole job is to confirm
  the installed version still resolves in the index, so that the version can
  become a pin. There is no pin, so there is nothing to confirm. The canonical
  id is already in hand and needs no lookup: `inst.name` is `winget list`'s `Id`
  column, canonical by construction — `adopt_one_winget`'s own comment says so.
- **No lock write.** `state.set(WINGET, &inst.name, Ownership::Adopted)` and, if
  the id is somehow not yet declared, the existing
  `config_edit::add_winget_package`.
- **`Matched::Unpinned`**, a fourth variant, rendered by `render_adopt`'s
  exhaustive `match matched` as:
  `nothing was pinned and nothing was confirmed -- only ownership was recorded`.
  `Matched`'s doc comment says the variants exist *"because the strength of the
  evidence differs and a user is entitled to know which one answered"*; here the
  honest answer is that no evidence was gathered because none was needed.
- **`previous_version` is `None`.** Nothing in the lock is replaced, because the
  lock is not written. A stale entry left over from a previous pinned
  declaration survives untouched and is cleared by `update`, per §8.

### `WriteLock` gains `Result<bool>`, matching `WritePkgToml`

`write_in_order` pushes `"pkg.lock"` onto `wrote` unconditionally once
`write_lock` returns `Ok`. For an unpinned adopt no lock write happens, so the
partial-write report would name a file that never changed — the exact false line
`WritePkgToml`'s `Result<bool>` was introduced to prevent, in its own words:
*"listing 'pkg.toml' there for a write that never happened would itself be the
kind of false line this module exists to avoid."*

`WriteLock` takes the same shape, and `run_winget` passes a closure returning
`Ok(false)` for an unpinned package. The wrapper types keep the arguments
un-swappable, so the ordering guarantee is unaffected.

---

## 10. The fence, the guards, ownership, and prune

**Answering question 9. Yes to all four, but not uniformly, and the differences
are worth naming rather than waving through.**

- **`[winget.guard]` and the running-process fence** apply wherever they can
  apply, which for an unpinned package is narrower than for a pinned one *and
  loses nothing*. The fence protects a package from being **replaced or
  removed** while live. An unpinned package is never replaced — that is the
  feature. So the fence's plan-time `running.covers(cur)` check, which sits in
  `plan_backend`'s `Some(cur)` arms, is never reached for it. Where it **is**
  reached is the undeclared loop: an owned unpinned package that stops being
  declared still gets `Skip { Running }` instead of `Prune` if its process is
  alive. Full protection on the only path that can hurt.
- **The mid-run re-sampler** is unchanged and does run. `WingetStep::Set` carries
  `guard: guard_for(name, installed)`, which for an absent package falls back to
  `guard_names(key, key)` — identical to every winget `Install` today, and
  correct for the same reason `guard_for`'s doc comment gives: *"a package that
  is not installed cannot be running, so the fallback's job is only to be defined
  rather than to be right."*
- **`unprotected_winget_changes` deliberately does not warn.** It matches
  `Upgrade`, `Prune`, and unreliable-comparison `Downgrade`; an `Ensure` is an
  install, and the README already states the rule this follows: *"An install
  never warns -- nothing is installed yet, so nothing of it can be running."*
  The omission is a decision, recorded here so it is not later read as an
  oversight.
- **`mass_prune_guard` is unchanged and correct.** It counts
  `declared.winget.packages.len()`, and an unpinned package is in `packages` — a
  `pkg.toml` truncated down to only `[winget.opts]` would have zero declared
  packages and trip the guard, which is right.
- **Ownership is unchanged.** `run_winget_step`'s `Set` arm claims
  `Ownership::Installed` when the rescan confirms the version, preserving an
  existing `Adopted`.
- **Prune works with no lock entry, for free, and it is worth saying why.**
  `plan_backend`'s undeclared loop builds `Action::Prune { version:
  inst.version.clone() }` — from the **scan**, not the lock — and
  `plan_to_steps` builds `WingetStep::Remove { version }` from that action. The
  removal's `--version` guard, which makes an unexpected version fail closed
  rather than remove the wrong one, is therefore just as strong for an unpinned
  package as for a pinned one. No lock is consulted on the removal path at all.

---

## 11. What does not change

Stated positively, because "we did not have to touch it" is the strongest claim
this design makes:

- **`src/lock.rs` entirely.** `Pin`, `parse`, `render`, `save`,
  `lock_coherence_guard`, `entry_coherence`, `incoherent_entries`. An unpinned
  package has no entry; there is no new pin kind and no new lock field. The
  `pin = "version-only"` string in `pkg.lock` is unrelated to `pkg.toml`'s
  `pin = "none"` and neither becomes readable as the other, because one is a
  `Pin` shape discriminator in a generated file and the other is a `Pinning`
  enum in a hand-written one.
- **`SkipReason` and its 81 references**, per §3.
- **`WingetStep`, `Mutates`, `Backends`, `run_step`, `execute`, `order`,
  `write_recovery`, `root_looks_like_scoop`, `gate_removals`,
  `refuse_elevated_winget_removal`, `winget_exec::set_argv`,
  `winget_exec::remove_argv`, `winget_exec::list_one_argv`,
  `winget_verdict`.** The executor never learns this feature exists.
- **Exit codes.** No new code, no change to `apply_exit_code` or
  `Preparation::outstanding`.
- **`Outcome::ReadyToSet`, `Intent::NeedsLiveness`, `check_pin_is_live`.**

---

## 12. Testing

Every test below gets a negative control: break the code, watch it go red with
the right message, restore, watch it go green, and report both. Baseline at
`d12e826` is 674 tests.

### `src/config.rs`

1. `pin_none_parses_and_the_absent_case_is_pin_version` — both spellings, and
   the default.
2. `a_misspelled_pin_value_is_an_error_that_lists_the_real_ones` — `pin =
   "latest"` fails, message contains `none` and `version`. Twin of
   `a_misspelled_architecture_is_an_error_not_a_permanent_drift`.
3. `a_scoop_opt_may_not_carry_pin_and_a_winget_opt_may_not_carry_arch` — both
   directions of the `deny_unknown_fields` split.
4. `a_winget_opts_entry_for_an_undeclared_package_is_refused` — names the entry
   and the section.
5. `two_winget_opts_keys_differing_only_in_case_are_rejected` — the `fold_map`
   twin of the existing `[winget.guard]` test.

### `src/config_edit.rs`

6. `the_round_trip_guard_rejects_a_winget_edit_that_drops_the_opts_table` — the
   named obligation from §1, written the same way its `guard` sibling is:
   positive control first (a clean `add_winget_package` preserves `opts`), then
   the guard called directly with a `before` carrying opts and an `out` without.

### `src/plan.rs` and `tests/planner.rs`

7. `an_unpinned_package_that_is_absent_is_an_ensure_and_not_an_install`.
8. `an_unpinned_package_that_is_installed_at_any_version_produces_no_action` —
   asserted at two versions far apart in both directions, so a mutant that
   compares versions at all is killed.
9. `an_unpinned_package_with_no_lock_entry_is_not_notlocked` — **the load-bearing
   one.** Its negative control is moving the unpinned branch below the lock
   lookup; the test must go red naming `NotLocked`.
10. `an_unpinned_package_that_is_opaque_is_still_skipped_as_opaque` — the
    ordering control for the branch placement, the other way.
11. `an_unscannable_backend_still_skips_an_unpinned_package` .
12. `an_owned_unpinned_package_that_stops_being_declared_is_pruned_at_the_scanned_version`.
13. `a_running_owned_unpinned_package_is_skipped_rather_than_pruned`.
14. `change_count_counts_an_ensure` — zero, one and two, per
    `refused_downgrade_count_is_the_number_of_winget_downgrades_not_a_fixed_one`'s
    recorded lesson that a one-item fixture cannot kill a `body → 1` mutant.

### `src/apply.rs` and `tests/prepare.rs`

15. `an_ensure_classifies_as_needs_index_lookup`.
16. `an_unpinned_install_prepares_with_the_canonical_id_winget_echoed_back` —
    fake `WingetCmd` returns `Found Brave [Brave.Brave]` for a declared
    `brave.brave`; asserts `ReadyToEnsure.id.to_string() == "Brave.Brave"`, and
    asserts the folded spelling is **not** what reaches the step.
17. `an_unpinned_install_whose_id_matches_a_different_package_fails_and_names_the_id_to_declare`.
18. `an_unpinned_install_that_is_not_in_the_index_fails_the_run`.
19. `an_ensure_becomes_a_winget_set_at_the_version_show_reported` — the routing
    arm; negative control is deleting it and watching the routing-bug message
    appear.
20. `ready_count_counts_a_ready_to_ensure`.
21. `a_stale_lock_entry_for_an_unpinned_package_warns_and_names_the_entry`, with
    its counterweight: a pinned package's entry warns about nothing.

### `src/update.rs` and `tests/update.rs`

22. `an_unpinned_package_gets_no_lock_entry_and_no_resolution_call` — the fake
    `WingetCmd` records its invocations; asserts **zero**.
23. `a_repeated_update_with_only_unpinned_packages_does_not_rewrite_the_lock` —
    the churn test. `wrote_anything()` false.
24. `a_package_that_becomes_unpinned_has_its_pin_dropped_and_the_lock_is_rewritten` —
    the other half; `wrote_anything()` **true**. These two are a matched pair and
    each is the other's control.
25. `a_named_update_of_an_unrelated_package_leaves_an_unpinned_packages_stale_pin_alone` —
    the scope-placement test, modelled on
    `a_named_scoop_only_update_does_not_revert_an_existing_winget_lock_entrys_canonical_case`.
26. `update_does_not_refresh_the_winget_index_when_every_declared_package_is_unpinned`.

### `src/adopt.rs` and `tests/adopt.rs`

27. `adopting_an_unpinned_package_records_ownership_and_writes_no_lock_entry`.
28. `adopting_an_unpinned_package_spawns_no_winget_call`.
29. `a_partial_write_after_an_unpinned_adopt_does_not_claim_pkg_lock_changed` —
    the `WriteLock(Result<bool>)` test, through the `write_in_order` seam like
    its two existing siblings, so it runs on Windows too.

### `src/render.rs`

30. `an_ensure_renders_as_an_unpinned_install_in_both_tables` — asserting the
    plan line has no version and the preparation line does.
31. `an_unpinned_change_line_reads_differently_when_a_pin_was_dropped` — both
    `Change::Unpinned` arms.
32. `render_adopt_names_the_unpinned_match_rule`.

### `tests/cli.rs`

33. End to end, hermetic (winget stripped from `PATH`, as that suite already
    does): a `pkg.toml` declaring one unpinned winget package refuses at the
    preparation stage rather than at `NotLocked`, and the message names the
    index lookup — proving the two failure paths are distinguishable from
    outside the binary.

### Gates

```
cargo test --all -- --test-threads=1
cargo clippy --all-targets -- -D warnings
cargo fmt --check
python3 scripts/check-citations.py
python3 scripts/check-ps1-style.py
```

Test names compared name by name against `cargo test -- --list`, before and
after, with the count stated as a set difference and never as a subtraction.

---

## 13. Documents

- **`README.md`** — a `### winget: declaring a package without managing its
  version` section, modelled on `### winget: naming the processes winget will not
  name`: the `pkg.toml` block, what it does and does not promise, the fact that
  no `pkg.lock` entry is written and why, and the `adopt` note for a package
  already installed. The existing bullet *"It will never downgrade"* gains a
  sentence pointing at it, because that bullet is where a user meets the problem.
- **`CHANGELOG.md`** — under Unreleased, as a behaviour change.
- **`docs/OPEN-ITEMS.md`**:
  - **Item 1** keeps its refusal — it is unchanged and still correct — and gains
    the other half: the refusal now has an answer, and what the answer does not
    cover (a *pinned* package that has moved ahead still refuses, still exits
    non-zero, and `dotpkg update` is still the fix).
  - **Item 8** loses `add` as its lead example's whole story. `add` is still
    unbuilt and still composes from `pkg.toml` + `update` + `apply` — for a
    **pinned** package. For an unpinned one it composes from `pkg.toml` +
    `apply`, with no `update` step at all, because there is nothing to resolve.
    That is a narrowing of item 8, not a closing of it. The recorded
    architecture-drift cost stays exactly as written: this design does nothing
    about drift.
  - **A new item: `--id`'s substring behaviour is claimed two ways and measured
    one way.** §4's conflict — §7 measured five `show --id <substring>` probes
    that all missed and concluded `--id` requires the whole id; three source
    comments and the changelog say a declared `OhMyPosh` matched
    `JanDeDobbeleer.OhMyPosh` on the same verb, with no measurement anywhere in
    `docs/`. Both refusals built on the second claim are safe either way (they
    refuse rather than act), so nothing is currently wrong — but the flag every
    winget resolution depends on is described two contradictory ways in the tree,
    and this design refuses option (b) on that basis.
  - **A noted asymmetry:** `[winget.opts]` refuses an entry for an undeclared
    package; `[scoop.opts]` accepts one and ignores it.

## Non-goals

- **Downgrading a winget package.** Unchanged. Item 1's refusal stands.
- **`pin = "none"` for scoop.** Refused above, with the reason.
- **`add`.** Unchanged, narrowed per §13.
- **Architecture drift.** Untouched.
- **Recording *what version* an unpinned package was installed at**, anywhere.
  That is a pin with a different name, and item 7 is why not.
