# dotpkg

Declarative package management for Windows: winget and scoop from one dotfile,
with a real lock file and prune.

## `status`

`dotpkg status` prints the plan it would execute and changes nothing — no
install, no uninstall, no network. It covers **both backends**: scoop from
`apps/*/current/manifest.json` on disk, winget from `winget list`.

```console
$ dotpkg status
  + scoop  ripgrep        14.1.0                   (install)
  v scoop  fzf            0.74.2 -> 0.74.1         (downgrade, from lock)
  ! scoop  kanata         running -- stop it first
  ^ winget Git.Git        2.51.0 -> 2.52.0         (upgrade)
  - scoop  aichat         0.30.0                   (prune, owned)
  ? scoop    1 installed outside dotpkg -- no action
      pass --show-unmanaged to list them

  4 change(s), 1 skipped, 1 unmanaged
```

**Installed-but-unmanaged packages collapse to one line per backend**, for both
backends, because a real machine carries a lot of them: on one measured machine
winget alone accounted for 36, and thirty-six `?` lines bury the handful that say
what will actually happen. `--show-unmanaged` lists each one individually
instead — the same lines earlier versions printed — and the summary carries the
count either way, so nothing on screen goes uncounted. Nothing dotpkg *does*
changes: an unmanaged package is a report, never an action.

It reads `pkg.toml` (what you declared), `pkg.lock` (what those declarations
resolved to), `state.json` (what dotpkg installed, so prune can never reach a
package it did not put there), scoop's own `apps/*/current/manifest.json` on
disk, and `winget list`.

`status` is the one command that never refuses over an *incoherent* lock — one
that parses, but that `apply` would reject. It warns and prints the plan
anyway, because that plan is the information you need in order to fix the lock.
A lock it cannot read at all (malformed TOML, or an I/O error that is not
"missing") still stops `status`, because there is no plan to print.

## `apply`

Brings the machine to what `pkg.toml` and `pkg.lock` describe, for **both
backends**: installs what is missing, changes the version of what disagrees
with the lock, and removes what is no longer declared and that dotpkg owns.
Prints the plan, stages and fetches everything a mutation needs, asks once,
then mutates — verifying every result afterward rather than believing the
package manager's exit code, because neither manager's exit code says whether
anything happened. For scoop that verification reads the installed manifest's
bytes off disk (see
[the exit-code measurements](docs/measurements-2026-08-08-scoop-exit-codes.md));
for winget it re-runs `winget list` for the one package, because winget returns
the same exit code for "already exactly where you asked" and "I declined" (see
[the write-path measurements](docs/measurements-2026-08-10-winget-write-path.md)).

**A version change is not the same operation on the two backends.** scoop
cannot change a version any other way than uninstall-then-install — `install`
over an installed app is a measured no-op — so a scoop version change opens a
window in which the package is absent. A winget version change is one
`install --version <pin>` call and opens no such window. The confirmation
prompt says so, and only when a scoop replacement is actually in the run.

```console
$ dotpkg apply
  + scoop  ripgrep        14.1.0                   (install)
  v scoop  fzf            0.74.2 -> 0.74.1         (downgrade, from lock)

  1 package(s) will be uninstalled and reinstalled, 1 installed, 0 removed. A
  scoop version change is an uninstall followed by an install, in both
  directions. Continue? [y/N] y
```

`--prepare` stops before the question, after staging and fetching, and
changes nothing either way. For a winget package there is nothing to stage —
winget holds the manifest, not dotpkg — so `--prepare` instead confirms with
`winget show` that the pinned version is still in winget's index.

### winget: what `apply` will and will not do

- **It will install, upgrade and remove.** One measured argv each, all through
  one seam, with the pinned version passed on both the install and the removal
  so a version dotpkg did not expect fails closed rather than removing the
  wrong one.
- **It will never downgrade.** Decided, not deferred: measured, `winget
  install --version <older>` cannot do it, and the alternative — uninstall then
  install — would put a nightly uninstall-and-reinstall loop on every
  self-updating application. A package installed *ahead* of its pin is printed
  as the refusal it will be, is not counted among the changes, and tells you to
  run `dotpkg update` to move the pin forward:

  ```console
  ! winget Brave.Brave    151.1.93.134 -> 151.1.93.132 (dotpkg will not downgrade a winget package -- run `dotpkg update`)
  ```

- **It refuses an elevated removal of a user-scope package before anything
  runs.** Measured: winget will not uninstall a user-scope package from an
  elevated process, and it says so with its own exit code. dotpkg checks
  before the run rather than only translating the failure after — and re-running
  without elevation is the fix.
- **A declared winget package with no `pkg.lock` entry fails the whole run**
  (exit 2), exactly as a scoop one does. It used to be a harmless report line;
  that exemption only ever existed because `apply` could not act on winget at
  all. Run `dotpkg update` first.
- **`dotpkg add` still does not exist for either backend.** Declaring a winget
  package is `pkg.toml`, then `dotpkg update <pkg>`, then `dotpkg apply`.

### winget: naming the processes winget will not name

dotpkg will not replace a package whose process is running: `status` reports it as
skipped, `apply`'s plan-time check skips it, and the mid-run re-sampler holds the
step if the process starts after the plan was made. For scoop it works that out on
its own: the
manifest names the executables, and a running binary's path sits under the app
directory. For winget it largely cannot, because winget exposes no way for dotpkg
to discover a package's process names — `winget list` reports neither executables
nor aliases. So dotpkg guesses from the id (its last dotted segment, plus the
display name) and reads the package directory of the `portable` packages that
have one. Measured on one real machine: the guesses caught 3 of 36 installed
winget ids, and only 4 of those 36 are `portable` installs with a package
directory to read at all.

