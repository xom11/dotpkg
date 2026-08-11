# dotpkg Phase 5 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make three things `status` and the running-process fence already claim
become true: give the fence a path signal and a user-declared name list for
winget, stop `Unmanaged` flooding 36 lines, and put the one measured winget
transient's retry on the one path it was measured on.

**Architecture:** Three independent halves against one spec. Half A adds a
`Running.dirs` producer for winget (a pure prefix test over process executable
paths) plus a `[winget.guard]` table merged into `Installed.bins` at one point,
so `covers`, `covers_name` and `covers_any` all improve without `Step` changing.
Half B collapses `Action::Unmanaged` at render time only — `Plan` keeps every
fact. Half C adds one retry to `Winget::update_source` keyed on a measured
signature, and adds no retry anywhere else.

**Tech Stack:** Rust 2021, `anyhow`, `serde`/`toml`, `sysinfo`, `clap`. No new
dependency. Target platforms macOS (development, full suite) and
`aarch64-pc-windows-msvc` (real behaviour).

## Global Constraints

Copied from the spec and from `docs/phase4b-notes.md`'s standing rules. Every
task's requirements implicitly include this section.

- **Spec:** `docs/specs/2026-08-11-phase5-guard-unmanaged-retry-design.md`.
  **Evidence:** `docs/measurements-2026-08-11-phase5-guard-unmanaged-retry.md`.
  Cite the measurement document, never the phase brief, for any number.
- **Every claim in a comment carries its evidence class**: *measured*,
  *structural* (provable by reading), or *reasoned only*. A comment that says
  "measured" about something only inferred is a defect, not a wording nit — the
  Phase 4b fix rounds caught two fabricated mechanisms this way.
- **Never write a false number in a user-facing count.** `Plan::change_count`
  counts Install / Upgrade / Prune / non-winget Downgrade only. This project has
  fixed a false number in the consent line three times.
- **No test may be unable to fail.** Every new test must be run in a red state
  first, and the transcript of the red run recorded in the task report. A
  fixture that cannot express the hazard (`opaque: Vec::new()`,
  `Running::new(["brave.brave"])`) is the named defect class here.
- **A new constant whose value came off a real exit code needs an independent
  cross-check**, not a restatement: `X as u32 == 0xNNNNNNNN`. Five constants
  already have one; the sixth must too.
- Run `cargo test --no-fail-fast`, `cargo fmt --check`, and
  `cargo clippy --all-targets -- -D warnings` before every commit. All three
  must be clean.
- Run `cargo check --target aarch64-pc-windows-msvc --all-targets` for any task
  touching `#[cfg(windows)]` or reading environment variables Windows sets. It
  type-checks Windows paths from macOS; it does **not** catch behavioural
  differences.
- **Expected test counts in this plan are as of `05841da` (588 macOS).** If a
  count disagrees with the tree, trust the tree and say so in the report — three
  Phase 4b briefs carried stale counts, every time too low.
- **Never run `cargo mutants` while any file is being edited.** cargo-mutants
  copies the source tree; a concurrent edit makes every verdict fiction. Phase
  4b discarded 421 verdicts to this. Use `-j 2` and watch free disk space.
- No backtick characters in any PowerShell file, comments included.
- Do not start or stop kanata on a14. Preserve `C:\Users\kln\dotpkg-build` and
  `C:\Users\kln\pkg.toml`.

## File Structure

| file | responsibility in this phase |
|---|---|
| `src/backend/winget.rs` | new `package_roots()` and `running_ids()`; `INTERNAL_ERROR` const; `update_source` retry; `version_liveness` generic-arm message; correct the `bins`-cannot-fire doc comment |
| `src/backend/mod.rs` | new `running_set()` — the single producer of a `Running` for production |
| `src/backend/scoop.rs` | `Scoop::running_set` **removed**; `running_apps` stays and becomes `pub(crate)`-visible to `backend::running_set` |
| `src/model.rs` | `Running`'s doc comment: `dirs` is no longer scoop-only |
| `src/sys.rs` | `normalize` becomes `pub(crate)` |
| `src/config.rs` | `[winget.guard]` parse + validation; `WingetSection::guard` |
| `src/apply.rs` | `load_everything` wires `backend::running_set`; guard-name merge point |
| `src/main.rs` | `status`'s own wiring; the re-sampler closure; `--show-unmanaged` on `Status` and `Apply` |
| `src/plan.rs` | `Plan::unmanaged_count()` |
| `src/render.rs` | `render(&Plan, show_unmanaged: bool)`; per-backend aggregate line; summary clause |
| `docs/phase5-notes.md` | new; the durable record |
| `README.md` | `[winget.guard]` and `--show-unmanaged` |

---

### Task 1: `winget::running_ids` — the path signal, as a pure function

**Files:**
- Modify: `src/backend/winget.rs` (add two functions near `guard_names` at `:204`; add tests to the `#[cfg(test)] mod tests` at `:1025`)

**Interfaces:**
- Consumes: `crate::sys::Process { pub name: String, pub exe: Option<PathBuf> }`; `crate::model::Name` with `Name::new(impl Into<String>)` and `Name::key() -> &str` (the ASCII-folded, lowercased form).
- Produces:
  - `pub(crate) fn package_roots() -> Vec<std::path::PathBuf>`
  - `pub(crate) fn running_ids(roots: &[std::path::PathBuf], procs: &[crate::sys::Process], scanned: &[Name]) -> std::collections::BTreeSet<Name>`

- [ ] **Step 1: Write the failing tests**

Add to `src/backend/winget.rs`'s test module. Every path below is a **measured**
path from the measurement document §1 and §3 — do not invent others.

```rust
    #[test]
    fn running_ids_catches_a_package_whose_process_runs_from_its_winget_package_dir() {
        // Measured on a14 (measurements-2026-08-11 §1): exactly one live
        // process ran from under WinGet\Packages, and this is its real path.
        let roots = vec![PathBuf::from(
            r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages",
        )];
        let procs = vec![Process {
            name: "vkey".to_string(),
            exe: Some(PathBuf::from(
                r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages\PhatMT97.VKey_Microsoft.Winget.Source_8wekyb3d8bbwe\VKey.exe",
            )),
        }];
        let scanned = vec![Name::new("PhatMT97.VKey")];
        assert_eq!(
            running_ids(&roots, &procs, &scanned),
            BTreeSet::from([Name::new("PhatMT97.VKey")])
        );
    }

    #[test]
    fn running_ids_requires_the_underscore_boundary_so_a_dead_sibling_dir_matches_nothing() {
        // Measured: PhatMT97.VKey.Classic_... still exists on disk holding only
        // a config.toml, has no <id>_<hash>.db and no ARP key, and is absent
        // from `winget list`. Its folded segment starts with the folded id
        // "phatmt97.vkey" and must NOT match, because what follows is '.' and
        // not '_'. Without the boundary check this test goes green while the
        // fence claims a package is running that is not even installed.
        let roots = vec![PathBuf::from(
            r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages",
        )];
        let procs = vec![Process {
            name: "whatever".to_string(),
            exe: Some(PathBuf::from(
                r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages\PhatMT97.VKey.Classic_Microsoft.Winget.Source_8wekyb3d8bbwe\config.exe",
            )),
        }];
        let scanned = vec![Name::new("PhatMT97.VKey")];
        assert_eq!(running_ids(&roots, &procs, &scanned), BTreeSet::new());
    }

    #[test]
    fn running_ids_ignores_a_process_whose_path_cannot_be_read() {
        // Measured: 22 of 223 live processes reported no readable path. That is
        // the blind spot `Running.names` covers and this function must not
        // pretend to; a path-only implementation that unwrapped `exe` would
        // panic, and one that treated None as a match would be worse.
        let roots = vec![PathBuf::from(
            r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages",
        )];
        let procs = vec![Process {
            name: "vkey".to_string(),
            exe: None,
        }];
        let scanned = vec![Name::new("PhatMT97.VKey")];
        assert_eq!(running_ids(&roots, &procs, &scanned), BTreeSet::new());
    }

    #[test]
    fn running_ids_only_answers_for_ids_the_scan_actually_found() {
        // The dead-directory case from the other side: a live process under a
        // package dir for an id that is not installed produces nothing,
        // because `covers` is only ever asked about an `Installed`.
        let roots = vec![PathBuf::from(
            r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages",
        )];
        let procs = vec![Process {
            name: "zoxide".to_string(),
            exe: Some(PathBuf::from(
                r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages\ajeetdsouza.zoxide_Microsoft.Winget.Source_8wekyb3d8bbwe\zoxide.exe",
            )),
        }];
        assert_eq!(running_ids(&roots, &procs, &[]), BTreeSet::new());
    }

    #[test]
    fn running_ids_folds_case_on_both_sides() {
        // The real directory is mixed case ("PhatMT97.VKey_...") and
        // `Name::key()` is the lowercased form. A comparison that folds only
        // one side silently never matches -- the exact trap `guard_names`' own
        // doc comment records for process names.
        let roots = vec![PathBuf::from(r"C:\ROOT\Packages")];
        let procs = vec![Process {
            name: "x".to_string(),
            exe: Some(PathBuf::from(
                r"c:\root\packages\AJEETDSOUZA.ZOXIDE_Microsoft.Winget.Source_x\zoxide.exe",
            )),
        }];
        let scanned = vec![Name::new("ajeetdsouza.zoxide")];
        assert_eq!(
            running_ids(&roots, &procs, &scanned),
            BTreeSet::from([Name::new("ajeetdsouza.zoxide")])
        );
    }

    #[test]
    fn running_ids_returns_nothing_when_no_root_exists() {
        // Off Windows `package_roots()` finds no environment variables and
        // returns an empty vector; the function must be a no-op, not a panic.
        let procs = vec![Process {
            name: "vkey".to_string(),
            exe: Some(PathBuf::from("/usr/bin/vkey")),
        }];
        assert_eq!(
            running_ids(&[], &procs, &[Name::new("PhatMT97.VKey")]),
            BTreeSet::new()
        );
    }
```

