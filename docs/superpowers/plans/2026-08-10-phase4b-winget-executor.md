# Phase 4b: the winget executor — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `dotpkg apply` install, upgrade and remove winget packages — never downgrade them — with every mutation confirmed by a rescan rather than by winget's exit code.

**Architecture:** Winget mutations go through a new `WingetMutator` seam, mirroring the existing `Mutator` seam for scoop, so no test spawns `winget.exe`. `Step` splits into `Step::Scoop(ScoopStep)` and `Step::Winget(WingetStep)` so routing a winget step through scoop's executor becomes a compile error. After every step, `winget_verdict` re-runs `winget list -e --id <canonical>` and re-applies `rows_to_scan`'s three opaque rules, because winget's exit code `0x8A15002B` covers both "already exactly where you asked" and "I declined what you asked". Downgrade direction is never decided by dotpkg: it fires `install --version <pin>` and translates winget's own measured refusal.

**Tech Stack:** Rust 2021, `rust-version = "1.85"`, `anyhow`, `clap`, `serde`, `serde_json`, `sysinfo`, `toml`, `toml_edit`, `tempfile` (dev). Adds one target-gated dependency: `windows` (Windows only, for `TOKEN_ELEVATION`).

## Global Constraints

- **The suite runs with `--no-fail-fast` on every run.** Non-negotiable, carried from Phase 3.
- **The verified macOS baseline on `main` at `98f3d33` is `511 passed, 0 failed`.** Measured by running it, not read from a brief. Every expected count in this plan is that number plus the tests the task adds.
- **Every filtered `cargo test` invocation states the number of tests it expects to select, and that number is checked.** Four filters in Phase 4 matched no test *name* and printed `test result: ok. 0 passed; 0 failed`, exiting 0. A filter that selects zero tests is a plan failure.
- **One `0 passed` line is expected and must not be "fixed": `Doc-tests dotpkg`.** The crate has no doc-tests, so an unfiltered run prints 13 `test result:` lines of which the last is always `ok. 0 passed`. Verified on `98f3d33`. The rule above is about *filtered* runs; this line is not an instance of it, and a run that reports 12 such lines has lost a test binary.
- **The suite runs on Windows before the dogfood AND again on the exact tree that ships.** Phase 4 needed three Windows runs because the tree changed twice. Compare **name-by-name**, never by subtracting totals.
- **Two tests are `#[cfg(unix)]` and are expected to be absent on Windows:** `tests/adopt.rs`'s `a_failed_last_write_leaves_a_prefix_that_plan_does_nothing_about` and `tests/scoop_scan.rs`'s `a_root_reached_through_a_symlink_still_matches_running_processes`.
- **No test may spawn `winget.exe` or `scoop.cmd`, and no test may create a file at `Scoop::scoop_exe()`'s resolved path.** Standing rule.
- **Fixtures under `tests/fixtures/winget/` are CRLF**, pinned by `.gitattributes` (`tests/fixtures/winget/** -text`). Before trusting any Windows run, verify `list-full.txt` is **30958 bytes with 143 CRLF pairs**.
- **No backtick may appear in any PowerShell file this plan writes, comments included.** A backtick in a double-quoted PowerShell string silently mangled a file in Phase 4.
- **PowerShell reaches a14 as a `.ps1` moved by `scp` and run with `powershell -NoProfile -ExecutionPolicy Bypass -File`.** Quoting through `ssh` corrupts inline PowerShell. Use `ssh -F /dev/null -o BatchMode=yes kln@100.83.225.100` and `scp -F /dev/null`; plain `ssh a14` does not work from the sandbox.
- **Files written on a14 that anything reads back use `[System.IO.File]::WriteAllText` with `UTF8Encoding($false)`.** PowerShell 5.1's `Set-Content -Encoding UTF8` writes a BOM.
- **`kanata` is never started or stopped.** `C:\Users\kln\dotpkg-build` and `C:\Users\kln\pkg.toml` are reused and kept.
- **The rule that outranks this plan:** if a negative control cannot be made to go red, that is a failure of *this plan*, not of the implementer. Fix the test, say so in the notes, and **do not ask first**.
- **A scope change is made in the task brief, never in the dispatch.** Reviewers read only the brief, so a scope change hidden in dispatch reads as an unapproved deviation.
- **The controller holds long waits.** An agent stops its turn the moment it backgrounds a process, so `cargo mutants` and the Windows suite are held by the controller, which resumes the agent once results exist.

---

## Execution Order

**Task numbers are stable and are NOT the execution order.** They are how
`task-brief PLAN_FILE N` finds a task, so renumbering would break the tooling.
Execute in this order:

```
1  →  2  →  6  →  3  →  11  →  4  →  5  →  7  →  8  →  9  →  10
   →  12 →  13 →  14 →  15 →  16 →  17
```

Three dependencies force it, and the first draft of this plan got two of them
wrong by deferring tests across task boundaries:

- **Task 6 before Task 3.** Task 6 gives `plan()` its `unscannable` parameter.
  Task 3's test calls `plan()`, so running it first would mean writing the call
  with a signature that is about to change — which is what the first draft
  papered over with "omit that argument if this task runs before Task 6."
- **Task 11 before Task 4.** Task 4 gives `execute` its `&dyn WingetMutator`
  parameter, and that trait is defined in Task 11. Task 11 depends only on
  `CmdError` (Task 6) and touches nothing Task 4 touches.
- **Task 4 before Tasks 5 and 7.** Once `execute`'s signature is final at Task
  4, every test in Tasks 5 and 7 compiles and runs inside its own task. The
  first draft instead deferred three tests to Task 15, which would have ended
  two tasks with code that does not compile — and a task that cannot run its
  own tests has no independently testable deliverable, which is the one thing
  every task here is required to have.

## File Structure

**Created:**

| Path | Responsibility |
|---|---|
| `src/backend/winget_exec.rs` | Winget's mutating half: argv builders, the `WingetMutator` seam, `RealWingetMutator`, the three new exit-code constants, and `winget_verdict`. Kept out of `winget.rs` (already 833 lines) because scanning/resolving and mutating are different responsibilities with different test fixtures. |
| `tests/winget_execute.rs` | Integration tests for the winget executor path, using a recording fake mutator. |
| `tests/common/fake_winget_mutator.rs` | A recording fake for `WingetMutator`, sibling to `tests/common/fake_winget.rs`. |
| `tests/fixtures/winget/` (12 new files) | The captured stdout of every W1/W2 shape, checked in as the recording. |

**Modified:**

| Path | Change |
|---|---|
| `src/model.rs:181-194` | `Installed.bins`'s doc comment widens; no field change. |
| `src/model.rs:207-252` | `Running`'s doc comment records that `dirs` is scoop-only by construction. |
| `src/backend/winget.rs` | `guard_names`, filled into `bins`; `WingetCmd::run` returns a typed error. |
| `src/backend/mod.rs` | `ScanOutcome`; `scan_or_warn` returns it. |
| `src/plan.rs` | `SkipReason::Unscannable`; `BackendView::capability` becomes `Acts` for winget; `Divergence::describe()`'s four sentences rewritten; `plan()` gains an `unscannable` parameter. |
| `src/execute.rs` | `Step` splits into `Step::Scoop`/`Step::Winget`; `run_step` routes; `execute` takes both mutators; `root_looks_like_scoop` becomes conditional; `write_recovery` emits winget lines. |
| `src/apply.rs` | `classify` branches on backend; `Outcome::ReadyToSet`; `plan_to_steps` builds typed steps; `is_outstanding` gains an arm. |
| `src/sys.rs` | `elevated() -> Option<bool>` behind `#[cfg(windows)]`. |
| `src/main.rs` | The A4 pre-check, per-backend `reconcile`, wiring the winget mutator. |
| `src/verify.rs` or `src/config_edit.rs` | The text-level round-trip half. |
| `Cargo.toml` | Target-gated `windows` dependency. |
| `tests/planner.rs:404` | The `brave.brave` false-premise fixture. |
| `tests/fixtures/winget/PROVENANCE.md` | Records the measured drift. |

---

### Task 1: Close the two measurement gaps A4 depends on, and check in the write-path fixtures

**Files:**
- Create: `tests/fixtures/winget/install-version-fresh.txt`, `install-no-upgrade-available.txt`, `install-already-installed-no-upgrade.txt`, `install-upgraded.txt`, `install-package-absent.txt`, `install-version-absent.txt`, `uninstall-refused-elevated.txt`, `uninstall-success.txt`, `uninstall-package-absent.txt`, `uninstall-version-absent.txt`, `upgrade-nothing-available.txt`, `list-single-with-available.txt`
- Modify: `tests/fixtures/winget/PROVENANCE.md`
- Create: `docs/measurements-2026-08-10-winget-write-path.md` — **already written and committed at `25ea0a0`**; this task appends one section.

**Interfaces:**
- Consumes: nothing.
- Produces: the fixture filenames above, referenced by name in Tasks 11, 12 and 14. `PROVENANCE.md` records, for each, the exact argv and exit code it was captured under.

**Why this is first:** the spec makes A4's refusal depend on `winget list --scope user|machine` discriminating in both directions, and that is measured on **exactly one package**, machine-scoped, on the read side. Nothing may depend on it before it is confirmed. Same sequencing Phase 4 used for `winget source update --name winget`.

- [ ] **Step 1: Write the a14 probe as a `.ps1`, locally, with no backtick**

Create `scratch/w3-scope.ps1` (not committed). It must:
1. Resolve `winget.exe` via `Get-Command winget -CommandType Application` and call it through that absolute path — **never** as a bare `winget`, and the helper function must **not** be named `Winget` (a function of that name shadows the executable, because PowerShell function names are case-insensitive; this cost a round on 2026-08-10).
2. Refuse to continue unless the first `winget list --disable-interactivity` exits 0 and exceeds 10 KB.
3. Capture, for a **known machine-scope** package and a **known user-scope** package:

```
list -e --id <id> --disable-interactivity
list -e --id <id> --scope machine --disable-interactivity
list -e --id <id> --scope user --disable-interactivity
```

Use `Microsoft.VisualStudio.2022.BuildTools` as the machine-scope case (measured: `--scope machine` returns its row, `--scope user` exits `0x8A150014`). For the user-scope case, **find one from the machine rather than guessing**: iterate the 36 source-backed installed ids, run `list -e --id <id> --scope user`, and record every id that exits 0.
4. Capture `winget list` SHA256 before and after and assert they are identical.

- [ ] **Step 2: Run it and read the result**

```bash
cd <scratch> && scp -F /dev/null -o BatchMode=yes w3-scope.ps1 kln@100.83.225.100:C:/Users/kln/w3-scope.ps1
ssh -F /dev/null -o BatchMode=yes kln@100.83.225.100 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\Users\kln\w3-scope.ps1'
```

Expected: at least one id exits 0 under `--scope user` and non-zero under `--scope machine`, and `BuildTools` does the reverse. **If no user-scope id exists on a14, `--scope` has not been shown to discriminate in that direction — stop and report.** Task 15's pre-check must then key on the `0x8A15007D` translation alone, and this plan changes, not the code.

- [ ] **Step 3: Append the result to the measurements document**

Append a `## 15. --scope discriminates in both directions` section to `docs/measurements-2026-08-10-winget-write-path.md` with the raw argv/exit/stdout table, and move the `--scope` caveat out of "What was deliberately not measured".

- [ ] **Step 4: Capture the write-path fixtures**

The stdout bodies are already recorded verbatim in `docs/measurements-2026-08-10-winget-write-path.md` §§1–9. Write each to its fixture file **with CRLF line endings** (`\r\n`), because `.gitattributes` pins these paths `-text` and every other winget fixture is CRLF. For example `install-already-installed-no-upgrade.txt`:

```
Found an existing package already installed. Trying to upgrade the installed package...
No available upgrade found.
No newer package versions are available from the configured sources.
```

and `uninstall-refused-elevated.txt`:

```
Found xh [ducaale.xh]
The package installed for user scope cannot be uninstalled when running with administrator privileges.
```

- [ ] **Step 5: Verify the fixtures are CRLF and non-empty**

```bash
for f in tests/fixtures/winget/install-*.txt tests/fixtures/winget/uninstall-*.txt tests/fixtures/winget/upgrade-*.txt; do
  printf '%-56s %6s bytes  %3s CRLF\n' "$f" "$(wc -c < "$f")" "$(grep -c $'\r$' "$f")"
done
```

Expected: every file non-empty, and its CRLF count equal to its line count. **A file with 0 CRLF pairs is wrong** — fix it before continuing.

- [ ] **Step 6: Record the drift in PROVENANCE.md**

Append: the machine now reports 140 rows / 125 ids / 36 installed / 89 opaque against the fixtures' 141 / 126 / 37 / 89; `wez.wezterm` uninstalled; `tailscale.tailscale` `1.98.2` → `1.102.2`; the source MSIX row rotated. State that the "numerically identical" claim was true when written and is not now, and that a dogfood must re-derive the machine's numbers rather than reuse the fixtures'.

- [ ] **Step 7: Confirm the existing suite is untouched**

Run: `cargo test --no-fail-fast`
Expected: **511 passed, 0 failed** — the same count as `main` at `98f3d33`. This task adds fixtures and documentation only; a changed count means something else moved.

- [ ] **Step 8: Commit**

```bash
git add tests/fixtures/winget docs/measurements-2026-08-10-winget-write-path.md
git commit -m "Measure --scope in both directions, and check in the write-path fixtures"
```

---

### Task 2: `Running::covers` learns to see a winget package

**Files:**
- Modify: `src/backend/winget.rs` (add `guard_names`, fill `bins` in `rows_to_scan`)
- Modify: `src/model.rs:190-193` (`Installed.bins` doc comment), `src/model.rs:200-212` (`Running` doc comment)
- Test: `src/backend/winget.rs` (unit, for `guard_names`), `tests/winget_scan.rs` (integration, for `rows_to_scan`)

**Interfaces:**
- Consumes: `WingetRow { name, id, version, available, source }` from `parse_list`.
- Produces: `pub(crate) fn guard_names(id: &str, display: &str) -> Vec<String>` — folded, de-duplicated, in the form `sys::running_processes` produces. Task 3 puts these same values into `WingetStep`.

