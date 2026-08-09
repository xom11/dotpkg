# Dogfood: Phase 2b-2, the executor — a14, 2026-08-09

Both stages. Stage 1 ran against a throwaway `$env:SCOOP` root on a14, driven by a
real `scoop` 0.5.3 and the real `main` bucket. The machine's own `~/scoop` was
read but never written: **31 apps and 76 cache entries before and after,
`kanata` (`kanata_windows_tty_winIOv2_arm64`, PID 15732) never started or
stopped, and `%LOCALAPPDATA%\dotpkg\state.json` still does not exist.**

Branch `phase2b2-executor`, built natively on a14 with `rustc 1.97.1`.

**The headline: the dogfood paid for itself four times before a single package
was removed, and one of those was a bug that would have made the executor
report every successful install as a failure.**

## What the machine is

Captured before anything ran. Two things had moved since the measurement round
of 2026-08-08 and are recorded so nothing downstream compares against stale
numbers: **`kanata`'s PID is 15732, not 7868** (the machine slept in between),
and **`beckon` is 0.4.0, not 0.2.9**. App count 31, cache 76.

`state.json` does not exist. dotpkg owns nothing on this machine, which is
exactly why the plan's prune could only be exercised after dotpkg's own install
path had put an entry there.

## Four defects found before any software was touched

### 1. A test green on macOS and red on Windows: rendered paths

`state::tests::the_temp_path_is_derived_from_the_real_target_not_hardcoded`
compared a whole rendered path against a hardcoded `"/x/…"` prefix. Windows
renders `Path::new("/x").with_file_name(..)` as `/x\name`, so the assertion
could not hold there. Production was correct; the test was not portable. It now
asserts `file_name()` and `parent()` separately.

### 2 and 3. Two more, hidden behind cargo's early stop

`cargo test` stops at the first failing target, so the first Windows run
reported one failure and concealed two. With `--no-fail-fast`:

- `exit_code_asserts_a_refused_run_changed_nothing` is `#[should_panic]` on a
  `debug_assert!`, which is compiled out under `--release`. A **release-vs-debug**
  divergence, not a platform one — CI runs debug and stayed green. Now gated on
  `debug_assertions`.
- `the_scoop_entry_point_is_the_cmd_shim` compared the shim path against
  `fs::canonicalize(root)`, which on Windows returns the `\\?\` extended-length
  form — while `Scoop::new` **deliberately strips** that prefix. The test
  contradicted the behaviour `strip_extended_prefix` exists to provide. It now
  compares canonicalised directories.

**Use `--no-fail-fast` on every future cross-platform run.** One failing target
hid two real defects.

### 4. The one that mattered: every successful install reported as a failure

With the suite green on both platforms, the first real `apply` produced this:

```
  ^ scoop  fzf            0.74.1 -> 0.74.2         (upgrade, arm64)
  ready   scoop  fzf          0.74.1 -> 0.74.2  (upgrade, arm64)
  FAILED  scoop  fzf          fzf: install did not happen -- the installed
                              manifest matches the staged one except for line endings
  0 verified on disk, 1 failed, 0 held.
  Some packages were changed and some were not. Look at the machine.

after:  fzf = 0.74.2      state = {}      EXIT = 1
```

**The upgrade had worked.** fzf was 0.74.2 on disk. dotpkg reported it failed.

scoop rewrites line endings when it copies the staged manifest into
`apps/<app>/current`, so `verdict`'s exact byte comparison fails on **every**
successful install on Windows. The consequences compound: every run exits 1,
and `state.json` stays `{}` — so nothing dotpkg installs is ever recorded as
owned, and therefore nothing is ever prunable. The whole ownership model would
have been dead on arrival.

`verdict` now accepts a difference that vanishes under `normalise` (CRLF folded,
trailing newlines dropped). That cannot change a `url` or a `hash`, so the check
still catches what it exists for — a *different* manifest carrying the same
version, which is what a lock naming a branch produces. A test pins exactly
that, with the line endings differing too. `Disagreement::LineEndingsDiffer` is
removed rather than left unreachable.

The Phase 2b-2 design anticipated this as a possibility and chose to report it
as its own diagnosis rather than accept it. **Measured, that was the wrong
call** — and it is the second time this project's design has been corrected by
running the thing rather than reasoning about it.

## What Stage 1 then established

All of the following on a real scoop, a real bucket, and real artifacts.

**The full loop works.** `apply --yes`, 7.57 s:

```
  ^ scoop  fzf            0.74.1 -> 0.74.2         (upgrade, arm64)
  ready   scoop  fzf          0.74.1 -> 0.74.2  (upgrade, arm64)
  done    scoop  fzf          verified on disk
  1 verified on disk, 0 failed, 0 held.            EXIT = 0

