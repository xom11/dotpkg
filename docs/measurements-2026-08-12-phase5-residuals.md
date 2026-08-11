# Measurements: closing Phase 5's residuals, and what the Windows gate is actually holding

Round run 2026-08-12 on a14 (`zenbook-a14`, winget `v1.29.280`) and on this
macOS machine, against `main` at **`c7086f0`**.

**Every probe on a14 is read-only.** `dotpkg status` (which mutates nothing),
`winget list`, `winget show`, `Get-Process`, `Get-ChildItem`, `Get-FileHash`,
and `rg` searching for a pattern that does not exist. No winget write verb was
invoked. kanata was never started, stopped, or signalled.

**Machine left as found, proven after the last probe:** kanata
`kanata_windows_tty_winIOv2_arm64` **PID 13676** before and after, `pkg.toml`
sha256 `32a238ff…` unchanged, no `pkg.toml.bak`, **31** scoop apps,
`WinGet\Links` still **5** entries, **0** `rg` processes alive.

Artefacts on a14, all under names this round chose so cleanup is unambiguous:
`C:\Users\kln\p6-build` (the shipping tree), `p6-item21`, `p6-suite`,
`p6-verify.ps1`, `p6-item21.ps1`, `p6-item17.ps1`, `p6-suite.ps1`,
`dotpkg-c7086f0.tgz`. `dotpkg-build` and `p4b-dogfood` are **Phase 4b's** and
were not touched — `dotpkg-build/SHIPPING-SHA.txt` reads `a9a6637`, 35 commits
behind `main`.

## The headline

1. **Still-open item 21 is closed.** The winget path signal is now observed
   firing on real hardware with both name signals proven dark. It took a
   different subject than the dogfood had, and four counterweights rather than
   one.
2. **Still-open item 19's premise is false, and this is measured, not argued.**
   A `cargo mutants` run on Windows **cannot** close `package_roots()`'s two
   survivors, and can reach at most **3** of the `sys.rs` four — and only under
   conditions the record never states. The "single gate holding six mutants" is
   holding three.
3. **The inherited `floor_char_boundary` count was never a stable quantity.**
   On Phase 4b's own tree, three completed runs of the identical scope produced
   three *different* survivor sets, and a fourth attempt aborted at baseline. On
   `c7086f0` the same scope produces the same set three times out of three.
   Phase 5's own background-maintenance fix is what made the number measurable.
4. **A portable package started through its `WinGet\Links` shim is still caught
   by the path signal**, even though every Windows API this round asked reports
   the process image as the *link*. That is a wider coverage claim than the
   record makes, and its mechanism is reasoned, not measured.

## 1. Item 21: the path signal, isolated

### Why the dogfood could not do this, and what changed

Task 9's stage A1 could not separate `Running.dirs` from the name half because
a14's only live process under a `Packages\<id>_…\` directory was `VKey.exe`,
and `PhatMT97.VKey` is reachable by a name signal too. The fix is a different
subject, not a different method.

**Subject: `BurntSushi.ripgrep.MSVC`**, installed `15.2.0`, `Source: winget`,
`portable`. Its executable is
`…\WinGet\Packages\BurntSushi.ripgrep.MSVC_Microsoft.Winget.Source_8wekyb3d8bbwe\ripgrep-15.2.0-aarch64-pc-windows-msvc\rg.exe`
— **one directory deeper than the package root**, which does not matter:
`running_ids` takes `rest.split('/').next()`, the first segment after the root,
so nesting below it is invisible to the match.

Every half of `Running::covers` except `dirs` is dark for this package:

| disjunct in `covers` | what it compares | live process needed | present? |
|---|---|---|---|
| `dirs.contains(name)` | scanned id vs `<root>/<id>_…` | `rg.exe` under `Packages\BurntSushi…` | **the variable** |
| `names.contains(name.key())` | `burntsushi.ripgrep.msvc` | a process folding to that | no |
| `bins.any(in names)` | `guard_names` = `["msvc", "ripgrep msvc"]` | a process folding to either | no |

`guard_names("BurntSushi.ripgrep.MSVC", "RipGrep MSVC")` yields **`["msvc",
"ripgrep msvc"]`** — the id's last dotted segment and the folded display name.
Neither is `rg`. No `[winget.guard]` entry was declared, so `bins` holds
nothing else.