**Why:** measured on a14, `Running::covers` catches **0 of 36** installed winget packages. `Running.dirs` is populated only by `Scoop::running_apps`, which inserts path segments under `$SCOOP/apps/` or `$SCOOP/persist/`, so a winget id can never appear there; `bins` is always empty for winget; and the surviving signal needs a process named after the whole dotted id. `Brave.Brave` was running and was missed. This is the `kanata` scenario.

- [ ] **Step 1: Write the failing unit test**

Add to `src/backend/winget.rs`'s `#[cfg(test)] mod tests`:

```rust
#[test]
fn guard_names_are_the_two_signals_measured_to_catch_a_real_process() {
    // Measured on a14 against the live process table: of 36 source-backed
    // installed winget ids, the whole dotted id caught 0, the id's LAST
    // dotted segment caught 4, and the display Name column caught 2.
    // Brave.Brave was running at the time and today's guard missed it.
    assert_eq!(guard_names("Brave.Brave", "Brave"), vec!["brave"]);
    // Chrome is the case the display name cannot reach and the last segment
    // can: the process is chrome.exe, the display name is "Google Chrome".
    assert_eq!(
        guard_names("Google.Chrome", "Google Chrome"),
        vec!["chrome", "google chrome"]
    );
    // An id with no dot at all must still yield its own name, not nothing.
    assert_eq!(guard_names("xh", "xh"), vec!["xh"]);
    // Case is folded, because `sys::running_processes` lowercases what it
    // reports and a comparison against unfolded text silently never matches.
    assert_eq!(guard_names("PhatMT97.VKey", "VKey"), vec!["vkey"]);
    // An empty display Name must not produce an empty guard name: `names`
    // is a BTreeSet<String> that could contain "" and match nothing, but a
    // future caller comparing against it would be comparing against noise.
    assert_eq!(guard_names("Some.Thing", ""), vec!["thing"]);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --lib guard_names_are_the_two_signals -- --exact backend::winget::tests::guard_names_are_the_two_signals_measured_to_catch_a_real_process`
Expected: **1 test selected**, FAIL with `cannot find function 'guard_names'`.
If it reports `0 passed; 0 filtered out` — the filter matched nothing and the run is meaningless. Fix the filter.

- [ ] **Step 3: Implement `guard_names`**

Add above `rows_to_scan` in `src/backend/winget.rs`:

```rust
/// Names a live process might plausibly report for a winget package.
///
/// winget exposes no executable list anywhere a scan can reach -- `winget
/// list` has no such column, and the aliases an install creates are announced
/// only on `install`'s own stdout ("Command line alias added: ..."), at
/// install time. So these are not executable names; they are the two guesses
/// measured to work, and they go into `Installed.bins` because that is the
/// field `Running::covers` consults.
///
/// Measured on a14 against the live process table, over the 36 source-backed
/// installed winget ids: the whole dotted id (`Installed.name.key()`, the
/// only winget signal that exists today) matched **0**; the id's last dotted
/// segment matched **4**; the folded display `Name` matched **2**. Both are
/// returned because they are different signals -- `Google.Chrome` is reached
/// only by the segment (`chrome`), and neither is reached by the id.
///
/// Over-matching is deliberate, per `Running::covers`'s own rule: "A false
/// positive costs one `!` line the user clears by closing an app; a false
/// negative costs the app."
///
/// **Known residual gap, measured:** installing `ducaale.xh` created TWO
/// aliases, `xh` and `xhs`, and `xhs` is neither the id, the display name,
/// nor the last segment of either. A package's second alias is invisible to
/// this, and no scan-time source for it exists.
pub(crate) fn guard_names(id: &str, display: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let last = id.rsplit('.').next().unwrap_or(id);
    for raw in [last, display] {
        let folded = raw.trim().to_ascii_lowercase();
        if folded.is_empty() || out.contains(&folded) {
            continue;
        }
        out.push(folded);
    }
    out
}
```

- [ ] **Step 4: Run the unit test to verify it passes**

Run: `cargo test --lib guard_names_are_the_two_signals -- --exact backend::winget::tests::guard_names_are_the_two_signals_measured_to_catch_a_real_process`
Expected: **1 test selected**, PASS.

- [ ] **Step 5: Write the failing integration test**

Add to `tests/winget_scan.rs`:

```rust
#[test]
fn a_winget_installed_entry_carries_guard_names_so_the_running_check_can_fire() {
    // `Running::covers` has three signals and, for winget, exactly one of
    // them could ever fire before this: `dirs` is filled only from
    // `$SCOOP/apps` and `$SCOOP/persist` (so a winget id can never be in
    // it) and `bins` was always empty, leaving a process named after the
    // WHOLE dotted id -- which nothing is. Measured: 0 of 36 caught.
    use dotpkg::model::Running;
    let scan = rows_to_scan(vec![WingetRow {
        name: "Brave".to_string(),
        id: "Brave.Brave".to_string(),
        version: "151.1.93.132".to_string(),
        available: None,
        source: Some("winget".to_string()),
    }]);
    let inst = &scan.installed[0];
    assert_eq!(inst.bins, vec!["brave"], "got {:?}", inst.bins);

    // The real process name on a14, folded and suffix-stripped the way
    // `sys::running_processes` reports it.
    let running = Running::new(
        std::collections::BTreeSet::from(["brave".to_string()]),
        Default::default(),
    );
    assert!(
        running.covers(inst),
        "a running Brave must be covered; before this it was not"
    );

    // The control that must stay green: an unrelated process must NOT cover
    // it. Without this, a `guard_names` that returned every possible string
    // would satisfy the assertion above.
    let unrelated = Running::new(
        std::collections::BTreeSet::from(["notepad".to_string()]),
        Default::default(),
    );
    assert!(!unrelated.covers(inst), "must not over-match to anything");
}
```

- [ ] **Step 6: Run it to verify it fails**

Run: `cargo test --test winget_scan a_winget_installed_entry_carries_guard_names -- --exact a_winget_installed_entry_carries_guard_names_so_the_running_check_can_fire`
Expected: **1 test selected**, FAIL on `assert_eq!(inst.bins, vec!["brave"])` — `bins` is `[]`.

- [ ] **Step 7: Fill `bins` in `rows_to_scan`**

In `src/backend/winget.rs`, in `rows_to_scan`'s final `scan.installed.push(...)`, replace `bins: Vec::new(),` with:

```rust
            bins: guard_names(&group[0].id, &group[0].name),
```

`group[0]`, not `name`: `name` is the folded `Name` key, and `guard_names` needs the **display** id and the display `Name` column, both of which only the row still has.

Then update `rows_to_scan`'s own doc comment: the paragraph beginning *"`arch` and `bucket` are always `None`… `bins` is always empty"* is now false. Replace the `bins` half with a pointer to `guard_names` and to the residual second-alias gap.

- [ ] **Step 8: Run the integration test to verify it passes**

Run: `cargo test --test winget_scan a_winget_installed_entry_carries_guard_names -- --exact a_winget_installed_entry_carries_guard_names_so_the_running_check_can_fire`
Expected: **1 test selected**, PASS.

- [ ] **Step 9: Widen the two doc comments the change makes wrong**

`src/model.rs:190-193` currently reads *"Lowercased, extension-stripped basenames of every executable this package's manifest names."* For winget there is no manifest and neither guard name **is** an executable name. Replace with a contract that is true of both backends — *"names a live process might plausibly report for this package"* — and name each backend's source (scoop: `declared_executables` from the manifest; winget: `guard_names`).

`src/model.rs`'s `Running` doc comment gains one sentence: `dirs` is scoop-only by construction, because `Scoop::running_apps` is its only producer and it only inserts segments under the scoop root.

- [ ] **Step 10: Grep for the three comments that now state the opposite**

```bash
grep -rn "only the first two can ever fire\|name and directory halves" src/ docs/
```

Expected: hits in `src/backend/winget.rs` (fix it — it is source), and in `docs/phase4-notes.md` and `docs/specs/2026-08-09-phase4-backend-winget-design.md` (leave those: this project records corrections in the *new* document rather than editing history, and `docs/specs/2026-08-10-phase4b-winget-executor-design.md` already carries the correction).

- [ ] **Step 11: Run the full suite**

Run: `cargo test --no-fail-fast`
Expected: **513 passed, 0 failed** (511 + the 2 new tests).
Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: both clean.

- [ ] **Step 12: Commit**

```bash
git add src/backend/winget.rs src/model.rs tests/winget_scan.rs
git commit -m "Give a winget Installed the guard names its running check needs"
```

---

### Task 3: Replace the `brave.brave` fixture that made the running guard look tested

**Files:**
- Modify: `tests/planner.rs:395-418`
- Test: same file

**Interfaces:**
- Consumes: `guard_names` behaviour from Task 2 (via `rows_to_scan`).
- Produces: nothing new.

**Why:** `tests/planner.rs:404` builds `Running::new(BTreeSet::from(["brave.brave"]), Default::default())`. No machine produces a process named `brave.brave` — Brave's is `brave.exe`, which `sys::normalize` reports as `brave`. So the one test guarding "a running winget package is never turned into a `ReportedOnly` line" is green against a `Running` set that cannot exist. This is `docs/phase4-notes.md`'s "test that cannot fail" class, one step worse than the `resolve_root` case: there the assertion was vacuous on one platform; here the fixture encodes a false premise about the world, so it is green on every platform forever.

- [ ] **Step 1: Prove the existing test is vacuous**

Change `"brave.brave"` to `"brave"` in `tests/planner.rs:405` and run:

Run: `cargo test --test planner -- --exact <the test's name at line 395>`
Expected: **1 test selected**, and it **FAILS** — the planner produces `SkipReason::ReportedOnly`, not `SkipReason::Running`, because before Task 2 nothing connects `brave` to `Brave.Brave`.

**If it passes**, Task 2 has already fixed it and this step's purpose is served differently: record that, and skip to Step 3.

- [ ] **Step 2: Restore, then write the honest test**

Replace the whole test with one built from `rows_to_scan` rather than from a hand-made `Installed`, so the `bins` under test are the ones production would produce:

```rust
#[test]
fn a_running_winget_package_is_skipped_for_running_not_reported_only() {
    // The fixture this replaces used a process named "brave.brave" -- the
    // whole dotted id. Nothing produces that: Brave's process is brave.exe,
    // which `sys::normalize` reports as "brave". So the old test was green
    // against a machine state that cannot exist, and on a real machine the
    // guard caught 0 of 36 installed winget packages.
    //
    // Built through `rows_to_scan` on purpose: a hand-made `Installed` would
    // let this test pass with `bins` values production never produces.
    let scan = dotpkg::backend::winget::rows_to_scan(vec![
        dotpkg::backend::winget::WingetRow {
            name: "Brave".to_string(),
            id: "Brave.Brave".to_string(),
            version: "151.1.93.132".to_string(),
            available: None,
            source: Some("winget".to_string()),
        },
    ]);

    let p = plan(
        &config::parse("[winget]\npackages = [\"Brave.Brave\"]\n").unwrap(),
        &lock::parse(
            "[winget.\"Brave.Brave\"]\nversion = \"151.1.93.134\"\npin = \"version-only\"\n",
        )
        .unwrap(),
        &scan.installed,
        &scan.opaque,
        &State::default(),
        &Running::new(
            // What a real machine reports.
            BTreeSet::from(["brave".to_string()]),
            Default::default(),
        ),
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: WINGET.into(),
            name: "Brave.Brave".into(),
            reason: SkipReason::Running,
        }],
        "a running winget package must be Running, never a version change"
    );
}
```

Note the trailing `&[]` — `plan()` gains its `unscannable` parameter in Task 6. **Until Task 6 lands, omit that argument.** If this task runs before Task 6, drop the `&[]` line; if after, keep it.

- [ ] **Step 3: Add the counterweight the old test never had**

```rust
#[test]
fn an_idle_winget_package_still_reports_its_version_difference() {
    // The positive control. Without it, a planner that returned
    // SkipReason::Running for every winget package would satisfy the test
    // above -- which is exactly the shape that made the old fixture useless.
    let scan = dotpkg::backend::winget::rows_to_scan(vec![
        dotpkg::backend::winget::WingetRow {
            name: "Brave".to_string(),
            id: "Brave.Brave".to_string(),
            version: "151.1.93.132".to_string(),
            available: None,
            source: Some("winget".to_string()),
        },
    ]);
    let p = plan(
        &config::parse("[winget]\npackages = [\"Brave.Brave\"]\n").unwrap(),
        &lock::parse(
            "[winget.\"Brave.Brave\"]\nversion = \"151.1.93.134\"\npin = \"version-only\"\n",
        )
        .unwrap(),
        &scan.installed,
        &scan.opaque,
        &State::default(),
        // Nothing running.
        &Running::default(),
    );
    assert_eq!(p.actions.len(), 1, "got {:?}", p.actions);
    assert!(
        !matches!(
            &p.actions[0],
            Action::Skip { reason: SkipReason::Running, .. }
        ),
        "an idle package must not be Running: {:?}",
        p.actions[0]
    );
}
```

- [ ] **Step 4: Run both to verify they pass**

Run: `cargo test --test planner a_running_winget_package_is_skipped an_idle_winget_package_still_reports`
Expected: **2 tests selected**, both PASS.

- [ ] **Step 5: Run the full suite**

Run: `cargo test --no-fail-fast`
Expected: **514 passed, 0 failed** (513 + 1 net: one test replaced, one added).

- [ ] **Step 6: Commit**

```bash
git add tests/planner.rs
git commit -m "Replace the running-winget test's impossible process name"
```

---

### Task 4: `Step` splits by backend, so a winget step cannot reach scoop's executor

**Files:**
- Modify: `src/execute.rs:56-107` (`Step`, `Step::app`, `order`), `:136-259` (`run_step`), `:397-446` (`write_recovery`), `:488-529` (`execute`)
- Modify: `src/apply.rs:564-599` (`plan_to_steps`), `:714-728` (`gate_removals`)
- Test: `tests/execute.rs`

**Interfaces:**
- Consumes: `guard_names` values, via `Installed.bins` (Task 2).
- Produces:

