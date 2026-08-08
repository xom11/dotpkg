# dotpkg Phase 2b-1 — `apply --prepare`

**Status:** design approved 2026-08-08, not yet implemented.
**Depends on:** Phase 2a (`status`, made truthful), merged at `131414f`.
**Carries:** `docs/phase2b-notes.md`, which this phase closes the top half of.

Everything `apply` must do up to, but not including, the first destructive act:
close the holes that make an executing plan unsafe, recover each locked
manifest from its bucket commit, fetch and hash-verify every artifact, and stop.

## Why this is its own phase

Phase 2b as the approved design scopes it is about twelve items, and the first
real run of it on a populated machine would also be the first run in which the
tool can remove software. The 2a split — cutting exactly at the read/write
boundary — worked, and the same cut is available here one level in: between
"prepared everything, changed nothing" and "changed something".

The payoff is the same and larger. `apply --prepare` can be run against a14's
**real** `~/scoop` with nothing at risk, and doing so exercises the entire
pinning claim — commit lookup, `git show` recovery, hash verification against
real upstream URLs, across all twenty-five declared packages — before the
binary is capable of uninstalling anything.

## What was measured, and what it changes

Measured on a14, scoop 0.5.3, 2026-08-08. Full record in
`docs/phase2b-notes.md`.

**Every version change is uninstall + install.** There is no force flag;
installing a different pinned manifest over an app exits 0 and silently does
nothing; `scoop update` rejects a manifest path; `scoop reset` can only relink
a version already on disk, which our own uninstall has just deleted. The
approved design's claim that downgrades are the one irreducible gap is wrong —
`^` carries the same risk as `↓`.

That is what makes this phase worth having. If every change is a removal
followed by a restore, then everything that can be checked *before* the removal
should be.

**`scoop download <path>/app.json` exists, takes a manifest path, and verifies
hashes.** This is the mechanism the approved design does not mention and the
reason `--prepare` is a real phase rather than a rehearsal. A dead upstream
URL, a hash mismatch, a network failure, a commit missing from the bucket — all
of them become "nothing happened, here is why" instead of "the app is gone".

Because the prefetch runs for every package before any package is mutated, that
is a whole-run guarantee, not a per-package one.

**Two scoop commands return success while doing nothing.** `scoop install` on
an already-installed app, and `scoop reset` to a version not on disk, both exit
0 silently. An executor that trusts exit codes will report work it did not do,
so 2b-2's rule is **verify the resulting state, never the exit code alone**.

For 2b-1 the honest position is narrower. `scoop download` was *not* measured
for silent-success behaviour — only `install` and `reset` were — so its exit
code is the signal this phase has, and the design does not pretend otherwise.
Inferring a cache path to stat would be inventing a check against an
unmeasured assumption. What closes the gap is 2b-2's install step, which
verifies the resulting version on disk; if a download silently no-ops, that is
where it surfaces. Measuring `scoop download`'s failure modes directly is a
cheap addition to 2b-2's own probe work.

**`install.json` records `url`, not `bucket`, when installing from a path.** The
recorded `url` is dotpkg's own staging path, so staging goes somewhere stable
and permanent, never `%TEMP%`.

### Invocation facts, measured so nobody has to guess

| Need | Answer |
|---|---|
| scoop entry point for `Command::new` | `<scoop root>/shims/scoop.cmd` — verified non-interactive, exit 0. Not `scoop.ps1` (Rust cannot exec it) and not bare `scoop` from `PATH` |
| git | `git` resolves on `PATH`; 2.55.0 on a14 |
| bucket layout | `<scoop root>/buckets/<name>/`, each a git repo, manifests under `bucket/<app>.json` |

**`git` is itself a scoop-managed package on this machine**
(`apps/git/current/cmd/git.exe`), so the tool dotpkg needs in order to stage a
manifest is one dotpkg manages. Prepare stages everything before 2b-2 mutates
anything, so the ordering already protects this — but it is written down here
because it is exactly the kind of self-reference that bites the person who
reorders the phases later. `scoop` itself is already excluded from the scan.

## The prepare phase

For every action in the plan that needs an artifact — `Install`, `Upgrade`,
`Downgrade` — in two steps, neither of which touches installed software.

### 1. Recover the pinned manifest

```
git -C <root>/buckets/<bucket> show <commit>:bucket/<app>.json
```

- The pin must be a `Pin::ScoopCommit`. A `WingetVersion` in the scoop map is
  the type-level asymmetry `docs/phase2-notes.md` records; here it is an error,
  not a panic.
- **A commit the bucket does not have is a hard failure for that package.** It
  must never fall back to the bucket's current manifest. This is one of the two
  mandatory tests the approved design names, and it gets a negative control.
