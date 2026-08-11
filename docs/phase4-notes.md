# Carried forward out of Phase 4

Findings from building Phase 4 (generalising the pipeline, then adding the
winget backend) that Phase 4b must handle, plus everything the whole-branch
verification found by *running* the branch rather than reading it.

Every item here was produced by mutation testing, by a control that actually
fired, by the Windows run, by the dogfood, or by a reviewer reproducing
something. None is reasoned-only; where an item **is** reasoned-only, it says
so. `docs/phase2-notes.md`, `docs/phase2b-notes.md` and `docs/phase3-notes.md`
still hold the earlier items; this file does not repeat them, except where a
Phase 3 "still open" item was carried into Phase 4's own scope and its status
changed here.

## Read this first

**`Running::covers` is weaker for winget than for scoop, and nothing notices
today only because nothing acts on winget yet.** `Winget::scan`'s `Installed`
rows always carry `bins: Vec::new()` — there is no winget-side manifest to
read executable names from — so of `Running::covers`'s three signals (package
directory, process name, declared executables), only the first two can ever
fire for a winget package. A running winget process whose binary lives outside
any name-matchable path and whose package directory isn't the one `sysinfo`
reports would be invisible to the running-process guard. Recorded, not fixed,
because `plan()` does not act on winget packages yet and the guard's only
consumer today is the report line, not a mutation decision — this is the
condition to clear **before** an executor exists, not after.