Add whatever imports the test module needs at its top (`use std::collections::BTreeSet;`, `use std::path::PathBuf;`, `use crate::sys::Process;`).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib running_ids -- --nocapture`
Expected: FAIL to compile, `cannot find function 'running_ids' in this scope`.

- [ ] **Step 3: Write the implementation**

Insert directly after `guard_names` (which ends at `src/backend/winget.rs:215`).

```rust
/// The directories winget installs a `portable` package into, one per scope.
///
/// **Measured** (`docs/measurements-2026-08-11-phase5-guard-unmanaged-retry.md`
/// §3): the user-scope root held 5 package directories on a14, and the
/// machine-scope root did not exist at all -- as did neither
/// `%ProgramFiles%\WinGet\Links` nor its `(x86)` sibling. The machine-scope
/// entry below is therefore **reasoned, not measured**: it is where a
/// machine-scope portable would live, and no such install has been observed.
///
/// Returns an empty vector wherever these variables are unset, which is every
/// non-Windows platform. `running_ids` is a no-op on an empty root list, so
/// nothing needs a `cfg`.
pub(crate) fn package_roots() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        out.push(
            std::path::PathBuf::from(local)
                .join("Microsoft")
                .join("WinGet")
                .join("Packages"),
        );
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        out.push(std::path::PathBuf::from(pf).join("WinGet").join("Packages"));
    }
    out
}

