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

Defined by what the operator must do next, not by what happened internally.

- **0** — the plan is fully realised on disk and nothing is outstanding.
- **1** — something is outstanding: a package failed, was held (by the
  running re-sampler, or because another package in the run could not be
  prepared), could not be prepared at all, or was skipped because its own
  process was running. That last one is not a failure and never gates a
  removal or refuses the run — the user can close the app and rerun — but it
  is still outstanding: the machine may or may not have actually changed.
- **2** — refused before anything was attempted; nothing changed. A guard
  fired, the user said no, or no answer was available.

## `update`

Re-resolves `pkg.toml` against the buckets on disk and rewrites `pkg.lock`.
The only command that asks what is newest, and the only one that fetches.
Never touches the machine — no install, no uninstall, no subprocess besides
`git`.

```console
$ dotpkg update
  + scoop  ripgrep        14.1.0                     (new pin)
  ^ scoop  fzf            0.74.1 -> 0.74.2           (version changed)

  2 changed, 23 unchanged, 0 could not be resolved.
```

Which declared bucket a package comes from is decided once and then pinned:
an existing `pkg.lock` entry wins over everything (so `update` never silently
moves a package to a different bucket), then `[scoop.opts] <pkg> = { bucket =
"..." }`, then a search of every declared bucket — ambiguous if more than one
carries it. A package whose bucket cannot be resolved at all (ambiguous, the
named bucket is not declared, a declared bucket is not on disk, or nothing
declared has it) never loses a pin it already had — a package re-resolving
for the first time simply gains none — and the diff names the reason either
way.

`dotpkg update <pkg>...` re-resolves only the packages named — nothing else
is rewritten and no entry is dropped, unlike a bare `dotpkg update`, which
also drops the pin for anything no longer declared.

### Flags

- `--offline` — Do not fetch. `latest` then means whatever this machine last
  pulled, and the output says so.
- `--config <path>`, `--lock <path>` — same as `status` and `apply`.

### Exit codes

- **0** — `pkg.lock` was rewritten (or was already current, and so not
  rewritten) and every package in scope resolved.
- **1** — either `pkg.lock` could not be written because of a plain I/O
  failure, or it was written (or needed no rewrite) but at least one package
  could not be re-resolved; the diff names which and why.
- **2** — refused before `pkg.lock` was touched: a package named on the
  command line is not declared in `pkg.toml`, or the write was refused
  because a carried-forward entry would make `apply` reject the resulting
  lock — named, with per-entry repair advice, since `update`'s own advice
  cannot be "run `dotpkg update`" when that is the command that just failed.

## `adopt`

Brings an already-installed package under dotpkg's management, without
installing, removing, or otherwise touching anything scoop has on disk.
Finds the commit whose manifest is the one actually running — matched by
exact content across the bucket's whole history where possible, by version
only where it is not — and writes `pkg.lock`, `pkg.toml` and `state.json`.
There is deliberately no "adopt everything": at least one package must be
named.

```console
$ dotpkg adopt aichat
  + scoop  aichat         adopted (the installed manifest matches the bucket exactly)

  1 adopted, 0 refused. Nothing installed and nothing removed.
```

Each named package is independent: one that cannot be adopted (not
installed, already managed, ambiguous or absent bucket, no matching commit)
is refused and reported by name, and the rest still proceed. A write that
stops part way through a package (`pkg.lock` written, `pkg.toml` write then
fails, say) is reported as what really changed on disk versus what did not
— the packages after it are not attempted.

### Flags

- `--state <path>` — Where dotpkg records what it owns. Must be an absolute
  path if given.
- `--config <path>`, `--lock <path>` — same as `status` and `apply`.

### Exit codes

- **0** — every named package was adopted.
- **1** — at least one named package was refused, or a write stopped part
  way through (whatever it did write stays written — this is not undone).
- **2** — refused before anything was attempted: `--state` did not resolve
  to an absolute path, or resolved to a directory rather than the state file
  itself.

## Not built yet

- **`add`.** Install a new package, add it to `pkg.toml`, and record it in
  `pkg.lock` — the other direction from `adopt`, which is for a package
  already on the machine.
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