**The winget executor is blocked on a measurement that has no throwaway
root.** Phase 3's `--offline`/staging-root design and this phase's own
`Scoop::stage` both lean on `$SCOOP` being redirectable to a fixture
directory nobody depends on. Winget has no equivalent — every install lives
under a single, real, per-machine registry and `%ProgramData%`/`%LOCALAPPDATA%`
tree, so a mutation experiment ("what does `winget install` actually do to a
disagreeing pin") would touch the real machine, which every phase since Phase
2a has refused to do on principle. This is the actual reason Phase 4 stops at
scan/plan/lock/report (`Capability::ReportsOnly`) rather than wiring an
executor: not scope discipline for its own sake, but the absence of a safe
place to measure one.

**`verify_round_trip`/`verify_round_trip_winget` compare the parsed `Config`,
not the text — so `pkg.toml`'s "byte-identical except the added line" promise
is unguarded, and the guard that exists to protect the user's hand-written
file cannot see the class of bug that broke it.** The comment-loss bug (a
same-line trailing comment on a `[winget]`/`[scoop] packages` array's last
element, silently dropped on append — found live by this task's own dogfood,
Q5) is invisible to both round-trip guards by construction: they re-parse the
edited text with `config::parse` and compare `Config` values, and `Config`
has no field for comments. **Fixed in Task 16** (`append_to_packages_array`,
commit `6683dd3`) — but the guard's blindness is structural, not
coincidental, and the next comment-shaped bug in `config_edit.rs` will sail
through it exactly the same way. Phase 4b should decide whether that is
acceptable (the guard's stated job was never "preserve every byte", only
"preserve the declared packages") or whether the promise in this project's
own design language needs a text-level check alongside the semantic one.

**Three winget-scan failure modes are one family, safe only because winget
is `Capability::ReportsOnly`, and nothing said so in one place until now.**

`scan_or_warn`'s own doc comment (`src/backend/mod.rs:198-202`) justifies
safety only in the prune direction — "`plan()`'s prune loop only ever
iterates `installed`, so an empty scan can never fabricate a prune." That
half is correct. It says nothing about the other direction: a declared,
locked, installed and **converged** winget package renders as
`Divergence::Install` ("would install `<version>`") after any scan that
comes back empty, which is a divergence that does not exist — the package
was never actually missing, `scan()` just could not confirm it this run.
Today that is *only* a false report line, because `apply` still cannot
install anything for winget (`Capability::ReportsOnly`, again) — a user
reading `apply`'s output is told dotpkg wants to install something it does
not, but nothing actually happens. The day `Capability` becomes `Acts`, the
same rendering, unchanged, is dotpkg installing a package that is already
there. Over-*reporting* today, over-*acting* tomorrow, from the same line.

`Winget::scan` widens how often that empty scan happens: it treats **every**
`WingetCmd::run` error as "winget is absent" (the trait's `anyhow::Result`
erases `io::ErrorKind` before `scan` ever sees it, unlike `Scoop::scan`,
which distinguishes `NotFound` from other error kinds), so a broken or
permission-denied `winget.exe` — present, not absent — hits the same
empty-`Scan` path as a genuinely missing one, and gets the same false
`Divergence::Install` treatment above.

Recorded, not fixed, because `plan()` does not act on winget packages yet
and the only consumer of a wrong `Divergence::Install` today is the report
line, not a mutation decision — this is the condition to clear **before** an
executor exists, not after.

**`plan_backend` calls the scoop-specific `Arch::as_scoop()` unconditionally**
(`src/plan.rs:315`, `:375`), regardless of `view.backend`. Harmless today
because scoop is the only `BackendView` that ever carries per-package arch
opts (winget's is always `empty_opts`, per `plan.rs`'s own comment on the
`backends` array) — but flagged as a forward hazard since Task 5 and never
revisited through Task 14: if winget, or a third backend, ever grows
per-package arch options, it would silently inherit scoop's `"64bit"`/`"arm64"`
vocabulary rather than refusing or asking for its own.

**The design's own promise is now half true, and that needs to be said
plainly rather than left implied.** The approved design this phase carries
forward from quoted, at `design.md:95`: *"the backend trait exists from v1 so
choco slots in without touching the planner."* Phase 4's own design doc
states outright: *"That was never built... Phase 4 is where the promise is
either made true or withdrawn."* After this phase: `plan()` does run one pass
per backend (`src/plan.rs:522-551`, the `backends` array plus a loop, not a
hardcoded scoop-then-winget duplication) — that half is real. But
`Capability` still exists specifically to let `plan_backend` special-case
`ReportsOnly` against `Acts`, and `as_scoop()` above is still called
unconditionally. `plan_backend` itself is generic -- one function, dispatched
per `BackendView`, with no backend-specific branch inside it -- but `plan()`
is not: it still enumerates backends by hand. Its `backends` array
(`src/plan.rs:531-548`) is a hardcoded two-element literal naming
`declared.scoop`/`declared.winget`/`lock.scoop`/`lock.winget` directly, and
both `Config` (`src/config.rs:8-11`) and `Lock` (`src/lock.rs:32-35`) are
per-backend structs, not maps a loop could walk. A third backend needs a code
change to `plan()` itself -- a new array element naming its own `Config`/
`Lock` fields, and those fields added to both structs first -- on top of
inheriting scoop's arch vocabulary unless someone remembers to fix that
first, and needing a `Capability` value decided for it by a human, not
inferred. **Half true, not fully
withdrawn, and a phase that overstated what it achieved is how the next one
inherits a surprise** — recording it here so Phase 4b does not have to
rediscover it by reading the diff.

## Verification

### macOS suite

`cargo test --no-fail-fast`: **509 passed, 0 failed**, on the tree that
ships (commit `6683dd3`). `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings` both clean; `cargo build
--all-targets` produces zero warnings. 505 of the 509 are the phase's
steady-state total; the other 4 were added by Task 16's own fix wave (two
`config_edit.rs` comment-preservation tests, two `render.rs` column-width
tests).

### Windows suite — two runs, because the first ran on a tree that then changed

**First run**, on the tree Task 16 started verifying (`12d9ba8`): **502
passed, 1 FAILED**. The failure —
`backend::scoop::tests::a_root_that_needs_no_prefix_stripping_is_kept_as_
canonicalize_returned_it` — was a genuine, unpredicted finding: the test
asserted `resolve_root(d) == canonicalize(d)` (the **unstripped** value),
which only ever held on macOS because `canonicalize` there never adds a
`\\?\` prefix. On real Windows it always does, and `resolve_root` correctly
strips it — so the test was checking the opposite of what the function
exists to do. Nothing was changed to make it pass at the time; it was
reported, matching the rule this project extracted from Phase 3 (the suite
runs on the tree that ships, and nothing may be changed to make Windows pass
in the same round that measures it).

That test was Task 2's own, kept in the plan after Task 2 had already found
it could not discriminate on macOS and added the `rewritten` pure-function
seam specifically because of that limitation — the original assertion
survived alongside the new seam rather than being replaced. **This is the
best vindication of the Windows-run rule in the whole phase**: a test that
passed vacuously on the only platform this project's own machines run on,
and was actively wrong on the one platform this tool ships for, caught only
by actually running the suite there.

**Fixed in the same task** (commit `6683dd3`): the assertion replaced with
`resolve_root_strips_any_extended_prefix_and_still_names_the_same_directory`,
which checks (1) the output never starts with `\\?\` and (2)
re-canonicalizing the output resolves to the same directory `canonicalize`
did directly — both platform-true by construction. **Deliberately not
`#[cfg(windows)]`/`#[cfg(unix)]`-gated**: a platform-gated version would have
hidden exactly this defect again, green on whichever platform never
exercises the interesting branch.