after:  fzf = 0.74.2   state = { "scoop": { "fzf": "installed" } }
recover.cmd removed after the clean run
```

Uninstall + install, verified on disk, **exactly one `state.json` key**, and the
recovery artifact cleaned up because nothing failed. The architecture appears in
the plan the user confirms (`upgrade, arm64`) — the Task 8 property, visible on
a real machine.

**A converged machine is not asked and does not fail.** Exit 0, no prompt:
`Nothing to do -- the machine already matches pkg.toml and pkg.lock.`

**A nonexistent commit refuses the whole run.** Exit 2, `commit deadbeef… is
not in bucket "main"`, fzf unchanged.

**A branch name as a commit is refused for its shape**, before anything is
staged — the live defect this phase opened with, now closed on real hardware:

```
pkg.lock is not usable. Run `dotpkg update` to rewrite it.: fzf: the lock's
commit "main" is not a commit hash -- it must be 40 (or 64) lowercase hex
characters. A branch or tag name resolves to whatever the bucket points at
today, which is not a pin.
```

**`stage_text`'s version check fires on real bucket data:** `the lock says
"0.0.0-not-real" but bucket/ripgrep.json at be98b1a4… is "15.2.0"`.

**The mass-prune guard fires:** `pkg.toml declares no scoop packages but dotpkg
owns 1. Refusing to prune everything.`

**The removals gate holds on real hardware.** With one package unpreparable and
a ready prune in the same plan, `--yes --allow-prune` still refused:
`1 of 2 changes ready, 1 failed` → exit 2, fzf still installed. No flag opens
that gate.

**`--yes` alone does not authorise a prune.** Exit 2, `this run would remove 1
package(s) and --yes was passed. Removals need --allow-prune as well.`

**And the prune itself works.** `--yes --allow-prune --allow-empty-config`:

```
  - scoop  fzf            0.74.2                   (prune, owned)
  done    scoop  fzf          verified on disk
  1 verified on disk, 0 failed, 0 held.            EXIT = 0

fzf = ABSENT      state = { "scoop": {} }      apps/ = (empty)
```

Removed from disk, released from the fence, verified.

## What Stage 1 did NOT establish, and why

Recorded because a dogfood that confirms everything is a dogfood that was not
trying.

**`Disagreement::ContentDiffers` was never exercised.** Two attempts to induce
it failed for the same reason, which is itself a finding: **`apply` re-runs
`prepare`, which re-derives the staged manifest from the lock**, so anything
tampered with between `--prepare` and `apply` is silently healed. That is a
genuinely good property — the run cannot be poisoned by editing the staging
area — and it means this failure mode cannot be induced from outside the
process. It remains covered only by unit tests against the lying fake.

**The "uninstall succeeded, install failed" path was never exercised**, for the
same reason: deleting the staged manifest between the two commands did not
survive prepare re-running. So `touched`, the exit-1-for-a-lost-package rule,
and `recover.cmd` surviving a failed run are all still unit-tested only.

**An earlier attempt at both of these looked like a pass and was not.** The
tamper test reported `done … verified on disk` and I nearly recorded it as
proof the swap had been caught, when in fact nothing had been swapped. Written
down because it is the same trap this project has hit twice before: a control
that runs, goes green, and demonstrates nothing.

**Shim behaviour is still unmeasured** in a probe root, for the reason recorded
in the measurement document: a throwaway root has no `apps/scoop`, so scoop
cannot copy `shim.exe`.

## Two facts about Windows worth keeping

**A missing `<root>/shims/scoop.cmd` does not error on Windows.** Measured with
`rustc 1.97.1`:

```
Command::new("C:\definitely\nope\scoop.cmd").arg("download").output()
  -> Ok, status = Some(1), stdout = "", stderr = "The system cannot find the path specified."
Command::new("C:\definitely\nope\scoop.exe")  -> Err(NotFound)
```

Rust runs `.cmd` through `cmd.exe`, so the "cannot run …" path in
`Scoop::download` is unreachable on Windows. `download_verdict` sees empty
stdout and refuses as `Unproven` — **fail-closed holds, by a different route
than on macOS** — and `Mutator::uninstall` returns `Ok`, caught only because the
executor never judges an exit code and asks `verdict` instead. It also means
scoop's "path not found" exit 1 is indistinguishable from its "unknown command"
exit 1, which is one more reason the exit code carries nothing.

**PowerShell 5.1's `Set-Content -Encoding UTF8` writes a BOM**, and `serde_json`
rejects it with `expected value at line 1 column 1`. A BOM'd `manifest.json`
makes an app invisible to `scan()` — with a warning, which is the Phase 2a
behaviour working. This bit the first draft of this dogfood's own scaffolding;
it is not a dotpkg defect, and real scoop-written manifests have no BOM.

## Method, for whoever runs Stage 2

- Run under plain elevated `ssh` for a throwaway root; the medium-integrity
  scheduled-task technique is only needed to read the real `~/scoop`'s
  junctions.
- Quoting through `ssh` still breaks PowerShell. `-EncodedCommand` with
  UTF-16LE base64, output to a file in `$env:TEMP`, `scp` it back.
- A throwaway root needs three things before dotpkg can drive it: a
  `buckets/<name>` (a junction to the real bucket works and stays read-only),
  a `shims/scoop.cmd` copied from the real one (scoop.ps1 honours `$env:SCOOP`),
  and something installed to change.
- Write every file dotpkg parses with `[System.IO.File]::WriteAllText` and a
  BOM-less `UTF8Encoding($false)`.

## Stage 2 — the real `~/scoop`

Run after Stage 1's fixes, against the machine's own scoop, with `%LOCALAPPDATA%`
left at its real value so `state.json` landed where it really lives. Scoped to
**act one only** — downgrade a leaf package, verify, restore. The prune was
deliberately not repeated here: Stage 1 had already exercised it end to end
against a real scoop and a real bucket, and repeating it on real software buys
a window in which a package the user depends on is genuinely absent.

Target: `shfmt` 3.13.1 → 3.13.0 → 3.13.1. Chosen because it is a leaf, small,
idle (`processes matching *shfmt* = NONE`), and neither `git` nor anything with
a live process. No package on this machine declares `depends`, measured
separately.

### Pre-flight, read-only

`dotpkg status` printed exactly one action and twenty-three `? unmanaged -- no
action` lines — the prune fence visible in practice: dotpkg owns nothing, so it
proposes to touch nothing it did not plan.

The architecture came through as `64bit`, which is what `shfmt` is actually
installed as. Preservation, not correction.

Three warnings, on the same three apps as every previous phase:

```
warning: scoop: actionlint: cannot read manifest.json: The path cannot be
  traversed because it contains an untrusted mount point. (os error 448)
