# Dogfood: Phase 4, the winget backend — a14, 2026-08-10

The first phase whose commands touch winget for real. `update`, `adopt` and
`apply --prepare` are all read-only or bounded for winget: `apply` cannot
install, upgrade or remove a winget package at all yet
(`Capability::ReportsOnly`), and the only mutating winget command this session
ever ran was `winget source update --name winget` — via `dotpkg update`
itself, not typed directly — already the one command Phase 4's design
measured inert. **No `winget install`, `winget uninstall`, `winget upgrade` or
`winget pin add` ran at any point.**

Branch `phase4-backend-winget` at `12d9ba8`, built natively on a14 (the same
build this session's Windows suite ran against), against the real, installed
`winget 1.29.280` and `scoop 0.5.3`. `C:\Users\kln\pkg.toml` and
`C:\Users\kln\dotpkg-build` were reused and left untouched — every dotpkg
command below ran against throwaway config/lock/state copies under a scratch
`dogfood\` directory inside the reused build tree, removed at the end.

**The headline: `dotpkg adopt --backend winget` drops a same-line trailing
comment on an array's last element when it appends a new one.** Found by
running the command for real, not by review — see Q5 and "The defect this
dogfood found" below.

Session ran under the same elevated `ssh` Phase 3 used, so the same junction
quirk applies throughout: `actionlint`, `antigravity` and `zellij`'s
`manifest.json` cannot be traversed and `scoop: ... cannot read manifest.json`
warnings appear in every scan below. Not new, not this phase's concern, and
consistent with every prior dogfood.

## What the machine is

Captured before anything ran.

| | |
|---|---|
| scoop apps | **31** (unchanged from Phase 3) |
| `kanata` | `kanata_windows_tty_winIOv2_arm64`, PID **14276**, started 08/10/2026 07:25:58 |
| `explorer` | PID **9576** |
| `pkg.toml` | 449 bytes, sha256 `32A238FF…` — **byte-identical to Phase 3's own recorded hash** |
| `%LOCALAPPDATA%\dotpkg` | does not exist |
| `winget` | `v1.29.280`, at `C:\Users\kln\AppData\Local\Microsoft\WindowsApps\winget.exe` |
| `winget list` | 141 rows, 126 distinct ids |

**A script bug, disclosed rather than smoothed over.** An early baseline pass
read `$env:SCOOP\cache` in a `-NoProfile` PowerShell session and got a false
`0` — `SCOOP` is only set inside this machine's interactive `$PROFILE`, which
`-NoProfile` skips, so the path resolved to nothing and `Get-ChildItem`
silently returned empty. `dotpkg.exe` itself was never affected — `Scoop::
discover()` does not depend on that env var, and its own output matched the
machine correctly throughout every run in this session (`app count: 31`
against the real, hardcoded path). The false `0` was caught before it reached
any conclusion: the real cache count, read against `C:\Users\kln\scoop\cache`
directly, is **82**, two more than Phase 3's own recorded ending value of 80.
Nothing in this session declared or touched a scoop package, so nothing here
could have grown it — recorded as unexplained rather than attributed, the same
honesty Phase 3 applied to the two things that moved in its own session.

## Method: how the 42/84 split was independently checked

`Winget::scan`'s own doc comment states the rule: group `winget list`'s rows
by id; a group is `opaque` if any row lacks a `Source`, if any row's version
starts `"> "`, or if the group disagrees on version; otherwise it is one
`Installed` entry, warned about once if it collapsed more than one row.

This was checked three independent ways against the **same, single, live
capture** of `winget list` and `winget export --include-versions
--accept-source-agreements`, taken back to back:

1. **`dotpkg status`'s own printed warnings and report lines** — the real
   compiled binary, the real machine.
2. **A from-scratch Python re-implementation of `parse_list` +
   `rows_to_scan`**, run against the raw captured bytes, independent of the
   Rust code entirely.
3. **`winget export`'s own JSON**, a third, independent tool.

All three agree exactly.

## Q1 — does `Winget::scan` agree with `winget export --include-versions` on
## the source-backed ids, and does it account for every row export drops as a
## duplicate?

**Yes, on every count, measured three independent ways.**

`winget list` on this machine: **141 rows, 126 distinct ids**, splitting
**84 opaque-for-no-`Source`, 5 opaque-for-a-`>`-version-or-disagreement, 37
installed** (84 + 5 + 37 = 126). Of the 42 ids that have a `Source` at all
(37 + 5), **57 rows** carry them — 15 more rows than ids, all duplicates of an
id already counted once.

`dotpkg status --config C:\Users\kln\pkg.toml` (unmodified, real) printed
**37** `? winget <id> ... (unmanaged -- no action)` lines and **5** per-id
warnings for the disagreeing/`>`-prefixed ids (`7zip.7zip`,
`Microsoft.UI.Xaml.2.8`, `Microsoft.VisualStudio.2022.BuildTools`,
`Microsoft.WindowsAppRuntime.1.8`, `Microsoft.WindowsAppRuntime.2`) — 42
source-backed ids, exactly.

`winget export --include-versions` wrote **42** `PackageIdentifier` entries,
no duplicates within the file itself. **The set of 42 ids from `dotpkg
status` and the set of 42 ids from `winget export` are identical — zero
symmetric difference** — including that export keeps `26.02` for
`7zip.7zip` (the greater of its two disagreeing versions) and preserves the
`"> 17.14.37"` string verbatim, exactly as `PROVENANCE.md` already recorded.

The independent Python re-parse of the raw `winget list` bytes confirms the
row/id split precisely: 57 source-backed rows collapsing to 42 ids is **15**
rows short of one-per-id — the 15 rows `winget export` silently drops as
duplicates.

This machine's live state today is **numerically identical** to the one
`tests/fixtures/winget/PROVENANCE.md` recorded (same 141/126/84/42/57/15,
same named ids in every category) — no drift since the fixtures were
captured.

## Q2 — does `scan` put exactly the sourceless rows into `opaque` and none
## into `installed`?

**Yes — 84 of them, and the spec's own number needed correcting, not the
code's.**

The spec's Q2 says "83 sourceless rows." `PROVENANCE.md` already explains why
that is stale: a `winget source update` between the spec being written and the
fixtures being captured added one new source-index row
(`Windows Package Manager Source (winget-font) V2`), moving 83 → 84. Today's
live measurement is **84**, matching the aggregate warning `dotpkg status`
itself prints (`"84 installed entries have no winget Source ..."`) and
matching the shipped fixtures exactly. **None of the 84 appear in `installed`**
— confirmed by the independent Python re-parse, and by the fact that zero of
the 37 `? winget` unmanaged lines or the 5 opaque-warning ids overlap with the
sourceless set.

One thing this dogfood did **not** get to exercise: an id with *some* rows
carrying a `Source` and *some* without, in the same group. On this machine the
source-backed and sourceless id sets are perfectly disjoint (verified: zero
overlap), so that branch of `rows_to_scan`'s "any row has no Source" check
stayed unit-tested only.

## Q3 — does `update` produce a lock for all 17 that `apply` then reports
## rather than acts on, and does `apply`'s exit code become 1 for that reason?

**Yes, fully, and named per line.**

17 real, installed, source-backed winget ids were declared in a throwaway
`[winget]`-only `pkg.toml` (no `[scoop]` section — safe under `mass_prune_
guard`, since nothing is owned in a fresh, isolated `state.json` regardless of
what is declared):

```
dotpkg update --config dogfood\pkg-17.toml --lock dogfood\lock-17.lock

  17 changed, 0 unchanged, 0 could not be resolved.        EXIT = 0
```

All 17 resolved (`winget show --id <id>` for each) and the lock was written
with all 17 `[winget."<id>"]` entries, `pin = "version-only"`. This run also
exercised the real, unscoped `winget source update --name winget` call `update`
makes before resolving (see Q6).

```
dotpkg apply --prepare --config dogfood\pkg-17.toml --lock dogfood\lock-17.lock \
  --state dogfood\state-17.json

  0 of 0 changes ready, 0 failed, 8 skipped, 0 not locked.
  Nothing has been changed.                                 EXIT = 1
```

**8 of the 17** had drifted between "latest, as `update` just resolved it" and
"currently installed" (real time passed between the two winget calls, and some
of these apps auto-update): `Brave.Brave`, `ByteDance.Lark`,
`Discord.Discord`, `Google.Chrome`, `JanDeDobbeleer.OhMyPosh`,
`Obsidian.Obsidian`, `OpenAI.Codex`, `Tailscale.Tailscale`. Each is printed
individually, named, with both versions and the reason:

```
  ! winget Brave.Brave    151.1.93.134 -> 151.1.93.132 -- reported only, dotpkg cannot install or remove winget packages yet
```

`0 failed, 0 not locked` — the exit code is not a verdict on the lock, exactly
Phase 3's own Q1 finding restated for winget. **The floor to `EXIT = 1` is
`floor_exit_code`'s `has_reported_only` branch**, and every one of the 8 lines
that caused it names itself as the reason, in place, rather than requiring the
reader to infer it from the exit code alone.

**A genuine rendering defect surfaced here, not fixed.** `apply --prepare`
prints the plan twice — once through the ordinary `render(plan)` path (well
spaced: `{name:<14} {version:<24}`, an explicit literal space between fields)
and once through `render_preparation`'s per-item `prepared_line`
(`src/render.rs:217`: `format!("  {marker:<8}{backend:<6} {name:<13}{rest}\n")`
— **no literal space between `{name:<13}` and `{rest}`**, relying entirely on
padding that a 13-character-or-longer name exhausts). Scoop package names in
this project's own `pkg.toml` are all under 13 characters, so this never
showed before; 8 of the 17 real winget ids in this run are 13 characters or
longer, and the second table glues the version onto the name with zero
separator:

```
  !       winget ByteDance.Lark7.72.9 -> 7.73.11 -- reported only, ...
  !       winget JanDeDobbeleer.OhMyPosh29.36.0.0 -> 30.6.3 -- reported only, ...
  ?       winget Microsoft.WindowsAppRuntime.1.61.6.9             (unmanaged -- no action)
```

Legible only because the id and version happen not to share digits at the
boundary. Reported for `docs/phase4-notes.md`; **not changed** — this dogfood
is read-only for source, same as every command it ran was read-only for
winget.

## Q4 — does any declared winget package fail `show -v <pinned>` today?

**No, on the one path this dogfood exercised — and the `0x8A150017` path was
independently confirmed live rather than left as documentation.**

`Winget::resolve_installed` (`show --id <id> -v <installed version>`) is the
actual caller of that path, and it is reached by `adopt`, not by `update`
(which calls `resolve_latest`, a different method, for all 17 above — so the
17-package run above does **not** exercise this question at all). This
dogfood ran `adopt --backend winget` once, against a real, installed,
undeclared id (`Vivaldi.Vivaldi`, not one of the 17):

```
dotpkg adopt --backend winget ... Vivaldi.Vivaldi

  + winget Vivaldi.Vivaldi adopted (winget confirms this version is still in its index)
  1 adopted, 0 refused. Nothing installed and nothing removed.   EXIT = 0
```

Succeeded — the installed version was still in the index. So on this machine,
today, the failure path stayed undemonstrated through `dotpkg` itself, exactly
as the spec anticipated as the likely outcome.

**Independently confirmed the path is real, not dead code**, by calling raw
`winget` directly (never through `dotpkg`, and `show` never installs or
removes anything):

```
winget show --id ajeetdsouza.zoxide -v "0.0.0-dotpkg-dogfood-does-not-exist" --disable-interactivity

exit code: -1978335209   (== 0x8A150017, decimal, signed)
"No version found matching: 0.0.0-dotpkg-dogfood-does-not-exist"
```

This exactly matches `NO_VERSION_FOUND` (`src/backend/winget.rs:558`,
`-1978335209`). The constant is correct against today's `winget.exe` on this
machine; `dotpkg`'s own code path to it just was not naturally triggered by
anything installed here.

**Not covered**: `adopt --backend winget` against all 17, or against any of
the 5 opaque ids (which `resolve_installed` should refuse before spawning
anything, per its own doc comment about the `"> "` case) — only the one id
above was adopted.

## Q5 — is `pkg.toml` byte-identical after `adopt` except for the added line?

**No. A real defect, found by running the command, not by review.**

Starting file (**150 bytes**, confirmed against `adopt`'s own `.bak` of the
pre-run file):

```
# dotpkg dogfood config -- this comment must survive `dotpkg adopt`
[winget]
packages = [
  "ajeetdsouza.zoxide",  # kept for comment-survival check
]
```

After `dotpkg adopt --backend winget ... Vivaldi.Vivaldi` (137 bytes):

```
# dotpkg dogfood config -- this comment must survive `dotpkg adopt`
[winget]
packages = [
  "ajeetdsouza.zoxide",
  "Vivaldi.Vivaldi",
]
```

**The trailing comment `# kept for comment-survival check` is gone.** The file
got *shorter* despite gaining a whole new declared package — the tell that
something besides "one line added" happened. Diffed against the `.bak`
`config_edit::save` itself wrote before touching the file (the same mechanism
Phase 3's dogfood trusted for its own byte-identical claims):

```diff
-  "ajeetdsouza.zoxide",  # kept for comment-survival check
-]
\ No newline at end of file
+  "ajeetdsouza.zoxide",
+  "Vivaldi.Vivaldi",
+]
```

**Reproduced locally, isolated from the Windows machine entirely**, with a
temporary unit test against `config_edit::add_winget_package` (written,
run, and reverted — not part of this commit): a comment trailing a
**non-last** array element (the existing `HAND_WRITTEN_WINGET` fixture's own
shape: `"Git.Git", # version control` followed by `"OpenAI.Codex",`) survives
correctly. A comment trailing the array's **last** element, immediately before
the closing `]`, does not. That is precisely why
`a_winget_package_is_added_and_every_comment_survives` (`src/config_edit.rs`)
did not catch this: its fixture's only comment sits on a non-last element, and
the assertion is `out.contains(...)`, which cannot tell "still attached to the
right line" apart from "moved" — the same class of gap `contains`-only
assertions caused in Phase 3's own review notes.

**Root cause, read from the code, not fixed**: `add_winget_package`
(`src/config_edit.rs`) calls `packages.set_trailing("\n")` unconditionally
inside its `multiline` branch after pushing the new element. When the old
last element's trailing comment lives in the array's own `trailing` decor
(the text between the last comma and `]`, which is exactly where a
same-line comment on the last element ends up in `toml_edit`'s model), that
call overwrites it outright.

## Q6 — does `winget source update --name winget` leave the installed set
## unchanged?

**Yes — confirmed byte-identical, not merely field-by-field.**

`winget list` was captured immediately before and immediately after the `dotpkg
update` run in Q3 (which invokes `winget source update --name winget`
unscoped-by-`--offline` internally, exactly once). Both captures hash to the
same SHA256 (`5C598929…`), so every field of every one of the 141 rows,
including ordering, is identical — a strictly stronger check than the
field-by-field diff the spec asked for, and it found no difference at all.

## Q7 — does a `pkg.toml` declaring a winget id in the wrong case get reported
## rather than silently rewritten?

**Yes, exactly as designed.** `vivaldi.vivaldi` (lower-case; the real id is
`Vivaldi.Vivaldi`) declared alone in a throwaway `pkg.toml`:

```
dotpkg update --config dogfood\pkg-wrongcase.toml --lock dogfood\lock-wrongcase.lock

warning: vivaldi.vivaldi: pkg.toml declares this as "vivaldi.vivaldi", but winget
matches it as "Vivaldi.Vivaldi" -- pkg.lock records the canonical spelling;
pkg.toml is left as you wrote it.

  + winget vivaldi.vivaldi 8.1.4087.62                (new pin)
  1 changed, 0 unchanged, 0 could not be resolved.     EXIT = 0
```

The lock was written keyed by the **canonical** spelling
(`[winget."Vivaldi.Vivaldi"]`), and `pkg.toml` — never touched by `update` at
all — still reads `vivaldi.vivaldi` verbatim. Reported, not rewritten, exactly
the design's claim.

## The defect this dogfood found, and what was not done about it

Restated together because Q5 is where it lives: `dotpkg adopt --backend
winget` silently drops a same-line trailing comment attached to a `[winget]
packages` array's last element when it appends a new one, because
`add_winget_package`'s `packages.set_trailing("\n")` overwrites the array's
own trailing decor without preserving whatever comment text lived there. Real,
reproducible on the machine that found it and, independently, in a from-source
Rust reproduction. **Not fixed** — Step 4 of this task is read-only, and
fixing a `src/config_edit.rs` defect discovered by the dogfood is exactly the
kind of change that belongs in `docs/phase4-notes.md`'s carried-forward list
for whoever picks it up next, not folded into this task silently.

## What this dogfood deliberately did NOT cover

**`apply` without `--prepare`.** `apply` cannot act on a winget package at all
yet (`Capability::ReportsOnly`), and the throwaway configs here declared zero
scoop packages, so there was nothing for an un-flagged `apply` to install,
upgrade or remove either way — but it was never run, matching Phase 3's own
restraint.

**`adopt --backend winget` against more than one real package.** Only
`Vivaldi.Vivaldi` was adopted. The 36 other unmanaged, installed, source-backed
ids were left alone; so were the 5 opaque ones, whose `resolve_installed`
refusal path (the `"> "` short-circuit specifically) is unit-tested only.

**A case mismatch found naturally on the machine.** All 42 real source-backed
ids already match their own canonical spelling exactly (`winget export`'s own
ids). Q7 had to be constructed (`vivaldi.vivaldi`, deliberately mis-cased) —
this machine gave the mechanism nothing to catch on its own.

**Any winget package actually installed, upgraded or removed by dotpkg.**
Structurally impossible this phase — there is no winget executor yet.

**Medium integrity.** This session ran over the same elevated `ssh` as every
prior dogfood; `actionlint`, `antigravity` and `zellij` were unreadable
throughout for the same junction-traversal reason Phase 2a first found. Not
new, not re-verified at medium integrity this time.

## The machine afterwards

All **31** scoop app directories: unchanged (not individually re-hashed this
session — no scoop package was ever declared or acted on, so nothing could
have moved them, and `apply --prepare`'s own `Nothing has been changed` is the
authoritative statement for the one run that could have staged anything).

`kanata` still `kanata_windows_tty_winIOv2_arm64`, PID **14276**, same start
time (07:25:58) — never started, never stopped. `explorer` still PID **9576**,
unchanged.

`pkg.toml`: sha256 `32A238FF…`, 449 bytes — **identical to before this session
and to Phase 3's own recorded hash.** `pkg.toml.bak`: does not exist — `adopt`
never touched the real file, only throwaway copies under `dogfood\`.
`%LOCALAPPDATA%\dotpkg`: still does not exist.

Scoop cache: **82**, same as measured at the (corrected) start of this
session — unchanged by anything this dogfood did, though its drift from Phase
3's ending value of 80 predates and is unrelated to this session (see "What
the machine is").

## Cleanup, each removal verified on its own

```
C:\Users\kln\dotpkg-build\dogfood\        removed   Test-Path False  (pkg-17.toml,
                                                     pkg-vivaldi.toml, pkg-wrongcase.toml,
                                                     lock-17.lock, lock-vivaldi.lock,
                                                     lock-wrongcase.lock, state-vivaldi.json,
                                                     winget-list-before/after.txt, all
                                                     command-output captures)
loose scripts/logs left in C:\Users\kln\dotpkg-build  removed  (baseline.ps1, envcheck.ps1,
                                                     dogfood-main.ps1, verify-and-cleanup.ps1,
                                                     the extraction tarball, and the AppleDouble
                                                     `._*` sidecar files a macOS `tar` leaves
                                                     behind — harmless to the build, cleaned
                                                     anyway)
```

Kept, matching every previous phase: `C:\Users\kln\dotpkg-build` (now holding
only `Cargo.toml`, `Cargo.lock`, `src\`, `tests\`, `target\`) and
`C:\Users\kln\pkg.toml`.

## Method, unchanged and still non-negotiable

- `ssh a14` does not work from this sandbox. Use
  `ssh -F /dev/null -o BatchMode=yes kln@100.83.225.100` and `scp -F /dev/null`.
- Every file this session wrote that dotpkg or a verification step reads back
  was written with `[System.IO.File]::WriteAllText` and a BOM-less
  `UTF8Encoding($false)`.
- **`$env:SCOOP` is unset in a `-NoProfile` PowerShell session on this
  machine** — it is only set inside the interactive `$PROFILE`. `dotpkg.exe`
  is unaffected (`Scoop::discover()` does not read it the same way), but a
  standalone verification script that reads `$env:SCOOP` directly needs its
  own hardcoded fallback or it will silently report an empty scoop root.
  Worth carrying forward for whichever session hits this next.
- Never start or stop `kanata`. Recorded its process name and PID before and
  after; both unchanged.
- Build from a tarball of `Cargo.toml`, `Cargo.lock`, `src/`, `tests/` —
  reusing `C:\Users\kln\dotpkg-build` — never `target/`, never `.git/`.
- `winget show`/`list`/`export` are read-only and were used freely, both
  through `dotpkg` and directly, to cross-check it. `install`/`uninstall`/
  `upgrade`/`pin add` never ran.