**Second run**, on the fixed tree (`6683dd3`), fresh tarball, CRLF
re-verified: **507 passed, 0 failed.** Full name-by-name cross-reference of
all 509 source-level `#[test]` names against every `test <name> ...
ok|FAILED` line the run printed found **zero discrepancies** beyond the two
predicted `#[cfg(unix)]` exclusions
(`tests/adopt.rs`'s `a_failed_last_write_leaves_a_prefix_that_plan_does_
nothing_about`, `tests/scoop_scan.rs`'s `a_root_reached_through_a_symlink_
still_matches_running_processes`). 509 − 2 = 507; **the delta from macOS is
exactly those two tests and nothing else**, on both runs — the first run's
extra, unpredicted failure was a third divergence found by running the
suite, not hidden by a matching total.

### CRLF fixture survival

Verified on **both** Windows rounds, independently: `tests/fixtures/winget/
list-full.txt` — **30958 bytes, 143 `\r\n` pairs** — identical on the source
side and on the far side each time. Transfer method: `tar` of `Cargo.toml`,
`Cargo.lock`, `src/`, `tests/` (never `target/`, never `.git/`), moved with
`scp`, never anything that rewrites line endings.

## Mutation testing

**Three runs were needed and two were discarded, for three different
causes.** This is worth recording as its own finding: contamination is not
one failure mode, and treating it as one cost real time twice.

1. **Run 1 (14 timeouts): a concurrent workload.** An agent (this task, mid
   Windows-verification work) briefly started `cargo test` on the same
   machine while `cargo mutants -j 3` was running. Disclosed by that agent,
   not detected by the measurement itself. Discarded.
2. **Run 2 (22 timeouts): I/O starvation.** The boot volume had **3.3 GB
   free** (79% used) while `cargo-mutants`' four parallel temp build trees
   held roughly 4.8 GB on it, on top of a 2.5 GB `target/` in the repo
   itself. Found because the user asked whether storage would run out — the
   numbers were about to be reported as clean before that question was
   asked. Timeouts went 0 → 22 between mutant 465 and 573, tracking disk
   pressure rather than any code change. Fixed by moving `TMPDIR` to a
   volume with 1.6 TB free (deliberately **not** `CARGO_TARGET_DIR`:
   `cargo-mutants` builds each job in its own copied tree, and a shared
   target directory would break that isolation). Discarded.
3. **Run 3 (71 timeouts, the numbers below): macOS itself.** Measured with
   `ps` while the timeout count was climbing: `syspolicyd` at **147.9% CPU**
   and `mds` at **32.8%**. `cargo-mutants` creates hundreds of fresh
   binaries in temp trees; Gatekeeper signature-checks and Spotlight indexes
   every one of them. **The mutation run manufactures its own competitor.**
   Not another workload (run 1's cause) and not disk exhaustion (run 2's
   cause; disk held at 19–20 GB free throughout this run). This is the run
   whose numbers are reported below.

### A correction to `docs/phase3-notes.md`'s own framing

Phase 3's notes say *"a timeout is the only outcome CPU pressure can
manufacture"* — true, but it does not follow that **every** timeout **is**
manufactured, and this phase made exactly that error mid-run, calling all of
runs 1 and 2's timeouts contamination without checking each one. The real
discriminator is not the count, and not which run it came from: it is
**whether that specific mutant can hang at all**.

- **Can hang, confirmed:** `verify.rs:121`, `normalise`'s `i += 1` → `i *= 1`
  in the non-CRLF branch of its byte-copy loop (with `i` starting at 0,
  `0 * 1 = 0`, so `i` never advances and the `while i < b.len()` loop never
  terminates); `verify.rs:124`, `while out.last() == Some(&b'\n') { out.pop();
  }`'s `==` → `!=` (once `out` is empty, `out.last()` is `None`, and `None !=
  Some(&b'\n')` stays true forever, so the loop keeps calling `.pop()` on an
  empty vector without end). **Both reappeared in the same two places across
  all three runs** — genuinely hanging mutants, not noise, and the reason
  `verify.rs` shows up in every run's timeout list regardless of what else
  was contaminating it.
- **Cannot hang, by construction:** any mutant that replaces a whole
  function with a constant — `uninstall_argv -> vec![]`, `tail -> String::
  new()`, `strip_cr -> vec![""]`, `found_id -> None` are the ones named
  directly. A pure function returning a fixed value cannot loop; its tests,
  which run in microseconds, cannot time out on their own. **17 of run 3's
  71 timeouts are of this shape** and are therefore starvation, not a
  product of the mutation itself.

### The final numbers

**669 of 672 mutants tested** (stopped three short by decision — the last
three were all 120-second timeouts):

```
caught 518   missed 9   timeout 71   unviable 72
```

**`caught`, `missed` and `unviable` are valid.** CPU pressure can only
manufacture a timeout; it cannot turn a genuinely failing test into a
passing one, so these three columns describe the code, not the machine that
measured it. **The `timeout` column is unresolved** — the two `verify.rs`
mutants above are known-real hangs; the other 69 (71 minus those 2) have not
been individually re-run to a clean verdict and must not be reported as
either survivors or as closed. That 69 **includes** the 17
confirmed-cannot-hang ones: knowing a timeout was starvation rules out
"the mutation loops forever", but it does not supply the `caught` or
`missed` verdict that mutant still lacks. An earlier version of this
sentence read "71 minus the 17 confirmed-cannot-hang", which computes to 54
and contradicted the 69 beside it. Re-running just
the `timeout` set on an otherwise-idle machine, with the `TMPDIR` fix from
run 2 already in place, is the concrete next step — not another full run.

**Timeouts by file:** `winget.rs` 55, `scoop.rs` 14, `verify.rs` 2. The
concentration in `winget.rs` tracks the file's size and its recency (every
line of it is Phase 4), not a special property of winget mutants — most of
that 55 is very likely starvation given only 2 of the 71 timeouts overall
are confirmed hangs and neither is in `winget.rs`, but this is inference from
the shape, not a re-measurement, and is recorded as such.

### The 9 missed, triaged

**Already adjudicated, carried from earlier phases:**

- **`bucket.rs:99`** — `tip`'s success guard. Accepted **in Phase 3**, with a
  measured reason: under the mutation, both arms produce the same `rev` and
  the same `Some`-ness: only the wording of the `stale` flag differs, and
  nothing downstream reads that wording differently in a way any test
  exercises. Unchanged in Phase 4; not revisited.
- **`verify.rs:146`** — the `Err(e) if e.kind() == std::io::ErrorKind::
  NotFound => return Err(Disagreement::HalfInstalled { .. })` idiom.
  `docs/phase3-notes.md`'s "Still open" item 4 named three instances of this
  exact pattern (`Err(e) if e.kind() == NotFound => <benign default>`):
  `lock.rs:99` was closed in Phase 3; `verify.rs:146` and `backend/scoop.rs`'s
  manifest-read guard were not. **`backend/scoop.rs`'s instance was closed
  in Task 4 of this phase** (verified directly: `cargo mutants --file
  src/backend/scoop.rs --regex "scoop.rs:275:"` — the guard's line after
  that task's own edits — found 3 mutants, all 3 caught, `missed.txt`
  empty, per `task-4-report.md`). **`verify.rs:146` is now the last of the
  three still open**, and remains this phase's explicit non-goal (verify.rs's
  job is comparing what is on disk against what was asked for, not
  distinguishing every possible I/O failure kind from a benign one — the
  same reasoning that left it open in Phase 3 still applies).

**Known untested defensive code, not re-litigated:**

- **`winget.rs:43` ×2 and `:46`** — `floor_char_boundary`, the guard that
  keeps a column slice from splitting a multi-byte UTF-8 character. Task 9's
  own review already flagged this as a no-op on every ASCII fixture — every
  captured winget id and every test package name in this suite is ASCII, so
  the function's interesting branch (a column boundary that lands inside a
  multi-byte character) has never been exercised, by any fixture, in any
  task. Not closed here: constructing a real non-ASCII winget package name
  fixture is a small, separable piece of work with no dependency on
  anything else in Phase 4b, and is better done deliberately than folded
  into this survivor triage.

**Genuinely new, each given its own ruling — three gaps, one accepted:**

- **`apply.rs:384` — GAP.** `outstanding_skips`'s filter,
  `if is_outstanding(reason)`, mutated to `true`. Task 4's own fix added a
  direct unit test for `is_outstanding` in isolation
  (`is_outstanding_floors_running_opaque_and_reported_only_but_not_not_
  locked`), and that test's own comment already explains why the guard *at
  its use site* cannot be reached through the real pipeline: `classify()`
  maps `SkipReason::NotLocked` to `Intent::NotLocked`, and `prepare()` turns
  that into `Outcome::NotLocked`, never `Outcome::Skipped` — so the one
  `SkipReason` the guard would ever exclude never reaches
  `outstanding_skips`'s match arm via `prepare()` at all. But
  `outstanding_skips` is `pub`, and `Preparation`/`Prepared` are
  independently constructible — this codebase's own test style builds them
  directly throughout `render.rs` and `apply.rs` (not exclusively through
  `prepare()`), and `is_outstanding`'s own doc comment stresses it is
  "exhaustive on purpose, with no wildcard arm" specifically so a future
  `SkipReason` variant's floor-or-not answer is "a decision made here, not
  an oversight this match's shape would let slip through." A guard built
  that deliberately, with zero coverage at its one call site, is a gap by
  the project's own stated intent for the code — not equivalent, because a
  caller that does not go through `classify()` (a test, or a future
  non-`prepare()` caller) can and does construct the excluded shape. Closes
  with one test in the same style as the existing paired sibling: build a
  `Preparation` whose `Prepared` list includes an `Action::Skip{reason:
  NotLocked}` paired directly with `Outcome::Skipped{why}`, and assert it is
  absent from `outstanding_skips()`'s result.
- **`adopt.rs:468` — GAP.** `adopt_one`'s `scan.installed.iter().find(|i|
  i.backend == SCOOP && &i.name == name)`, `&&` mutated to `||`. Traced
  precisely: `run_scoop` calls `Backend::scan` **once**, before the
  per-name loop (`src/adopt.rs:206`), so a multi-package `adopt` call sees
  the same `scan.installed` list for every name in the batch. Under `||`,
  `.find()` would match the **first** scoop-backend entry in that list
  regardless of name, once any entry with `backend == SCOOP` precedes the
  actually-requested one. The one existing test with more than one installed
  scoop package,
  `adopting_two_packages_in_one_command_does_not_lose_the_first`, adopts
  `aichat` then `widget` from a shared two-entry scan — exactly the shape
  that could expose this — but only asserts that `lock.scoop`/`declared.
  scoop.packages`/`state.ownership` each **contain a key** for both names
  and that ownership is `Adopted`; it never checks the **content** (commit,
  version) of the second package's recorded pin. Because `lock.scoop.insert
  (name.clone(), pin)` (`src/adopt.rs:250`) keys the write by the loop's
  own `name`, not by `inst.name`, a `||`-caused wrong match would silently
  attribute the wrong package's version/commit to the *correctly-keyed*
  lock entry — invisible to every assertion this test currently makes. A
  real, closeable gap: add an assertion on the recorded commit/version for
  the second package in that test (or a dedicated one), not a new mechanism.
- **`winget.rs:150` — GAP.** `parse_list`'s end-of-table check,
  `if name.is_empty() || id.is_empty() || version.is_empty() { break; }`,
  `||` mutated to `&&`. The one fixture built specifically to exercise this
  boundary, `list-upgrade-available.txt` (a real captured `winget list
  --upgrade-available` output whose first table is followed by a "N upgrades
  available." line and a second table under a different heading — the exact
  case this line's own comment cites), is used by exactly one test,
  `the_available_column_is_read_when_it_is_there`, which only checks that
  `Google.Chrome`'s row parsed with the right version/available values — it
  never asserts the row **count**, or that parsing stopped where it should.
  Under `&&`, the trailing "N upgrades available." line (whose `Name` field
  slice is non-empty but whose `Id`/`Version` slices are empty) would fail
  to trigger the break, and the second table's rows would be parsed as if
  they belonged to the first. dotpkg's own production argv (`["list",
  "--disable-interactivity"]`, no `--upgrade-available`) may never reach
  this exact second-table shape, which tempers how urgent this is — but the
  general invariant (any one of the three required fields going empty must
  stop the table) is untested at its boundary either direction: this is the
  mirror image of Task 9's already-known deferred minor ("a genuine row
  with an empty Id or Version would silently truncate the table"), and both
  risks live in the same three-field check with zero row-count coverage.
  Closes with one assertion: `parse_list(&fixture("list-upgrade-available.
  txt")).unwrap().len() == 8`.
- **`RealWinget::run`'s `unwrap_or(-1)` (reported as `winget.rs:545`; the
  only `-` inside that function today, and the only site consistent with
  "delete `-`", is `out.status.code().unwrap_or(-1)` at the current
  `winget.rs:526` — noted rather than silently reconciled, since this
  document did not re-run the mutation itself) — ACCEPTED, equivalent
  mutant, mutation `-1` → `1`.** Every
  consumer of `CmdOut.code` in this file checks `== 0`, `!= 0`, `==
  NO_APPLICATIONS_FOUND` (`-1978335212`) or `== NO_VERSION_FOUND`
  (`-1978335209`) — grepped directly, confirmed: no call site anywhere
  checks the literal fallback value itself. `-1` and `1` are therefore
  observably identical to every current caller; both are simply "some
  nonzero code, neither 0 nor a recognised winget constant." The branch is
  reachable only when `Command::output()`'s child process is terminated by
  a signal rather than exiting normally (`ExitStatus::code()` returns `None`
  in exactly that case) — a condition this suite cannot construct without
  sending a real signal to a real subprocess, which nothing in this
  codebase's testing style does. Distinct from the three gaps above: this
  one is not "nobody tests it," it is "no test *could* observe a difference
  even if it tried," which is the project's own definition of an equivalent
  mutant, not a missing test.

## The dogfood

Full record: `docs/dogfood-phase4-2026-08-10.md`. Two real, reproducible
defects found by running commands on a14, not by review:

- **`dotpkg adopt --backend winget` drops a same-line trailing comment on a
  `[winget] packages` array's last element when appending a new one.**
  Root cause traced to `add_winget_package`'s `packages.set_trailing("\n")`
  unconditionally overwriting the array's own trailing decor, where
  `toml_edit` (measured directly against 0.22.27) stores a same-line comment
  on an array's last element. **Fixed in Task 16's own fix wave** (shared
  `append_to_packages_array`, commit `6683dd3`) — see "Read this first"
  above for why the round-trip guard could not have caught it on its own.
- **`apply --prepare`'s preparation table ran a package name into its
  version with no separator** for any id ≥ 13 characters — the common case
  for real winget ids, rare for scoop's own shorter names, which is why
  this had never shown up before a winget-declaring machine exercised it
  for real. **Fixed in the same fix wave** (`src/render.rs:229`, widened
  column plus a literal separator).

Both were verified fixed on the second Windows run above, and both have
paired long-name/short-name or last-element/non-last-element tests, per
the "Verification" section.

## Plan defects: a class worth naming, not just fifteen individual entries

Fifteen numbered plan defects were found across this phase's sixteen tasks
(the full list is in `.superpowers/sdd/2026-08-09-phase4-backend-winget/
progress.md`). **Three of them share one shape, and it is the shape worth a
name of its own: a test or a command that could not fail, reading as a pass
for a reason that has nothing to do with the code being right.**

1. **Four `cargo test` filters selected zero tests.** `cargo test --bin
   dotpkg floor_exit_code` (Task 3), `cargo test --lib mass_prune` (Task 6),
   and two more of the same shape — a filter string that matches no test
   *name*, only the function under test, prints `test result: ok. 0 passed;
   0 failed` and exits 0. Every plan command in this phase was swept
   afterward and the four fixed to unfiltered runs with the expected count
   stated (commit `73a7fa5`).
2. **A `debug_assert!` made its own test unreachable.** Task 5's brief
   mandated both a `debug_assert!` guarding a duplicate-`Installed` shape
   and a Step-1 test constructing exactly that shape to exercise the code
   *past* the assert — but the assert panics first, under the same debug
   profile the test itself runs in, so the test's own `assert_eq!` was dead
   code from the moment it was written. Found empirically by the
   implementer, not by reading; resolved by extracting the guarded logic
   into a pure, unconditionally-tested function rather than profile-gating
   either half away.
3. **The `resolve_root` assertion — vacuous on macOS, actively wrong on
   Windows.** Already covered in full above. The strongest member of this
   class: the first two are structural — a filter that cannot select
   anything, an assert that cannot be reached — and fail (or rather,
   "pass") the same way on every machine. This one **passed differently
   depending on which platform ran it**, and the difference was never
   visible until the real Windows run in Task 16, seven tasks after the
   test was written.

**This is the strongest argument in the whole phase for "the suite runs on
the tree that ships, on the target platform" as a non-negotiable rule, not
a nice-to-have**: two of these three would have been caught by simply
reading the test output carefully (a `0 passed` line, a test that never
runs its own assertion); the third is invisible to every check available on
this project's development machine and exists only on the platform dotpkg
ships for.

## Deferred minors, by originating task

Closed within the phase (verified, not just claimed): the `Mutator`-seam
guard for `clone_missing_buckets` (Task 2); the `SkipReason::Opaque`
exit-code/closing-table gap (Task 4, its own Important finding, fixed
within the same task); the winget-declared-unlocked-package Critical
regression, the cross-backend scan-failure coupling, and the macOS-only
`path_without_winget()` precondition (all three Task 14, all fixed within
the task and the last one **re-verified for real on Windows** in Task 16);
the canonical-id carry-forward defect across three of `fold_backend`'s four
branches, and `adopt`'s missing case-difference warning (both Task 15,
fixed within the task); the config comment-loss bug and the preparation
table's squished column (both found by Task 16's dogfood, fixed within the
same task).