- **Filename casing is git's problem, not ours.** `Name` folds case; git object
  paths do not. `pkg.toml` saying `FZF` against a bucket file `fzf.json` makes
  `git show` fail. So: try the name as written, then the folded form. If both
  miss, spend one `git ls-tree --name-only <commit> bucket/` to find the real
  spelling and put it in the error — `bucket main at a28d0c56 has
  bucket/FzF.json, not bucket/FZF.json` is a message the user can act on,
  where "file not found" is not.
- The recovered manifest must parse as JSON and its `version` must equal the
  lock's. A mismatch means the lock is internally inconsistent — fail that
  package rather than install a version nobody asked for.

### 2. Stage it where scoop can install from it

```
%LOCALAPPDATA%\dotpkg\manifests\<app>\<version>\<app>.json
```

Three constraints, all measured rather than assumed:

- **The filename determines the installed app name.** Staging as
  `dotpkg-tmp-9f2.json` installs an app called `dotpkg-tmp-9f2`. Each manifest
  therefore gets its own directory so the file itself stays `<app>.json`.
- **Use the bucket's own spelling** for that filename, which step 1 already
  had to discover. Then the app directory scoop creates is byte-identical to
  the one a plain `scoop install <app>` would create, rather than inheriting
  whatever case the user typed in `pkg.toml`.
- **The location is permanent**, because `install.json` will record it. A
  staging directory that gets cleaned leaves the installed app pointing at a
  path that no longer exists.

### 3. Fetch and verify

```
<root>/shims/scoop.cmd download <staged manifest>
```

Never with `--skip-hash-check`. The approved design forbids it and this is the
one place it would be tempting.

A non-zero exit, or a hash failure, means that package is not ready. It is
reported and the run continues with the others; nothing has been changed for
any of them.

## The three holes that must close first

From `docs/phase2b-notes.md`, all found by review of Phase 2a and all safe
today only because `status` acts on nothing.

**Two declared names differing only in case produce two `Install` actions.**
`packages = ["fzf", "FZF"]` folds to one entry in the declared set but iterates
the `Vec` twice. Rejected at parse time, naming both spellings.

**A duplicated `[scoop.opts]` key is swallowed.** `python` and `Python` in the
same table yield one entry carrying the first key and the **last** value, with
no error. TOML cannot express a literal duplicate key, so serde never sees a
collision — the collision is created by `Name`'s folding. Fix: deserialize the
table with raw `String` keys, then fold into `Name` keys and reject a collision.
Same treatment for the `packages` list above.

**`Scoop::discover()` does not canonicalise its root.** With `$SCOOP` reached
through a junction, a `subst` drive, or an 8.3 path, `scan()` still works — it
opens through the alias — but `sysinfo` reports resolved paths, so path
matching prefix-compares against the wrong string and silently returns nothing.
`nodejs` and `rustup` have no other running signal, so in 2b-2 they become
prunable while running.

Canonicalise when the path exists, keeping the given path when it does not (a
machine with no scoop is a valid state). On Windows `canonicalize` returns an
extended-length `\\?\C:\…` path, which would break the very comparison it is
meant to fix, so the `\\?\` prefix is stripped.

## `NotLocked` is a failure; `Running` is a skip

Both are `Action::Skip` in the plan, and `status` prints both as `!`. `apply`
must treat them differently, and the distinction is worth stating because it is
easy to collapse:

- **`Running`** — the user can close the app and run again. A benign skip. It
  does not make the run fail.
- **`NotLocked`** — declared in `pkg.toml` with no `pkg.lock` entry. There is
  nothing to prepare and nothing `apply` may do about it, because resolving a
  version itself is forbidden. It is a **failure** for that package: reported,
  counted, and the run exits non-zero. The message names `dotpkg update`.

The planner is unchanged. This is the apply driver's reading of the plan.

## The mass-prune guard

An empty or truncated `pkg.toml` parses successfully to zero packages, and
every owned package becomes a prune. Verified against the merged planner: five
owned packages, empty config, five prunes, no signal.

**A config declaring zero packages for a backend while `state.json` owns at
least one for that backend is a hard error**, before anything else happens. It
names the count and the override.

Two deliberate choices:

- **`--yes` does not bypass it.** `--yes` means "I have read the plan"; the
  empty-config case is file corruption, so the plan itself is the thing that
  cannot be trusted. Overriding takes its own explicit flag,
  `--allow-empty-config`.
- **No ratio or count threshold.** A user who genuinely deletes half their
  `pkg.toml` gets shown the plan and asked, which is the protection that
  already exists. Adding a second threshold buys little and blocks legitimate
  edits.

## Output

```
$ dotpkg apply --prepare
  ready   scoop  ripgrep      14.1.0            (install)
  ready   scoop  bat          0.25.0 -> 0.26.1  (upgrade)
  FAILED  scoop  fzf          commit a28d0c56 is not in bucket main
  FAILED  scoop  neovim       download failed: hash mismatch
  !       scoop  kanata       running -- stop it first
  !       scoop  zellij       no lock entry -- run `dotpkg update`

  2 of 4 changes ready, 2 failed, 1 skipped, 1 not locked.
  Nothing has been changed.
