# Dogfood: Phase 3, `update` and `adopt` — a14, 2026-08-09

The first phase whose commands change no installed software. `update` and
`adopt` write files only, so the risk was never that the machine breaks during
the run — it is that **the lock these commands produce is wrong and the next
`apply` acts on it**. The product of this dogfood is therefore a lock, and the
test of the lock is `apply --prepare` against it.

Branch `phase3-update-adopt`, built natively on a14 with `rustc 1.97.1`,
`cargo 1.97.1`, `git 2.55.0.windows.3`, against `scoop 0.5.3` and the real
`main` (78,473 commits), `extras` (90,191) and `xom11` (22) buckets.

**Nothing was installed, upgraded or removed.** 31 app directories and every
one of their versions identical before and after; `kanata`
(`kanata_windows_tty_winIOv2_arm64`, PID **5676**) never started or stopped;
`%LOCALAPPDATA%\dotpkg` absent at the end exactly as it was at the start;
`C:\Users\kln\pkg.toml` restored to its byte-exact original
(`sha256 32A238FF…`).

**The headline: a defect that made `adopt` print a false sentence with no
diagnostic — `antigravity is not installed`, about a package that is
installed.** Found by running the command on a real machine, not by review.
Fixed, tested, and re-confirmed on the same machine.

Two things on the machine did move and are recorded rather than explained away.
Both are in "The machine afterwards".

## What the machine is

Captured read-only before anything ran, so nothing downstream compares against
stale numbers.

| | |
|---|---|
| apps (incl. `scoop`) | **31** |
| cache entries | **80** (Phase 2b-2 recorded 76) |
| `kanata` | `kanata_windows_tty_winIOv2_arm64`, PID **5676**, started 09:13:31 |
| `explorer` | PID **15524** |
| `%LOCALAPPDATA%\dotpkg` | **does not exist** |
| `pkg.toml` | 449 bytes, 25 declared packages, 3 buckets, **no comments** |
| `main` | `d7fe19ecae…`, `master` tracking `origin/master` |
| `extras` | `30a4ac5c8f…` |
| `xom11` | `d535b3fa3c…` |

`kanata`'s PID is 5676, not the 15732 Phase 2b-2 recorded; the machine slept in
between. `explorer` is 15524 — the value 2b-2 recorded *after* its own run, so
it has not moved since.

Seven declared packages are behind their bucket's latest, which is what made
questions 1 and 4 answerable at all: `nodejs`, `fastfetch`, `tree-sitter`,
`lazygit`, `opencode`, `python`, `uv`.

**One artifact of the previous dogfood is visible in this one.** `shfmt`'s
`install.json` carries **no `bucket` field**: Phase 2b-2 downgraded and restored
it through dotpkg, and dotpkg installs from a staged manifest *path*, so scoop
recorded a `url` and not a bucket. Nothing here depends on that field —
`update` reads `pkg.toml`'s declared buckets — but the Phase 2b-1 rehearsal
script, which took the bucket from `install.json` alone, **would fail on `shfmt`
today**. A dogfood changed the input to a later dogfood.

## Q1 — does `update` produce a lock `apply --prepare` accepts?

**Yes.** `update` resolved **25 of 25** declared packages and exited **0**:

```
  + scoop  git            2.55.0.3                   (new pin)
  + scoop  nodejs         26.7.0                     (new pin)
  + scoop  gh             2.97.0                     (new pin)
  + scoop  bat            0.26.1                     (new pin)
  + scoop  ripgrep        15.2.0                     (new pin)
  + scoop  fzf            0.74.2                     (new pin)
  + scoop  fastfetch      2.67.0                     (new pin)
  + scoop  neovim         0.12.4                     (new pin)
  + scoop  tree-sitter    0.26.12                    (new pin)
  + scoop  lazygit        0.64.0                     (new pin)
  + scoop  lazydocker     0.25.2                     (new pin)
  + scoop  yazi           26.5.6                     (new pin)
  + scoop  zellij         0.44.3                     (new pin)
  + scoop  opencode       1.18.15                    (new pin)
  + scoop  shfmt          3.13.1                     (new pin)
  + scoop  yamlfmt        0.21.0                     (new pin)
  + scoop  stylua         2.5.2                      (new pin)
  + scoop  actionlint     1.7.12                     (new pin)
  + scoop  kanata         1.12.0                     (new pin)
  + scoop  beckon         0.5.2                      (new pin)
  + scoop  python         3.14.7                     (new pin)
  + scoop  go             1.26.5                     (new pin)
  + scoop  rustup         1.29.0                     (new pin)
  + scoop  uv             0.12.3                     (new pin)
  + scoop  age            1.3.1                      (new pin)

  25 changed, 0 unchanged, 0 could not be resolved.        EXIT = 0
```

