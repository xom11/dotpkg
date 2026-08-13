# Changelog

## Unreleased

Nothing yet.

## 0.2.0 — 2026-08-13

Since `v0.1.0` (`7ab9413`). **Nine behaviour changes: one new `pkg.toml`
surface, one deletion, six bug fixes and a new exit code.**

**Why this is a release and not just commits on `main`.** Until it, a build from
`main` and the published `v0.1.0` binary both answered `dotpkg 0.1.0`, so no
consumer could tell them apart — not by `--version`, not by any gate a calling
repository could write. `github.com/xom11/nix` cannot adopt `pin = "none"`, or
tighten its handling of exit 1, until a machine can be *known* to carry the new
binary. That is what the version number is for, and leaving it at `0.1.0` made
every fix below unusable downstream however correct it was.

**The headline for a caller: `[winget.opts] pin = "none"`, and exit 3.** Between
them they remove the two reasons a nightly `dotpkg apply` had to be treated as
"any non-zero means broken": a self-updating application no longer has to be
deleted from the declaration to stop failing the run, and a run whose only
outstanding item is an open application no longer looks like a failure.

**Six of the fixes are ways dotpkg could silently lose or withhold something** —
an ownership record, a usable pin, a warning about a running application, or the
truth about which version is installed — and losing any of them looked exactly
like working correctly.

**Two came from outside this project** — the trailing zero and exit 3 — from
the first dotfiles repository to call dotpkg rather than be managed by hand.
Being *called by* something is a different test surface from being run, and it
found what this project's own review had not. The same report also produced the
symlink warning below, and named `pkg.lock.bak`, which this tree had already
removed; the other five fixes came from this project's own audit.

### The behaviour changes

- **`[winget.opts] pin = "none"`: install it if absent, never manage its
  version.** The other half of the decision `docs/OPEN-ITEMS.md` item 1
  records. That item refuses to downgrade a winget package and the reasoning
  holds — uninstall-then-reinstall would put a nightly loop on every
  self-updating application — but nothing was ever built to say the thing a
  user of such an application actually means. Measured on `zenbook-a14` (ARM64,
  winget 1.29.280) on 2026-08-12: `Brave.Brave`, `Vivaldi.Vivaldi`,
  `Google.Chrome`, `Discord.Discord` and `Warp.Warp` all had to be **removed
  from the declaration entirely**, because each updates itself past its pin
  within days and the correct refusal then failed the calling module on every
  invocation. The hand-written PowerShell dotpkg replaced could express it; it
  only ever ensured presence.

  ```toml
  [winget.opts]
  "Brave.Brave" = { pin = "none" }
  ```

  **An unpinned package gets no `pkg.lock` entry.** That follows from item 7 —
  *"two sources of truth about permitted versions is how a tool starts lying"* —
  rather than being a shortcut: `pkg.lock` records what a declaration resolved
  to, and an unpinned one resolves to nothing. Absent, it is installed at
  whatever winget's index offers that day; present, it produces no line and no
  count, in either direction, forever; undeclared and owned, it is pruned like
  any other, since `Action::Prune` reads its version off the scan rather than
  the lock. `dotpkg update` writes nothing for it and no longer rewrites
  `pkg.lock` on account of it.

  Three things are worth naming because each was a live bug in a draft.
  `SkipReason::NotLocked` is untouched at all **81** of its reference sites: the
  planner's unpinned branch returns before the lock lookup, so such a package
  never reaches the rule that fails the whole run at exit 2, and the rule is not
  weakened by one line for pinned packages. `Update::wrote_anything` counts a
  dropped pin as a write and an unpinned steady state as not one — with neither
  half, five declared browsers rewrite `pkg.lock` on every `update`; with both
  halves excluded, a dropped pin is reported and never written, so the stale
  entry survives forever. And an id that resolves to a **different** id is
  refused rather than installed, which is correctness rather than tidiness:
  accepted, a declared `OhMyPosh` installs `JanDeDobbeleer.OhMyPosh`, the next
  run's scan misses the declared name, and the package is reinstalled on every
  run forever.

  `dotpkg adopt --backend winget` is the only way an already-installed unpinned
  package becomes prunable, since `apply` never installs a package that is
  present and so never comes to own one. It records ownership and nothing else,
  spawning no `winget` call at all.

  The install reuses the measured `set_argv` verbatim rather than inventing an
  argv no measurement covers, resolving the canonical id first with the same
  `show --id <declared>` call `update` already uses.

  **Verified on real hardware, and it refuted one of this changelog's own
  sentences.** Built and run on `zenbook-a14` (ARM64, winget 1.29.280) on
  2026-08-13 from `main` at `9c2f9e7`: an unpinned `Brave.Brave` already
  installed produced no line and exit 0 against an empty `pkg.lock`, where the
  same package pinned produced `NotLocked`; an absent `ducaale.xh` resolved to
  `0.26.2` out of winget's own index; `update` wrote no entry and reported
  `pkg.lock is already current -- not rewritten` on the second run; and a stale
  pin warned and then cleared. Nothing was installed or removed — every run
  stopped at `--prepare`, so **item 29's "no winget mutation has run anywhere"
  is unchanged.** Round in
  `docs/measurements-2026-08-13-phase14-winget-unpinned.md`.

  The same round settled the conflict that decided the argv question, **against
  the claim stated above under "A winget id that matches a *different* id"**.
  `winget show --id OhMyPosh` returns "No package found matching input criteria"
  on a machine where `JanDeDobbeleer.OhMyPosh` is installed, so `--id` requires
  the whole id and that entry's "substring filter" reasoning is wrong — measured,
  where it never was. The refusal it justifies is kept as defence rather than
  removed, and `docs/OPEN-ITEMS.md` item 30 is where the correction lives.

