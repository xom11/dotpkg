# Carried forward out of Phase 3

Findings from building Phase 3 (`dotpkg update`, `dotpkg adopt`, `src/bucket.rs`)
that Phase 4 (the winget backend) must handle, plus everything the whole-branch
review found by *running* the branch rather than reading it.

Every item here was produced by mutation testing, by a negative control that was
actually fired, by the Windows run, or by a reviewer reproducing something. None
is reasoned-only; where an item **is** reasoned-only, it says so.

`docs/phase2-notes.md` and `docs/phase2b-notes.md` still hold the earlier items;
this file does not repeat them.

## Read this first

**`mass_prune_guard` still reads scoop only, and that is now a live hazard.**
`src/apply.rs:37`. It returns `Ok(())` the moment `declared.scoop.packages` is
non-empty, and the only ownership it ever counts is `state.owned_count(SCOOP)`.
Two consequences the moment a winget backend exists:

- A `pkg.toml` that declares winget packages and no scoop packages reaches the
  `owned == 0` check with `owned` counting scoop only. It passes, and a plan
  built from it prunes every owned winget package with no guard at all.
- A `pkg.toml` that declares *any* scoop package short-circuits at the first
  line, so winget ownership is never considered even in principle.

This must grow a backend loop **in the same change that adds the winget
backend**, not afterwards. It is the one guard standing between a truncated
`pkg.toml` and an uninstall of the whole machine, and it is currently half a
guard. `tests/cli.rs`'s `an_empty_config_is_refused_before_the_machine_is_even_
scanned` covers the scoop half and will keep passing while the winget half is
missing — so it is not a warning that will fire on its own.

**`update` and `adopt` resolve scoop only, on purpose, and say so.**
`resolve_into_lock` carries `old.winget` through untouched rather than dropping
what it cannot resolve, and `update::run` emits a warning naming the count of
winget packages it ignored. Phase 4 replaces the warning, not the carry-through.

## Inherited, unfixed: 15 surviving mutants in `src/backend/scoop.rs`

Phase 2b code. This branch does not touch it, and a Phase 3 review rewriting a
Phase 2b module's test suite is the wrong shape of change — so it is **listed,
not fixed**, exactly as Phase 2a listed things for 2b. It is the largest single
concentration of survivors on the branch (15 of 55, in one file of 97 mutants),
and nobody has looked at it.

**`scoop.rs:219` — `<impl Backend for Scoop>::name -> ""` and `-> "xyzzy"` both
survive. The backend's own name is asserted by nothing**, and everything keys on
it: `state.json` is a map keyed by backend name, `plan()` compares against
`model::SCOOP`, and `owned_count(SCOOP)` is what `mass_prune_guard` reads. Do
this one first, and do it before adding a second backend — the whole point of
Phase 4 is that there will be two names to tell apart.

The rest, grouped by what they are:

- **`:124` ×3** — `resolve_root`'s `b.len() == s.len()`.
- **`:533` ×2 and `:525`** — `clone_missing_buckets`' `.git` existence guard and
  its entire return value. One item, not three. Directly related to the
  no-`.git` guard in `update::run`, which this branch left untested for the same
  reason: no fixture ever has a bucket without `.git`.
- **`:699`, `:712`** — `download_verdict`'s `&&` in two places.
- **`:731` ×2** — `tail`'s `skip > 0`.
- **`:654`** — `strip_ansi`'s `&&`.
- **`:227`** — `scan`'s `NotFound` guard. See the pattern below.
- **`:67`** — the `Value::Array` arm of `declared_executables::walk`.

**A pattern worth more than any single item:** `lock.rs:99`, `verify.rs:146` and
`scoop.rs:227` are the same mutation of the same idiom —
`Err(e) if e.kind() == NotFound => <benign default>` — and **all three
survived**. Wherever this codebase writes that, the benign path is tested and
the other error kinds are not, so an unreadable file reads as an absent one.
`lock.rs:99` was closed here; the other two were not. Phase 4 should treat this
as one fix in three places rather than three unrelated tests.

## The Windows run, before the dogfood

Run on the real dogfood machine (a14, `100.83.225.100`) from
`C:\Users\kln\dotpkg-build`, native `cargo test --no-fail-fast`, source copied
as a tarball (`Cargo.toml`, `Cargo.lock`, `src/`, `tests/`; no `target/`, no
`.git/`).

**Result: 342 passed, 0 failed, 0 ignored, across all 10 targets.** Build
finished in 1m 08s. No failures, no warnings, nothing skipped.

The macOS suite reports 344 for the same tree. The difference is exactly two
`#[cfg(unix)]` tests that do not compile on Windows and therefore never appear
in a count:

- `tests/adopt.rs:459` `a_failed_last_write_leaves_a_prefix_that_plan_does_
  nothing_about` (chmod `0o555` on the state directory)
- `tests/scoop_scan.rs:476` (a symlink-shaped fixture)

18 → 17 in `tests/adopt.rs` and 27 → 26 in `tests/scoop_scan.rs`. Nothing else
differs. Confirmed target by target rather than by subtracting totals, because a
total that happens to match can hide two failures and two extra tests.

### And again, on the tree this review actually leaves behind

The run above measured the tree as Task 13 left it. This review then added 21
tests, six of which spawn the real binary from `tests/cli.rs` and build git
buckets — exactly the kind of code that has broken on Windows before. Shipping
after a Windows run that predates the changes would have been the same mistake
in a new place, so the suite was rebuilt and rerun on a14 against the final
tree.

**Result: 364 passed, 0 failed** (macOS: 366; the same two `#[cfg(unix)]`
tests account for the entire difference, again confirmed target by target).
`tests/cli.rs` goes 17 → 23 on Windows, so all six new CLI tests — including
`adopt --state some/relative/path.json`, which is a path-shaped refusal, and
the `bucket_only` fixture that removes a `pkg.lock` after creating a git
bucket — pass natively. `tests/update.rs` goes 6 → 7, so the new
`[winget]`-section test passes there too.

Three Windows runs were needed in the end, not one: the tree changed twice
after the first. Every one of them was green, and **nothing was ever changed to
make Windows pass** — no fixture edits, no `#[cfg]` gates added.

**Rule this makes explicit for the next phase: the Windows run belongs at the
end of the change as well as before the dogfood.** "Run Windows first" is not
the same instruction as "run Windows on what you ship", and this task needed
both.

### And a fourth time, after the final review's fix wave — this is the tree that ships

The final review's own fix wave (`0a4b7f9`, "Move what the final fix wave
closed into the closed list") added tests after the run above, including six
in `tests/cli.rs`/`tests/update.rs`/`tests/adopt.rs` that build git buckets
and compare rendered paths — the fix wave's own report flagged
`tests/update.rs`'s `a_declared_bucket_that_is_not_on_this_machine_is_warned_
about_by_name_and_path`, whose assertion contains
`scoop_root().join("buckets").join("extras").display()`, as exactly the
rendered-path shape this document lists as a predicted failure class. Shipping
on the run above, which predates that wave, would have repeated the same
mistake this document already named once. So the suite was rebuilt from a
fresh tarball (`Cargo.toml`, `Cargo.lock`, `src/`, `tests/`; no `target/`, no
`.git/`) and rerun on a14 at `0a4b7f970916227af103cd7dd4e767438d34d5b5`.

**Result: 375 passed, 0 failed, 0 ignored, `cargo test --no-fail-fast` exit
code 0, across all 10 build targets (11 `test result:` lines including an
empty doc-tests run).** macOS at the same commit: 377. Confirmed target by
target, not by subtracting totals: every target matches count-for-count
except `tests/adopt.rs` (19 → 18) and `tests/scoop_scan.rs` (27 → 26), and a
full test-name diff (not just a count diff) on both of those two targets shows
the missing name is exactly one test each —
`a_failed_last_write_leaves_a_prefix_that_plan_does_nothing_about` and
`a_root_reached_through_a_symlink_still_matches_running_processes`, the same
two `#[cfg(unix)]` tests named above and nowhere else. Every other target
(`lib` unittests 192, `bucket.rs` 22, `cli.rs` 26, `execute.rs` 26 — including
its one `#[should_panic]` test, which fired identically on both platforms
since this is a `test`-profile build and `debug_assert!` was never compiled
out — `planner.rs` 33, `prepare.rs` 22, `update.rs` 10) is identical name-for-
name, not merely count-for-count.