2,627 bytes, 25 entries, three buckets represented (`main`, `extras`, `xom11`).

`apply --prepare` against that lock:

```
  8 of 8 changes ready, 0 failed, 1 skipped, 0 not locked.
  Nothing has been changed.                                EXIT = 1
```

**The exit code is 1 and the lock was not rejected.** `0 failed, 0 not locked`
is the answer to this question; the 1 comes from `! scoop python running --
stop it first`, and `--prepare`'s documented rule is that a running skip is
outstanding work. Read carelessly, `EXIT = 1` looks like a verdict on the lock.
It is a verdict on the machine.

**Two packages were planned as `install`, not `upgrade`, and that is wrong.**

```
  + scoop  zellij         0.44.3                   (install)
  + scoop  actionlint     1.7.12                   (install)
```

Both *are* installed, at exactly the pinned version. Under plain elevated `ssh`
their `manifest.json` cannot be traversed ("untrusted mount point", os error
448) — the junction quirk Phase 2a identified and every phase since has seen —
so `scan` warns and treats them as absent. dotpkg is behaving correctly: it
warns rather than lying, and the warnings are printed above the plan. But the
consequence is **new to this phase**, because in 2b-2 those packages were
undeclared and therefore invisible. Now they are declared, so the elevated-`ssh`
reading of this machine produces a plan with **8 changes where a
medium-integrity run would produce 6**. Nothing was applied, so nothing came of
it — but a `--yes` run under these conditions would reinstall two packages for
no reason.

## Q2 — how long does it take against real buckets?

Wall clock, 25 declared packages, `main` at 78,473 commits:

| run | wall clock |
|---|---|
| `update` (fetches all three buckets, fresh lock) | **31.469 s** |
| `update --offline` (fresh lock, no fetch) | **23.909 s** |
| `update` again over the same lock (converged) | **16.377 s** |

So the fetch of three buckets costs roughly **7.6 s** and resolution costs the
rest. The converged run is faster because it re-resolves but writes nothing:
`0 changed, 25 unchanged, 0 could not be resolved. / pkg.lock is already
current -- not rewritten.`

**This does not validate the 153× in `docs/measurements-2026-08-09-git-resolution.md`,
and nothing here should be read as validating it.** That figure is a spawn-count
ratio measured on fabricated repositories, and it is about the *history walk*
— which is `adopt`'s algorithm, not `update`'s. `update` does one `git log -1`
and one `git show` per package. The comparable real-bucket numbers this dogfood
did produce are in Q4 and Q7.

## Q3 — does the fetch actually change an answer?

**Yes, and it was made to prove it rather than assumed.** This property is
invisible when the buckets are already current, which they were.

It did not have to be manufactured out of nothing: `update`'s own first run
moved `refs/remotes/origin/master` forward in two buckets, so the pre-fetch
values were real, observed, and only minutes old.

The rewind target was chosen so a changed answer was *forced*, not hoped for:
the newest commit touching any declared `main` manifest was `b5859a0e`
(*tree-sitter: Update to version 0.26.12*), and `refs/remotes/origin/master` was
set to its first parent `7e202af1` (*oh-my-posh: Update to version 30.6.4*).