- **`pkg.toml.bak` and `pkg.lock.bak` are no longer written.** Both files are
  **committed**, so the user's own history already holds every version of them
  and `git checkout` recovers strictly more than a copy of the last one ever
  did; what the copies produced in practice was a permanently dirty
  `git status` beside files people commit. **`state.json.bak` stays**, because
  `state.json` is deliberately not committed — it is the truth of one machine —
  so nothing else can recover it, and it lives in the platform state directory
  where no version control is watching. Existing `.bak` files are safe to
  delete. The rule this leaves is in the README: a `.bak` is for a file nothing
  else can recover.
- **A trailing zero component is no longer read as a downgrade.**
  `plan::is_older` became `plan::version_order` and returns `std::cmp::Ordering`
  rather than `bool`, because a `bool` cannot say *the same version, spelt
  differently*: the two sides were compared ragged, lexicographic ordering made
  the longer one greater once the shared prefix tied, and `30.6.4.0` therefore
  read as ahead of a pin of `30.6.4`. Zero-extending the shorter side to the
  same width makes that `Equal`, and `Equal` is not a direction, so the pair
  produces no action at all. Found by an integrator rather than by this project,
  and measured on `zenbook-a14` (ARM64, winget 1.29.280) on 2026-08-12 with
  dotpkg being called *by* a real dotfiles repository rather than run by hand:

  ```
  ! winget JanDeDobbeleer.OhMyPosh 30.6.4.0 -> 30.6.4 (dotpkg will not
    downgrade a winget package -- run `dotpkg update`)
  ```

  winget's ARP version carries four components where winget's own index carries
  three, and it did not self-heal: `dotpkg update` re-pins the three-component
  spelling the index gives it, so the refusal came back on every run and floored
  the calling module to exit 1 every time. **The prerelease case is deliberately
  not closed by this** — `1.0.0-rc1` against a pin of `1.0.0` still classifies
  as a downgrade, and `version_order`'s doc comment carries that as a surviving
  residual. They are different bugs and only one of them is fixed.
