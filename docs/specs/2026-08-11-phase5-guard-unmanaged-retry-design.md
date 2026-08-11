# dotpkg Phase 5 — stop lying about the machine

Three things `status` and the running-process fence report wrongly. No new
capability: this phase makes what dotpkg already says become true.

Full measurement record:
[`docs/measurements-2026-08-11-phase5-guard-unmanaged-retry.md`](../measurements-2026-08-11-phase5-guard-unmanaged-retry.md).
Base: `main` at `1d633c6`, 588 tests macOS / 586+1 Windows, clean.

The scope below is **not** the scope the phase brief asked for. The measurement
round changed all three targets before any of them reached a design, which is
what the round was for.

## Scope

| # | brief's target | what the measurement made it |
|---|---|---|
| 1 | a package's second alias is invisible to the running-process fence | the fence has no **path** signal for winget, and a path is the only thing protecting `kanata` today |
| 2 | a winget dependency becomes `Unmanaged` forever | `Unmanaged` for winget is already saturated at **36 lines**; the dependency framing suppresses **0** of them |
| 3 | no retry policy for a transient winget failure | the fragile path has **no observed transient** (0 of 105); the path with a measured transient (3 of 10) is already non-fatal, and retrying it fixes a real staleness bug |

## Corrections to earlier documents and to the brief

Recorded here rather than edited in place, matching the precedent set by the 2a,
2b-2, Phase 3, Phase 4 and Phase 4b designs.

### The brief's `leftover Links: 2` is not in the record

The brief reports that a dogfood cleanup script "counted `leftover Links: 2`
for pattern `xh*`". The record says **0 leftover Links**, in three places
(`progress.md:214`, `:396`,
`docs/measurements-2026-08-10-winget-write-path.md:40`). The `2` is that
document's §11, observed **while `xh` was installed**. The clue survives the
correction and is strengthened by it — `Links` went 2 → 0 across install →
uninstall — but the phase must not cite a leftover that never existed.

### `Links` is not still-open item 10's oracle. It is item 9's missing signal

The brief asks whether `Links` could be "something on disk to read back",
closing item 10 (*"there is no independent oracle for a winget mutation"*).
It cannot: **4 of 36 installed ids**, all `portable`, and every EXE/MSI
application — the class the fence exists for — appears in neither `Links` nor
`Packages`. `C:\Program Files\WinGet\Links` does not exist on the measured
machine at all.