```
rewind:  refs/remotes/origin/master  8b6fba7f -> 7e202af1
         refs/heads/master           d7fe19ec  (untouched)

update --offline
  ^ scoop  tree-sitter    0.26.12 -> 0.26.11         (version changed)
  1 changed, 24 unchanged, 0 could not be resolved.

  commit  "b5859a0e…" -> "242a3475…"

update  (fetching)
  ^ scoop  tree-sitter    0.26.11 -> 0.26.12         (version changed)
  1 changed, 24 unchanged, 0 could not be resolved.

  origin/master back to 8b6fba7f — restored by the fetch itself
  lock now byte-identical to the lock before the rewind
```

Three things this establishes on real hardware, none of which a fixture can:

- Resolution reads the **remote-tracking ref**, and follows it exactly: one
  package moved and twenty-four did not, which is what a surgical rewind should
  produce and what a blunt "re-resolve everything" would not.
- **The fetch is what changes the answer.** The only difference between the two
  runs was `--offline`.
- **`update` fetches and never pulls.** `refs/heads/master` sat at `d7fe19ec…`
  through the entire session, in all three buckets, and every working tree was
  clean afterwards. This is the design's central promise about not touching what
  scoop owns, and it is now measured rather than reasoned.

Restoration was verified explicitly, not assumed: the saved SHA was checked to
be an ancestor of the post-fetch value, with a forced `update-ref` standing by
if it were not. It was not needed.

## Q4 — does `update` disagree with the Phase 2b-1 rehearsal script?

**The design predicted it would. Confirmed: they disagree on 7 of 25, in both
version and commit.**

The 2b-1 rehearsal was re-implemented as its own document describes it — read
the installed version, walk `git log` newest-first, `git show` each candidate,
stop at the first blob whose `version` matches — and run against today's
buckets.

| | rehearsal (installed) | `update` (latest) |
|---|---|---|
| nodejs | 26.5.1 `7549b4d8` | 26.7.0 `e3aa537a` |
| fastfetch | 2.66.0 `0ed14266` | 2.67.0 `52496578` |
| tree-sitter | 0.26.11 `242a3475` | 0.26.12 `b5859a0e` |
| lazygit | 0.63.1 `88ff9b8b` | 0.64.0 `39b549d6` |
| opencode | 1.18.11 `d34c68b8` | 1.18.15 `00d5b56b` |
| python | 3.14.5 `87e995f1` | 3.14.7 `1b2868b1` |
| uv | 0.12.1 `2ff2b541` | 0.12.3 `36160060` |

**18 agree entirely** — same bucket, same version, same commit — and the reason
is a fact about the machine, not agreement between the algorithms: those 18 are
already at their bucket's latest, so "newest commit carrying the installed
version" and "the version at the tip" are the same commit. Checked separately
rather than accepted: `at latest: 18 of 25`. **Zero bucket disagreements.**

**A coincidence that should not be mistaken for a result.** Phase 2b-1 also
found 7 of 25 behind. It is not the same seven: `beckon` has since caught up
(0.5.2 is now latest) and `tree-sitter` has since fallen behind. Same count,
different set, one day apart.

Timing, as a real-bucket data point rather than a synthetic one: the rehearsal
took **133.767 s** for 25 packages against these buckets, where `update` took
31.469 s including the fetch. The two answer different questions and this is
**not** a like-for-like benchmark — the rehearsal walks history and `update` does
not — but it is the first measurement of that walk against a 78,000-commit
repository, and the per-candidate `git show` spawn is visible in it.

## Q5 — is any declared package in more than one declared bucket?

**No. Zero of 25.** Every declared package resolves to exactly one declared
bucket: 21 in `main`, 3 in `extras` (`lazygit`, `kanata`, `age`), 1 in `xom11`
(`beckon`).

Across the *entire* union of the three declared buckets — 1,624 + 2,363 + 1
manifests — exactly **one** name appears twice: `flux`, in both `main` and
`extras`. It is neither declared nor installed.

**So the ambiguity refusal never fires on this machine in real use, and
`[scoop.opts] bucket` does nothing for this `pkg.toml`.** That must be said
plainly rather than dressed up.