- **Exit 3: everything worked, an app was open.** `apply` and `apply
  --prepare` now answer 3 when nothing failed, everything prepared, and *every*
  outstanding item is a package skipped because its own process was running.
  The README already said such a skip "is not a failure" while giving it a
  failure's exit code, and the cost landed on the first caller that was a
  program rather than a person: on a machine where `python`, `beckon` and
  `kanata` run essentially all the time, a fully resolved lock with nothing
  wrong reported `7 verified on disk, 0 failed, 1 held.` and exited 1 every
  night, so an automated caller had to treat 1 as success and lose every real
  failure. 3 is deliberately narrow — one package whose state could not be read
  anywhere in the same run makes the whole run 1, because closing a window does
  not fix a package dotpkg could not read. A caller that treats any non-zero
  code as failure is still correct. `floor_exit_code` became `apply_exit_code`:
  it no longer only floors.
- **A failed ghost reconciliation no longer discards the run's ownership.**
  `reconcile_ghosts` ran ahead of the run's only `State::save` with `?`, so the
  one `Err` the post-run scoop rescan can return took `main` out before the
  save *and* before the closing table was printed — leaving every package the
  run had just installed unowned, which the next run reads as unmanaged and no
  prune can ever reach. Ownership is now saved first, reconciliation is a
  warning rather than a return, and the ordering is pinned by a source guard,
  since the failure cannot be reached from a hermetic fixture: forcing the
  post-run scan to fail would fail the pre-run scan too, and `apply` refuses
  there first. Found by this project's own audit, not on hardware.

- **A winget id that matches a *different* id is refused instead of pinned.**
  `winget show` runs without `--exact` on purpose — that is what folds case on
  the way in. **This entry originally went on to say that omitting `--exact`
  also "leaves `--id` a substring filter, so a declared `OhMyPosh` matches
  `JanDeDobbeleer.OhMyPosh`", and that was measured false on 2026-08-13** —
  `show --id OhMyPosh` returns `NO_APPLICATIONS_FOUND` on a machine where
  `JanDeDobbeleer.OhMyPosh` is installed. `--id` requires the whole id. The
  fix below is unchanged and still correct; only its stated cause was wrong.
  See `docs/OPEN-ITEMS.md` item 30. `update` wrote the lock under
  the canonical id while `plan` looks the pin up under the declared name, so
  the two never met: `apply` refused the whole run at exit 2 with
  `Skip { NotLocked }`, and `update` rewrote the identical unusable lock every
  time it was run to fix it. There was no way out from inside dotpkg. Both
  `update` and `adopt` now fail that one package and name the id to declare
  instead; a difference of case alone still warns and is still recorded, which
  is what it always did.
- **An app directory with no manifest keeps its ownership record.**
  `Scoop::scan` skipped it silently — correct, since the ordinary cause is a
  half-finished install — but skipping made the name absent from the scan, and
  ownership reconciliation deletes any owned name a fresh scan does not
  mention. So a `Replace` whose uninstall succeeded and whose install failed
  had its ownership dropped in the same output that reported the failure, and
  for an `Adopted` package that is unrecoverable. `Scan` gained a third
  category, `residual`: on disk, invisible to the planner. Not folded into
  `opaque`, because `opaque` means "do not act" and `Install` is exactly the
  right action for a half-finished install.
- **The unguarded-winget-change warning is no longer withheld from a downgrade
  dotpkg only guessed at.** The exclusion read `Action::Downgrade` as proof
  that winget would refuse and nothing would reach the disk. That holds only
  when dotpkg could tell which direction the change is, and
  `version_order`'s own doc comment says it cannot whenever a version carries
  anything but digits and dots. So for `1.0.0-rc1` against a pin of `1.0.0`
  winget saw an ordinary upgrade and performed it, while the one warning that
  would have let a user protect a running application was suppressed — on the
  majority class of winget package, the 32 of 36 ids on a14 the path signal
  cannot see either. Purely numeric pairs are still excluded, so the measured
  Chrome refusal stays silent; the verb is "change", because the direction is
  precisely what is not known.

