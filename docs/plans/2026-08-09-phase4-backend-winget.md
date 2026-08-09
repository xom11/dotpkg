# Phase 4 — generalise the pipeline, then add winget: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan
> task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `Backend` a real seam, then add a winget backend that scans,
resolves, locks and reports — but never acts.

**Architecture:** Half A (Tasks 2–10) turns three hardcoded scoop pipelines into
one backend-parameterised one and closes five carried debts that each get harder
once a second backend exists. Half B (Tasks 11–18) adds `src/backend/winget.rs`,
split at a pure-text seam (`parse_list`, `parse_show`) so the whole thing is
testable on macOS against **bytes captured from a real winget**, with the
subprocess behind a trait so no test spawns `winget.exe`.

**Tech Stack:** Rust 2021, `anyhow`, `serde`/`serde_json`, `toml`/`toml_edit`,
`tempfile`. No new dependency is added by this phase.

**Spec:** [`docs/specs/2026-08-09-phase4-backend-winget-design.md`](../specs/2026-08-09-phase4-backend-winget-design.md)
**Measurements:** [`docs/measurements-2026-08-09-winget.md`](../measurements-2026-08-09-winget.md)
**Fixtures:** `tests/fixtures/winget/` — read `PROVENANCE.md` there first.

## Global Constraints

- **`cargo test --no-fail-fast`** on every run. A failing target must not hide
  the ones after it.
- **No test may create a file at `Scoop::scoop_exe()`'s path, and no test may
  spawn `winget.exe`.** Both are standing rules; the second is new.
- **Fixtures under `tests/fixtures/winget/` are CRLF and stay CRLF.**
  `.gitattributes` pins `tests/fixtures/winget/** -text`. A parser tested only
  against `\n` passes on macOS and fails on Windows.
- **Every negative control must be shown to go red, and the assertion that
  actually fired must be recorded** in the task's report.
- **The rule that outranks this document:** if a negative control cannot be made
  to go red, that is a **failure of this plan**, not of the implementer. Fix the
  test, say so, and do not ask first. Phase 3 lost a round to an implementer who
  diagnosed exactly this correctly, verified the fix, and did not dare apply it.
- **No `unwrap_err()` before a test's other assertions.** Write
  `let r = ...; assert!(r.is_err(), "...");` and put side-effect assertions
  above the point the error is consumed.
- **Every refusal assertion is paired** — with a count of files written (which
  must be zero) or with a positive sibling that must stay green.
- **Windows suite run twice**: once before the dogfood, once at the end of the
  change, on the tree that ships. `ssh -F /dev/null -o BatchMode=yes
  kln@100.83.225.100`; a14 sleeps — if it does not answer, say so, do not
  invent results.
- **Never start or stop `kanata`.** No `winget install`, `winget uninstall`,
  `winget upgrade` or `winget pin add` at any point in this phase.
- Exit codes measured and used verbatim:
  `NO_APPLICATIONS_FOUND = -1978335212` (`0x8A150014`),
  `NO_VERSION_FOUND = -1978335209` (`0x8A150017`).

---

## File Structure

| File | Responsibility |
|---|---|
| `src/backend/mod.rs` | `Scan` (gains `opaque`), `Backend` trait (gains `resolve_*`), `Capability` |
| `src/backend/scoop.rs` | unchanged responsibility; `name()` returns `&'static str`, `scan` fills `opaque`, mutant gaps closed |
| `src/backend/winget.rs` | **new.** `parse_list`, `parse_show`, `parse_versions` (pure); `WingetCmd` seam; `Winget` backend |
| `src/plan.rs` | one pass per backend; `SkipReason::{Opaque, ReportedOnly}`, `Divergence` |
| `src/apply.rs` | `mass_prune_guard` per backend; `entry_coherence` grows a winget arm |
| `src/update.rs` | `Resolution::Resolved { pin: Pin }`; resolves both backends |
| `src/adopt.rs` | adopts from either backend |
| `src/main.rs` | `floor_exit_code` extracted; both backends constructed and scanned |
| `src/render.rs` | renders the new skip reasons |
| `tests/winget_scan.rs` | **new.** `parse_list` / `rows_to_scan` against the fixtures |
| `tests/winget_resolve.rs` | **new.** `parse_show` / `parse_versions` / resolver against the fixtures |

---

## Task 1: Verify `winget source update --name winget` is inert

The spec defers exactly one measurement, and Task 15 depends on the answer.
Bare `winget source update` **installed** winget's own `winget-font` source
MSIX (`docs/measurements-2026-08-09-winget.md` §9). Nothing yet shows the
scoped form does not.

**Files:**
- Modify: `docs/measurements-2026-08-09-winget.md` (append to §9)

**Interfaces:**
- Consumes: nothing.
- Produces: a yes/no that Task 15 reads. If the answer is **no**, Task 15 must
  not run any `source update` and `update --offline` becomes the only winget
  resolve mode until Phase 4b.

- [ ] **Step 1: Capture the installed set, run the scoped update, capture again**

Write `capture.ps1` to the scratchpad and run it via
`scp -F /dev/null` + `powershell -NoProfile -ExecutionPolicy Bypass -File`.
**No backticks in PowerShell string literals** — the backtick is PowerShell's
escape character and it silently destroys the file (this cost one round already).

```powershell
$tmp = $env:TEMP
function Snap($f) {
  Start-Process -FilePath "winget.exe" -ArgumentList @("list","--disable-interactivity") `
    -NoNewWindow -Wait -RedirectStandardOutput (Join-Path $tmp $f) `
    -RedirectStandardError (Join-Path $tmp "x.txt") | Out-Null
}
Snap "su-before.txt"
$p = Start-Process -FilePath "winget.exe" `
      -ArgumentList @("source","update","--name","winget","--disable-interactivity") `
      -NoNewWindow -Wait -PassThru -RedirectStandardOutput (Join-Path $tmp "su-out.txt") `
      -RedirectStandardError (Join-Path $tmp "su-err.txt")
Snap "su-after.txt"
Write-Output ("exit=" + $p.ExitCode)
```

(The backticks above are **line continuations at end of line**, which are legal.
Backticks *inside* a `"..."` literal are what must never appear.)

- [ ] **Step 2: Diff the two captures by field, not by byte**

`scp` both files back and compare `(Name, Id, Version, Source)` as a multiset,
plus the `Available` column separately — the same comparison
`docs/measurements-2026-08-09-winget.md` §9 used. A byte diff is not enough:
column widths move when any field changes.

- [ ] **Step 3: Append the result to §9**

Record the exit code, the row-count delta, any row that appears or disappears,
and the number of `Available` changes. **If a row appears, say so and name it**
— that is the finding, not a failure of the task.

- [ ] **Step 4: Commit**

```bash
git add docs/measurements-2026-08-09-winget.md
git commit -m "Measure whether a scoped winget source update installs anything"
```

---

## Task 2: `Backend::name` returns `&'static str`, and the scoop mutants die

`src/backend/scoop.rs:219` survives mutation to `""` and to `"xyzzy"`.
Everything keys on that string. Do this before there is a second name.

**Files:**
- Modify: `src/backend/mod.rs` (trait signature)
- Modify: `src/backend/scoop.rs:219` and the mutant sites listed below
- Test: `tests/scoop_scan.rs`

**Interfaces:**
- Produces: `fn name(&self) -> &'static str` on `Backend`. Every later task's
  backend impl uses this signature.

- [ ] **Step 1: Write the failing test**

In `tests/scoop_scan.rs`:

```rust
#[test]
fn the_backend_reports_the_name_every_map_and_guard_is_keyed_by() {
    // state.json is a map keyed by this string, plan() compares against
    // model::SCOOP, and owned_count(SCOOP) is what mass_prune_guard reads.
    // Mutating it to "" or "xyzzy" left the whole suite green.
    let s = Scoop::new(std::path::PathBuf::from("/nonexistent"));
    assert_eq!(Backend::name(&s), dotpkg::model::SCOOP);
    assert_eq!(Backend::name(&s), "scoop");
}
```

- [ ] **Step 2: Run it and confirm it passes, then confirm it discriminates**

Run: `cargo test --test scoop_scan the_backend_reports_the_name -- --exact`
Expected: PASS. Then **hand-apply the mutation** — change `fn name` to return
`"xyzzy"` — rerun, and record that this test is the one that fires. Restore.

- [ ] **Step 3: Change the trait to `&'static str`**

```rust
pub trait Backend {
    fn name(&self) -> &'static str;
    fn scan(&self) -> Result<Scan>;
}
```

`Scoop::name` already returns the `SCOOP` constant, so the body is unchanged;
the signature stops an impl from returning a borrowed field.

- [ ] **Step 4: Close the remaining `scoop.rs` survivors**