**Left open, organized by where they were found:**

- **Task 5, forward hazard, never revisited (Tasks 6/14 did not touch it):**
  `plan_backend`'s unconditional `Arch::as_scoop()` call. Elevated to "Read
  this first" above — this is the item most likely to bite Phase 4b first.
- **Task 9:** the "header recognised but missing Version" refusal branch
  (`winget.rs:107-109`) has no test — both existing refusal tests hit the
  earlier no-header gate first, inherent to the brief's own test list. The
  end-of-table heuristic's "genuine empty field silently truncates" risk —
  now paired with this task's own `winget.rs:150` gap above, both living in
  the same three-field check. `find("Id")` matches as a substring of a
  header column name (a column literally named `"Identifier"` would
  resolve to the same offset); theoretical, no measured winget header does
  this.
- **Task 11:** `Winget::scan` treats any `WingetCmd::run` error as "winget
  absent" — the trait's `anyhow::Result` erases `io::ErrorKind` before
  `scan` ever sees it, unlike `Scoop::scan`, which distinguishes `NotFound`
  from other error kinds. Matches the brief exactly. Elevated to "Read this
  first" above and merged with `scan_or_warn`'s over-acting risk — a broken
  `winget.exe` reading as "absent" is the same empty-scan failure mode, not
  a separate one.
