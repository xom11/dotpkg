# Carried forward into Phase 2b

Findings from building Phase 2a (`status`, made truthful) that Phase 2b
(`apply`, the executor) must handle. Every one was found by review, by
mutation testing, or by the dogfood run — none guessed. Recorded here because
the build ledger they came from is scratch and gets deleted.

`docs/phase2-notes.md` still holds the Phase 1 items 2a did not touch; this
file does not repeat them.

## Read this first

**The `git show` restore path belongs to 2b, not Phase 3.** The approved design
assigns it to Phase 3, but `git show <commit>:bucket/<app>.json | scoop install`
is *how `apply` installs a locked package*. Without it 2b can only resolve
latest — breaking "never degrade silently" — or prune, which on a14 is a
guaranteed no-op because `state.json` is empty. Phase 3 keeps `update` and
`adopt`. Full reasoning in `docs/specs/2026-08-08-phase2a-design.md`,
"Corrections to the approved design".

**`scoop install <path>.json` takes the app name from the filename.** Staging a
manifest as `%TEMP%\dotpkg-tmp-9f2.json` installs an app called
`dotpkg-tmp-9f2`. Each staged manifest needs its own directory so the file can
stay named `<app>.json`.

## Measured: how a version change actually happens, and what the design got wrong

Measured 2026-08-08 on a14, scoop **0.5.3** (`b588a06e`), in a throwaway
`$env:SCOOP` root using two real pinned `dos2unix` manifests recovered with
`git show` from the main bucket (7.5.5 at `39160de954ce`, 7.5.6 at
`8042a958a4e3`). The real `~/scoop` was untouched throughout — 31 apps before
and after — and the probe root was removed.

**Every version change is uninstall + install. Upgrades are not safer than
downgrades.** This contradicts the approved design, which states "**Downgrades
are the one irreducible gap**, because they are uninstall + install" and marks
`↓` as the arrow that earns a warning. Measured, `^` carries exactly the same
risk. Whatever 2b does about the confirmation prompt, it must not imply that an
upgrade is the safe direction.

What was tried, and what each did:

| Command | Result |
|---|---|
| `scoop install <path>/app.json` (not installed) | works, exit 0 |
| `scoop install <path>/app.json` (installed, different version) | exit 0, `WARN` on stdout, nothing changes — corrected below |
| `scoop install -f …` | `Option -f not recognized` |
| `scoop install --force …` | `Option --force not recognized` |
| `scoop update <path>/app.json` | exit 1, no output |
| `scoop reset app@<version present on disk>` | works, relinks shims |
| `scoop reset app@<version not on disk>` | **exit 0, no output, nothing changes** |
| `scoop uninstall app` then install the pin | the only sequence that works |

**Corrected 2026-08-08, remeasured for Phase 2b-2.** The row above originally
read "exit 0, no output, nothing changes". Remeasured with
`System.Diagnostics.Process.ExitCode` (`docs/measurements-2026-08-08-scoop-exit-codes.md`,
tests E4/E5), it is not silent: scoop prints `WARN  '<app>' (<version>) is
already installed.` / `Use 'scoop update <app>' to install a new version.` on
**stdout** — and it names the version **already installed**, not the version
just requested. Installing fzf 0.74.2 over an installed 0.74.1 prints the
same line naming 0.74.1. "Exit 0, nothing changes" still holds; only "no
output" was wrong.

**There is no force flag.** The authoritative list from `scoop help install` is
`-g/--global`, `-i/--independent`, `-k/--no-cache`, `-s/--skip-hash-check`,
`-u/--no-update-scoop`, `-a/--arch`. Nothing replaces an existing install.

**`scoop reset` is not a shortcut around this.** It can relink to another
version *already on disk*, but dotpkg's only version-change mechanism is
uninstall + install, and the uninstall deletes the old version directory. After
a change, exactly one version directory exists. The cheap in-place relink is
therefore unavailable to dotpkg by construction.

**The executor cannot trust exit codes.** Two distinct silent-success traps were
observed above, both returning 0 while doing nothing. An `apply` that checks
only the exit code will report an upgrade that never happened — a "never
degrade silently" violation produced by the tool it orchestrates. **Verify the
resulting on-disk state after every mutation, not only after uninstall** as the
design's error table currently requires.

