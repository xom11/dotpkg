# Measurements: making the record self-checkable, and closing the three items that only needed doing

Round run 2026-08-12 on this macOS machine and on a14 (`zenbook-a14`), against
`main` at **`3666d38`**, landing on **`b795d9e`** and the commit that carries
this file.

**Every probe on a14 is read-only or confined to this round's own prefix.**
`winget source update` (which refreshes the local index and installs nothing),
`Get-Process`, `Get-FileHash`, `cargo test` and `cargo mutants` inside
`C:\Users\kln\ph6-build`. kanata was never started, stopped or signalled;
its pid was **9644** at the start of the round and is recorded as a landmark,
not asserted. Artefacts all carry the `ph6-` prefix so cleanup can name exactly
what it owns: `ph6-build`, `ph6-target`, `ph6-mutants`, `ph6-dotpkg.tgz`,
`ph6-idle-gate.ps1`, `ph6-idle-baseline.ps1`, `ph6-names-windows.txt`,
`ph6-deelev-result.txt`. Phase 4b's `dotpkg-build` and `p4b-dogfood`, the
previous round's `p6-*`, and `C:\Users\kln`'s unrelated session work were not
touched.

## The headline

1. **The citation class had recurred, and the existing gate could not see it.**
   Six citations in shipped `.rs` files were stale on `3666d38`, all six passing
   the "does the cited line exist" check, two of them citations an earlier phase
   had already corrected once. They are now gone as a *shape*: `src/` and
   `tests/` hold zero line citations, and a test in the suite refuses to let one
   back in.
2. **A content-checking gate over all three directories was measured infeasible
   before it was rejected** -- 221 of 421 citations fire, almost all
   legitimately. That number is why the class is closed at the source instead.
3. **Two citation defect classes nobody had counted.** Ten citations named a
   basename two tracked files share, and 38 named a file not in the repository
   at all. All 430 remaining now resolve to exactly one tracked file.
4. **§11's re-measured `source update` durations were themselves measuring a
   busy machine, and are withdrawn.** On a machine proven idle before and after,
   the steady-state success range is **294-621 ms**, not 1.2-5.4 s -- essentially
   the original 348-623 ms that §11 said had aged.
5. **`package_roots()`'s two survivors are closed**, on both platforms, by one
   assertion. The gate that was "holding six", then "holding three", is now
   holding **one**.

## 1. The citation surface, reproduced and then extended

The prompt's table was reproduced exactly before anything was changed, with a
script rather than by reading, because the script that produced the original
numbers was not kept:

| | prompt | this round | note |
|---|---|---|---|
| citations in `src/` | 24 | **24** | |
| in `tests/` | 9 | **9** | |
| in `docs/` | 435 | **437** | two more; the regex here also admits `.lock`/`.ps1` |
| pointing at a line that exists | 399 | **391 + 8** | the 8 are the ambiguous ones below, which the original count resolved by guessing a basename |
| pointing at an unresolvable file | 36 | **36 + 2** | 31 `design.md`, 4 `progress.md`, 1 ledger path; plus `buckets.ps1` and `p1-report.txt`, two gitignored probe scripts |
| pointing past end of file | 0 | **0** | |

**Two classes the original count did not separate, and both are real:**

- **Ten citations name a basename two tracked files share** -- `execute.rs`,
  `bucket.rs`, `adopt.rs`, `mod.rs`. This is not a nit. `docs/phase5-notes.md`
  cites `execute.rs` line 223 for a group-assignment match arm; resolving that
  basename from the citing document's own directory lands on **`tests/execute.rs`
  line 223, which is blank**, while the sentence means `src/execute.rs`. The two
  files are 1300 lines apart and both resolve, so "the line exists" is satisfied
  either way.
- **Thirty-eight name a file no reader can open.** 31 of them are `design.md`
  with no directory (it is `docs/specs/2026-08-08-design.md`, confirmed by
  line 257 of that file still holding the 1213 ms `winget list` row the citing
  sentence quotes); the rest point into the untracked `.superpowers/` ledger or at
  throwaway probe scripts.

## 2. Why a content gate was rejected, and it is a number rather than an opinion

`docs/phase5-notes.md` states the anchor rule itself: a citation's target is
whatever it pointed at **in the commit that wrote it**, settled by `git show
<origin>:<file>`. That rule is mechanisable -- `git blame` gives the origin of
the citing line, `git show` gives the content then, and the working tree gives
the content now.

