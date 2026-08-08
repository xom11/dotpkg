# dotpkg

Declarative package management for Windows: winget and scoop from one dotfile,
with a real lock file and prune.

## Status

Phase 1: `dotpkg status` works, for scoop, and it is **read-only**. It prints
the plan it would execute and changes nothing — no install, no uninstall, no
subprocess, no network.

```console
$ dotpkg status
  + scoop  ripgrep        14.1.0                   (install)
  v scoop  fzf            0.74.2 -> 0.74.1         (downgrade, from lock)
  ! scoop  kanata         running -- stop it first
  ! winget Git.Git        winget backend not implemented until phase 4
  - scoop  aichat         0.30.0                   (prune, owned)
  ? scoop  antigravity    2.0.6                    (unmanaged -- no action)

  3 change(s), 2 skipped
```

It reads `pkg.toml` (what you declared), `pkg.lock` (what those declarations
resolved to), `state.json` (what dotpkg installed, so prune can never reach a
package it did not put there), and scoop's own `apps/*/current/manifest.json`
on disk.

## `apply`

Brings the machine to what `pkg.toml` and `pkg.lock` describe: installs what
is missing, replaces a version change (always uninstall + install, in either
direction), and removes what is no longer declared and that dotpkg owns.
Prints the plan, stages and fetches everything a mutation needs, asks once,
then mutates — verifying every result against the filesystem afterward,
because scoop's own exit code cannot be trusted to say whether anything
actually happened (see
[the exit-code measurements](docs/measurements-2026-08-08-scoop-exit-codes.md)).

```console
$ dotpkg apply
  + scoop  ripgrep        14.1.0                   (install)
  v scoop  fzf            0.74.2 -> 0.74.1         (downgrade, from lock)

  1 package(s) will be uninstalled and reinstalled, 1 installed, 0 removed.
  Every version change is an uninstall followed by an install, in both
  directions. Continue? [y/N] y
```

`--prepare` stops before the question, after staging and fetching, and
changes nothing either way.

### Flags

- `--yes` — Skip the confirmation prompt. Answers that one question and
  nothing else: it does not authorise a prune (pass `--allow-prune` for that)
  and does not bypass any other guard.
- `--allow-prune` — Required, in addition to `--yes`, for an unattended run
  that removes anything. Answering the confirmation prompt by hand still
  authorises a prune on its own — this only gates the `--yes` fast path,
  which is the cheapest answer to one surviving declared package disarming
  the mass-prune guard while everything else it owned gets pruned.
- `--keep-going` — Install what is ready even though some packages could not
  be prepared. Removals stay held regardless; this flag never opens that
  gate.
- `--clone-missing-buckets` — Clone every bucket `pkg.toml` declares that is
  not already on disk, before staging begins.
- `--state <path>` — Where dotpkg records what it owns. Defaults to the
  platform state directory. Must be an absolute path if given.
- `--prepare` — Stage and fetch everything the plan needs, then stop before
  changing anything.
- `--allow-empty-config` — Proceed even though `pkg.toml` declares nothing
  while dotpkg owns packages. Only pass this if the empty file is deliberate.
- `--config <path>`, `--lock <path>` — same as `status`, default `pkg.toml`
  and `pkg.lock`.

### Exit codes

- **0** — every planned action verified on disk.
- **1** — something changed and something failed. Mixed state; go look.
- **2** — refused, and nothing changed. A guard fired, the user said no, or
  no answer was available.

## Not built yet

- **`update` and `adopt`** — resolving bucket commits, and taking ownership of
  packages already on the machine.
- **The winget backend.** `pkg.toml` and `pkg.lock` accept `[winget]` and the
  planner reports every package declared there, but it cannot scan or act on
  them. They print as skipped, not as nothing.

## Documentation

- [Design](docs/specs/2026-08-08-design.md) — why a lock file, why scoop and
  winget pin to different things, and what dotpkg refuses to do.
- [Phase 2b-2 design](docs/specs/2026-08-08-phase2b2-executor-design.md) — the
  `apply` executor: the confirmation prompt, the state write path, and why
  scoop's exit code cannot be trusted.
- [Scoop exit-code measurements](docs/measurements-2026-08-08-scoop-exit-codes.md)
  — the raw commands and output behind that design.
- [Phase 1 plan](docs/plans/2026-08-08-phase1-status-scoop.md) — the task
  breakdown this phase was built from.
- [Dogfood notes](docs/dogfood-2026-08-08.md) — the first run against a real
  machine.

## Build

```console
cargo build --release
cargo test --all
```

Requires Rust 1.85 or newer.