```rust
pub enum Step { Scoop(ScoopStep), Winget(WingetStep) }

pub enum ScoopStep {
    Install { app: Name, staged: PathBuf, arch: Option<String> },
    Replace { app: Name, staged: PathBuf, arch: Option<String> },
    Remove  { app: Name },
}

pub enum WingetStep {
    /// Install OR version-change: one `install --version` call either way.
    Set    { id: Name, version: String, guard: Vec<String> },
    Remove { id: Name, version: String, guard: Vec<String> },
}

impl Step { pub fn app(&self) -> &Name; pub fn guard_names(&self) -> &[String]; pub fn is_remove(&self) -> bool; }
```

Tasks 11–14 consume `WingetStep`; Task 16 consumes both.

**Why:** `Step` names only `app`, `staged` and `arch`, and `plan_to_steps` matches `Action::Install { name, arch, .. }` — **ignoring `backend`**. The only thing keeping a winget action out of scoop's executor is `stage_and_fetch`'s `backend != SCOOP` check at the *staging* layer (`src/apply.rs:509`), which is the wrong layer for the guard. Splitting the type makes the mistake unwritable.

**`WingetStep` has no `Replace`, and that is the point.** `ScoopStep::Replace` exists because scoop cannot change a version any other way — `install` over an installed app is a measured no-op — so it needs an uninstall half and therefore a window where the package is absent. Measured, winget's `install --version <pin>` performs the upgrade directly (0.24.1 → 0.26.1, exit 0), so a winget version change opens no such window and `run_step`'s `touched` bookkeeping has no uninstall half to reason about.

- [ ] **Step 1: Write the failing test that a winget step cannot be built for scoop**

Add to `tests/execute.rs`:

```rust
#[test]
fn a_winget_step_and_a_scoop_step_are_different_types() {
    use dotpkg::execute::{ScoopStep, Step, WingetStep};
    let s = Step::Scoop(ScoopStep::Remove {
        app: Name::new("fzf"),
    });
    let w = Step::Winget(WingetStep::Remove {
        id: Name::new("Vivaldi.Vivaldi"),
        version: "8.1.4087.62".to_string(),
        guard: vec!["vivaldi".to_string()],
    });
    assert_eq!(s.app(), &Name::new("fzf"));
    assert_eq!(w.app(), &Name::new("Vivaldi.Vivaldi"));
    assert!(s.is_remove() && w.is_remove());
    // The guard names travel with the step, because `execute`'s re-sampler
    // has only a Step and `covers_name` (its two-signal form) is 0-of-36 for
    // winget -- see Task 5.
    assert_eq!(s.guard_names(), &[] as &[String]);
    assert_eq!(w.guard_names(), &["vivaldi".to_string()]);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --test execute a_winget_step_and_a_scoop_step -- --exact a_winget_step_and_a_scoop_step_are_different_types`
Expected: **1 test selected**, FAIL to compile — `ScoopStep` and `WingetStep` do not exist.

- [ ] **Step 3: Split the type**

In `src/execute.rs`, rename today's `Step` to `ScoopStep` verbatim (its three variants and their fields are unchanged), then add `WingetStep` and the new outer `Step` exactly as in **Interfaces** above. Implement:

```rust
impl Step {
    pub fn app(&self) -> &Name {
        match self {
            Step::Scoop(s) => s.app(),
            Step::Winget(WingetStep::Set { id, .. }) | Step::Winget(WingetStep::Remove { id, .. }) => id,
        }
    }
    /// Names a live process might report for this step's package, for
    /// `execute`'s per-step re-sampler. Empty for scoop, whose packages are
    /// already reachable through `Running`'s `dirs` half and whose `bins` the
    /// planner consulted at plan time.
    pub fn guard_names(&self) -> &[String] {
        match self {
            Step::Scoop(_) => &[],
            Step::Winget(WingetStep::Set { guard, .. }) | Step::Winget(WingetStep::Remove { guard, .. }) => guard,
        }
    }
    pub fn is_remove(&self) -> bool {
        matches!(self, Step::Scoop(ScoopStep::Remove { .. }) | Step::Winget(WingetStep::Remove { .. }))
    }
}
```

- [ ] **Step 4: Update `order`, and say why `DEFER_LAST` is scoop-only**

`order`'s sort key becomes:

```rust
pub fn order(mut steps: Vec<Step>) -> Vec<Step> {
    steps.sort_by_key(|s| {
        let group = match s {
            Step::Scoop(ScoopStep::Install { .. }) | Step::Winget(WingetStep::Set { .. }) => 0u8,
            Step::Scoop(ScoopStep::Replace { .. }) => 1,
            Step::Scoop(ScoopStep::Remove { .. }) | Step::Winget(WingetStep::Remove { .. }) => 2,
        };
        // DEFER_LAST is scoop-only by construction: it holds back `git` and
        // the extraction helpers because `Scoop::stage` shells out to git and
        // scoop unpacks with 7zip/dark/innounp/lessmsi. Nothing in the winget
        // path shells out to any of them -- winget downloads and extracts
        // inside its own process -- so a winget id whose last segment happens
        // to be "git" must not be deferred for a reason that does not apply
        // to it.
        let deferred = match s {
            Step::Scoop(_) => u8::from(DEFER_LAST.contains(&s.app().key())),
            Step::Winget(_) => 0,
        };
        (group, deferred, s.app().key().to_string())
    });
    steps
}
```

`WingetStep::Set` groups with installs, not replacements: it is one call and opens no absent-window, so it carries none of the reason `Replace` sorts after `Install`.

- [ ] **Step 5: Make `run_step` and `execute` route, and finalise both signatures here**

**`run_step` and `execute` each gain a `wm: &dyn WingetMutator` parameter in this
step**, from Task 11 — which is why Task 11 runs before this task (see Execution
Order). The parameter is unused until Task 14 fills the winget arm, and that is
deliberate: finalising the signature here is what lets Tasks 5 and 7 write tests
that compile and run inside their own task instead of deferring them. An unused
parameter for two tasks is a smaller cost than two tasks that cannot run their
own tests.

`run_step`'s existing body becomes `run_scoop_step(root, m, state, step: &ScoopStep)` unchanged. Add:

```rust
pub fn run_step(
    root: &Path,
    m: &dyn Mutator,
    state: &mut State,
    step: &Step,
) -> StepOutcome {
    match step {
        Step::Scoop(s) => run_scoop_step(root, m, state, s),
        // Task 14 replaces this. A `todo!()` would panic in a release build
        // on a real machine; a Failed outcome reports and continues, which is
        // this module's own contract ("one package's failure never stops
        // another's").
        Step::Winget(w) => StepOutcome::Failed {
            why: format!("{}: the winget executor is not wired yet", w_app(w)),
            touched: false,
        },
    }
}
```

`gate_removals` uses `step.is_remove()` instead of `matches!(step, Step::Remove { .. })`.

- [ ] **Step 6: Update `plan_to_steps` to route on the ACTION's backend**

In `src/apply.rs:564-599`, every arm gains a backend check. The `Install`/`Upgrade`/`Downgrade` arms already destructure `backend`; use it:

```rust
            (Action::Install { backend, name, arch, .. }, Outcome::ReadyToFetch { manifest })
                if backend == SCOOP =>
            {
                steps.push(Step::Scoop(ScoopStep::Install {
                    app: name.clone(),
                    staged: manifest.clone(),
                    arch: arch.clone(),
                }))
            }
```

and likewise for `Upgrade`/`Downgrade` → `ScoopStep::Replace`, and `Prune` → `ScoopStep::Remove` gated on `backend == SCOOP`. Task 13 adds the winget arms. **Do not add a wildcard that silently drops a non-scoop action** — the existing `_ => {}` arm at the end already exists and must be narrowed so an unrouted action becomes a reported failure rather than a silent no-step. Add, immediately before `_ => {}`:

```rust
            (a, Outcome::ReadyToFetch { .. }) | (a, Outcome::ReadyToRemove) => unusable.push((
                action_name(a),
                format!(
                    "{}: prepared, but no executor claimed it -- this is a routing bug, \
                     not a package problem",
                    action_backend(a)
                ),
            )),
```

where `action_backend` mirrors `action_name`. A prepared action nobody executes must be loud.

- [ ] **Step 7: Fix the fallout in `tests/execute.rs`**

Every `Step::Install {` / `Step::Replace {` / `Step::Remove {` in `tests/execute.rs` becomes `Step::Scoop(ScoopStep::Install {` etc. Mechanical.

- [ ] **Step 8: Run the whole suite**

Run: `cargo test --no-fail-fast`
Expected: **515 passed, 0 failed** (514 + 1 new). Any *failure* here is a routing mistake in Step 6, not test churn.
Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 9: Commit**

```bash
git add src/execute.rs src/apply.rs tests/execute.rs
git commit -m "Split Step by backend so a winget step cannot reach scoop's executor"
```

---

### Task 5: `execute`'s re-sampler stops being blind to winget

**Files:**
- Modify: `src/model.rs` (add `Running::covers_any`), `src/execute.rs:509-521`
- Test: `src/model.rs` unit, `tests/execute.rs` integration

**Interfaces:**
- Consumes: `Step::guard_names()` from Task 4.
- Produces: `pub fn covers_any(&self, name: &Name, guard: &[String]) -> bool` on `Running`.

**Why:** `execute` calls `running().covers_name(&app)` (`src/execute.rs:513`), the deliberately weaker two-signal form, because a `Step` used to carry only a `Name`. After Task 2 that call is **still 0-of-36 for winget**, because `covers_name` has no `bins` half at all. Task 2 closes the plan-time hole; this closes the during-the-run hole, and the during-the-run one is the case the sampler exists for — *"a prefetch of two dozen packages can take minutes, and a user who opens their editor partway through must not have it uninstalled out from under them."*

- [ ] **Step 1: Write the failing unit test**

Add to `src/model.rs`'s test module:

```rust
#[test]
fn covers_any_sees_a_guard_name_that_covers_name_cannot() {
    // `covers_name` is dirs-or-names only. For a winget package `dirs` can
    // never contain the id (it is filled from the scoop root alone) and the
    // id itself is never a process name, so the mid-run re-sampler was
    // 0-of-36 even after `bins` was populated for the planner.
    let r = Running::new(BTreeSet::from(["brave".to_string()]), BTreeSet::new());
    let id = Name::new("Brave.Brave");
    assert!(!r.covers_name(&id), "this is the blind spot being closed");
    assert!(r.covers_any(&id, &["brave".to_string()]));

    // Both halves of covers_name must still work through covers_any, or a
    // scoop step (whose guard list is empty) loses its guard entirely.
    assert!(r.covers_any(&Name::new("brave"), &[]), "the names half");
    let by_dir = Running::new(BTreeSet::new(), BTreeSet::from([Name::new("nodejs")]));
    assert!(by_dir.covers_any(&Name::new("nodejs"), &[]), "the dirs half");

    // And it must not match everything.
    assert!(!r.covers_any(&Name::new("fzf"), &["notepad".to_string()]));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --lib covers_any_sees_a_guard_name -- --exact model::tests::covers_any_sees_a_guard_name_that_covers_name_cannot`
Expected: **1 test selected**, FAIL — `covers_any` does not exist.

- [ ] **Step 3: Implement `covers_any`**

Add to `impl Running` in `src/model.rs`:

```rust
    /// `covers_name`'s two signals plus an explicit guard list, for a caller
    /// that has a package name and a set of plausible process names but no
    /// `Installed` -- which is `execute`'s per-step re-sampler, whose only
    /// input is a `Step`.
    ///
    /// `covers` remains the form to use wherever an `Installed` is available;
    /// this exists because the executor deliberately does not carry one.
    pub fn covers_any(&self, name: &Name, guard: &[String]) -> bool {
        self.covers_name(name) || guard.iter().any(|g| self.names.contains(g))
    }
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test --lib covers_any_sees_a_guard_name -- --exact model::tests::covers_any_sees_a_guard_name_that_covers_name_cannot`
Expected: **1 test selected**, PASS.

- [ ] **Step 5: Use it in `execute`**

`src/execute.rs:513` becomes:

```rust
        if running().covers_any(&app, step.guard_names()) {
```

- [ ] **Step 6: Write the integration test that the sampler holds a winget step**

Add to `tests/execute.rs`, in the same style as the existing re-sampler test:

```rust
#[test]
fn a_winget_package_that_starts_running_mid_run_is_held() {
    // The case the re-sampler exists for, for the backend it could not see.
    // Before `covers_any`, this step ran and the browser was replaced.
    let steps = vec![Step::Winget(WingetStep::Remove {
        id: Name::new("Brave.Brave"),
        version: "151.1.93.132".to_string(),
        guard: vec!["brave".to_string()],
    })];
    let mut state = State::default();
    let sample = || {
        Running::new(
            std::collections::BTreeSet::from(["brave".to_string()]),
            Default::default(),
        )
    };
    let ex = /* call execute with a winget-only step list; see Task 15 for the
                signature once both mutators are parameters */;
    assert_eq!(ex.held(), 1, "got {:?}", ex.results);
    assert!(matches!(&ex.results[0].1, ItemResult::Held(_)));
}
```

`execute`'s signature is already final: Task 4 gave it the `&dyn WingetMutator`
parameter (see Execution Order for why Task 4 runs before this one). So pass
`FakeWingetMutator::unreachable()` here — the step is held before any mutation is
attempted, so a winget call would mean the guard did not fire, and
`unreachable()` turns that into a loud panic instead of a silent pass.

**Then verify this test can fail:** delete the `covers_any` call added in Step 5,
confirm the test goes red, and restore it. A guard test that was never seen red is
a guard test that proves nothing — three of Phase 4's fifteen plan defects were
exactly this shape.

- [ ] **Step 7: Run the suite**

Run: `cargo test --no-fail-fast`
Expected: **516 passed, 0 failed**.

- [ ] **Step 8: Commit**

```bash
git add src/model.rs src/execute.rs
git commit -m "Let the mid-run re-sampler see a winget package"
```

---

### Task 6: "winget could not be scanned" stops being spelled the same as "winget found nothing"

**Files:**
- Modify: `src/backend/winget.rs` (`WingetCmd::run`'s error type, `Winget::scan`), `src/backend/mod.rs` (`ScanOutcome`, `scan_or_warn`), `src/plan.rs` (`SkipReason::Unscannable`, `plan()`'s new parameter, `plan_backend`), `src/apply.rs` (`classify`, `is_outstanding`), `src/main.rs` (`print_scan_warnings_and_merge`)
- Test: `tests/winget_scan.rs`, `tests/planner.rs`, `src/backend/mod.rs` unit