**Run over the whole repository: 221 of 421 resolvable citations differ.**

| citing file | drifted | | citing file | drifted |
|---|---|---|---|---|
| `docs/phase3-notes.md` | 48 | | `docs/phase4-notes.md` | 10 |
| `docs/phase5-notes.md` | 28 | | `docs/plans/2026-08-08-phase2b2-executor.md` | 8 |
| `docs/superpowers/plans/2026-08-11-…` | 25 | | others (7 files) | 17 |
| `docs/superpowers/plans/2026-08-10-…` | 20 | | **`src/` + `tests/`** | **6** |
| `docs/specs/2026-08-08-phase2b2-executor-design.md` | 19 | | | |
| `docs/plans/2026-08-09-phase4-backend-winget.md` | 16 | | | |
| `docs/specs/2026-08-11-…` / `2026-08-10-…` | 26 | | | |

**Most of those 221 are not defects.** `docs/plans/` and `docs/specs/` cite code
that had not been written when the plan was written; `docs/phase3-notes.md` is a
closed record whose citations were true about Phase 3's tree. A gate needing a
221-entry allowlist is a gate that gets switched off, and an allowlist that large
is indistinguishable from no gate.

**But the same measurement isolates the surface that *is* live: 6 of the 33
citations in `src/` and `tests/`.**

## 3. The six, each named, and two of them are recurrences

Every one passes "does the cited line exist". Origin is the commit that wrote the
citing line.

| citing site | cites | at its origin it held | at `3666d38` that line holds |
|---|---|---|---|
| `src/apply.rs` | `tests/cli.rs:1000` | `fn a_declared_package_skipped_as_running_is_outstanding_not_success()` | `#[test]` -- off by one |
| `src/backend/winget_exec.rs` | `src/backend/winget.rs:899` | the `no --exact` doc comment | a `push(format!(…))` in `scan` |
| `src/config_edit.rs` | `config_edit.rs:296` | `'"' => in_string = !in_string,` | prose in a doc comment |
| `src/config_edit.rs` | `config_edit.rs:297` | `'#' if !in_string => …` | prose in a doc comment |
| `src/render.rs` | `src/update.rs:403-459` | the winget-resolution block | the block, with a different tail |
| `tests/winget_resolve.rs` | `src/backend/winget.rs:1086` | `Ok(versions_out) if versions_out.code == 0 =>` | a doc comment |

**Rows 2 and 6 are Instance 2 and Instance 3 of `docs/phase5-notes.md`'s own
account, recurring after being fixed.** Task 9d corrected the first from `:867`
to `:899`; the post-merge audit corrected the second from `:870` through `:1054`
to `:1086`. Both drifted again within the same phase. That is the evidence that
decided this round: the convention had already been fixed by hand twice and came
back, so nothing short of removing the shape closes it.

## 4. What was built, and what each half can decide

**The rule: a citation into code names a symbol, never a line.** The repository
had already found this by hand -- *"named rather than cited by line on purpose …
a test name does not drift"* -- and 32 line citations survived the convention,
because a convention nobody enforces is not a gate. §6b of the previous round
says the same thing about a different precondition.

- **`tests/citations.rs`** -- in the suite, so it runs on both platforms and
  inside the Windows shipping tarball. Two assertions: no citation in `src/` or
  `tests/` names a line, and every `path::symbol` citation resolves to a file
  that contains that symbol. **Both were confirmed able to fail**, separately,
  by injecting one of each and watching the matching test go red. The first one
  also failed on its own doc comment on its first run, because that comment
  spelled the banned shape out as an example.
- **`scripts/check-citations.py`** -- `docs/`, in CI, which is where a full
  checkout exists. Every citation must name exactly one tracked file and a line
  that file has. It refuses if it finds zero citations, because a gate that
  scans nothing passes everything.
- **Neither claims the other's scope**, and both say so in their own headers.
  That is Instance 3's lesson: a sweep that does not state its scope is read as
  covering everything.

**What is deliberately not done: the 221 historical citations are not
re-pointed.** They were correct about the trees they were written against.
`tests/cli.rs` already carried the right convention for this -- a
`HISTORICAL, DO NOT RE-POINT` marker naming the tree (`58c8e29`) the numbers are
true about -- and the gate honours it, requiring the marker to name a tree.

