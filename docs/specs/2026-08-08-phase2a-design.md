# dotpkg Phase 2a — a plan you can trust

**Status:** design approved 2026-08-08, not yet implemented.
**Depends on:** Phase 1 (`status`, scoop backend), merged at `e798916`.
**Supersedes parts of:** `docs/phase2-notes.md` (see "Corrections" below).

Phase 2a changes no command and writes no file. It makes `dotpkg status` print a
plan that is *true*, so that Phase 2b's `apply` executes a plan that has already
been checked against a real machine.

## Why this is its own phase

Phase 2 as the approved design scopes it — `apply`, the executor, the state
fence, the running-process skip, post-uninstall verification, plus the four
carried-forward findings — is fourteen work items in which every mistake removes
somebody's software. Phase 1 was nine items for a tool that could not write.

The split here is not arbitrary. It falls exactly on the read/write boundary:

- **2a** fixes the decision layer. Everything is testable on macOS, and the
  dogfood run against a14's real thirty apps carries no risk at all, because the
  binary still cannot execute anything.
- **2b** adds the executor, and with it the first real risk.

The payoff is sequencing: every correctness fix below gets validated against a
real machine *while the tool is still incapable of acting on it*. When 2b lands,
the plan it executes is one that has already been read and confirmed.

## Corrections to the approved design

### The `git show` restore path belongs to `apply`, not to `update`

`docs/specs/2026-08-08-design.md` assigns "Bucket commit resolution and the
`git show` restore path" to Phase 3. Resolution does belong there — it is how
`update` turns "latest" into a commit. Restore does not:

```
git -C ~/scoop/buckets/main show <commit>:bucket/fzf.json > <staging>\fzf.json
scoop install <staging>\fzf.json
```

That is how `apply` installs a locked package. Without it, `apply` has only two
options, and both are excluded by the design: `scoop install fzf` resolves the
latest version, which breaks "never degrade silently" and makes the lock a lie;
or `apply` prunes only, which on a14 is a guaranteed no-op because `state.json`
is empty and the prune fence therefore owns nothing.

The two were conflated because the spec's "Recording the lock commit" section
describes resolve and restore in one block. **Phase 2b takes restore. Phase 3
keeps `update` and `adopt`.**

A consequence worth writing down before someone hits it: `scoop install <path>`
derives the app name from the *filename*. Staging a manifest as
`%TEMP%\dotpkg-tmp-9f2.json` installs an app called `dotpkg-tmp-9f2`. The
staging layout must give each manifest its own directory so the file itself can
stay named `<app>.json`.

### `docs/phase2-notes.md` is wrong about kanata

The note says the spec's running-process example "happens to match, which is why
nine task reviews missed it." Measured on a14, it does not match. `kanata`
declares no top-level `bin`; the architecture branches declare:

```json
"arm64": { "bin": [ ["kanata_windows_tty_winIOv2_arm64.exe", "Kanata"],
                    ["kanata_windows_tty_winIOv2_cmd_allowed_arm64.exe", "Kanata-cmd"] ],
           "shortcuts": [ ["kanata_windows_gui_winIOv2_arm64.exe", "Kanata"], ... ] }
```

The executable is `kanata_windows_tty_winIOv2_arm64.exe`. Only the *shim alias*
is `Kanata`. So whether today's name comparison catches a running kanata depends
entirely on how it was launched: through the scoop shim, a `Kanata.exe` process
exists and matches; from the Start Menu shortcut (a GUI variant, which is not in
`bin` at all) or from a scheduled task pointing straight at the executable,
nothing matches.

The application whose failure costs the keyboard on the machine you would need
to fix it is the one the current guard protects *least* reliably. This is a
coin flip, not a fence.

Nothing was running at probe time, so this is established from the manifest, not
from an observed process.

## Measured on a14, 2026-08-08

Thirty installed apps, manifests read as raw bytes and parsed off-machine.
Reading through the `current` junction fails for some apps in an elevated SSH
session (the artifact settled in `docs/dogfood-2026-08-08.md`); reading the
version directory underneath it does not, so all thirty were recovered.

**`bin` shapes actually present:**

