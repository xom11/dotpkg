# Measurements: the mutation debt, sorted into four kinds

Taken 2026-08-12 on the macOS development machine (10 logical cores), against
`main` after the item-27 merge.

The standing list said "20 surviving mutants" as one number. It is four
different things, and only two of them are debt.

## 1. The idle gate refused this machine, and the gate was right to

`scripts/idle-gate.sh` at its default threshold **REFUSED**: machine busy
**30.40%** against a 10% threshold, with `ReportCrash` at 62.8% of one core. A
second sample gave **15.78%**, still refusing.

`scripts/idle-baseline.sh` then measured this machine's actual floor across
three windows: **17.10 – 19.47%**, with the largest burners `WindowServer`
(28–35% of one core), **three `claude` processes** (10–32% each), `Brave` and
`Storage`.

**Two things follow, and the second is the uncomfortable one.**

The 10% threshold is a14's, measured there at 2.85–3.26%. The gate's own header
says to re-derive it per machine before trusting it, and that had never been
done here. Re-derived at 3× the observed maximum (58%, the procedure
`idle-baseline` itself prescribes), the gate passes at 18.30%.

**And the agent running the measurement is part of the load it is measuring.**
The a14 rounds never hit this because the load was on a14 and the controller was
elsewhere. On a single machine, "idle" cannot include the process asking the
question. That is not a reason to ignore the gate; it is a reason to state the
confound beside every number below.

**One run in this round was taken while the gate refused**, on the per-process
threshold. It is kept rather than discarded, with its own evidence that the
refusal did not corrupt it: auto-timeout 37 s, slowest mutant 10 s, **0
TIMEOUT** across 12 mutants. A load problem shows up as timeouts, and there were
none. The re-run after the fix was taken with the gate passing.

## 2. Phase 8/9 code: 0 surviving mutants

`cargo mutants --in-diff` over `git diff 07dd86b..HEAD -- src/` (446 lines),
`-j 2`. Complete: `start_time` and `end_time` both present, 9 outcomes = 1
baseline + 8 mutants.

**8 mutants tested in 40 s: 5 caught, 3 unviable, `missed.txt` empty.**

```
caught    src/main.rs:453         main -> Ok(())
caught    src/config_edit.rs:335  save -> Ok(())
caught    src/execute.rs:1082     execute -> Ok(Default::default())
caught    src/lock.rs:177         save -> Ok(())
caught    src/state.rs:154        State::save -> Ok(())
unviable  src/execute.rs:703      <impl Mutates for ScoopSide>::run  -> Default::default()
unviable  src/execute.rs:720      <impl Mutates for WingetSide>::run -> Default::default()
unviable  src/execute.rs:756      run_step -> Default::default()
```

**Unviable is not caught, and folding the two together would be the exact
mistake this project keeps finding.** Those three did not compile — `StepOutcome`
has no `Default` — so the run says *nothing* about whether a test would notice a
wrong `StepOutcome`. What covers them is the manual negative control recorded in
the `Mutates` commit: routing `ScoopSide::run` at a bogus root turns the suite
red. Pinned, but by a control rather than by this run.

`config_edit::save -> Ok(())` being caught is worth naming: that is the positive
half added to the inverted `.bak` test — *"an edit that silently wrote nothing
would also leave no `.bak`"* — shown to be load-bearing rather than decorative.

## 3. The four kinds, and what happened to each

### Kind A — the price of the seam. 3 mutants. Not closable, not debt.

`RealWinget::run` and `RealWingetMutator::run` are the only two functions that
spawn `winget.exe`. Every test in the crate goes through the `WingetCmd` /
`WingetMutator` seam to a fake, which is the entire reason 662 tests run on
macOS. So these are by construction the one place no test reaches, and their
mutants (`NotFound ==` → `!=`, `unwrap_or(-1)` → `unwrap_or(1)`) can only be
killed by spawning a real winget from the suite — which this project forbids.

**These should be recorded as accepted, not carried as debt.** They are the
shadow the seam casts, and the seam is worth more than the mutants.

### Kind B — unreached because real data never went there. 7 → 3 + 1 timeout.

