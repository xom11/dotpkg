# Dogfood: Phase 2a `dotpkg status` against a real scoop install (a14, 2026-08-08)

Task 8 of the Phase 2a plan. Branch `phase2a-truthful-plan`, built and run at
`e8ba8bf` (HEAD after the six-commit whole-branch fix wave that followed the
first, blocked attempt at this task). `cargo test --release` on the build
machine (a macOS checkout of the same tree): **83 passed, 0 failed** (27 lib
unit + 30 `tests/planner.rs` + 26 `tests/scoop_scan.rs`), confirmed
independently before touching a14. `status` performs no write, no subprocess
and no network call, so this run is read-only by construction; that claim is
checked empirically in "Read-only, verified" below, as in Phase 1.

**This task ran in two parts.** The first attempt found a14 fully unreachable
(`ssh` timeout, 100% ICMP loss, `tailscale status` itself reporting the peer
offline across a bounded ~6-minute retry) and was reported BLOCKED rather than
worked around, per the task's standing rule. The coordinator later confirmed
a14 was back; everything below happened in that second session.

## Build

Copied `Cargo.toml`, `Cargo.lock`, `src/`, `tests/` (not `target/`, not
`.git/`) to the existing `C:\Users\kln\dotpkg-build` (left over from Phase 1)
and built natively there, over a plain (elevated) `ssh` session — the
elevation/integrity concern only applies to the *checks*, not to compiling:

```
rustc 1.97.1 (8bab26f4f 2026-07-14)
cargo 1.97.1 (c980f4866 2026-06-30)
Compiling dotpkg v0.1.0 (C:\Users\kln\dotpkg-build)
Finished `release` profile [optimized] target(s) in 5.53s
```

Binary present at `target\release\dotpkg.exe` afterward (`Test-Path` → `True`).
5.53s is much faster than Phase 1's 45.42s cold build; the likely reason is a
warm `target\` directory from Phase 1's own build still caching dependency
crates in the same directory, not anything about this task — dependencies
didn't change, only `dotpkg`'s own code recompiled.

## `pkg.toml`

Written verbatim from the brief, including the one deliberate addition:

```toml
[scoop]
buckets  = ["main", "extras", "xom11=https://github.com/xom11/scoop-bucket"]
packages = [
  "git", "nodejs", "gh", "bat", "ripgrep", "fzf", "fastfetch", "neovim",
  "tree-sitter", "lazygit", "lazydocker", "yazi", "zellij", "opencode",
  "shfmt", "yamlfmt", "stylua", "actionlint", "kanata", "beckon",
  "python", "go", "rustup", "uv", "age",
]

