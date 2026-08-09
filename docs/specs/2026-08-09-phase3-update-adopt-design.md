# dotpkg Phase 3 — `update` and `adopt`

**Status:** design proposed 2026-08-09, not yet implemented.
**Depends on:** Phase 2b-2 (the executor), with `main` at `c13026e`.
**Carries:** the one item left open by `docs/phase2b-notes.md`, "Carried into
Phase 2b-2" — `download` is not behind `Mutator`, so no `Outcome::ReadyToFetch`
in the suite was ever produced by production code. This phase closes it first.

`update` re-resolves what `pkg.toml` declares against the buckets on disk and
rewrites `pkg.lock`. `adopt` brings a package that is already installed under
management. After this phase the reproducibility claim is finally true by
running dotpkg, rather than by running a PowerShell script from a dogfood
appendix.

## The measurement that sets the definition of `commit`

Measured 2026-08-09 against real git, on fabricated repositories with
constructed ground truth. Full record, including the first run that had to be
thrown away and the two sections that were contaminated or broken:
[`docs/measurements-2026-08-09-git-resolution.md`](../measurements-2026-08-09-git-resolution.md).

Three independent scenarios converge on one fact:

- A **merge** that resolves a manifest to one parent's content leaves the other
  parent's version invisible to `git log -- <path>`. `--full-history` finds
  it — and then returns the *merge* commit for the version that is on HEAD,
  not the commit that produced it.
- A **rename** commit carries the previous version's content under the new
  path, so the walk lands on the rename rather than on the authoring commit.
- Two commits carrying the **same version** with different content are both
  legitimate answers to "which commit has version V".

So the claim "`commit` is the commit that produced this version" is not
provable from a bucket, and nothing in dotpkg ever needed it:

> **`commit` is a commit at which `bucket/<app>.json` has the pinned content.**
> It is not a claim about which commit authored that version.

`Scoop::stage` already relies only on the weaker statement — it runs `git show
<commit>:bucket/<app>.json` and checks the `version` field. The stronger claim
was never enforced and never used. Writing the weaker one down is what makes
`update` and `adopt` able to use different `git log` flags without one of them
being wrong.

Two further measured results drive the algorithms below:

- **Matching a version alone pins a machine to content it is not running.**
  Given two same-version commits, version matching selects the newer; matching
  the installed manifest's bytes selects the one actually installed. The 2b-1
  rehearsal script matched on version only.
- **Byte matching requires `verify::normalise`.** scoop rewrites line endings
  when it copies a manifest into `apps/<app>/current`, so a raw comparison
  against a bucket blob matches nothing. This is the same fact that nearly made
  every successful install in Phase 2b-2 report as a failure.

## Corrections to the approved design

Recorded here rather than edited in place, matching the precedent set by
`docs/specs/2026-08-08-phase2a-design.md` and continued by the 2b-2 design.

**`design.md:174` and `design.md:176` contradict each other, two rows apart.**
The command table states `update` is *"**The only command that writes the
lock.**"* and then, two rows below, that `add` *"Install, add to `pkg.toml`,
**record in `pkg.lock`**."* The rule was already false in the approved design;
`adopt` does not break it.

The rule was also protecting the wrong property. What makes `apply`
deterministic is that it never asks a network what is newest — not that one
command owns a file. `adopt` reaches no network at all: it reads `git log` and
`git show` in a bucket already on disk. The honest rule, which this phase
adopts:

> **`apply` never writes the lock, and no command resolves "latest" on
> `apply`'s behalf.**

**`design.md:176`'s `add` is `adopt` plus `update` plus `apply`.** It is not
implemented in this phase and, on the reading above, probably should not exist
as a third resolver. Recorded as a question for Phase 5 rather than settled
here.

**`design.md:189` — "`update` drops the entry" — is right, and one step from a
defect.** Dropping an entry for a package removed from `pkg.toml` is correct.
Dropping an entry for a package that merely *failed to re-resolve* would turn a
working pin into `Skip{NotLocked}`, which makes the next `apply` refuse the
whole run. See "A failed re-resolve keeps the previous entry" below.