It was then fired deliberately, on real bucket data, in a throwaway `$env:SCOOP`
root — because a guard nobody has ever seen run is not a tested guard:

```
  ! scoop  flux    could not be resolved, nothing to keep: 2 declared buckets
                   carry it (main, extras). Say which with
                   `[scoop.opts] flux = { bucket = "..." }`.

  0 changed, 0 unchanged, 1 could not be resolved.         EXIT = 1
  (no pkg.lock written)
```

and the documented escape hatch works:

```
[scoop.opts]
flux = { bucket = "extras" }

  + scoop  flux           4.141                      (new pin)       EXIT = 0
  [scoop.flux] bucket = "extras"  commit = "6c70b346…"  version = "4.141"
```

So the mechanism is real and now demonstrated against real buckets. What is
**not** demonstrated is that anything on this machine needs it.

**A second, smaller thing that run exposed, recorded and not fixed.** With no
`pkg.lock` on disk and the single declared package unresolvable, `update`
printed `pkg.lock is already current -- not rewritten.` There was no `pkg.lock`,
and it was not current. The line follows correctly from `wrote_anything()` being
false, and nothing acts on it — but in a tool whose spine is that every printed
line is true, it is a false one.

## Q6 — `adopt` on a genuinely unmanaged package

`aichat` 0.30.0: installed, undeclared, unowned, and readable.

```
  + scoop  aichat         adopted (the installed manifest matches the bucket exactly)

  1 adopted, 0 refused. Nothing installed and nothing removed.    EXIT = 0
```

1.294 s, and the strong evidence variant — `Matched::Content`, the installed
manifest is byte-equal to the bucket blob, not merely the same version string.

**"Exactly three files changed" is false, and this is the check that earned its
keep.** Three files are *written*, but `lock::save`, `config_edit::save` and
`state::save` each copy the file they displace to `.bak` first, so adopting one
package against a pre-existing `pkg.toml` and `pkg.lock` touched **five** paths:

```
pkg.lock       modified   2627 -> 2732 bytes
pkg.lock.bak   created    (the displaced lock)
pkg.toml       modified   449  -> 461  bytes
pkg.toml.bak   created    (byte-identical to the pre-run file, sha 32A238FF…)
state.json     created
```

That is the documented behaviour of all three writers and not a defect. It is
recorded because the expectation this dogfood was handed said three, and three
is not what a machine shows you.

**`pkg.toml` is byte-identical except the added line.** One line inserted,
`  "aichat",`, on its own indented line with a trailing comma, matching the
surrounding multiline style. Every other line unchanged, including
`[scoop.opts]` and all three arch pins.

**The "comments and all" half of that check was vacuous here, and is reported as
vacuous.** This machine's `pkg.toml` contains zero comments, so a `toml_edit`
that dropped every comment would have passed. It was therefore re-run against a
deliberately commented copy, with the harder shapes in it:

```
# dotpkg config for a14 -- this comment must survive `dotpkg adopt`
...
packages = [
  "git", "ripgrep",  # two only
  "fzf",
+ "dark",
+ "7zip",
+ "innounp",
]

# opts below; toml_edit must not reflow or drop any of this
[scoop.opts]
python = { arch = "64bit" }   # trailing comment on an inline table
```

**5 comments in, 5 comments out**, including the one *inside* the packages array
and the one trailing an inline table. Every original line present and unchanged,
shifted only by the three insertions. No BOM introduced.

That run also exercised **bulk adopt with no `--all`** — four names, one
command, `3 adopted, 1 refused`, exit 1.

**`state.json` says `adopted`, not `installed`:**

```json
{
  "scoop": {
    "aichat": "adopted"
  }
}
```

**`status` afterwards shows it as managed** — which in practice means it
disappears from the output entirely, because a package that is declared, locked
and installed at the pinned version produces no line at all. It is not reported
as a prune and not reported as unmanaged. Worth stating in those words: the
observable proof of "managed" is an absence.

### The undo