What it is instead: `Packages\<id>_<sourceIdentifier>\` is structurally the
same thing as `$SCOOP/apps/<name>/`, so it can fill `Running.dirs` — the signal
`docs/phase4-notes.md`, `src/backend/winget.rs:244-249` and the Phase 4 design
all described as unable to fire for winget, and which the Phase 4b design
correctly re-described as *"the package-directory half cannot fire"*. That
sentence stays true of scoop's root and becomes false of winget's own.

**Item 10 is not closed by this phase, and a stronger lead than `Links` was
found and deliberately not followed:**
`LocalState\Microsoft.Winget.Source_8wekyb3d8bbwe\installed.db`, 262144 bytes, a
winget-written catalog that is not portable-only. Reading it means bundling
SQLite and depending on an undocumented internal schema. Recorded as a lead.

### Still-open item 9 is narrower than the class it belongs to

Item 9 is written as *"a package's **second** alias is invisible"*, from `xh` /
`xhs`. Measured: `rg` is ripgrep's **only** command and it is invisible, because
`BurntSushi.ripgrep.MSVC`'s last dotted segment is `MSVC`. The class is "the
process name is not derivable from the id", and a second alias is one member.

### Still-open item 2's mechanism holds; its framing does not

`Microsoft.VCRedist.2015+.{x64,x86,arm64}` all carry `Source: winget`, so they
do reach `installed` and are reported — item 2 is right about that. They are
3 of **36**, about 11 of which are runtimes nobody declares, including
`Microsoft.AppInstaller`, which is winget itself. With **0** winget packages
declared in the measured `pkg.toml`, a dependency-aware fix has no manifest to
read a `Dependencies` list from and suppresses **0 of 36 lines**.

The code already contains the argument for the real fix and applies it to only
one of the two floods: `rows_to_scan` collapses 84 sourceless ids into one
aggregate warning because *"84 lines for the ordinary shape of a winget machine
is exactly the false-positive flood that gets a feature silenced and never
turned back on"*. `plan.rs:532` gives winget `helpers: &[]` where scoop gets
`SCOOP_HELPERS`.

### Still-open item 11 is falsified in the direction that matters

Item 11: *"a momentarily unhappy winget — a locked index, a source mid-update —
fails the whole run, scoop included."* Measured: the reader argv
`version_liveness` uses returned **0 nonzero exits in 105 invocations**,
including 30 fired against a continuously running `source update` loop. The
writer, `source update --name winget`, failed **3 of 10** under contention with
`0x8A150001` — and its failure has been a warning, not a refusal, since
`src/update.rs:410`.

Two things item 11 does not say and should:

- **`--keep-going` is not the documented escape hatch it is called.**
  `gate_removals` holds **every** removal step whenever `is_ok()` is false,
  scoop's included.
- **`status` is already resilient.** It never calls `version_liveness`; it uses
  `backend::scan_or_warn` (`main.rs:468`), which exists so a winget hiccup
  cannot abort scoop's half. Item 11 is an `apply` problem only.

## Half A — the running-process fence

### A1. `Running.dirs` learns to see a winget package

**`Running::covers` is `dirs || names || bins`; `covers_name` is `dirs ||
names`; `covers_any` calls `covers_name`.** So a `dirs` entry keyed on the
winget id closes the plan-time hole and the mid-run re-sampler hole **in one
place**, with no change to `Step` — the opposite of Phase 4b's `bins` fix, which
had to thread guard names through `Step` because `covers_name` has no `bins`
half. Structural, provable by reading `src/model.rs:250-277`.

A new pure function in `backend::winget`:

```
running_ids(roots: &[PathBuf], procs: &[Process], scanned: &[Name]) -> BTreeSet<Name>
```

For each scanned id, insert it if any process's `exe` lies under
`<root>/<id>_…`, comparing case-folded with `/` separators — the same `fold`
shape `Scoop::running_apps` already uses. A bare `<root>/<id>/` with no
`_<sourceIdentifier>` suffix is also accepted; **all 5 measured directories
carry the suffix, so that branch is reasoned, not measured**, and it must be
labelled as such where it is written.

**Why a per-id prefix test and not `segment.split_once('_')`.** Splitting the
directory name assumes a winget id contains no `_`, which is **unmeasured**, and
the failure direction is wrong: a truncated segment matches no installed id, so
the guard silently misses and a running package can be replaced. The prefix test
makes no assumption about winget's directory naming, and its only failure
direction is "no match when the package is genuinely not running".

Roots, and their evidence labelled separately because they differ:

- `%LOCALAPPDATA%\Microsoft\WinGet\Packages` — **measured**, 5 directories.
- `%ProgramFiles%\WinGet\Packages` — **measured absent** on a14. Included
  anyway for a machine-scope portable, which is **reasoned, not measured**, and
  the code must say so at the call site rather than let the pair read as one
  measurement.

`roots` is a parameter, not read from the environment inside the function, for
the reason this crate has extracted a seam six times before (`floor_exit_code`,
`write_in_order`, `parse_batch`, `rewritten`, `dedupe_installed_for_backend`,
`count_replaces_and_installs`): the rule is what needs proving and macOS can
prove it.

**Three wiring sites, not one, and missing the third would repeat Phase 4b's
own named mistake.** `running_set` is called at `src/main.rs:470` (`status`),
`src/apply.rs:1064`, and `src/main.rs:716` — the last being the mid-run
re-sampler's closure. A fix that reaches the first two and not the third closes
the plan-time hole and leaves the during-the-run hole exactly as wide, which is
the case the sampler exists for.

`Running`'s doc comment currently says `dirs` is scoop-only by construction.
That sentence must change in the same commit, and `src/backend/winget.rs:244-249`
with it — this is `docs/phase4-notes.md`'s pattern 2 pre-scheduled, the same way
the Phase 4b design pre-scheduled `Divergence::describe()`'s four sentences.

**What A1 does not do, stated in the notes rather than implied.** On the
measured machine it adds **zero** new catches: the one live process under
`Packages` is `VKey.exe`, and `guard_names` already catches `PhatMT97.VKey` by
name. Its coverage is 4 of 36 ids, all `portable`, and it reaches **none** of the
three live misses A2 exists for. Its value is the class — `kanata`'s shape — not
a number, and the notes must not dress it as a number.

### A2. `[winget.guard]` — the only mechanism the data supports for the rest

```toml
[winget.guard]
"Tailscale.Tailscale"   = ["tailscaled", "tailscale-ipn"]
"AutoHotkey.AutoHotkey" = ["autohotkey64"]
"Microsoft.WSL"         = ["wslservice"]
```

Those three entries close exactly the three misses measured against a14's live
process table, and A1 reaches none of them because none is a `portable` install.

- `RawWingetSection` gains `guard: BTreeMap<String, Vec<String>>`. It already
  carries `deny_unknown_fields`, so a typo like `guards` is refused rather than
  read as "you declared nothing" — the argument `config.rs`'s own test already
  makes for `packagess`.
- Keys fold through `fold_map` like `[scoop.opts]`. **Values must fold through
  `sys::normalize`**, which is what `sys::running_processes` applies to what it
  reports. `normalize` is currently private and becomes `pub(crate)`: a second
  implementation is the "two copies can drift" class, and `guard_names`' own doc
  comment already records why unfolded text silently never matches.
- A value that is empty after normalisation is a parse error. It would sit in a
  `BTreeSet<String>` matching nothing while reading as protection.
- **Merged into `Installed.bins` once, after the scan and before `plan`.**
  `rows_to_scan` stays a pure function of winget's output — it has no `Config`
  and must not gain one. Because `guard_for` copies `inst.bins` into the step
  (`apply.rs:908-914`), one merge point serves the plan-time fence and the
  re-sampler both.
- A key naming an id that is in neither `[winget] packages` nor the scan gets
  one warning. A stale or misspelled entry otherwise protects nothing, silently;
  keying the warning on "matches nothing at all" is what keeps it from firing
  every run on a machine where the app is merely not installed. **It cannot be a
  parse error and cannot live in `config.rs`**: only the merge point below knows
  the scan, so the warning is emitted there and joins the same warning stream
  `print_scan_warnings_and_merge` already prints.
- The text-level round-trip guard needs no change: it compares parsed `Config`
  values, so a new field is covered by construction. `config_edit` needs none
  either — nothing in the tool ever writes this table.

### A3. Rejected: widening `guard_names` heuristically

Prefix or substring matching would catch `tailscaled` from `tailscale`. At 36
installed ids against 86 live process names it fires constantly, and the `!`
lines become the flood Half B exists to remove. `Installed.bins`' own contract
("names a live process might plausibly report") tolerates over-matching because
a false positive costs one line; it does not tolerate over-matching that costs
thirty. Rejected on the measurement, not on taste.

## Half B — the `Unmanaged` flood

### B1. Aggregate at render, keep every fact in the plan

`Plan` keeps all 36 `Action::Unmanaged` entries; `plan.rs:553` still
concatenates `reports` into `actions`. Only `render` collapses them:

```
  ? scoop    6 installed outside dotpkg -- no action
  ? winget   36 installed outside dotpkg -- no action
             pass --show-unmanaged to list them

  0 change(s), 0 skipped, 42 unmanaged