| Count | Shape | Example |
|---|---|---|
| 17 | string | `"bin": "fzf.exe"` |
| 8 | list of strings | `"bin": ["age.exe", "age-keygen.exe"]` |
| 4 | absent | `nodejs`, `rustup`, `antigravity`, `scoop` |
| 1 | mixed list | `python`: `[["python.exe","python3"], "Lib\\idlelib\\idle.bat", [...]]` |
| 1 | under `architecture.<arch>` | `kanata`, as above |

**Where the package name is not a process name:** `7zip`→`7z`,
`neovim`→`nvim`, `ripgrep`→`rg`, `kanata`→the long executable names. Paths use
backslashes (`bin\\nvim.exe`), so basename extraction is required.

**Where no manifest field names an executable at all:**

| App | Mechanism | Where its processes live |
|---|---|---|
| `nodejs` | `env_add_path: ["bin", "."]` | `apps/nodejs/current/node.exe` |
| `rustup` | `env_add_path: ".cargo\\bin"` | **`persist/rustup/.cargo/bin/cargo.exe`** |
| `antigravity` | no bin, but `shortcuts` names `antigravity` | `apps/antigravity/current/` |

**Installed architecture:** 20 `arm64`, 10 `64bit` on an ARM64 machine. Of the
ten, `python` is deliberate (`arch = "64bit"` in `pkg.toml`) and `dark` /
`innounp` are helpers, leaving roughly seven emulated without an obvious reason.
The design document's "17 emulated packages" describes an earlier state of the
machine; the current number is smaller.

These measurements changed two decisions that had been made from memory. They
are the reason `shortcuts` is collected and the reason path matching is in 2a
rather than deferred.

## The six changes

### 1. Package names compare case-insensitively

Verified against the merged code: `pkg.toml` saying `FZF` with `fzf` installed
and owned yields

```
[ Install { name: "FZF" }, Prune { name: "fzf" } ]
```

and prune runs last, so `apply` would remove the app it had just installed.

`src/plan.rs` alone compares names six times — against the installed list, the
lock, the running set, the declared set, the helper list, and `State::owns` —
and the key types those comparisons run against are declared in three other
files. Fixing six call sites leaves nothing to stop the seventh, and Phase 3
(`update`, `adopt`) and Phase 4 (winget) each add writers. So the fix is a type,
not six edits:

```rust
/// A package name. Scoop and winget both resolve names case-insensitively;
/// comparing them any other way is how `apply` uninstalls what it just
/// installed. Keeps what the user wrote for display.
pub struct Name { display: String, key: String }   // key = ASCII-lowercased
```

`Eq`, `Ord` and `Hash` run on `key`; `Display` yields `display`; serde converts
via `String` in both directions so it works as a TOML section key, a JSON object
key, and a plain value.

**`Borrow<str>` is deliberately not implemented.** It would make
`map.get("FZF")` compile and silently miss. Callers must construct a `Name`.

ASCII lowercasing rather than Unicode: scoop app names come from filenames in a
git repository and are ASCII in practice, and `to_lowercase()` carries the
Turkish dotless-i hazard, which is not a trade worth making in a function that
decides whether to uninstall something.

`Name` replaces `String` in `Config.scoop.packages`, `Config.scoop.opts`,
`Config.winget.packages`, `Lock.scoop`, `Lock.winget`, `State`'s inner map,
`Installed.name`, and every `Action` variant.

`SCOOP_HELPERS` stays a `&[&str]` of lowercase names and is compared against
`Name::key()`, so a bucket shipping `7Zip` cannot smuggle a helper past the
filter and be reported as a stray.

This also fixes a second instance nobody had noticed: `[scoop.opts]` declaring
`Python` while `packages` says `python` misses today.

### 2. Running detection: bins, shortcuts, and executable paths

Two independent signals, unioned, because each covers the other's blind spot.

**Name-based.** `scan()` collects every string appearing under any `bin` or
`shortcuts` key, at any depth, across every architecture branch — then takes the
basename, strips a trailing `.exe`/`.cmd`/`.bat`/`.ps1`/`.com`, and lowercases.
Strings beginning with `-` are dropped as argument flags.

Walking for the key rather than modelling the schema is deliberate. This code is
written on a machine with no scoop install; a depth-first collect cannot be
broken by a shape nobody anticipated, and it produced the correct answer for all
five shapes above, including `python`'s mixed list and `kanata`'s
architecture-scoped pairs. Over-collection is the safe direction: a spurious
entry can only ever cause a skip.

Collecting `shortcuts` is not padding. For `antigravity` it is the only field in
the manifest that names an executable at all.