**Result on this tree:** `src/` **0**, `tests/` **0**, `docs/` **430 of 430
resolving**, 0 ambiguous, 0 unresolvable, 0 past end of file.

## 5. Still-open item 2: `package_roots()`, closed on both platforms

The record said a mutation run on Windows would close these two, then measured
that it could not, and concluded the fix had to be **one assertion tying
`package_roots()` to the environment it reads**. That assertion now exists. It
**reads** the environment and never sets it -- `std::env::set_var` is
process-global and this suite runs in parallel, which is why
`package_roots_with` was split out in the first place.

| mutant | macOS before | macOS after | Windows after |
|---|---|---|---|
| `package_roots -> vec![]` | MISSED | MISSED | **CAUGHT** |
| `package_roots -> vec![Default::default()]` | MISSED | **CAUGHT** | **CAUGHT** |
| `package_roots_with -> vec![]` | CAUGHT | CAUGHT | CAUGHT |
| `package_roots_with -> vec![Default::default()]` | CAUGHT | CAUGHT | CAUGHT |

macOS: `4 mutants tested: 1 missed, 3 caught` (was 2 missed, 2 caught).
Windows, from `ph6-build`, `-j 2`, two-`--` form, machine gated IDLE at 3.14%
first: **`4 mutants tested in 2m: 4 caught`, 0 TIMEOUT.**

`vec![]` surviving on macOS is not a gap: with both variables unset the real
answer *is* empty, so the mutant is genuinely equivalent there -- the same shape
as `elevated -> Some(true)` in an elevated session. **The pair is closed.**

## 6. Still-open item 3: the idle gate, and its thresholds are measured

`scripts/idle-gate.ps1` samples the process table twice and decides on the
CPU-seconds burned between the samples. `scripts/idle-baseline.ps1` is what its
thresholds come from, so the numbers can be re-derived rather than trusted.

**The first version was wrong, and measuring is what caught it.** It refused a
genuinely idle machine, because its per-process threshold was a share of one
core and Windows' own `MsMpEng`, `dwm` and `System` sit at 5-9% of one core all
the time. Baseline, three consecutive 6 s windows on idle a14 (8 logical cores):

| round | machine busy | largest process | `Win32_Processor.LoadPercentage` |
|---|---|---|---|
| 1 | **3.26 %** | dwm 7.5% of one core | 20 |
| 2 | **3.02 %** | System 6.7% | 16 |
| 3 | **2.85 %** | dwm 7.0% | 6 |

**`Win32_Processor.LoadPercentage` is unusable as a gate signal and is now
recorded rather than decided on.** It read 20, 16 and 6 on the same idle
machine across three rounds a few seconds apart, because it is an instantaneous
sample. The first gate keyed on it at a 15% threshold and would have admitted or
refused the same machine depending on which second it looked.

**Positive control, same machine, minutes apart** -- because a gate only ever
observed passing is a gate nobody has shown can fail:

| | machine busy | verdict | exit |
|---|---|---|---|
| A. quiet | 5.14 % | IDLE | 0 |
| B. three spinning children | **38.3 %**, three processes at ~100% of one core | **NOT IDLE** | **1** |
| C. quiet again | 3.00 % | IDLE | 0 |

The spinners were deliberately *not* on the gate's builder-name list: that
exercises the CPU path, which is the one that has to catch a neighbouring
session the name list does not know about -- the 02:46-03:20 case.

**Observed idle range across all five samples is 2.85-5.14 %**, against a 10 %
threshold. The header's "roughly three times the noise floor" is really about
two times the observed maximum; the separation from a working machine (38.3 %)
is what makes it hold.

### 6b. The Unix half, and the two things measuring it changed

`scripts/idle-gate.sh` and `scripts/idle-baseline.sh` are the same decision on
the other platform, added because the standing rule covers both while the gate
covered one -- and because this round's own macOS mutation run went ahead on a
machine it had measured as busy.

**The macOS baseline is nothing like a14's, and that is the finding rather than
an inconvenience.** Three rounds on this developer desktop (10 logical cores):
**14.42 / 15.60 / 15.68 %**, with `WindowServer` alone at 28.3 % of one core
every round, plus a browser and the editor session. a14, a laptop with a
keyboard remapper and little else, sits at 2.85-5.14 %. **A single default
threshold cannot be right for both**, which is why both gates read theirs from
the environment and both ship a baseline script that prints what to set it to.