warning: scoop: antigravity: ...
warning: scoop: zellij: ...
```

That is the known elevated-`ssh` junction quirk, not a dotpkg defect. Worth
noting for what it demonstrates: dotpkg **warns** rather than silently treating
those three as absent, which is the Phase 2a fix earning its keep on a real
machine. None of the three is declared or owned, so the plan was unaffected.

### Act one

```
before: shfmt = 3.13.1   apps = 31   state = (absent)

  v scoop  shfmt          3.13.1 -> 3.13.0         (downgrade, from lock, 64bit)
  ready   scoop  shfmt        3.13.1 -> 3.13.0  (downgrade, 64bit)
  1 of 1 changes ready, 0 failed, 0 skipped, 0 not locked.
  done    scoop  shfmt        verified on disk
  1 verified on disk, 0 failed, 0 held.            EXIT = 0

after:  shfmt = 3.13.0   apps = 31
state.json = { "scoop": { "shfmt": "installed" } }
shim still present = True
the binary itself reports: v3.13.0
```

**Exactly one `state.json` key**, which is the plan's central claim for this
act. And the check that matters most: `shfmt --version` reports `v3.13.0`, so
the executable on the machine really changed — not merely the manifest dotpkg
compares against.

`recover.cmd` was absent afterwards, correctly: the run had no failures.

### Restore

```
  ^ scoop  shfmt          3.13.0 -> 3.13.1         (upgrade, 64bit)
  done    scoop  shfmt        verified on disk     EXIT = 0
the binary reports: v3.13.1
```

### The machine afterwards

All **31** packages compared against the baseline captured before anything ran:
**no package added, none removed, no version changed.** Cache 76, unchanged.
`kanata` still `kanata_windows_tty_winIOv2_arm64` PID **15732** — never started
or stopped.

One thing did change and is recorded rather than explained away: **`explorer`'s
PID went from 9620 to 15524** during the session. dotpkg touched only a shell
formatter, so this is almost certainly Windows or ordinary desktop activity —
but nothing here establishes that, so it is written down as an observation, not
as a cleared suspicion.

### Cleanup

`%LOCALAPPDATA%\dotpkg` removed entirely, verified absent. That mattered more
than ordinary tidiness: `state.json` said `{"scoop":{"shfmt":"installed"}}`, so
leaving it would have meant dotpkg believed it owned a package it had not
originally installed — and a later `apply` with `shfmt` undeclared would have
pruned it. Both probe roots, both staging trees, every `dotpkg-*.txt`, and the
source tarball removed and re-checked individually.

Kept, matching the precedent of earlier phases: `C:\Users\kln\dotpkg-build` and
`C:\Users\kln\pkg.toml`. No `Dotpkg*` scheduled task remains; `AHKWatchdog` and
`KanataWatchdog` are untouched and `Ready`.

### What Stage 2 deliberately did not cover

The prune on real software, and everything Stage 1 already listed as
un-exercised — `ContentDiffers`, the lost-package path, `touched`, and
`recover.cmd` surviving a failed run. Stage 2 changes none of those; they remain
unit-tested only, for the reason Stage 1 records: `apply` re-runs `prepare`,
which re-derives the staged manifest from the lock, so the tampering needed to
induce them is healed before it can bite.

Also not exercised: the medium-integrity scheduled-task path. Stage 2 ran over
plain elevated `ssh`, which is why the three junction warnings appear. A run at
medium integrity would see all 31 apps; it was not needed here because the three
unreadable packages are neither declared nor owned, and `status` was used as a
pre-flight to confirm the plan was unaffected.
