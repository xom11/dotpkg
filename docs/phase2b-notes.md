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

**Unresolved, and it must be measured before the executor is written.** The lock
pins a version; what command applies it? `scoop install <manifest>` refuses an
already-installed app. `scoop update <app>` goes to the bucket's *current*
manifest, not the pinned one. Whether `scoop install -f` uninstalls the old
version or only adds a directory and moves `current` was never determined. If
it turns out every version change is uninstall + install, then the design's
claim that "downgrades are the one irreducible gap" is wrong and the dangerous
window is far wider than the spec describes. Measure this in a throwaway
`$env:SCOOP` before writing code against it.

## Must not reach an `apply` that can execute

These are safe today only because `status` acts on nothing.

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