### The trap that had to be avoided first

**`rg` on a14's `PATH` is scoop's, not winget's**: `Get-Command rg` resolves to
`C:\Users\kln\scoop\shims\rg.exe`, and a14's real `pkg.toml` declares scoop
`ripgrep` on line 4. Launching `rg` the obvious way would have produced a
process under `$SCOOP/apps/`, which is scoop's path signal and not winget's.
Every run below launches the winget binary **by absolute path**.

### Results

Run from `C:\Users\kln\p6-item21` with its own `pkg.toml` declaring only
`BurntSushi.ripgrep.MSVC` and its own `pkg.lock`. a14's real `pkg.toml` was
never the input and is byte-unchanged.

**Phase A — lock `0.0.1-dogfood` (stage A2's shape, reproduced):**

| run | winget `rg` alive? | line printed | counters |
|---|---|---|---|
| A1 | no | `! … 15.2.0 -> 0.0.1-dogfood  (dotpkg will not downgrade …)` | `0 change(s), 0 skipped, 1 winget downgrade(s) that will be refused` |
| A2 | **yes** (pid 5416) | `! winget BurntSushi.ripgrep.MSVC running -- stop it first` | `0 change(s), 1 skipped` |
| A3 | no (killed) | identical to A1 | identical to A1 |

**Phase B — lock `99.0.0`, so the counterweight is a real action rather than a
refusal:**

| run | winget `rg` alive? | line printed | counters |
|---|---|---|---|
| B1 | no | `^ winget BurntSushi.ripgrep.MSVC 15.2.0 -> 99.0.0  (upgrade)` | `1 change(s), 0 skipped` |
| B2 | **yes** (pid 16596) | `! winget BurntSushi.ripgrep.MSVC running -- stop it first` | `0 change(s), 1 skipped` |
| B3 | no (killed) | identical to B1 | identical to B1 |

**The confound check ran on every one of the eight `status` invocations**: the
count of live processes whose folded name is in `{msvc, ripgrep msvc,
burntsushi.ripgrep.msvc}` was **0** immediately before and immediately after
each run. Without that number the runs would prove the fence held, not which
signal held it — which is exactly what stage A1 was missing.

**Probe C — a process named `rg` that is not under `Packages`.** Scoop's
`rg.exe` started instead, producing **two** live `rg` processes
(`scoop\shims\rg.exe` pid 14620 and `scoop\apps\ripgrep\current\rg.exe` pid
15288). `status` printed `^ winget … (upgrade)`, `1 change(s), 0 skipped` — **no
skip**. So the match is by path under the winget package root, not by the mere
existence of a process called `rg`.

**Conclusion, and its exact width.** With the lock the only thing held constant
and the live process the only thing changed, `status` flips between an action
and `running -- stop it first`, in both lock directions, with both name
disjuncts measured dark. **The `dirs` half of `Running::covers` is the only
thing that can have produced the skip.** Item 21 asked for exactly this and it
is now observed.

**What it does not prove.** The confound count samples immediately before and
after each `status`, not during it; a process named `msvc` that appeared and
vanished inside a ~2 s window would defeat it. Nothing else about the guard is
widened: coverage is still bounded to `portable` packages, still 4 of 36 ids on
this machine.

## 2. A `Links`-shim process is caught too, and the reason is reasoned

Not part of item 21. Item 21 never asks it, and it is the question a user's
actual habits raise: a portable winget package is normally started through its
`%LOCALAPPDATA%\Microsoft\WinGet\Links\<name>.exe` shim, not by its real path.

**Measured.** `rg` started via `…\WinGet\Links\rg.exe` (pid 12720): `status`
printed `! winget BurntSushi.ripgrep.MSVC running -- stop it first`,
`0 change(s), 1 skipped`, with the confound count 0 either side. **The path
signal fires for the shim invocation.**

**And the surprise:** every Windows API this round asked reports that process's
image as the *link*, not the target.

| source | value for a `Links`-launched `rg` |
|---|---|
| `Get-Process .Path` | `…\WinGet\Links\rg.exe` |
| `MainModule.FileName` | `…\WinGet\Links\rg.exe` |
| WMI `Win32_Process.ExecutablePath` | `…\WinGet\Links\rg.exe` |
| what `running_ids` must have seen | a path under `…\WinGet\Packages\BurntSushi…` |

`…\WinGet\Links\` is not a prefix of `…\WinGet\Packages\`, so `running_ids`
could not have matched the value those three APIs report — yet it matched.

**Reasoned, not measured:** the three APIs above all read the process's PEB,
which records the path as given at load; `sysinfo` (which is what
`sys::running_processes` uses) reads the kernel's own image name, which is the
file the kernel actually opened — the symlink target. **Nobody has printed
`sysinfo`'s value for that pid**, so the mechanism is an explanation for an
observation, not itself an observation. A dozen-line probe would settle it and
is the obvious next step.

**Consequence for the record, and it cuts the reader's way:** §1 of the
2026-08-11 document categorised live processes by a PowerShell-captured `Path`.
For a symlinked portable shim that view and dotpkg's disagree, so a count of
"processes under `Packages`" taken that way is a **lower bound** on what the
fence can see, not the figure itself.

## 3. Item 17: the two winget spellings, measured for the first time

Item 17 is labelled *reasoned only* — "nothing in this phase's measurement
document compares the two spellings for one package". Now something does.

`Running.dirs` holds `winget list`'s `Id` as `parse_list` read it;
`Step::app()` for a `WingetStep::Set` holds `winget show`'s `Id` as
`parse_show` read it out of the `Found <name> [<id>]` line. `Name` folds case,
so only a difference that is **not** case can make the `dirs` half answer "not
running" mid-run.

Surveyed **all 36** source-backed installed ids on a14 (the 35 `dotpkg status
--show-unmanaged` reports plus the one declared package), one `winget show
--id <id> -e --disable-interactivity` each:

| | |
|---|---|
| byte-identical | **36** |
| differ by case only | **0** |
| differ by more than case | **0** |
| `show` produced no `Found` line | **0** |

**The residual does not close, and the label changes.** 36 ids on one machine
is not a guarantee about winget; it is the first evidence either way. Item 17
stops being *reasoned only* and becomes *measured on 36 of 36, no difference
observed*.

## 4. Item 19: what a Windows mutation run can actually close

### 4.1 `package_roots()`'s two survivors: not closable by a Windows run

The record's reasoning is that `vec![]` survives on macOS because
`LOCALAPPDATA` and `ProgramFiles` are unset there, so the mutant's output
equals the real one — and that on Windows, where they are set, "a test
exercising that path would see a real, non-empty, correctly-shaped answer that
either mutant would visibly break".

**Measured, by supplying exactly the difference Windows supplies.** With
`LOCALAPPDATA=/tmp/fakelocal ProgramFiles=/tmp/fakepf` set for the whole run on
macOS — so `package_roots()` returns a real two-element vector — the same two
mutants were re-tested:

```
4 mutants tested in 40s: 2 missed, 2 caught
MISSED src/backend/winget.rs:251:5: replace package_roots -> … with vec![]
MISSED src/backend/winget.rs:251:5: replace package_roots -> … with vec![Default::default()]
```

The two `package_roots_with` mutants are caught, as the record says. **The two
`package_roots()` mutants survive with the environment set.** The environment
is not what keeps them alive.

**What keeps them alive is structural and platform-independent.** `running_ids`
only ever returns ids that are in `scanned`, and no test in this suite produces
a non-empty winget scan: `tests/cli.rs` gives every spawned `dotpkg` a `PATH`
with winget stripped (`path_without_winget()`, the single spawn site for the
binary under test), and every other test calls library functions with
fabricated data. With `scanned` empty, `running_ids` returns the empty set for
*any* roots, so no observable output depends on `package_roots()`'s value.
Windows changes none of that.

**A correction the record needs:** it also says "Nothing in the suite calls
`package_roots()`". The suite does call it — `tests/cli.rs` spawns the real
binary, whose `status`/`apply` path reaches `apply::sample_fence` and therefore
`package_roots()`. The true statement is the weaker one: **nothing in the suite
*asserts* anything that depends on its return value.** That difference matters,
because the two have different fixes, and only the weaker one is about tests.

### 4.2 The `sys.rs` four: at most three, and only from an elevated session

**Measured on macOS**, `cargo mutants -f src/sys.rs`:

```
15 mutants tested in 54s: 4 missed, 10 caught, 1 unviable
MISSED src/sys.rs:139:5  elevated -> None / Some(true) / Some(false)
MISSED src/sys.rs:163:71 replace != with == in elevated
CAUGHT src/sys.rs:216:5  elevated -> Some(true) / Some(false)
```

`:216` is the `cfg(not(windows))` arm, and what kills it is
`elevated_answers_or_admits_it_does_not_know`'s `assert_eq!(answer, None)` —
which exists **only** in that arm. Its Windows arm is `let _ = answer;`, which
asserts nothing.

**So on Windows the only test that asserts a value from `sys::elevated()` is
`tests/cli.rs`'s `on_a_real_elevated_windows_session_…`, the single `#[ignore]`
in the repository.** `cargo mutants` runs `cargo test`, which skips it — Phase
5's own Windows runs report `1 ignored` for exactly that reason. A default
`cargo mutants` run on Windows therefore reproduces "4 missed" and settles
nothing.

**With the ignored test included, and from an elevated session, three of the
four die and one cannot:**

| mutant | verdict from an elevated session with `--include-ignored` | why |
|---|---|---|
| `elevated -> None` | CAUGHT | `assert_eq!(elevated, Some(true))` fails |
| `elevated -> Some(false)` | CAUGHT | same |
| `:163 != -> ==` | CAUGHT | inverts `TokenIsElevated`, so `verdict` answers `Some(false)` |
| `elevated -> Some(true)` | **SURVIVES** | `Some(true)` *is* the correct answer in that session; it is a genuinely equivalent mutant there |

**Reasoned**, not yet measured — the run that would have measured it was cut
off (§4.4). Killing `Some(true)` needs a **non**-elevated session plus a test
that asserts `Some(false)`, which does not exist and which is precisely
still-open item 15's unmeasured half. The two open items are the same gap seen
from two directions.

**So the gate is holding three, not six**, and only for a run that is both
elevated and told to include ignored tests. Neither condition appears anywhere
in the record.

### 4.3 Two facts about running it at all, both measured

- **`cargo mutants -- --include-ignored` does not reach libtest.** The help says
  trailing args are passed "after `--`"; they are not. Verified by having libtest
  reject a bogus flag: `cargo mutants … -- --nonexistent-flag-xyz` fails with
  *cargo's* usage error, while `cargo mutants … -- -- --nonexistent-flag-xyz`
  fails with `error: Unrecognized option: 'nonexistent-flag-xyz'` from libtest.
  **The correct invocation has two `--`.** A run written the documented way
  would silently skip the one test the whole exercise depends on.
- **`cargo install cargo-mutants --locked` cannot build on a14.** It pins
  `winapi` 0.3, which fails on `aarch64-pc-windows-msvc` with 285 errors
  (`ai_bloblen: ::size_t` against `::SIZE_T`). **Without** `--locked` it
  installs in 2.7 min, because a newer semver-compatible dependency no longer
  pulls `winapi`. This is why "no `cargo mutants` invocation has ever happened
  on a Windows machine in this project" is not merely an omission: the
  documented incantation does not work there.

**a14's ssh session is elevated** — `IsInRole(Administrator)` is `True`, and the
`#[ignore]`d test passes when invoked by name (`1 passed; 51 filtered out`). So
the elevated half of §4.2's condition is available on this machine without any
`runas`.

### 4.4 The run itself: cut off, and no verdicts are claimed

`cargo mutants -f src/sys.rs -f src/backend/winget.rs -j 4 -o … -- --
--include-ignored` was started on a14 (8 cores, 340.9 GiB free). The ssh
connection dropped mid-run (`Read from remote host: Connection reset by peer`),
and the machine then went offline — Tailscale: `zenbook-a14… offline, last seen
1m ago`. **No verdicts were recovered and none are reported here.** The next
attempt must run detached on the machine so a dropped connection cannot kill
it.

**macOS baseline for the eventual comparison**, same scope, same flags:

```
141 mutants tested in 7m: 22 missed, 110 caught, 9 unviable   (0 TIMEOUT, -j 4)
```

The 22 break down as `floor_char_boundary` 7, `parse_list` 7, `parse_versions`
1, `RealWinget::run` 1 (= the inherited 16), plus `package_roots` 2 and
`sys.rs` 4. That reproduces the corrected 16 from a differently-scoped run,
which is the first independent confirmation of it.

## 5. The inherited counts were never a deterministic quantity

This is the finding that changes how the earlier corrections should be read.

The record says Phase 4b's `floor_char_boundary: 6` and `parse_list: 6` "were
low", that "nothing closed them and nothing reopened them", and that "the
earlier number was simply never re-derived". That presumes there is one number
to re-derive.

**Measured: on Phase 4b's own tree there is not.** Four attempts at
`cargo mutants -f src/backend/winget.rs --re 'floor_char_boundary' -j 4` on a
worktree at `1d633c6`:

| attempt | outcome | survivors |
|---|---|---|
| whole-file run | 12 missed overall | **4**: `43:12 >→==`, `46:15 >→<`, `46:15 >→>=`, `47:13 -=→+=` |
| scoped run 1 | **baseline aborted** | — ("cargo test failed in an unmutated tree") |
| scoped run 2 | 3 missed | **3**: `43:12 >→==`, `46:15 >→>=`, `47:13 -=→/=` |
| scoped run 3 | 4 missed | **4**: `46:15 >→<`, `46:15 >→==`, `46:15 >→>=`, `47:13 -=→/=` |

Three completed runs, three **different** sets. Only **one** mutant
(`46:15 >→>=`) survives in all three. The union across them is **6** — the
number Phase 4b recorded — but no single run produced 6.

**On `c7086f0` the same scope is stable: 7 missed, the identical set, three
runs out of three.**

**The mechanism is in the logs, and it is this project's own defect class 2.**
At `1d633c6`, the mutants that flip are killed by `tests/cli.rs` disk-snapshot
assertions failing with *"the run changed something on disk"* — the
`assert_nothing_was_touched` comparison that Phase 5 itself **measured** was
being raced by git's background maintenance writing
`objects/maintenance.lock`. A verdict that depends on whether git ran
housekeeping during that particular mutant's test run is not a verdict, and the
baseline abort in attempt 1 is the same flake reproduced live.

**So the honest correction is not "Phase 4b undercounted".** It is:

- before Phase 5's `gc.auto 0` + `maintenance.auto 0` fix, this count was not a
  stable quantity, and any single value for it — 6, 4, 3 — is a sample rather
  than a measurement;
- Phase 5's fix is what made it reproducible, and **7** is now reproducible
  three times over;
- the extra survivors at `c7086f0` were never closed and then reopened. They
  were intermittently *appearing* closed, by a test failing for a reason
  unrelated to what it asserts.

This makes Phase 5's flake fix more load-bearing than its own record claims:
it did not only unblock a merge gate, it converted an unmeasurable number into
a measurable one.

**Also measured and previously unrecorded:** the `1d633c6` whole-file run
produced **2 TIMEOUTs**, both `rows_to_scan:350:25` (`>` → `<` and `>` → `>=`),
at `-j 4`. Phase 5's claim of two consecutive timeout-free phases is about
`-j 2` and is not contradicted, but "Phase 4 is the last phase that saw any" is
a statement about runs that were performed, not about the tree.

## 6. The Windows suite, on the tree that was shipping

Run because the recorded Windows numbers were measured on `765e091`, and
`765e091 → c7086f0` changes `src/backend/winget.rs`, `tests/cli.rs`,
`tests/execute.rs` and `tests/winget_resolve.rs`. Carrying a suite result
across a `.rs` change is the thing this project's own discipline forbids.

**Provenance was proven by content, not asserted.** The tarball carried
`SHIPPING-SHA.txt` **and** a `SHIPPING-MANIFEST.txt` naming a sha256 for all 72
shipped files; the runner recomputed every one on the machine and refused on any
mismatch, any missing file, and any file on disk the manifest does not name:

```
manifest entries : 72     verified equal : 72
mismatched : 0    missing : 0    unlisted on disk : 0
shipping sha : c7086f00d24a7d914202e1fd0aa048f2448df5fe
```

That last counter is the one that matters: Phase 5 had to hand-hash five files
to rule out an extract-over-an-old-tree. A whole-tree manifest removes the class
rather than sampling it.

- **`cargo test --no-fail-fast`: exit 0, 636 passed / 0 failed / 1 ignored**,
  across **14** `test result:` lines.
- **Fixture by sha, folded**: 30958 bytes, 143 CRLF pairs, sha256
  `c71284a393f87686…` — identical to the checked-in file.
- **Name-by-name cross-reference, from `--list`, never from run output**: macOS
  **638**, Windows **637**, common **636**; macOS-only the two `#[cfg(unix)]`
  tests, Windows-only the one `#[cfg(windows)] #[ignore]` test. Byte-identical
  difference set to every earlier run.

## 7. Method failures of my own, this round

Recorded because a probe that reports a wrong answer confidently is what this
project keeps paying for.

1. **I put backticks in a PowerShell file — in comments, the exact Phase 5
   defect — and the parse-check passed.** `[…]::Parser::ParseFile` reported 0
   errors because a backtick inside a `#` comment is not a parse error. The
   file was caught by a separate `grep` gate before it ran. **Parse-checking is
   not a backtick check, and Phase 5's own record pairs them for a reason.**
2. **A regex anchored on `: test$` returned 0 test names from Windows and I
   nearly recorded the zero.** The lines were split on `[char]10`, leaving a
   trailing CR, so nothing matched `$`. The sibling filter in the same script
   (`-match 'test result:'`) worked only because it was unanchored. The suite
   name set was re-derived by pulling the raw capture back and parsing it here.
   This is the same shape as Phase 5's `... ok` regex trap, one character over.
3. **A helper that printed with `Write-Output` would have swallowed the entire
   run into a variable** had its return value been assigned — PowerShell
   captures a function's pipeline output, so `$x = Invoke-Status …` collects
   every reported line instead of displaying it. Caught by reading the script,
   not by any gate.
4. **I wrote down trap 3 and then walked into it, one script later.**
   `p6b-trigger.ps1`'s `Measure-SourceUpdate` reports with the same
   `Write-Output` helper and its result was assigned, so every reported line was
   captured into the caller's variable and then re-emitted inside the summary
   string. The data survived and is readable, but only because the summary
   happened to print the variable; it was luck, not design. Documenting a trap is
   not the same as putting a gate in front of it.
5. **I reported an `scp` exit code that was `grep`'s.** `scp … | grep -v …; echo
   $?` reports the last element of the pipeline. The upload was verified
   afterwards by listing the files on the machine, which is what should have
   been done first.

## 9. The Windows suite on the branch tree

The suite in §6 describes `c7086f0`. This branch changes `.rs`, so it was run
again on **`ea6d91f`**, shipped the same way — whole-tree manifest, 72 files,
recomputed on the machine, `unlisted on disk : 0`.

- **exit 0, 641 passed / 0 failed / 1 ignored**, across **14** `test result:`
  lines.
- Fixture unchanged: 30958 bytes, 143 CRLF pairs, sha256 `c71284a393f87686…`.
- **Name by name**: macOS **643**, Windows **642**, common **641**. The
  difference set is byte-identical to every previous run — the same two
  `#[cfg(unix)]` tests missing on Windows, the same one `#[cfg(windows)]
  #[ignore]` test missing on macOS.
- The `#[ignore]`d elevated test passes when invoked by name.

The five-test rise (638 → 643) is this branch's own: two round-trip guard tests,
one `SourceRefresh` unit test, and two `update.rs` integration tests.

## 10. Item 20 live: the retry still has not fired, and now that means something

**The point of the instrumentation was to make a zero informative.** Stage C's
zero could not distinguish "the contention never reproduced" from "it reproduced
and the retry absorbed it". This round's zero can: the line is printed whenever
`AfterRetry` comes back, so its absence says the retry did not fire.

**Measured, on `ea6d91f`, 70 contended `dotpkg update` rounds in three
configurations, and a quiet counterweight before each:**

| configuration | rounds | rounds printing the retry line |
|---|---|---|
| no competitor (counterweight) | 5 + 5 + 5 | **0** |
| one `winget list` loop | 20 | **0** |
| four `winget list` loops | 30 | **0** |
| two `winget source update` loops | 20 | **0** |

**The plumbing was proven rather than assumed**, because otherwise this table is
a gate that narrates its own result. Two checks:

1. The shipped binary contains both new strings (`0x8A150001` and `succeeded
   on…`).
2. **Positive control on the sibling arm of the same `match`**: with winget
   removed from `PATH` for one run, the real binary printed
   `warning: winget: could not refresh its index (winget source update could not
   be run: winget.exe is not on PATH); resolving against whatever it already
   has.` So the `warnings` vector from exactly this code path reaches the
   terminal. A zero from the other arm is a fact about the trigger, not about
   the wiring.

**So item 20 is half closed.** The ambiguity it was written about is gone; the
observation it asked for has not happened. What is now known and was not: 70
contended rounds through `dotpkg update` are not enough, which is itself a bound
nobody had.

## 11. The trigger, re-measured — and §5's account of it has aged

Run because 0 of 70 through `dotpkg update` disagrees with §5's `3 of 10`, and
this project's rule is to re-measure the number that disagrees before explaining
it. Same argv as §5, invoked directly, exit codes captured by direct invocation.

| condition | calls | `0x8A150001` | 2026-08-11 recorded |
|---|---|---|---|
| alone | 10 | **0** | 0 of 10 |
| one concurrent `winget list` | 10 | **0** | **3 of 10** |
| four concurrent `winget list` | 10 | **1** | not measured |

**The trigger still exists but is rarer than the record says.** One competitor
reproduced it 3 of 10 times on 2026-08-11 and 0 of 10 today; it took four
competitors to see one. That is enough to explain the dogfood zero without
appealing to anything about dotpkg.

**Two things about §5 that no longer hold, both worth the record:**

- **Duration no longer distinguishes a failure from a success.** §5: failures
  60–72 ms, successes 348–623 ms, "distinguishable on three independent axes —
  exit code, duration, and output presence". Today the one failure took
  **1245 ms** and a success in the same round took **1266 ms** — 21 ms apart.
  Exit code and empty stdout still separate them; duration does not.
- **The 1 s retry delay no longer clears the success range.** `update_source`'s
  own comment reasons that 1 s "clears that success range with margin" against
  348–623 ms. Measured today on the same machine, successful `source update`
  calls take **1.2–5.4 s**. A 1 s wait is now *inside* the range of how long the
  competitor it is waiting out takes to finish. Still-open item 11 asks whether
  1 s is "sufficient on a slower machine"; it is now measured to be insufficient
  on *this* machine, against today's durations. **Nothing here says the retry is
  wrong** — a retry that fires too early simply fails twice and warns, which is
  the behaviour already shipped — but the comment's stated justification is no
  longer true and should not be read as if it were.

## 8. What is still outstanding, and exactly what it needs

a14 went offline partway through this round and did not come back, so three
things are set up but unmeasured. Each is listed with what has already been
proven about it, so the next attempt does not re-derive any of that.

1. **The Windows mutation run.** Scope `-f src/sys.rs -f src/backend/winget.rs`,
   and it **must** be invoked as `cargo mutants … -- -- --include-ignored` (two
   `--`; the one-`--` form documented by `--help` does not reach libtest, and
   without it the run settles nothing). The session must be elevated; a14's ssh
   session already is. Expect **3** of the `sys.rs` four to die and
   `elevated -> Some(true)` plus both `package_roots()` mutants to survive — if
   anything else happens, that is the finding. **Run it detached on the machine**:
   the last attempt died with the ssh connection.
2. **The retry, observed firing** (still-open item 20). The instrumentation is
   in the tree and unit-pinned; what is missing is one `dotpkg update` round with
   a declared winget package and a concurrent winget process, watching for the
   `0x8A150001 … succeeded on one retry` line. The trigger was measured at 3 of
   10 under contention, so ~10–20 rounds should produce it.
3. **The Windows suite, again.** The run recorded in §6 describes `c7086f0`.
   This branch changes `.rs`, so that result no longer describes the tree, and
   by this project's own rule it has to run again before the branch is
   verifiable. Nothing here has been merged on the strength of the `c7086f0` run.

**One thing that does not need the machine**: printing `sysinfo`'s `exe` for a
`Links`-launched process, which would turn §2's reasoned mechanism into a
measured one. It needs a dozen lines and a Windows host — the same host, but not
the same round.