**One piece of good news: `scoop uninstall` keeps persistent data by default.**
`-p/--purge` is opt-in, so the uninstall+install window risks binaries and
shims, not the user's data under `persist`.

**`install.json` records `url`, not `bucket`, when installing from a path.**
Measured: `{ "url": "…\\_stage\\7.5.5\\dos2unix.json", "architecture": "64bit" }`.
Two consequences. `Installed.bucket` is `None` for everything dotpkg installs,
so nothing downstream may depend on it. And the recorded path is dotpkg's own
staging directory — stage manifests somewhere stable such as
`%LOCALAPPDATA%\dotpkg\manifests\<app>\<version>\<app>.json`, never `%TEMP%`,
or the app is left pointing at a path that no longer exists.

**Not usable as measured:** `scoop help install` documents
`scoop install \path\to\app.json@version`, but invoked with a quoted path it
produced no exit code and no effect. It is redundant anyway — a pinned manifest
already carries its own version — so it is recorded only so nobody re-tries it
expecting a way around the above.

## Must not reach an `apply` that can execute

**All three items in this section were closed by Phase 2b-1**, and the first
turned out to be wider than recorded here — the same folding bug also lived in
`pkg.lock` and `state.json`, where a colliding pair silently kept the first key
and the *last* value. The whole-branch review found that by running it. Folding
now happens once, in `crate::model`, for every `Name`-keyed map.

The original text is kept below because the *shapes* still matter: they are
what a future `Name`-keyed map will reproduce if it deserializes directly
instead of folding.

**Two declared names differing only in case produce two `Install` actions.**
`packages = ["fzf", "FZF"]` collapses to one entry in the declared set — `Name`
folds case — but the declared loop iterates the `Vec` twice. `change_count()`
says 2 for one app. Needs a dedupe, or a parse-time rejection.

**A duplicated `[scoop.opts]` key is swallowed silently.** Declaring both
`python = { arch = "64bit" }` and `Python = { arch = "arm64" }` yields **one**
entry that displays as `python` and carries `Arm64` — first key, last value, no
error. `deny_unknown_fields` catches a typo; nothing catches this. Same root
cause as the item above and probably the same fix.

**`Scoop::discover()` does not canonicalise its root.** If `$SCOOP` points
through a junction, a `subst` drive, or an 8.3 path, `scan()` still works — it
opens through the alias — but `sysinfo` reports resolved paths, so
`running_apps` prefix-matches nothing and returns empty **with no warning**.
`nodejs` and `rustup` have no other running signal, so in 2b they become
prunable while running. Cheap mitigation: if `scan()` finds apps and
`running_apps` matches nothing across the whole process table, warn.

## Test coverage gaps, in the order they would hurt

~~**`sys::running_processes()` is the crate's only untested OS boundary.**~~
**Closed before merge.** Mutating `exe: p.exe()...` to `exe: None` silently
disabled the entire path signal — the only signal `nodejs` and `rustup` have —
and left the suite green. `the_real_process_table_yields_at_least_one_readable_executable_path`
in `src/sys.rs` now covers it, and was confirmed to go red under exactly that
mutation with `no process reported a readable executable path -- path matching
is dead`. It is the one test in the crate that touches the OS, and it asserts
only a floor a test process can always meet: it can read its own image path.

**`SCOOP_HELPERS` is compared against `Name::key()`, but every helper fixture is
already lowercase.** Reverting to a display comparison leaves the suite green.
One-word fix: make a fixture `installed("7Zip", "26.01")`.

**Three branches with no test at all:** `render.rs`'s `drift_count() == 0` path
(flipping the guard prints `, 0 architecture drift` on every ordinary run,
uncaught); `scan()`'s JSON-parse-error branch — the test that reads as covering
it actually covers the missing-`version` branch; and `scan()`'s `read_dir`
iteration branch, which is deliberately untested because no portable trigger
exists and is recorded as inspection-verified rather than counted as covered.

**`Name`'s `Hash` is untested and currently dead** — no `HashMap`/`HashSet` of
`Name` exists anywhere. Hashing `display` instead of `key` would pass
everything. Write the test when the first `HashSet<Name>` appears.

## Smaller, real, and cheap