The flagged rendered-path test passed, so the fix wave's own concern is
falsified the same way the four predictions below were: `/x/` vs `/x\` did not
occur. **Nothing was changed to make this run pass** — no fixture edits, no
production code touched — so this entry closes the "no Windows run" concern
the fix wave's report recorded against itself, on the exact tree it ships,
satisfying the rule stated just above.

### Four predicted Windows failures, all four falsified

The plan named four failure classes to expect, all seen in earlier phases. **None
of them occurred.** Recorded by name so the next phase's plan author does not
budget for them again without new evidence:

| Predicted | Outcome |
|---|---|
| A rendered-path comparison (`/x/` vs `/x\`) | did not occur |
| `#[should_panic]` on a `debug_assert!` compiled out under `--release` | did not occur (the run is a `test` profile build; the prediction never applied) |
| A comparison against `fs::canonicalize` where production strips `\\?\` | did not occur |
| `file://` clone URLs built from a Windows path in `tests/common/mod.rs` | **did not occur** — `a_fetch_moves_the_pin_forward` and the shallow-clone fixture in `tests/adopt.rs` both build `file://{}` from a `C:\...` path and both passed |

The `file://` prediction was the new one this phase and it was the most specific,
so its falsification is the most informative: git on Windows accepts
`file://C:\Users\...` as written by `Path::display()`. That is now measured, not
assumed.

**What this does not license.** Running the suite on Windows before the dogfood
remains mandatory. The value of this run was not that it found nothing — it was
that "342/342 green on the target platform" is now a fact rather than a
prediction, and it cost eight minutes. In Phase 2b-2 one failing target hid two
real Windows defects for several rounds; the reason that cannot happen here is
`--no-fail-fast` plus the per-target comparison above.

## Mutation testing

`cargo mutants 27.1.0`, macOS, `--no-shuffle -- --no-fail-fast`. **509 mutants**
against the branch as Task 13 left it.

### The measurement was contaminated part way through, and how

Recorded rather than quietly rerun, because the failure mode is easy to repeat
and the numbers in a mutation report are worthless if nobody says how they were
produced.

Run 1 was launched with `-j 4`. Its auto-set per-mutant test timeout was **43s**,
derived from an 8s unmutated baseline. Part way through, two more workloads were
started on the same 10-core machine — a second `cargo mutants` job verifying a
fix, and repeated full `cargo test` runs — and the load average went to ~12. From
that moment the suite stopped finishing inside 43s and cargo-mutants began
recording **TIMEOUT** instead of caught/missed. Fifty-five mutants ended that
way, and they are not real: the list includes `State::owns -> true`,
`State::owns -> false` and `Update::wrote_anything -> true`, each of which has
direct unit tests that fail in milliseconds and cannot possibly hang.

**What survives the contamination.** A timeout is the only outcome CPU pressure
can manufacture. It cannot turn a failing test into a passing one, so every
`caught` and every `missed` recorded in run 1 is still valid. Ten files finished
with **zero** timeouts and their numbers are used as-is:

| file | tested | caught | **missed** | unviable |
|---|---|---|---|---|
| `src/adopt.rs` | 14 | 11 | **0** | 3 |
| `src/apply.rs` | 52 | 38 | **0** | 14 |
| `src/config.rs` | 7 | 6 | **0** | 1 |
| `src/execute.rs` | 31 | 28 | **0** | 3 |
| `src/main.rs` | 32 | 24 | **8** | 0 |
| `src/bucket.rs` | 67 | 55 | **8** | 4 |
| `src/config_edit.rs` | 10 | 4 | **6** | 0 |
| `src/plan.rs` | 27 | 22 | **5** | 0 |
| `src/lock.rs` | 19 | 16 | **3** | 0 |
| `src/model.rs` | 29 | 19 | **3** | 0 |
| **subtotal** | **288** | **223** | **33** | **25** |

`src/adopt.rs` finishing at **0 survivors** is the single most reassuring number
on this branch: `resolve_installed`'s two-loop ordering — content across the
whole history before version is tried at all — is the plan's number one
correctness concern, and nothing survives there. `src/apply.rs` and
`src/execute.rs` likewise.

The remaining five files plus `src/backend/scoop.rs` (which run 1 never reached)
were re-measured cleanly: `-j 3`, `--timeout 120`, nothing else running. Those
runs produced **zero** timeouts, which is the confirmation that the 55 in run 1
were contention and not code — `src/state.rs` alone went from 6 timeouts to 0
with no change to the file.

### The whole branch, totalled

**509 mutants, 55 survivors.** Every one is accounted for below.

| file | mutants | survivors | disposition |
|---|---|---|---|
| `src/adopt.rs` | 14 | **0** | — |
| `src/apply.rs` | 52 | **0** | — |
| `src/config.rs` | 7 | **0** | — |
| `src/execute.rs` | 31 | **0** | — |
| `src/update.rs` | 21 | 1 | closed |
| `src/sys.rs` | 6 | 1 | closed |
| `src/config_edit.rs` | 10 | 6 | closed |
| `src/plan.rs` | 27 | 5 | closed |
| `src/lock.rs` | 19 | 3 | closed |
| `src/model.rs` | 29 | 3 | closed |
| `src/main.rs` | 32 | 8 | 6 closed, 2 accepted |
| `src/bucket.rs` | 67 | 8 | 5 closed, 3 accepted (2 = the predicted KNOWN) |
| `src/render.rs` | 43 | 3 | 2 closed, 1 accepted |
| `src/state.rs` | 24 | 1 | accepted (dead code) |
| `src/verify.rs` | 30 | 1 | accepted |
| `src/backend/scoop.rs` | 97 | **15** | **out of scope — see below** |
| **total** | **509** | **55** | **32 closed, 8 accepted, 15 deferred** |

After this task the same files re-measure at: `main.rs` 2 missed (was 8),
`bucket.rs` 3 (was 8), `config_edit.rs` 0 (was 6), `update.rs` 0 (was 1),
`sys.rs` 0 (was 1), `render.rs` 1 (was 3).

### The 15 survivors in `src/backend/scoop.rs`, left for Phase 4

This is Phase 2b code, untouched by this branch's diff, and fixing it here would
have meant a Phase 3 review task rewriting a Phase 2b module's test suite. Listed
rather than silently skipped, because 15 is the largest concentration on the
branch and nobody has looked at it:

- `:219` ×2 — `<impl Backend for Scoop>::name -> ""` / `"xyzzy"`. **The backend's
  own name is asserted by nothing.** Everything keys on it.
- `:124` ×3 — `resolve_root`'s `b.len() == s.len()`.
- `:533` ×2 and `:525` — `clone_missing_buckets`' `.git` existence guard and its
  whole return value. Directly related to the untested no-`.git` guard in
  `update::run`, so these two are one item.
- `:699`, `:712` — `download_verdict`'s `&&`.
- `:731` ×2 — `tail`'s `skip > 0`.
- `:654` — `strip_ansi`'s `&&`.
- `:227` — `scan`'s `NotFound` guard (same family as the `lock.rs:99` gap this
  task closed, and as `verify.rs:146` below).
- `:67` — the `Value::Array` arm of `declared_executables::walk`.

The `NotFound`-guard family is worth calling out as a pattern in its own right:
`lock.rs:99`, `verify.rs:146` and `scoop.rs:227` are the same mutation of the
same idiom, and all three survived. Whenever this codebase writes
`Err(e) if e.kind() == NotFound => <benign default>`, the benign default is
tested and the *other* error kinds are not.

Where a verdict below is "gap", a test was added and the kill was verified by
re-running `cargo mutants -f <file>` against the changed tree; those re-runs are
reported inline.

### The known survivor the plan predicted — confirmed

`src/bucket.rs:191` — `resolve_latest`'s tip self-check, two mutants:
`replace match guard !per_file.is_empty() && t == tip_text with true`, and
`replace && with ||`.

**Verdict: KNOWN, not a gap.** Ordinary git never needs the repair. The ledger
records five separately-constructed shapes — including a hand-built evil merge
where both parents changed the file and the merge resolved to content neither
parent had — in which `git log -1 -- <path>` returned a commit whose blob equals
the tip's every time. The self-check is nonetheless *not* dead code: measured by
the Task 4 implementer, mutating the argv to `--skip=1` makes it catch the wrong
commit and substitute the tip (`fell_back_to_tip = true` observed). So it
demonstrably repairs a wrong `git log` answer; what no constructible repository
does is *produce* that wrong answer. Deleting the comparison alone therefore
survives, and that is the correct outcome, not a missing test.