**Interfaces:**
- Produces:

```rust
// src/backend/winget.rs
pub enum CmdError { NotFound, Other(anyhow::Error) }
pub trait WingetCmd { fn run(&self, args: &[&str]) -> Result<CmdOut, CmdError>; }

// src/backend/mod.rs
pub enum ScanOutcome { Scanned(Scan), Unscannable(String) }
pub fn scan_or_warn(backend: &dyn Backend) -> ScanOutcome;

// src/plan.rs
pub enum SkipReason { Running, NotLocked, Opaque, Unscannable, ReportedOnly(Divergence) }
pub fn plan(declared: &Config, lock: &Lock, installed: &[Installed], opaque: &[Name],
            state: &State, running: &Running, unscannable: &[&'static str]) -> Plan;
```

**Why:** two defects in one family, both currently safe only because winget cannot act. `Winget::scan` treats **every** `WingetCmd::run` error as "winget is absent", because `anyhow::Result` erases `io::ErrorKind`; and `scan_or_warn`'s doc comment justifies safety only in the prune direction, while in the other direction a declared, locked, installed, converged winget package renders as `Divergence::Install` after any empty scan. Today a wrong report line; with `Capability::Acts` (Task 13) it is dotpkg installing a package that is already there.

- [ ] **Step 1: Write the failing test that a broken winget is not an absent winget**

Add to `tests/winget_scan.rs`:

```rust
#[test]
fn a_winget_that_fails_for_a_reason_other_than_absence_is_not_an_empty_machine() {
    // `Scoop::scan` distinguishes NotFound from every other io error kind.
    // `Winget::scan` could not, because the trait returned anyhow::Result and
    // the kind was gone before `scan` saw it -- so a broken or
    // permission-denied winget.exe read as "this machine has no winget",
    // which is the one answer `mass_prune_guard` exists to catch too late.
    use dotpkg::backend::{scan_or_warn, ScanOutcome};
    let broken = FakeWinget::failing_with(dotpkg::backend::winget::CmdError::Other(
        anyhow::anyhow!("Access is denied. (os error 5)"),
    ));
    match scan_or_warn(&Winget::new(broken)) {
        ScanOutcome::Unscannable(why) => assert!(
            why.contains("Access is denied"),
            "the cause must survive: {why}"
        ),
        ScanOutcome::Scanned(s) => panic!("a broken winget must not scan as empty: {s:?}"),
    }

    // The positive control: a genuinely ABSENT winget is still a valid,
    // empty machine -- not every Windows install has winget.exe.
    match scan_or_warn(&Winget::new(FakeWinget::failing_with(
        dotpkg::backend::winget::CmdError::NotFound,
    ))) {
        ScanOutcome::Scanned(s) => assert!(s.installed.is_empty() && s.warnings.len() == 1),
        ScanOutcome::Unscannable(why) => panic!("an absent winget is a valid empty machine: {why}"),
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --test winget_scan a_winget_that_fails_for_a_reason_other_than_absence -- --exact a_winget_that_fails_for_a_reason_other_than_absence_is_not_an_empty_machine`
Expected: **1 test selected**, FAIL to compile — `CmdError`, `ScanOutcome` and `FakeWinget::failing_with` do not exist.

- [ ] **Step 3: Introduce `CmdError` and thread it through**

In `src/backend/winget.rs`:

```rust
/// Why a winget invocation could not be made at all -- distinct from a
/// winget that ran and reported failure through its exit code.
///
/// `anyhow::Error` erases `io::ErrorKind`, and `Winget::scan` needs exactly
/// that one bit: a machine with no `winget.exe` is a legitimate, empty
/// machine, while a `winget.exe` that exists and cannot be run is a machine
/// whose state dotpkg does not know. Before this split both reached the same
/// arm -- `Scoop::scan` had always distinguished them.
#[derive(Debug)]
pub enum CmdError {
    /// `winget.exe` is not on `PATH`.
    NotFound,
    Other(anyhow::Error),
}

impl std::fmt::Display for CmdError { /* NotFound => "winget.exe is not on PATH"; Other(e) => "{e:#}" */ }
```

`RealWinget::run` maps `Command::output()`'s `Err(e)` by `e.kind() == std::io::ErrorKind::NotFound` into `CmdError::NotFound`, everything else into `CmdError::Other`.

`Winget::scan`'s error arm splits: `CmdError::NotFound` keeps today's empty-`Scan`-plus-warning behaviour; `CmdError::Other` returns `Err`.

Update `Winget::resolve_latest`/`resolve_installed`/`update_source` call sites: they already turn any error into `Resolution::Failed`/`Err`, so they need only the new type in their `match`.

- [ ] **Step 4: Add `FakeWinget::failing_with` and keep `failing_to_spawn`**

In `tests/common/fake_winget.rs`, `Plan::FailingToSpawn` becomes `Plan::Failing(CmdError)`; `failing_to_spawn()` constructs `Plan::Failing(CmdError::NotFound)` so no existing test changes meaning, and `failing_with(e)` is the new constructor. `CmdError` is not `Clone`, so `Plan::Failing` holds it in an `Option` and `run` takes it, panicking with a clear message on a second call — a fake asked twice for a one-shot error is a test/fake mismatch, matching `Plan::Script`'s existing rule.

- [ ] **Step 5: Introduce `ScanOutcome`**

In `src/backend/mod.rs`, replace `scan_or_warn`'s `-> Scan` with `-> ScanOutcome` as in **Interfaces**. Rewrite its doc comment: the paragraph beginning *"Continuing with an empty `installed`/`opaque` is safe here in a way it would not be for scoop"* is the half that was only ever true in the prune direction, and it must now say so and point at `SkipReason::Unscannable` as the other half's answer.

- [ ] **Step 6: Write the failing planner test**

Add to `tests/planner.rs`:

```rust
#[test]
fn a_declared_locked_winget_package_is_not_installed_again_when_the_scan_failed() {
    // The over-acting direction `scan_or_warn`'s doc comment never covered.
    // An empty scan turns every declared+locked winget package into
    // Divergence::Install -- a divergence that does not exist. Harmless as a
    // report line; with Capability::Acts it is dotpkg installing a package
    // that is already there.
    let p = plan(
        &config::parse("[winget]\npackages = [\"Brave.Brave\"]\n").unwrap(),
        &lock::parse(
            "[winget.\"Brave.Brave\"]\nversion = \"151.1.93.134\"\npin = \"version-only\"\n",
        )
        .unwrap(),
        &[],   // the failed scan: nothing installed, nothing opaque
        &[],
        &State::default(),
        &Running::default(),
        &[WINGET],
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: WINGET.into(),
            name: "Brave.Brave".into(),
            reason: SkipReason::Unscannable,
        }],
        "got {:?}",
        p.actions
    );
    assert_eq!(p.change_count(), 0, "an unscannable backend performs nothing");
}

#[test]
fn an_unscannable_backend_does_not_silence_the_other_one() {
    // The control. A winget scan failure must not stop scoop's entirely
    // unrelated half of the run -- the same reasoning `scan_or_warn` was
    // added for in Phase 4.
    let p = plan(
        &config::parse("[scoop]\npackages = [\"fzf\"]\n[winget]\npackages = [\"Brave.Brave\"]\n")
            .unwrap(),
        &lock::parse(
            "[scoop.fzf]\nbucket = \"main\"\ncommit = \"abc\"\nversion = \"0.74.1\"\n",
        )
        .unwrap(),
        &[],
        &[],
        &State::default(),
        &Running::default(),
        &[WINGET],
    );
    assert!(
        p.actions.iter().any(|a| matches!(a, Action::Install { backend, .. } if backend == SCOOP)),
        "scoop must still be planned: {:?}",
        p.actions
    );
}
```

- [ ] **Step 7: Run them to verify they fail**

Run: `cargo test --test planner a_declared_locked_winget_package_is_not_installed_again an_unscannable_backend_does_not_silence`
Expected: **2 tests selected**, both FAIL to compile — `plan` takes six arguments and `SkipReason::Unscannable` does not exist.

- [ ] **Step 8: Add `SkipReason::Unscannable` and `plan()`'s parameter**

`SkipReason` gains:

```rust
    /// This backend's scan could not be completed at all, so the absence of a
    /// name from `installed` is not evidence of anything. Distinct from
    /// `Opaque`, which is per-package: this is the whole backend.
    Unscannable,
```

`plan()` gains `unscannable: &[&'static str]` and passes it to `plan_backend`, whose declared loop begins:

```rust
    if unscannable.contains(&view.backend) {
        for name in view.declared {
            actions.push(Action::Skip {
                backend: view.backend.into(),
                name: name.clone(),
                reason: SkipReason::Unscannable,
            });
        }
        // The undeclared loop is skipped too: `installed` is empty for this
        // backend by construction, so it would emit nothing -- but returning
        // here says that on purpose rather than relying on it.
        return;
    }
```

`classify` gains an arm mapping `SkipReason::Unscannable` to `Intent::Skip` with the text *"this backend could not be scanned -- see the warnings above; nothing was attempted for it"*. `is_outstanding` gains `SkipReason::Unscannable => true` — a scan failure could differ next run, exactly like `Running` and `Opaque`. **Its match has no wildcard on purpose; adding the variant is a compile error until the decision is made here.**

- [ ] **Step 9: Update `main.rs`**

`print_scan_warnings_and_merge` takes `&ScanOutcome` for winget and returns the `unscannable` list alongside `installed`/`opaque`. `load_everything`'s `winget_scan` field becomes `ScanOutcome`. All three `plan()` call sites (`status`, `apply`, and any in `update`/`adopt`) pass the new argument.

- [ ] **Step 10: Run everything**

Run: `cargo test --no-fail-fast`
Expected: **520 passed, 0 failed** (516 + 4 new).
Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 11: Commit**

```bash
git add src/backend src/plan.rs src/apply.rs src/main.rs tests/
git commit -m "Tell a failed winget scan apart from an empty machine, in the type"
```

---

### Task 7: `execute` stops demanding a scoop root on a run with no scoop steps

**Files:**
- Modify: `src/execute.rs:448-467` (`root_looks_like_scoop`), `:488-497` (`execute`)
- Test: `tests/execute.rs`

**Interfaces:**
- Consumes: `Step::Scoop`/`Step::Winget` from Task 4.
- Produces: no signature change to `root_looks_like_scoop`; `execute` calls it conditionally.

**Why:** `root_looks_like_scoop(root)?` is `execute`'s unconditional first line and `main.rs` passes `d.scoop.root()`. A Windows machine with winget and no scoop would have its whole run refused, winget steps included. Masked today only by the `Nothing to do` early return (`src/main.rs:378`), which stops firing once winget produces steps.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_winget_only_run_does_not_need_a_scoop_root() {
    // The check exists because a wrong or typo'd $SCOOP makes every scoop
    // uninstall verify as successful against an empty tree. That hazard is
    // entirely scoop's; refusing a winget-only run for it refuses a run that
    // was never in danger.
    let t = tempfile::tempdir().unwrap(); // no apps/ directory at all
    let steps = vec![Step::Winget(WingetStep::Remove {
        id: Name::new("Vivaldi.Vivaldi"),
        version: "8.1.4087.62".to_string(),
        guard: vec!["vivaldi".to_string()],
    })];
    // Must NOT be Err(...) about a missing apps directory.
    let r = /* execute(...) with the step list above */;
    assert!(r.is_ok(), "a winget-only run was refused: {r:?}");
}

#[test]
fn a_run_with_even_one_scoop_step_still_needs_a_scoop_root() {
    // The control that must stay red-able. Dropping the condition entirely
    // would satisfy the test above and reopen the exact hazard.
    let t = tempfile::tempdir().unwrap();
    let steps = vec![
        Step::Winget(WingetStep::Remove {
            id: Name::new("Vivaldi.Vivaldi"),
            version: "8.1.4087.62".to_string(),
            guard: vec![],
        }),
        Step::Scoop(ScoopStep::Remove { app: Name::new("fzf") }),
    ];
    let r = /* execute(...) */;
    assert!(r.is_err(), "a scoop step against a non-scoop root must refuse");
}
```

`execute`'s signature is final as of Task 4, so both call sites compile here.
Pass `FakeWingetMutator::unreachable()` in the first (a refusal check must not
reach a mutator) and a plain `FakeWingetMutator::returning(0, String::new())` in
the second, whose point is that `execute` returns `Err` before any step runs.

- [ ] **Step 2: Make the check conditional**

```rust
    // Conditional, not unconditional. The hazard this guards is scoop's
    // alone: `verify::verdict` maps "no apps/ directory" to absent and
    // `Expected::Absent` maps absent to Ok(()), so a wrong $SCOOP verifies
    // every scoop uninstall as successful. A winget removal is verified by
    // re-asking winget, which does not read this root at all.
    if steps.iter().any(|s| matches!(s, Step::Scoop(_))) {
        root_looks_like_scoop(root)?;
    }
```

Checked **after** `order(steps)` is bound but **before** the recovery file is written, keeping today's ordering property (refuse before writing anything).

- [ ] **Step 3: Update `root_looks_like_scoop`'s doc comment**

It currently says the check is `execute`'s first act. Say instead that it runs whenever the step list contains a scoop step, and why a winget-only run is exempt.

- [ ] **Step 4: Run the suite**

Run: `cargo test --no-fail-fast`
Expected: **520 passed, 0 failed** (the two new tests are incomplete and are completed in Task 15; they must not be committed in a state that compiles-and-passes vacuously — if they cannot compile yet, they are not committed yet either).

- [ ] **Step 5: Commit**

```bash
git add src/execute.rs
git commit -m "Require a scoop root only for a run that has scoop steps"
```

---

### Task 8: `state.reconcile` runs per backend

**Files:**
- Modify: `src/main.rs:468-472`
- Test: `tests/cli.rs`

**Interfaces:**
- Consumes: `ScanOutcome` from Task 6.
- Produces: nothing new; `State::reconcile` is unchanged.

**Why:** `main.rs` scans scoop after the run and reconciles `SCOOP` only. A winget package removed outside dotpkg leaves a `state.json` entry forever, inflating `owned_count(WINGET)` — the number `mass_prune_guard` reads. The winget scan already exists in the same function; the reconcile pass does not.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_winget_ghost_is_dropped_from_state_at_the_end_of_a_run() {
    // `State::reconcile` already refuses to drop everything on an empty
    // scan, so this is safe; what was missing is that it was never CALLED
    // for winget. An entry with no package is inert for planning (plan()
    // consults `owns` only while iterating installed) but it inflates
    // owned_count(WINGET), which is what mass_prune_guard reads.
    // ... build a state.json owning a winget id that the scan does not
    // report, run `apply` with at least one real step, assert the ghost is
    // gone from state.json afterwards and the live entry is not.
}
```