From `docs/phase3-notes.md`, "The 15 survivors": `:124` ×3 (`resolve_root`'s
`b.len() == s.len()`), `:533`×2 and `:525` (`clone_missing_buckets`' `.git`
guard and its return value), `:699`/`:712` (`download_verdict`'s `&&`),
`:731`×2 (`tail`'s `skip > 0`), `:654` (`strip_ansi`'s `&&`), `:227` (`scan`'s
`NotFound` guard), `:67` (`declared_executables::walk`'s `Value::Array` arm).

`:227` is deferred to Task 4, which rewrites that guard. The
`NotFound`-guard family (`lock.rs:99` closed in Phase 3, `verify.rs:146` still
open) is **not** in scope here — `verify.rs:146` is Still-open item 4 and the
spec's non-goals leave it.

For `resolve_root`'s `b.len() == s.len()`, the discriminating input is a path
that `canonicalize` does **not** prefix, so nothing is stripped:

```rust
#[test]
fn a_root_that_needs_no_prefix_stripping_is_kept_as_canonicalize_returned_it() {
    // The `b.len() == s.len()` arm means "nothing was stripped, keep `canon`
    // itself rather than rebuilding it from a lossy string". Three mutants
    // survived because no test ever took that arm with a real path.
    let d = tempfile::tempdir().unwrap();
    let got = resolve_root(d.path().to_path_buf());
    assert_eq!(got, std::fs::canonicalize(d.path()).unwrap_or_else(|_| d.path().to_path_buf()));
    assert!(!got.to_string_lossy().starts_with(r"\\?\"), "got {got:?}");
}
```

- [ ] **Step 5: Verify the kills**

Run: `cargo mutants -f src/backend/scoop.rs --no-shuffle -- --no-fail-fast`
Expected: the 15 go to at most the ones this task deferred (`:227`). **Record
the actual before/after counts in the task report**, not "the mutants died".

- [ ] **Step 6: Commit**

```bash
git add src/backend/ tests/scoop_scan.rs
git commit -m "Assert the backend's own name, and close the scoop scan mutants"
```

---

## Task 3: Extract `floor_exit_code` as a pure function

`src/main.rs:411`. Two mutants (`&&`→`||`, `delete !`) are unreachable from
`tests/cli.rs`: they diverge only on a fully successful non-empty `apply`, which
no fixture can build without a real scoop. Same seam move as `write_in_order`
and `parse_batch`.

**Files:**
- Modify: `src/main.rs:395-425`
- Test: in `src/main.rs`'s own `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `fn floor_exit_code(code: i32, preparation_ok: bool, has_running_skips: bool) -> i32`.
  Task 18 adds a fourth parameter; do not add it here.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_successful_run_with_nothing_outstanding_keeps_its_own_exit_code() {
    // The case tests/cli.rs cannot construct: no fixture may provide a fake
    // scoop binary, so a fully successful non-empty apply is unreachable there.
    // This is the only case that distinguishes `&&` from `||` and `!` from ``.
    assert_eq!(floor_exit_code(0, true, false), 0);
}

#[test]
fn outstanding_work_floors_a_zero_to_one() {
    assert_eq!(floor_exit_code(0, false, false), 1, "a package that failed to prepare");
    assert_eq!(floor_exit_code(0, true, true), 1, "a package skipped because it is running");
    assert_eq!(floor_exit_code(0, false, true), 1, "both");
}

#[test]
fn a_nonzero_code_is_never_lowered_or_raised() {
    assert_eq!(floor_exit_code(2, true, false), 2);
    assert_eq!(floor_exit_code(2, false, true), 2, "the floor is a floor, not an override");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --bin dotpkg floor_exit_code`
Expected: FAIL, `cannot find function 'floor_exit_code'`.

- [ ] **Step 3: Extract the function verbatim from the call site**

```rust
/// The `apply` exit-code floor, lifted out of `main` so it can be observed.
///
/// A package that failed to PREPARE never becomes a `Step`, so `Execution`
/// cannot see it; a package skipped because its own process was running is
/// outstanding work the user asked for and did not get. Either one means 0
/// would tell a scheduled task the machine is fine when it is not.
///
/// A floor, not an override: a non-zero code passes through untouched.
fn floor_exit_code(code: i32, preparation_ok: bool, has_running_skips: bool) -> i32 {
    if code == 0 && (!preparation_ok || has_running_skips) {
        1
    } else {
        code
    }
}
```

Replace the inline expression at `src/main.rs:411` with
`let code = floor_exit_code(code, preparation.is_ok(), !running_skips.is_empty());`

- [ ] **Step 4: Run the tests and the CLI suite**

Run: `cargo test --no-fail-fast --bin dotpkg && cargo test --no-fail-fast --test cli`
Expected: PASS, and `keep_going_does_not_report_success_when_a_declared_package_could_not_be_prepared`
still passes — that test reaches the floor through the `preparation` branch and
is the positive sibling proving the extraction changed no behaviour.

- [ ] **Step 5: Negative control**

Hand-apply `&&` → `||` in `floor_exit_code`, run
`cargo test --bin dotpkg floor_exit_code`, and **record which assertion fired**
(expect `a_successful_run_with_nothing_outstanding_keeps_its_own_exit_code`,
`left: 1, right: 0`). Restore.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "Extract the apply exit-code floor so its two mutants are reachable"
```

---

## Task 4: `Scan` carries what it could not establish, and `plan()` skips it

`docs/phase3-notes.md` Still-open item 11. On a14, `zellij` and `actionlint` are
installed at exactly the pinned version but their `manifest.json` cannot be
traversed under elevated `ssh`, so `scan` omits them, `plan()` reads the
omission as "not installed", and `--yes` would reinstall two working packages.

**Files:**
- Modify: `src/backend/mod.rs` (`Scan`)
- Modify: `src/backend/scoop.rs` (the three `continue`s that warn)
- Modify: `src/plan.rs` (declared loop, and `SkipReason`)
- Modify: `src/render.rs` (the skip match arm)
- Test: `tests/planner.rs`, `tests/scoop_scan.rs`

**Interfaces:**
- Produces: `Scan { installed, opaque: Vec<Name>, warnings }` and
  `SkipReason::Opaque`. Tasks 6, 12 and 16 all read `opaque`.

- [ ] **Step 1: Write the failing tests**

In `tests/planner.rs`:

```rust
#[test]
fn a_declared_package_the_scan_could_not_read_is_skipped_rather_than_installed() {
    // Measured on a14: zellij and actionlint are installed at exactly the
    // pinned version, but their manifest cannot be traversed, so scan omits
    // them. plan() used to read that omission as "not installed" and emit
    // Install -- which under --yes is uninstall-then-install of a package
    // that was never absent.
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[],                                  // scan found nothing readable
        &[Name::new("fzf")],                  // ...because fzf was opaque
        &State::default(),
        &Running::default(),
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: SCOOP.into(),
            name: "fzf".into(),
            reason: SkipReason::Opaque,
        }]
    );
}

#[test]
fn an_undeclared_package_the_scan_could_not_read_is_not_a_stray_and_not_a_prune() {
    // The counterweight. An entry whose state is unknown is not evidence of
    // a stray, and it must not become a Prune even when dotpkg owns it --
    // "I cannot see it" is not "it is not declared".
    let mut state = State::default();
    state.set(SCOOP, &Name::new("aichat"), Ownership::Adopted);
    let p = plan(
        &config::parse(DECLARED_FZF).unwrap(),
        &lock::parse(LOCK_FZF_741).unwrap(),
        &[installed("fzf", "0.74.1")],
        &[Name::new("aichat")],
        &state,
        &Running::default(),
    );
    assert!(p.actions.is_empty(), "got {:?}", p.actions);
}
```

In `tests/scoop_scan.rs` — the scan half, using a directory that exists with a
manifest that is not readable as a file (a **directory** at `manifest.json`,
which is portable and needs no `chmod`):

```rust
#[test]
fn an_app_whose_manifest_cannot_be_read_is_reported_as_opaque_not_as_absent() {
    let root = tempfile::tempdir().unwrap();
    let current = root.path().join("apps").join("zellij").join("current");
    std::fs::create_dir_all(current.join("manifest.json")).unwrap(); // a DIRECTORY
    let scan = Backend::scan(&Scoop::new(root.path().to_path_buf())).unwrap();
    assert!(scan.installed.is_empty(), "got {:?}", scan.installed);
    assert_eq!(scan.opaque, vec![Name::new("zellij")], "the name must survive");
    assert_eq!(scan.warnings.len(), 1, "and still be explained to the user");
    assert!(scan.warnings[0].contains("zellij"), "got {:?}", scan.warnings);
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --no-fail-fast --test planner --test scoop_scan`
Expected: FAIL to compile — `plan` takes 5 arguments, `Scan` has no field
`opaque`, `SkipReason` has no variant `Opaque`.

- [ ] **Step 3: Add the field, with the doc comment that says why**

In `src/backend/mod.rs`:

```rust
pub struct Scan {
    pub installed: Vec<Installed>,
    /// Installed, but this backend could not establish its state.
    ///
    /// `plan()` must not read a name's absence from `installed` as "not
    /// installed". The scoop case is a manifest that cannot be traversed; the
    /// winget case is a row with no source, which cannot be compared against
    /// any index. Both would otherwise become `Install` and then, under
    /// `--yes`, an uninstall-and-reinstall of a package that was never absent.
    ///
    /// One field rather than two: the *cause* differs per backend and belongs
    /// in `warnings`, but the *consequence* for the planner is identical.
    pub opaque: Vec<Name>,
    pub warnings: Vec<String>,
}
```

- [ ] **Step 4: Fill it in `Scoop::scan`**

Every site in `scan` that currently pushes a warning and `continue`s now also
pushes the name onto `out.opaque`: the `read_dir` entry error, the
non-`NotFound` `read_to_string` error, the `serde_json` parse error, and the
missing-`version` case. **The `NotFound` arm stays a bare `continue`** — a
half-finished install with no manifest yet is the ordinary shape and is not
opaque. That distinction is `scoop.rs:227`'s mutant, deferred here from Task 2;
add the test that kills it:

```rust
#[test]
fn a_half_finished_install_with_no_manifest_yet_is_silent_not_opaque() {
    // The NotFound arm is the benign one and must stay benign: this is the
    // `Err(e) if e.kind() == NotFound => <benign default>` idiom whose other
    // error kinds went untested in three places across this crate.
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("apps").join("fzf").join("current")).unwrap();
    let scan = Backend::scan(&Scoop::new(root.path().to_path_buf())).unwrap();
    assert!(scan.installed.is_empty());
    assert!(scan.opaque.is_empty(), "an absent manifest is not an unreadable one");
    assert!(scan.warnings.is_empty(), "and says nothing");
}
```

- [ ] **Step 5: Thread it through `plan()` and render it**

`plan`'s signature gains `opaque: &[Name]` after `installed`. In the declared
loop, before the lock check:

```rust
if opaque.iter().any(|o| o == name) {
    actions.push(Action::Skip {
        backend: SCOOP.into(),
        name: name.clone(),
        reason: SkipReason::Opaque,
    });
    continue;
}
```

The undeclared loop needs no change — it iterates `installed`, which an opaque
name is not in. That is the whole of `an_undeclared_package_..._is_not_a_stray`.

Add to `SkipReason`, and to `render.rs`'s match:

```rust
SkipReason::Opaque => {
    "installed, but its state could not be read -- see the warnings above".to_string()
}
```

Update every `plan(...)` call site: `src/main.rs`, `src/apply.rs`,
`tests/planner.rs`, `tests/adopt.rs`, `tests/prepare.rs`.

- [ ] **Step 6: Run the suite**

Run: `cargo test --no-fail-fast`
Expected: PASS, all targets.

- [ ] **Step 7: Negative controls, both directions**

1. Revert `plan()`'s opaque check → expect
   `a_declared_package_the_scan_could_not_read_is_skipped_rather_than_installed`
   to fire with `Action::Install` in place of `Action::Skip`.
2. Make `Scoop::scan` push onto `opaque` from the `NotFound` arm too → expect
   `a_half_finished_install_with_no_manifest_yet_is_silent_not_opaque` to fire
   on the `scan.opaque.is_empty()` assertion.

Record which assertion fired in each case.

- [ ] **Step 8: Commit**

```bash
git add src/backend/ src/plan.rs src/render.rs src/main.rs src/apply.rs tests/
git commit -m "Carry the names a scan could not read, and skip them instead of installing"
```

---

## Task 5: `plan()` becomes one pass per backend

**Files:**
- Modify: `src/plan.rs`
- Test: `tests/planner.rs`

**Interfaces:**
- Produces: `plan()` unchanged in signature, but the scoop-specific loops are
  replaced by a loop over a private `BackendView`. Task 6 adds winget's view.

- [ ] **Step 1: Write the failing test — the invariant, stated**

```rust
#[test]
fn two_installed_entries_for_one_package_do_not_produce_two_prunes() {
    // Measured: winget's `list` returns 7zip.7zip twice, with two different
    // versions. plan()'s declared loop takes the first with `.find()`; its
    // undeclared loop iterates all of them and would emit TWO Prune actions
    // for one package. The invariant "at most one Installed per (backend,
    // name)" has never been written down and is false for the second backend.
    let mut state = State::default();
    state.set(SCOOP, &Name::new("aichat"), Ownership::Adopted);
    let p = plan(
        &config::parse("[scoop]\npackages = []\n").unwrap(),
        &lock::parse("").unwrap(),
        &[installed("aichat", "0.30.0"), installed("aichat", "0.29.0")],
        &[],
        &state,
        &Running::default(),
    );
    let prunes = p.actions.iter().filter(|a| matches!(a, Action::Prune { .. })).count();
    assert_eq!(prunes, 1, "one package is one prune, got {:?}", p.actions);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test planner two_installed_entries -- --exact`
Expected: FAIL, `left: 2, right: 1`.

- [ ] **Step 3: Introduce the view and the invariant**

```rust
/// One backend's slice of the inputs. `plan()` runs the same pass over each.
struct BackendView<'a> {
    backend: &'static str,
    declared: &'a [Name],
    lock: &'a BTreeMap<Name, Pin>,
    /// `[scoop.opts]`. Empty for backends that have no per-package options.
    opts: &'a BTreeMap<Name, PkgOpts>,
    /// Names this backend installs for itself and does not record. Empty for
    /// winget, whose equivalent is not a fixed list but the sourceless rows,
    /// which never reach `installed` at all.
    helpers: &'static [&'static str],
}
```

At the top of the per-backend pass:

```rust
debug_assert!(
    {
        let mut seen = BTreeSet::new();
        installed.iter().filter(|i| i.backend == view.backend).all(|i| seen.insert(&i.name))
    },
    "a backend returned two Installed entries for one name; \
     Scan must collapse them or mark them opaque"
);
```

and make the undeclared loop deduplicate defensively so a release build cannot
double-prune:

```rust
let mut acted: BTreeSet<&Name> = BTreeSet::new();
for inst in installed.iter().filter(|i| i.backend == view.backend) {
    if !acted.insert(&inst.name) {
        continue;
    }
    // ... existing body
}
```

- [ ] **Step 4: Run the whole planner suite**

Run: `cargo test --no-fail-fast --test planner`
Expected: PASS, including every pre-existing test — this task must change no
scoop behaviour.

- [ ] **Step 5: Negative control**

Remove the `acted.insert` guard → expect `two_installed_entries_...` to fire
with `left: 2, right: 1`. Restore.

- [ ] **Step 6: Commit**

```bash
git add src/plan.rs tests/planner.rs
git commit -m "Run the planner once per backend, and state the one-Installed-per-name invariant"
```

---

## Task 6: `mass_prune_guard` grows a backend loop

`src/apply.rs:37`. The bug is the shape: `if !declared.scoop.packages.is_empty()
{ return Ok(()) }` returns from the whole function, so **any** declared scoop
package disables the check for every other backend.

**Files:**
- Modify: `src/apply.rs:37-48`
- Test: `src/apply.rs` unit tests and `tests/cli.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_config_that_declares_no_winget_packages_while_dotpkg_owns_some_is_refused() {
    let mut state = State::default();
    state.set(WINGET, &Name::new("Git.Git"), Ownership::Adopted);
    let cfg = config::parse("[winget]\npackages = []\n").unwrap();
    let r = mass_prune_guard(&cfg, &state);
    assert!(r.is_err(), "an emptied [winget] section must not prune silently");
    let msg = format!("{:#}", r.unwrap_err());
    assert!(msg.contains("winget"), "the message must name the backend: {msg}");
    assert!(msg.contains('1'), "and how many are owned: {msg}");
}

#[test]
fn a_declared_scoop_package_does_not_disable_the_winget_half_of_the_guard() {
    // THE bug. The old short-circuit returned from the whole function on the
    // first non-empty backend, so a pkg.toml with any scoop package at all
    // could drop its entire [winget] section and prune every owned winget
    // package with no guard.
    let mut state = State::default();
    state.set(WINGET, &Name::new("Git.Git"), Ownership::Adopted);
    let cfg = config::parse("[scoop]\npackages = [\"fzf\"]\n").unwrap();
    let r = mass_prune_guard(&cfg, &state);
    assert!(r.is_err(), "a non-empty [scoop] must not vouch for an empty [winget]");
}

#[test]
fn a_config_that_declares_packages_for_every_owned_backend_is_allowed() {
    // The positive sibling: without it, a guard that always refused would
    // satisfy both assertions above.
    let mut state = State::default();
    state.set(SCOOP, &Name::new("fzf"), Ownership::Installed);
    state.set(WINGET, &Name::new("Git.Git"), Ownership::Adopted);
    let cfg = config::parse(
        "[scoop]\npackages = [\"fzf\"]\n[winget]\npackages = [\"Git.Git\"]\n",
    ).unwrap();
    assert!(mass_prune_guard(&cfg, &state).is_ok());
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib mass_prune`
Expected: the first two FAIL (`Ok` where `Err` was required), the third passes.

- [ ] **Step 3: Replace the `return` with a `continue`**

```rust
pub fn mass_prune_guard(declared: &Config, state: &State) -> Result<()> {
    for (backend, declared_count) in [
        (SCOOP, declared.scoop.packages.len()),
        (WINGET, declared.winget.packages.len()),
    ] {
        // `continue`, not `return`: the old code returned from the whole
        // function on the first non-empty backend, so any declared scoop
        // package vouched for an emptied [winget] section.
        if declared_count > 0 {
            continue;
        }
        let owned = state.owned_count(backend);
        anyhow::ensure!(
            owned == 0,
            "pkg.toml declares no {backend} packages but dotpkg owns {owned}. \
             Refusing to prune everything. If the file is right, pass --allow-empty-config."
        );
    }
    Ok(())
}
```

- [ ] **Step 4: Run the suite**

Run: `cargo test --no-fail-fast`
Expected: PASS, including `tests/cli.rs`'s
`an_empty_config_is_refused_before_the_machine_is_even_scanned`, which covers
the scoop half and must stay green.

- [ ] **Step 5: Negative control**

Restore the `return Ok(())` short-circuit → expect
`a_declared_scoop_package_does_not_disable_the_winget_half_of_the_guard` to
fire on its `r.is_err()` assertion. Restore.

- [ ] **Step 6: Commit**

```bash
git add src/apply.rs
git commit -m "Check the mass-prune guard per backend instead of returning on the first"
```

---

## Task 7: `Resolution` carries a `Pin`, and `State::names` gets a caller or goes

**Files:**
- Modify: `src/update.rs` (`Resolution`, `resolve_into_lock`, `run`)
- Modify: `src/state.rs:112` (`names`)
- Test: `tests/update.rs`

**Interfaces:**
- Produces: `Resolution::Resolved { pin: Pin }`. Tasks 14–16 construct
  `Pin::WingetVersion` through it, and cannot construct a commit.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_winget_resolution_cannot_carry_a_commit() {
    // A type-level fix, in the spirit of WriteLock/WritePkgToml/WriteState:
    // the wrong program stops being a bug to be caught and becomes one that
    // cannot be written. This test documents the shape; the compiler enforces
    // it. `Resolution::Resolved { bucket, commit, version }` allowed a winget
    // pin to be built with a bucket and a commit; `Pin` does not.
    let r = Resolution::Resolved { pin: Pin::WingetVersion { version: "2.55.0".into() } };
    let Resolution::Resolved { pin } = r else { panic!("built above") };
    assert_eq!(pin.version(), "2.55.0");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test update a_winget_resolution -- --exact`
Expected: FAIL to compile — `Resolution::Resolved` has no field `pin`.

- [ ] **Step 3: Change the variant and every construction site**

```rust
pub enum Resolution {
    /// The pin this backend resolved. `Pin` is deliberately asymmetric, so a
    /// winget resolution carrying a commit is a compile error rather than a
    /// bug a test has to catch.
    Resolved { pin: Pin },
    /// Per package, never fatal to the run.
    Failed { why: String },
}
```

At `src/update.rs:321` the scoop construction becomes:

```rust
Resolution::Resolved {
    pin: Pin::ScoopCommit {
        // `key()`, not the display spelling -- `choose_bucket` folds the
        // directory it opens and `Scoop::stage` opens what the lock says
        // verbatim. (Winget is the opposite case and is handled in Task 14:
        // `--exact --id` is case-SENSITIVE, so the canonical spelling is the
        // only one that works there.)
        bucket: bucket_name.key().to_string(),
        commit: latest.commit,
        version: latest.version,
    },
}
```

`resolve_into_lock` reads `pin` directly instead of rebuilding a
`Pin::ScoopCommit` from three fields.

- [ ] **Step 4: Decide `State::names`**

It has zero callers. Give it one in `update::run`'s named-scope path — the
re-insert of no-longer-declared entries already needs "every name dotpkg owns
for this backend" — or delete it. **If you delete it, delete its doc comment
too and say so in the report**; a survivor whose cause is dead code is a
different fact from one whose cause is a missing test.

- [ ] **Step 5: Run the suite**

Run: `cargo test --no-fail-fast`
Expected: PASS. `winget_entries_survive_a_scoop_update_untouched` in
`tests/update.rs` must stay green — it is the positive sibling.

- [ ] **Step 6: Commit**

```bash
git add src/update.rs src/state.rs tests/update.rs
git commit -m "Make Resolution carry a Pin so a winget resolution cannot hold a commit"
```

---

## Task 8: `Scoop::stage` names the command that fixes a missing commit

Settled before the spec was written. When `git cat-file -e` fails, the message
must say `git -C <dir> fetch`, in the same shape `src/adopt.rs` already uses for
a shallow clone.

**Files:**
- Modify: `src/backend/scoop.rs` (`stage`'s `cat-file -e` failure arm)
- Test: `tests/prepare.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn a_commit_the_bucket_does_not_have_names_the_fetch_that_would_get_it() {
    // A lock committed on another machine names a commit this clone has never
    // fetched. Naming a cause without the command that fixes it is half a
    // message -- the same rule tests/adopt.rs already applies to shallowness.
    let (root, dir) = a_bucket_without_the_pinned_commit();   // existing helper
    let r = stage_one_pinned_at(&root, "fzf", "0".repeat(40).as_str());
    assert!(r.is_err(), "an absent commit must refuse");
    let msg = format!("{:#}", r.unwrap_err());
    assert!(msg.contains("git -C"), "must name the command: {msg}");
    assert!(msg.contains(&dir.display().to_string()), "and the directory: {msg}");
    assert!(msg.contains("fetch"), "and what to run: {msg}");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --test prepare a_commit_the_bucket_does_not_have -- --exact`
Expected: FAIL on `msg.contains("git -C")`.

- [ ] **Step 3: Extend the message**

Append to the existing `cat-file -e` failure context:
`` format!("Run `git -C {} fetch` if the commit was made on another machine.", dir.display()) ``

- [ ] **Step 4: Run and commit**

Run: `cargo test --no-fail-fast --test prepare`

```bash
git add src/backend/scoop.rs tests/prepare.rs
git commit -m "Name the fetch that would recover a commit this clone does not have"
```

---

## Task 9: `parse_list` — the winget table, against captured bytes

**Files:**
- Create: `src/backend/winget.rs`
- Modify: `src/backend/mod.rs` (`pub mod winget;`)
- Test: `tests/winget_scan.rs` (new)

**Interfaces:**
- Produces:

```rust
pub struct WingetRow {
    pub name: String,
    pub id: String,
    pub version: String,
    pub available: Option<String>,
    pub source: Option<String>,
}
pub fn parse_list(stdout: &str) -> anyhow::Result<Vec<WingetRow>>;
```

Task 10 consumes `Vec<WingetRow>`.

- [ ] **Step 1: Write the failing tests against the fixtures**

`tests/winget_scan.rs`. **Read `tests/fixtures/winget/PROVENANCE.md` first** —
every number below is measured, not invented.

```rust
use dotpkg::backend::winget::{parse_list, WingetRow};

fn fixture(name: &str) -> String {
    // include_str! keeps the CRLF the fixture was captured with. Reading with
    // std::fs would too, but include_str! also fails the BUILD if a fixture is
    // renamed, which a runtime read would only fail at test time.
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/winget")
            .join(name),
    )
    .unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

#[test]
fn the_full_table_parses_to_every_row_winget_printed() {
    let rows = parse_list(&fixture("list-full.txt")).unwrap();
    assert_eq!(rows.len(), 141, "141 rows were captured");
    let ids: std::collections::BTreeSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids.len(), 126, "126 of them are distinct");
    assert_eq!(
        rows.iter().filter(|r| r.source.is_none()).count(),
        84,
        "84 rows have no Source and cannot be compared against any index"
    );
}

#[test]
fn a_table_with_no_available_column_still_parses() {
    // The column SET is data-dependent: `Available` is absent whenever no row
    // has an upgrade. A parser keyed on column count instead of on header
    // names reads Source out of the Available slot and reports every package
    // as sourceless.
    let rows = parse_list(&fixture("list-duplicate-id.txt")).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.available.is_none()));
    assert!(rows.iter().all(|r| r.source.as_deref() == Some("winget")),
            "Source must not be read out of the missing Available column: {rows:?}");
}

#[test]
fn one_id_can_appear_twice_with_two_different_versions() {
    let rows = parse_list(&fixture("list-duplicate-id.txt")).unwrap();
    let versions: Vec<&str> = rows.iter().map(|r| r.version.as_str()).collect();
    assert_eq!(rows[0].id, "7zip.7zip");
    assert_eq!(rows[1].id, "7zip.7zip");
    assert_eq!(versions, vec!["26.01.00.0", "26.02"]);
}

#[test]
fn a_version_winget_will_not_commit_to_is_kept_verbatim() {
    // "> 17.14.37" is winget saying *at least*: one machine-scoped install
    // whose exact version it cannot determine. Kept as written here; Task 10
    // is what refuses to treat it as a version.
    let rows = parse_list(&fixture("list-greater-prefix.txt")).unwrap();
    assert_eq!(rows.len(), 1, "ONE row -- not several installs");
    assert_eq!(rows[0].version, "> 17.14.37");
}

#[test]
fn the_available_column_is_read_when_it_is_there() {
    let rows = parse_list(&fixture("list-upgrade-available.txt")).unwrap();
    let chrome = rows.iter().find(|r| r.id == "Google.Chrome").expect("in the fixture");
    assert_eq!(chrome.version, "150.0.7871.187");
    assert_eq!(chrome.available.as_deref(), Some("151.0.7922.109"));
}

#[test]
fn a_not_found_message_is_not_a_table_and_is_not_silently_empty() {
    // list-not-found.txt and list-source-filter-empty.txt are BYTE-IDENTICAL
    // and came back with different exit codes. So the parser may not decide
    // "found nothing" from the text -- it must say "this is not a table" and
    // let the caller read the exit code.
    let r = parse_list(&fixture("list-not-found.txt"));
    assert!(r.is_err(), "no header row means the parser must refuse");
    let msg = format!("{:#}", r.unwrap_err());
    assert!(msg.contains("header"), "and say why: {msg}");
}

#[test]
fn a_header_that_is_not_the_shape_this_parser_measured_is_refused() {
    // The header is English and therefore locale-dependent. Guessing offsets
    // on an unrecognised header reports an empty machine -- and an empty
    // machine is what mass_prune_guard exists to catch far too late.
    let r = parse_list("Nom  Identifiant  Version\r\n----\r\nx  y  z\r\n");
    assert!(r.is_err(), "an unrecognised header must refuse, not guess");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --no-fail-fast --test winget_scan`
Expected: FAIL to compile — `dotpkg::backend::winget` does not exist.

- [ ] **Step 3: Implement `parse_list`**

Rules, all measured:
- Find the header line: the first line that starts with `Name` and contains
  ` Id`. If there is none, `bail!("winget list produced no header row: ...")`
  with the first 120 characters of the input.
- Column starts come from `find` on the header, **searched left to right from
  the previous column's end**, for `["Name", "Id", "Version", "Available",
  "Source"]`. A name that is not found is simply absent from the layout.
- `Name`, `Id` and `Version` are required; refuse if any is missing.
- Skip the `---` rule line and any blank line.
- A field is `line[start..next_start]`, trimmed; if `start >= line.len()`, the
  field is empty. `available` and `source` are `None` when empty.
- Stop at the first line that is not part of a table — `list-upgrade-available.txt`
  has a `9 upgrades available.` line and then a **second** table. Parse the
  first table only, and record in a comment that the trailing count line
  disagrees with the first table's row count by design.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test --no-fail-fast --test winget_scan`
Expected: PASS, all eight.

- [ ] **Step 5: Negative controls**

1. Key the layout on column **count** rather than header names → expect
   `a_table_with_no_available_column_still_parses` to fire on the `source`
   assertion.
2. Make the missing-header case return `Ok(vec![])` → expect
   `a_not_found_message_is_not_a_table_and_is_not_silently_empty` to fire on
   `r.is_err()`.
3. Split on `'\n'` and forget to strip `'\r'` → expect
   `a_table_with_no_available_column_still_parses` or
   `the_full_table_parses_...` to fire on a trailing-`\r` mismatch in `source`.
   **If it does not fire, that is a plan failure: add the assertion that
   catches it and say so.**

- [ ] **Step 6: Commit**

```bash
git add src/backend/winget.rs src/backend/mod.rs tests/winget_scan.rs
git commit -m "Parse winget's list table against bytes captured from a real winget"
```

---

## Task 10: `rows_to_scan` — duplicates, sourceless rows and unusable versions

**Files:**
- Modify: `src/backend/winget.rs`
- Test: `tests/winget_scan.rs`

**Interfaces:**
- Produces: `pub fn rows_to_scan(rows: Vec<WingetRow>) -> Scan`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_whole_captured_machine_splits_into_exactly_these_counts() {
    // Computed from tests/fixtures/winget/list-full.txt, not estimated.
    // 141 rows -> 126 distinct ids -> 89 opaque + 37 installed.
    //
    //   84  ids with no Source        -- installed, comparable against nothing
    //    2  ids whose version is "> " -- Microsoft.VisualStudio.2022.BuildTools,
    //                                    Microsoft.WindowsAppRuntime.1.8
    //    3  ids whose duplicate rows disagree on a version --
    //                                    7zip.7zip, Microsoft.UI.Xaml.2.8,
    //                                    Microsoft.WindowsAppRuntime.2
    //   ---
    //   89  opaque        37 installed, 4 of them collapsed from duplicate rows
    //
    // 89 + 37 = 126 is the cross-check. If these numbers disagree with the
    // fixture, THE FIXTURE IS RIGHT and this comment is wrong: recompute,
    // fix the numbers, and say so in the report.
    let scan = rows_to_scan(parse_list(&fixture("list-full.txt")).unwrap());
    assert_eq!(scan.opaque.len(), 89);
    assert_eq!(scan.installed.len(), 37);
    assert_eq!(scan.opaque.len() + scan.installed.len(), 126, "every id is one or the other");
    assert!(scan.installed.iter().all(|i| i.backend == dotpkg::model::WINGET));
    assert!(
        !scan.installed.iter().any(|i| i.name.key().starts_with("msix\\")
                                    || i.name.key().starts_with("arp\\")),
        "no MSIX or ARP row may reach `installed`"
    );
}

#[test]
fn duplicate_ids_that_agree_on_a_version_collapse_to_one_entry_and_warn() {
    let rows = vec![row("WindowsAppRuntime.1.7", "1.7.9"), row("WindowsAppRuntime.1.7", "1.7.9")];
    let scan = rows_to_scan(rows);
    assert_eq!(scan.installed.len(), 1, "one package is one entry");
    assert_eq!(scan.installed[0].version, "1.7.9");
    assert_eq!(scan.warnings.len(), 1, "winget's export collapses these silently; dotpkg may not");
    assert!(scan.warnings[0].contains("WindowsAppRuntime.1.7"));
}

#[test]
fn duplicate_ids_that_disagree_on_a_version_are_opaque_rather_than_guessed() {
    // 7zip.7zip is installed twice, at 26.01.00.0 and 26.02. Two versions of
    // one package is a state dotpkg has no vocabulary for; picking one would
    // be inventing a fact. winget's own export picks 26.02 and says nothing.
    let scan = rows_to_scan(vec![row("7zip.7zip", "26.01.00.0"), row("7zip.7zip", "26.02")]);
    assert!(scan.installed.is_empty(), "got {:?}", scan.installed);
    assert_eq!(scan.opaque, vec![Name::new("7zip.7zip")]);
    assert!(scan.warnings[0].contains("26.01.00.0") && scan.warnings[0].contains("26.02"),
            "both versions must be named: {:?}", scan.warnings);
}

#[test]
fn a_greater_than_version_is_opaque_because_it_is_not_a_version() {
    // Left in `installed`, `cur.version == want` is false forever and
    // is_older() picks Downgrade, so status prints a false down-arrow on
    // every run and apply --yes acts on it.
    let scan = rows_to_scan(parse_list(&fixture("list-greater-prefix.txt")).unwrap());
    assert!(scan.installed.is_empty());
    assert_eq!(scan.opaque, vec![Name::new("Microsoft.VisualStudio.2022.BuildTools")]);
}

#[test]
fn an_ordinary_single_row_becomes_an_ordinary_installed_entry() {
    // The positive sibling. Without it, a rows_to_scan that marked EVERYTHING
    // opaque would satisfy all four assertions above.
    let scan = rows_to_scan(parse_list(&fixture("list-single.txt")).unwrap());
    assert!(scan.opaque.is_empty());
    assert_eq!(scan.installed.len(), 1);
    assert_eq!(scan.installed[0].name, Name::new("ajeetdsouza.zoxide"));
    assert_eq!(scan.installed[0].version, "0.10.0");
    assert_eq!(scan.installed[0].arch, None, "winget does not expose an architecture");
    assert_eq!(scan.installed[0].bucket, None);
    assert!(scan.installed[0].bins.is_empty(), "and names no executables");
}
```

Add the `row` helper. The counts above were computed from the fixture and
cross-check (89 + 37 = 126); if the implementation disagrees with them,
**the fixture is right** — recompute before changing an assertion.

- [ ] **Step 2–4: Run, implement, run**

Implement: group rows by `Name`, then per group — any row whose `source` is
`None` or whose `version` starts with `"> "` makes the whole group opaque; a
group whose rows disagree on `version` is opaque with a warning naming every
version; otherwise one `Installed` (and a warning if the group had more than
one row).

`bins` is empty and `arch`/`bucket` are `None`. Add the doc comment recording
that **`Running::covers` is therefore weaker for winget than for scoop** — the
`bins` half cannot fire — and that nothing depends on it while dotpkg does not
act on winget packages.

- [ ] **Step 5: Negative control**

Make the disagreeing-duplicates case keep the greatest version (what
`winget export` does) → expect
`duplicate_ids_that_disagree_on_a_version_are_opaque_rather_than_guessed` to
fire on `scan.installed.is_empty()`.

- [ ] **Step 6: Commit**

```bash
git add src/backend/winget.rs tests/winget_scan.rs
git commit -m "Refuse to invent a version winget would not commit to"
```

---

## Task 11: The `WingetCmd` seam and `Winget::scan`

**Files:**
- Modify: `src/backend/winget.rs`
- Test: `tests/winget_scan.rs`

**Interfaces:**
- Produces:

```rust
pub struct CmdOut { pub code: i32, pub stdout: String }
pub trait WingetCmd { fn run(&self, args: &[&str]) -> anyhow::Result<CmdOut>; }
pub struct RealWinget;                       // spawns winget.exe
pub struct Winget<C: WingetCmd> { cmd: C }
pub const NO_APPLICATIONS_FOUND: i32 = -1978335212;
pub const NO_VERSION_FOUND: i32 = -1978335209;
```

Tasks 12–16 use `Winget<C>`; tests use a recording fake, never `RealWinget`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn scan_asks_winget_exactly_once_with_the_argv_this_phase_measured() {
    // The exit code is a function of the FILTER, not of the output:
    // `list -s msstore` returns the same 53-byte sentence as
    // `list -e --id <absent>` and exits 0 where the other exits 0x8A150014.
    // So the argv is part of the contract and is pinned here.
    let fake = FakeWinget::returning(0, fixture("list-single.txt"));
    let scan = Backend::scan(&Winget::new(fake.clone())).unwrap();
    assert_eq!(fake.calls(), vec![vec!["list", "--disable-interactivity"]]);
    assert_eq!(scan.installed.len(), 1);
}

#[test]
fn a_machine_without_winget_is_an_empty_scan_and_a_warning_not_an_error() {
    // Symmetric with Scoop::scan, where a missing ~/scoop/apps is a valid
    // empty state rather than a failure.
    let fake = FakeWinget::failing_to_spawn();
    let scan = Backend::scan(&Winget::new(fake)).unwrap();
    assert!(scan.installed.is_empty() && scan.opaque.is_empty());
    assert_eq!(scan.warnings.len(), 1);
    assert!(scan.warnings[0].contains("winget"), "got {:?}", scan.warnings);
}

#[test]
fn a_nonzero_exit_from_list_is_an_error_not_an_empty_machine() {
    // An empty machine is exactly what mass_prune_guard exists to catch too
    // late. A `list` that fails must never look like "nothing is installed".
    let fake = FakeWinget::returning(NO_APPLICATIONS_FOUND, fixture("list-not-found.txt"));
    let r = Backend::scan(&Winget::new(fake));
    assert!(r.is_err(), "a failed list must not read as an empty machine");
}

#[test]
fn the_backend_reports_the_name_the_lock_and_state_are_keyed_by() {
    assert_eq!(Backend::name(&Winget::new(FakeWinget::returning(0, String::new()))),
               dotpkg::model::WINGET);
}
```

- [ ] **Step 2–4: Run, implement, run**

`RealWinget::run` uses `std::process::Command::new("winget")` with
`.stdout(Stdio::piped()).stderr(Stdio::piped())`, reads stdout as UTF-8 lossily,
and returns `code` from `status.code().unwrap_or(-1)`. **stderr is captured and
discarded with a comment**: it was 0 bytes in all ~45 measured invocations,
including every failure, so anything on it is a surprise worth not silently
merging into stdout.

`Winget::scan` runs `["list", "--disable-interactivity"]`; on a spawn error it
returns the empty-scan-plus-warning; on `code != 0` it errors; otherwise
`rows_to_scan(parse_list(&out.stdout)?)`.

- [ ] **Step 5: Negative control**

Make `scan` return `Ok(Scan::default())` on `code != 0` → expect
`a_nonzero_exit_from_list_is_an_error_not_an_empty_machine` to fire on
`r.is_err()`.

- [ ] **Step 6: Commit**

```bash
git add src/backend/winget.rs tests/winget_scan.rs
git commit -m "Put winget behind a seam so no test spawns it"
```

---

## Task 12: `parse_show` and the canonical-id rule

**Files:**
- Modify: `src/backend/winget.rs`
- Test: `tests/winget_resolve.rs` (new)

**Interfaces:**
- Produces:

```rust
pub struct Found { pub id: String, pub version: String }
pub fn parse_show(stdout: &str) -> anyhow::Result<Found>;
pub fn parse_versions(stdout: &str) -> anyhow::Result<(String, Vec<String>)>;
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn show_yields_the_canonical_id_even_when_asked_in_the_wrong_case() {
    // MEASURED: `--exact` is what makes `--id` case-sensitive.
    //   show -e --id git.git  -> 0x8A150014, "No package found"
    //   show    --id git.git  -> 0,          "Found Git [Git.Git]"
    // src/model.rs's "scoop and winget both resolve names case-insensitively"
    // is false for --exact, and Name folds case -- so dotpkg can hold a name
    // that compares equal to the right package and is unusable against winget.
    // Asking without --exact and recording what came back is the fix.
    let f = parse_show(&fixture("show-canonical-echo.txt")).unwrap();
    assert_eq!(f.id, "Git.Git", "the canonical spelling, not the one we asked with");
    assert_eq!(f.version, "2.55.0.3");
}

#[test]
fn show_of_the_canonical_spelling_gives_the_same_answer() {
    // The positive sibling: both fixtures are 1550 bytes for a reason.
    let a = parse_show(&fixture("show-git.txt")).unwrap();
    let b = parse_show(&fixture("show-canonical-echo.txt")).unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(a.version, b.version);
}

#[test]
fn a_not_found_body_is_refused_rather_than_parsed_into_an_empty_found() {
    let r = parse_show(&fixture("show-package-gone.txt"));
    assert!(r.is_err(), "an empty Found would be a package named \"\" at version \"\"");
}

#[test]
fn versions_come_back_newest_first_and_the_retention_depth_is_countable() {
    // ripgrep.MSVC keeps 8; zoxide keeps 11; OhMyPosh keeps 828. Retention is
    // a publisher policy, not a winget guarantee, so `update` can say how deep
    // the index is when a pin falls off the end.
    let (id, vs) = parse_versions(&fixture("show-versions-ripgrep.txt")).unwrap();
    assert_eq!(id, "BurntSushi.ripgrep.MSVC");
    assert_eq!(vs.len(), 8);
    assert_eq!(vs[0], "15.2.0", "row 1 is what `show` calls Version:");

    let (_, zs) = parse_versions(&fixture("show-versions-zoxide.txt")).unwrap();
    assert_eq!(zs.len(), 11);
    assert_eq!(zs.first().map(String::as_str), Some("0.10.0"));
    assert_eq!(zs.last().map(String::as_str), Some("0.9.0"));
}

#[test]
fn show_and_show_versions_agree_on_the_newest() {
    // Measured on 6 of 6 packages. If this ever fails, `resolve_latest` is
    // reading the wrong line and the lock will pin something nobody chose.
    let f = parse_show(&fixture("show-git.txt")).unwrap();
    let (_, vs) = parse_versions(&fixture("show-versions-zoxide.txt")).unwrap();
    assert_eq!(f.version, "2.55.0.3");
    assert_eq!(parse_show(&fixture("show-old-version.txt")).unwrap().version, "0.9.0",
               "show -v pins the version that was ASKED for, not the newest");
    assert_eq!(vs[0], "0.10.0");
}
```

- [ ] **Step 2–4: Run, implement, run**

`parse_show`: find the first line matching `Found <name> [<id>]` — take the id
between the last `[` and the trailing `]`; find the first line starting
`Version:` and take the rest, trimmed. Refuse, naming the first 120 characters,
if either is absent.

`parse_versions`: the `Found …[id]` line, then skip the `Version` header and its
`---` rule, then every non-blank line trimmed, in order.

- [ ] **Step 5: Negative control**

Make `parse_show` read the id from the `Found <name>` part instead of the
brackets → expect `show_yields_the_canonical_id_even_when_asked_in_the_wrong_case`
to fire with `left: "Git", right: "Git.Git"`.

- [ ] **Step 6: Commit**

```bash
git add src/backend/winget.rs tests/winget_resolve.rs
git commit -m "Read winget's canonical id back rather than trusting the spelling we asked with"
```

---

## Task 13: `Winget::resolve_latest` and `resolve_installed`

**Files:**
- Modify: `src/backend/mod.rs` (trait gains the two methods), `src/backend/winget.rs`,
  `src/backend/scoop.rs` (moves its existing resolvers onto the trait)
- Test: `tests/winget_resolve.rs`

**Interfaces:**
- Produces, on `Backend`:

```rust
fn resolve_latest(&self, name: &Name, ctx: &ResolveCtx) -> Resolution;
fn resolve_installed(&self, inst: &Installed, ctx: &ResolveCtx) -> Resolution;
```

`ResolveCtx` carries `{ offline: bool, declared: &Config, scoop_root: &Path }`
— whatever `bucket::choose_bucket` and `bucket::resolve_latest` already need,
moved rather than redesigned. **`update::run` must no longer name
`bucket::resolve_latest`**; that is the test of whether this task worked.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn resolving_latest_asks_without_exact_and_pins_what_came_back() {
    let fake = FakeWinget::returning(0, fixture("show-canonical-echo.txt"));
    let w = Winget::new(fake.clone());
    let r = w.resolve_latest(&Name::new("git.git"), &ResolveCtx::offline());
    assert_eq!(fake.calls(), vec![vec!["show", "--id", "git.git", "--disable-interactivity"]],
               "no --exact: it is case-sensitive and would refuse this spelling");
    let Resolution::Resolved { pin } = r else { panic!("got {r:?}") };
    assert_eq!(pin, Pin::WingetVersion { version: "2.55.0.3".into() });
}

#[test]
fn a_pin_whose_version_left_the_index_is_refused_and_says_how_deep_the_index_is() {
    let fake = FakeWinget::script(vec![
        (NO_VERSION_FOUND, fixture("show-version-gone.txt")),
        (0, fixture("show-versions-zoxide.txt")),
    ]);
    let w = Winget::new(fake);
    let inst = installed_winget("ajeetdsouza.zoxide", "0.8.0");
    let Resolution::Failed { why } = w.resolve_installed(&inst, &ResolveCtx::offline())
        else { panic!("0.8.0 is not in the index") };
    assert!(why.contains("0.8.0"), "name the version: {why}");
    assert!(why.contains("11"), "and how many the publisher keeps: {why}");
}

#[test]
fn a_package_that_left_the_index_entirely_is_a_different_message() {
    // 0x8A150014 and 0x8A150017 are distinct codes for distinct facts.
    let fake = FakeWinget::returning(NO_APPLICATIONS_FOUND, fixture("show-package-gone.txt"));
    let w = Winget::new(fake);
    let Resolution::Failed { why } = w.resolve_latest(&Name::new("Xyzzy.NoSuch"), &ResolveCtx::offline())
        else { panic!("absent package") };
    assert!(why.contains("no longer") || why.contains("not in"), "got {why}");
    assert!(!why.contains("version"), "this is not a version problem: {why}");
}

#[test]
fn an_opaque_package_is_refused_by_adopt_rather_than_pinned() {
    // rows_to_scan never puts an opaque name into `installed`, so
    // resolve_installed cannot be reached for one through scan -- but it is
    // public, and a caller with a hand-built Installed must still be refused.
    let w = Winget::new(FakeWinget::returning(0, fixture("show-git.txt")));
    let inst = installed_winget("Microsoft.VisualStudio.2022.BuildTools", "> 17.14.37");
    let Resolution::Failed { why } = w.resolve_installed(&inst, &ResolveCtx::offline())
        else { panic!("a version dotpkg cannot vouch for must not be pinned") };
    assert!(why.contains("> 17.14.37"), "got {why}");
}
```

- [ ] **Step 2–4: Run, implement, run**

`resolve_latest`: `["show", "--id", <display spelling>, "--disable-interactivity"]`;
`code == NO_APPLICATIONS_FOUND` → `Failed` naming the package; `code != 0` →
`Failed` with the first stdout line; else `parse_show` → `Pin::WingetVersion`.

`resolve_installed`: refuse a version starting `"> "` before spawning anything;
otherwise `["show", "--id", …, "-v", <version>, "--disable-interactivity"]`;
`NO_VERSION_FOUND` → a second call to `--versions` for the depth, then `Failed`;
`NO_APPLICATIONS_FOUND` → the package-level message; `0` → `Pin::WingetVersion`
with the **canonical** id recorded by the caller.

Move `Scoop`'s resolvers onto the trait unchanged — `update` keeps `git log -1`,
`adopt` keeps `--full-history`. **Do not unify them**: Phase 3 measured that the
flags must differ and each has a measurement justifying it.

- [ ] **Step 5: Negative control**

Add `--exact` to `resolve_latest`'s argv → expect
`resolving_latest_asks_without_exact_and_pins_what_came_back` to fire on the
`fake.calls()` assertion.

- [ ] **Step 6: Verify the seam actually moved**

Run: `grep -n 'bucket::resolve_latest' src/update.rs`
Expected: **no output.** If there is output, the trait is decoration.

- [ ] **Step 7: Commit**

```bash
git add src/backend/ src/update.rs tests/winget_resolve.rs
git commit -m "Move resolve onto the Backend trait and add winget's two resolvers"
```

---

## Task 14: `SkipReason::ReportedOnly`, and the exit code that follows

**Files:**
- Modify: `src/plan.rs`, `src/render.rs`, `src/main.rs`, `src/apply.rs`
- Test: `tests/planner.rs`, `src/render.rs` tests, `tests/cli.rs`

**Interfaces:**
- Produces: `SkipReason::ReportedOnly(Divergence)`, `Divergence`, and
  `floor_exit_code(code, preparation_ok, has_running_skips, has_reported_only)`.
- `SkipReason::BackendNotImplemented` is **deleted** — 13 sites across `src/`
  and `tests/`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_winget_package_that_differs_from_the_lock_is_reported_with_its_diff() {
    let p = plan(
        &config::parse("[winget]\npackages = [\"Brave.Brave\"]\n").unwrap(),
        &lock::parse("[winget.\"Brave.Brave\"]\nversion = \"151.1.93.134\"\npin = \"version-only\"\n").unwrap(),
        &[installed_winget("Brave.Brave", "151.1.93.132")],
        &[],
        &State::default(),
        &Running::default(),
    );
    assert_eq!(
        p.actions,
        vec![Action::Skip {
            backend: WINGET.into(),
            name: "Brave.Brave".into(),
            reason: SkipReason::ReportedOnly(Divergence::Change {
                from: "151.1.93.132".into(),
                to: "151.1.93.134".into(),
            }),
        }]
    );
}

#[test]
fn a_reported_only_package_is_not_counted_as_a_change() {
    // change_count() prints "N changes, M skipped. Continue?" -- the one line
    // the user reads before saying yes. Counting a change that will never
    // happen puts a false number in it, which is the defect class Phase 3
    // fixed twice in render.rs.
    let p = /* the plan above */;
    assert_eq!(p.change_count(), 0, "nothing in this plan will be done");
    assert_eq!(p.skip_count(), 1);
}

#[test]
fn an_owned_undeclared_winget_package_is_reported_not_pruned() {
    let mut state = State::default();
    state.set(WINGET, &Name::new("OpenAI.Codex"), Ownership::Adopted);
    let p = plan(
        &config::parse("[winget]\npackages = []\n[scoop]\npackages = [\"fzf\"]\n").unwrap(),
        &lock::parse("").unwrap(),
        &[installed_winget("OpenAI.Codex", "0.145.0")],
        &[],
        &state,
        &Running::default(),
    );
    assert!(!p.actions.iter().any(|a| matches!(a, Action::Prune { .. })),
            "dotpkg cannot uninstall a winget package in this phase: {:?}", p.actions);
    assert!(p.actions.iter().any(|a| matches!(a, Action::Skip {
        reason: SkipReason::ReportedOnly(Divergence::Prune { .. }), .. })));
}

#[test]
fn outstanding_reported_only_work_floors_the_exit_code_to_one() {
    // Same rule already applied to running skips: work the user asked for and
    // did not get must not report success to a scheduled task.
    assert_eq!(floor_exit_code(0, true, false, true), 1);
    assert_eq!(floor_exit_code(0, true, false, false), 0, "the positive sibling");
}
```

Plus, in `src/render.rs`'s tests, a **paired** assertion — one that the
`reported only` line is printed with both versions in it, and one that it is
**absent** from a plan with no winget divergence.

- [ ] **Step 2–4: Run, implement, run**

Add the variants; delete `BackendNotImplemented` and its 13 sites; give the
winget view `Capability::ReportsOnly` so the per-backend pass turns what would
have been `Install`/`Upgrade`/`Downgrade`/`Prune` into
`Skip { ReportedOnly(...) }`. Extend `floor_exit_code` with the fourth
parameter and update its Task 3 tests to the new arity.

`apply::prepare` must produce `Outcome::Skipped` for a `ReportedOnly`, never a
`Step` — `src/apply.rs:156` is where `BackendNotImplemented` was handled and is
where the replacement goes.

- [ ] **Step 5: Negative controls**

1. Count `ReportedOnly` in `change_count()` → expect
   `a_reported_only_package_is_not_counted_as_a_change` to fire on
   `p.change_count() == 0`.
2. Give the winget view `Capability::Acts` → expect
   `an_owned_undeclared_winget_package_is_reported_not_pruned` to fire on the
   `Action::Prune` assertion.
3. Drop the fourth term from `floor_exit_code` → expect
   `outstanding_reported_only_work_floors_the_exit_code_to_one` to fire,
   `left: 0, right: 1`.

- [ ] **Step 6: Commit**

```bash
git add src/ tests/
git commit -m "Report what dotpkg will not do to winget, with the diff and an honest count"
```

---

## Task 15: `update` and `adopt` over both backends

**Files:**
- Modify: `src/update.rs`, `src/adopt.rs`, `src/main.rs`, `src/apply.rs`
  (`entry_coherence`)
- Test: `tests/update.rs`, `tests/adopt.rs`, `tests/cli.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn update_resolves_winget_packages_instead_of_warning_that_it_cannot() {
    // src/update.rs:350's warning ("N winget package(s) were not resolved")
    // is what this phase replaces. Its absence is asserted, not assumed.
    let u = /* update::run over a config with one winget package, fake backend */;
    assert!(!u.warnings.iter().any(|w| w.contains("lands in phase 4")),
            "the phase-4 warning must be gone: {:?}", u.warnings);
    assert_eq!(u.lock.winget.len(), 1);
}

#[test]
fn a_winget_lock_entry_is_written_with_the_canonical_id_not_the_declared_case() {
    let u = /* declared as "git.git", winget answers Found Git [Git.Git] */;
    assert!(u.lock.winget.keys().any(|k| k.to_string() == "Git.Git"),
            "the lock records what winget matched: {:?}", u.lock.winget.keys().collect::<Vec<_>>());
}

#[test]
fn an_incoherent_winget_entry_is_named_by_the_same_guard_as_a_scoop_one() {
    // apply::incoherent_entries iterates lock.scoop only, so a winget pin has
    // never been checked. Once they are real, an empty version or a path
    // separator in an id must be caught before pkg.lock is written.
    let lock = lock::parse("[winget.\"Git.Git\"]\nversion = \"\"\npin = \"version-only\"\n").unwrap();
    let bad = apply::incoherent_entries(&lock);
    assert_eq!(bad.len(), 1, "an empty version must be refused");
    assert_eq!(bad[0].0.to_string(), "Git.Git");
}
```

- [ ] **Step 2–4: Run, implement, run**

`update::run` loops backends; `resolve_into_lock` writes both maps. Delete the
`src/update.rs:350` warning **and** its test's absence counterweight becomes the
assertion above. `adopt::run` takes a backend name and dispatches.

`update` runs `winget source update --name winget` unless `--offline` —
**conditional on Task 1's answer.** If Task 1 found that the scoped update
installs something, do **not** run it: warn that winget's index was not
refreshed and that `latest` means whatever this machine last pulled, exactly as
the scoop no-upstream path already does.

`entry_coherence` grows a `Pin::WingetVersion` arm: non-empty version, and
`ensure_plain_component` over the version and the name.

- [ ] **Step 5: Negative control**

Revert `entry_coherence` to bail on every non-`ScoopCommit` pin → expect
`an_incoherent_winget_entry_is_named_by_the_same_guard_as_a_scoop_one` to fire
with a length of 1 for the wrong reason; **check the reason string, not just the
count**, and if the control cannot discriminate, strengthen the assertion and
say so.

- [ ] **Step 6: Commit**

```bash
git add src/ tests/
git commit -m "Resolve, lock and adopt winget packages through the generalised pipeline"
```

---

## Task 16: The Windows run, the mutation run, and the notes

- [ ] **Step 1: Full suite on macOS**

Run: `cargo test --no-fail-fast && cargo fmt --check && cargo clippy -- -D warnings`

- [ ] **Step 2: Full suite on Windows, on the tree that ships**

Tarball `Cargo.toml`, `Cargo.lock`, `src/`, `tests/` (**including
`tests/fixtures/`** — a missing fixture directory makes `include`-style reads
panic, and a fixture stripped of CRLF makes the parser tests pass for the wrong
reason). Build in `C:\Users\kln\dotpkg-build`, `cargo test --no-fail-fast`.

Compare **target by target and name by name**, not by subtracting totals. The
expected difference from macOS is exactly the two `#[cfg(unix)]` tests
(`tests/adopt.rs`'s `a_failed_last_write_leaves_a_prefix_that_plan_does_nothing_about`
and `tests/scoop_scan.rs`'s `a_root_reached_through_a_symlink_still_matches_running_processes`).
**Nothing may be changed to make Windows pass.**

- [ ] **Step 3: Mutation run**

Run: `cargo mutants --no-shuffle -- --no-fail-fast`, unloaded machine, `-j 3`,
`--timeout 120`. Phase 3's run was contaminated by CPU contention that turned
55 results into TIMEOUT; do not repeat it. Account for **every** survivor as
closed, accepted-with-a-reason, or deferred-with-a-reason.

- [ ] **Step 4: Dogfood on a14**

The seven questions in the spec's Dogfood section. Read-only for winget:
no `install`, `uninstall`, `upgrade` or `pin add`. Write
`docs/dogfood-phase4-2026-08-XX.md`.

- [ ] **Step 5: Write `docs/phase4-notes.md`**

Carry forward what Phase 4b needs, in the shape of `docs/phase3-notes.md`:
every item produced by mutation, by a control that actually fired, by the
Windows run, or by a reviewer reproducing something — and where an item is
reasoned-only, **say so**.

Start it with the two things already known to be carried:
`Running::covers` is weaker for winget (no `bins`), and the winget executor is
blocked on a mutation measurement that has no throwaway root.

- [ ] **Step 6: Commit**

```bash
git add docs/ src/ tests/
git commit -m "Record what Phase 4 measured, and what Phase 4b inherits"
```

---

## Self-Review

**Spec coverage.** A1→Task 2, A2→Task 4, A3→Task 3 (+14), A4→Task 7 step 4,
A5→Tasks 5 and 14, A6→Task 6, A7→Tasks 7 and 13, A8→Task 8, B1→Tasks 9–11,
B2→Tasks 12–13, B3→Tasks 1 and 15, B4→Task 15. The spec's "What the user sees"
is Task 14; its Testing table is distributed across the tasks that own each
producer; its Dogfood section is Task 16.

**Known gap, stated rather than hidden.** The spec's `Capability` enum is
introduced in Task 14 but the `BackendView` it belongs to is introduced in
Task 5. Task 5's view therefore has no `cap` field and Task 14 adds it. That is
deliberate — Task 5 must change no behaviour, and a capability field with one
possible value would be untestable there — but it means **Task 5 and Task 14
edit the same struct** and should not be run in parallel.

**Open question the spec did not settle, and this plan does not either.**
`Scan::opaque` collapses three causes (an unreadable scoop manifest, a
sourceless winget row, an unusable `> ` version) into one planner outcome. If
the dogfood shows a user cannot tell them apart from the warnings alone,
splitting `SkipReason::Opaque` into named causes is a Phase 4b change, not a
mid-flight one — the planner consequence is identical in all three cases and
that is what `opaque` encodes.