- **A winget row whose Name is not ASCII is cut at the column, not at the
  byte.** `parse_list` took its column offsets from the header and applied them
  to data rows as *byte* offsets. winget pads to **character** columns:
  measured on `list-full.txt` line 67, whose two U+00AE make it 220 bytes
  against 218 characters, the `Id` column's content begins at character 64 and
  byte 66 while the header puts `Id` at 64. Every field on such a row was
  therefore cut two bytes early. That row survives because its columns are wide
  and `trim` eats the shift — but the narrow one-row table `winget list -e --id
  <id>` prints has a single space of slack, and that is the table
  `winget_verdict` parses to decide whether a mutation happened. A truncated
  `Id` there reads as "not the package I asked about", so `apply` would report
  a change that never occurred, and the winget path fence would be dark for the
  same package. Offsets are now translated through each line's own
  `char_indices`. `floor_char_boundary` is deleted: it existed to keep a
  byte-offset cut from panicking mid-character, which was defending the wrong
  thing — the byte offset was never the right place to cut. Its mutation record
  in `docs/OPEN-ITEMS.md` §C is marked as describing a function that is gone.

### One thing documented rather than changed

- **Do not symlink `pkg.toml`, `pkg.lock` or `state.json`.** Every write is
  temp-then-rename, and `rename` replaces the *path* — so a symlink at it is
  destroyed and the new contents land in an ordinary file where the link used
  to be, while the file the link pointed at never sees the write and
  `git status` stays clean throughout. Someone wired a dotfiles repository up
  through a symlinked `pkg.lock`, having read an `fs::write` elsewhere in the
  tree and reasonably expected overwrite-in-place, and the repository silently
  stopped receiving updates. The atomic write is correct and has not been
  weakened; the README now says outright not to symlink these files, and
  `a_save_over_a_symlink_replaces_the_link_itself` pins the behaviour so that
  sentence cannot quietly stop being true.

### The claims got stronger without the features moving

- **`apply` has now run from the published binary**, sha256 `9daeae0c…` and not
  a rebuild of it, installing and then pruning a real package on real hardware
  and verifying both on disk. Until this it had only ever run `status`, which
  mutates nothing. The prune was preceded by its own counterweight: the same
  command with the flags withheld refused at exit 2 and left the package
  installed.
- **The design's third test layer exists at last.** "Real scoop in a throwaway
  `$env:SCOOP` on a Windows runner" was specified on 2026-08-08 and never built;
  `tests/cli.rs` was never it, being hermetic by design. A CI job now installs
  scoop into a throwaway root, builds a bucket that is a real git repository,
  and serves a real archive over HTTP so scoop does its own hash check.
- **The version change closed twice, and the two are not interchangeable.** In
  CI, the `scoop-integration` job now publishes a second bucket commit at 1.0.1
  — a different archive, hash and url, so scoop cannot satisfy the install from
  cache and skip the download half — and applies it, asserting the plan
  presented it as a version change rather than as an install. From the published
  binary, on real hardware, in **both directions**: `jq` 1.8.2 → 1.8.1 and
  1.8.1 → 1.8.2, each verified on disk, with the shims and the ownership
  surviving the window in which the package is absent. That window — scoop's
  uninstall-then-install gap, the most dangerous path this tool has — had never
  been watched by anything before.
- **A winget mutation has now run outside Phase 4b, and only half of one.**
  On 2026-08-13 `dotpkg apply --yes` installed `ducaale.xh` 0.26.2 on a14 as an
  unpinned package — `done winget ducaale.xh verified on disk`, confirmed by a
  fresh scan, with no `pkg.lock` file written at all and ownership recorded.
  The matching **removal was refused at exit 2** by the elevation pre-check, the
  session being elevated and the package user-scope; winget itself then returned
  `0x8A15007D` for that same uninstall, so the refusal was right. `WingetStep::
  Remove` still has no evidence outside Phase 4b, and neither half has
  release-binary or CI evidence. See `docs/OPEN-ITEMS.md` item 29 and
  `docs/measurements-2026-08-13-phase14b-winget-mutation.md`.

### Code

- **`execute::Mutates`** names the write half of a backend, which had no
  contract at all: the two per-backend seams were threaded through `execute` and
  `run_step` as a hand-written pair of parameters, so a third backend meant a
  third parameter at 27 call sites. `execute::Backends` carries both sides in one
  value. Measured name by name: identical 658-name test sets before and after,
  0 lost, 0 added.