- `src/sys.rs`'s `normalize()` has no non-empty guard, unlike its manifest-side
  twin in `backend/scoop.rs`. A process named `.exe` enters `Running.names` as
  `""`. Harmless today — nothing empty can match — but the asymmetry is a trap.
- `if running.covers(inst) { Skip } else { <action> }` appears twice in
  `plan()`. Two different decisions, so inlining is defensible, but a private
  helper returning `Option<Action>` would collapse both.
- `running_apps` allocates a `String` per process per root because `fold(exe)`
  is an unnamed temporary; `main.rs` clones every process name. Negligible at a
  once-per-run process scan.
- `tests/planner.rs`'s ordering test now exercises `ArchDrift`, but whoever adds
  an eighth `Action` variant must extend that fixture too — the compiler forces
  the match arm, not the scenario.

## Dogfooding a14 — three things this run paid for

**The medium-integrity scheduled-task technique from Phase 1 no longer works as
written.** `New-ScheduledTaskPrincipal -UserId 'kln' -LogonType
Interactive|S4U -RunLevel Limited` registers fine and then sits at **Queued**
forever, on a machine with an active, unlocked interactive session. What works
is cloning the `<Principal>` XML of an already-registered task known to fire
(`AHKWatchdog`): raw **SID** as `UserId`, `<LogonType>InteractiveToken</LogonType>`,
and **no `<RunLevel>` element** at all. Which of those three differences is the
actual cause was not bisected. Phase 2b will need this technique for its own
non-read-only checks, so budget for it.

**`runas /trustlevel:0x20000` is not a substitute.** It filters group membership
correctly but does not relabel integrity to Medium on this machine. Tested
directly; do not re-try expecting otherwise.

**`query` / `quser` / `query session` do not exist on a14.** Their silence reads
as "nobody is logged on", which is a plausible and wrong explanation for the
Task Scheduler symptom above. Check `explorer.exe`'s owner instead.

## Settled by measurement, do not re-litigate

**Kanata was never protected before Phase 2a, and this is now measured rather
than argued.** `docs/phase2-notes.md` claimed the spec's example "happens to
match". It does not. On a14, kanata's only live process is
`kanata_windows_tty_winIOv2_arm64` — verified twice, independently, with zero
`Kanata.exe` shim processes alive. The pre-2a code compared the package name
`kanata` against process names and would have found nothing. After 2a,
`dotpkg status` prints `! scoop kanata running -- stop it first`.

The over-collecting depth-first walk in `declared_executables` turns out to
catch both the real filename and the shim alias in every `[filename, alias]`
pair, so detection no longer depends on how the app was launched. That is
stronger than the design predicted, and it is the payoff of choosing
over-collection as the safe default.

**The machine's architecture picture: 20 `arm64`, 10 `64bit`.** Of the ten,
`python` is deliberate and `dark`/`innounp` are helpers, leaving roughly seven
emulated for no stated reason. The design document's "17 emulated packages"
described an earlier state of the machine. This is the number 2b's decision
about whether `apply` should act on drift must be taken against — reinstalling
seven packages is a real cost, and every reinstall is an uninstall plus an
install.

---

# Carried into Phase 2b-2 (the executor)

Added 2026-08-08, after Phase 2b-1 shipped `apply --prepare`. Everything above
this line is either closed or still open as marked; everything below is new.

## What 2b-1 established that 2b-2 depends on

`apply --prepare` recovers each locked manifest with `git show`, stages it at
`%LOCALAPPDATA%\dotpkg\manifests\<app>\<version>\<app>.json`, and fetches it
with `scoop download`, which verifies hashes. Dogfooded against the real
`~/scoop`: 23 of 25 declared packages attempted (2 correctly skipped as
running), all recovered, all versions matched, all 24 fetches verified, and a
deliberately corrupted commit failed loudly while staging nothing and leaving
the other 22 untouched.

**The plan's prediction that upstream rot would break some fetches did not
hold**, even under a deliberate adversarial probe against the smallest bucket
and an older, never-cached release. Recorded as a falsified prediction rather
than smoothed over. It may simply mean these particular pins are young.

## Things that will bite the executor