[scoop.opts]
python = { arch = "64bit" }
rustup = { arch = "keep" }
stylua = { arch = "arm64" }
```

## Running at medium integrity — the biggest deviation from the plan

The brief's exact technique (`New-ScheduledTaskPrincipal -UserId 'kln'
-LogonType Interactive -RunLevel Limited`, then `schtasks /run`) **did not
work in this session**, and getting it to work took real investigation. This
section records that investigation in full, because it is the most valuable
finding this task produced.

**Symptom.** Every attempt — `LogonType Interactive` demand-started via
`schtasks /run`, `LogonType S4U` (which is not supposed to need a live
session at all) demand-started the same way, and `LogonType Interactive`
registered with only a natural one-time trigger and no demand-start — landed
in the same place: `Get-ScheduledTask` reported `State: Queued` (for the
demand-start attempts) or stayed `Ready` with `LastRunTime` pinned at the
Windows "never run" sentinel (`11/30/1999 12:00:00 AM`) and `LastTaskResult:
267011` (`SCHED_S_TASK_HAS_NOT_RUN`) long after the trigger time had passed.
`Microsoft-Windows-TaskScheduler/Operational` showed event 110 ("launched...
for user kln") immediately followed by event 325 ("queued instance") every
time, with no completion event ever following.

**Hypotheses ruled out with direct evidence, not assumed:**

- *No interactive session for kln.* `query user` returned nothing — but so
  did `query session` and `quser`, and the reason turned out to be that
  **`query`/`quser` are not available on this machine at all**
  (`CommandNotFoundException`, confirmed after the empty output was
  investigated rather than accepted at face value). Direct evidence instead:
  `explorer.exe` is running as `ZENBOOK-A14\kln` in `SessionId 1`, and
  `(Get-CimInstance Win32_ComputerSystem).UserName` also reports
  `ZENBOOK-A14\kln`. kln has a real, active desktop session throughout.
- *Locked screen.* `Get-Process -Name LogonUI` found nothing — the console is
  not at a lock screen.
- *A general Task Scheduler/session-broker failure on this box.* Ruled out by
  a working counter-example found on the same machine: `KanataWatchdog`
  (`RunLevel Highest`) and **`AHKWatchdog`** — which uses the *exact* logical
  configuration this task needed, `LogonType Interactive` + `RunLevel
  Limited`, `UserId kln` — both fired and completed successfully during this
  session's own testing window (`Microsoft-Windows-TaskScheduler/Operational`
  events 100/200/201/102, "successfully completed... with return code 0" at
  15:30:01, while my own task sat queued).

**What actually differed, found by exporting the working task's XML**
(`Export-ScheduledTask -TaskName AHKWatchdog`): its `<Principal>` specifies
the user by **raw SID** (`S-1-5-21-...-1001`) with
**`<LogonType>InteractiveToken</LogonType>`**, and has **no `<RunLevel>`
element at all** (which defaults to `LeastPrivilege`, i.e. Limited) — as
opposed to every principal this task built via
`New-ScheduledTaskPrincipal -UserId 'kln' -LogonType Interactive|S4U -RunLevel
Limited`, all of which stuck at Queued. Whether the friendly `-LogonType
Interactive` parameter's mapping, the plain-username `UserId`, or the
explicit `<RunLevel>` element is the actual cause was not isolated further —
time was not spent bisecting a working fix once one was found, in favour of
getting real evidence from the actual machine while it was awake.

**The fix:** clone `AHKWatchdog`'s exported XML unchanged except for
`<Triggers>` (a new near-future `StartBoundary`) and `<Actions>` (the
`dotpkg`-check script instead of AutoHotkey), then
`Register-ScheduledTask -TaskName DotpkgPhase2a -Xml <that XML> -Force`. This
worked on the very next trigger, confirmed from the task's own output:

```
GROUP INFORMATION
-----------------
...
NT AUTHORITY\Local account and member of Administrators group  Group used for deny only
BUILTIN\Administrators                                         Group used for deny only
NT AUTHORITY\INTERACTIVE                                        Mandatory group, Enabled by default, Enabled group
CONSOLE LOGON                                                    Mandatory group, Enabled by default, Enabled group
...
Mandatory Label\Medium Mandatory Level                          Label            S-1-16-8192
```

`kln` is a local administrator (`Get-LocalGroupMember Administrators` lists
`kln`); the deny-only `Administrators`/`Local account and member of
Administrators group` entries confirm the admin rights were genuinely
filtered, not merely relabelled, and `Medium Mandatory Level` is exactly what
Step 3 requires. `NT AUTHORITY\INTERACTIVE` and `CONSOLE LOGON` being present
confirm this is a real interactive-desktop token, not a batch/service
substitute.

**A red herring also tested and rejected:** `runas /trustlevel:0x20000`
(Windows' built-in "run this at reduced trust" mechanism, which needs no
scheduled task or session negotiation at all) *did* filter `Administrators`
to deny-only, but the resulting `whoami /groups` still showed **`High
Mandatory Level`**, not Medium — group filtering without integrity relabelling
is not what Step 3 asks for, so this path was abandoned in favour of the
cloned-XML scheduled task above, which unambiguously produces both.

Every `dotpkg status` invocation below ran inside that correctly-configured
task, confirmed Medium by its own `whoami /groups` each time it ran.

## The five questions

Two medium-integrity task runs were needed: the first covers questions 2–5;
kanata turned out to already be running, unprompted, which made a live
(rather than manifest-only) answer to question 1 possible with one more run
adding a probe entry for it. Both runs' `whoami /groups` confirmed `Medium
Mandatory Level` as above.

### 1. Is kanata detected if it is already running? — Yes, live-confirmed, and better than the plan expected

Kanata was already running when this task reached a14 — **not started by
this investigation, and never stopped by it.** The only process alive was:

```
ProcessName                        Id
kanata_windows_tty_winIOv2_arm64  7868
```

Only the long executable name, no `Kanata.exe` shim process. Per
`docs/specs/2026-08-08-phase2a-design.md`, this is exactly the shape the
design called "a coin flip, not a fence": kanata's manifest names its real
executables only under `architecture.arm64.bin`/`shortcuts` as
`[filename, alias]` pairs (e.g. `["kanata_windows_tty_winIOv2_arm64.exe",
"Kanata"]`), and the note predicted that launching via the long name directly
(rather than the `Kanata` shim) "matches nothing."

**Measured, not started:** a probe lock entry was added for `kanata` (forced
to a wrong version, `0.0.1`) without touching the running process at all, and
`dotpkg status` printed:

```
  ! scoop  kanata         running -- stop it first