Write this against `tests/cli.rs`'s existing helpers for building a throwaway config/lock/state and asserting on the written `state.json`. **`State::reconcile` returns `Vec::new()` when `present` is empty and the map is not**, so the test must supply at least one *present* winget package alongside the ghost, or it passes for the wrong reason.

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --test cli a_winget_ghost_is_dropped -- --exact a_winget_ghost_is_dropped_from_state_at_the_end_of_a_run`
Expected: **1 test selected**, FAIL — the ghost survives.

- [ ] **Step 3: Reconcile both backends**

```rust
            // Report only what a fresh scan confirms -- for every backend
            // that acted, not just scoop. A winget entry whose package was
            // removed outside dotpkg would otherwise sit in state.json
            // forever, inflating the count `mass_prune_guard` reads.
            let after_scoop = <Scoop as Backend>::scan(&d.scoop)?;
            let present: Vec<_> = after_scoop.installed.iter().map(|i| i.name.clone()).collect();
            ex.dropped_ghosts = d.state.reconcile(dotpkg::model::SCOOP, &present);

            let winget = Winget::new(RealWinget);
            if let ScanOutcome::Scanned(after_winget) = dotpkg::backend::scan_or_warn(&winget) {
                let present: Vec<_> =
                    after_winget.installed.iter().map(|i| i.name.clone()).collect();
                ex.dropped_ghosts
                    .extend(d.state.reconcile(dotpkg::model::WINGET, &present));
            }
            // An `Unscannable` winget deliberately reconciles nothing: a scan
            // that failed is not evidence that anything is absent, and this
            // is the direction where acting on that mistake deletes an
            // ownership record dotpkg needs to prune the package later.
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test --test cli a_winget_ghost_is_dropped -- --exact a_winget_ghost_is_dropped_from_state_at_the_end_of_a_run`
Expected: **1 test selected**, PASS.

- [ ] **Step 5: Run the suite and commit**

Run: `cargo test --no-fail-fast` → **521 passed, 0 failed**.

```bash
git add src/main.rs tests/cli.rs
git commit -m "Reconcile winget ownership at the end of a run, not just scoop's"
```

---

### Task 9: The round-trip guard gets a text-level half

**Files:**
- Modify: `src/config_edit.rs` (wherever `verify_round_trip`/`verify_round_trip_winget` live)
- Test: `src/config_edit.rs`

**Interfaces:**
- Produces: `fn exactly_one_line_added(before: &str, after: &str) -> Result<()>`, called by both round-trip guards.

**Why:** both guards re-parse the edited text with `config::parse` and compare `Config` values, and `Config` has no field for comments. So `pkg.toml`'s "byte-identical except the added line" promise is unguarded, and the comment-loss bug fixed in Phase 4's Task 16 was invisible to it **by construction** — the next comment-shaped bug sails through identically.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_dropped_trailing_comment_is_caught_by_the_text_level_guard() {
    // The exact bytes of the Phase 4 dogfood's finding: a same-line comment
    // on the array's LAST element, before the closing bracket.
    let before = "[winget]\npackages = [\n  \"ajeetdsouza.zoxide\",  # keep me\n]\n";
    // What the bug produced: the comment gone, one element added.
    let buggy = "[winget]\npackages = [\n  \"ajeetdsouza.zoxide\",\n  \"Vivaldi.Vivaldi\",\n]\n";
    assert!(
        exactly_one_line_added(before, buggy).is_err(),
        "a lost comment must be caught"
    );

    // The positive control: the correct output must pass. Without it, a
    // guard that rejected everything would satisfy the assertion above.
    let good = "[winget]\npackages = [\n  \"ajeetdsouza.zoxide\",  # keep me\n  \"Vivaldi.Vivaldi\",\n]\n";
    assert!(
        exactly_one_line_added(before, good).is_ok(),
        "the correct edit must pass"
    );

    // And a second added line is not "exactly one".
    let two = "[winget]\npackages = [\n  \"ajeetdsouza.zoxide\",  # keep me\n  \"A.A\",\n  \"B.B\",\n]\n";
    assert!(exactly_one_line_added(before, two).is_err());
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --lib a_dropped_trailing_comment_is_caught -- --exact config_edit::tests::a_dropped_trailing_comment_is_caught_by_the_text_level_guard`
Expected: **1 test selected**, FAIL — function does not exist.

- [ ] **Step 3: Implement it**

```rust
/// Every line of `before` must appear in `after`, in order, with exactly one
/// line inserted and nothing else changed.
///
/// The semantic guard beside this one re-parses the text and compares
/// `Config` values, and `Config` has no field for comments -- so it cannot
/// see a lost comment at all. That is not an oversight to be patched in the
/// semantic guard; it is what comparing parsed values means. This is the
/// other half, and the two run together: the semantic one catches a declared
/// package that changed meaning, this one catches a byte that changed
/// without meaning to.
fn exactly_one_line_added(before: &str, after: &str) -> Result<()> {
    let b: Vec<&str> = before.lines().collect();
    let a: Vec<&str> = after.lines().collect();
    anyhow::ensure!(
        a.len() == b.len() + 1,
        "pkg.toml went from {} lines to {} -- exactly one line should have been added",
        b.len(),
        a.len()
    );
    let mut bi = 0usize;
    let mut inserted: Option<&str> = None;
    for line in &a {
        if bi < b.len() && *line == b[bi] {
            bi += 1;
        } else if inserted.is_none() {
            inserted = Some(line);
        } else {
            anyhow::bail!(
                "pkg.toml changed more than one line: {:?} is neither the original text \
                 nor the single added line {:?}",
                line,
                inserted.unwrap()
            );
        }
    }
    anyhow::ensure!(
        bi == b.len(),
        "pkg.toml lost {} of its original line(s) -- the first missing one is {:?}",
        b.len() - bi,
        b[bi]
    );
    Ok(())
}
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test --lib a_dropped_trailing_comment_is_caught -- --exact config_edit::tests::a_dropped_trailing_comment_is_caught_by_the_text_level_guard`
Expected: **1 test selected**, PASS.

- [ ] **Step 5: Wire it into both guards**

Call `exactly_one_line_added(&original_text, &edited_text)?` in `verify_round_trip` and `verify_round_trip_winget`, **before** their existing semantic comparison, so a text-level failure reports the byte-level cause rather than a downstream parse difference. Keep the semantic comparison — this is an addition, not a replacement.

- [ ] **Step 6: Run the suite**

Run: `cargo test --no-fail-fast`
Expected: **522 passed, 0 failed**. If an existing `config_edit` test now fails, the new guard has found a real second instance of the same class — **report it, do not relax the guard**.

- [ ] **Step 7: Commit**

```bash
git add src/config_edit.rs
git commit -m "Guard pkg.toml at the text level too, where a lost comment is visible"
```

---

### Task 10: `sys::elevated()`

**Files:**
- Modify: `Cargo.toml`, `src/sys.rs`
- Test: `src/sys.rs`

**Interfaces:**
- Produces: `pub fn elevated() -> Option<bool>` — `Some(true)`/`Some(false)` on Windows, **`None` everywhere else and whenever the query fails**.

**Why:** Task 15's pre-check needs it. Measured: `winget install` succeeds from an elevated session and `winget uninstall` of that same user-scope package is then refused with `0x8A15007D`. dotpkg's shape is a scheduled `apply`, so an elevated run can install and never remove.

**`None` must never trigger a refusal.** A machine where the query fails is a machine dotpkg knows nothing about, and refusing every winget removal there would be a refusal caused by a missing answer rather than by a measured hazard.

- [ ] **Step 1: Add the dependency, target-gated**

In `Cargo.toml`:

```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.57", features = [
  "Win32_Foundation", "Win32_Security", "Win32_System_Threading",
] }
```

Target-gated so macOS and Linux builds pull nothing new. `windows` 0.57 is already in `Cargo.lock` transitively via `sysinfo`, so this adds no new *version* to the tree — confirm with `cargo tree -i windows` after the edit and record the output in the task report.

- [ ] **Step 2: Write the test that holds on every platform**

```rust
#[test]
fn elevated_answers_or_admits_it_does_not_know() {
    // The only assertion that is true on all three platforms this crate is
    // built on. The VALUE cannot be asserted: it depends on how the test
    // runner was launched. What must hold is that the function is total and
    // never panics -- because its caller (`apply`'s winget removal
    // pre-check) treats `None` as "do not refuse", and a panic here would
    // take down a run that was about to do useful work.
    // The call itself is the assertion on Windows: `#[test]` fails on a panic,
    // and "does not panic" is the property the caller depends on -- `apply`'s
    // winget-removal pre-check treats `None` as "do not refuse", so a panic
    // here would take down a run that was about to do useful work.
    let answer = elevated();
    #[cfg(not(windows))]
    assert_eq!(answer, None, "there is no elevation concept to report here");
    #[cfg(windows)]
    let _ = answer;
}
```

**There is deliberately no `#[cfg(windows)]` assertion on the value**, and the
first draft of this plan had one — `assert!(answer.is_some() || answer.is_none())`
— which is a tautology dressed as a test. The value cannot be asserted: it
depends on how the test runner was launched. Writing a tautology to give the
`cfg` arm a body is `docs/phase4-notes.md`'s pattern 1 (a test that cannot fail,
reading as a pass) reproduced on purpose, so it is not written.

State plainly in the task report: **`elevated()`'s Windows branch is verified
only by Step 5's a14 build and by the dogfood** — the same position
`resolve_root`'s prefix stripping was in, which is the defect the Windows run
caught in Phase 4 after seven tasks of green macOS runs.

- [ ] **Step 3: Implement it**

```rust
/// Whether this process holds an elevated token.
///
/// `None` means "could not tell", and every caller must treat that as "do not
/// refuse". Measured on a14 (docs/measurements-2026-08-10-winget-write-path.md
/// §5): `winget install` succeeds elevated and `winget uninstall` of that same
/// user-scope package is then refused with 0x8A15007D. dotpkg runs as a
/// scheduled `apply`, so an elevated run can install a package and be
/// structurally unable to remove it -- every prune failing forever.
#[cfg(windows)]
pub fn elevated() -> Option<bool> {
    use std::mem;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return None;
        }
        let mut info = TOKEN_ELEVATION::default();
        let mut written = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut info as *mut _ as *mut _),
            mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut written,
        )
        .is_ok();
        let _ = CloseHandle(token);
        if ok {
            Some(info.TokenIsElevated != 0)
        } else {
            None
        }
    }
}

/// No elevation concept to report. `None`, not `Some(false)`: a caller that
/// refuses on `Some(false)` would be wrong here, and one that refuses on
/// `None` is wrong everywhere -- see the Windows arm's doc comment.
#[cfg(not(windows))]
pub fn elevated() -> Option<bool> {
    None
}
```

- [ ] **Step 4: Verify the macOS build and test**

Run: `cargo test --lib elevated_answers_or_admits -- --exact sys::tests::elevated_answers_or_admits_it_does_not_know`
Expected: **1 test selected**, PASS.
Run: `cargo build --all-targets` → zero warnings; `cargo tree -i windows` → record what depends on it.

- [ ] **Step 5: Verify it compiles on Windows — this is the only verification the Windows branch gets**

Build on a14, from a tarball of `Cargo.toml`, `Cargo.lock`, `src/`, `tests/` — never `target/`, never `.git/`. The `windows` 0.57 API shape for `OpenProcessToken`/`GetTokenInformation` (whether they return `Result` or `BOOL`) is version-specific and **has not been compiled anywhere yet**. If it does not compile, fix it against the real crate on the real target; do not guess a second time from the macOS side.

Also print `elevated()`'s value from a one-off binary under both an elevated `ssh` session and under `runas /trustlevel:0x20000`, and record both. Expected: `Some(true)` and `Some(false)`.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/sys.rs
git commit -m "Add sys::elevated, the one bit winget's uninstall refusal turns on"
```

---

### Task 11: winget's mutating argv, and the `WingetMutator` seam

**Files:**
- Create: `src/backend/winget_exec.rs`
- Modify: `src/backend/mod.rs` (`pub mod winget_exec;`)
- Create: `tests/common/fake_winget_mutator.rs`
- Test: `src/backend/winget_exec.rs` (unit, argv), `tests/winget_execute.rs`

**Interfaces:**
- Produces:

```rust
pub const NO_AVAILABLE_UPGRADE: i32 = -1978335189;      // 0x8A15002B
pub const ALREADY_INSTALLED: i32 = -1978335135;         // 0x8A150061
pub const CANNOT_UNINSTALL_ELEVATED: i32 = -1978335107; // 0x8A15007D

pub fn set_argv(id: &Name, version: &str) -> Vec<String>;
pub fn remove_argv(id: &Name, version: &str) -> Vec<String>;
pub fn list_one_argv(id: &Name) -> Vec<String>;