**The default stays at 10 %, and it refuses this machine** -- `machine_busy_pct
17.83, VERDICT: NOT IDLE, exit 1`. That is the correct answer, not a
miscalibration: a mutation run competing with a browser and a compositor is
exactly what the rule forbids. **It would also have refused this round's own
macOS run**, which recorded `syspolicyd` at 100 % of one core against a 35 %
per-process rule.

**Positive-controlled at three points, because a gate that only ever refuses is
as useless as one that only ever passes:**

| | threshold | machine busy | verdict | exit |
|---|---|---|---|---|
| A | 10 % (default) | 17.83 % | NOT IDLE | **1** |
| B | 60 % | 16.02 % | IDLE | **0** |
| C | 60 %, four spinners | **51.53 %**, four processes at ~100 % of one core | NOT IDLE | **1** |

**One thing the measurement corrected, in both gates.** The first Unix version
refused an otherwise-quiet machine because `node` was on its
presence-alone-is-disqualifying list, and the editor session runs node. That
list is now only names that exist *while a compile is happening*
(`cargo`, `rustc`, `cc1`, `ld`, `clang`, …); long-lived runtimes are left to the
CPU threshold, which is the signal that actually separates a resident runtime
from a working one. **The same correction was applied to the PowerShell half**,
where the bug was latent -- a14 has no node running, so it never fired there.

## 7. §11's durations, re-measured on a machine proven quiet -- and withdrawn

§6b said the 1.2-5.4 s success range was the measurement most exposed to the
busy machine, that no timestamp was recorded beside it, and that it had to be
taken again before being treated as settled. Taken again, same argv as
production and as §5/§11, with the idle gate run immediately before and
immediately after each block and **every call carrying its own wall-clock
stamp**:

| block | gate before | calls | successes | duration range | over 1 s | gate after |
|---|---|---|---|---|---|---|
| cold | IDLE 3.49% | 10 | 10 | 308, 315, 319, 324, 329, 329, 329, 405, **1340**, **2070** ms | 2 | IDLE 3.43% |
| warm | IDLE 3.20% | 20 | 20 | **294 .. 621** ms | **0** | IDLE 3.59% |

**The 1.2-5.4 s range does not reproduce.** The steady-state range is
294-621 ms, which is essentially §5's original **348-623 ms** -- the number §11
said had aged had not aged; the round that re-measured it was measuring a busy
machine. The 2070 ms outlier is the first call of the cold block.

**What survives, and it is not nothing:** 1 s does not clear a *cold* first
call. So the production comment's original justification is restored for the
steady state and remains false for a cold index, and it now says exactly that.
The delay stays at 1 s: a retry that fires too early fails a second time and
warns, which is the shipped and tested behaviour.

**What this does not touch:** the trigger's rate (0/10, 0/10, 1/10). §6b already
said so, and it is right -- that counts exit codes, not milliseconds.

## 8. A non-elevated Windows session: a third measured negative

Phase 4b measured that `runas /trustlevel:0x20000` leaves `TokenIsElevated` set
and that `schtasks /RL LIMITED` does not lower it. One route was left untried:
the interactive shell holds the user's *filtered* token, so having
`Shell.Application`'s `ShellExecute` launch the process should inherit it.

**Measured, not assumed** -- Phase 4b assumed a de-elevation once and was wrong:

| | |
|---|---|
| ssh session elevated | `True`, session id **0** |
| `explorer.exe` present | **4 processes, all session 1** (a desktop is logged in) |
| process launched via `Shell.Application.ShellExecute` | `isinrole_admin=True`, **High Mandatory Level** (S-1-16-12288) |

The launched process runs in session 0 with the same token. **This route does
not de-elevate either.**

**Two further routes were tried and neither produced an observation** -- which
is a different and weaker result than "measured not to de-elevate", and is
recorded as such so a later round does not read it as settled:

| route | what happened | what it settles |
|---|---|---|
| `schtasks /RL LIMITED /IT` (Phase 4b tested `/RL LIMITED` without `/IT`) | task created and triggered, `Last Result: 0`, but `Status: Queued` and no output after 30 s | **nothing about elevation** -- the task never ran |
| duplicate `explorer.exe`'s token, `CreateProcessWithTokenW` (needs `SeImpersonatePrivilege`, which an elevated admin holds, unlike `CreateProcessAsUser`) | process created, `win32_error 0`, in session 1; redirected output file **0 bytes** across two variants | **nothing about elevation** -- no output was recovered |