```

**It matched — via the long name, not just the alias — contrary to the
plan's own stated uncertainty.** The reason is in `declared_executables`
(`src/backend/scoop.rs`): its depth-first collector records *every* string in
a `bin`/`shortcuts` array, and a `[filename, alias]` pair is just an array of
two strings to that walk — both `kanata_windows_tty_winiov2_arm64` (the real
name) and `kanata` (the alias) end up as independent entries in
`Installed.bins`, not only the alias. So detection does not, in practice,
depend on which of the two names the process runs under, which is a stronger
guarantee than `docs/specs/2026-08-08-phase2a-design.md` describes. Whether
this holds for a *shortcut-only* launch (Start Menu, GUI variant, not in
`bin` at all) was not tested — nothing running in that shape was observed on
this machine at probe time — so the design's warning about that specific path
is neither confirmed nor refuted here.

Side finding, incidental to this task: a pre-existing `KanataWatchdog`
scheduled task (`RunLevel Highest`, AutoHotkey-based) is registered on this
machine and is almost certainly what keeps kanata alive/restarted outside of
`dotpkg` entirely — worth knowing for context, out of `dotpkg`'s scope.

Kanata was never started, stopped, or otherwise touched by this task at any
point; `Get-Process` after cleanup shows the identical PID (`7868`)
throughout, confirming it was purely observed.

### 2. Are `neovim` and `ripgrep` matched by their real executables? — Yes

Started as children of the medium-integrity task itself (not from the
elevated `ssh` session — see the path-matching note under question 3 for why
that distinction matters):

- `nvim --headless -c "sleep 60" -c "qa!" <scratch file>` (PID `10376`, then
  `15208` on the second run) — the explicit `sleep`+`qa!` avoids relying on
  any assumption about how Neovim treats a non-tty stdin with no file
  argument.
- `rg --no-ignore -uu -e zz_dotpkg_probe_zz_never_matches_anything C:\Windows`
  (PID `6792`, then `11168`) — an unrestricted recursive search large enough
  to still be running a couple of seconds later.

With `pkg-probe.lock` forcing both to a wrong locked version (`0.0.1`):

```
  ! scoop  neovim         running -- stop it first
  ! scoop  ripgrep        running -- stop it first
```

Neither an upgrade nor a downgrade line, exactly as required. Both processes
were confirmed killed immediately after (`Stop-Process` by PID, then
re-checked: `nvim still alive: False`, `rg still alive: False`, and absent
from the final post-cleanup process listing too).

### 3. Is `nodejs` caught by path matching alone? — Yes

`node -e "setTimeout(()=>{}, 120000)"` (PID `14056`, then `15496`), started
**as a child of the medium-integrity scheduled task**, deliberately not from
the elevated `ssh` session. This distinction — flagged before this task
resumed, and written into the regenerated brief — mattered in practice: the
design doc records that `sysinfo` cannot read `exe()` for a process at a
*higher* integrity level than the reader, so a `node.exe` started at High IL
over plain `ssh` would have been invisible to a Medium-IL `dotpkg`, for
`nodejs` the *only* signal there is (its manifest names no executable
anywhere). Started correctly, its path resolved and matched:

```
node        14056 C:\Users\kln\scoop\apps\nodejs\current\node.exe
```

```
  ! scoop  nodejs         running -- stop it first
