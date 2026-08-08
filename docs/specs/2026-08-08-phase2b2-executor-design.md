# dotpkg Phase 2b-2 — the `apply` executor

**Status:** design approved 2026-08-08, not yet implemented.
**Depends on:** Phase 2b-1 (`apply --prepare`), merged at `b1239dd`.
**Carries:** `docs/phase2b-notes.md`, "Carried into Phase 2b-2", which this
phase closes.

Everything `apply` does from the first destructive act onward: `scoop
uninstall` and `scoop install`, verification of the resulting on-disk state
after every mutation, the `state.json` write path, the confirmation prompt and
`--yes`, cloning a missing bucket, and the decision about `--fix-arch`.

This is the phase in which the tool starts removing other people's software.

## The measurement that rewrites this phase

Measured on a14, scoop **0.5.3** (`b588a06e`), 2026-08-08, in a throwaway
`$env:SCOOP` root under `%TEMP%`. Exit codes were read from
`System.Diagnostics.Process.ExitCode`, not from `Start-Process -PassThru`,
which returns an unpopulated value. Full raw record, every command and its
verbatim stdout, machine state before and after, and the two results that are
contaminated or falsified rather than smoothed over:
[`docs/measurements-2026-08-08-scoop-exit-codes.md`](../measurements-2026-08-08-scoop-exit-codes.md).

**`scoop` does not report operational failure through its exit code. At all.**
Eleven invocations were tried in the dedicated exit-code round — a wrong
hash, a dead URL, installing over a manifest path that does not exist,
uninstalling an app that was never installed, re-adding an existing bucket —
and every one exits **0**. The only non-zero result anywhere in this
measurement effort is `scoop thisisnotacommand`, exit **1**.

This is not the `.cmd` shim swallowing a code. `shims/scoop.cmd` is
`pwsh -noprofile -ex unrestricted -file …\scoop.ps1 %*`; invoking `scoop.ps1`
directly through `powershell -File` gives 0, and through `powershell -Command`
followed by `Write-Host $LASTEXITCODE` prints `LASTEXITCODE=0`. Scoop reserves
exit 1 for "I do not know that command", not for "what you asked me to do
failed".

Three consequences, and they are the reason this document exists in this shape:

1. **`Scoop::download`'s `anyhow::ensure!(out.status.success(), …)`
   (`src/backend/scoop.rs:517`) can never fire.** Every `Outcome::ReadyToFetch`
   is produced unconditionally. `download_failure_detail`
   (`src/backend/scoop.rs:542`) and its four unit tests cover a function
   production can never reach.
2. **Phase 2b-1's central promise is not implemented.** Its design says a dead
   URL, a hash mismatch or a network failure "become *nothing happened, here is
   why*". Measured, all three print `ready`.
3. **Verifying on-disk state after every mutation is not a second safety net.
   It is the only signal that exists.** The approved design's correction block
   already required it; this measurement removes the alternative.