**Path-based.** A package is running if any live process's executable sits under
`<scoop>/apps/<name>/` **or** `<scoop>/persist/<name>/`. The `persist` half is
not decoration: `rustup` puts `cargo.exe` under `persist/rustup/.cargo/bin/`,
outside `apps` entirely.

This was going to be deferred on the grounds that it only reduces false
positives. The measurement says otherwise — for `nodejs` and `rustup` it is the
*only* available signal, because neither manifest names an executable anywhere.

**Why both.** `sysinfo` cannot read `exe()` for a process at a higher integrity
level, so an elevated kanata is invisible to the path check — and is caught by
the name check, since `bin` and `shortcuts` name all four of its executables.
Conversely `node.exe` runs as the user with a readable path and is named
nowhere. Their blind spots do not overlap. Together they cover all thirty apps
on a14; either alone leaves a hole.

```rust
// model.rs — pure
pub struct Running { names: BTreeSet<String>, dirs: BTreeSet<Name> }
impl Running {
    pub fn covers(&self, inst: &Installed) -> bool;   // dirs ∪ name ∪ bins
}
```

`sys.rs` grows `running_processes() -> Vec<Process { name, exe: Option<PathBuf> }>`.
Resolving a path to an app name needs scoop's on-disk layout, so it is
`Scoop::running_apps(&self, &[Process]) -> BTreeSet<Name>` on the backend, and
`Scoop::running_set(&self, &[Process]) -> Running` unions the two signals. The
planner stays pure and still takes the running set as an input, exactly as the
approved design requires.

The union lives in the library rather than in `main.rs` for a reason found by
the final review: assembled inline in `main.rs` it was the one part of the
mechanism no test could reach, and four separate mutations that each destroyed
half the detection left the whole suite green. The design's central claim is
that the two signals cover each other's blind spots — a claim that has to be
testable somewhere.

Prefix comparison is case-insensitive, matching the filesystem.

### 3. Prune consults the running set

Not in `docs/phase2-notes.md`, and worse than the finding that is. The prune
loop in `src/plan.rs` never reads `running` at all — not a mismatched
comparison, an absent one. Verified with an exact name match, which had no
excuse to miss:

```
kanata running + owned + removed from pkg.toml  ->  Prune { name: "kanata" }
```

No skip, no `!`. Fixing change 2 does not touch this; they are different loops.
A running package that is owned and undeclared becomes `Skip { Running }`.

### 4. Architecture is a closed vocabulary, and drift is reported

`PkgOpts.arch` is `Option<String>` and accepts anything, so `arch = "arm"`
parses cleanly and means "permanently drifted". It becomes an enum:

```rust
pub enum Arch { X64, X86, Arm64, Keep }   // "64bit" | "32bit" | "arm64" | "keep"
```

A typo is now a parse error naming the bad value, and `keep` stops being a magic
string.

The planner gains a report-only action:

```rust
Action::ArchDrift { backend, name, have: String, want: String }
```

Rules, stated so they cannot be read two ways:

- Only for declared scoop packages that are installed.
- No declared `arch`, or `arch = "keep"` → never reported.
- Installed architecture unknown (no `install.json`, which older scoop versions
  did not write) → **never reported**. Phase 1 already establishes that treating
  unknown as wrong would make dotpkg want to reinstall such apps on every run.
- Otherwise, declared ≠ installed → reported.
- Emitted **independently of the version verdict**. A package can be both an
  `Upgrade` and an `ArchDrift`; those are two true facts and suppressing one
  would require a rule the reader has to remember.

`ArchDrift` counts as a report, not a change: it joins `Unmanaged` after the
prunes and is excluded from `change_count()`.

Whether `apply` should *act* on drift is deliberately left to 2b, to be decided
against the measured number rather than from memory. With a14's current
`pkg.toml` declaring `arch` for only two packages, `status` will report at most
two lines — the machine-wide picture in the table above is a dogfood
measurement, not a feature.

### 5. `scan()` stops swallowing two more error classes

Both from `docs/phase2-notes.md`, both confirmed:

- The narrowed error handling on the manifest *read* branch has no test.
  Reverting it to swallow every I/O error leaves the whole suite green. A
  portable fixture: create `manifest.json` as a *directory*, which yields a
  non-`NotFound` error on every platform.
