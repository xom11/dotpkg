# Dogfood: Phase 2b-2, the executor — a14, 2026-08-09

Stage 1 of two. Run against a throwaway `$env:SCOOP` root on a14, driven by a
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

## Stage 2 — not run

Stage 2 is the real `~/scoop`. It has not been run, and nothing in this document
should be read as covering it.
