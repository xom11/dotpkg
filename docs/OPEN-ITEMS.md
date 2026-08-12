# Open items

The live list. Everything here is either unmeasured, measured-and-still-open, or
a decision recorded so it is not rediscovered as a gap.

Item numbers **1–21** are the numbers `docs/phase5-notes.md` used, and they are
kept even where an item is closed, so a reference written against the old
numbering resolves to its answer rather than to a hole in the sequence. Items
**22 and up** are new here.

## How to read a reference to a record that is no longer in the tree

Twelve documents — six dogfood records and six phase notes, about 8,700 lines —
were removed on 2026-08-12. **They are not gone; they are not in the working
tree.** Every one of them exists at `07dd86b`, the commit immediately before the
removal:

```console
git show 07dd86b:docs/phase5-notes.md
git show 07dd86b:docs/dogfood-phase4-2026-08-10.md
```

Removed: `dogfood-2026-08-08.md`, `dogfood-phase2a-2026-08-08.md`,
`dogfood-phase2b1-2026-08-08.md`, `dogfood-phase2b2-2026-08-09.md`,
`dogfood-phase3-2026-08-09.md`, `dogfood-phase4-2026-08-10.md`,
`phase2-notes.md`, `phase2b-notes.md`, `phase3-notes.md`, `phase4-notes.md`,
`phase4b-notes.md`, `phase5-notes.md`.

**What was deliberately not done:** the surviving `docs/specs/`, `docs/plans/`
and `docs/measurements-*` documents still name those files in their prose, and
those mentions were **left exactly as written**. They are frozen records; a
sentence that was true about the tree it was written against stays true, and
re-pointing it would falsify it. This section is how such a mention resolves.
The only reference that was rewritten is one `file:line` citation that would
otherwise have failed `scripts/check-citations.py`, and it is marked at its site.

**What survived the removal:** every `docs/measurements-*` document, all of
`docs/specs/`, all of `docs/plans/`. The measurement documents are what the
README and the code cite as evidence; the phase notes were the narrative around
them, and this file is the part of that narrative that is still live.

---

## A. Decisions, not gaps

Recorded so they are not reopened as if nobody had looked.

- **1. Downgrading a winget package.** Measured: `winget install --version
  <older>` only ever moves a package up. The alternative — uninstall then
  install — would put a nightly uninstall-and-reinstall loop on every
  self-updating application. dotpkg prints the refusal and tells you to run
  `dotpkg update`.
- **7. `winget pin`.** Not used. Two sources of truth about permitted versions is
  how a tool starts lying.
- **8. `add`, architecture drift, same-version re-pin, locking against two
  concurrent dotpkg runs, Chocolatey.** All unbuilt. `add` composes today from
  `pkg.toml` + `dotpkg update <pkg>` + `dotpkg apply`.
- **2 (the rejected fix).** A hardcoded `WINGET_HELPERS` list was rejected
  explicitly: it would *exclude* dependency-installed packages from `Unmanaged`
  rather than count them, which is less honest than collapsing a line. What
  remains open about item 2 is below.
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

## C. Coverage and mutation debt

- **13. Inherited from Phase 4, none of it in a later phase's scope:**
  `plan_backend`'s unconditional `Arch::as_scoop()`; the design's "a new backend
  slots in without touching the planner" promise being half true (see item 24);
  `verify.rs`'s `NotFound`-idiom guard; `floor_char_boundary` and the
  missing-`Version` refusal branch being untested defensive code; no fixture
  pairing a plain `show` with `show --versions` for the same package;
  `resolve_installed`'s `fell_back_to_tip` warning path being untested.
- **`src/sys.rs` needs three mutation runs, not one.** macOS, elevated Windows,
  and ordinary Windows — because each platform is blind to the other's `cfg`
  arm, and **no two of them together kill all six mutants**. A Windows-only run
  closes three and silently reopens the two that live in the
  `#[cfg(not(windows))]` arm. The documented invocation is also wrong in two
  ways: `cargo mutants -- --include-ignored` does not reach libtest (it needs the
  two-`--` form), and `cargo install cargo-mutants --locked` cannot build on a14
  at all — it pins `winapi` 0.3, which fails on `aarch64-pc-windows-msvc`.
- **16 surviving mutants in `src/backend/winget.rs`**, all in Phase 4 code:
  `floor_char_boundary` 7, `parse_list` 7, `parse_versions` 1, `RealWinget::run`
  1. Confirmed by two runs of different scope agreeing on 16.
- **2 surviving mutants in `src/backend/winget_exec.rs`**, inside
  `RealWingetMutator::run`. Covering them means spawning a real `winget.exe` from
  the suite, which this project does not do.
- **2 survivors no phase recorded until a file-scoped run found them:** one in
  `src/main.rs` (`replace > with <`, distinct from the accepted equivalent
  mutant on the `outstanding_skips` check) and one in `src/backend/scoop.rs`
  (`NotFound` match guard replaced with `true`, inside `<impl Backend for
  Scoop>::scan`).
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
- ~~**23. `apply` has never been exercised from a release binary.**~~ —
  **closed 2026-08-12** (`docs/measurements-2026-08-12-phase8-release-apply.md`).
  The published artifact, sha256 `9daeae0c…` and no rebuild of it, installed and
  then pruned a real package on a14 and verified both on disk: `jq` absent
  before, present at the pinned 1.8.2 after, `jq --version` answering
  `jq-1.8.2`, ownership written and then released to `{ "scoop": {} }`, and the
  other 31 packages untouched. The mass-prune guard was proved to be in the way
  by a counterweight run first — same command with the flags withheld refused at
  exit 2 and left the package installed.

  **What that does not buy, and the difference matters:** only `Install` and
  `Remove` were exercised. **A scoop `Replace` — the uninstall-then-install
  window, the most dangerous path this tool has — still has no evidence from a
  release binary**, and neither does any winget mutation. Both are now numbered
  as item 29 rather than folded into a closed item.
- **29. The version-change path has never run from a release binary**, on either
  backend. New 2026-08-12, split out of 23 so that closing 23 does not read as
  covering it.
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

  **What it still does not cover:** a version change. The job installs and
  removes; a scoop `Replace` needs a second manifest in the bucket and is the
  obvious next step for it. See item 29.
- **26. `dotpkg` cannot install itself.** The design's phase 5 said release
  *"through the existing scoop bucket"*. The release is GitHub binaries plus
  `SHA256SUMS`; there is no scoop manifest, so the tool is not distributed by the
  mechanism it advocates.

## F. Found on real hardware 2026-08-12, not looked for

All three from `docs/measurements-2026-08-12-phase8-release-apply.md` §7.

- **27. `update` and `apply` leave `.bak` files beside what they rewrite, and
  nothing says so.** Measured: after one round, `pkg.lock.bak` held the previous
  lock and `state.json.bak` the previous state. Keeping the prior content is the
  durable-save path working as intended; **leaving the files** is undocumented
  and uncleaned, so a user running these commands in their dotfiles repository
  accumulates them next to files they do commit. Decide whether they are a
  feature (then document and name them) or debris (then remove them), rather
  than leaving the answer to whoever notices first.
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
