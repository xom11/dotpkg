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

## Not built yet

- **`apply`** — the executor. Nothing in this repo installs or removes anything.
- **`update` and `adopt`** — resolving bucket commits, and taking ownership of
  packages already on the machine.
- **The winget backend.** `pkg.toml` and `pkg.lock` accept `[winget]` and the
  planner reports every package declared there, but it cannot scan or act on
  them. They print as skipped, not as nothing.

## Documentation

- [Design](docs/specs/2026-08-08-design.md) — why a lock file, why scoop and
  winget pin to different things, and what dotpkg refuses to do.
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