`floor_char_boundary` walks a byte offset back to a character boundary so
`parse_list` can slice fixed-width columns without panicking. **All 15 captured
fixtures are from an en-US machine and are pure ASCII, so the loop body had
never executed once.** Measured before: 7 of 12 mutants surviving.

Two tests added, inline rather than as fixtures — every file under
`tests/fixtures/winget/` is raw stdout winget actually wrote, and inventing one
would put fabricated bytes in the one directory whose value is that nothing
there is fabricated. The layout was computed, not eyeballed: `Id` starts at byte
8 and the three bytes of `日` occupy 6, 7 and 8, so the column's own offset is
that character's last byte and not a boundary.

After: **12 mutants, 8 caught, 3 missed, 1 timeout.**

| mutant | before | after | why |
|---|---|---|---|
| `:46 > → ==` | missed | **caught** | loop never runs, slice panics |
| `:46 > → <` | missed | **caught** | same |
| `:47 -= → +=` | missed | **caught** | walks UP to the next boundary, putting the character in `Name` and leaving `Id` empty, which drops the row |
| `:47 -= → /=` | missed | **timeout** | `idx /= 1` is a no-op, so the loop cannot terminate once it is entered |
| `:43 > → ==` | missed | missed | equivalent under the callers |
| `:43 > → >=` | missed | missed | equivalent under the callers |
| `:46 > → >=` | missed | missed | genuinely equivalent |

**The three survivors are characterised rather than left open.** Both call sites
clamp before calling — `end.min(line.len())`, and an early
`if start >= line.len() { return String::new() }` — so `idx > s.len()` is
unreachable-true and its two mutants cannot be distinguished from behind the
callers. This is *equivalent under current callers*, which is weaker than
*equivalent*: a future caller passing an unclamped index would make `==` wrong.
And `:46 > → >=` is equivalent outright, because `is_char_boundary(0)` is always
true, so the loop stops at zero either way.

### Kind C — the `NotFound` idiom. 1 mutant, killed, and it was the dangerous one.

`Scoop::scan` maps one io error to a valid empty machine and every other to a
failure. With the guard replaced by `true`, **every** read failure reads as "no
scoop packages installed" — and an empty scan is not a wrong number, it is the
input that makes every owned package undeclared-and-absent, leaving
`mass_prune_guard` as the only thing between that and a plan full of prunes, a
guard the design itself describes as catching the case *"far too late"*.

Nothing in the suite could produce a non-`NotFound` `read_dir` error, so nothing
could see it. A `#[cfg(unix)]` test now makes the directory unreadable with mode
`0o000` and asserts the scan **fails**. It carries its own control: root ignores
mode bits, so the test checks it genuinely could not read the directory and
fails loudly rather than passing quietly if it could.

**Confirmed by putting the mutant in by hand:** the suite goes to 583 passed, 1
failed, and the 1 is exactly this test.

### Kind D — platform blindness in `sys.rs`. Unchanged, and not a test gap.

`#[cfg(windows)]` bodies are not compiled on macOS, so mutating them has no
effect there, and the mirror is equally true for the `cfg(not(windows))` arm on
Windows. A run on one platform closes three and silently reopens two. The rule
stays three runs — macOS, elevated Windows, ordinary Windows — and this round
was macOS only.

## 4. Where the number stands

| | before | after |
|---|---|---|
| surviving mutants named in the standing list | 20 | **15**, plus 1 detected only by timeout |
| of those, accepted-by-design (Kind A) | 0 named as such | **3** — `RealWinget::run` 1, `RealWingetMutator::run` 2 |
| of those, characterised as equivalent (Kind B) | 0 | **3** |
| genuinely open and unexamined | 20 | **9** — `parse_list` 7, `parse_versions` 1, `main.rs` 1 |
| killed outright | — | **4** — three in `floor_char_boundary`, one in `Scoop::scan` |
| suite | 659 | **662** |

3 + 3 + 9 = 15, which is the cross-check. The one timeout is counted separately
on purpose: a mutant that hangs the suite **is** detected, but by not
terminating rather than by an assertion, and calling that "caught" would be
claiming an assertion nobody wrote.

The `parse_list` 7 are the largest remaining block and the next thing worth
measuring. This round did not touch them.