- `entries.flatten()` four lines away still discards `read_dir` iteration
  errors. It becomes an explicit match that records a warning.

### 6. Rendering

`ArchDrift` gets the `~` marker, distinct from the five in use:

```
  ~ scoop  python         64bit, declared arm64    (architecture drift -- reported, not fixed)
```

The summary line gains a drift count only when it is non-zero, so a machine with
none sees no change:

```
  2 change(s), 1 skipped, 7 architecture drift
```

## What 2a deliberately leaves out

Each belongs to 2b, and each is listed so its absence is a decision rather than
an oversight:

- The executor: `install`, `uninstall`, and the `git show` restore path.
- Turning `SkipReason::NotLocked` into a hard failure.
- The `state.json` write path.
- The mass-prune guard. An empty `pkg.toml` still parses to zero packages and
  still plans a prune of everything owned — verified, five owned packages give
  five prunes and no warning. That is correct behaviour for `status`, which
  *should* report the truth about a config that declares nothing. The guard
  belongs at the point of execution, and it must not be bypassed by `--yes`,
  since the empty-config case is file corruption rather than an editing mistake.
- Post-uninstall verification, per-package failure accumulation, the
  confirmation prompt, and cloning a missing bucket.
- Path matching for winget, which has no equivalent layout.
- Moving `SCOOP_HELPERS` from the planner to the backend, per the design's
  `helpers()` method. The planner is pure and cannot call a backend, so this
  means passing helpers in as data, like `running`. It costs nothing today and
  belongs with the winget backend in Phase 4.
- Splitting `Lock.scoop` and `Lock.winget` into distinct pin types. Phase 2a
  writes no lock; Phase 3 does, and that is when it starts to matter.

## Testing

Layers 1 and 2 from the approved design, unchanged: everything below runs on
Linux and macOS.

New fixtures: real manifests copied from a14 into
`tests/fixtures/scoop-manifests/`, covering every observed shape — `fzf`
(string), `age` (list of strings), `python` (mixed list), `kanata`
(architecture-scoped pairs plus `shortcuts`), `neovim` (backslash paths, name
unequal to bin), `nodejs` (no executable named anywhere). Testing the extractor
against manifests that exist beats testing it against manifests imagined on a
Mac.

**Every new test requires a negative control with recorded evidence.** Phase 1
shipped three tests that passed for reasons unrelated to their names — one
depended on struct field declaration order, one called itself "every action
kind" while missing a kind, and one was a purity guard that stayed green after
`fs::read_to_string` was added to the file it guarded. The rule is not "consider
a negative control": break the code, paste the red output into the task, restore.

The controls to use, named so they are not invented at implementation time:

| Test | Break to confirm it goes red |
|---|---|
| mixed-case end-to-end | make `Name::new` skip lowercasing |
| `bin` extraction | stop descending into `architecture` |
| `shortcuts` extraction | collect `bin` only |
| path matching | drop the `persist` prefix |
| prune skips a running package | remove the `running` check from the prune loop |
| arch drift reported | make the comparison always equal |
| unknown arch is not drift | treat `None` as a mismatch |
| scan read-branch warning | revert the branch to swallowing every error |
| `read_dir` iteration warning | restore `.flatten()` |

## Dogfood

Read-only, on a14, and framed so it can fail:

1. `! kanata running` appears when kanata is running — the first time the guard
   has ever fired for it. If kanata is not running at the time, start it, or
   record that the check was not exercised rather than claiming it passed.
2. `neovim` resolves to `nvim`, `ripgrep` to `rg`, `7zip` to `7z`.
3. `nodejs` and `rustup` are caught by path matching alone while `node` or
   `cargo` is running. This is the case that justified including it.
4. All thirty apps still scan, with no regression against the Phase 1 run.
5. The machine-wide architecture picture is recorded as a measurement, to be
   the input to 2b's decision on whether `apply` acts on drift.

Every run is invoked at medium integrity via the scheduled-task technique in
`docs/dogfood-2026-08-08.md`, not over plain `ssh`, since the elevated path
already produced one false finding on this machine. Output goes to a file in
`$env:TEMP` and is copied back; CLIXML wrapping truncates anything returned
directly.

## Non-goals

Unchanged from the approved design. Additionally, 2a does not attempt to detect
a running package by opening its files to test for a lock, and does not shell
out to scoop for anything — subprocesses remain reserved for mutation, which
means 2a spawns none at all.