Every step verified individually, because a `state.json` left behind claiming
dotpkg owns a package it did not install is exactly the trap the Phase 2b-2
dogfood caught:

```
pkg.toml restored from adopt's own .bak   sha256 = 32A238FF…  MATCHES RECON
pkg.toml.bak removed                      Test-Path = False
%LOCALAPPDATA%\dotpkg removed entirely    Test-Path = False
  (state.json = False, manifests = False; no recover.cmd existed)

status now prints:
  ? scoop  aichat         0.30.0                   (unmanaged -- no action)
```

`aichat` is back to being a package dotpkg reports and refuses to touch.

## Q7 — does `adopt` refuse cleanly, writing no file at all?

**The failure mode had to be constructed. It does not occur on this machine.**
Every installed package whose manifest dotpkg can read — all 25 declared, plus
`aichat`, `dark`, `7zip`, `innounp` — resolved to a real commit in a real
bucket. Nothing was left over to refuse.

Constructed in a throwaway `$env:SCOOP` root: junctions to the real `main` and
`extras` buckets, and a `ripgrep` manifest copied from the real one with its
version rewritten to `0.0.0-not-real` (BOM-less `UTF8Encoding($false)`).

```
  ! scoop  ripgrep        no commit in bucket main carries ripgrep 0.0.0-not-real

  0 adopted, 1 refused. Nothing installed and nothing removed.    EXIT = 1
```

6.52 s — a full walk of all 21 commits touching `bucket/ripgrep.json` that finds
nothing, against a 78,000-commit repository.

**No file at all:**

| | |
|---|---|
| `pkg.lock` | not created |
| `pkg.lock.bak` | not created |
| `state.json` | not created |
| `state.json.bak` | not created |
| `pkg.toml.bak` | not created |
| `pkg.toml` | sha256 unchanged |
| `*.tmp*` leftovers | 0 |

The refusal did **not** mention shallowness, correctly — the bucket is a full
clone. The shallow clause therefore remains unit-tested only.

## The defect this dogfood found, and the fix

`dotpkg adopt antigravity` printed:

```
  ! scoop  antigravity    antigravity is not installed. `adopt` brings an
                          existing package under management; to install one,
                          declare it and run `dotpkg update` then `dotpkg apply`.
```

`antigravity` **is** installed, at 2.0.6. Under elevated `ssh` its
`manifest.json` cannot be traversed, so `scan` excluded it — and `scan` said so,
into a `warnings` list that **`adopt` was the only command to throw away**.
`status` (`main.rs:145`), `apply` (`:189`) and `update` (`:450`) have each
printed these since Phase 2a. `adopt::run` called `Backend::scan` and never read
`scan.warnings`.

The refusal is not wrong given what dotpkg could see. It is *unactionable* on
its own, and it reads as a flat contradiction of the machine.

This is `docs/phase3-notes.md`'s **THIRD PATTERN** repeating precisely as
written — *the coverage hole sits where the output meets a human* — in the one
place nobody looked, and it is the second time on this branch that `adopt`'s
user-facing output has been the weak point.

**Fixed.** `adopt::Outcome` grew a `warnings` field carried out of the scan, and
the `Adopt` arm of `main.rs` prints it above the outcome, in the same format and
for the same stated reason as `status` and `apply`. Two tests in `tests/cli.rs`,
deliberately paired:

- `adopt_prints_what_the_scan_could_not_read_before_calling_a_package_uninstalled`
- `adopt_prints_no_warning_when_the_scan_read_everything`

The second exists because an unpaired `contains` is satisfied by an
implementation that warns on every run and teaches the user to ignore the line.

**Negative control fired.** Deleting the four-line print made the first test go
red on its own named assertion (`tests/cli.rs:1252`, the stderr `contains`) and
left the counterweight green — not at an `unwrap` above it, which is the failure
mode `docs/phase3-notes.md` sweeps for.

**Verified on the machine that found it**, with the rebuilt binary:

```
warning: scoop: actionlint: cannot read manifest.json: ... (os error 448)
warning: scoop: antigravity: cannot read manifest.json: ... (os error 448)
warning: scoop: zellij: cannot read manifest.json: ... (os error 448)
  ! scoop  antigravity    antigravity is not installed. ...
  0 adopted, 1 refused.                                  EXIT = 1
  (no pkg.lock, no state.json, no pkg.toml.bak written)
```

**Suites:** macOS **368 passed, 0 failed**; a14 **366 passed, 0 failed**,
`--no-fail-fast`, per target. The difference is exactly the two `#[cfg(unix)]`
tests already documented (`tests/adopt.rs` 18→17, `tests/scoop_scan.rs` 27→26);
`tests/cli.rs` is 25 on both, so both new tests run natively on Windows.
`cargo fmt --check` and `cargo clippy --all-targets` clean.

This satisfies the rule `docs/phase3-notes.md` set for itself: **the Windows run
belongs at the end of the change as well as before the dogfood.** The suite was
rerun on the tree this branch now ships, not on the tree the dogfood started
from.

## One more observation, neither defect nor finding

`--state` relocates `state.json` but **not** the staging root.
`apply --prepare --state C:\Users\kln\dotpkg-run\state.json` still created
`%LOCALAPPDATA%\dotpkg\manifests\…`. `default_staging_root`'s own doc comment
gives the reason — `install.json` records that path, so a staging directory that
moves leaves installed apps pointing at nothing — so this is deliberate. It is
recorded because "point `--state` somewhere else and dotpkg writes nowhere else"
is a reasonable thing to believe and is not true.

## The machine afterwards

All **31** app directories compared against the baseline captured before
anything ran: **no package added, none removed, no version changed.**

`kanata` still `kanata_windows_tty_winIOv2_arm64`, PID **5676**, same start
time — never started, never stopped. `explorer` still PID **15524**, unchanged
this session (2b-2 saw it move; here it did not).

`AHKrunning` Running; `AHKWatchdog`, `BeckonServeWatchdog`, `Kanata`,
`KanataWatchdog` all Ready. No `Dotpkg*` scheduled task exists.

**Two things moved. Neither is explained away.**

**1. The cache went 80 → 87.** `apply --prepare`'s job is download-and-verify,
and it downloaded the seven artifacts the plan needed:

```
+ actionlint#1.7.12#a302452.zip      + nodejs#26.7.0#dfb5def.7z
+ fastfetch#2.67.0#1ecd15f.7z        + opencode#1.18.15#f055c8d.zip
+ lazygit#0.64.0#4927d7a.zip         + tree-sitter#0.26.12#b5be060.gz
+ uv#0.12.3#83bc070.zip
```

115 MB, for upgrades the user never asked for. Removed by exact name afterwards,
each verified individually; **cache back to 80.** Deleting them is itself a
change to the machine, and it is recorded as one — scoop would simply
re-download if those upgrades were ever wanted.

**2. `refs/remotes/origin/master` moved forward in two buckets:**

```
main    d7fe19ecae… -> 8b6fba7f8f…
extras  30a4ac5c8f… -> 5f19a42b21…
xom11   unchanged
```

This is `update`'s own fetch doing exactly what the command exists to do, and it
was **not** undone: it is the same thing scoop's next `scoop update` would do,
and reverting a remote-tracking ref to a stale value would be the more invasive
choice. `refs/heads/master` did not move in any bucket and every working tree is
clean, which is the property that actually matters.

## Cleanup, each removal verified on its own

```
pkg.toml.bak               removed   Test-Path False
%LOCALAPPDATA%\dotpkg      removed   Test-Path False  (state.json, manifests)
C:\Users\kln\dotpkg-run    removed   Test-Path False  (12 files)
C:\Users\kln\dotpkg-probe  removed   Test-Path False
C:\Users\kln\dotpkg-src.tgz removed  Test-Path False
%TEMP%\dotpkg-fixcheck     removed   Test-Path False
%TEMP%\dotpkg-step.ps1     removed   Test-Path False
```