`[winget.guard]` in `pkg.toml` is how you close the rest — it names the processes
a winget id really runs:

```toml
[winget.guard]
"Tailscale.Tailscale"   = ["tailscaled", "tailscale-ipn"]
"AutoHotkey.AutoHotkey" = ["autohotkey64"]
"Microsoft.WSL"         = ["wslservice"]
```

Those three are measured misses on a real machine rather than illustrations: the
ids yield `tailscale`, `autohotkey` and `wsl`, and none of the three is what the
machine is actually running. Names are compared case-insensitively with any
executable suffix removed, so `Tailscaled.EXE` and `tailscaled` are the same
entry, and a name dotpkg already guessed is not doubled. An entry that matches
nothing installed and nothing declared warns once per run, because a stale or
misspelled entry protects nothing and silence about that is worse than the
warning; so does an entry on a package winget reported with no source at all,
since dotpkg cannot establish such a package's state and skips it before any
process check. A `pkg.toml` with no `[winget.guard]` table behaves exactly as
before.

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
- `--show-unmanaged` — List every installed-but-unmanaged package instead of
  collapsing them to one line per backend. Reaches both tables `apply` prints,
  the plan and the preparation report, so the two cannot disagree.
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

Re-resolves `pkg.toml` and rewrites `pkg.lock` — scoop against the buckets on
disk, winget against winget's own index. The only command that asks what is
newest, and the only one that fetches. Never touches the machine: no install,
no uninstall, and the only subprocesses are `git` and the two read-only winget
calls (`winget source update --name winget`, measured to change nothing on the
machine it was run against twice, and `winget show`).

The source refresh is retried **once**, after a second, and only on the one exit
code measured to mean another winget process held the index — measured 0 failures
in 10 calls alone, 3 in 10 with a second winget process alive. A failure there has
always been a warning rather than a run-ending error; the retry exists because
otherwise the run resolves `latest` against an index it failed to refresh and only
warns about the refresh.

For winget, `pkg.lock`'s key is the **canonical id winget itself echoed back**,
not the spelling in `pkg.toml`. If the two differ in case, `update` warns and
leaves `pkg.toml` as you wrote it — that spelling is yours, and only the lock's
goes on a winget command line.

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
installing, removing, or otherwise touching anything either backend has on the
machine. For scoop it finds the commit whose manifest is the one actually
running — matched by exact content across the bucket's whole history where
possible, by version only where it is not. For winget (`--backend winget`) it
pins the installed version. Either way it writes `pkg.lock`, `pkg.toml` and
`state.json`. There is deliberately no "adopt everything": at least one package
must be named.

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

- `--backend <scoop|winget>` — Which backend to adopt from. Defaults to
  `scoop`, unchanged from before winget adoption existed.
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
  already on the machine. `pkg.toml` plus `dotpkg update <pkg>` plus `dotpkg
  apply` composes to the same thing today, for either backend.
- **Downgrading a winget package.** Decided against, not deferred — see the
  `apply` section above.
- **Dependency handling.** A winget manifest can declare dependencies, and
  five of twelve packages surveyed on a real machine declare
  `Microsoft.VCRedist.2015+.x64`. An install that also installs a second
  package leaves that package with no lock entry, no ownership record and no
  declaration, so the next `status` counts it among that backend's unmanaged
  packages. Still a real gap — but a dependency-aware rule was measured and
  rejected as the fix, not merely deferred: those VCRedist rows do carry
  `Source: winget`, so they are reported rather than lost, and on a machine
  declaring no winget packages such a rule would have suppressed none of the 36
  unmanaged lines. See
  [carried forward out of Phase 5](docs/phase5-notes.md).
- **Chocolatey.** Nothing beyond the two backends.

## Documentation

- [Design](docs/specs/2026-08-08-design.md) — why a lock file, why scoop and
  winget pin to different things, and what dotpkg refuses to do.
- [Phase 2b-2 design](docs/specs/2026-08-08-phase2b2-executor-design.md) — the
  `apply` executor: the confirmation prompt, the state write path, and why
  scoop's exit code cannot be trusted.
- [Scoop exit-code measurements](docs/measurements-2026-08-08-scoop-exit-codes.md)
  — the raw commands and output behind that design.
- [winget backend design](docs/specs/2026-08-09-phase4-backend-winget-design.md)
  — how winget is scanned, why `pkg.lock` holds the canonical id winget echoed
  back, and why `winget export` was measured and rejected.
- [winget executor design](docs/specs/2026-08-10-phase4b-winget-executor-design.md)
  — how `apply` installs, upgrades and removes a winget package, and why it
  never downgrades one.
- [winget write-path measurements](docs/measurements-2026-08-10-winget-write-path.md)
  — the 27 write-verb invocations that design rests on, exit codes and stdout
  included.
- [Phase 1 plan](docs/plans/2026-08-08-phase1-status-scoop.md) — the task
  breakdown the first phase was built from.
- [Dogfood notes](docs/dogfood-2026-08-08.md) — the first run against a real
  machine.
- [Carried forward out of Phase 4](docs/phase4-notes.md),
  [out of Phase 4b](docs/phase4b-notes.md) and
  [out of Phase 5](docs/phase5-notes.md) — what each phase measured, what it
  only reasoned about, and what it left open.
- [Phase 5 measurements](docs/measurements-2026-08-11-phase5-guard-unmanaged-retry.md)
  — the running-process fence, the `Unmanaged` flood, and whether winget has a
  transient failure at all.

## Build

```console
cargo build --release
cargo test --all
```

Requires Rust 1.85 or newer.