```

**Clean negative control in the same run:** `rustup` also had a forced
wrong-version lock entry, but no `cargo`/`rustup` process was started for it.
It fell through to an ordinary version-change line rather than being
spuriously skipped:

```
  v scoop  rustup         1.29.0 -> 0.0.1          (downgrade, from lock)
```

This confirms the running-check is not a blanket skip — only the package
with an actual live process under it was caught. `node` was confirmed killed
afterward (`node still alive: False`).

### 4. Do all thirty apps still scan, with no new warnings? — Yes

Raw `apps` directory listing, 31 entries (30 real packages + `scoop` itself,
excluded from `status` by name):

```
7zip, actionlint, age, aichat, antigravity, bat, beckon, dark, fastfetch,
fzf, gh, git, go, innounp, kanata, lazydocker, lazygit, neovim, nodejs,
opencode, python, ripgrep, rustup, scoop, shfmt, stylua, tree-sitter, uv,
yamlfmt, yazi, zellij
```

Identical to Phase 1's listing. Every declared/unmanaged name `status`
printed traces to this list; nothing fabricated, nothing missing. **Zero
`warning: scoop:` lines** appeared in either status invocation's captured
output (both stdout and stderr were merged and captured). This matches
Phase 1's own medium-integrity ("Round 2") result — all 30 apps visible, no
warnings — rather than that same task's *elevated*-SSH run, which lost 3
apps to the NTFS-junction/integrity mismatch and is exactly why Step 3 exists.

### 5. Does the stylua drift line appear, with `1 architecture drift`? — Yes, exactly as predicted

Appeared verbatim, identically in both the probe-lock run and the plain
(`pkg.lock`, nonexistent) run:

```
  ~ scoop  stylua         64bit, declared arm64    (architecture drift -- reported, not fixed)
```

```
  0 change(s), 25 skipped, 1 architecture drift
```

(`1 change(s), 24 skipped, 1 architecture drift` in the probe-lock run, where
4 packages had forced version changes instead of "no lock entry" — the drift
count is unaffected by the lock, exactly as `plan()`'s ordering promises:
architecture is checked before the lock, independently of the version
verdict.) `python` (`arch = "64bit"`, matching its real installed
architecture) and `rustup` (`arch = "keep"`) correctly produced **no** drift
line each — only `stylua` did, so the count of `1` is right, not incidental.

## Verbatim output (probe-lock run, in full)

```
  ! scoop  git            no lock entry -- run `dotpkg update`
  ! scoop  nodejs         running -- stop it first
  ! scoop  gh             no lock entry -- run `dotpkg update`
  ! scoop  bat            no lock entry -- run `dotpkg update`
  ! scoop  ripgrep        running -- stop it first
  ! scoop  fzf            no lock entry -- run `dotpkg update`
  ! scoop  fastfetch      no lock entry -- run `dotpkg update`
  ! scoop  neovim         running -- stop it first
  ! scoop  tree-sitter    no lock entry -- run `dotpkg update`
  ! scoop  lazygit        no lock entry -- run `dotpkg update`
  ! scoop  lazydocker     no lock entry -- run `dotpkg update`
  ! scoop  yazi           no lock entry -- run `dotpkg update`
  ! scoop  zellij         no lock entry -- run `dotpkg update`
  ! scoop  opencode       no lock entry -- run `dotpkg update`
  ! scoop  shfmt          no lock entry -- run `dotpkg update`
  ! scoop  yamlfmt        no lock entry -- run `dotpkg update`
  ! scoop  stylua         no lock entry -- run `dotpkg update`
  ! scoop  actionlint     no lock entry -- run `dotpkg update`
  ! scoop  kanata         running -- stop it first
  ! scoop  beckon         no lock entry -- run `dotpkg update`
  ! scoop  python         no lock entry -- run `dotpkg update`
  ! scoop  go             no lock entry -- run `dotpkg update`
  v scoop  rustup         1.29.0 -> 0.0.1          (downgrade, from lock)
  ! scoop  uv             no lock entry -- run `dotpkg update`
  ! scoop  age            no lock entry -- run `dotpkg update`
  ~ scoop  stylua         64bit, declared arm64    (architecture drift -- reported, not fixed)
  ? scoop  aichat         0.30.0                   (unmanaged -- no action)
  ? scoop  antigravity    2.0.6                    (unmanaged -- no action)

  1 change(s), 24 skipped, 1 architecture drift