```

The last line is the promise of the phase and is printed unconditionally.
Exit code is non-zero if anything is `FAILED` or not locked.

## Testability, and where it stops

The split matters because one half can be tested honestly on any OS and the
other cannot, and conflating them would let a mock look like proof.

**The git side is fully testable on macOS and Linux.** A fixture is a real
local git repository with `bucket/<app>.json` committed twice at two versions.
Everything in step 1 — commit recovery, a missing commit, a case-mismatched
filename, a version that disagrees with the lock — is exercised for real
against real git. This is where the approved design's mandatory test "a lock
pointing at a nonexistent commit fails and does not install latest" lives, with
its negative control.

**The scoop side is not.** `scoop download` needs scoop. Rather than mock it,
the argv construction is a pure function and is tested directly:

```rust
pub fn download_argv(manifest: &Path) -> Vec<String>
```

What that test can honestly prove is that the argv names the staged path and
**never contains `--skip-hash-check`**. What scoop then does with it was
measured, not asserted, and is covered by the Windows dogfood. Saying so here
is the point: a mock that "verifies the download" would be claiming something
no test in this repository can establish.

## What 2b-1 deliberately leaves out

Each is listed so its absence is a decision, and each belongs to 2b-2:

- `scoop uninstall` and `scoop install` — every actual mutation.
- Post-mutation state verification, which the measurement made mandatory after
  *every* operation rather than only after uninstall.
- The `state.json` write path.
- The confirmation prompt and `--yes`.
- Per-package failure accumulation across mutations (prepare accumulates its
  own failures; the executor will need its own).
- Cloning a missing bucket. Prepare reports the bucket as missing and names it;
  `scoop bucket add <name> [<repo>]` is the 2b-2 answer.
- `--fix-arch`. Architecture drift stays reported and unacted-on, as in 2a.
  Roughly seven packages on a14 are emulated, and each fix is now known to be a
  full uninstall + install — seven windows is not something a default `apply`
  should open.
- The diagnostic for a scoop root that no live process resolves into. The
  canonicalisation above addresses the cause; the warning is a net for cases it
  misses, and is speculative until one is seen.

## Testing

Layers 1 and 2 from the approved design, unchanged: everything below runs on
Linux and macOS.

New fixture kind: **a real local git repository**, built in a `tempfile` dir,
with a bucket manifest committed at two versions. This is not a mock of git —
it is git.

Mandatory, each with a negative control whose red output is recorded:

1. A lock naming a commit the bucket does not contain fails, and **no install
   is attempted** — the approved design's second mandatory test.
2. A staged manifest's filename is the bucket's spelling, not `pkg.toml`'s.
3. A recovered manifest whose `version` disagrees with the lock fails.
4. `download_argv` never contains `--skip-hash-check`.
5. A duplicate declared name, in either the list or the opts table, is rejected
   at parse time naming both spellings.
6. The mass-prune guard fires on an empty config with owned packages, and is
   not bypassed by `--yes`.

## Dogfood

On a14, against the **real** `~/scoop`. Writes only to dotpkg's staging
directory and scoop's download cache; installs, upgrades and removes nothing.

It needs a `pkg.lock` with real bucket commits, which no command yet produces —
`update` is Phase 3. Generating one by script from the currently installed
versions is the rehearsal for that command, and the script belongs in the
dogfood record rather than in the crate.

Framed so it can fail:

1. Do all twenty-five declared packages recover their manifests?
2. Does every recovered manifest's version match the lock?
3. Do the downloads verify?
4. Does a deliberately corrupted lock entry — a commit that does not exist —
   fail loudly and install nothing?

**A prediction worth recording before the run: some packages will fail because
their upstream URL is gone.** Scoop manifests pin a URL and a hash, and old
releases are deleted. If that happens it is not a bug — it is the phase doing
the job it exists for, catching upstream rot before an uninstall rather than
after. If *nothing* fails, that is worth being suspicious about.

Run at medium integrity via the scheduled-task technique, using the XML-clone
workaround `docs/phase2b-notes.md` records: building a principal with
`New-ScheduledTaskPrincipal -LogonType Interactive -RunLevel Limited` leaves the
task stuck at Queued on this machine.

## Non-goals

Unchanged from the approved design. Additionally, 2b-1 does not reach the
network except through `scoop download`, does not write `pkg.lock`, and spawns
no subprocess other than `git` and `scoop download` — the approved design's
"subprocesses are for mutation only" rule was written to stop the tool
shelling out to *ask questions*, which is slow and which `scan()` avoids by
reading disk. Fetching an artifact is neither a question nor a mutation, and
the rule's purpose is preserved.