The 2b-1 dogfood's answer to "Do the downloads verify? — Yes" rested on
"`Outcome::Ready` is only produced after `scoop download` exits 0, and `scoop
download` hash-verifies by construction". The first clause is vacuous. Its
independent cross-check against the cache still stands, and is now the only
part of that answer that carries weight.

### Everything else measured the same day

Summary only — the full stdout for every item below is in the linked
measurement document.

Read-only against the real `~/scoop` (31 apps, 75 cache entries, unchanged
before and after): `git` on a14 is scoop-managed (`where.exe git` resolves
first to `scoop\apps\git\current\cmd\git.exe`, confirming the self-reference
in `docs/specs/2026-08-08-phase2b1-prepare-design.md` is real, not
hypothetical); **no package declares `depends`** — zero of 30 installed
manifests, zero of the 25 declared packages' bucket-HEAD manifests, recorded
as a falsified concern rather than a smoothed-over one; an installed
`manifest.json` is byte-identical to its bucket blob for every one of the six
apps where a same-version comparison was possible.

In the throwaway root: `-u` and `-a` are accepted on a manifest path
(`install.json` records `{"architecture": "arm64", "url": "<the staging
path>"}`); the installed `manifest.json` is byte-identical to the staged
file; `scoop download` without `-a` fetches the default architecture's
artifact, and the probe cache ended with two distinct files for one version —
so a prefetch that omits `-a` warms the wrong artifact and the install then
reaches the network **inside the mutation window**; `scoop uninstall` is
clean, no residue, `persist/` never created; **a failed install leaves
`apps/<app>/<version>/` containing only the downloaded archive**, no
`current` junction, no `manifest.json`, so `Scoop::scan()` skips it silently
(`src/backend/scoop.rs:255`, `continue` on `NotFound`) — invisible to every
command, but never masquerading as installed; `scoop bucket add` clones in
full, not shallow (`is-shallow = false`, all 16 commits) — old pins survive a
clone; warm-cache timing for `fzf` is spawn-dominated, not
extraction-dominated (full uninstall+install window **11.63 s**); stderr is
non-empty on success, carrying ANSI colour codes and non-fatal
`Cannot find path …` noise — no logic may read "stderr said something" as
"something went wrong".

**Corrected, not merely contaminated: `scoop install` over an installed
app.** `docs/phase2b-notes.md` recorded it as "exit 0, **no output**, nothing
changes". Measured, it prints `WARN 'fzf' (0.74.1) is already installed. /
Use 'scoop update fzf' to install a new version.` on **stdout** — and
installing a *different* version's manifest prints that same line, naming
the version **already installed**, not the one requested. The substance of
the note holds; the detail does not, and the detail is the part an executor
would key on. `docs/phase2b-notes.md` now carries the corrected text
directly.

**One measurement is contaminated and yields nothing.** Shim creation: the
probe root has no `apps/scoop`, so scoop could not copy `shim.exe` and created
only `fzf.shim`. Nothing about shim behaviour on a real root may be inferred
from this run.

## Corrections to the approved design

Recorded here rather than edited in place, matching the precedent set by
`docs/specs/2026-08-08-phase2a-design.md`.

**`design.md:205-208` still asserts what `design.md:323-336` retracts.** The
correction block sits 115 lines below the paragraph it corrects, and the
paragraph — "without pretending a downgrade is safe … `↓` makes that visible" —
is the one a prompt author reads. Every version change is uninstall + install;
`^` carries the same risk as `↓`. The prompt specified below must not imply
otherwise.

**`design.md:239-247`'s `Backend` trait sketch is unimplementable.**
`install(&self, pkg: &str, pin: &Pin)` cannot be written: only a staged
manifest path installs anything, and `pkg: &str` reintroduces the untyped name
that `src/model.rs:19-21` exists to make unrepresentable. `helpers()` shipped
in the planner (`src/plan.rs:13`), not the backend. The real seam is the
`Mutator` trait below.

**`design.md:309`'s "offer to clone (URL is in `pkg.toml`)" describes code that
does not exist.** `ScoopSection.buckets` (`src/config.rs:15`) is read by
nothing outside a length assertion in its own test module, and the documented
`name=url` form is never split.

**`design.md:289-292`'s restore recipe stages to `%TEMP%` and uses `scoop
install` to change a version.** Both are wrong: `install.json` records the
staging path, so staging is permanent (`src/apply.rs:46-56`), and `scoop
install` over an installed app changes nothing.

**`design.md:307` and `design.md:315-317` contradict each other** — "hard fail,
print the recovery command" against "per-package failures accumulate; the run
continues". This document picks, in "Halt or proceed" below.

**`src/plan.rs:269-270` states a falsehood about its own function.**
`is_older` splits on every non-digit and keeps each numeric run, so
`1.0.0-rc1` reduces to `[1,0,0,1]`, not `[1,0,0]`. The two do not compare
equal, and the displayed arrow is inverted: installed `1.0.0-rc1` against a
lock of `1.0.0` renders as a downgrade.

**`docs/phase2b-notes.md:249-255` defers `commit` validation to Phase 3 on a
reasoning that protects the wrong thing.** Verified against a real git
repository: `git cat-file -e` does reject `-oops` and `--upload-pack=touch`,
exactly as the note says — and accepts `main`, `HEAD`, `@`, and
`refs/heads/main`. The reachable hole is an ordinary branch name, not a dash.
See "Rev-locking" below.

## The architecture

### One driver, one seam

`src/main.rs` carries two inline copies of load → scan → plan (`:64`, `:100`),
and `config::load(` appears at exactly those two sites and nowhere in
`tests/` — the whole assembly has no in-process coverage. Phase 2b-2 makes one
of those arms destructive, so the assembly is extracted rather than duplicated
a third time.

The only thing faked in tests is the subprocess:

```rust
/// Every scoop invocation that changes something, plus the one that fetches.
/// `download` is behind this trait too, which is what finally makes a real
/// `Outcome::ReadyToFetch` reachable from a test on macOS.
pub trait Mutator {
    fn uninstall(&self, app: &Name) -> Result<CommandReport>;
    fn install(&self, manifest: &Path, arch: Option<&str>) -> Result<CommandReport>;
    fn download(&self, manifest: &Path, arch: Option<&str>) -> Result<CommandReport>;
}

/// What one scoop invocation said. The exit code is recorded and never
/// believed; see "The measurement that rewrites this phase".
pub struct CommandReport { pub code: Option<i32>, pub stdout: String, pub stderr: String }
```

`Scoop` implements it for real. Observation of the resulting state goes through
the existing `Scoop::scan()` and through `verdict` below, against a real
directory tree — never through the fake.

### `verdict` — the only evidence

```rust
pub enum Expected {
    Absent,
    Present { staged: PathBuf },
}

/// Pure over the filesystem. No subprocess, no network.
pub fn verdict(root: &Path, app: &Name, want: &Expected) -> Result<(), Disagreement>;
```

- **`Present { staged }`** — read `<root>/apps/<app.key()>/current/manifest.json`
  and compare its **bytes** to the staged manifest. Byte comparison, not
  version comparison, because a version check cannot see a same-version content
  swap, which is exactly what the rev-locking hole below produces. Measured:
  the installed file is byte-identical to the staged one.
- **`Absent`** — `<root>/apps/<app.key()>` must not exist. If it does, the
  disagreement names the leftover path; `apps/<app>/<version>/<archive>.zip`
  with no `current` is the measured shape of a failed install.

A byte mismatch that disappears under `\r\n` → `\n` normalisation is reported
as its own kind. On a14 the bytes match exactly, so this is not a normalisation
policy — it is a distinct diagnosis, so that a machine whose scoop rewrites
line endings produces an accurate message instead of a false failure.

### Prefetch, re-promised

`scoop download` cannot fail through its exit code, so the prefetch verdict is
read from stdout. The markers were measured, and one of them is a trap:
**`'<app>' (<version>) was downloaded successfully!` is printed even when the
hash check failed**, so it is not a success marker.

A download succeeded when **all three** hold:

- at least one line matching `Checking hash of … ok.`
- no `ERROR Hash check failed!`
- no `ERROR URL … is not valid`

Fail-closed: absence of the success marker is failure. A manifest that declares
no `url`/`hash` prints none of these and is therefore refused; that is a known
and accepted limitation, recorded rather than papered over.

`ensure!(out.status.success(), …)` is deleted, with a comment naming this
measurement, and `download_failure_detail` is rewritten around stdout. stderr
is not consulted for a verdict: it is non-empty on success and carries ANSI
escapes.

### Architecture is resolved at plan time, and `-a` is mandatory

Because `scoop download` without `-a` fetches the default architecture's
artifact, an install that then wants a different one reaches the network inside
the mutation window. So:

- `Action::{Install, Upgrade, Downgrade}` gain the resolved architecture,
  which means it appears in the plan the user confirms.
- Resolution is `declared.scoop.opts[name].arch.as_scoop()`, falling back to
  the currently-installed `Installed.arch`. `Arch::Keep` suppresses `-a`
  entirely.
- `-a` is passed to **both** `download` and `install`.

This is arch *preservation*, and it is mandatory. `--fix-arch` as a sweep is
**out**: `ArchDrift` is `Intent::Report` (`src/apply.rs:95`) so no manifest is
staged for it, it carries no version to reinstall, and the drift branch
(`src/plan.rs:136-162`) consults no running check. Acting on drift needs a new
`Action::Reinstall { version }`, not a reclassified `ArchDrift`, and that is a
later phase's decision. Roughly seven emulated packages on a14 stay emulated
by default, and preservation is what stops `apply` from silently changing that.

### Ordering, and what is allowed inside the window

Measured: an `fzf` uninstall+install from a warm cache is 11.63 s, dominated by
two process spawns rather than by extraction. The window's length is not the
dial. Its contents are.

Order within a run:

1. Pure `Install` — no window at all.
2. Version changes, one package at a time, never batched, so no package's
   window spans another's.
3. Prunes.
4. `git` and any declared `SCOOP_HELPERS`, last within their group. `git` is
   line 1 of the real 23-change exercise plan from the 2b-1 dogfood, and it is
   the binary `stage()` needs.

Inside the window, exactly two spawns and nothing else. `-u` keeps scoop from
self-updating and from `git pull`-ing a bucket mid-window. Verification is file
reads. Staging never re-runs — it shells out to `git` three times, and `git`
may itself be mid-window.

**Retry exactly once, and only when `apps/<app>` is entirely absent.** A retry
over a half-install receives `WARN … is already installed`, exit 0, and no
change — manufacturing the silent success that verification exists to catch.

**The recovery artifact is written before the first mutation, not printed after
a failure.** `%LOCALAPPDATA%\dotpkg\recover.cmd`, one
`scoop install -a <arch> "<staged manifest>"` line per package in the change
set. A run that dies leaves a file that puts the machine back; a run that
merely prints advice leaves nothing if the terminal is gone.

**`running` is re-checked immediately before each mutation.** Today it is
sampled once (`src/main.rs:97`), before roughly two dozen downloads. A user who
opens their editor during the prefetch would otherwise have it uninstalled —
the row `design.md:306` exists to prevent, reopened by the phase split.

### The `state.json` write path

Ownership is written per package, and the ordering differs by action because
the two failure directions are not symmetric.

- **Install:** install → verify → `set` + save. A crash between install and
  save leaves a package installed and unowned, which reads as `Unmanaged` and
  is never touched. Inert.
- **Upgrade / downgrade of a package dotpkg already owns: no writes at all.**
  Ownership is intent; the uninstall half is an implementation detail. A crash
  mid-window leaves the package absent and still declared, so the next run's
  `plan()` sees `current == None` and re-emits `Install`. Self-healing.
- **Prune:** uninstall → verify → `remove` + save.

That last one is deliberately the reverse of what a "claim late, release early"
reading suggests, and the reason is worth stating. Releasing first means a
crash leaves a package that is **still installed and no longer owned** — it
becomes `Unmanaged`, and recovering it needs `dotpkg adopt`, a command that
does not exist. Releasing last means a crash leaves a **ghost** entry, and a
ghost is inert: `state.owns` is consulted only inside `for inst in installed`
(`src/plan.rs:222-229`), so an entry with no corresponding installed package is
never read. Its only effect is to inflate `owned_count`, which only the
mass-prune guard reads, in the direction that makes the guard fire more
readily.

Ghosts are then cleaned up rather than tolerated: at the end of a run, state
entries with no corresponding package in the closing scan are dropped. A ghost
dies in the run that created it.

Supporting changes:

- `State::save` becomes write-temp-in-the-same-directory + `sync_all` +
  `rename`, keeping the displaced file as `state.json.bak`. Today it is
  `fs::write`, and a torn write makes even `status` fail
  (`src/state.rs:63-70`).
- `State::remove(&mut self, backend, name) -> bool` is added; there is no way
  to release an entry today.
- The existing `Ownership` variant is read and written back verbatim. It is
  currently never read anywhere, so a careless `set(…, Installed)` in the
  upgrade path would erase every `adopt` decision with nothing going red.
- A command that **writes** state refuses a relative resolution of
  `State::default_path()` outright, and prints the absolute path before the
  first mutation. `--state <path>` is added so the scheduled-task dogfood can
  pin it.

### The prompt, `--yes`, and stdin

Order of a full `apply`:

```
load -> guards -> scan -> plan -> print plan -> [clone missing buckets]
     -> stage + download everything -> print preparation
     -> ONE question -> mutate -> re-scan -> summary
```

The question goes to **stderr**, flushed, so `apply | tee` still shows it while
the plan and preparation tables stay on stdout. `Ok(0)` from `read_line` —
which is what a child process with no console returns, and what the
medium-integrity scheduled task the dogfood uses will produce — and `Err` both
mean **No**, with their own message and their own exit code. `is_terminal()`
never decides consent.

The question states the uninstall+install count, not a "changes" count.
`Plan::change_count()` lumps an additive `Install` together with three
destructive variants, so `4 change(s). Continue? [y/N]` reads identically for
four fresh installs and four uninstall+installs of running toolchains.

`--yes` answers that question and nothing else. It does not bypass
`mass_prune_guard`, `Skip{Running}`, the pre-mutation running re-check,
post-mutation verification, the removals gate, or architecture preservation.
`--yes` on a run containing prunes additionally requires `--allow-prune`. That
is the cheapest honest answer to `src/apply.rs:34`, where one surviving
declared package disarms the mass-prune guard entirely — a `pkg.toml` truncated
from 25 packages to 1 is 24 uninstalls with no protection at all.

### Halt or proceed

Two separate questions, easily collapsed into one and answered wrongly: what
happens when the **preparation** could not be completed, and what happens when
a **mutation** fails partway through a run that did start.

**When `preparation.is_ok()` — zero `Failed`, zero `NotLocked` — the run
proceeds.** From there, a mutation failure is per package: it is recorded and
reported, and it neither stops nor cancels its neighbours. Stopping halfway
through 25 packages leaves a worse machine than finishing the ones that work.
This is where `design.md:315-317` is right and `design.md:307` is wrong; the
"hard fail" that row calls for is the *exit code and the recovery text*, not an
abort of the remaining packages.

**When the preparation is not ok, the default is to refuse the whole run** —
exit 2, nothing changed — and `--yes` does not narrow that. `--keep-going`
does, and only in one direction: it executes the installs that are ready and
holds everything else.

**Removals execute only when `preparation.is_ok()`, and no flag bypasses
that** — not `--yes`, not `--keep-going`. Under `--keep-going` every prune is
printed as `held` and deferred to a later run. The unbypassable fence goes
where software is *deleted*, so that the flag people end up pasting into a
shell alias cannot do damage.

The tempting narrower gate, `failed_count() == 0`, is *less* safe than what
already ships: a plan of `Install{zellij}` with no lock entry plus a ready
`Prune{aichat}` has `failed_count() == 0`, so aichat would be deleted while
zellij never arrives. Since `update` is Phase 3, every newly typed package name
is `NotLocked` by construction, which makes that the only swap shape reachable
today.

Exit codes become three:

- **0** — every planned action verified on disk.
- **1** — something changed and something failed. Mixed state; go look.
- **2** — refused, and nothing changed. A guard fired, the user said no, or no
  answer was available.

The closing summary reports only what the re-scan confirms. It never says
"N upgraded". An empty or implausibly shrunk re-scan is reported as an error
about the scan, not as a result about the machine — `src/backend/scoop.rs:217-222`
treats a missing `apps/` as a legal empty scan with no warning.

## Rev-locking: `commit` must be a hash

Verified against a real git repository, with a bucket whose tip is a
same-version URL/hash correction:

```
cat-file -e main^{commit}              ACCEPTED
cat-file -e HEAD^{commit}              ACCEPTED
cat-file -e @^{commit}                 ACCEPTED
cat-file -e refs/heads/main^{commit}   ACCEPTED
cat-file -e -oops^{commit}             rejected
cat-file -e --upload-pack=touch^{commit}  rejected

git show main:bucket/tool.json    -> {"version":"1.0.0","url":"…/evil.zip", "hash":"bbbb"}
git show <pinned sha>:…           -> {"version":"1.0.0","url":"…/v1.zip",   "hash":"aaaa"}
```

Both carry `version: "1.0.0"`, so `stage_text`'s version equality check
(`src/backend/scoop.rs:449-452`) passes and the tip is staged. A lock that
looks pinned silently installs whatever the bucket says now — "quietly falls
back to latest", arriving through the field that exists to prevent it, and
reaching the machine through an uninstall.

`commit` must be 40 or 64 lowercase hex characters. The rule is enforced in
**two** places, deliberately:

- `lock_coherence_guard`, a pure function beside `mass_prune_guard`, refuses
  the whole run before the plan is built, with a message that names the field
  and points at `dotpkg update`.
- `Scoop::stage` re-checks, because it is a public API and Phase 3 will call it
  from somewhere else.

`lock_coherence_guard` also refuses, with no I/O: a `Pin::WingetVersion` in the
scoop map, any `bucket`, `version` or package name failing
`ensure_plain_component` (which also validates a `Name` that came from
`pkg.toml` — `config::parse` currently accepts `"../evil"`), and a lock naming
a bucket that `pkg.toml` does not declare.

## Carried debts closed in this phase

Each was found by the pre-design audit and each is a live defect, not a
speculative one.

1. **`commit` accepts any git revision** — above. A bug in shipped 2b-1 code.
2. **`tests/cli.rs`'s `Snapshot` records path names, not content**
   (`tests/cli.rs:87-105`). Measured: injecting the exact `state.json` write
   this phase adds left `cargo test --test cli` at 3/3 green while the file's
   content was replaced. It hashes file bytes, using `DefaultHasher`, before
   any 2b-2 test is written — otherwise every "the fence held" assertion is
   empty.
3. **The helper skip shadows both the prune and the unmanaged branch**
   (`src/plan.rs:226` above `:229` and `:247`). A declared `7zip` is installed
   and owned; undeclare it and `plan()` emits no line at all. dotpkg acquires
   software it can never release and never mentions again.
4. **`Ownership` is never read, and `is_older`'s doc comment is false.** The
   first becomes load-bearing the moment state has a write path; the second is
   a wrong comment plus an inverted display arrow.

## Cloning a missing bucket

In, behind `--clone-missing-buckets`, default off, honoured by `apply` and
`apply --prepare` alike, and run as step 0 — before staging and before the
prompt, because cloning does not change the plan, only whether staging can
succeed.

`src/config.rs` first parses `buckets` into
`Vec<BucketDecl { name: Name, url: Option<String> }>`, folded for collisions
like `packages`, with `ensure_plain_component` on the name and a scheme check
on the URL. Only buckets declared in `pkg.toml` are cloned; a lock naming an
undeclared bucket is a per-package failure that says so, never a guessed URL.

A clone is verified by attempting `stage()`, not by a bespoke check — `scoop
bucket add` exits 0 on a duplicate. Measured, it clones in full, so old pins
remain recoverable.

## Testing

Layers 1 and 2 from the approved design, unchanged: everything below runs on
macOS and Linux.

**Standing rule for this phase:** every stdout assertion about what happened is
paired, in the same test, with a disk assertion. A stdout assertion alone may
only prove wording, and its comment must say so.

**The fake must be able to lie the way scoop was measured to lie.** Two
independent booleans, `uninstall_really_removes` and `install_really_installs`,
defaulting to "exit 0, changed nothing", with a comment citing the table at the
top of this document. A fake that mutates its own map in `install()` and serves
it back from observation makes every verification test pass by construction and
cannot distinguish an executor that calls `verdict` from one that does not.

**Forbidden by name:** no test may create a file at `Scoop::scoop_exe()`'s
path. `Command::new` execs `<root>/shims/scoop.cmd` by path, and `execve` on
macOS ignores the `.cmd`, so a `#!/bin/sh` script there buys a green
"end-to-end" test that restates this document by construction and means
something different on a Windows runner. Enforced by a source scan over
`tests/`, in the allowlist idiom `tests/planner.rs` already uses.

**Argv tests** are whole-vector equality for `uninstall_argv` and
`install_argv`, matching the `download_argv` precedent, plus a structural guard
that no inline `.args([` literal exists in `src/backend/scoop.rs`.

### Mandatory tests, each with a negative control

Every control below must be run, and the assertion that fires must be recorded.
Two controls in the previous phase "ran" and proved nothing — one because an
unrelated error message happened to contain the asserted substring, one because
the mutation had nothing to bite. The design below is aimed at that specifically.

| # | Test | Negative control | What must fire |
|---|---|---|---|
| 1 | An install that scoop silently did not perform is reported as failed | `verdict` always returns `Ok` | the two silent-no-op tests go red; the positive control stays green |
| 2 | The same, worded-error trap | `verdict` always returns `Err` **whose string contains every substring the negative tests assert** | caught only by an anti-substring assertion naming the *neighbouring* branch, plus a positive-control sibling that must stay green |
| 3 | Post-install verification is reached | delete only the post-install `verdict` call | install tests red, uninstall tests green |
| 4 | Post-uninstall verification is reached | delete only the post-uninstall `verdict` call | uninstall tests red, install tests green |
| 5 | A prune refuses a package absent from `state.json` | remove the `state.owns` check | the approved design's first mandatory test |
| 6 | A lock at a nonexistent commit fails and installs nothing | remove the `cat-file -e` guard | the approved design's second mandatory test |
| 7 | A lock naming a branch instead of a sha is refused | remove the hex check from `lock_coherence_guard` **and** from `stage` | both sites are independently covered |
| 8 | A hash failure in `scoop download` is reported despite exit 0 | make the stdout parser return success unconditionally | the prefetch promise |

**Reachability sentinel.** `cannot run <root>/shims/scoop.cmd` is produced by
production code (`src/backend/scoop.rs:516`), and nothing that stops before the
mutation can print `scoop.cmd`. Every refusal test asserts its **absence** and
has a sibling asserting its **presence**, the discipline `tests/cli.rs` already
invented for its `ghost` sentinel. Two of the three existing `cli.rs` tests
call `assert_nothing_was_touched` at a point where it cannot fail; that pattern
is not inherited.

**Also on macOS, and cheap:** the three partial-install shapes as fixtures
(`apps/b/` empty; `apps/c/1.0.0/c.zip` with no `current`; `install.json`
present with `manifest.json` deleted), all of which currently produce zero
lines and zero warnings; `is_older("1.0.0-rc1", "1.0.0") == false` pinned as a
fact; the ordering function; and an extension of the planner's ordering fixture
to contain a version change, which it currently does not.

**Provable only on a14:** that scoop honours the argv at all, that the cache
entry survives from prepare to install, what a killed install leaves behind,
whether shims are created and when, and the medium-integrity scheduled-task
technique.

## Dogfood

The first run in which dotpkg removes software from a real machine.

It is staged, and the first stage is not the real root. `%TEMP%`-rooted
`$env:SCOOP` probes already worked twice; the executor gets one there first,
with a deliberately divergent lock, before it is pointed at `~/scoop`.

Two facts shape the real-root stage. A lock that matches the installed versions
gives `apply` zero actions, so a deliberately divergent "exercise" lock is
required — budgeted, not rediscovered. And `state.json` on a14 owns nothing, so
**prune cannot be exercised until dotpkg's own install path has put an entry
there**. The real-root dogfood is therefore two gated acts: install or downgrade
one leaf package and confirm exactly one new state key appears, then undeclare
that package and prune it.

`kanata` is never started or stopped. Run at medium integrity via the
scheduled-task XML-clone technique from `docs/phase2b-notes.md` — raw SID as
`UserId`, `<LogonType>InteractiveToken</LogonType>`, no `<RunLevel>` element.

Framed so it can fail:

1. Does verification actually catch a scoop invocation that exits 0 and did
   nothing? Induce it, do not wait for it.
2. Does `recover.cmd` restore a package after a deliberately killed install?
3. Does exactly one `state.json` key appear for one installed package, and
   disappear for one pruned one?
4. Does the run refuse, and change nothing, when one package's prefetch fails?
5. Is the machine's app count and version table identical afterwards except for
   the packages the plan named?

A prediction worth recording before the run: **verification will fire at least
once for a reason nobody predicted**, because it is the first mechanism in this
project whose whole job is to disbelieve a tool that reports success. If it
never fires across the whole dogfood, that is a reason for suspicion, not
comfort.

## Non-goals

Unchanged from the approved design. Additionally, Phase 2b-2 does not implement
`update`, `adopt` or `add`; does not act on architecture drift; does not
reimplement hash verification; and does not add a lock file or single-instance
check — two concurrent `apply` runs remain undefined behaviour, recorded here
so its absence is a decision.