```

**One collapsed line per backend, and the summary counts the total across
both** — per-backend, because the `{backend:<6}` column is what tells a user
which tool to go look at, and a single merged line would repeat
`docs/phase4-notes.md`'s "the merged `opaque` list's lost backend attribution"
minor, which is still open. The scoop count above is illustrative; the measured
machine's scoop half was not counted in this round.

`0 change(s)` is not a typo. `Plan::change_count` (`plan.rs:129-131`) counts
Install / Upgrade / Prune / non-winget Downgrade; an `Unmanaged` is counted by
nothing. **The summary clause is mandatory, and the code states the rule
already**: `refused_downgrade_count` earned its own clause because *"a printed
`!` line counted in no number at all would read as `0 change(s), 0 skipped`
above a line the user can see"*. A collapsed `?` line is that same shape, one
step worse, because collapsing removes the 36 lines that used to carry the fact.

`render` takes `show_unmanaged: bool` (about ten call sites). No options struct:
one caller wants one flag today.

### B2. The rule applies to both backends, and that is a disclosed behaviour change

The flood is measured for winget. Applying the collapse to winget alone means a
per-backend special case with no measurement behind the asymmetry either;
applying it to both means **scoop's `status` output changes for every user** who
has an undeclared scoop app. `--show-unmanaged` restores the previous output
byte for byte, and nothing about what dotpkg *does* changes.

This is a user-visible behaviour change that is not an addition, and it belongs
in the phase notes' section 1 alongside the two Phase 4b already lists — not in
a deferred-minor list. `docs/phase4b-notes.md`'s own parked finding is that its
heading claimed "the one user-visible behaviour change" while the same wave
changed the consent prompt for every scoop run; this phase must not repeat that
by disclosing only the winget half.

### B3. Rejected: `WINGET_HELPERS`, and rejected: `[winget] ignore`

A hardcoded list of runtime ids ("VCRedist, VCLibs, WindowsAppRuntime …") is a
`docs/phase4-notes.md` pattern-2 generator in list form, and it would *exclude*
those packages from `Unmanaged` entirely (`plan.rs:482`) rather than count them
— a different and less honest thing than collapsing a line. `[winget] ignore`
makes the user maintain 36 entries to silence noise dotpkg created. Neither is
in scope.

## Half C — the transient, put back where it was measured

### C1. One retry for `update_source`, on the measured signature

Measured: 0 of 10 solo, **3 of 10 with another winget process alive**, every
failure `0x8A150001` in **60–72 ms with empty stdout**, against 348–623 ms and
`Updating source: winget...` on success. Distinguishable on exit code,
duration and output presence independently.

The consequence today is not a failed run — `update_source`'s `Err` is already a
warning — it is that **3 of 10 times `dotpkg update` resolves `latest` against
an index it failed to refresh and only warns.** One retry fixes that on the one
path where the transient was actually measured.

- New `INTERNAL_ERROR: i32 = -1978335231`, with a hex cross-check
  (`INTERNAL_ERROR as u32 == 0x8A150001`) in the same test as the other five.
  This is the sixth constant that would otherwise exist exactly once in the tree
  with no test pinning its value — the defect class `NO_AVAILABLE_UPGRADE`
  already fell into.
- Retry **only** on `INTERNAL_ERROR`, once, after **1 s**. Any other nonzero
  exit keeps today's behaviour exactly. The delay comes off the measurements
  rather than being picked: the failure returns in 60–72 ms, and the competing
  winget call it lost to runs 407–1117 ms (measured max, `list -e --id`), so a
  delay shorter than the competitor's remaining runtime retries into the same
  contention. 1 s covers the measured maximum; it is **not** measured to be
  sufficient for a slower machine, and the comment must say that rather than
  present the number as a measured floor.
- `update_source_with(retry_delay: Duration)` is the seam, so the retry test
  passes `Duration::ZERO` and the suite stays fast. `update_source()` supplies
  the real delay.

### C2. No retry for `version_liveness`, and `gate_removals` is not touched

0 of 105. Building a retry loop on an unobserved failure mode only slows a
certain failure down — the brief's own standard. The generic arm at
`winget.rs:893` gains one thing instead: when the code is `INTERNAL_ERROR`, a
message that names the likely cause (another winget process was running) and the
action (re-run), rather than a bare `exited <n>`.

**The provenance of that arm must be labelled at the arm.** `INTERNAL_ERROR` was
measured from `source update`, never from `show`. In every contention probe the
reader was the winner — 105 of 105 — so "readers share the index, the updater
needs it exclusively" is a **mechanism inferred from the numbers, not a measured
property of the reader**. A comment claiming `show` was measured returning this
code would be exactly the fabricated-mechanism defect the Phase 4b fix rounds
caught twice.

Item 11 becomes a **decision with numbers behind it**, not an open gap.

## Testing

Each guard below must be able to fail, and its fixture must be able to express
the hazard — the `opaque: Vec::new()` and `Running::new(["brave.brave"])`
lessons.

- **A1**: the fixture path is the measured one,
  `…\Packages\PhatMT97.VKey_Microsoft.Winget.Source_8wekyb3d8bbwe\VKey.exe`.
  Three counterweights in the same test file: a `Process` with `exe: None` (the
  22-of-223 blind spot, so a path-only implementation cannot pass by ignoring
  it); a process under `Packages` belonging to `PhatMT97.VKey.Classic`, an id
  that is **measured absent** from `winget list`, proving a dead directory
  matches nothing; and a process under a *sibling* directory whose name merely
  starts with a shorter id, proving the `_` boundary is load-bearing.
- **A1, the third wiring site**: a test that goes red if the mid-run re-sampler
  is left on scoop's `Running`. Phase 4b's equivalent hole was found by a
  reviewer, not a test.
- **A2**: red when `[winget.guard]` is removed from the data path, not merely
  "it parses". Plus a value given as `Tailscaled.EXE` that must still match the
  process `tailscaled`, which fails if `sys::normalize` is not reused.
- **B1**: the count must come from a fixture with 36 real rows. A `vec![]` or a
  one-row fixture cannot tell the aggregate from the per-line form.
- **B1**: the summary clause pinned by an assertion on the literal text. Three
  previous fixes to that line were not, and the third is the one that finally
  got an assertion.
- **C1**: two calls observed on the fake, first returning `INTERNAL_ERROR` and
  the second `0`, with a sibling test proving a *different* nonzero code is not
  retried.

Standing rules from Phase 4b carry unchanged: fixture bytes checked before
trusting any Windows result; `cargo check --target aarch64-pc-windows-msvc
--all-targets`; the Windows suite run on the tree that ships and cross-referenced
name by name, never by subtracting totals; `cargo mutants -j 2` on an idle
machine with nothing editing the tree concurrently.

## Non-goals

- **Still-open item 10.** Not closed. `installed.db` is recorded as the lead;
  reading it is not in scope.
- **A path signal for non-portable winget packages.** Measured to be
  unreachable: 0 winget-shaped ARP keys in HKLM or WOW6432Node, so an EXE/MSI
  package's ARP entry is named by its publisher and mapping it back to a winget
  id is the guesswork this crate refuses. `[winget.guard]` is the answer.
- **Dependency vocabulary.** Rejected on the measurement, not deferred.
- **Loosening `gate_removals` or `Preparation::is_ok()`.** The trigger has never
  been observed on the path that would need it.
- **Everything Phase 4b left open** except items 2, 9 and 11 above, notably: the
  three `#[cfg(windows)]` `sys.rs` mutants (a platform gap, resolvable only by
  mutating on Windows), `main.rs:773`'s accepted equivalent mutant, the 14
  Phase 4 `winget.rs` mutants, the two `RealWingetMutator::run` mutants, and an
  ordinary non-elevated Windows session with no `runas`, which remains
  unmeasured.