- One test added, joining `Backend::scan` to the `[winget.guard]` opaque warning
  — two halves that were each pinned and never connected.
- **The planner's purity guard stopped carrying two copies of its own
  allowlist.** `the_planner_source_performs_no_io` checked fully-qualified
  `std::` paths against a hardcoded string rather than against the `ALLOWED`
  list beside it, so admitting one more pure import — `std::cmp::Ordering`, for
  the fix above — made the guard fail for a reason that had nothing to do with
  purity. It now reads the `std::` entries out of `ALLOWED`, and asserts that it
  found some, because a guard that vouches for nothing rejects everything.

### Record

- **`docs/OPEN-ITEMS.md`** is the one live list, keeping every item under the
  number it already had. Items that closed outright no longer carry their prose:
  23 and 27 are rows in its own "Closed, with what closed it" table, which
  exists so a reference to one of those numbers still finds its resolution.
- Twenty documents removed, about 28,200 lines: the phase narratives and then
  all eight task-breakdown plans. Every one still reads with `git show`; both
  waves name their commit in `OPEN-ITEMS.md`.
- **`LICENSE` now exists.** `Cargo.toml` had claimed MIT since the first commit
  with no such file in the tree.
- A flake was added and removed the same day; the reasoning is in the README.
- **`flake.lock` was left behind by that removal and is now gone too** — a lock
  file for a flake that does not exist, in a project whose whole thesis is that a
  lock records what was actually resolved.

**The caveat this section carried while it was unreleased is now spent.** It
said the `version_order` fix and the purity-guard change were in the working
tree and not yet committed. Everything in this section is committed and tagged
`v0.2.0`; nothing here is a prediction.

## 0.1.0 — 2026-08-12

First release. 321 commits over five days, and the whole of it is one idea:
**declare Windows packages in a file, resolve them into a lock, and let a tool
bring the machine to that state without ever guessing.**

### What it does

Four commands, both backends (winget and scoop) in each:

- **`status`** — prints the plan it would execute and changes nothing. No
  install, no uninstall, no network.
- **`apply`** — brings the machine to what `pkg.toml` and `pkg.lock` describe.
  Stages and fetches everything first, asks once, then mutates — and **verifies
  every result afterwards rather than believing the package manager's exit
  code**, because neither manager's exit code says whether anything happened.
- **`update`** — moves the lock forward, not the machine.
- **`adopt`** — records an already-installed package as dotpkg's, so `prune`
  can reach it.

### The parts worth naming

- **A real lock file.** `pkg.lock` records what each declaration resolved to —
  for scoop, the bucket commit as well as the version — so the same `pkg.toml`
  produces the same machine.
- **Prune can never reach a package dotpkg did not install.** Ownership lives
  in `state.json`, written when dotpkg installs something and never inferred.
- **A running-process fence.** dotpkg refuses to replace or remove a package
  whose process is alive, by two independent signals: a live process running
  out of the package's own directory, and a name match. `[winget.guard]` lets
  you name the processes a winget id really runs, and **dotpkg now tells you
  which entry you are missing** rather than failing silently.
- **Exit codes that mean something.** `2` is "refused before anything was
  attempted; nothing changed."

### What is measured, and what is not

This project's rule is that a claim carries the measurement that settles it,
and the same rule applies to the release. **See the "Verified on" section of
the README for the exact scope**, which is narrower than the feature list
suggests: one real Windows machine, one architecture, one winget version.

Every number in the documents under `docs/` names the machine and the tree it
was taken on. Where something ships structurally verified and live-unverified,
it is numbered in a still-open list rather than described as done.

### Known gaps, decided rather than forgotten

- **Downgrading a winget package** — decided against, not deferred. Measured:
  `winget install --version <older>` only ever moves a package up.
- **Dependency handling** — a winget install that pulls in a second package
  leaves that package unmanaged. Measured and reported, not silently absorbed.
- **`add`** — `pkg.toml` plus `update` plus `apply` composes to the same thing.
- **Chocolatey** — nothing beyond the two backends.