**The probe root's bucket junctions were removed with `cmd /c rmdir`, one at a
time, before anything recursive touched the directory**, and the real buckets
were re-counted afterwards (1,624 / 2,363 / 1 manifests, `.git` present, working
trees clean). `Remove-Item -Recurse` across a junction is exactly how a
throwaway probe root eats the real bucket it borrowed, and this run never gave
it the chance.

Kept, matching every previous phase: `C:\Users\kln\dotpkg-build` and
`C:\Users\kln\pkg.toml`.

## What this dogfood deliberately did NOT cover

Recorded because a dogfood that confirms every expectation is a dogfood that was
not trying.

**No `apply` without `--prepare`.** Phase 3 changes no installed software, and
Phase 2b-2 already drove the executor through install, upgrade, downgrade and
prune on this machine's real scoop. Repeating it here would buy a window in
which a package the user depends on is genuinely absent, for a phase whose
commands cannot cause that. So the lock was proven usable and never used.

**The shallow-bucket refusal.** `adopt`'s message names `--unshallow` when the
bucket is shallow; all three buckets here are full clones, and no throwaway
shallow clone was constructed. Unit-tested only.

**`update <packages>` — the positional scope filter.** Every run here was
whole-file. The "nothing else is rewritten and no entry is dropped" promise is
covered by `tests/update.rs` and by nothing on this machine.

**A genuinely stale machine for `--offline`.** The warning was seen and is
correct, but because the buckets were current, `--offline` and a fetching run
produced byte-identical locks. `--offline` was only *proved* to matter by the
artificial rewind in Q3.

**Anything winget.** This `pkg.toml` declares no `[winget]` section, so
`update`'s carry-through warning — the one `docs/phase3-notes.md` records as the
only survivor in `src/update.rs`, closed by a test — never fired here.

**A lock or opt naming an undeclared bucket.** This dogfood did not exercise
it — every bucket the machine's lock and opts named was declared. It was
untested at the time this dogfood ran, but was subsequently closed by the
scoped re-review's `9e8092d`: that commit gave the case its own
`BucketChoice::Undeclared` exit, distinct from `NotFound`, with three tests
and a negative control (see `docs/phase3-notes.md`, "Closed by the scoped
re-review of the final fix wave"). The `NotFound { searched: [stated] }`
shape this entry originally named no longer exists for this case.

**Medium integrity.** Everything ran over plain elevated `ssh`, which is why
`actionlint`, `antigravity` and `zellij` are unreadable throughout. That quirk
is not new; its *consequence* is (see Q1), and the honest statement is that this
dogfood measured dotpkg against a slightly wrong picture of the machine and said
so at every point where it mattered.

## Method, unchanged and still non-negotiable

- `ssh a14` does not work from the development machine — `~/.ssh/config` has an
  `Include ~/.colima/ssh_config` the sandbox cannot read. Use
  `ssh -F /dev/null -o BatchMode=yes kln@100.83.225.100` and `scp -F /dev/null`.
- Quoting through `ssh` breaks PowerShell. `scp` the script up, run it with a
  constant `-EncodedCommand` (UTF-16LE base64) that redirects into
  `$env:TEMP\dotpkg-out.txt`, `scp` that back.
- **Every file dotpkg parses is written with `[System.IO.File]::WriteAllText`
  and a BOM-less `UTF8Encoding($false)`.** PowerShell 5.1's `Set-Content
  -Encoding UTF8` writes a BOM and `serde_json` rejects it with `expected value
  at line 1 column 1`.
- Read an app's version through the reparse point's own `.Target`, not by
  joining `current\manifest.json` — the latter silently returns `False` for the
  three junction-affected apps.
- Never start or stop `kanata`. Record its process name and PID before and after.
- Build from a tarball of `Cargo.toml`, `Cargo.lock`, `src/`, `tests/` — never
  `target/`, never `.git/` — reusing `C:\Users\kln\dotpkg-build`.
- Confirm `update --help` and `adopt --help` show this phase's flags before
  trusting a single result. The binary under test must be the one just built.