**`src/lock.rs`'s parse test enshrines a commit the shipped guards reject.**
`parses_both_backends_into_distinct_pin_shapes` uses `commit = "a28d0c5648f1"`,
twelve hex characters. `lock::parse` accepts it — there is no hex check
there — and `lock_coherence_guard` then refuses the whole run. The split is
deliberate and stays: a lock too broken to run must still be *readable*, or
`status` could not explain it. But the test should say so instead of looking
like the documented shape, and `design.md:51`'s illustrative `git show
a28d0c5648:` is prose, not a lock, so it is left alone.

**`status` does not run `lock_coherence_guard`.** Combined with the above, the
worst pairing available: `status` prints an actionable plan from a lock that
`apply` exits 2 on. Fixed here — as a **warning**, not a refusal. `status` is
read-only and its whole product is telling the truth about the machine;
refusing to print would withhold the information the user needs to fix it. The
guard's message already ends in "Run `dotpkg update`", which stops being a
pointer to a command that does not exist.

## `update`

Reads `pkg.toml` and the buckets. Touches no installed software and runs no
scoop subprocess.

### "Latest" means fetching, and this is the network reach the design allows

`update` is the one command the approved design permits to reach the network
(`design.md:250` — "`resolve` is the only method that reaches the network, and
only `update` calls it"). It has to be: **"latest" in a bucket nobody has
fetched is "latest as of the last time something else pulled it"**, and a lock
built from that is stale while claiming to be current. This is the same class
of error as a lock quietly falling back to "latest" — a guarantee that is not
there — only in the other direction.

So `update` refreshes each declared bucket before resolving, and does it
without touching anything scoop owns:

- **`git fetch` only.** Not `git pull`, not a checkout: the bucket's local
  branch and working tree are left exactly where scoop put them. A bucket is
  scoop's directory, and dotpkg moving it is a change nobody asked for.
- **Resolution reads the remote-tracking ref**, found via `git rev-parse
  --abbrev-ref @{u}`, in place of `HEAD` everywhere `HEAD` appears below. The
  fetched objects are reachable from `refs/remotes/`, so `Scoop::stage` can
  `git show` them at `apply` time even though no branch moved.
- **No upstream configured, or the fetch fails:** warn, name the bucket and the
  reason, resolve against the local ref instead, and say in the diff that the
  result is offline. Never resolve stale in silence.
- **`--offline`** skips the fetch deliberately, for a machine with no network
  or for re-resolving a second time without moving the target.

`apply` is unaffected: it never fetches, and everything it needs is in the
object database once `update` has run.

### Resolving one package

1. **Bucket.** The existing lock entry's bucket wins, so `update` never moves a
   package between buckets silently. With no entry, search every bucket
   `pkg.toml` declares for `bucket/<app>.json` at HEAD.
   - Found in exactly one: use it.
   - Found in several: **refuse this package**, naming every bucket that has
     it, and point at `[scoop.opts] <pkg> = { bucket = "..." }` — a new field
     on the existing `PkgOpts`, which is the only place that information can
     live. This mirrors scoop's own behaviour, which also refuses and makes you
     name the bucket.
   - Found in none: refuse this package, naming the buckets searched.
2. **Filename.** The same chain `Scoop::stage` uses — the user's spelling, then
   the folded form, then a case-insensitive `git ls-tree` — resolved **at
   HEAD**, because `update` means "latest".
3. **Commit.** `git log -1 --format=%H -- <path>`, **without**
   `--full-history`: measured, that flag makes this step return a merge commit.
4. **Self-check.** `git show <commit>:<path>` and `git show HEAD:<path>`; if
   they differ, record HEAD instead. This makes "the recorded commit's blob is
   the bucket's current content for this file" true by construction rather than
   by trusting `git log`'s history simplification. It costs one extra `git
   show` and it is the reason the shallow-clone case degrades correctly instead
   of silently.
5. **Version.** Read from that blob. Never from the machine.

### Writing

- **Validated by `lock_coherence_guard` before the file is written.** The
  writer checks its own output with the reader's guard. A `Pin` that would make
  `apply` refuse the run must never reach disk.
- Written with the same temp-then-rename discipline as `State::save`, keeping
  the displaced file as `pkg.lock.bak`. `pkg.lock` is committed, so a torn
  write is a git conflict on top of a broken tool.
- `update` with no argument rewrites the whole file, **dropping entries for
  packages `pkg.toml` no longer declares**. `update <pkg>...` touches only the
  named entries and drops nothing.
- **A failed re-resolve keeps the previous entry**, and says so. The failure is
  per package, the run continues, and the exit code is 1. Silently dropping the
  entry would convert a package that works today into one that refuses the next
  `apply`.
- `[winget]` packages are reported as not resolvable until Phase 4, the same
  way `plan()` already reports them, rather than dropped in silence.

### What `update` prints

The diff between the old lock and the new one — the only place in dotpkg where
both exist at once:

```
  + scoop  ripgrep     15.2.0                     (new pin)
  ^ scoop  fzf         0.74.1 -> 0.74.2           (version changed)
  = scoop  bat         0.26.1, commit re-pinned   (apply will not act on this)
  - scoop  aichat      dropped, no longer declared
  ! scoop  zellij      kept the previous pin: bucket "extras" has no zellij.json