- **Task 12 / Task 13, still open across both:** no fixture pair exists for
  one package having both a plain `show` and a `show --versions` capture
  taken together, so the "`show`'s `Version:` agrees with `--versions`
  row 0, for the SAME package" cross-check cannot be written from the
  checked-in fixtures. The controller tried to capture
  `winget show -e --id ajeetdsouza.zoxide` before Task 13; a14 had gone
  back to sleep. **Not captured through Task 16 either** — a14 was reached
  twice more in this task (Windows verification, the dogfood) and this
  specific capture was not on either list. Worth doing the next time a14 is
  reachable, since it costs one `winget show` call.
- **Task 13:** `resolve_installed`'s `fell_back_to_tip` warning path
  (`ctx.warnings`, reached when the installed version's commit search falls
  back to the bucket tip) has no dedicated test, before or after this
  phase — reconstructing the git-history shape that triggers it was judged
  too complex for that task's scope, and behaviour was preserved by
  inspection of the `RefCell` sink's design, not by a new test. Still open.
- **Task 14:** `Divergence::describe()`'s text hardcodes the word
  "winget" rather than reading a backend name, correct only because every
  `Capability::ReportsOnly` backend today *is* winget — a second
  report-only backend would need this generalised. The merged `opaque`
  list loses which backend each name came from once `print_scan_warnings_
  and_merge` concatenates scoop's and winget's. `--prepare` does not
  consult `has_reported_only` in its own exit-code path (only `apply`'s
  final floor does). `status`'s exit code is untouched — reads as
  deliberate (the brief's "exit code" language was read as `apply`'s), not
  yet confirmed as the intended final answer.