**The guard was positive-controlled while finding this out:**
`scripts/nonelevated-mutants.ps1` run over ssh reported `session_id 0`,
`isinrole_admin True`, `High Mandatory Level`, and refused with exit 1. It
cannot pass from the wrong place.

**It was then run by hand from the desktop, and it closed the gate.** The
session is non-elevated by two APIs independent of the function under
measurement -- `session_id 1`, `IsInRole(Administrator) False`, **Medium
Mandatory Level (S-1-16-8192)** -- with the machine gated IDLE at 0.97% first.

**`6 mutants tested in 2m: 4 missed, 2 caught`**, and the four missed are named
in the output, so the two caught are `sys.rs:139 -> None` and
**`sys.rs:139 -> Some(true)`** -- the mutant the whole exercise existed for.

**Every one of the six, across all three runs, and each MISSED has a mechanism
rather than a shrug:**

| mutant | Windows elevated | Windows ordinary | macOS | killed by |
|---|---|---|---|---|
| `sys.rs:139 -> None` | CAUGHT | **CAUGHT** | MISSED | either Windows session |
| `sys.rs:139 -> Some(false)` | CAUGHT | MISSED | MISSED | elevated only |
| `sys.rs:139 -> Some(true)` | MISSED | **CAUGHT** | MISSED | **ordinary only** |
| `sys.rs:163 != -> ==` | CAUGHT | MISSED | MISSED | elevated only |
| `sys.rs:216 -> Some(true)` | MISSED | MISSED | CAUGHT | macOS only |
| `sys.rs:216 -> Some(false)` | MISSED | MISSED | CAUGHT | macOS only |

- `:139 -> Some(false)` survives an ordinary session because `Some(false)` **is**
  the correct answer there -- the exact mirror of `Some(true)` surviving an
  elevated one.
- `:216`'s two are in the `cfg(not(windows))` arm and are not compiled on
  Windows at all, so both Windows runs are blind to them by construction.
- **`:163` is the one that needed explaining, and the explanation is a
  deliberate production decision rather than a hole.** In an ordinary session
  `CheckTokenMembership` honours the filtered token's DENY_ONLY Administrators
  group, so `in_admins` is `Some(false)`. Inverting the `TokenIsElevated`
  comparison turns `is_elevated` from `Some(false)` into `Some(true)`, which
  lands in `verdict`'s **third** arm -- `(Some(true), Some(false)) =>
  Some(false)`, the measured restricted-token case that exists because winget
  succeeded there and dotpkg must not refuse. That arm returns the same answer
  the unmutated code returns, so the mutant is genuinely equivalent in this
  session and dies in the elevated one, where `in_admins` is `Some(true)` and
  the same flip changes the verdict.

**So the rule the record needed is stronger than "run it on both platforms".**
It is: **this function needs three runs -- macOS, an elevated Windows session,
and an ordinary one -- because each is blind to what the other two can see, and
no two of them together cover all six.** Phase 5 recorded a gate "holding six",
the residual round corrected that to three, and the correct number was always
zero-with-three-runs and six-with-any-two.

`mutants_exit: 2` is cargo-mutants' code for "some mutants missed", not a
failure; the baseline's `0s test` is one test running. `tests/cli.rs` now carries the test that window
would run, `#[ignore]`d, asserting `elevated() == Some(false)` and failing
loudly if the session it is run from is elevated.

## 9. Verification

- **macOS**: `cargo test --all` **646 passed / 0 failed**, **15** `test result:`
  lines. `cargo fmt --check` clean, `cargo clippy --all-targets -D warnings`
  clean on the host and on **`aarch64-pc-windows-msvc`**.
- **Windows**, shipped as a tarball carrying `SHIPPING-SHA.txt` and a
  `SHIPPING-MANIFEST.txt` naming a sha256 for all **73** files:
  `manifest entries : 73     verified equal : 73`, `mismatched : 0    missing :
  0    unlisted on disk : 0`. That last counter is the one that rules out
  extracting over an older tree, and it subsumes the separate fixture check --
  `tests/fixtures/winget/list-full.txt` is one of the 73 verified by sha256.

  **It ran twice, because the tree moved after the first run.** On `b795d9e`:
  exit 0, 644 passed / 0 failed / 1 ignored, 15 result lines, and a name
  cross-reference of macOS 646 / Windows 645 / common 644 with the long-standing
  three `cfg` exclusions. That tree was then superseded by the commit
  withdrawing the duration claim, which touches `src/backend/winget.rs` and
  `tests/cli.rs` (`git diff --numstat`: 28/15 and 53/0), so it was shipped and
  run again.
