# Carried forward into Phase 2

Findings from building Phase 1 that Phase 2 (`apply`) must handle. Every one was
found by review or by the dogfood run, not guessed. Recorded here because the
build ledger they came from is scratch and gets deleted.

## Must be designed for before `apply` executes anything

**Name matching is case-sensitive; scoop app names are not.** `src/plan.rs`
compares `String`s exactly. Scoop treats app names case-insensitively on Windows,
so `pkg.toml` saying `FZF` while `state.json` says `fzf` yields
`Install{FZF}` + `Prune{fzf}` — the same app. Combined with install-before-prune,
the prune runs last and removes what was just installed. Harmless in Phase 1
because nothing executes. This is not "not fixable in the planner" — it is a
planner-local change, testable on macOS.

**An empty or truncated `pkg.toml` produces a maximally destructive plan.**
`Config` is all `#[serde(default)]`, so an empty file parses successfully to zero
packages and every owned package becomes a `Prune`, with no signal that anything
is wrong. The fix belongs at apply time — a guard against mass prune — not in the
parser: on a fresh machine, `status` with an empty config *should* list
everything as unmanaged. `deny_unknown_fields` already catches the typo case; it
does not catch truncation.

**The running-process check compares package names to process names.**
`src/sys.rs` returns process base names (`nvim`), `src/plan.rs` matches them
against scoop package names (`neovim`). They differ for real packages, so the
"skip what is running" guard silently does nothing for them — verified: a
`neovim` upgrade planned cleanly while `nvim.exe` was running. The spec's own
example (`kanata`) happens to match, which is why nine task reviews missed it.
The data needed is already there: scoop manifests carry a `bin` field, and
`scan()` currently discards it. In Phase 2 this stops being a missing `!` line
and becomes upgrading a running application — the exact scenario the spec's
error-handling table exists to prevent.

> **Correction, 2026-08-08.** The sentence about `kanata` matching is wrong, and
> it is wrong in the dangerous direction. Measured from the real manifest on
> a14: `kanata` declares no top-level `bin`, and its architecture branches name
> `kanata_windows_tty_winIOv2_arm64.exe` with only the *shim alias* called
> `Kanata`. Whether the current comparison catches a running kanata depends on
> how it was launched — through the shim it matches, from the Start Menu
> shortcut (a GUI variant absent from `bin`) or a scheduled task it does not.
> Two further gaps found at the same time: `Prune` never consults `running` at
> all, and `nodejs` / `rustup` name no executable in their manifests, so `bin`
> alone cannot cover them. See `docs/specs/2026-08-08-phase2a-design.md`.

**Architecture drift is parsed but unhandled.** `PkgOpts.arch` is read from
`pkg.toml` and ignored by the planner. Correct for a read-only phase, since
acting on drift means a reinstall. The Phase 2 plan should open with it.

## Smaller, real, and cheap

- ~~`Scoop::scan`'s error handling is narrowed so only `NotFound` is swallowed, but
  only the *parse* branch is tested. Reverting the *read* branch to swallow every
  I/O error leaves all scoop tests green. A portable fixture: make `manifest.json`
  a directory, which produces a non-NotFound `Err`.~~ **Fixed in Phase 2a.** The
  directory fixture is exactly what was used; reverting the read branch no longer
  leaves the suite green.
- ~~`entries.flatten()` in the same function still swallows `read_dir` iteration
  errors — same class of bug, four lines away.~~ **Fixed in Phase 2a**, but with
  no test: producing a failing `read_dir` iteration needs a directory entry that
  cannot be stat'd, which is not portable across macOS, Linux and Windows. Verified
  by inspection only, and recorded as such rather than counted as covered.
- The *JSON parse* branch of the same function still has no test. The test that
  reads as covering it actually covers the missing-`version` branch.
- The planner purity guard is an allowlist over `use` lines plus a ban on
  fully-qualified `std::` paths. Stronger than the denylist it replaced, but still
  not sound: `crate::` is allowed wholesale, and `crate::sys` / `crate::config` do
  I/O, so `use crate::sys; sys::running_processes()` passes. A third-party
  crate written full-path (`sysinfo::System::new_all()`) also passes. Treat it as
  a tripwire for accidents, not a proof.
- The guard matches text, so prose containing `std::fs` inside `src/plan.rs` fails
  the test. A comment explaining why the file avoids `std::fs` cannot be written
  there.
- `Lock.scoop` is `BTreeMap<String, Pin>`, so the type system permits a
  `Pin::WingetVersion` in the scoop map. `parse()` never does it and a test would
  catch it, but Phase 3's `update` becomes a second writer. Separate types per map
  would make the asymmetry unbreakable at the type level rather than by convention.
- No test covers the warning-printing block in `src/main.rs`.
- `State::default_path()` falls back to `"."` when `LOCALAPPDATA`, `XDG_STATE_HOME`
  and `HOME` are all unset, putting state in the current directory. No safety
  consequence — a misplaced state file reads as empty, never as a false "owns" —
  but it varies by working directory.
- Repo ships no example `pkg.toml` / `pkg.lock`, so `dotpkg status` in a fresh
  clone only prints a file-read error.

## Settled, do not re-investigate

**Packages vanishing from the scan was an SSH artifact, not a bug.** The first
dogfood run showed 3 of 30 apps missing with "untrusted mount point". That was
caused by reading over an *elevated* SSH session (High IL, no UAC) crossing
junctions owned by the plain user. A second run at Medium Integrity, via a
scheduled task with `LogonType Interactive` / `RunLevel Limited`, saw all 30
including `antigravity`. Windows gates that protection on the reading process's
integrity level. A normal user never hits it.

What survives from that episode is narrower and still true: `scan()` could not
distinguish "half-finished install" from "permission denied". That is now fixed
for the parse branch (see above for the untested read branch).

**`is_older()` is deliberately not semver.** Scoop ships versions semver rejects
(`26.01`, `2026.07.15.08.55`). Today its result only picks which arrow the plan
displays, never whether a change happens — the change is already decided by an
inequality before it is called. The moment Phase 2 gates on the variant (for
example, refusing a downgrade without `--force`), every edge case in it becomes
load-bearing: differing component counts, non-numeric versions, prerelease
suffixes, and a component overflowing `u64` (silently dropped today).