```

The `=` line is the answer to "does `update` converge by version or by commit".
It converges by **commit** when it writes; `apply` converges by **version**
when it acts; and the one place a user can see the gap is here, at the moment
it is created, said in words rather than left to be discovered. A same-version
re-pin is a real change to the lock and a real non-event for the machine, and
pretending either half away would be a lie.

**Acting on a same-version re-pin is deliberately out.** Every version change
is uninstall + install, measured, in both directions; making `apply` reinstall
a working package because a maintainer fixed a `checkver` regex is a bad
default. `ArchDrift` set exactly this precedent — reported, not acted on — and
the 2b-2 design already recorded that acting on drift needs its own
`Action::Reinstall`, as a later phase's decision.

## `adopt`

Brings an installed, unowned package under management. Reaches no network,
changes no installed software.

### Why it writes all three files

Read off the shipped planner, not assumed:

- **`state.json` alone makes the package a prune candidate.** `plan()` walks
  installed packages and emits `Prune` for `installed ∧ ¬declared ∧ owned`
  (`src/plan.rs`). So `dotpkg adopt aichat` followed by `dotpkg apply` would
  *remove* aichat.
- **`state.json` + `pkg.toml` breaks every later `apply`.** Declared with no
  lock entry is `Skip{NotLocked}` → `Outcome::NotLocked` → `is_ok()` false →
  `main.rs` refuses the whole run at exit 2, and under `--keep-going`
  `gate_removals` holds every prune in the plan.

All three, or the machine is left in a state dotpkg itself refuses to act on.

### Resolving one package

1. **Bucket.** `install.json`'s `bucket` if it names one `pkg.toml` declares —
   available precisely because `adopt` targets packages dotpkg has never
   touched, and it is dotpkg's own installs that lose the field. Otherwise the
   same declared-bucket search, and the same refusal on ambiguity, as `update`.
2. **Candidates.** `git log --full-history --format=%H -- <path>`.
   `--full-history` is **required** here: measured, without it a version that
   reached the bucket through a superseded merge parent is unreachable, and
   `adopt` would report "not in this bucket" about a commit that is a genuine
   ancestor of HEAD.
3. **One `git cat-file --batch`**, fed every `<commit>:<path>`, rather than one
   `git show` per candidate. Measured on a 400-commit history with the match at
   position 394: **2 processes and 0.02 s against 395 processes and 3.16 s,
   identical answer.** The ratio is from a synthetic repository; the process
   count is what transfers, and it transfers to Windows, where spawning is
   dearer than on macOS.
4. **Match, in this order, recording which rule fired:**
   1. the installed `apps/<app>/current/manifest.json` equals the blob under
      `verify::normalise` — exact, and the only rule that can tell two
      same-version commits apart;
   2. the blob's `version` equals the installed version — the fallback for a
      machine whose manifest was rewritten by something other than line
      endings.
5. **No match: refuse, and write nothing at all.** The message names the
   package, its installed version, every bucket searched, and **whether the
   bucket is shallow** (`git rev-parse --is-shallow-repository`) — measured, a
   shallow clone produces exactly the same "not found" with no other signal,
   and `scoop bucket add`'s full clone does not cover a bucket the user cloned
   by hand.

### Writing, and the order

`pkg.lock` → `pkg.toml` → `state.json`. Every prefix of that order is inert:

- lock only — an entry for an undeclared package; `plan()` never reads it, and
  the next full `update` drops it.
- lock + `pkg.toml` — declared, locked, installed at the locked version, so
  `plan()` emits nothing.
- all three — adopted.

The dangerous order is `state.json` first, which is the prune-candidate shape
above. This mirrors the executor's own reasoning about claiming ownership late.

**`pkg.toml` is the user's file, not the tool's**, and it is the only file
dotpkg writes that a human wrote by hand and committed with comments. So:

- edited with `toml_edit`, which preserves comments, ordering and formatting.
  It is **already in `Cargo.lock` at 0.22.27** as a transitive dependency of
  `toml`, so promoting it to a direct dependency adds no crate to the tree.
- the displaced file is kept as `pkg.toml.bak`, as `State::save` already does;
- and the result is **re-parsed with `config::parse` and compared to the intended
  config before it replaces the original**. If the round trip changed anything
  but the added name, the write is abandoned and the original stands.

`Ownership::Adopted` is written here — the variant has been readable since
2b-2 and has never had a writer. 2b-2 already has a test that an upgrade of an
adopted package does not silently rewrite it to `Installed`; that test stops
being vacuous in this phase.

### Bulk

`dotpkg adopt <pkg>...` takes several names. Per package it is all-or-nothing
across the three files; across packages, a failure is reported and the others
proceed — the same shape as `prepare`. `dotpkg adopt` with no argument is
**not** "adopt everything unmanaged": that is one keystroke from handing dotpkg
permission to delete every package on the machine the next time `pkg.toml`
changes. Bulk adoption of a fresh machine is `adopt` with an explicit list,
which `status`'s `? unmanaged` lines already produce.

## Closed in this phase, from the carried debts

1. **`download` behind `Mutator`** — the one item `docs/phase2b-notes.md` still
   lists as open. Done **first**, because it is what makes a real
   `Outcome::ReadyToFetch` reachable from a test on macOS, and because the
   untested last line of `stage_and_fetch` (that `arch` actually reaches the
   argv) dies with it.
2. **Staging paths are not content-addressed** — `Scoop::stage_text` writes to
   `<staging_root>/<app>/<version>`, keyed on app and version only, so
   re-pinning the same version to a different commit overwrites the file an
   installed app's `install.json` points at. Phase 3 is the phase that makes
   re-pinning routine, so this is where the debt starts costing something.
   The commit joins the path.
3. **`status` does not run `lock_coherence_guard`** — above.

Not closed, and unchanged: `mass_prune_guard` still reads scoop only, and still
must grow a backend loop in the same change that adds the winget backend.

## Testing

Layers 1 and 2 from the approved design, unchanged: everything below runs on
macOS and Linux.

**The asymmetry this phase gets to exploit: git is on every machine, scoop is
not.** Every previous phase had to fake its subprocess and could only prove the
real thing on a14. Phase 3's riskiest code — history walking, filename
resolution, version and content matching — talks to `git`, so its tests build
**real git repositories in a `tempfile::tempdir`** and assert against ground
truth they constructed. There is no fake to be self-consistent with.

The fixtures are exactly the shapes the measurements found, so each one is a
regression test for a result rather than an invention:

| Fixture | What it pins |
|---|---|
| linear history with unrelated churn | the per-file commit is not the bucket tip |
| a merge that supersedes a side branch | `adopt` finds the version, `update` does not name the merge |
| two commits, one version, different content | content matching beats version matching |
| the same, with the installed manifest rewritten to CRLF | `normalise` is reached |
| a rename | the walk lands on a commit with the right content |
| delete then re-add | neither algorithm is confused by a gap |
| a shallow clone | `adopt` refuses with a message naming shallowness |
| a bucket with the manifest under a different case | the `ls-tree` fallback resolves at the locked commit |

**One fixture cannot be built the obvious way.** The case-different filename
(`bucket/Tool.json` at an old commit, `bucket/tool.json` at HEAD) cannot be
made with `git mv` on macOS or Windows, whose filesystems are case-insensitive
— the first probe run tried exactly that and measured nothing. Git stores the
name in the tree object regardless of the filesystem, so the fixture is built
with plumbing (`hash-object -w`, `update-index --add --cacheinfo`,
`write-tree`, `commit-tree`) and never checked out. Named here so it is not
rediscovered mid-task, and so the same trap does not produce a green test that
means nothing on the one platform this tool runs on.

**Every negative control must be shown to be able to go red**, and the
assertion that fires must be recorded. Three controls in the Phase 2b-2 plan
could not go red — one because `order()` sorts alphabetically, so the good
package ran first no matter what the mutation did. Specifically here:

- A control that makes the content match always succeed must leave the
  same-version fixture red and the ordinary fixture green — a single fixture
  cannot distinguish them.
- `msg.contains(...)` alone does not survive a mutation that always fails with
  the right words. Every refusal assertion is paired with a count of how many
  files were written (which must be **zero**) or with a positive sibling.
- The `pkg.toml` round-trip guard is controlled by mutating it to accept any
  parse, and the fixture that must then go red is a `pkg.toml` with comments
  and a `[scoop.opts]` table, not a bare package list.

**Standing rules inherited and kept:** `--no-fail-fast` on every run; no test
may create a file at `Scoop::scoop_exe()`'s path; the suite runs on Windows
before the dogfood, not after.

## Dogfood

a14, and for the first time a phase whose commands change no installed
software — `update` and `adopt` write files only. The risk is not that the
machine breaks during the run; it is that the lock they produce is wrong and
the *next* `apply` acts on it. So the dogfood's product is a lock, and the test
of the lock is `apply --prepare` against it.

Framed so it can fail:

1. Does `update` produce, for all 25 declared packages, a lock that
   `apply --prepare` accepts — and does it agree with the lock the 2b-1
   rehearsal script produced by a different algorithm? Three independent runs
   of two scripts already agree with each other; a fourth implementation
   disagreeing is a finding either way.
2. How long does `update` take across 25 packages against the real
   `main`/`extras` buckets? The 153× in the measurements is synthetic and
   proves nothing about a 78,000-commit repository.
3. Does any declared package exist in more than one declared bucket? If none
   does, the ambiguity refusal never fires and `[scoop.opts] bucket` is
   documentation rather than a working path — which must be said.
4. Does `adopt` on one of the three genuinely unmanaged packages
   (`aichat`, `antigravity`) write exactly three files, and does `status`
   afterwards show it as managed rather than as a prune?
5. Does `adopt` refuse cleanly on a package whose version is not in its
   bucket's history — and can such a package be found at all, or does that
   failure mode have to be constructed?
6. Is `pkg.toml` byte-identical afterwards except for the added line, comments
   and all?
7. Does `git fetch` on `main` and `extras` actually move anything, and does
   `update` resolve differently before and after it? If the buckets are already
   current the fetch is invisible, and the one property that most needs
   proving — that "latest" means fetched, not cached — goes unexercised. It has
   to be induced: reset a bucket's remote-tracking ref back a few commits, run
   `update`, and confirm the pin moves forward again.

`kanata` is never started or stopped. `C:\Users\kln\dotpkg-build` and
`C:\Users\kln\pkg.toml` are reused. Everything dotpkg writes is under
`%LOCALAPPDATA%\dotpkg` or is a file this run created, and is removed and
re-checked individually.

A prediction worth recording: **`update` will disagree with the 2b-1 rehearsal
script on at least one package**, because the rehearsal resolved the *installed*
version by walking history — which is `adopt`'s algorithm — while `update`
resolves the *latest*. Seven of 25 packages already had a matching commit that
was not their bucket's HEAD. If the two agree on all 25, that means every
declared package is already at its bucket's latest, which is a claim about the
machine worth checking separately rather than accepting as agreement.

## Non-goals

Unchanged from the approved design. Additionally, Phase 3 does not implement
`add`; does not act on architecture drift or on a same-version commit re-pin;
does not touch the winget backend; does not resolve dependencies (no package on
a14 declares `depends`, measured twice); and does not add locking against two
concurrent dotpkg runs, which remains undefined behaviour.