- **Task 15:** `config_edit.rs`'s scoop-side `unwrap_err()`-before-assertion
  ordering (pre-existing, adjacent to the line this phase touched, left as
  found). No winget-specific partial-write test exists — that property
  currently rests entirely on the shared `write_in_order` seam tests
  covering both backends generically. `Change::Kept`'s name in the
  `Failed` branch still reports the **declared** spelling while the lock
  entry it refers to is keyed by the **canonical** one — a minor
  inconsistency in which spelling a rendered line uses, not a functional
  bug.

## Still open

1. **`Running::covers` is weaker for winget than for scoop** (no `bins`).
   Clear before an executor exists. See "Read this first."
2. **The winget executor has no throwaway root to measure against.** The
   reason Phase 4 stopped at `Capability::ReportsOnly`. See "Read this
   first."
3. **`verify_round_trip`/`verify_round_trip_winget` cannot see comment
   loss**, structurally, because they compare parsed `Config` values.
   The one bug this blindness let through is fixed; the blindness itself
   is not.
4. **Three winget-scan failure modes are one family, safe only because
   winget is `Capability::ReportsOnly`**: `scan_or_warn`'s doc comment
   overstates in the install direction (a converged, installed winget
   package can render as a false `Divergence::Install` after a failed
   scan), and `Winget::scan` treats any `WingetCmd::run` error — including a
   broken or permission-denied `winget.exe`, not just a genuinely absent
   one — as that same empty-scan case, widening how often it fires. Today
   only a wrong report line; over-*acting*, not just over-*reporting*, the
   day `Capability` becomes `Acts`. Clear before an executor exists. See
   "Read this first."