This is the survivor the plan told Task 14 to expect. It is the only one on the
branch that was predicted in advance and confirmed exactly as described.

### Survivors that were real gaps, now closed

**`src/main.rs` — five survivors, and the largest finding of the review.**

`main.rs:438` (the undeclared-package refusal), `main.rs:459` (three mutants on
`u.failed_count() > 0`), `main.rs:470` (the relative `--state` refusal) and
`main.rs:496` (adopt's refusal exit).

Root cause: **`tests/cli.rs` never invoked `update` or `adopt` at all.** It ran
only `apply` and `status`. Every exit-code decision in the two commands this
entire branch exists to add was unreachable from the suite. Nothing in the plan's
test lists asked for it, and no review round noticed, because reading the tests
shows plenty of `update` and `adopt` coverage — at the *library* level, where
exit codes do not exist.

This is the ledger's THIRD PATTERN, repeating exactly as described: the coverage
hole sits where the output meets a human or the next command. For `update` the
exit code *is* the product — it runs unattended, and a scheduled task learns "a
declared package could not be re-resolved" only from exit 1.

Closed by six new tests in `tests/cli.rs`, deliberately in pairs (a refusal test
alone is satisfied by an implementation that always fails, so each is paired
with a positive sibling that must stay green):

- `update_resolves_a_declared_package_and_exits_zero`
- `update_exits_one_when_a_declared_package_could_not_be_reresolved`
- `update_refuses_a_package_pkg_toml_does_not_declare_and_writes_nothing`
- `adopt_brings_an_installed_package_under_management_and_exits_zero`
- `adopt_exits_one_when_a_package_is_refused`
- `adopt_refuses_a_relative_state_path_before_anything_runs`

Verified: `cargo mutants -f src/main.rs` on the changed tree went from 8 missed
to **2 missed, 30 caught**.

**`src/config_edit.rs` — six survivors in the `multiline` decision.**
`config_edit.rs:49` (`&&`→`||`, and `>`→`==`/`<`/`>=`) and `config_edit.rs:52`
(`-`→`+`/`/`).

Root cause: every existing test asserted that the edited `pkg.toml` **parses**
and that **comments survive**. None asserted the *shape of the text*. So the
whole `multiline` branch — the entire reason this module uses `toml_edit`
instead of re-rendering the file — could be computed any way at all and the
suite stayed green. "Match the surrounding style" was the promise and nothing
held it.

Closed by three tests asserting the actual layout, with the exact shapes
measured first rather than guessed:

- multiline in → new entry on its own indented line with a trailing comma
- single-line in → stays on one line (`packages = ["fzf", "ripgrep"]`)
- empty in → keeps whichever of `[]` / `[\n]` it already had

That last case is also the empty-`pkg.toml` input the ledger recorded as
untested (Task 10 deferred minor), and it is the only input that distinguishes
the `count() >= 0` mutant, since `>= 0` on a `usize` is otherwise a tautology.

Verified: `cargo mutants -f src/config_edit.rs` on the changed tree went from 6
missed to **10 caught, 0 missed**.

**`src/bucket.rs:382` — five survivors in `blobs`'s response parser.**
The bounds check `nl + 1 + n < data.len()`: guard→`true`, `<`→`<=`, and three
arithmetic mutants.

Root cause: the check is defence against a truncated or malformed `git cat-file
--batch` stream, and every test fed it well-formed output from a real git. The
ledger already recorded this as a deferred minor ("unreachable under well-formed
git output"); mutation confirmed it and put a number on it.

Closed by extracting the parser as a private `parse_batch(&[u8], usize)` — the
same seam move `write_in_order` and `Mutator` already make in this crate, and for
the same reason: so the behaviour is *observed* rather than inferred — and
testing it directly against a well-formed stream, a `missing` reply in the
middle, a stream ending exactly at a body boundary, a body shorter than its
header claims, and fewer responses than requests.

Verified: `cargo mutants -f src/bucket.rs` on the changed tree caught **all 15**
`parse_batch` mutants, 0 missed — including `replace match guard nl + 1 + n <
data.len() with true`, which is the one the extraction existed to reach. That
re-run was stopped at 52 of 72 once it was past the parser (the remaining
mutants are in `choose_bucket`, all of which run 1 had already caught); at that
point its only survivors were `src/bucket.rs:99` and the two at `src/bucket.rs:191`,
both ruled below.

The strictness of `<` is the point and now has a test that says so: a stream
ending exactly at the body end is truncated output, not a complete answer,
because git always follows a body with a bare newline.

**`src/lock.rs` — three survivors, three different real gaps.**

- `lock.rs:99` — `load_or_empty`'s `e.kind() == NotFound` guard. Mutated to
  `true`, *any* read failure yields an empty lock. An unreadable `pkg.lock` (a
  directory at that path, a permission denial) would then make `update`
  re-resolve and rewrite every pin, and make `apply` call every declared package
  `NotLocked`, on a machine whose real pins are sitting right there. This is the
  exact hazard Task 12 adjudicated for `state.json` (the CRITICAL finding about
  a `state_path.is_dir()` guard that made `adopt` write on a false belief);
  `pkg.lock` had the same shape and no test. Closed.
- `lock.rs:171` — `if !dir.as_os_str().is_empty()` around `create_dir_all`.
  Every test saved into an existing tempdir, so skipping the call changed
  nothing. Closed with a test that saves to a path whose parent does not exist.
- `lock.rs:119` — `render::key`'s `b == b'_' || b == b'-'`. Mutated to `&&` the
  predicate is always false for those bytes, so `oh-my-posh` renders as
  `[scoop."oh-my-posh"]`. That still *parses*, so nothing broke — but the
  committed file's documented shape is the bare key, and `Git.Git` (which must
  be quoted) was the only key shape any test pinned. Closed with a hyphen and
  an underscore name. Real scoop packages are called `oh-my-posh` and
  `win32-openssh`, so this is the common case, not an exotic one.

**`src/plan.rs:305–308` — five survivors in `is_older`'s string fallback.**
`||`→`&&`, three mutants on the `a < b` fallback, and `pa <= pb`.

Root cause: no test ever passed a version with no digits in it, and no test
compared a version against itself. Scoop manifests really do carry `nightly` and
`latest`. The function's own doc comment calls this branch out as one where
"answers a user can be hurt by" live, and it had no coverage at all.

Notable: `pa <= pb` and `a <= b` both make `is_older(v, v)` true, which would
make `apply` uninstall-and-reinstall a package that is already at the pinned
version, **every single run**. That is a real machine-touching consequence
sitting behind a one-character mutation. Closed.

**`src/model.rs:137, 143, 155` — `PartialEq<&str>`, `PartialOrd`, `Hash` for
`Name`.** Each could be replaced by a constant (`true`, `None`, `()`) and the
whole suite stayed green. `Ord` — the one `BTreeMap` actually uses — was already
covered, which is why the case-folding contract felt tested when three quarters
of it was not. There is no `HashMap<Name, _>` in the crate today, so `Hash` is
genuinely unused; it is implemented because a `Name`-keyed hash map that
disagreed with `Eq` about two spellings of one package is exactly the collision
`lock::parse` refuses elsewhere. Closed with one test pinning all three.

**`src/render.rs:101` and `:172` — two survivors, both "a true line printed
unconditionally".** `plan.drift_count() > 0` mutated to `>= 0` prints
", 0 architecture drift" on every ordinary run; `ex.failed() > 0` mutated to
`>= 0` ends a clean run with "the failure(s) above are everything that
happened". In a tool whose spine is that every printed line is true, both are
defects, and both survived because every assertion was a `contains` on the
positive case with no counterweight asserting the clause is *absent*. Closed
with two tests that assert absence. Re-measured clean: both now caught, and
`render.rs:181` is the file's only remaining survivor (ruled below).

**`src/update.rs:314` — the winget warning, the file's only survivor.**
`!declared.winget.packages.is_empty()` with the `!` deleted survived because
**no test in the suite had ever declared a `[winget]` section at all**. This is
the Task 9 deferred minor, confirmed by measurement. It matters more than its
size suggests: that warning is the only thing standing between "your winget
packages were skipped, on purpose, and Phase 4 will do them" and a user
believing `update` handled the whole file. Closed, and paired with the absence
assertion that makes it discriminate.

**The rest of `src/update.rs` was already solid**, which is the reassuring half
of this result. `Scope::covers`, `failed_count`, `wrote_anything`, and every
mutant of the `RepinnedSameVersion` / `Unchanged` split and the `Kept`
re-insert — the plan's number two correctness concern — were all caught.
Re-measured clean after the fix: **21 mutants, 17 caught, 4 unviable, 0
missed.**

### Survivors accepted, with reasons

**`src/main.rs:411` — two mutants (`&&`→`||`, and `delete !`), the `apply`
exit-code floor.** ACCEPTED, structurally unreachable from this suite.

Measured, not argued: the mutation was applied by hand and all 17 pre-existing
`tests/cli.rs` tests passed with it. The two mutants are only distinguishable
when `code == 0` **and** the preparation was ok **and** there were no running
skips — that is, a fully successful non-empty `apply`. A converged machine
returns early at `src/main.rs:290` and never reaches line 411, and every
non-empty-plan fixture in `tests/cli.rs` ends with a failure because the fixture
deliberately refuses to provide a fake `scoop.cmd` ("no test may provide a fake
scoop binary" is asserted in `Fixture::run`). So a fully successful run cannot be
constructed on a machine without a real scoop.

Two honest ways to close it, neither taken here: run the suite against a real
scoop on the Windows runner, or extract the floor into a pure library function
(`floor_exit_code(code, preparation_ok, has_running_skips)`) and unit-test it —
the same seam move `write_in_order` makes. **The second is recommended for
Phase 4**, because this floor is what two separate commits on `main`
(`9817720`, `95c73e3`) were about, and it is currently the least-tested
exit-code decision in the binary. Note that line 411 is pre-existing `main`
code, not this branch's work.

**`src/bucket.rs:99` — `tip`'s `Ok(o) if o.status.success()` guard → `true`.**
ACCEPTED. Under the mutation a failing `git rev-parse` falls into the success
arm, produces an empty `rev`, and returns `Tip { rev: "HEAD", stale: Some(...) }`
— the same `rev` and the same `Some`-ness as the real failure arm. Only the
*wording* of `stale` differs ("names no upstream" vs "has no upstream to fetch
from"), and both real-world cases (no upstream configured; not a git repository)
already take the failure arm. This is the Task 3 deferred minor — `tip`'s `_`
arm collapsing two causes into one message — showing up as a mutant. Closing it
would mean pinning one of two near-identical warning strings, which buys a
mutation score and no safety. Recorded rather than papered over.

**`src/state.rs:113` — `State::names -> Vec<Name>` replaced with `vec![]`.**
ACCEPTED, and worth more as a finding than as a fix: `State::names` has **zero
callers** anywhere in `src/` or `tests/`. It is unused public API, which is the
entire reason nothing can kill the mutant. Writing a test for a function nobody
calls would raise the mutation score and protect nothing, so it was not
written. **Phase 4 should either give it a caller or delete it.** Recorded
separately because a survivor whose cause is "dead code" is a different fact
from one whose cause is "missing test", and averaging the two into a single
score hides both.

**`src/render.rs:181` — `ex.changed() > 0 || ex.touched() > 0` with `>` → `<`.**
NOT equivalent — the mutant is live. **Correction:** this entry originally
reasoned that the comment above the line, which says `touched()` catches
cases `changed()` misses, implies `touched() ⊇ changed()`, and that if that
containment holds the `changed()` disjunct is redundant and the mutant is
behaviourally identical. The containment is backwards. `changed()` counts
`ItemResult::Done`; `touched()` counts `ItemResult::Failed { touched: true }`
(`Execution::changed`/`touched`, `src/execute.rs:292-318`) — **disjoint
sets**, not nested ones. `touched() >= changed()` only holds when
`changed() == 0`; it says nothing when `changed() > 0`.

The differing case is constructible and reachable: one `Done` and one
`Failed { touched: false }` — the latter reachable via `Step::Remove`'s
uninstall-command-failed arm (`src/execute.rs:221`), which fails before
touching the machine. `changed() == 1`, `touched() == 0`. Real code:
`1 > 0 || 0 > 0` is true, prints "Some packages were changed and some were
not." Mutant: `usize < 0` is always false on both disjuncts, so it prints
"Nothing was changed" instead — a false statement about a machine that just
gained a package. Verified by hand-applying the mutation and confirming the
constructed case goes red.

Closed: `src/render.rs`'s
`a_done_package_alongside_an_untouched_failure_still_says_some_changed` test
now covers this shape, with a negative control (the `>` → `<` mutation
applied by hand, run, and confirmed red) firing on its first assertion
(`out.contains("Some packages were changed and some were not")`) with output
ending in "Nothing was changed; the failure(s) above are everything that
happened." — the mutant's sentence in place of the real one. Deleting the
`changed()` disjunct, as this entry previously recommended, would have
reintroduced exactly the class of false printed line this phase fixed twice
elsewhere (see "Accepted, with reasons" above and the closed entries below).

## The negative-control audit, re-derived and fired

The ledger records that **nine control sets this plan specified were un-fireable
or mis-aimed**, every one caught by an implementer *running* the control rather
than by the plan author writing it. This audit found a tenth, so **the count for
this plan is ten**, and it was found the same way as the other nine: by running
the control rather than reading it.

So this audit did not re-read the plan's control list. Each of the four controls
the plan singled out was re-derived from what the code can actually do wrong,
then **applied to a copy of the tree and run**, and the assertion that actually
fired was recorded.

| Control | Claim | Measured | Verdict |
|---|---|---|---|
| Task 11 control 2 — raw bytes instead of `normalise` | fires on the `matched` assertion, not merely "something was found" | `tests/adopt.rs:59`, `left: Version, right: Content` | **as claimed** |
| Task 9 control 2 — `merge --ff-only` after fetch | fires on the branch-did-not-move assertion | `tests/update.rs:217`, `assert_ne!` with both sides equal | **as claimed** |
| Task 13 control — revert the content-addressed staging path | fires on the CONTENT assertion, not the path-inequality one | `tests/prepare.rs:252`, the `contains("good")` read | **as claimed** |
| Task 12 control 4 — the forbidden write order | "goes red on the **Prune** panic" | `tests/adopt.rs:521`, `"the lock was written first"` | **FALSIFIED** |

### The tenth mis-aimed control, and a Windows hole behind it

Reversing the write order **at the call site in `adopt::run`** (state.json first)
was applied and the suite run. Two things came out of it, neither of which the
plan predicted:

1. **It does not fire on the Prune panic.** It fires four assertions earlier, on
   `"the lock was written first"`. Under the forbidden order the *first* write is
   `state.save`, which the fixture's read-only parent directory makes fail — so
   `write_in_order` short-circuits and nothing at all is written. The Prune panic
   later in that same test is unreachable under this control by construction. The
   Prune *consequence* is covered, but by a different test entirely
   (`the_forbidden_write_order_leaves_a_shape_plan_turns_into_a_prune`), which
   builds the forbidden shape by hand through each file's own writer and never
   calls `adopt::run`.

2. **Only one test in the whole suite catches the reversal, and it is
   `#[cfg(unix)]`.** All 175 library unit tests passed with the call site
   reversed — *including both `write_in_order` seam tests*. Those tests prove
   `write_in_order` invokes its three arguments in order; they say nothing about
   which closure `run` passes in which position. The portable seam that Task 12's
   fix round 2 added to close the Windows gap closes it for the sequencing logic
   and **not** for the call site's argument order.

   So on Windows — this tool's only real target — swapping the three closures in
   `adopt::run` was undetectable.

### Fixed, and not with another test

`write_in_order` now takes one wrapper type per write:

```rust
struct WriteLock<F>(F);
struct WritePkgToml<F>(F);
struct WriteState<F>(F);
```

Swapping two arguments at the call site is now a **compile error**, on every
platform, with no test involved:

```
error[E0308]: arguments to this function are incorrect
   --> src/adopt.rs:159:17
    |
160 |                     WriteState(|| state.save(state_path)),
    |                     ------------------------------------- expected `WriteLock<_>`, found `WriteState<{closure@src/adopt.rs:160:32: 160:34}>`
162 |                     WriteLock(|| crate::lock::save(&lock, lock_path)),
    |                     ------------------------------------------------- expected `WriteState<_>`, found `WriteLock<{closure@src/adopt.rs:162:31: 162:33}>`
help: swap these arguments
```

This is the same move `Name` makes in `crate::model`: the wrong thing stops
being a bug to be caught and becomes a program that cannot be written. It needs
no test, runs on every platform including the one a `#[cfg(unix)]` test cannot
reach, and cannot rot. rustc even offers the correct order in its `help:`.

Both seam tests are kept. **Three properties, three holders**, and they should
not be confused again:

| property | held by | platforms |
|---|---|---|
| which closure goes in which position | the wrapper types | all, at compile time |
| that `write_in_order` calls them in order and short-circuits | the two seam tests | all |
| that the sequence survives a real interrupted write | `a_failed_last_write_leaves_a_prefix...` | unix only |

**The residual risk, stated honestly:** the types stop the three *arguments*
being reordered. They do not stop someone writing
`WriteLock(|| state.save(state_path))` — putting the wrong body inside the right
wrapper. That line is self-contradictory on its face, which is the best a type
can do here short of making each write a distinct method on a trait.

This also answers the plan's Step 6 question directly. **The write-order test in
`adopt` does discriminate the orders — but not on the assertion its own name and
comments point at, and, before this fix, not on any platform where
`#[cfg(unix)]` is false.** The induced failure is a read-only parent directory,
not a rename onto a directory, so the Unix/Windows error-shape difference the
plan worried about never arises; the real problem was simpler and worse, namely
that the check did not exist on Windows at all.

## The three patterns, for the next plan author

These are the ledger's, restated with what this task added. They are about
**authoring** controls and coverage, not about executing them — execution has
been reliable; specification has not.

### 1. Controls aimed at an external tool's behaviour fail about half the time

Three of the first six control sets in this plan were un-fireable as written:
Task 2 control 2 (the guard's own `.context()` already supplied the asserted
substring), Task 4 control 2 (`git log -1` cannot disagree with the tip's blob on
any constructible shape), Task 5 control 2 (a "missing" reply from `git cat-file
--batch` has no trailing blank line, so a +1 byte over-advance is absorbed by the
next header's discarded sha field).

The pattern in the failures: **every control that failed was aimed at git's
behaviour or at an error string's content. Every control that held was aimed at
this crate's own logic.** Writing a control by reasoning about an external tool
is unreliable at roughly a 50% rate. Write those as *measurements first* — run
the tool, record what it does, then write the control against the recording.

Task 14 adds a tenth instance (Task 12 control 4 above), and it is a variant
worth naming separately: the control fired, but on a different assertion than
claimed, because the plan author reasoned about what the *mutation* would do
without reasoning about what the *fixture* would do to it first.

### 2. A control that fires at an `unwrap` before its own assertion

Seen three times before this task (Task 7 control 1, Task 8 control 2, and one
earlier). The property is still protected and the test still goes red — but the
diagnostic is `called \`Result::unwrap_err()\` on an \`Ok\` value`, and a future
reader believes the named assertion below it is what guards the property when
that assertion never ran.

Swept in this task. Five tests were restructured to
`let r = ...; assert!(r.is_err(), ...);` with the side-effect assertions moved
**above** the point where the error is consumed:

- `src/lock.rs` `a_lock_the_guards_would_reject_is_never_written` — was skipping
  `!path.exists()`, which is the entire point of the test
- `tests/execute.rs` — was skipping both `fake.calls()` is empty and
  `owned_count(SCOOP) == 0`
- `tests/prepare.rs` ×3 — each was skipping a `read_dir(...).count() == 0`
  counterweight

The remaining `unwrap_err()` sites in `src/config.rs`, `src/config_edit.rs`,
`src/state.rs` and `src/apply.rs` were left alone deliberately: their only
assertion *is* about the error message, so an unwrap panic and a failed
`contains` report the same fact and no named assertion is bypassed.

**Rule for the next plan: if a test asserts anything other than the error's text,
it may not consume the `Result` before those assertions run.**

### 3. The test list omits whatever carries the task's point downstream

Three IMPORTANT findings during the build traced to this: Task 9's false `Kept`
line plus a completely untested `render_update`, and Task 11's `Found.version`
asserted by no test. The controls were fine; the *coverage specified* had holes
exactly where the output meets a human or the next command.

Task 14 found the largest instance of all: **no CLI test invoked `update` or
`adopt`**, so every exit code in the two new commands was untested. Five
surviving mutants, all of them exit-code decisions.

**Rule for the next plan: for each module, ask what it produces that something
downstream consumes — a human reading stdout, a shell reading `$?`, the next
command reading a file — and require that thing to be asserted by name.** A
library-level test of the same logic does not count; exit codes and rendered
lines do not exist at that level.

## The `contains(...)` counterweight sweep

Every refusal assertion in this phase was checked for a counterweight — either a
count of files written (which must be zero) or a positive sibling that stays
green. A mutation that always fails with the right words survives an unpaired
`contains`.

Three were unpaired and are now paired:

- `tests/adopt.rs` `a_package_that_is_not_installed_is_refused_rather_than_
  invented` — asserted only the message. Now also asserts nothing was adopted,
  no `pkg.lock`, no `state.json`, and that `pkg.toml` did not grow the package.
- `tests/adopt.rs` `a_refusal_names_shallowness_when_that_is_the_likely_cause` —
  now also asserts the counts, and that the message names `unshallow` (naming a
  cause without the command that fixes it is half a message).
- `src/render.rs` — the drift-summary and failure-sentence assertions described
  above, which were `contains`-only on the positive case.

### One test that was passing vacuously

`tests/adopt.rs` `an_adopted_package_is_not_a_prune_candidate_and_not_notlocked`
called `adopt::run(...).unwrap()` and then asserted that the resulting plan
contained no `Prune` and no `Skip{NotLocked}`. But `run` returns `Ok(Outcome)`
for a *refusal* too, and a refused adopt writes nothing — leaving the package
installed, undeclared and **unowned**, which is not a prune candidate either. Any
failure of adopt would have made this test pass for entirely the wrong reason.

This is the same shape as the fixture bug the Task 12 implementer found in the
plan's brief (a manifest committed without a `.json` extension, which made
several Step-1 tests pass vacuously) — fourth and now fifth instance of the
plan's own fixtures being the weak link. Fixed by asserting
`out.adopted.len() == 1` and `out.refused.is_empty()` before the plan is built,
and by adding `assert!(plan.actions.is_empty())` as the positive counterweight to
the two `panic!` arms, which on their own are satisfied by any plan that happens
to contain neither variant.

## Deferred minors: what was closed and what was accepted

Closed in this task (details above): the `Mutator` trait's doc comment; the
`wrote_anything` comment block that still said "has no direct test as of Task 8"
while introducing three direct tests; `tests/update.rs`'s test whose name claimed
"keeps the old pin" while deliberately running with `Lock::default()` (renamed,
and its dead `old` binding deleted); `src/adopt.rs`'s seam test named "stops on
the third" when the third write is last and it cannot discriminate that;
`lock.rs`'s `unwrap_err()`-first control; the empty-`pkg.toml` / inline-table gap
in `config_edit`; `blobs`'s untested off-by-one; the four `unwrap_err()`-first
tests outside `lock.rs`.

**Closed by the final whole-branch review (the fix wave before merge):**

- **A declared bucket that is not on disk was skipped in silence, and both
  `update` and `adopt` then printed a false reason.** Three places, all in the
  new code. `update::run`'s fetch loop did a bare `continue`; `choose_bucket`'s
  search loop did the same and then returned `NotFound { searched:
  declared_names }` — the *full declared list*, including buckets it never
  opened; and `choose_bucket`'s `stated` branch (a bucket named by the lock or
  by `[scoop.opts]`) had no `.git` check at all, so an absent bucket was opened
  as though it were there, `tip()` fell to its `_` arm, every `git_show`
  failed, `resolve_latest` returned `Ok(None)`, and `update` rendered that as
  `bucket <name> has no manifest for it`. Measured before the fix: with
  `buckets = ["main", "extras"]` and only `main` on disk, `update` printed
  `no declared bucket has it (searched: main, extras)` and exited 1 — for
  every declared package on a fresh machine, with nothing anywhere saying an
  uncloned bucket was the cause. `apply` already knew how to say it
  (`src/backend/scoop.rs`: `bucket {bucket:?} is not present at {path}`).

  Closed: `BucketChoice::NotFound` now carries `searched` **and** `missing` as
  separate lists, and a new `BucketChoice::NotCloned { name, dir }` covers the
  `stated` branch — a distinct variant rather than a `NotFound` with an empty
  `searched`, because "a search happened and found nothing" and "no search was
  possible" are different facts and only one of them is about the bucket's
  contents. `bucket::not_found_why` and `bucket::not_cloned_why` are shared by
  `update` and `adopt`, because the version of these messages that was written
  twice is how the false line got printed twice. The fetch loop warns by name
  and path and points at `dotpkg apply --clone-missing-buckets`; the presence
  check was also lifted out of the `!offline` branch, since whether a bucket is
  on this machine is a fact about the machine and not about the network.

  Tests: `tests/bucket.rs`'s `a_declared_bucket_that_is_not_on_disk_is_
  reported_as_missing_not_as_searched` and `a_bucket_named_by_the_lock_or_by_
  an_opt_that_is_not_on_disk_is_reported_as_absent` (which carries its own
  counterweight: the same stated bucket, present, must still be `Chosen`);
  `tests/update.rs`'s `a_declared_bucket_that_is_not_on_this_machine_is_
  warned_about_by_name_and_path`, `the_refusal_names_only_the_buckets_
  actually_searched_and_says_which_were_missing` and `a_locked_bucket_that_is_
  not_on_this_machine_is_not_reported_as_a_missing_manifest`;
  `tests/adopt.rs`'s `a_declared_bucket_that_is_not_on_this_machine_is_named_
  as_absent_rather_than_as_manifestless`. Note that `tests/bucket.rs`'s
  pre-existing `a_package_no_declared_bucket_has_names_what_was_searched`
  declares `main` and creates `main`, which is why no test in the branch could
  see any of this.

  Negative controls fired, with the assertion each one hit:

  - Revert the search loop to `searched: declared_names` →
    `tests/bucket.rs:550` (`assert_eq!(searched, vec![main])`, got `[main,
    extras]`) and `tests/update.rs:188` (`!why.contains("searched: main,
    extras")`, reproducing the reviewer's measured line verbatim). The
    `not searched: extras` assertion sits after the one that fired and was not
    reached.
  - Remove the `.git` check from the `stated` branch → `tests/bucket.rs:586`
    (the `NotCloned` match arm, got `Chosen { name: extras, ... }`),
    `tests/update.rs:235` (`!why.contains("no manifest")`, got `bucket extras
    has no manifest for it`) and `tests/adopt.rs:475` (`why.contains("not
    present at")`, got `no commit in bucket extras carries aichat 0.30.0`).
  - Revert the fetch loop to a bare `continue` → `tests/update.rs:131`
    (`assert_eq!(absent.len(), 1)`, got 0: the only warning was the offline one).

  This also closes two items the ledger filed as coverage gaps — Task 6 item 2
  and Task 9's no-`.git` guard. Neither was a missing test; both were this
  defect.

- **A carried-forward pin could block the whole write, and the advice was the
  command that had just failed.** `lock::save` runs `lock_coherence_guard` over
  the entire new lock, and `resolve_into_lock` inserts pins `update` never
  produced: a package outside a named scope keeps its old pin, a failed
  re-resolve keeps its old pin, and a named run re-inserts every no-longer-
  declared entry. So one malformed entry anywhere made `lock::save` return
  `Err`, `main.rs`'s `?` aborted, and every other package's resolution in the
  run was discarded — under the message `refusing to write a pkg.lock that
  dotpkg apply would reject: pkg.lock is not usable. Run \`dotpkg update\` to
  rewrite it.`, printed by `dotpkg update`. For `dotpkg update <pkg>` it was
  also unconditionally unhelpful: because of the carry-forward, a targeted run
  cannot repair a bad entry elsewhere.

  **The guard's placement is correct and stayed.** What changed is the message.
  `apply::lock_coherence_guard` was split so the per-entry rules live in
  `entry_coherence` with **no advice attached**, and `apply::incoherent_entries`
  returns every rejected entry rather than stopping at the first — a guard's
  product is "refuse or don't", but `update`'s failure message's product is
  "which entries do I repair", and stopping at the first turns one repair into
  N runs. `main.rs`'s `Update` arm now says outright that pkg.lock was **not**
  written (the diff `render_update` already printed reads as an accomplished
  fact), names the blocking entries, and gives advice that is not the command
  the user is already running: delete the `[scoop.<name>]` block, or
  `dotpkg update <name>` for it. A refusal exits 2 (nothing was touched); a
  plain I/O failure from `lock::save` still exits 1, and the two are told apart
  by whether `incoherent_entries` is empty.

  Test: `tests/cli.rs`'s `a_carried_forward_entry_that_blocks_the_write_is_
  named_and_the_advice_is_not_the_command_that_just_ran`, paired with the
  existing `update_resolves_a_declared_package_and_exits_zero` — without that
  positive sibling, an `update` that always refused to write would satisfy
  every assertion. Negative control: revert to `dotpkg::lock::save(&u.lock,
  &lock)?` → `tests/cli.rs:1167` (the exit-code assertion; exit 1 instead of 2,
  with the circular `Run \`dotpkg update\` to rewrite it.` on stderr).

- **`adopt`'s mid-run write failure said nothing about what it had written.**
  It propagated with `?`, which skipped `render_adopt` entirely: the user saw
  `cannot create ...\state.json.tmpNNN` and no line anywhere saying `pkg.lock`
  and `pkg.toml` had already been rewritten. Closed: `write_in_order` returns
  the prefix that really landed, `Outcome::partial_write` carries it out, and
  `render_adopt` prints both lists — what changed on disk and what did not,
  computed as a complement rather than phrased as "and the rest". `main.rs`
  exits 1 (not 2: files changed, so "refused, and nothing was touched" would be
  a lie). `run`'s doc comment, which said "across packages a refusal is
  reported and the rest proceed", now distinguishes a refusal from a write
  failure — the latter is not a refusal and does stop the rest.

  Tests: `tests/adopt.rs`'s `a_failed_last_write_leaves_a_prefix_that_plan_
  does_nothing_about` (extended: it used to assert only `result.is_err()`),
  `src/render.rs`'s `a_partial_write_names_the_files_that_changed_and_the_
  files_that_did_not` with an absence counterweight, and the two
  `write_in_order` seam tests, which now assert the prefix as well as the
  order. Negative controls: propagate with `?` again →
  `tests/adopt.rs:613` (the `expect("a partial write is reported through the
  outcome, not through \`?\`")`); delete the `partial_write` block from
  `render_adopt` → `src/render.rs:1261` (`text.contains("changed on disk:
  pkg.lock, pkg.toml")`) and `tests/adopt.rs:635` (`the report must name what
  really changed on disk`).

- **`adopt` turned an unreadable installed manifest into an empty one.**
  `std::fs::read(...).unwrap_or_default()` meant the content loop could not
  match, the version loop answered, and the user was told `matched by version
  only -- the installed manifest differs` — false, because the manifest was
  never compared. Now a per-package `Err` naming the path. **No test**: the
  only way in is a TOCTOU window between `scan`'s read and `adopt_one`'s read
  of the same file, and it cannot be opened deterministically from outside
  `adopt::run`. Recorded rather than papered over.

- **The lock recorded a bucket's display spelling while `choose_bucket` opened
  its folded key.** `Scoop::stage` opens what the lock says verbatim, so
  `buckets = ["Extras"]` resolved during `update` and failed at `apply` on any
  case-sensitive filesystem. Windows is case-insensitive so the real target was
  unaffected. Both `update` and `adopt` now record `bucket_name.key()`.
  **Chosen over folding inside `stage`** because the lock should record the
  directory that was actually opened and read — the display spelling names
  nothing that was verified — and because folding in `stage` would make it
  accept a bucket spelling that never existed on disk, in a committed file
  whose diff people read.

- Three comment/assertion repairs: `src/render.rs`'s pointer at
  `an_ambiguous_bucket_keeps_the_old_pin_and_names_both_candidates`, renamed on
  this branch (fixed to the current name); `ensure_plain_component`'s doc,
  which said "composes three of them" while listing four components (now says
  four, and names `ensure_commit_hash` as what covers `<commit>`); and
  `tests/cli.rs`'s `lock.contains("0.74.1") || lock.contains("1.0.0")`, whose
  first disjunct could never fire because the fixture only ever commits
  `1.0.0` (dropped).

**Already closed during the build, recorded here so nobody re-opens them:**
`Change::Kept`'s doc comment now describes both shapes (Task 9's fix changed
`version` to `Option<String>` and documented why an empty-string sentinel was
refused); `wrote_anything` now has three direct tests.

Accepted, with reasons:

- **`src/main.rs:101-102` prints two consecutive `warning:` lines.** Brief-
  mandated text, slightly repetitive on a real terminal. Cosmetic; left.
- **`src/bucket.rs`'s `tip` collapses two causes into one message.** See the
  accepted-survivor entry above.
- **`git_ok` takes an arbitrary `&[&str]`**, so nothing in the type system
  enforces the module doc's "nothing here writes to a working tree or moves a
  branch". Pre-existing, caller discipline only. Worth noting that the Task 9
  control above (`merge --ff-only` inserted into `fetch`) is exactly the
  violation this would allow, and it *is* caught by a test — so the invariant is
  covered behaviourally even though it is not covered by types.
- **The pre-existing `blobs` race** where git exiting before the writer's
  `writeln!` lands surfaces "cannot feed git cat-file: Broken pipe" rather than
  the more informative "git cat-file failed: `<stderr>`". Not introduced by this
  branch; either way it returns `Err`, never the silent `Ok(vec![None])`.
  Stress-tested 30×, never observed.
- **`verify_round_trip`'s failure path is unreachable through the public
  `add_scoop_package`.** Structural and honestly disclosed in the function's own
  doc comment; exercised directly by a white-box test.
- **`config_edit::save`, `lock::save` and `state::save` all leave the temp file
  behind if `File::create`/`write_all`/`sync_all` fails** — only the rename
  branch cleans up. All three have the identical gap, so it is not a regression.
  Fix all three together or accept all three; accepted here, and named so Phase 4
  can decide once.
- **`key()`'s quoted branch uses Rust `Debug` escaping**, which diverges from
  TOML's `\uXXXX` for control characters and exotic Unicode. Not reachable from
  real scoop or winget package identifiers.
- **No committed test for `render()`'s no-panic on a `Pin` of the wrong variant
  in the wrong map.** Guaranteed only by the let-else pattern.
- **One of the three Task 6 coverage gaps remains**: **no test proves the lock
  beats a PRESENT, CONFLICTING opt**. Every lock test runs with no
  `[scoop.opts]` at all, so it proves the lock works *alone*, not that it
  *wins*. That is the precedence claim's core and it is still inspection-only.
  `choose_bucket`'s mutants all died, so nothing here is currently wrong — but
  the claim in the doc comment is stronger than the evidence. **Recommended for
  Phase 4**, since winget will add a second consumer of the same precedence
  rule.

  The other two items filed here — a lock or opt naming a bucket `pkg.toml`
  does not declare, and a declared bucket without `.git` being skipped — were
  not coverage gaps at all: both are defects. **Correction, entered by the
  scoped re-review of the final fix wave:** this bullet originally closed the
  first one too, claiming it "is now tested alongside its fix." Both halves of
  that were false. The fix wave's `choose_bucket` rewrite touched the
  `stated` branch to add the `.git` check described in the closed entry below,
  but left its `!declared_names.contains(&stated)` arm returning `NotFound {
  searched: vec![stated], missing: [] }` exactly as before — routing the
  branch through the new helper without addressing it, not fixing it, and
  nothing in the branch's tests reached it either, since every `choose_bucket`
  test up to that point declared the bucket it stated. The re-review caught
  it; see "Closed by the scoped re-review of the final fix wave" below for the
  actual fix, its tests, and its negative controls. The second item — a
  declared bucket without `.git` being skipped — was fixed as this bullet
  originally described; see the closed entry below.
- **The no-`.git` fetch-loop guard (`src/update.rs:238-240`) has no direct
  test.** Also not a coverage gap: the guard was silent, and that was the
  defect. See the closed entry below.

## What the dogfood added

`docs/dogfood-phase3-2026-08-09.md`. It found one defect, and the defect is the
THIRD PATTERN below repeating in the one place nobody had looked.

**`adopt` was the only command that discarded `scan.warnings`.** `status`
(`main.rs:145`), `apply` (`:189`) and `update` (`:450`) have printed them since
Phase 2a; `adopt::run` called `Backend::scan` and never read the field. The
visible consequence on a14 was `dotpkg adopt antigravity` printing
`antigravity is not installed` about a package that **is** installed — the
scan could not traverse its junction, and the one line that would have
explained that was thrown away. Closed: `adopt::Outcome` now carries
`warnings`, `main.rs` prints them above the outcome, and `tests/cli.rs` holds
it with a paired assertion (`adopt_prints_what_the_scan_could_not_read_…` plus
an absence counterweight). Recorded here because the *class* is what matters —
every command that scans must print what the scan could not read, and Phase 4
adds a second backend that will scan.

Three things the dogfood measured that were previously assumed:

- **`update` fetches and never pulls, on real hardware.**
  `refs/remotes/origin/master` moved forward in `main` and `extras`;
  `refs/heads/master` did not move in any bucket and every working tree stayed
  clean.
- **The fetch really does change the answer.** Rewinding
  `refs/remotes/origin/master` by one commit moved exactly one pin back
  (`tree-sitter 0.26.12 -> 0.26.11`) and left twenty-four alone; the next
  fetching run moved it forward and restored the ref.
- **Real-bucket timings**, replacing the synthetic 153×:
  `update` over 25 packages against a 78,473-commit `main` is 31.5 s with a
  fetch, 23.9 s offline, 16.4 s converged; `adopt`'s full history walk that
  finds nothing is 6.5 s for one package.

Two things it could not exercise on this machine and said so rather than
implying coverage: **no declared package is ambiguous** (only `flux` is, in the
whole union of the three buckets, and it is neither declared nor installed —
the refusal and `[scoop.opts] bucket` were fired deliberately in a throwaway
root), and **no installed package has a version missing from its bucket's
history**, so `adopt`'s refusal had to be constructed.

## Still open

1. **`mass_prune_guard` reads scoop only.** Top of this file. Must grow a backend
   loop in the same change that adds winget. **This is now the highest-value
   open item in this document.**
2. **`main.rs:411`'s exit-code floor is tested, but its two mutants are not.**
   `tests/cli.rs:666`
   (`keep_going_does_not_report_success_when_a_declared_package_could_not_be_prepared`)
   reaches the floor through the `!preparation.is_ok()` branch and pins exit
   1; deleting the line turns that test red. What survives is the two
   line-411 mutants (`&&`→`||`, delete `!`) described above, which are
   structurally unreachable from this suite because they diverge only on a
   fully successful non-empty `apply`, and no fixture can construct one
   without a real scoop binary. Recommend extracting the floor as a pure
   function in Phase 4.
3. **15 surviving mutants in `src/backend/scoop.rs`**, listed in their own
   section above. `Backend::name` being asserted by nothing is the one to do
   first, and to do before there is a second backend to tell it apart from.
4. **The `Err(e) if e.kind() == NotFound => <benign default>` idiom is tested
   only on its benign path**, in three separate places. `lock.rs:99` was closed
   here; `verify.rs:146` and `backend/scoop.rs:227` were not. One fix, three
   places.
5. ~~**`render.rs:181` may be an equivalent mutant.**~~ CLOSED. It is not
   equivalent: `changed()` and `touched()` are disjoint, not nested, so the
   claimed containment only holds when `changed() == 0`. One `Done` and one
   `Failed { touched: false }` makes the real code and the `>` → `<` mutant
   print different, and differently false, sentences. See the corrected
   "Accepted, with reasons" entry above and the new
   `a_done_package_alongside_an_untouched_failure_still_says_some_changed`
   test.
6. **The lock-beats-a-conflicting-opt precedence claim is inspection-only.**
7. **`config_edit::save` / `lock::save` / `state::save` temp-file cleanup gap**,
   all three identical.
8. **`State::names` has no callers.** Give it one or delete it.
9. **`src/adopt.rs:63`: on a `Content` match whose blob has no parseable
   `version`, `Found.version` falls back to the caller's string.** If the blob
   has no version, `Found` cannot honestly claim one — the value it reports
   would then describe the *installed* manifest rather than the commit it just
   pinned, and `stage_text` (which compares the lock's version against the blob
   at that commit) would reject the resulting pin at `apply` time. Unreachable
   through `adopt::run`: a manifest with no `version` never enters
   `scan.installed` (`Scoop::scan` warns and skips it), so the caller's string
   is always a version that came from a manifest that had one. A seam-level
   wart rather than a live bug — `resolve_installed` is public and the fallback
   is reachable by calling it directly. Left as found, by the final review's
   own instruction, and recorded so the shape is visible if Phase 4 gives
   `resolve_installed` a second caller.
10. **`--state` relocates `state.json` but not the staging root**, so
   `apply --prepare --state <elsewhere>` still writes
   `%LOCALAPPDATA%\dotpkg\manifests`. Deliberate — `default_staging_root`'s
   doc comment gives the reason — but "point `--state` somewhere else and
   dotpkg writes nowhere else" is a reasonable belief and is false. Decide
   whether to document it in `--state`'s own help text.
11. **`plan()` reads an unreadable manifest as "not installed" and emits
   `Install`, not a skip.** `docs/dogfood-phase3-2026-08-09.md:110-127`: on
   a14, `zellij` and `actionlint` are both installed at exactly the pinned
   version, but their `manifest.json` cannot be traversed under plain
   elevated `ssh` ("untrusted mount point", os error 448), so
   `Scoop::scan` warns and omits them from `installed`. `plan()` only sees
   the omission — `current = installed.iter().find(...)` comes back `None`
   — and, having no way to tell "not installed" from "installed but
   unreadable," emits `Action::Install` for both. The dogfood measured the
   consequence: `--yes` under these conditions would reinstall two working
   packages for no reason, i.e. uninstall-then-install a package that was
   never actually absent — a real window in which it is. **New to this
   phase**: in 2b-2 these packages were undeclared and so invisible to
   `plan()`; Phase 3 is what declared them and made the misreading live.
   This is distinct from the `adopt`-swallows-`scan.warnings` defect closed
   above (`## What the dogfood added`) — that was `adopt` discarding a
   warning it had; this is `plan()` never receiving one, because `Scan`
   does not carry the names it could not read. The auditor's suggested
   fix: give `Scan` a field for the names `scan` could not read, and have
   `plan()` emit a skip for those names instead of `Install`. **Close this
   before a second backend doubles the scanner** — each backend's `scan`
   will need the same field, and retrofitting it after two implementations
   exist is more work than adding it to the one that exists now.

**Closed by this review, so not carried:** `adopt::run`'s call-site write order,
which was unprotected on Windows and is now a compile error. Recorded here
because the previous draft of this file listed it as the highest-value open
item, and it should be visible that it moved rather than vanished.

**Also closed, so not carried:** an item an earlier draft of this list carried
at number 9, before the final review's fix wave renumbered it away — `update`
printing `pkg.lock is already current -- not rewritten.` when there was no
`pkg.lock` at all and the only declared package could not be resolved.
Measured by the dogfood on a14. `render_update` now prints that line only when
`wrote_anything()` is false **and** `failed_count() == 0` (every change really
is `Unchanged`); when `wrote_anything()` is false because everything failed
instead, it prints `pkg.lock was not rewritten -- N package(s) could not be
resolved.` instead, which is true in both the fully-empty-lock case the
dogfood hit and the mixed case where some packages are genuinely unchanged
and others merely failed to re-resolve. `src/render.rs`'s
`the_not_rewritten_line_does_not_claim_convergence_when_resolution_failed`
reproduces the dogfood's exact state — an empty old lock, one declared
package, a `Change::Kept` with no previous version — and asserts both that
"already current" is absent and that the true reason is named. The summary
counts line immediately above (`N changed, N unchanged, N could not be
resolved.`) was checked in the same state and found already accurate: it
correctly reports `0 changed, 0 unchanged, 1 could not be resolved.` in the
dogfood's case, so it needed no change.

## Closed by the scoped re-review of the final fix wave

**`choose_bucket`'s fourth exit still printed the defect the final fix wave
existed to kill, and this file falsely claimed it had been fixed.** See the
correction entered above, in the Task 6 "Accepted, with reasons" bullet, for
the false claim itself. What follows is the actual history.

The final fix wave (closed entry above, "A declared bucket that is not on
disk was skipped in silence...") added the `.git` check to `choose_bucket`'s
`stated` branch — the bucket named by the lock or by `[scoop.opts] bucket`
— but that branch has an earlier exit above the `.git` check: `if
!declared_names.contains(&stated)`, for when `pkg.toml` does not declare the
named bucket at all. That exit was left returning `BucketChoice::NotFound {
searched: vec![stated], missing: [] }`, unchanged from before the wave.
`not_found_why`'s `(false, true)` arm (`searched` non-empty, `missing`
empty) rendered that as `no declared bucket has <pkg> (searched: <bucket>)`
— naming the undeclared bucket as searched, when it was neither declared nor
searched. `apply` was already accurate about the identical state
(`src/apply.rs`'s `bucket_is_declared` check: `bucket "extras" is not
declared in pkg.toml -- add it to [scoop] buckets`), so `update` and `adopt`
disagreed with `apply` about the same machine — the same shape as the defect
the wave was closing.

Not exotic: `adopt` passes `install.json`'s `bucket` as its hint
(`src/adopt.rs`'s `hint = already.or(inst.bucket.as_deref())`), so adopting
any package scoop installed from a bucket the user has not declared — the
ordinary reason `adopt` fails on a real machine — hit this. For `update`,
dropping a bucket line from `pkg.toml` while its pin survives in `pkg.lock`
does the same.

**Why it survived the wave's own tests.** Every `choose_bucket` call in
`tests/bucket.rs` before this fix declared the bucket it stated — the same
shape of gap the wave's own "Closed" entry above notes about
`a_package_no_declared_bucket_has_names_what_was_searched`. No test in the
branch reached this branch of the `if`.

**Closed:** `BucketChoice` gained a third, distinct exit,
`Undeclared { name: Name }`, alongside `NotCloned` — a separate variant
rather than a `NotFound` with a one-element `searched`, for the same reason
`NotCloned` is separate from `NotFound`: "not declared at all" is a
different fact from "a search happened and found nothing," and only one of
them is about a bucket's contents. `bucket::not_declared_why` renders it,
shared by `update` and `adopt` for the same reason `not_found_why` and
`not_cloned_why` are — pointing at `[scoop] buckets`, not at
`--clone-missing-buckets`, which clones a bucket `pkg.toml` already declares
and is the wrong fix for a bucket that was never declared to begin with.

Tests: `tests/bucket.rs`'s
`a_bucket_named_by_the_lock_or_by_an_opt_that_pkg_toml_does_not_declare_is_reported_as_undeclared`
covers both entry points — the lock and `[scoop.opts]` — at the unit level,
with the bucket cloned on disk in both cases so the test cannot be satisfied
by `NotCloned` instead. `tests/update.rs`'s
`a_locked_bucket_that_pkg_toml_does_not_declare_is_named_and_told_to_declare_it`
and `tests/adopt.rs`'s
`install_json_naming_a_bucket_pkg_toml_does_not_declare_is_named_and_told_to_declare_it`
cover the same shape end to end through each command.

Negative control fired, with the assertion each site hit: reverting
`choose_bucket`'s `stated`-and-undeclared arm to
`BucketChoice::NotFound { searched: vec![stated], missing: Vec::new() }` —

- `tests/bucket.rs:645` — the match's catch-all `other => panic!(...)`, got
  `NotFound { searched: [Name { display: "extras", key: "extras" }], missing:
  [] }`.
- `tests/update.rs:286` — `assert!(!why.contains("searched"), ...)`,
  reproducing `no declared bucket has it (searched: extras)` verbatim.
- `tests/adopt.rs:530` — `assert!(why.contains("does not declare"), ...)`.
  Not the first assertion in that test: `why.contains("extras")` (line 529)
  still passes against the old wording, because `extras` names the bucket in
  both the old and the new message and cannot by itself tell them apart. The
  assertion that discriminates is the one that fired.