pub trait WingetMutator {
    fn set(&self, id: &Name, version: &str) -> Result<CmdOut, CmdError>;
    fn remove(&self, id: &Name, version: &str) -> Result<CmdOut, CmdError>;
    fn list_one(&self, id: &Name) -> Result<CmdOut, CmdError>;
}
pub struct RealWingetMutator;
```

**Why `set`, not `install` and `upgrade`:** measured, `winget install --version <pin>` performs an upgrade directly (0.24.1 → 0.26.1, exit 0). And `winget upgrade` goes to the **newest** version in the index, not to a requested one — it took the guinea pig 0.26.1 → 0.26.2 while the pin was neither. **A pinning tool cannot use a verb whose target is "latest".** One method, one measured argv, both directions.

- [ ] **Step 1: Write the failing argv test**

```rust
#[test]
fn the_mutating_argv_is_exactly_what_was_measured() {
    // Every flag here has a measured reason, and the argv is part of this
    // module's contract: docs/measurements-2026-08-10-winget-write-path.md
    // §§1-9 are the only invocations winget's exit codes are trusted for.
    assert_eq!(
        set_argv(&Name::new("Brave.Brave"), "151.1.93.134"),
        vec![
            "install", "-e", "--id", "Brave.Brave", "--version", "151.1.93.134",
            "--silent", "--accept-package-agreements", "--accept-source-agreements",
            "--disable-interactivity",
        ]
    );
    assert_eq!(
        remove_argv(&Name::new("Vivaldi.Vivaldi"), "8.1.4087.62"),
        vec![
            "uninstall", "-e", "--id", "Vivaldi.Vivaldi", "--version", "8.1.4087.62",
            "--disable-interactivity", "--accept-source-agreements",
        ]
    );
    assert_eq!(
        list_one_argv(&Name::new("Brave.Brave")),
        vec!["list", "-e", "--id", "Brave.Brave", "--disable-interactivity"]
    );
}

#[test]
fn the_id_on_the_wire_is_the_display_spelling_never_the_folded_key() {
    // Measured: `--exact` is what makes `--id` case-sensitive, on the WRITE
    // verbs too -- `install -e --id SHARKDP.HYPERFINE` returns 0x8A150014
    // ("no package") for a package that exists, where the correctly-cased
    // call reaches 0x8A150017. `Name::key()` is the folded form, so putting
    // it on the wire means "not found" for a package that is there. The lock
    // holds the canonical spelling winget itself echoed back, which is why
    // `-e` is safe here at all.
    let n = Name::new("Git.Git");
    assert!(set_argv(&n, "1").contains(&"Git.Git".to_string()));
    assert!(!set_argv(&n, "1").contains(&"git.git".to_string()));
    assert!(remove_argv(&n, "1").contains(&"Git.Git".to_string()));
    assert!(list_one_argv(&n).contains(&"Git.Git".to_string()));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib the_mutating_argv_is_exactly_what_was_measured the_id_on_the_wire_is_the_display_spelling`
Expected: **2 tests selected**, FAIL — module does not exist.

- [ ] **Step 3: Write `src/backend/winget_exec.rs`**

Module doc comment states the central rule: *winget's exit code is never the verdict.* Then the three constants, each with a doc comment naming the measured argv it was seen under, and for `NO_AVAILABLE_UPGRADE` the fact that **it covers a success and a failure at once** — returned both when the package is already at the version asked for and when winget declines a downgrade. Then the three argv builders (`id.to_string()`, never `id.key()`), the trait, and `RealWingetMutator` delegating to one private `run` that shells out exactly as `RealWinget::run` does, including discarding stderr with the same measured justification (0 bytes across all 27 write-verb invocations, every failure included).

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --lib the_mutating_argv_is_exactly_what_was_measured the_id_on_the_wire_is_the_display_spelling`
Expected: **2 tests selected**, both PASS.

- [ ] **Step 5: Write the recording fake**

`tests/common/fake_winget_mutator.rs`, modelled directly on `tests/common/fake_winget.rs`: `#![allow(dead_code)]`, `Rc<RefCell<_>>` so a clone handed to the code under test shares the call log, and the same four plans — `Constant`, `Script`, `Failing(CmdError)`, `Unreachable`. `Unreachable` matters as much here as it does there: a test that declares no winget packages must make any winget mutation a loud panic, not a silent pass.

- [ ] **Step 6: Prove the fake records argv**

```rust
#[test]
fn the_fake_records_the_argv_the_real_mutator_would_have_run() {
    // A fake nobody can inspect proves nothing about the argv, and the argv
    // is the whole contract -- exit codes are trusted only for these shapes.
    let f = FakeWingetMutator::returning(0, String::new());
    f.set(&Name::new("Brave.Brave"), "151.1.93.134").unwrap();
    assert_eq!(
        f.calls(),
        vec![set_argv(&Name::new("Brave.Brave"), "151.1.93.134")],
        "the fake must record the same argv the builder produces"
    );
}
```

- [ ] **Step 7: Run the suite and commit**

Run: `cargo test --no-fail-fast` → **525 passed, 0 failed**.

```bash
git add src/backend/winget_exec.rs src/backend/mod.rs tests/common/fake_winget_mutator.rs tests/winget_execute.rs
git commit -m "Add winget's mutating argv and its seam, with the three new exit codes"
```

---

### Task 12: `winget_verdict` — the rescan that is the verdict

**Files:**
- Modify: `src/backend/winget_exec.rs`
- Test: `src/backend/winget_exec.rs` unit, `tests/winget_execute.rs` integration

**Interfaces:**
- Consumes: `parse_list`, `rows_to_scan` (`src/backend/winget.rs`), `list_one_argv`, `WingetMutator` (Task 11).
- Produces:

```rust
pub enum WingetState {
    Absent,
    At(String),
    /// Present, but `rows_to_scan`'s rules say its state cannot be established.
    Unconfirmable(String),
}
pub fn winget_verdict(m: &dyn WingetMutator, id: &Name) -> Result<WingetState, CmdError>;
```

**Why:** measured, `0x8A15002B` is returned both when the package is already at the version asked for and when winget declines what was asked; `0x8A150014` from `uninstall` means "no *installed* package", which for a removal is the desired end state and is indistinguishable by code from "that id is wrong". So the exit code cannot be the verdict for either direction, and the rescan is.

**It re-applies `rows_to_scan`'s three opaque rules on purpose.** An id that comes back sourceless, `> `-prefixed or version-disagreeing after a mutation is *"dotpkg cannot confirm this"*, not *"done"*. A second, looser check written just for the executor would make it more credulous than the scanner.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_rescan_that_finds_the_pinned_version_confirms_it() {
    let m = FakeWingetMutator::returning(0, fixture("list-single-with-available.txt"));
    assert_eq!(
        winget_verdict(&m, &Name::new("ducaale.xh")).unwrap(),
        WingetState::At("0.24.1".to_string())
    );
    assert_eq!(m.calls(), vec![list_one_argv(&Name::new("ducaale.xh"))]);
}

#[test]
fn a_rescan_that_finds_nothing_is_absent_not_an_error() {
    // Measured: `list -e --id <absent>` exits 0x8A150014 and prints "No
    // installed package found matching input criteria." For a Remove step
    // this is the DESIRED end state, so it must be a state, not a failure.
    let m = FakeWingetMutator::returning(
        NO_APPLICATIONS_FOUND,
        fixture("list-not-found.txt"),
    );
    assert_eq!(
        winget_verdict(&m, &Name::new("ducaale.xh")).unwrap(),
        WingetState::Absent
    );
}

#[test]
fn a_rescan_whose_row_is_opaque_cannot_confirm_anything() {
    // The rule that keeps the executor from being more credulous than the
    // scanner. `> 17.14.37` is winget saying *at least*; a version it will
    // not commit to cannot confirm a mutation.
    let m = FakeWingetMutator::returning(0, fixture("list-greater-prefix.txt"));
    match winget_verdict(&m, &Name::new("Microsoft.VisualStudio.2022.BuildTools")).unwrap() {
        WingetState::Unconfirmable(why) => {
            assert!(why.contains("> 17.14.37"), "the reason must name it: {why}")
        }
        other => panic!("expected Unconfirmable, got {other:?}"),
    }
}