5. **`plan_backend` calls `Arch::as_scoop()` unconditionally.** A third
   backend with per-package arch options would silently inherit scoop's
   vocabulary.
6. **The design's "a new backend slots in without touching the planner"
   promise is half true**: the per-backend loop is real; `Capability` and
   the unconditional `as_scoop()` call are not yet clear of it.
7. **`verify.rs:146`'s `NotFound`-idiom guard is the last of the three
   `docs/phase3-notes.md` "Still open" item 4 named** (`lock.rs:99` closed
   in Phase 3, `backend/scoop.rs`'s manifest-read guard closed in this
   phase's Task 4). Explicit Phase 4 non-goal, carried again.
8. **Four new mutation survivors, ruled in this document**: three gaps
   (`apply.rs:384`, `adopt.rs:468`, `winget.rs:150`, each with the specific
   test that would close it named above) and one accepted equivalent
   mutant (`winget.rs`'s `RealWinget::run` fallback code).
9. **69 of the 71 timeout mutants from the final mutation run are
   unresolved**, not survivors and not closed. Two (`verify.rs:121`,
   `:124`) are confirmed genuine hangs; 17 are confirmed starvation
   (function-replacement mutants, cannot hang); the remaining 52 need a
   clean re-run of just the timeout set, on an idle machine, before they
   can be called anything.
10. **`winget.rs:43`×2, `:46`** (`floor_char_boundary`) and **`winget.rs:
   107-109`** (missing-Version refusal branch) are untested defensive
   code — no non-ASCII winget package name and no such-shaped fixture
   exist yet.
11. **No fixture pairs a plain `show` with `show --versions` for the same
   package**, open since Task 12, not closed by two further a14 sessions in
   this task. One `winget show -e --id ajeetdsouza.zoxide` call away.
12. **`resolve_installed`'s `fell_back_to_tip` warning path is untested**,
   open since Task 13.
13. **Several Task 14 minors are unresolved reading judgments, not
   measured defects**: `Divergence::describe()`'s hardcoded "winget",
   the merged `opaque` list's lost backend attribution, `--prepare` not
   consulting `has_reported_only`, and whether `status`'s exit code should
   float at all.