- **Windows suite on the tree this round ends on** (`060a124`):
  `cargo test --no-fail-fast` **exit 0, 644 passed / 0 failed / 2 ignored**,
  **15** `test result:` lines.
- **Name by name, from `--list`, never from run output, and never by
  subtracting totals**: macOS **646**, Windows **646**, common **644**. The
  difference set is now **four** `cfg` exclusions rather than three, and the
  fourth is this round's own: the two `#[cfg(unix)]` tests absent on Windows
  (`a_failed_last_write_leaves_a_prefix_that_plan_does_nothing_about`,
  `a_root_reached_through_a_symlink_still_matches_running_processes`) and the
  **two** `#[cfg(windows)] #[ignore]` tests absent on macOS -- the existing
  elevated one and the non-elevated mirror added here.
- **The rebuild was checked by content, not by its own report.** `git archive`
  stamps every file with the commit time, which can be older than the previous
  build's artifacts, so a 4-second "build_exit=0" is exactly what a skipped
  rebuild would also print. What settles it is that `--list` on the machine now
  returns **646** names including
  `on_an_ordinary_windows_session_elevated_answers_some_false`, which did not
  exist in the previous tree.
- **Mutation runs on both platforms**, `-j 2`, 0 TIMEOUT, machine state recorded
  beside each. The macOS machine was **not** idle for its run (load 2.34 rising
  to 10.01, `syspolicyd` at 100% of one core from this round's own builds) and
  that is stated rather than discovered later; §6b's finding that CAUGHT and
  MISSED do not depend on timing is why the verdicts still stand, and there were
  no timeouts to be explained.

## 10. Method failures of my own, this round

1. **I reverted uncommitted work with `git checkout <file>` while
   positive-controlling the docs gate.** The probe line I wanted to undo was in
   `docs/phase5-notes.md`, which also held this round's unfixed citations, so the
   checkout took both. The gate caught it on the next run -- which is the gate
   working -- but the right move was to commit first or probe a copy.
2. **I reported an exit code that was a pipeline's.** `python3 … | tail; echo $?`
   reports `tail`. The same trap the previous round recorded for `scp | grep`,
   walked into one command after reading about it. Re-run without the pipe.
3. **I uploaded two scripts without this round's `ph6-` prefix**, briefly
   putting un-prefixed files in a directory that holds another session's work.
   Removed in the next call, and the removal is in the transcript rather than
   asserted here.
4. **My first idle gate refused an idle machine**, because its thresholds were
   chosen rather than measured -- the exact defect class this phase exists to
   close, committed inside the tool built to close it. Caught by running it.
5. **A `-match 'FAILED'` filter reported every passing line as a failure.**
   PowerShell's `-match` is case-insensitive by default, so it matched `failed`
   in `0 failed`. Harmless because `cargo_test_exit=0` was captured separately,
   which is the point: the exit code is the verdict and the text filter is not.
6. **This document failed the gate it describes**, on both of the classes it
   reports, because it quoted the bad forms literally as examples. It also
   revealed that the gate reads `git ls-files`, so a document that has not been
   `git add`ed yet is not scanned at all -- the first run "passed" by skipping
   this file. Both are now what the gate reports; the shape is written out in
   prose here instead.
7. **`Get-Content` on a one-line file returns a string, not an array**, so
   `[0]` took a character and the runner died silently after printing the
   manifest result. The failure looked like a dropped connection, which is a
   failure mode this project already has, and I nearly attributed it to that.

## 11. The automated Windows gate had been red for ten commits, and nobody looked

Found by checking whether the CI step added in this round had ever been observed
to run. It had not -- and the reason turned out to be much larger than the step.

**`windows-latest` had failed on every push since `98f3d33` (2026-08-10). The
last green run was `5bd69c3` (2026-08-09): ten consecutive commits, spanning the
end of Phase 4b, all of Phase 5, the residual round and this one.** No phase
document mentions it. `ubuntu-latest` and `macos-latest` were green throughout,
which is why nothing announced itself.

**This is expensive in a specific way.** Three consecutive phases built a manual
Windows verification apparatus -- a hashed 73-file shipping manifest, an ssh
session to a14, a name-by-name cross-reference, a first-ever `cargo mutants` run
on Windows -- while the *automatic* Windows gate sat red and unwatched. It is
this project's own fourth defect class (a gate whose result nobody reads) at the
top level, and it survived because every phase's Verification section describes
what that phase ran by hand.

**One test, and the mechanism is measured with a control rather than reasoned.**
`tests/prepare.rs`'s `a_commit_the_bucket_does_not_have_names_the_fetch_that_would_get_it`
asserts the refusal message contains the bucket directory, comparing against the
**raw** `tempfile` path. `Scoop::new` canonicalizes its root, so the message
carries the **true on-disk spelling**. Where the two differ, `String::contains`
-- case-sensitive, and knowing nothing about 8.3 aliases -- fails.

Reproduced on a real Windows machine by making `TEMP` an 8.3 alias, with the
long form as the control, in the same session:

| | `TEMP` | before the fix | after the fix |
|---|---|---|---|
| control | `C:\Users\kln\ph6-longtemp-directory` | 1 passed | 1 passed |
| probe | `C:\Users\kln\PH6-LO~1` | **1 failed**, at exactly that assertion | **1 passed** |

`windows-latest` puts `TEMP` under `runneradmin`, which shortens to `RUNNER~1`;
a14's `C:\Users\kln` is already short and correctly cased, which is why the
machine this project verifies on could never reproduce what CI was reporting.

**The product message was never wrong** -- the resolved path is the one a user
can paste into `git -C`. The expectation was, and it now asks the object under
test which root it resolved, through the public `Scoop::root()`.

**Observed green afterwards, rather than assumed:** all three jobs succeeded,
and the `docs/` citation gate ran on **all three runners** with identical counts
(35 files, 436 citations), which also settles the open question of whether
`python3` exists on the Windows runner -- it does, and this is the first run in
which that step ever executed there.

## 13. An explanation of mine, offered confidently and measured false

The first non-elevated mutation run failed at the baseline because cargo could
not start one test binary:

```
could not execute process ...\ph6-target\debug\deps\update-<hash>.exe (never executed)
Caused by: The requested operation requires elevation. (os error 740)
```

**That much is measured.** What followed was not. I explained it as Windows' UAC
installer detection, which flags an executable whose filename contains
`install`, `setup`, `update` or `patch` -- a real, documented behaviour, which
fits the observation exactly, and which I wrote into this document, into
`docs/phase5-notes.md`, into a script header and into two commit messages before
testing it.

**The experiment that settles it, with the control built in.** One binary,
copied to three names -- same bytes, same sha256, same (absent) manifest, so the
name is the only thing an outcome difference can be about. Run from a session
proven non-elevated (session 1, `IsInRole` False, Medium Mandatory Level):

| name | contains a keyword | result |
|---|---|---|
| `update-<hash>.exe` (the original) | yes, `update` | **LAUNCHED**, exit 0 |
| `ph6-neutral-probe.exe` | no | **LAUNCHED**, exit 0 |
| `ph6-setup-probe.exe` | yes, a *different* one | **LAUNCHED**, exit 0 |

~~**All three launch. The filename explanation is false**, and the third name is
why the refutation covers the class rather than one file.~~ ~~**And the symptom
does not reproduce at all** -- same file, same path, same kind of session:
`os error 740` then, exit 0 now.~~

**Both of those are withdrawn within the hour, and the second correction is
worth more than the first.** Two things came in:

1. **The symptom is reproducible.** `cargo test --no-fail-fast` from an ordinary
   session fails on the same binary with the same `os error 740`, after the
   probe above had reported it launching. So "does not reproduce" was false.
2. **The probe that produced the table above cannot be trusted, and the flaw is
   mine.** It decided `LAUNCHED` from `$LASTEXITCODE` alone. **PowerShell leaves
   `$LASTEXITCODE` at its previous value when it fails to start a native
   command**, so a stale `0` is indistinguishable from a success, and the
   scan for an error string can miss a message it did not anticipate. The probe
   never checked that the binary produced any *output*. A libtest binary asked
   to `--list` prints one line per test; checking for those lines is the
   difference between observing a launch and assuming one.

   **This is defect class 4 -- a check whose output narrates its own result --
   committed inside the tool written to refute a claim, one commit after the
   document describing that class.**

**So the position is now precisely this, and it is smaller than either previous
version of it:**

| claim | status |
|---|---|
| `cargo` cannot start `update-<hash>.exe` from a non-elevated session, `os error 740` | **measured, reproduced twice** (mutants baseline, then the full suite) |
| the cause is the filename (UAC installer detection) | **untested** -- the probe that "refuted" it was unsound |
| a `RUNASADMIN` compatibility layer | **negative**, and this one holds: it was a registry read, not a launch |
| a zone-identifier stream | **negative**, same reason: 0 alternate data streams |
| the same binary launched *directly* rather than by cargo | **unknown** -- this is the axis the corrected probe adds |

`scripts/uac-name-probe.ps1` was rewritten to verify every launch **by output**
and to separate the two axes -- three names, and two launchers.

### 13a. Run properly, it confirms the original explanation

Same session, output-verified, one binary under three names:

| name | keyword | test names printed | verdict |
|---|---|---|---|
| `update-<hash>.exe` (the original) | `update` | **0** | DID NOT RUN |
| `ph6-neutral-probe.exe` | none | **24** | **RAN** |
| `ph6-setup-probe.exe` | `setup` | **0** | DID NOT RUN |
| the original, launched by cargo | `update` | 0 | DID NOT RUN, `os error 740` |

**The filename is the cause, and it is a class rather than one file** -- a
*different* keyword fails the same way, and a name with none runs and prints 24
test names. Same bytes, same sha256; the name is the only variable. Axis B adds
nothing: cargo and PowerShell fail identically, so it was never about who calls
`CreateProcess`.

**And the two `exit=0` values next to `DID NOT RUN` are the proof of the first
probe's flaw**, not an inconsistency: the process never started, and
`$LASTEXITCODE` kept the previous command's zero. The first probe read exactly
that and called it a launch.

**So the sequence, kept whole because the shape of it is the lesson:** a correct
explanation was offered without being tested; an unsound probe appeared to
refute it; the correct explanation was withdrawn on that evidence; a probe that
checked *output* rather than an exit code restored it. The original claim was
never the problem. **Asserting it without measuring it was**, and so was
accepting a refutation that measured nothing.

**One cosmetic flaw remains in the probe and is not worth a round trip:** its
error-scan matched the substring `cannot` inside the test name
`a_winget_resolution_cannot_carry_a_commit`, so the RAN row prints a spurious
`error:` line. The verdict does not depend on it -- 24 printed test names is the
verdict.

**Why all of this is in the record rather than tidied away.** The first
explanation was plausible, fitted the evidence, named a real Windows behaviour,
and went into two documents and two commit messages before it was tested. The
first refutation of it was itself unsound in the exact way this document spends
a section warning about. Both were committed by the person who built the gates,
within an hour of building them, and both were caught the same way: by running
the thing again and looking at what it actually printed.

## 12. Still outstanding

1. ~~A non-elevated Windows session~~ -- **done, §8.** `elevated -> Some(true)`
   is CAUGHT, and with it items 15 and 19 are closed and the `sys.rs` gate holds
   nothing. What replaces it is an **open question with a refuted answer**, §13.
2. **Item 20's observation** -- unchanged by this round. 70 contended rounds
   remains the bound. §7 above removes one of the arguments that had been read
   as evidence against the retry's delay.
3. **The 221 historical citations remain stale against today's code**, by
   decision. What has changed is that they all resolve to a file a reader can
   open, and that no new one can be written into `src/` or `tests/`.
4. **`docs/` has no drift check and will not get one from this round.** The
   number that says why is 221 of 421.
5. ~~The idle precondition is enforceable on Windows and only prose on macOS~~ --
   **closed, §6b.** Both halves exist, both are positive-controlled, and the
   macOS one refuses this machine at its default, which is the right answer.
   What it does **not** do is get wired into anything: no `cargo mutants`
   invocation is gated on it automatically, so it is still a gate someone has to
   remember to run. That is one step better than prose and one step short of a
   precondition.
6. **The `docs/` gate reads `git ls-files`, so a file that has not been
   `git add`ed is not scanned.** It skipped this document on its first run. CI
   is unaffected, because a checkout only contains tracked files; a local run
   before staging can still mislead.