/// Which of `scanned` has a live process running out of its own winget package
/// directory -- the winget analogue of `Scoop::running_apps`, and the signal
/// three documents said could never fire for winget.
///
/// **This is the only signal that would catch a process whose name resembles
/// nothing about its package.** Measured on a14: `kanata`'s process is
/// `kanata_windows_tty_winIOv2_arm64`, and scoop's fence catches it purely
/// because its executable lives under `$SCOOP/apps/kanata/`. `guard_names`
/// would miss it entirely. Nothing gave winget that protection until this
/// function.
///
/// **Coverage is bounded and the bound is measured, not guessed:** winget only
/// creates these directories for `portable` packages -- 4 of 36 installed ids
/// on a14 -- so every EXE/MSI application is invisible here and reachable only
/// through `names` or `[winget.guard]`.
///
/// **Why a per-id prefix test rather than parsing the directory name.** The
/// segment is `<id>_<sourceIdentifier>` in all 5 measured cases, but splitting
/// on `_` assumes a winget id contains none, which is **unmeasured**, and the
/// failure direction is the dangerous one: a truncated segment matches no
/// installed id, so the fence misses and a running package can be replaced.
/// Testing `scanned` against the segment assumes nothing about winget's naming
/// and can only fail toward "no match".
///
/// The `_` boundary is load-bearing rather than decorative. a14 still carries
/// `PhatMT97.VKey.Classic_...` from an uninstalled package, whose folded
/// segment begins with installed `phatmt97.vkey`; a bare `starts_with` would
/// report a package running that is not installed. A bare `<id>` segment with
/// no suffix is accepted too, which is **reasoned, not measured** -- all 5
/// observed directories carry a suffix.
pub(crate) fn running_ids(
    roots: &[std::path::PathBuf],
    procs: &[crate::sys::Process],
    scanned: &[Name],
) -> std::collections::BTreeSet<Name> {
    fn fold(p: &std::path::Path) -> String {
        p.to_string_lossy().replace('\\', "/").to_ascii_lowercase()
    }

    let mut out = std::collections::BTreeSet::new();
    for root in roots {
        let prefix = format!("{}/", fold(root));
        for p in procs {
            // A process whose path cannot be read is `names`' job, not this
            // function's: 22 of 223 on a14.
            let Some(exe) = p.exe.as_deref() else {
                continue;
            };
            let Some(rest) = fold(exe).strip_prefix(&prefix).map(str::to_string) else {
                continue;
            };
            let Some(seg) = rest.split('/').next().filter(|s| !s.is_empty()) else {
                continue;
            };
            for id in scanned {
                let key = id.key();
                let hit = seg == key
                    || seg
                        .strip_prefix(key)
                        .is_some_and(|tail| tail.starts_with('_'));
                if hit {
                    out.insert(id.clone());
                }
            }
        }
    }
    out
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib running_ids`
Expected: PASS, 6 tests.

- [ ] **Step 5: Prove the boundary check is load-bearing**

Temporarily replace the `hit` expression with a bare
`seg.starts_with(key) || seg == key`. Run
`cargo test --lib running_ids_requires_the_underscore_boundary`.
Expected: FAIL. Restore the real expression and confirm
`git diff src/backend/winget.rs` shows only the intended addition. Record both
transcripts in the task report.

- [ ] **Step 6: Full verification and commit**

```bash
cargo test --no-fail-fast          # expect 594 passed, 0 failed
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo check --target aarch64-pc-windows-msvc --all-targets
git add src/backend/winget.rs
git commit -m "Give winget the path signal that is the only thing protecting kanata"
```

---

### Task 2: `backend::running_set` — one producer, three wiring sites

**Files:**
- Modify: `src/backend/mod.rs` (add `running_set`)
- Modify: `src/backend/scoop.rs:220-225` (**remove** `Scoop::running_set`)
- Modify: `src/model.rs:212-216` (`Running`'s doc comment)
- Modify: `src/backend/winget.rs:279-285` (`rows_to_scan`'s doc comment on `bins`)
- Modify: `src/apply.rs:1064`
- Modify: `src/main.rs:470` and `src/main.rs:716`
- Modify: `tests/scoop_scan.rs:649,665,682` (three tests call the removed method)
- Test: `tests/scoop_scan.rs` (union test) and `tests/execute.rs` (re-sampler test)

**Interfaces:**
- Consumes: Task 1's `winget::running_ids` and `winget::package_roots`;
  `Scoop::running_apps(&self, procs: &[Process]) -> BTreeSet<Name>`;
  `Running::new(BTreeSet<String>, BTreeSet<Name>) -> Running`;
  `ScanOutcome::{Scanned(Scan), Unscannable(String)}` with
  `Scan { installed: Vec<Installed>, opaque: Vec<Name>, warnings: Vec<String> }`.
- Produces: `pub fn crate::backend::running_set(scoop: &scoop::Scoop, winget_ids: &[Name], winget_roots: &[PathBuf], procs: &[crate::sys::Process]) -> Running`.
  `Scoop::running_set` no longer exists.

**Why the method is removed rather than kept alongside.** Its own doc comment
says "a caller that drops either input silently loses whatever only that half
could see". Leaving a scoop-only producer in place makes exactly that mistake
writable, and Phase 4b's spec names the consequence: fixing the scanner and not
the sampler "would close the plan-time hole and leave the during-the-run hole
exactly as wide". Deleting it turns that into a compile error.

**`winget_ids` is the winget `installed` names only, never `opaque`.**
Structural: `plan()` reaches `running.covers(inst)` only through an `Installed`,
and an `opaque` id never becomes one — it becomes `Action::Skip { Opaque }`
before any fence is consulted.

- [ ] **Step 1: Write the failing tests**

Add to `tests/scoop_scan.rs`, beside the three existing `running_set` tests at
`:649`, `:665`, `:682`. It uses that file's own helpers — `proc(name, exe)` at
`:357`, `installed_pkg(name, bins)` at `:631`, and `Scoop::new(root)` — and adds
one winget-flavoured `Installed` builder, because `installed_pkg` hardcodes
`SCOOP`:

```rust
fn installed_winget_pkg(name: &str, bins: &[&str]) -> Installed {
    Installed {
        backend: WINGET.to_string(),
        name: Name::new(name),
        version: "0".to_string(),
        arch: None,
        bucket: None,
        bins: bins.iter().map(|b| b.to_string()).collect(),
    }
}

// `backend::running_set` is the ONE producer of a `Running` in production, and
// it must union three inputs, not two. The winget half is what Phase 5 added;
// a caller that kept scoop's old two-input version is green on all three tests
// above, which is why this one exists.
#[test]
fn the_running_set_unions_scoop_paths_with_winget_package_dirs() {
    let root = PathBuf::from("/tmp/dpk-root");
    let wg_root = PathBuf::from("/tmp/dpk-winget/Packages");
    let scoop = Scoop::new(root.clone());
    let procs = [
        // Caught only by its scoop path. Measured on a14: this is kanata's real
        // process name, and it resembles neither the package name nor any
        // prefix or suffix of it.
        proc(
            "kanata_windows_tty_winiov2_arm64",
            Some(root.join("apps/kanata/current/kanata_windows_tty_winIOv2_arm64.exe")),
        ),
        // Caught only by its winget package dir. Measured: the one live process
        // under WinGet\Packages on a14.
        proc(
            "vkey",
            Some(
                wg_root
                    .join("PhatMT97.VKey_Microsoft.Winget.Source_8wekyb3d8bbwe")
                    .join("VKey.exe"),
            ),
        ),
    ];
    let winget_ids = [Name::new("PhatMT97.VKey")];
    let running = dotpkg::backend::running_set(&scoop, &winget_ids, &[wg_root], &procs);

    assert!(running.covers(&installed_pkg("kanata", &[])), "scoop path half lost");
    // `bins` deliberately EMPTY: with "vkey" in it this would pass on the
    // `names` half alone and prove nothing about the path half.
    assert!(
        running.covers(&installed_winget_pkg("PhatMT97.VKey", &[])),
        "winget path half lost"
    );
}
```

Import `WINGET` alongside the file's existing `SCOOP` import.

Add to `tests/execute.rs`, beside
`a_winget_package_that_starts_running_mid_run_is_held`:

```rust
// The mid-run re-sampler is the THIRD wiring site and the easiest to forget:
// `main.rs`'s closure used to be `d.scoop.running_set(...)`, which has no
// winget half at all. This pins the sampler against a winget package caught
// ONLY by its package directory -- no matching process name, no `bins` entry --
// so a sampler still built from scoop alone lets the step run.
#[test]
fn the_re_sampler_holds_a_winget_step_caught_only_by_its_package_directory() {
    // Body: build a Step::Winget(Remove) for PhatMT97.VKey with an EMPTY guard
    // list, a sampler returning a `Running` whose `dirs` contains
    // Name::new("PhatMT97.VKey") and whose `names` contains nothing related,
    // and assert the step is Held rather than executed -- following the exact
    // construction `a_winget_package_that_starts_running_mid_run_is_held`
    // already uses for its fake mutator and its assertions.
}
```

Fill that body by copying the construction of
`a_winget_package_that_starts_running_mid_run_is_held` and changing only the two
things named above (empty guard list; `dirs` instead of `names`). Do not invent
a different fake.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test scoop_scan the_running_set_unions && cargo test --test execute the_re_sampler_holds`
Expected: FAIL to compile, `cannot find function 'running_set' in module 'dotpkg::backend'`.

- [ ] **Step 3: Add `backend::running_set`**

In `src/backend/mod.rs`:

```rust
/// The `Running` every production path receives: process names, scoop's package
/// directories, and winget's package directories, unioned.
///
/// **The only producer of a `Running` outside tests, deliberately.**
/// `Scoop::running_set` used to be that producer and was removed here rather
/// than kept: its own doc comment warned that "a caller that drops either input
/// silently loses whatever only that half could see", and a scoop-only
/// producer left in place keeps that mistake writable. Phase 4b named the
/// consequence exactly -- fixing the scanner and not the mid-run sampler
/// "would close the plan-time hole and leave the during-the-run hole exactly
/// as wide".
///
/// `winget_ids` is the winget scan's `installed` names and never its `opaque`
/// ones. Structural: `plan()` only ever reaches `Running::covers` through an
/// `Installed`, and an `opaque` id becomes `Action::Skip { Opaque }` before any
/// fence is consulted.
pub fn running_set(
    scoop: &scoop::Scoop,
    winget_ids: &[Name],
    winget_roots: &[PathBuf],
    procs: &[crate::sys::Process],
) -> crate::model::Running {
    let names = procs.iter().map(|p| p.name.clone()).collect();
    let mut dirs = scoop.running_apps(procs);
    dirs.extend(winget::running_ids(winget_roots, procs, winget_ids));
    crate::model::Running::new(names, dirs)
}

/// The winget `installed` names a `running_set` call needs, or none when the
/// scan failed outright. An `Unscannable` winget backend contributes no fence
/// entries, which matches `State::reconcile` refusing to reconcile anything
/// from the same outcome.
pub fn winget_fence_ids(outcome: &ScanOutcome) -> Vec<Name> {
    match outcome {
        ScanOutcome::Scanned(s) => s.installed.iter().map(|i| i.name.clone()).collect(),
        ScanOutcome::Unscannable(_) => Vec::new(),
    }
}
```

- [ ] **Step 4: Remove `Scoop::running_set` and rewire the three sites**

Delete `src/backend/scoop.rs:220-225` (the whole `pub fn running_set`), keeping
its doc comment's two-blind-spots paragraph by **moving** it into
`backend::running_set` above — do not delete the reasoning, it is still the
reason the union exists.

`src/apply.rs:1064` becomes:

```rust
    let procs = crate::sys::running_processes();
    let winget_ids = crate::backend::winget_fence_ids(&winget_scan);
    let running = crate::backend::running_set(
        &scoop,
        &winget_ids,
        &crate::backend::winget::package_roots(),
        &procs,
    );
```

`src/main.rs:470` becomes the same four lines, using that arm's own `scoop` and
`winget_scan` bindings.

`src/main.rs:716` becomes:

```rust
            // The winget ids are sampled once: a package cannot become
            // installed part-way through this run without dotpkg installing
            // it, and a step that installs one is not a step this fence gates.
            // The PROCESSES are re-sampled per step, which is the whole point.
            let fence_ids = dotpkg::backend::winget_fence_ids(&d.winget_scan);
            let fence_roots = dotpkg::backend::winget::package_roots();
            let sample = || {
                dotpkg::backend::running_set(
                    &d.scoop,
                    &fence_ids,
                    &fence_roots,
                    &dotpkg::sys::running_processes(),
                )
            };
```

`winget::package_roots` and `winget::running_ids` must be reachable from
`main.rs`, so widen both from `pub(crate)` to `pub` **only if the compiler
demands it**; prefer keeping `running_ids` `pub(crate)` and exporting only
`package_roots`.

- [ ] **Step 5: Update the three existing tests that called the removed method**

`tests/scoop_scan.rs:649`, `:665`, `:682` each call `scoop.running_set(&procs)`.
Change each to
`dotpkg::backend::running_set(&scoop, &[], &[], &procs)` and add to each a
one-line comment saying the two empty slices are the winget half this test
deliberately does not exercise, so the empties read as a choice rather than an
oversight.

- [ ] **Step 6: Fix the two doc comments this task makes false**

`src/model.rs:212-216` currently says `dirs` "is scoop-only by construction:
`Scoop::running_apps` is its only producer … so a winget package's name can
never land in it." Replace with the truth and its bound:

```rust
/// `dirs` carries both backends since Phase 5. `Scoop::running_apps` inserts a
/// path segment under `$SCOOP/apps` or `$SCOOP/persist`;
/// `backend::winget::running_ids` inserts a winget id whose own package
/// directory holds a running executable. **Measured:** winget creates such a
/// directory only for a `portable` package -- 4 of 36 installed ids on a14 --
/// so a winget EXE/MSI application still reaches this fence only through
/// `names` or a declared `[winget.guard]` entry.
```

`src/backend/winget.rs:279-285` says `bins` is filled by `guard_names` and
records the second-alias residual. Keep the residual and correct its framing:
the measured example is `rg`, ripgrep's **only** command, invisible because
`BurntSushi.ripgrep.MSVC`'s last segment is `MSVC` — a wider class than a second
alias. Add that `running_ids` covers the portable subset and `[winget.guard]`
(Task 3) covers the rest.

Then run the check that catches the rest:

```bash
grep -rn "scoop-only" src/ docs/
grep -rn "only the first two can ever fire" src/ docs/
```

Every `src/` hit must be gone or corrected. A `docs/` hit in a **historical**
document (`phase4-notes.md`, the Phase 4/4b specs) stays — this project records
corrections rather than editing history — but each one must appear in Task 8's
corrections list. Note that both greps are line-based and this prose is
line-wrapped: Phase 4b shipped a grep whose prediction could never fire for
exactly that reason. Verify by reading the surrounding lines, not by trusting a
zero count.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --no-fail-fast`
Expected: 596 passed, 0 failed. Investigate any other number rather than
adjusting an assertion to match.

- [ ] **Step 8: Prove the re-sampler test can fail**

Revert only `src/main.rs:716`'s closure to a scoop-only `running_set` call
(passing `&[]` for `winget_ids`). Run
`cargo test --test execute the_re_sampler_holds`. Expected: FAIL. Restore, and
confirm `git diff src/main.rs` shows only the intended change. Record the
transcript.

- [ ] **Step 9: Full verification and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo check --target aarch64-pc-windows-msvc --all-targets
git add -A
git commit -m "Make backend::running_set the one fence producer, so the mid-run hole cannot be left open"
```

---

### Task 3: `[winget.guard]` — parse, fold, validate

**Files:**
- Modify: `src/sys.rs:17` (`normalize` becomes `pub(crate)`)
- Modify: `src/config.rs:79-81` (`WingetSection`), `:146-151` (`RawWingetSection`), `:153-179` (`parse`)
- Test: `src/config.rs`'s `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `crate::sys::normalize(&str) -> String`; `fold_map` and `fold_names`
  already in `config.rs`.
- Produces: `WingetSection { pub packages: Vec<Name>, pub guard: BTreeMap<Name, Vec<String>> }`.
  Values are already normalised — lowercased with any `sys::EXECUTABLE_SUFFIXES`
  suffix removed — so a consumer may compare them against
  `Running`'s `names` directly.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn winget_guard_names_are_normalised_the_way_running_processes_reports_them() {
        // Measured on a14: `Tailscale.Tailscale` is installed and its live
        // processes are `tailscaled` and `tailscale-ipn`, neither of which is
        // the id, the display name, or the last dotted segment. This table is
        // the only mechanism that reaches them -- winget creates no package
        // directory for a non-portable install.
        //
        // The value is written with an extension and mixed case on purpose:
        // `sys::running_processes` lowercases and strips a known executable
        // suffix, so an unfolded comparison silently never matches.
        let cfg = parse(
            r#"
[winget]
packages = ["Tailscale.Tailscale"]

[winget.guard]
"Tailscale.Tailscale" = ["Tailscaled.EXE", "tailscale-ipn"]
"#,
        )
        .unwrap();
        assert_eq!(
            cfg.winget.guard.get(&Name::new("Tailscale.Tailscale")),
            Some(&vec!["tailscaled".to_string(), "tailscale-ipn".to_string()])
        );
    }

    #[test]
    fn a_winget_guard_name_that_is_empty_after_folding_is_a_parse_error() {
        // An empty string in the guard list would sit in the comparison set
        // matching nothing, while reading in pkg.toml as protection.
        let err = parse(
            r#"
[winget.guard]
"Tailscale.Tailscale" = ["  "]
"#,
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("[winget.guard]"), "message was: {msg}");
        assert!(msg.contains("Tailscale.Tailscale"), "message was: {msg}");
    }

    #[test]
    fn a_typo_in_the_winget_guard_table_name_is_refused_not_ignored() {
        // deny_unknown_fields, for the reason this file's `packagess` test
        // already gives: a typo must not read as "you declared nothing".
        assert!(parse(
            r#"
[winget]
packages = ["Tailscale.Tailscale"]
guards = { }
"#
        )
        .is_err());
    }

    #[test]
    fn an_absent_winget_guard_table_is_an_empty_map_not_a_failure() {
        let cfg = parse("[winget]\npackages = [\"Git.Git\"]\n").unwrap();
        assert!(cfg.winget.guard.is_empty());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib winget_guard`
Expected: FAIL to compile, `no field 'guard' on type 'WingetSection'`.

- [ ] **Step 3: Widen `sys::normalize`**

`src/sys.rs:17`: `fn normalize` becomes `pub(crate) fn normalize`. Add to its
doc comment:

```rust
/// `pub(crate)` because `config::parse` folds `[winget.guard]`'s values with
/// this exact function. A second implementation is the "two copies can drift"
/// class, and drift here is silent: an unfolded guard name never matches and
/// reads as protection.
```

- [ ] **Step 4: Add the field and the parse**

`src/config.rs`:

```rust
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WingetSection {
    pub packages: Vec<Name>,
    /// Process names the user says belong to a winget package, because winget
    /// exposes no way for dotpkg to find them out.
    ///
    /// **Measured** (`docs/measurements-2026-08-11-…` §2): `Tailscale.Tailscale`
    /// runs `tailscaled` and `tailscale-ipn`, `AutoHotkey.AutoHotkey` runs
    /// `autohotkey64`, and `Microsoft.WSL` runs `wslservice`. None is the id,
    /// the display name, or the id's last dotted segment, and none is a
    /// `portable` install, so neither `guard_names` nor
    /// `backend::winget::running_ids` reaches any of them.
    ///
    /// Values are normalised by `sys::normalize` at parse time, so they are
    /// directly comparable against `Running`'s `names`.
    pub guard: BTreeMap<Name, Vec<String>>,
}
```

```rust
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWingetSection {
    #[serde(default)]
    packages: Vec<String>,
    #[serde(default)]
    guard: BTreeMap<String, Vec<String>>,
}
```

In `parse`, replace the `winget:` initialiser and add the validation beside the
existing `[scoop.opts]` bucket check:

```rust
        winget: WingetSection {
            packages: fold_names(raw.winget.packages, "[winget]")?,
            guard: fold_map(raw.winget.guard, "[winget.guard]")?
                .into_iter()
                .map(|(id, raw_names)| {
                    let mut names = Vec::new();
                    for raw_name in raw_names {
                        let folded = crate::sys::normalize(raw_name.trim());
                        if folded.is_empty() {
                            anyhow::bail!(
                                "pkg.toml [winget.guard] {id}: a guard name is empty after \
                                 folding. An empty name matches no process while reading here \
                                 as protection."
                            );
                        }
                        if !names.contains(&folded) {
                            names.push(folded);
                        }
                    }
                    Ok((id, names))
                })
                .collect::<Result<BTreeMap<Name, Vec<String>>>>()?,
        },
```

Add `use std::collections::BTreeMap;` if `config.rs` does not already have it
(it does, for `[scoop.opts]`).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib winget_guard && cargo test --no-fail-fast`
Expected: the four new tests PASS; suite 600 passed, 0 failed.

- [ ] **Step 6: Prove the normalisation test can fail**

Temporarily drop the `crate::sys::normalize(...)` call, keeping only `.trim()`.
Run `cargo test --lib winget_guard_names_are_normalised`. Expected: FAIL with
`Tailscaled.EXE` present instead of `tailscaled`. Restore. Record the
transcript.

- [ ] **Step 7: Verify and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
git add src/sys.rs src/config.rs
git commit -m "Let pkg.toml name the winget processes winget will not name"
```

---

### Task 4: Merge `[winget.guard]` into `Installed.bins`, once

**Files:**
- Modify: `src/backend/mod.rs` (add `apply_guard_overrides`)
- Modify: `src/apply.rs` (`load_everything`, after the winget scan)
- Modify: `src/main.rs:468-476` (`status`'s equivalent point)
- Test: `src/backend/mod.rs`'s `#[cfg(test)] mod tests`, and `tests/planner.rs`

**Interfaces:**
- Consumes: Task 3's `WingetSection::guard`; `ScanOutcome`; `Scan::installed`;
  `Installed { backend, name, version, arch, bucket, bins }`.
- Produces: `pub fn crate::backend::apply_guard_overrides(outcome: &mut ScanOutcome, guard: &BTreeMap<Name, Vec<String>>) -> Vec<String>` — returns one
  warning per guard key that matched nothing.

**Why here and not in `rows_to_scan`.** `rows_to_scan` is a pure function of
winget's own output and must not gain a `Config` parameter. Merging into
`Installed.bins` at one post-scan point serves both fences at once, because
`guard_for` (`src/apply.rs:908-914`) copies `inst.bins` into the `Step` the
mid-run re-sampler reads.

- [ ] **Step 1: Write the failing tests**

In `src/backend/mod.rs`'s test module:

```rust
    #[test]
    fn a_guard_entry_is_merged_into_that_packages_bins() {
        let mut outcome = ScanOutcome::Scanned(Scan {
            installed: vec![Installed {
                backend: crate::model::WINGET.to_string(),
                name: Name::new("Tailscale.Tailscale"),
                version: "1.102.2".to_string(),
                arch: None,
                bucket: None,
                bins: vec!["tailscale".to_string()],
            }],
            opaque: Vec::new(),
            warnings: Vec::new(),
        });
        let mut guard = BTreeMap::new();
        guard.insert(
            Name::new("Tailscale.Tailscale"),
            vec!["tailscaled".to_string(), "tailscale-ipn".to_string()],
        );
        let warnings = apply_guard_overrides(&mut outcome, &guard);
        assert!(warnings.is_empty(), "warnings were: {warnings:?}");
        let ScanOutcome::Scanned(s) = &outcome else {
            panic!("outcome changed variant");
        };
        // The guard_names value survives: this ADDS signals, it does not
        // replace them.
        assert_eq!(
            s.installed[0].bins,
            vec![
                "tailscale".to_string(),
                "tailscaled".to_string(),
                "tailscale-ipn".to_string()
            ]
        );
    }

    #[test]
    fn a_guard_entry_matching_no_installed_package_warns_once() {
        // A stale or misspelled id otherwise protects nothing, in silence.
        // This cannot be a parse error: only this point knows the scan.
        let mut outcome = ScanOutcome::Scanned(Scan::default());
        let mut guard = BTreeMap::new();
        guard.insert(Name::new("Tailscale.Typo"), vec!["tailscaled".to_string()]);
        let warnings = apply_guard_overrides(&mut outcome, &guard);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Tailscale.Typo"), "was: {warnings:?}");
        assert!(warnings[0].contains("[winget.guard]"), "was: {warnings:?}");
    }

    #[test]
    fn a_guard_entry_for_a_declared_but_not_installed_package_does_not_warn() {
        // A machine where the app is merely not installed yet must not print a
        // warning on every run. `declared` is what distinguishes that from a
        // typo.
        let mut outcome = ScanOutcome::Scanned(Scan::default());
        let mut guard = BTreeMap::new();
        guard.insert(
            Name::new("Tailscale.Tailscale"),
            vec!["tailscaled".to_string()],
        );
        let warnings =
            apply_guard_overrides_with_declared(&mut outcome, &guard, &[Name::new("Tailscale.Tailscale")]);
        assert!(warnings.is_empty(), "was: {warnings:?}");
    }

    #[test]
    fn an_unscannable_winget_backend_takes_no_guard_names_and_does_not_warn() {
        // Same rule `State::reconcile` follows for the same outcome: an
        // Unscannable backend yields no facts, so nothing can be said about
        // whether a guard key matched.
        let mut outcome = ScanOutcome::Unscannable("winget exploded".to_string());
        let mut guard = BTreeMap::new();
        guard.insert(Name::new("Tailscale.Typo"), vec!["x".to_string()]);
        assert!(apply_guard_overrides(&mut outcome, &guard).is_empty());
    }
```

The third test names `apply_guard_overrides_with_declared`. Settle the shape
now rather than shipping two functions: make the **one** public function take
`declared`, and have `apply_guard_overrides(outcome, guard)` in the other three
tests be a thin `#[cfg(test)]` helper passing `&[]`. Signature:

```rust
pub fn apply_guard_overrides(
    outcome: &mut ScanOutcome,
    guard: &BTreeMap<Name, Vec<String>>,
    declared: &[Name],
) -> Vec<String>
```

Update the three tests to pass `&[]` explicitly, and delete the helper idea —
an explicit `&[]` at four call sites reads better than a second name.

Add to `tests/planner.rs` the end-to-end counterweight:

```rust
// The whole point, through the planner rather than through one function: a
// winget package whose live process name appears NOWHERE in winget's own
// output is skipped as running, because pkg.toml said so.
#[test]
fn a_winget_package_is_held_by_a_guard_name_only_pkg_toml_knows() {
    // Build: declared+locked Tailscale.Tailscale at a version differing from
    // installed, an installed winget entry whose `bins` is exactly what
    // `guard_names("Tailscale.Tailscale", "Tailscale")` returns plus the merged
    // guard name, a `Running` whose `names` contains only "tailscaled", and
    // assert the action is Skip { reason: Running } and NOT Upgrade.
    //
    // Counterweight in the same test: a second declared winget package with no
    // guard entry and no matching process must still produce its Upgrade, so a
    // fix that holds everything cannot pass.
}
```

Fill the body following the construction `tests/planner.rs` already uses for
its running-winget-package test.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib apply_guard_overrides && cargo test --test planner a_winget_package_is_held_by_a_guard`
Expected: FAIL to compile, `cannot find function 'apply_guard_overrides'`.

- [ ] **Step 3: Implement**

In `src/backend/mod.rs`:

```rust
/// Add `[winget.guard]`'s process names to the matching `Installed.bins`, and
/// report every guard key that matched nothing.
///
/// **One merge point serves both fences.** `plan()` reads `Installed.bins`
/// through `Running::covers`, and `apply::guard_for` copies the same `bins`
/// into the `Step` the mid-run re-sampler reads through
/// `Running::covers_any`. Merging in `backend::winget::rows_to_scan` instead
/// would mean handing that pure function a `Config`, which it must not take.
///
/// Names are ADDED, never substituted: `guard_names`' two measured signals
/// still apply, and this is a third.
///
/// A key that matches no installed package and is not declared in
/// `[winget] packages` gets one warning. Keyed on both, because a declared
/// package that is merely not installed yet is the ordinary state of a fresh
/// machine and must not warn on every run; a key that is in neither is a stale
/// or misspelled entry protecting nothing in silence.
pub fn apply_guard_overrides(
    outcome: &mut ScanOutcome,
    guard: &BTreeMap<Name, Vec<String>>,
    declared: &[Name],
) -> Vec<String> {
    let ScanOutcome::Scanned(scan) = outcome else {
        // An Unscannable backend established no facts, so nothing here can say
        // whether a key matched. Same rule `State::reconcile` applies to the
        // same outcome.
        return Vec::new();
    };

    let mut warnings = Vec::new();
    for (id, names) in guard {
        let mut matched = false;
        for inst in scan.installed.iter_mut() {
            if inst.backend != crate::model::WINGET || &inst.name != id {
                continue;
            }
            matched = true;
            for n in names {
                if !inst.bins.contains(n) {
                    inst.bins.push(n.clone());
                }
            }
        }
        if !matched && !declared.contains(id) {
            warnings.push(format!(
                "pkg.toml [winget.guard] {id}: nothing installed and nothing declared by that \
                 name, so these guard names protect nothing"
            ));
        }
    }
    warnings
}
```

- [ ] **Step 4: Wire both call sites**

In `src/apply.rs`'s `load_everything`, between the winget scan and the
`running_set` call added in Task 2:

```rust
    let mut winget_scan = crate::backend::scan_or_warn(&winget);
    let guard_warnings = crate::backend::apply_guard_overrides(
        &mut winget_scan,
        &declared.winget.guard,
        &declared.winget.packages,
    );
    for w in &guard_warnings {
        eprintln!("warning: {w}");
    }
```

Apply the same three statements in `src/main.rs`'s `status` arm, immediately
after its own `scan_or_warn` call at `:468` and **before**
`print_scan_warnings_and_merge` at `:475`, so the guard names are present in
`installed` before `plan()` sees it.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --no-fail-fast`
Expected: 605 passed, 0 failed.

- [ ] **Step 6: Prove the planner test can fail**

Temporarily remove the `apply_guard_overrides` call from `src/main.rs`'s
`status` arm. Run `cargo test --test planner a_winget_package_is_held_by_a_guard`
— if it still passes, the test is exercising the function rather than the wiring
and must be strengthened until removing the wiring turns it red. Restore.
Record the transcript.

- [ ] **Step 7: Verify and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo check --target aarch64-pc-windows-msvc --all-targets
git add -A
git commit -m "Merge declared winget guard names into bins at one point, so both fences get them"
```

---

### Task 5: Collapse `Unmanaged`, per backend, with the summary clause

**Files:**
- Modify: `src/plan.rs` (add `unmanaged_count`)
- Modify: `src/render.rs:8` (`render` signature), `:122-128` (the `Unmanaged` arm), `:145-168` (the summary), and its ten `render(&plan)` test call sites
- Modify: `src/main.rs:24-33` (`Status` args), the `Apply` args, `:487`, `:538`
- Test: `src/render.rs`'s test module; `tests/cli.rs`

**Interfaces:**
- Consumes: `Action::Unmanaged { backend: String, name: Name, version: String }`;
  `Plan::{change_count, skip_count, drift_count, refused_downgrade_count}`.
- Produces: `pub fn render(plan: &Plan, show_unmanaged: bool) -> String`;
  `pub fn Plan::unmanaged_count(&self) -> usize`.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn thirty_six_unmanaged_winget_packages_collapse_to_one_line_per_backend() {
        // 36 is the measured count on a14 (measurements-2026-08-11 §4), and the
        // fixture carries 36 real entries: a `vec![]` or a one-entry plan
        // cannot tell the collapsed form from the per-line form at all.
        let mut plan = Plan::default();
        for i in 0..36 {
            plan.actions.push(Action::Unmanaged {
                backend: WINGET.to_string(),
                name: Name::new(format!("Vendor.Pkg{i}")),
                version: "1.0".to_string(),
            });
        }
        for i in 0..6 {
            plan.actions.push(Action::Unmanaged {
                backend: SCOOP.to_string(),
                name: Name::new(format!("app{i}")),
                version: "1.0".to_string(),
            });
        }
        let out = render(&plan, false);
        assert!(out.contains("? winget   36 installed outside dotpkg"), "was:\n{out}");
        assert!(out.contains("? scoop    6 installed outside dotpkg"), "was:\n{out}");
        assert!(out.contains("--show-unmanaged"), "was:\n{out}");
        // Collapsed means collapsed: no individual id survives.
        assert!(!out.contains("Vendor.Pkg17"), "was:\n{out}");
        assert!(!out.contains("app3"), "was:\n{out}");
        // The clause is mandatory. `change_count` counts an Unmanaged as
        // nothing, so without it 42 printed facts sit under "0 change(s), 0
        // skipped" -- the exact shape `refused_downgrade_count` earned its own
        // clause to avoid.
        assert!(out.contains("0 change(s), 0 skipped, 42 unmanaged"), "was:\n{out}");
    }

    #[test]
    fn show_unmanaged_restores_every_line_and_drops_the_hint() {
        let mut plan = Plan::default();
        for i in 0..36 {
            plan.actions.push(Action::Unmanaged {
                backend: WINGET.to_string(),
                name: Name::new(format!("Vendor.Pkg{i}")),
                version: "1.0".to_string(),
            });
        }
        let out = render(&plan, true);
        assert!(out.contains("Vendor.Pkg17"), "was:\n{out}");
        assert_eq!(
            out.lines().filter(|l| l.contains("(unmanaged -- no action)")).count(),
            36
        );
        assert!(!out.contains("--show-unmanaged"), "was:\n{out}");
        // The clause stays: the count is true in both forms.
        assert!(out.contains("36 unmanaged"), "was:\n{out}");
    }

    #[test]
    fn a_single_unmanaged_package_is_still_collapsed_so_there_is_one_shape_not_two() {
        // Deliberate: no threshold. A threshold is a magic number and gives the
        // output two shapes a reader has to learn.
        let mut plan = Plan::default();
        plan.actions.push(Action::Unmanaged {
            backend: WINGET.to_string(),
            name: Name::new("Vendor.One"),
            version: "1.0".to_string(),
        });
        let out = render(&plan, false);
        assert!(out.contains("? winget   1 installed outside dotpkg"), "was:\n{out}");
        assert!(!out.contains("Vendor.One"), "was:\n{out}");
        assert!(out.contains("0 change(s), 0 skipped, 1 unmanaged"), "was:\n{out}");
    }

    #[test]
    fn a_plan_with_no_unmanaged_packages_gains_no_clause_and_no_line() {
        let mut plan = Plan::default();
        plan.actions.push(Action::Install {
            backend: WINGET.to_string(),
            name: Name::new("Git.Git"),
            version: "2.0".to_string(),
            arch: None,
        });
        let out = render(&plan, false);
        assert!(!out.contains("unmanaged"), "was:\n{out}");
        assert!(out.contains("1 change(s), 0 skipped"), "was:\n{out}");
    }
```

Add to `tests/cli.rs` one test that runs the real binary with
`--show-unmanaged` and one without, against a fixture machine with at least one
undeclared installed scoop app, asserting the two outputs differ in exactly the
documented way. Follow the file's existing `Fixture` construction.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib unmanaged`
Expected: FAIL to compile, `this function takes 1 argument but 2 arguments were supplied`.

- [ ] **Step 3: Add `Plan::unmanaged_count`**

```rust
    /// How many installed-but-unmanaged packages this plan reports, across
    /// every backend.
    ///
    /// Printed as its own clause in the summary line for
    /// `refused_downgrade_count`'s stated reason: a printed line counted in no
    /// number at all reads as "0 change(s), 0 skipped" above facts the user can
    /// see. That argument is stronger here, because `render` collapses these
    /// lines and the collapse is what removes the 36 lines that carried the
    /// fact.
    pub fn unmanaged_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a, Action::Unmanaged { .. }))
            .count()
    }
```

- [ ] **Step 4: Change `render` and collapse the arm**

`pub fn render(plan: &Plan, show_unmanaged: bool) -> String`.

When `show_unmanaged` is false: skip every `Action::Unmanaged` in the main loop,
and after the loop emit one line per backend in the order the backends first
appeared, each `format!("  ? {backend:<6} {n} installed outside dotpkg -- no action")`,
followed by one indented `pass --show-unmanaged to list them` line. When true,
keep today's per-line arm exactly and emit no hint.

In the summary, add — beside the existing `drift_count` and
`refused_downgrade_count` clauses, and following their shape:

```rust
        if plan.unmanaged_count() > 0 {
            summary.push_str(&format!(", {} unmanaged", plan.unmanaged_count()));
        }
```

**Per backend, not merged into one line**, because the `{backend:<6}` column is
what tells a user which tool to go look at, and one merged line would repeat
`docs/phase4-notes.md`'s still-open "the merged `opaque` list's lost backend
attribution" minor.

Note the `plan.actions.is_empty()` guard at `:145`: a plan whose only actions
are `Unmanaged` is **not** empty, so it must not print "nothing to do". Verify
that path explicitly.

- [ ] **Step 5: Update the ten in-module call sites and both binary call sites**

`src/render.rs:638, 1071, 1097, 1152, 1195, 1234, 1281, 1323, 1357, 1384` each
call `render(&plan)`. Pass `false` at each **except** any whose assertions read
an individual `? ` line — pass `true` there and add a one-line comment saying
the test is about the per-line form. Do not weaken an assertion to fit the
default.

`src/main.rs`: add to both `Status` and `Apply`

```rust
        /// List every installed-but-unmanaged package instead of collapsing
        /// them to one line per backend.
        #[arg(long)]
        show_unmanaged: bool,
```

and pass it at `:487` and `:538`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --no-fail-fast`
Expected: 611 passed, 0 failed.

- [ ] **Step 7: Prove the summary clause can fail**

Delete the `unmanaged_count` clause from the summary. Run
`cargo test --lib thirty_six_unmanaged`. Expected: FAIL on the
`0 change(s), 0 skipped, 42 unmanaged` assertion. Restore. Record the
transcript — three earlier fixes to this line shipped without one.

- [ ] **Step 8: Verify and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
git add -A
git commit -m "Collapse unmanaged packages per backend, and count them in the line a user reads"
```

---

### Task 6: One retry for `update_source`, on the measured signature

**Files:**
- Modify: `src/backend/winget.rs` (new constant near the other exit codes; `update_source` at `:1002-1022`)
- Test: `src/backend/winget.rs`'s test module

**Interfaces:**
- Consumes: `WingetCmd::run(&[&str]) -> Result<CmdOut, CmdError>` and
  `CmdOut { code: i32, stdout: String }` as the existing fakes construct them.
- Produces: `pub(crate) const INTERNAL_ERROR: i32 = -1978335231;`,
  `Winget::update_source(&self) -> Result<()>` (unchanged signature), and
  `Winget::update_source_with(&self, retry_delay: std::time::Duration) -> Result<()>`.

- [ ] **Step 1: Add a scripted fake to `winget.rs`'s test module**

`src/backend/winget.rs`'s test module has **no** `WingetCmd` fake today — its
tests all exercise pure functions. The one that exists is `FakeWinget` in
`src/apply.rs`'s test module (`:1332`), which is not reachable from here. Add a
sibling modelled on it, in `src/backend/winget.rs`'s test module:

```rust
    /// A `WingetCmd` answering from a scripted queue, counting its calls.
    ///
    /// Modelled on `src/apply.rs`'s `FakeWinget` (`RefCell` queue plus a call
    /// recorder), which lives in a different module's `#[cfg(test)]` and cannot
    /// be reached from here. Two fakes for one trait is worse than one, but the
    /// alternative is making `apply`'s fake `pub(crate)` and dragging its four
    /// canned constructors along with it.
    ///
    /// `calls()` is readable after `Winget::new` moves the fake because this
    /// module is a child of the one that declares `Winget`, so its private
    /// `cmd` field is in scope.
    struct ScriptedWinget {
        queue: std::cell::RefCell<std::collections::VecDeque<Result<CmdOut, CmdError>>>,
        calls: std::cell::Cell<usize>,
    }

    impl ScriptedWinget {
        fn new(script: Vec<Result<CmdOut, CmdError>>) -> ScriptedWinget {
            ScriptedWinget {
                queue: std::cell::RefCell::new(script.into_iter().collect()),
                calls: std::cell::Cell::new(0),
            }
        }
        fn calls(&self) -> usize {
            self.calls.get()
        }
    }

    impl WingetCmd for ScriptedWinget {
        fn run(&self, _args: &[&str]) -> Result<CmdOut, CmdError> {
            self.calls.set(self.calls.get() + 1);
            self.queue
                .borrow_mut()
                .pop_front()
                .expect("a winget call was made that the script did not anticipate")
        }
    }
```

The `expect` is load-bearing: a third call would otherwise return a stale answer
and the retry tests below could not tell two attempts from three.

- [ ] **Step 2: Write the failing tests**

Replace every `ScriptedWinget::new(vec![...])` below with
`ScriptedWinget::new(vec![...])`.

```rust
    #[test]
    fn the_internal_error_codes_decimal_and_hex_forms_still_agree() {
        // The sixth constant that would otherwise exist exactly once in the
        // tree with no test pinning its value -- the defect class
        // NO_AVAILABLE_UPGRADE fell into, where every test builds its CmdOut
        // from the constant so a sign flip flips the tests with it. Measured:
        // 0x8A150001 = 2316632065 = (-1978335231 as u32).
        assert_eq!(INTERNAL_ERROR as u32, 0x8A150001);
    }

    #[test]
    fn update_source_retries_once_on_the_measured_contention_failure() {
        // Measured (measurements-2026-08-11 §5): `source update --name winget`
        // exited 0 of 10 times alone and 3 of 10 with another winget process
        // alive, every failure 0x8A150001 in 60-72 ms with empty stdout. The
        // consequence today is not a failed run -- update.rs downgrades the
        // Err to a warning -- it is that `dotpkg update` resolves `latest`
        // against an index it failed to refresh, 3 times in 10, and only warns.
        let fake = ScriptedWinget::new(vec![
            Ok(CmdOut { code: INTERNAL_ERROR, stdout: String::new() }),
            Ok(CmdOut { code: 0, stdout: "Updating source: winget...\n".to_string() }),
        ]);
        let w = Winget::new(fake);
        assert!(w.update_source_with(std::time::Duration::ZERO).is_ok());
        assert_eq!(w.cmd.calls(), 2);
    }

    #[test]
    fn update_source_does_not_retry_any_other_nonzero_exit() {
        // A retry on a definitive answer only slows a certain failure down.
        let fake = ScriptedWinget::new(vec![
            Ok(CmdOut { code: NO_APPLICATIONS_FOUND, stdout: "No package found\n".to_string() }),
            Ok(CmdOut { code: 0, stdout: "Updating source: winget...\n".to_string() }),
        ]);
        let w = Winget::new(fake);
        assert!(w.update_source_with(std::time::Duration::ZERO).is_err());
        assert_eq!(w.cmd.calls(), 1);
    }

    #[test]
    fn update_source_gives_up_after_one_retry() {
        let fake = ScriptedWinget::new(vec![
            Ok(CmdOut { code: INTERNAL_ERROR, stdout: String::new() }),
            Ok(CmdOut { code: INTERNAL_ERROR, stdout: String::new() }),
        ]);
        let w = Winget::new(fake);
        let err = w.update_source_with(std::time::Duration::ZERO).unwrap_err();
        assert!(format!("{err:#}").contains("another winget process"), "was: {err:#}");
        assert_eq!(w.cmd.calls(), 2);
    }
```

Every `Ok(CmdOut { .. })` above needs its `Err` arm too where the script models a
spawn failure; none of these four do, so all entries are `Ok`.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib update_source`
Expected: FAIL to compile, `cannot find value 'INTERNAL_ERROR'`.

- [ ] **Step 4: Add the constant**

Beside the other exit-code constants in `src/backend/winget.rs`:

```rust
/// `0x8A150001` -- winget's generic internal error.
///
/// **Measured** (`docs/measurements-2026-08-11-…` §5), and measured for
/// **`source update` only**: that command exited nonzero 0 of 10 times run
/// alone and 3 of 10 with one other winget process alive, every failure this
/// code, returning in 60-72 ms with empty stdout where a success takes
/// 348-623 ms and prints `Updating source: winget...`. So the failure is
/// distinguishable on exit code, duration and output presence independently,
/// and its trigger is a concurrent winget process rather than the network.
///
/// **Never observed from `show` or `list`.** Those argvs returned 0 nonzero
/// exits in 105 invocations, including 30 fired against a continuously running
/// `source update` loop. "Readers share the index and the updater needs it
/// exclusively" is a **mechanism inferred from those numbers, not a measured
/// property of the reader**, and nothing in this crate may state otherwise.
pub(crate) const INTERNAL_ERROR: i32 = -1978335231; // 0x8A150001
```

- [ ] **Step 5: Add the retry**

Replace `update_source`'s body with a thin wrapper plus the real function.
Keep the existing doc comment and add the retry paragraph to it.

```rust
    pub fn update_source(&self) -> Result<()> {
        // 1 s comes off the measurements rather than being picked: the failure
        // returns in 60-72 ms and the competing winget call it lost to runs
        // 407-1117 ms, so a shorter delay retries into the same contention.
        // 1 s covers the measured maximum and is **not** measured to be
        // sufficient on a slower machine.
        self.update_source_with(std::time::Duration::from_secs(1))
    }

    /// `update_source` with the retry delay injected, so the tests do not
    /// sleep. The seam this crate has extracted six times before, for the same
    /// reason: the rule is what needs proving.
    pub fn update_source_with(&self, retry_delay: std::time::Duration) -> Result<()> {
        let argv = [
            "source",
            "update",
            "--name",
            "winget",
            "--disable-interactivity",
        ];
        let mut last: Option<CmdOut> = None;
        for attempt in 0..2 {
            let out = match self.cmd.run(&argv) {
                Ok(out) => out,
                Err(e) => bail!("winget source update could not be run: {e}"),
            };
            if out.code == 0 {
                return Ok(());
            }
            // Retry only the measured transient. Any other nonzero exit is a
            // definitive answer, and retrying one only slows a certain failure.
            if out.code != INTERNAL_ERROR {
                last = Some(out);
                break;
            }
            last = Some(out);
            if attempt == 0 && !retry_delay.is_zero() {
                std::thread::sleep(retry_delay);
            }
        }
        let out = last.expect("the loop runs at least once");
        if out.code == INTERNAL_ERROR {
            anyhow::bail!(
                "winget source update exited {} twice ({:#x} -- measured to mean another \
                 winget process held the index): {}",
                out.code,
                out.code as u32,
                out.stdout.lines().next().unwrap_or("(no output)")
            );
        }
        anyhow::bail!(
            "winget source update exited {}: {}",
            out.code,
            out.stdout.lines().next().unwrap_or("(no output)")
        )
    }
```

Note `retry_delay.is_zero()` guards the sleep so `Duration::ZERO` in tests does
not call into the scheduler at all.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib update_source && cargo test --no-fail-fast`
Expected: the four new tests PASS; suite 615 passed, 0 failed.

- [ ] **Step 7: Prove the hex cross-check can fail**

Change the constant to `1978335231` (drop the minus). Run
`cargo test --lib the_internal_error_codes_decimal_and_hex`. Expected: FAIL.
Restore. Record the transcript.

- [ ] **Step 8: Verify and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
git add src/backend/winget.rs
git commit -m "Retry winget source update once, on the one transient that was actually measured"
```

---

### Task 7: `version_liveness`'s generic arm learns the one code, and no retry

**Files:**
- Modify: `src/backend/winget.rs:893-900` (the generic nonzero arm)
- Test: `src/backend/winget.rs`'s test module

**Interfaces:**
- Consumes: Task 6's `INTERNAL_ERROR`.
- Produces: no signature change. `version_liveness` still returns
  `Result<Found, String>` and still returns `Err` for every nonzero exit.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn version_liveness_names_the_contention_cause_without_retrying_it() {
        // No retry here, deliberately: the argv this function uses returned 0
        // nonzero exits in 105 invocations (measurements-2026-08-11 §5),
        // including 30 against a continuous source-update loop. Building a
        // retry loop on an unobserved failure mode only slows a certain
        // failure down. What it CAN do is say what the code has been measured
        // to mean elsewhere.
        // No `Winget` wrapper: `version_liveness` is a free function taking
        // `&dyn WingetCmd`, which is also why it is the seam `main.rs` holds.
        let fake = ScriptedWinget::new(vec![Ok(CmdOut {
            code: INTERNAL_ERROR,
            stdout: String::new(),
        })]);
        let err = version_liveness(&fake, &Name::new("Git.Git"), "2.0").unwrap_err();
        assert!(err.contains("another winget process"), "was: {err}");
        assert!(err.contains("re-run"), "was: {err}");
        // Exactly one call: this arm must not have grown a retry.
        assert_eq!(fake.calls(), 1);
    }
```

Uses Task 6's `ScriptedWinget`, which is already in this test module.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib version_liveness_names_the_contention_cause`
Expected: FAIL — the message is the generic `winget show … exited …`.

- [ ] **Step 3: Implement**

Insert before the existing generic arm at `:893`:

```rust
    if out.code == INTERNAL_ERROR {
        // `INTERNAL_ERROR` was measured from `source update`, never from
        // `show`: this argv returned 0 nonzero exits in 105 invocations, 30 of
        // them against a continuously running `source update`. That the reader
        // wins the race is a MECHANISM inferred from those numbers, not a
        // measured property of this call, and this arm exists so that if the
        // inference is ever wrong the operator gets the cause rather than a
        // bare exit code. There is no retry: see this arm's own test.
        return Err(format!(
            "{}: winget exited {:#x}, which was measured to mean another winget process held \
             the index -- re-run once nothing else is using winget",
            id, out.code as u32
        ));
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib version_liveness && cargo test --no-fail-fast`
Expected: 616 passed, 0 failed.

- [ ] **Step 5: Verify and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
git add src/backend/winget.rs
git commit -m "Name the contention cause in version_liveness, and add no retry there"
```

---

### Task 8: The durable record

**Files:**
- Create: `docs/phase5-notes.md`
- Modify: `README.md`

**Interfaces:** none.

- [ ] **Step 1: Write `docs/phase5-notes.md`**

Follow `docs/phase4b-notes.md`'s structure exactly. Mandatory sections:

1. **"Read this first" — the user-visible behaviour changes that are not
   additions.** There are **two**, and the second must not be omitted the way
   Phase 4b's heading omitted its scoop half:
   - `status` and `apply --prepare` now collapse `Unmanaged` lines **for both
     backends**, so a scoop user with undeclared apps sees different output on a
     machine whose configuration did not change. `--show-unmanaged` restores the
     previous output exactly; nothing dotpkg *does* changed.
   - `[winget.guard]` is a new `pkg.toml` table. A file that does not use it
     behaves identically.
2. **What was measured versus what was only reasoned**, copying the labels from
   the measurement document rather than re-deriving them. State plainly that A1
   adds **zero** new catches on the measured machine and that its value is the
   class, not a number.
3. **Corrections to earlier documents**, carrying every item from the spec's own
   corrections section: the brief's non-existent `leftover Links: 2`; `Links` is
   not item 10's oracle; item 9's class is wider than a second alias (`rg`);
   item 2's framing; item 11's falsified direction plus the two things it does
   not say (`--keep-going` holds every removal; `status` never calls
   `version_liveness`). Plus every historical `docs/` hit from Task 2 Step 6.
4. **Method failures**, copying §6 of the measurement document verbatim in
   substance — the four probe failures and the designer's own `36 change(s)`
   mock-up.
5. **Still open**, renumbered from `docs/phase4b-notes.md`'s list with each
   item's status stated: 2, 9 and 11 rewritten by this phase; **10 not closed**,
   with `installed.db` recorded as the lead and explicitly not opened; and every
   inherited verification debt carried forward unchanged — the three
   `#[cfg(windows)]` `sys.rs` mutants, `main.rs:773`, the 14 Phase 4
   `winget.rs` mutants, the two `RealWingetMutator::run` mutants, and an
   ordinary non-elevated Windows session with no `runas`.
6. **Verification**, filled in by Task 9 and left with a placeholder-free
   "pending Task 9" sentence until then.

- [ ] **Step 2: Update `README.md`**

Document `[winget.guard]` with the three measured examples and one sentence on
why it exists (winget exposes no way for dotpkg to discover a package's process
names). Document `--show-unmanaged` on both `status` and `apply`. Then check the
README for sentences this phase falsified:

```bash
grep -n "unmanaged\|winget" README.md
```

Read every hit. Phase 4b's post-merge audit found three stale README claims at
once; this grep is line-based and the README wraps, so read surrounding lines.

- [ ] **Step 3: Commit**

```bash
git add docs/phase5-notes.md README.md
git commit -m "Record Phase 5: what was measured, what was reasoned, and what is still open"
```

---

### Task 9: Verification — the machine, not the argument

**Files:** none in `src/`. Updates `docs/phase5-notes.md`'s Verification
section.

**Interfaces:** none.

This task is **controller-held**: it owns long waits, and an agent that
backgrounds a process ends its turn.

- [ ] **Step 1: macOS suite on the tree that ships**

```bash
cargo test --no-fail-fast
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo build --all-targets
cargo check --target aarch64-pc-windows-msvc --all-targets
cargo clippy --target aarch64-pc-windows-msvc --all-targets -- -D warnings
```
Record the exact `test result:` line count and totals. Expected 616 passed / 0
failed / 0 ignored across 14 lines — verify against the tree, and if it differs,
report the tree's number and why.

- [ ] **Step 2: Fixture integrity, before trusting any Windows result**

```bash
wc -c tests/fixtures/winget/list-full.txt   # expect 30958
python3 -c "print(open('tests/fixtures/winget/list-full.txt','rb').read().count(b'\r\n'))"  # expect 143
```
A wrong `autocrlf` checkout rewrites the only real bytes in the suite. **Note:
30958 bytes and 143 CRLF pairs are ALSO what the corrupted probe capture
`p1-list.txt` measured, with different content** — so check the file's sha256
against the committed blob, not only these two numbers.

- [ ] **Step 3: `cargo mutants`, on an idle tree with nothing editing it**

```bash
cargo mutants --in-diff <(git diff 05841da..HEAD -- 'src/*.rs' 'src/**/*.rs') -j 2 --timeout 600
```
Phase 4b discarded 421 verdicts because an agent ran this concurrently with its
own edits. Do not edit any file while it runs. Watch free disk space; Phase 4
lost a whole run to a full disk at `-j 4`. Triage every survivor into "real test
gap, closing it" or "recorded with why", and close the real ones in a separate
commit **after** the run finishes.

- [ ] **Step 4: Windows suite run #1, before the dogfood**

Tarball `Cargo.toml`, `Cargo.lock`, `src/`, `tests/` — never `target/`, never
`.git/`. Put the tree's sha in `SHIPPING-SHA.txt` inside the tarball and have
the runner echo it back, so the run cannot be attributed to another tree.
Verify fixture bytes on the Windows checkout first. Then:

```
cargo test --no-fail-fast
cargo test --test cli -- --ignored on_a_real_elevated_windows_session
```

**Cross-reference name by name, never by subtracting totals.** Decode captured
output by BOM, not by assumption — `Tee-Object` writes UTF-16LE and once made
this check read 0 names while grep read 565. Match `#[should_panic]` tests,
which print as `test <name> - should panic ... ok` and are dropped by a
`^test (\S+) \.\.\. ` regex. Key on the full test path, not the bare name:
`a_save_leaves_no_temp_file_behind` exists in both `src/state.rs` and
`src/lock.rs`. The expected difference set is the same three `cfg` exclusions
every Phase 4b run saw — two `#[cfg(unix)]` tests absent on Windows, one
`#[cfg(windows)] #[ignore]` test absent on macOS.

- [ ] **Step 5: Dogfood on a14, at medium integrity**

Use `runas /trustlevel:0x20000 "powershell -File <script>"` to de-elevate;
`schtasks /RL LIMITED` does not work and leaves the task Queued. Cover, in one
session:
- **A1 live**: start a process from under `WinGet\Packages` (`VKey` is already
  running there) and confirm `status` reports that package as running.
- **A2 live**: add the three measured `[winget.guard]` entries to a **dogfood
  copy** of `pkg.toml`, not `C:\Users\kln\pkg.toml`, and confirm
  `Tailscale.Tailscale` is held while `tailscaled` runs.
- **B live**: confirm the collapsed line reports the real count and that
  `--show-unmanaged` lists them.
- **C live**: run `dotpkg update` against a declared winget package while a
  second winget process runs, and look for the retry in behaviour.
Restore the machine and prove it: `winget list` sha before and after, scoop app
count, `pkg.toml` sha, no `pkg.toml.bak`, kanata's PID unchanged. Do not start
or stop kanata.

- [ ] **Step 6: Windows suite run #2, on the exact tree that will merge**

Repeat Step 4 on the post-dogfood, post-mutation-fix tree. The tree changed, so
the run must happen again — Phase 4b ran the suite four times for this reason.

- [ ] **Step 7: Whole-branch review, then merge, then an independent audit**

Review the branch as one change rather than as nine tasks: Phase 4b's Critical
finding was invisible to all seventeen per-task reviews because each saw only a
piece. Merge fast-forward only. Then dispatch an independent post-merge audit to
a fresh agent with no part in the work.

- [ ] **Step 8: Fill in `docs/phase5-notes.md`'s Verification section and commit**

Record every count, every cross-reference result, and every machine-state check.
If a watch-list item was not met, say so in the notes — Phase 4b shipped with
its own pre-merge gate unmet and the shipped documents did not say so.

- [ ] **Step 9: Clean up the probe directory**

`C:\Users\kln\phase5-probe` holds this phase's probe scripts and raw captures.
Ask the human before deleting: the scripts are the reusable part, and
`p1-list.txt` must be marked in the notes as **not fixture-grade** so nobody
promotes it later.

---

## Self-Review

**1. Spec coverage.** Every spec section maps to a task: A1 → Tasks 1 and 2;
A2 → Tasks 3 and 4; A3 (rejected heuristic widening) → recorded in Task 8's
notes, no code; B1 → Task 5; B2 (the disclosed scoop behaviour change) →
Task 5 plus Task 8's "Read this first"; B3 (rejected `WINGET_HELPERS` and
`[winget] ignore`) → Task 8's notes; C1 → Task 6; C2 → Task 7. The spec's
Testing section is distributed across each task's red-state step, and its
"standing rules from Phase 4b" are in Global Constraints and Task 9.
The spec's Non-goals need no task except item 10's explicit non-closure, which
is Task 8 Step 1 item 5 and Task 9 Step 9.

