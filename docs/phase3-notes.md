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

**Result: 363 passed, 0 failed** (macOS: 365; the same two `#[cfg(unix)]`
tests account for the difference). `tests/cli.rs` goes 17 → 23 on Windows, so
all six new CLI tests — including `adopt --state some/relative/path.json`,
which is a path-shaped refusal, and the `bucket_only` fixture that removes a
`pkg.lock` after creating a git bucket — pass natively.

**Rule this makes explicit for the next phase: the Windows run belongs at the
end of the change as well as before the dogfood.** "Run Windows first" is not
the same instruction as "run Windows on what you ship", and this task needed
both.

### Four predicted Windows failures, three of them falsified

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
point its only survivors were `bucket.rs:99` and the two at `bucket.rs:191`,
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
ACCEPTED as very likely equivalent. `changed() < 0` on a `usize` is always
false, leaving `touched() > 0`. The comment above that line argues `touched()`
catches cases `changed()` misses, which implies `touched() ⊇ changed()` — and if
that containment holds, the `changed()` disjunct is redundant and the mutant is
behaviourally identical. Not proven either way here. Phase 4 should either prove
the containment and delete the disjunct, or find the case where they differ and
test it.

## The negative-control audit, re-derived and fired

The ledger records that **nine control sets this plan specified were un-fireable
or mis-aimed**, every one caught by an implementer *running* the control rather
than by the plan author writing it. So this audit did not re-read the plan's
control list. Each of the four controls the plan singled out was re-derived from
what the code can actually do wrong, then **applied to a copy of the tree and
run**, and the assertion that actually fired was recorded.

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
   `adopt::run` is currently undetectable. **This is the highest-value open item
   in this document.** The fix is small: have `write_in_order` take a single
   ordered structure, or have the seam test call `run` rather than
   `write_in_order`, or assert the order through a recording fake threaded from
   `run`. Left open deliberately rather than fixed under a review task, because
   it is a production-shape change and deserves its own review.

This also answers the plan's Step 6 question directly. **The write-order test in
`adopt` does discriminate the orders — but not on the assertion its own name and
comments point at, and not on any platform where `#[cfg(unix)]` is false.** The
induced failure is a read-only parent directory, not a rename onto a directory,
so the Unix/Windows error-shape difference the plan worried about never arises;
the real problem is simpler and worse, namely that the test does not exist on
Windows at all.

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
- **The three Task 6 coverage gaps that remain**: a lock or opt naming a bucket
  `pkg.toml` does not declare (`NotFound { searched: [stated] }`) has no test; a
  declared bucket without `.git` being skipped has no test; and — the one that
  matters most — **no test proves the lock beats a PRESENT, CONFLICTING opt**.
  Every lock test runs with no `[scoop.opts]` at all, so it proves the lock works
  *alone*, not that it *wins*. That is the precedence claim's core and it is
  still inspection-only. `choose_bucket`'s mutants all died, so nothing here is
  currently wrong — but the claim in the doc comment is stronger than the
  evidence. **Recommended for Phase 4**, since winget will add a second consumer
  of the same precedence rule.
- **The no-`.git` fetch-loop guard (`src/update.rs:238-240`) has no direct
  test.** Its mutant did not survive, so it is covered in effect; the
  closely-related `clone_missing_buckets` `.git` guard in `backend::scoop` did
  survive and is listed above. **The winget warning is now closed** — its mutant
  did survive, which is what found it.

## Still open

1. **`mass_prune_guard` reads scoop only.** Top of this file. Must grow a backend
   loop in the same change that adds winget.
2. **`adopt::run`'s call-site write order is unprotected on Windows.** Only a
   `#[cfg(unix)]` test catches a reversal. Highest-value open item.
3. **`main.rs:411`'s exit-code floor has no reachable test.** Recommend
   extracting it as a pure function in Phase 4.
4. **`render.rs:181` may be an equivalent mutant.** Prove the `touched() ⊇
   changed()` containment and delete the redundant disjunct, or find the case
   where they differ.
5. **The lock-beats-a-conflicting-opt precedence claim is inspection-only.**
6. **`config_edit::save` / `lock::save` / `state::save` temp-file cleanup gap**,
   all three identical.
7. **15 surviving mutants in `src/backend/scoop.rs`**, listed above. Phase 2b
   code, deliberately out of scope for a Phase 3 review, and the largest single
   concentration on the branch. `Backend::name` being asserted by nothing is the
   one to do first.
8. **`State::names` has no callers.** Give it one or delete it.
9. **The `Err(e) if e.kind() == NotFound => <benign default>` idiom is tested
   only on its benign path**, in three separate places. `lock.rs:99` was closed
   here; `verify.rs:146` and `backend/scoop.rs:227` were not.