~~**`Outcome::ReadyToRemove` is still attachable to an `Install`.**~~ **Closed
by `626a276`** ("Add the whole-run executor, the recovery file, and the
running re-check"). The type still does not bind an outcome to an action
variant — that part of the risk is unchanged — but `plan_to_steps`
(`src/apply.rs`) now matches on `(&p.action, &p.outcome)` together, with a
comment quoting this note almost verbatim: "Branch on the ACTION, never on
the outcome." The split from `Ready { manifest: Option<PathBuf> }` that
killed the "prune vs. fallback" ambiguity in `None` was already in place
before this phase.

**Still open.** Nothing produced by real code is asserted to be
`ReadyToFetch`. `stage_and_fetch` (`src/apply.rs`) calls `scoop.download()`
directly, not through `Mutator`; `src/execute.rs`'s `Mutator` trait still
declares only `uninstall` and `install`. The call site says so itself: "a
later task puts it behind the `Mutator` trait, as `install` and `uninstall`
already are." No commit in this branch moves it there, so the only test that
stages for real still lands in `Failed` (no scoop binary on the test
platform), and every `ReadyToFetch` value asserted in the suite is still
hand-built rather than produced.

~~**`tests/cli.rs`'s `Snapshot` records path names, not content.**~~ **Closed
by `cd8420f`** ("Make the cli Snapshot compare content, and forbid a fake
scoop binary"). `Snapshot` now stores `(path, DefaultHasher content hash)`
pairs instead of bare paths. The commit message records the measurement that
justified it: injecting the exact `state.json` write this phase adds left
`cargo test --test cli` at 3/3 green under the old path-only form, while the
file's content was replaced.

~~**`main.rs`'s Apply arm is the third inline copy of load → scan → plan.**~~
**Corrected: it was the *second*, not the third** —
`docs/specs/2026-08-08-phase2b2-executor-design.md` already had the count
right (`:64`, `:100`); this note did not. **Closed by `0712445`** ("Wire up
the executor: one driver, one question, three exit codes"), which extracted
`apply::load_everything` for the Apply arm instead of inlining a second copy
of load → scan → plan into the newly-destructive version. `main.rs` still has
two separate assemblies today — Status stays fully inline, and `plan()`
itself is still called once from each arm — but no third one was ever added.

~~**`commit` is still unvalidated where it reaches git argv, and the reason it
is harmless today is not the obvious one.**~~ **Closed by `741bf91`** ("Refuse
a pkg.lock commit that is not a hash"). Verified against a real git
repository: this note's own reasoning was itself incomplete. `cat-file -e`
does reject a leading dash, but it also **accepts** `main`, `HEAD`, `@`, and
`refs/heads/main` — so `commit = "main"` sailed through the exact protection
this note described, and when the bucket tip happened to carry the same
version, staged the tip anyway: a lock that looks pinned silently tracking
latest. `commit` must now be 40 or 64 lowercase hex characters, enforced in
two places — `lock_coherence_guard` (whole-run, before the plan is built) and
`Scoop::stage`'s `ensure_commit_hash` (re-checked, because `stage` is a public
API a later phase calls from elsewhere) — rather than deferred to Phase 3 as
this note originally proposed. Full reasoning:
`docs/specs/2026-08-08-phase2b2-executor-design.md`, "Rev-locking: `commit`
must be a hash".

**Still open.** `mass_prune_guard` checks scoop only. `Config` already has a
`WingetSection`. The function reads like "the fence" and is not one yet; it
must grow a backend loop in the same commit as the winget backend. Confirmed
unchanged in `src/apply.rs`: it still reads only
`declared.scoop.packages.is_empty()` and `state.owned_count(SCOOP)`. The
winget backend is out of scope for Phase 2b-2 (design doc, "Non-goals"), so
this is deferred, not forgotten.

**Still open.** Staging paths are not content-addressed. Confirmed unchanged:
`Scoop::stage_text` (`src/backend/scoop.rs`) still writes to
`staging_root.join(app.key()).join(version)` — keyed on app and version only,
never on `commit`. Re-pinning the same version to a different commit still
silently overwrites the file that an installed app's `install.json` already
points at.

## Method, for whoever runs the next dogfood

**A lock that exactly matches the installed versions gives `apply --prepare`
zero actions.** The natural lock to generate for a dogfood is therefore a no-op,
and a separate deliberately-divergent "exercise" lock is needed to test
anything. Budget for it rather than rediscovering it.

**The medium-integrity scheduled-task workaround is still required** — see the
section above. It has now been used successfully twice.