```

(This is the run with the kanata probe entry added; the neovim/ripgrep/nodejs
probe-only run is identical except `kanata` reads `no lock entry` instead of
`running`, since that run's lock did not yet include it.)

## Step 5: the machine-wide architecture picture

Read directly, over the plain elevated `ssh` session (no medium-integrity
task needed for this — the brief frames it as a way *around* the elevated
session's limitation, and that held up). Refined slightly from the brief's
literal suggestion: rather than assuming exactly one non-`current`
subdirectory per app, each app's `current` junction *target* was resolved via
`(Get-Item ...\current -Force).Target` — reading a reparse point's own
metadata, which is a different operation from traversing through it as a
path prefix and is not subject to the ownership check that blocks the latter
— and `install.json` was then read from that specific resolved version
directory.

**20 `arm64`, 10 `64bit` — matches the design doc's measurement exactly.**

| Architecture | Count | Apps |
|---|---|---|
| `arm64` | 20 | 7zip, bat, beckon, fastfetch, fzf, gh, git, go, kanata, lazydocker, lazygit, neovim, nodejs, opencode, ripgrep, rustup, tree-sitter, uv, yamlfmt, yazi |
| `64bit` | 10 | actionlint, age, aichat, antigravity, dark, innounp, python, shfmt, stylua, zellij |

Of the ten `64bit` (emulated) apps: `python` is deliberate
(`arch = "64bit"` in `pkg.toml`), `dark` and `innounp` are scoop's own
extraction helpers. That leaves **`actionlint`, `age`, `aichat`,
`antigravity`, `shfmt`, `stylua`, `zellij` — seven emulated without an
obvious reason**, matching the design document's "roughly seven emulated"
prediction exactly, both in count and (as far as can be checked) in kind.

## What came back different from the plan's prediction

Consolidated here per the brief's own request, even though each is also
noted where it happened:

1. **The medium-integrity scheduled-task technique, exactly as specified in
   the brief, did not work in this session** and needed real investigation to
   fix — see "Running at medium integrity" above in full. This is the
   headline finding of this task: a working, previously-dogfooded technique
   (used successfully in Phase 1) failed under `New-ScheduledTaskPrincipal
   -LogonType Interactive|S4U -RunLevel Limited` built fresh via PowerShell
   cmdlets, on a machine that had an active, unlocked interactive session for
   `kln` the entire time — and was fixed only by cloning the exact
   `<Principal>`/`<Settings>` XML of a different, already-registered task
   known to work (`AHKWatchdog`). The likely-relevant XML difference (SID vs.
   plain username as `UserId`, `InteractiveToken` vs. whatever `-LogonType
   Interactive` actually serializes to, an omitted vs. explicit `<RunLevel>`)
   was not isolated further; a follow-up dogfood or a `dotpkg`-unrelated
   investigation could usefully bisect it, since Phase 2b will likely reuse
   this exact technique for its own real (non-read-only) checks.
2. **Kanata's detection turned out not to depend on launch method, contrary
   to the design document's explicit "coin flip, not a fence" framing** —
   see question 1. The over-collecting depth-first walk in
   `declared_executables` happens to catch both the real filename and the
   alias in every `[filename, alias]` pair, which is a stronger guarantee
   than the design predicted, for a reason (over-collection as the
   deliberately safe default) the design doc already states but did not
   connect to this specific consequence.
3. Minor, environmental rather than about `dotpkg`: **`query`/`quser`/`query
   session` are not available commands on this machine.** Their absence
   initially looked like "nobody is logged on," which would have been a
   plausible but wrong explanation for the Task Scheduler symptom above; it
   was caught by checking a second, more direct signal (`explorer.exe`'s
   owner) rather than trusting the first tool's silence. Worth remembering
   for any future diagnostic work on this host.
4. `runas /trustlevel:0x20000` looked like a promising simpler alternative
   to the whole scheduled-task mechanism and was tested directly; it filters
   group membership correctly but does **not** relabel integrity to Medium on
   this machine, so it does not substitute for the scheduled-task technique
   here. Recorded so nobody re-tries it expecting a different result.
5. The build was 5.53s, not Phase 1's 45.42s — almost certainly a warm
   `target/` directory left over from Phase 1's own build in the same
   folder, not a property of anything in this task.

Everything else matched the plan: the five questions' *expected* answers were
all confirmed once the medium-integrity technique itself was working, and
the architecture count matched the design document exactly.

## Step 6: cleanup, verified

`DotpkgPhase2a` unregistered; `Get-ScheduledTask -TaskName DotpkgPhase2a`
afterward found nothing. Every file this investigation created was deleted
and independently re-checked with `Test-Path`, all returning `False`
afterward:

```
check-out.txt, build-out.txt, arch-out.txt, diag-out.txt, diag2-out.txt,
diag3-out.txt, diag4-out.txt, register-s4u-out.txt, schtasks-classic-out.txt,
register-natural-out.txt, runas-test-out.txt, runas-whoami.txt,
clone-xml-out.txt, clone-register-out.txt, ahkwatchdog-original.xml,
dotpkgphase2a-cloned.xml, t8-run-check.ps1, nvim-scratch.txt,
nvim-stdout.txt, nvim-stderr.txt, rg-stdout.txt, rg-stderr.txt,
node-stdout.txt, node-stderr.txt, pkg-probe.lock, pkg.lock
```

All of the above were authored by the investigator as scaffolding — none by
`dotpkg`, which writes nothing. `C:\Users\kln\dotpkg-build\` (source +
`target\release\dotpkg.exe`) and `C:\Users\kln\pkg.toml` (this task's
version, with the `stylua` addition) were deliberately **kept**, matching
Phase 1's own precedent, for whatever follow-up work comes next.

Processes: every `nvim`, `rg`, and `node` process started by this task
(PIDs `10376`/`15208`, `6792`/`11168`, `14056`/`15496` respectively) was
stopped by PID immediately after use and re-confirmed dead both right after
killing and again at final cleanup (`Get-Process` for `nvim`/`rg`/`node`
found nothing). `kanata` (PID `7868`) was never started or stopped by this
task at any point and was still running, same PID, at final cleanup —
observed only, exactly as required.

## Read-only, verified

- `C:\Users\kln\pkg.lock`: absent before this task touched the machine,
  absent after (confirmed inside both status runs and again at cleanup).
- `C:\Users\kln\pkg-probe.lock`: created by the investigator as scaffolding,
  deleted and confirmed absent at cleanup.
- `%LOCALAPPDATA%\dotpkg\state.json`: absent before, absent after (checked
  inside both status runs and again at final cleanup).
- `Cargo.lock` was copied verbatim and never modified by this task (no
  `cargo update` was run); the build did not need to re-resolve anything
  since the same lockfile already builds clean on macOS. This reasoning was
  not additionally verified by an on-machine hash comparison this round, to
  avoid a further round-trip against a machine that had already dropped
  offline once this session — flagged here rather than silently assumed.
- Nothing in `dotpkg`'s `status` path (`config::load`, `lock::load_or_empty`,
  `State::load_or_empty`, `Scoop::scan`, `sys::running_processes`,
  `Scoop::running_set`, `plan::plan`, `render::render`) performs a write, a
  subprocess spawn, or a network call — unchanged from Phase 1's review of
  this same code path, and Phase 2a added no new I/O to it.

## Machine facts confirmed or updated by this task

- `ssh a14` still runs at High Mandatory Level with no UAC — confirmed again
  (this session's own diagnostics, and the fact the scheduled-task detour was
  needed at all).
- `kln` is a local **Administrator**, not a standard user — newly recorded
  here; relevant to why `RunLevel Limited`'s token-filtering matters at all
  on this machine (a standard-user account would need none of this).
- Two more scheduled tasks exist on this machine outside `dotpkg`'s scope,
  discovered incidentally: `KanataWatchdog` (`RunLevel Highest`) and
  `AHKWatchdog` (`RunLevel Limited`, `LogonType` cloned above), both running
  `C:\Program Files\AutoHotkey\v2\AutoHotkey64.exe` against
  `C:\Users\kln\.nix\home-manager\dotfiles\windows\ahk\launch-ahk.ahk`.
- `query`, `quser`, and `query session` are not available on this Windows
  build.