**2. Placeholder scan.** Two task bodies deliberately describe a test body
rather than spelling it out — Task 2's re-sampler test and Task 4's planner
test — because both must copy an existing test's fake and fixture construction
exactly, and transcribing that construction here would create a second source
of truth that drifts from the file. Each names the exact existing test to copy
and the exact two things to change. Tasks 6 and 7 no longer defer their fake:
`ScriptedWinget` is written out in Task 6 Step 1, because `src/backend/winget.rs`
has no `WingetCmd` fake today and the plan's first draft cited one that does not
exist. Everything else carries its code.

**3. Type consistency.** Three first-draft slips were found by reading the tree
and are fixed above rather than shipped: `install_app` in `tests/scoop_scan.rs`
does not exist (the real helpers are `proc`, `installed_pkg` and `Scoop::new`,
and `installed_pkg` hardcodes `SCOOP`, so Task 2 adds a winget sibling);
`RecordingWinget` does not exist anywhere (Task 6 now defines `ScriptedWinget`);
and `Winget<C>::cmd` is a private field, reachable only from a test module inside
`winget.rs`, which is why Task 7's test calls `version_liveness(&fake, ...)`
directly instead of through a wrapper.

`running_ids(roots, procs, scanned)` is called with
that argument order in Tasks 1 and 2. `running_set(scoop, winget_ids,
winget_roots, procs)` matches at all three wiring sites and both new tests.
`apply_guard_overrides(outcome, guard, declared)` is three arguments everywhere
after Step 1's correction — the plan resolves its own two-signature slip inline
rather than shipping it. `render(plan, show_unmanaged)` matches Task 5's tests
and both `main.rs` call sites. `INTERNAL_ERROR` is `i32`, matching `CmdOut::code`
and the five existing constants. `Name::key() -> &str`, verified against
`src/model.rs:56`.

**4. Expected test counts.** 588 → 594 (T1) → 596 (T2) → 600 (T3) → 605 (T4)
→ 611 (T5) → 615 (T6) → 616 (T7). Each task's step says to trust the tree over
the plan if they disagree.