#[test]
fn a_rescan_of_an_id_installed_at_two_versions_cannot_confirm_either() {
    // 7zip.7zip, measured twice on a14: two rows, two different versions.
    // Picking one would be inventing a fact -- the same reason
    // `rows_to_scan` sends it to `opaque`.
    let m = FakeWingetMutator::returning(0, fixture("list-duplicate-id.txt"));
    assert!(matches!(
        winget_verdict(&m, &Name::new("7zip.7zip")).unwrap(),
        WingetState::Unconfirmable(_)
    ));
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --test winget_execute a_rescan`
Expected: **4 tests selected**, all FAIL — `winget_verdict` does not exist.

- [ ] **Step 3: Implement `winget_verdict`**

```rust
pub fn winget_verdict(m: &dyn WingetMutator, id: &Name) -> Result<WingetState, CmdError> {
    let out = m.list_one(id)?;
    if out.code == crate::backend::winget::NO_APPLICATIONS_FOUND {
        return Ok(WingetState::Absent);
    }
    if out.code != 0 {
        return Ok(WingetState::Unconfirmable(format!(
            "winget list exited {}: {}",
            out.code,
            out.stdout.lines().next().unwrap_or("(no output)")
        )));
    }
    let rows = match crate::backend::winget::parse_list(&out.stdout) {
        Ok(rows) => rows,
        Err(e) => return Ok(WingetState::Unconfirmable(format!("{e:#}"))),
    };
    if rows.is_empty() {
        // Exit 0 with no rows. Measured shape: `list -s msstore` prints the
        // byte-identical "not found" sentence and exits 0, so a zero code
        // does not imply a row exists.
        return Ok(WingetState::Absent);
    }
    let scan = crate::backend::winget::rows_to_scan(rows);
    if let Some(inst) = scan.installed.iter().find(|i| &i.name == id) {
        return Ok(WingetState::At(inst.version.clone()));
    }
    if scan.opaque.iter().any(|o| o == id) {
        return Ok(WingetState::Unconfirmable(
            scan.warnings.join("; "),
        ));
    }
    // A row came back and it is neither this id's `Installed` nor this id's
    // `opaque` entry. `-e --id` cannot match a different package (measured:
    // `--id` never fuzzy-matches, with or without `--exact`), so this is a
    // shape nothing has produced -- reported, never guessed at.
    Ok(WingetState::Unconfirmable(format!(
        "winget list -e --id {id} returned rows that do not name {id}"
    )))
}
```

The `Unconfirmable(scan.warnings.join("; "))` arm depends on `rows_to_scan` warning about every `opaque` push. **It does not for the sourceless case** — that one is a single aggregate warning after the loop, deliberately, to avoid 84 lines per run. So a sourceless row would produce an `Unconfirmable` whose reason is the aggregate sentence. Verify that reads sensibly for one package; if it does not, pass the id through rather than the raw warning list.

- [ ] **Step 4: Run them to verify they pass**

Run: `cargo test --test winget_execute a_rescan`
Expected: **4 tests selected**, all PASS.

- [ ] **Step 5: Run the suite and commit**

Run: `cargo test --no-fail-fast` → **529 passed, 0 failed**.

```bash
git add src/backend/winget_exec.rs tests/winget_execute.rs tests/fixtures/winget
git commit -m "Make the rescan the verdict for a winget mutation"
```

---

### Task 13: winget becomes `Capability::Acts`, and every sentence that says otherwise is rewritten

**Files:**
- Modify: `src/plan.rs` (`BackendView` for winget, `Divergence::describe()`, `Divergence`'s and `SkipReason::ReportedOnly`'s doc comments), `src/apply.rs` (`classify`, `Outcome::ReadyToSet`, `stage_and_fetch`'s backend gate, `plan_to_steps`'s winget arms)
- Test: `tests/planner.rs`, `tests/prepare.rs`

**Interfaces:**
- Consumes: `Step`/`WingetStep` (Task 4), `guard_names` via `Installed.bins` (Task 2).
- Produces: `Outcome::ReadyToSet { version: String }`; `Intent::NeedsLiveness`.

**Why:** this is the task where `Capability::ReportsOnly` stops being true for winget, and **all four sentences in `Divergence::describe()` become lies in the same moment** — every one hardcodes *"dotpkg cannot install or remove winget packages yet"*. `docs/phase4-notes.md` files this as a backend-name-generalisation minor; it is not. It is that document's own pattern 2, pre-scheduled.

- [ ] **Step 1: Grep for every sentence that will become false**

```bash
grep -rn "cannot install or remove winget\|winget packages yet\|reported only\|ReportsOnly\|Capability::ReportsOnly" src/ tests/
```

Record the full list in the task report **before** changing anything. Every hit is either rewritten in this task or justified in the report as still true. This is the step Phase 4 skipped and paid for: a justifying comment that outlived the behaviour it justified, and three tests pinning the false premise.

- [ ] **Step 2: Write the failing planner test**

```rust
#[test]
fn a_declared_locked_uninstalled_winget_package_is_a_real_install_now() {
    let p = plan(
        &config::parse("[winget]\npackages = [\"BurntSushi.ripgrep.MSVC\"]\n").unwrap(),
        &lock::parse(
            "[winget.\"BurntSushi.ripgrep.MSVC\"]\nversion = \"15.2.0\"\npin = \"version-only\"\n",
        )
        .unwrap(),
        &[],
        &[],
        &State::default(),
        &Running::default(),
        &[],
    );
    assert_eq!(
        p.actions,
        vec![Action::Install {
            backend: WINGET.into(),
            name: "BurntSushi.ripgrep.MSVC".into(),
            version: "15.2.0".into(),
            // winget exposes no architecture: `Installed.arch` is always None
            // and `[winget.opts]` does not exist.
            arch: None,
        }]
    );
    assert_eq!(p.change_count(), 1, "it counts as a change now");
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test --test planner a_declared_locked_uninstalled_winget_package_is_a_real_install -- --exact a_declared_locked_uninstalled_winget_package_is_a_real_install_now`
Expected: **1 test selected**, FAIL — the action is `Skip { ReportedOnly(Install{..}) }`.

- [ ] **Step 4: Flip the capability and delete what it existed for**

`src/plan.rs`'s `backends` array: winget's `capability` becomes `Capability::Acts`.

Then **decide, in the task report, whether `Capability`, `SkipReason::ReportedOnly`, `Divergence`, `floor_exit_code`'s `has_reported_only` parameter and `main.rs`'s `has_reported_only` computation survive at all.** With both backends `Acts`, every one of them is dead code. Two defensible answers:

- **Delete them.** `Capability` was introduced specifically to let `plan_backend` special-case `ReportsOnly` against `Acts`; with no `ReportsOnly` backend it is a one-valued enum. Dead code that looks live is how the next phase inherits a surprise.
- **Keep `Capability`, delete the rest.** A third backend will need a capability decided for it by a human, and `docs/phase4-notes.md` names that as one of the two things still standing between this crate and the design's "a new backend slots in without touching the planner" promise.

**Recommendation: keep `Capability` (one variant, `Acts`, with a doc comment saying a future `ReportsOnly` backend re-earns the branch), delete `SkipReason::ReportedOnly`, `Divergence`, `has_reported_only` and its computation.** `Divergence::describe()`'s four false sentences then cannot survive by accident — deleting the type is what guarantees the grep in Step 1 comes back clean.

- [ ] **Step 5: Give `prepare` a winget arm that cannot carry a manifest path**

`stage_and_fetch` returns `Outcome::Failed` for any `backend != SCOOP`, so with `Acts` every winget install would fail preparation. winget has nothing to stage — there is no local manifest, which is the whole content of Phase 4's own correction about *who holds the manifest*.

What `--prepare` **can** do is a liveness check: `show -e --id <canonical> -v <pin>`, whose `0x8A150017` says the pinned version has fallen out of the index. That path already exists in `Winget::resolve_installed`.

`Outcome` gains:

```rust
    /// Ready, and nothing was fetched: winget has no local manifest to stage.
    /// The pinned version was confirmed still present in winget's index.
    ///
    /// A separate variant rather than `ReadyToFetch { manifest: None }`, for
    /// the reason `ReadyToFetch` and `ReadyToRemove` were split from one
    /// another in Phase 2b-2: as one variant, "no manifest" would mean "this
    /// is a winget action" only for values `prepare` itself produced, and an
    /// executor branching on `manifest.is_none()` would be right by luck.
    ReadyToSet { version: String },
```

`Intent` gains `NeedsLiveness`; `classify` routes a winget `Install`/`Upgrade`/`Downgrade` to it; `prepare` gains the branch. `ready_count()` counts `ReadyToSet` alongside the other two ready shapes.

- [ ] **Step 6: Add `plan_to_steps`'s winget arms**

```rust
            (
                Action::Install { backend, name, .. }
                | Action::Upgrade { backend, name, .. }
                | Action::Downgrade { backend, name, .. },
                Outcome::ReadyToSet { version },
            ) if backend == WINGET => steps.push(Step::Winget(WingetStep::Set {
                id: name.clone(),
                version: version.clone(),
                guard: guard_for(name, installed),
            })),
            (Action::Prune { backend, name, version }, Outcome::ReadyToRemove)
                if backend == WINGET =>
            {
                steps.push(Step::Winget(WingetStep::Remove {
                    id: name.clone(),
                    version: version.clone(),
                    guard: guard_for(name, installed),
                }))
            }
```

`Upgrade` and `Downgrade` both become `Set`, because a winget version change is **one** `install --version` call in either direction and dotpkg does not decide the direction — Task 14 translates winget's own refusal. `guard_for` looks the guard names up from the scan's `Installed` for that name, or returns `guard_names(name, name)` when the package is not installed (an `Install` has no `Installed` to read from, and a package that is not running cannot be held anyway).

`plan_to_steps` needs the scan's `installed` slice to do that lookup — add it as a parameter and update its two call sites.

- [ ] **Step 7: Rewrite the render text**

`render.rs`'s winget lines and `--prepare`'s table must stop saying "reported only". Grep from Step 1 is the checklist.

- [ ] **Step 8: Run everything**

Run: `cargo test --no-fail-fast`
Expected: a **large** number of failures on the first run — every test pinning `ReportedOnly` is now pinning a deleted variant. Each must be either rewritten to the new truth or deleted with a reason in the task report. **A test deleted without a written reason is a lost guard.**
Then: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`.

- [ ] **Step 9: Re-run the Step 1 grep**

```bash
grep -rn "cannot install or remove winget\|winget packages yet\|ReportsOnly" src/ tests/
```

Expected: **zero hits in `src/`**. Hits in `docs/` are history and stay.

- [ ] **Step 10: Commit**

```bash
git add src/ tests/
git commit -m "Let winget act, and delete every sentence that said it could not"
```

---

### Task 14: `run_winget_step`

**Files:**
- Modify: `src/execute.rs` (`run_step`'s winget arm), `src/backend/winget_exec.rs`
- Test: `tests/winget_execute.rs`

**Interfaces:**
- Consumes: `WingetMutator`, `winget_verdict`, `WingetState`, the three constants (Tasks 11–12); `WingetStep` (Task 4).
- Produces: `pub fn run_winget_step(m: &dyn WingetMutator, state: &mut State, step: &WingetStep) -> StepOutcome`.

**Why:** this is where the measurements land. Four rules, each with a fixture:

1. `WingetStep::Set` fires `set_argv` once, then `winget_verdict`. `At(v)` where `v == step.version` is `Done`, **whatever the exit code was** — including `0x8A15002B`, the converged case.
2. `At(v)` where `v != step.version` **and** the exit code was `NO_AVAILABLE_UPGRADE` is the downgrade refusal: the machine is ahead of the pin. Named, with `dotpkg update` as the advice.
3. `WingetStep::Remove` fires `remove_argv`, then `winget_verdict`. `Absent` is `Done` — including when the exit code was `NO_APPLICATIONS_FOUND`, which means "no *installed* package" and is the desired end state.
4. `CANNOT_UNINSTALL_ELEVATED` becomes a named failure that says re-running unelevated is the fix.

- [ ] **Step 1: Write the five failing tests**

```rust
#[test]
fn an_install_confirmed_by_the_rescan_is_done() {
    let m = FakeWingetMutator::script(vec![
        (0, fixture("install-version-fresh.txt")),
        (0, fixture("list-single-with-available.txt")), // reports 0.24.1
    ]);
    let mut st = State::default();
    let step = WingetStep::Set {
        id: Name::new("ducaale.xh"),
        version: "0.24.1".to_string(),
        guard: vec!["xh".to_string()],
    };
    assert_eq!(run_winget_step(&m, &mut st, &step), StepOutcome::Done);
    assert_eq!(m.calls().len(), 2, "one mutation, one rescan: {:?}", m.calls());
    assert_eq!(st.ownership(WINGET, &Name::new("ducaale.xh")), Some(Ownership::Installed));
}

#[test]
fn a_converged_package_is_done_even_though_winget_exited_nonzero() {
    // Measured: asking for the version already installed returns
    // 0x8A15002B "No available upgrade found." That is a SUCCESS -- the
    // machine is exactly where the pin says. Reading nonzero as failure
    // would report a failure on a converged machine every run.
    let m = FakeWingetMutator::script(vec![
        (NO_AVAILABLE_UPGRADE, fixture("install-already-installed-no-upgrade.txt")),
        (0, fixture("list-single-with-available.txt")), // still 0.24.1
    ]);
    let mut st = State::default();
    let step = WingetStep::Set {
        id: Name::new("ducaale.xh"),
        version: "0.24.1".to_string(),
        guard: vec![],
    };
    assert_eq!(run_winget_step(&m, &mut st, &step), StepOutcome::Done);
}

#[test]
fn a_machine_ahead_of_its_pin_is_a_named_downgrade_refusal_not_a_bare_failure() {
    // The measured Brave.Brave shape. Same exit code as the converged case
    // above; the rescan is what tells them apart -- which is the whole
    // reason the exit code is never the verdict.
    let m = FakeWingetMutator::script(vec![
        (NO_AVAILABLE_UPGRADE, fixture("install-already-installed-no-upgrade.txt")),
        (0, fixture("list-single-ahead-of-pin.txt")), // reports 0.26.2
    ]);
    let mut st = State::default();
    let step = WingetStep::Set {
        id: Name::new("ducaale.xh"),
        version: "0.24.1".to_string(),
        guard: vec![],
    };
    match run_winget_step(&m, &mut st, &step) {
        StepOutcome::Failed { why, touched } => {
            assert!(!touched, "nothing was changed: {why}");
            assert!(why.contains("0.26.2") && why.contains("0.24.1"), "both versions: {why}");
            assert!(why.contains("will not downgrade"), "the rule: {why}");
            assert!(why.contains("dotpkg update"), "the actionable advice: {why}");
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }
    assert_eq!(
        st.ownership(WINGET, &Name::new("ducaale.xh")),
        None,
        "a refused step must not claim ownership"
    );
}

#[test]
fn a_removal_whose_rescan_finds_nothing_is_done_even_at_0x8A150014() {
    // Measured: `uninstall` of an absent package exits 0x8A150014 and prints
    // "No installed package found matching input criteria." For a Remove,
    // "already gone" is the desired end state -- and the exit code cannot be
    // told apart from "that id is wrong", so the rescan decides.
    let m = FakeWingetMutator::script(vec![
        (NO_APPLICATIONS_FOUND, fixture("uninstall-package-absent.txt")),
        (NO_APPLICATIONS_FOUND, fixture("list-not-found.txt")),
    ]);
    let mut st = State::default();
    st.set(WINGET, &Name::new("ducaale.xh"), Ownership::Installed);
    let step = WingetStep::Remove {
        id: Name::new("ducaale.xh"),
        version: "0.24.1".to_string(),
        guard: vec![],
    };
    assert_eq!(run_winget_step(&m, &mut st, &step), StepOutcome::Done);
    assert_eq!(st.ownership(WINGET, &Name::new("ducaale.xh")), None);
}

#[test]
fn the_elevation_refusal_says_what_to_do_about_it() {
    // Measured: install succeeds elevated, uninstall of that same user-scope
    // package is refused with 0x8A15007D. A scheduled apply at high
    // integrity can install and never remove.
    let m = FakeWingetMutator::script(vec![
        (CANNOT_UNINSTALL_ELEVATED, fixture("uninstall-refused-elevated.txt")),
        (0, fixture("list-single-with-available.txt")), // still installed
    ]);
    let mut st = State::default();
    st.set(WINGET, &Name::new("ducaale.xh"), Ownership::Installed);
    let step = WingetStep::Remove {
        id: Name::new("ducaale.xh"),
        version: "0.24.1".to_string(),
        guard: vec![],
    };
    match run_winget_step(&m, &mut st, &step) {
        StepOutcome::Failed { why, touched } => {
            assert!(!touched, "the package is still there, untouched: {why}");
            assert!(why.contains("elevat"), "name the cause: {why}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(
        st.ownership(WINGET, &Name::new("ducaale.xh")),
        Some(Ownership::Installed),
        "a failed removal must not release ownership -- the package is still there"
    );
}
```

`list-single-ahead-of-pin.txt` is a new fixture: the same shape as `list-single-with-available.txt` but reporting `0.26.2`. Add it in this task.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test --test winget_execute run_winget_step an_install_confirmed a_converged_package a_machine_ahead_of_its_pin a_removal_whose_rescan the_elevation_refusal`
Expected: **5 tests selected**, all FAIL — `run_winget_step` does not exist.

- [ ] **Step 3: Implement `run_winget_step`**

```rust
/// Perform one winget step and prove by rescan that it happened.
///
/// `touched` is `false` on every failure path here, and that is a measured
/// claim rather than an assumption: every failing shape observed
/// (0x8A15002B declining a downgrade, 0x8A15007D refusing an elevated
/// uninstall, 0x8A150014, 0x8A150017) left `winget list` byte-identical.
/// A winget version change is also ONE call -- `install --version` performs
/// the upgrade directly -- so unlike `ScoopStep::Replace` there is no
/// uninstall half that could leave the package absent mid-step.
pub fn run_winget_step(
    m: &dyn WingetMutator,
    state: &mut State,
    step: &WingetStep,
) -> StepOutcome {
    match step {
        WingetStep::Set { id, version, .. } => {
            let out = match m.set(id, version) {
                Ok(out) => out,
                Err(e) => return StepOutcome::Failed {
                    why: format!("{id}: could not run winget install: {e}"),
                    touched: false,
                },
            };
            match winget_verdict(m, id) {
                Err(e) => StepOutcome::Failed {
                    why: format!("{id}: install ran (exit {}) but the rescan could not: {e}", out.code),
                    touched: false,
                },
                Ok(WingetState::At(v)) if v == *version => {
                    if state.ownership(WINGET, id).is_none() {
                        state.set(WINGET, id, Ownership::Installed);
                    }
                    StepOutcome::Done
                }
                Ok(WingetState::At(v)) if out.code == NO_AVAILABLE_UPGRADE => {
                    StepOutcome::Failed {
                        why: format!(
                            "{id}: installed {v}, pinned {version} -- dotpkg will not downgrade \
                             a winget package. Measured: `winget install --version` only ever \
                             moves a package up, and reports \"No available upgrade found\" \
                             instead. Run `dotpkg update` to move the pin forward.",
                        ),
                        touched: false,
                    }
                }
                Ok(WingetState::At(v)) => StepOutcome::Failed {
                    why: format!(
                        "{id}: asked winget for {version} (exit {}), rescan reports {v}",
                        out.code
                    ),
                    touched: false,
                },
                Ok(WingetState::Absent) => StepOutcome::Failed {
                    why: format!(
                        "{id}: install did not happen -- winget exited {} and the rescan finds \
                         nothing installed: {}",
                        out.code,
                        out.stdout.lines().next().unwrap_or("(no output)")
                    ),
                    touched: false,
                },
                Ok(WingetState::Unconfirmable(why)) => StepOutcome::Failed {
                    why: format!("{id}: winget exited {}, and the rescan cannot confirm the result -- {why}", out.code),
                    // Unknown, so treated as touched in the safe direction:
                    // an operator looks instead of being told nothing
                    // happened. Same rule as `verify::Disagreement::Unreadable`.
                    touched: true,
                },
            }
        }
        WingetStep::Remove { id, version, .. } => {
            let out = match m.remove(id, version) {
                Ok(out) => out,
                Err(e) => return StepOutcome::Failed {
                    why: format!("{id}: could not run winget uninstall: {e}"),
                    touched: false,
                },
            };
            match winget_verdict(m, id) {
                Err(e) => StepOutcome::Failed {
                    why: format!("{id}: uninstall ran (exit {}) but the rescan could not: {e}", out.code),
                    touched: false,
                },
                Ok(WingetState::Absent) => {
                    state.remove(WINGET, id);
                    StepOutcome::Done
                }
                Ok(WingetState::At(v)) if out.code == CANNOT_UNINSTALL_ELEVATED => {
                    StepOutcome::Failed {
                        why: format!(
                            "{id}: still installed at {v}. winget refuses to uninstall a \
                             user-scope package while dotpkg is running elevated. Re-run \
                             without elevation.",
                        ),
                        touched: false,
                    }
                }
                Ok(WingetState::At(v)) => StepOutcome::Failed {
                    why: format!(
                        "{id}: uninstall did not happen -- winget exited {} and the rescan \
                         still reports {v}: {}",
                        out.code,
                        out.stdout.lines().next().unwrap_or("(no output)")
                    ),
                    touched: false,
                },
                Ok(WingetState::Unconfirmable(why)) => StepOutcome::Failed {
                    why: format!("{id}: winget exited {}, and the rescan cannot confirm the removal -- {why}", out.code),
                    touched: true,
                },
            }
        }
    }
}
```

- [ ] **Step 4: Run them to verify they pass**

Run: the same filter as Step 2.
Expected: **5 tests selected**, all PASS.

- [ ] **Step 5: Route it from `run_step`**

`run_step`'s winget arm, which Task 4 left as a `Failed` placeholder, becomes `run_winget_step(wm, state, w)`. `run_step` gains the `&dyn WingetMutator` parameter; so does `execute`.

- [ ] **Step 6: Run the suite and commit**

Run: `cargo test --no-fail-fast` → count = previous + 5.

```bash
git add src/execute.rs src/backend/winget_exec.rs tests/winget_execute.rs tests/fixtures/winget
git commit -m "Run a winget step, with the rescan as the verdict and no downgrade"
```

---

### Task 15: Wire it into `main.rs`, with the elevation pre-check

**Files:**
- Modify: `src/main.rs` (the `apply` arm), `src/execute.rs` (`execute`'s signature)
- Test: `tests/cli.rs`, plus completing the deferred tests from Tasks 5 and 7

**Interfaces:**
- Consumes: everything above.
- Produces: `execute(root, steps, m: &dyn Mutator, wm: &dyn WingetMutator, state, running, opts)`.

- [ ] **Step 1: Complete the three tests deferred from Tasks 5 and 7**

`a_winget_package_that_starts_running_mid_run_is_held`, `a_winget_only_run_does_not_need_a_scoop_root`, and `a_run_with_even_one_scoop_step_still_needs_a_scoop_root` all have call sites that could not compile until `execute` took both mutators. Fill them in and run each. **Verify each one can fail**: for the first, delete the `covers_any` guard call and confirm it goes red; restore it.

- [ ] **Step 2: Write the failing pre-check test**

```rust
#[test]
fn an_elevated_run_refuses_a_user_scope_winget_removal_before_anything_happens() {
    // Measured: install succeeds elevated and uninstall of that same
    // user-scope package is then refused. Fail closed, before acting, the
    // same shape as `root_looks_like_scoop`.
    //
    // The fake mutator is `unreachable()`: if the pre-check does not fire,
    // this test panics loudly rather than passing for the wrong reason.
}
```

`elevated()` reads the real process token, so the pre-check must take the answer as a parameter rather than calling `sys::elevated()` internally — otherwise the test's verdict depends on how the test runner was launched, which is exactly the non-discriminating shape Phase 4's `resolve_root` test had. Signature:

```rust
/// `elevated: Option<bool>` — `None` means "could not tell", and must NOT
/// refuse: a machine dotpkg knows nothing about is not a machine with a
/// measured hazard.
pub fn refuse_elevated_winget_removal(
    steps: &[Step],
    elevated: Option<bool>,
    is_user_scope: &dyn Fn(&Name) -> bool,
) -> Result<(), String>;
```

Write four cases: elevated + user-scope removal → refuses; elevated + machine-scope removal → allows; not elevated + user-scope removal → allows; `None` + user-scope removal → allows.

- [ ] **Step 3: Run them to verify they fail, then implement**

Run: `cargo test --test cli an_elevated_run_refuses_a_user_scope_winget_removal`
Expected: **1 test selected**, FAIL.

Implement, then call it from `main.rs`'s `apply` arm **after** `gate_removals` and **before** `execute`, refusing via `refuse(...)` (exit 2, machine untouched). `is_user_scope` is backed by `winget list -e --id <id> --scope user`, whose discrimination Task 1 confirmed.

- [ ] **Step 4: Wire the mutator and run everything**

`main.rs` constructs `RealWingetMutator` and passes it to `execute`. Run:

Run: `cargo test --no-fail-fast`
Expected: all green, count recorded.
Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/execute.rs tests/
git commit -m "Wire the winget executor in, refusing the removal it cannot perform"
```

---

### Task 16: `recover.cmd` gains winget lines, and says what they are worth

**Files:**
- Modify: `src/execute.rs:397-446` (`write_recovery`)
- Test: `tests/execute.rs`

**Interfaces:**
- Consumes: `set_argv` (Task 11), `Step`/`WingetStep` (Task 4).

**Why:** `write_recovery` emits `scoop install` lines only, built from `install_argv` rather than typed out a second time — for a measured reason: hand-duplicating it left a mutation that deleted `-u` from only the recovery line green across the whole suite. The winget half must be built the same way, from `set_argv`.

And the file needs one honest sentence. A scoop recovery line names a manifest dotpkg staged and hash-verified **on local disk**. A winget line is a **request re-resolved against an index dotpkg does not hold**, and if that version has fallen out — measured retention spans 8 to 828 versions, publisher policy, not a winget guarantee — the line fails. Two different promises in one file; the file must not imply they are one.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn the_recovery_file_carries_a_winget_line_built_from_the_mutators_own_argv() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("recover.cmd");
    write_recovery(&p, &[
        Step::Winget(WingetStep::Set {
            id: Name::new("ducaale.xh"),
            version: "0.24.1".to_string(),
            guard: vec![],
        }),
        // A removal never appears: this file only ever puts software BACK.
        Step::Winget(WingetStep::Remove {
            id: Name::new("Vivaldi.Vivaldi"),
            version: "8.1.4087.62".to_string(),
            guard: vec![],
        }),
    ])
    .unwrap();
    let text = std::fs::read_to_string(&p).unwrap();

    // Built from set_argv, not typed twice: a flag added there must appear
    // here without anyone remembering.
    for part in set_argv(&Name::new("ducaale.xh"), "0.24.1") {
        assert!(text.contains(&part), "missing {part:?} from:\n{text}");
    }
    assert!(
        !text.contains("Vivaldi"),
        "a removal must never appear in a file that only reinstalls:\n{text}"
    );
    // The honest sentence about what a winget line is worth. Asserted on a
    // phrase that ONLY that sentence can contain -- `text.contains("winget")`
    // would pass on the argv line itself and prove nothing, which is what the
    // first draft of this plan asserted.
    assert!(
        text.contains("re-resolved against an index dotpkg does not hold"),
        "the file must say what a winget line is worth, not just contain the \
         word winget:\n{text}"
    );
    // And the control: the scoop half's own promise must still be stated, or a
    // rewrite that replaced one sentence with the other would pass above.
    assert!(
        text.contains("hash-verified"),
        "the scoop promise must survive alongside the winget one:\n{text}"
    );
}
```

- [ ] **Step 2: Run to verify it fails, implement, run to verify it passes**

Run: `cargo test --test execute the_recovery_file_carries_a_winget_line -- --exact the_recovery_file_carries_a_winget_line_built_from_the_mutators_own_argv`
Expected: **1 test selected**, FAIL then PASS.

The winget line is `winget ` followed by `set_argv`'s elements joined with spaces; no element can contain a space or a `%` (an id and a dotted version cannot), so the scoop line's `%` → `%%` doubling and quoting are not needed — **state that in a comment** rather than leaving the asymmetry unexplained.

- [ ] **Step 3: Commit**

```bash
git add src/execute.rs tests/execute.rs
git commit -m "Put winget lines in recover.cmd, and say what they are worth"
```

---

### Task 17: Verification, mutation, Windows, dogfood

**Files:** none in `src/`. Produces `docs/dogfood-phase4b-<date>.md` and `docs/phase4b-notes.md`.

- [ ] **Step 1: Full local suite on the tree that will ship**

Run: `cargo test --no-fail-fast`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo build --all-targets`
Expected: all green, zero warnings. Record the exact test count.

- [ ] **Step 2: Windows suite, run #1 — controller-held**

Tarball `Cargo.toml`, `Cargo.lock`, `src/`, `tests/` (never `target/`, never `.git/`), `scp` it, build and run natively on a14. **Before trusting the result, verify `tests/fixtures/winget/list-full.txt` is 30958 bytes with 143 CRLF pairs on the far side.**

Cross-reference **name-by-name**: every source-level `#[test]` name against every `test <name> ... ok|FAILED` line the run printed. Expect exactly two absences, both `#[cfg(unix)]` (named in Global Constraints). **Never subtract totals.**

This is also the first real compile of `sys::elevated()`'s Windows branch (Task 10, Step 5) if that step was deferred.

- [ ] **Step 3: `cargo mutants` — controller-held**

Run on an **idle** machine with `TMPDIR` on a volume with room: `cargo-mutants` builds each job in its own copied tree and manufactures its own competitor (measured: `syspolicyd` at 147.9% CPU signature-checking the binaries the run itself creates). Deliberately **not** `CARGO_TARGET_DIR` — a shared target directory breaks that isolation.

Then re-run **just Phase 4's unresolved timeout set** with `--timeout 600`, per `docs/phase4-notes.md` item 9: 69 of 71 are unresolved, 2 are confirmed genuine hangs (`verify.rs:121`, `:124`), 17 are confirmed starvation (function-replacement mutants cannot hang). The discriminator is *"can this mutant hang at all"*, not the count.

Triage every survivor with a written ruling: gap, equivalent, or accepted-with-a-measured-reason.

- [ ] **Step 4: Dogfood on a14, at MEDIUM integrity**

Every prior dogfood used the elevated `ssh`; this is the first phase where that choice decides whether a code path can run at all. `runas /trustlevel:0x20000` is the measured working route; `schtasks /RL LIMITED` is **not**, with or without `/IT` (the task stays `Queued`, `Logon Mode: Interactive only`, `Last Result: 0`).

**Re-derive the machine's numbers; do not reuse the fixtures'** — they have already drifted once.

The seven questions from the spec's Dogfood section, each framed so it can fail. Use a purpose-picked disposable package, not one of the 36 real installed ids, and hash `winget list` before and after to prove the machine returned to its starting state.

- [ ] **Step 5: Windows suite, run #2, on the exact tree that ships**

Phase 4 needed three runs because the tree changed twice after the first. Any fix made during Steps 2–4 means this run happens again on the fixed tree.

- [ ] **Step 6: Write `docs/phase4b-notes.md`**

Carry forward, with the same discipline: what was measured versus what was reasoned, every survivor's ruling, every "deliberately not measured" item from the spec's Non-goals restated as still open, and the method findings (the `Winget` function-shadowing bug, `runas` versus `schtasks`, the capture-method byte artifact).

- [ ] **Step 7: Whole-branch review, then merge**

Review the whole branch by **running** it, not by reading it. Then fast-forward merge to `main` and push, per standing authorization.

---

## Self-Review

**1. Spec coverage.** Every spec section maps to a task: A1 → Tasks 2, 3, 5; A2 → Task 6; A3 → Task 9; A4 → Tasks 1, 10, 15; A5 → Task 4; A6 → Task 7; A7 → Task 8; B1 → Task 11; B2 → Task 12; B3 → Tasks 11, 14; B4 → Tasks 13, 14; B5 → Task 13; B6 → Task 16; Testing/Dogfood → Task 17. The spec's four "Corrections to earlier documents" are covered by Task 2 Step 10 (the three `Running` comments), Task 1 Step 6 (PROVENANCE drift), Task 13 Step 1 (the `describe()` sentences); the `depends` correction is documentation-only and already committed at `25ea0a0`.

**2. Placeholders.** Task 8 Step 1's test body is prose rather than code, because it depends on `tests/cli.rs` helpers whose exact names this plan has not read — the implementer reads them; the two conditions the test must satisfy (a present package alongside the ghost, or it passes for the wrong reason) are stated exactly. Everything else contains the code to write.

**Pre-flight scan, run before Task 1 was dispatched — three findings, all author-side, all fixed in place:**

1. **Three tests with a call site that could not compile** (Task 5 Step 6, Task 7 Step 1 ×2), deferred to Task 15 because `execute`'s signature was not final. That would have ended two tasks with code that does not compile, and a task that cannot run its own tests has no independently testable deliverable. **Fixed** by finalising `execute`'s signature in Task 4 and adding the Execution Order section that makes Task 11 precede it. The deferral is gone.
2. **Task 10 mandated a tautology** — `assert!(answer.is_some() || answer.is_none())` — to give a `#[cfg(windows)]` arm a body. That is `docs/phase4-notes.md`'s pattern 1 written on purpose. **Fixed:** no value assertion on Windows, and the report states outright that the Windows branch's only verification is the a14 build.
3. **Task 16 asserted `text.contains("winget")`**, which the argv line satisfies on its own, so the assertion could not fail for the reason it existed. **Fixed:** asserts a phrase only the explanatory sentence can contain, plus a control that the scoop promise survives alongside it.

The rest of the plan scanned clean against the Global Constraints: no task contradicts another, and no task mandates verbatim duplication of a logic block — Task 16 explicitly forbids it (`recover.cmd`'s winget line is built from `set_argv`, for the measured reason that hand-duplicating the scoop line left a `-u`-deleting mutant green).

**3. Type consistency.** `guard_names` (Task 2) → `Installed.bins` → `Step::guard_names()` (Task 4) → `Running::covers_any` (Task 5): one concept, three names, each pointing at the next. `CmdError` (Task 6) is used by `WingetCmd` and `WingetMutator` (Task 11) — one error type for both seams. `WingetState`/`winget_verdict` (Task 12) are consumed only by `run_winget_step` (Task 14). `Outcome::ReadyToSet` (Task 13) → `WingetStep::Set` (Task 4). `plan()` gains `unscannable` in Task 6, which is why Task 3's test carries a note about the argument's presence depending on task order.

**One gap this review found and closed:** Task 13's `plan_to_steps` winget arms need the scan's `installed` slice to look up guard names, which the function does not currently receive. Added as an explicit sub-step (Task 13, Step 6) rather than left for the implementer to discover.
