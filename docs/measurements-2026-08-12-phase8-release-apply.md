# Measurements: `apply` exercised from the published release binary

Taken 2026-08-12 on a14 (`zenbook-a14`, aarch64, 8 logical cores), against the
artifact published for 0.1.0 rather than against a rebuild of it.

This settles **open item 23** — *"`apply` has never been exercised from a
release binary. Only `status`, which mutates nothing."* It does not settle item
22, and §6 says what else it leaves alone.

## 1. The binary is the published one, not a build of the same commit

The 0.1.0 release round found that this toolchain does not produce identical
bytes twice, so "a build of the commit" and "the artifact a user downloads" are
different objects. This round used the second:

| | |
|---|---|
| path | `C:\Users\kln\.local\bin\dotpkg.exe` |
| bytes | 2058240 |
| sha256 | `9daeae0cf5159d1096447340fda9e5534a38430b0eedaefc4e5ab8d7bd23d46f` |
| `--version` | `dotpkg 0.1.0` |
| `--help` | four commands: `status`, `apply`, `update`, `adopt` |

That sha256 is the one `docs/measurements-2026-08-12-release-0.1.0.md` §2
records as **the published build**, agreeing across three independent
computations. `Get-FileHash` returns uppercase and was folded before comparing.

## 2. Blast radius, stated before the run

The subject was **`jq` 1.8.2** from the `main` bucket: single binary, no
dependencies, and **measured absent first** — an install proves nothing about a
package that was already there.

| bound | how it was enforced |
|---|---|
| ownership | `--state C:\Users\kln\ph8-state.json`, this round's own file, naming nothing at the start. The prune fence can reach only what a state file names, so the 31 packages already installed were unreachable however the config was written. |
| declaration | `C:\Users\kln\ph8-pkg.toml`, this round's own file. The machine's real `pkg.toml` is a symlink into the nix repo and was never read or passed to anything. |
| default state | `%LOCALAPPDATA%\dotpkg\state.json` did not exist before the round and was confirmed absent after every stage. |
| kanata | never started, never stopped, never signalled. See §5. |
| naming | every artefact carried a `ph8-` prefix, over a `Get-ChildItem` baseline of `C:\Users\kln` taken before anything ran. |

Idle gate before the mutating stage, recorded rather than assumed:
`machine_busy_pct 4.94` over a 6.05 s window, 8 cores, 0 processes over
threshold, **VERDICT: IDLE**. (`LoadPercentage` read 19 in the same sample and
is printed but not decided on, which is why the gate exists.)

## 3. Read-only first, and it really was read-only

| step | result |
|---|---|
| `update --config … --lock … --offline` | `+ scoop jq 1.8.2 (new pin)`, `1 changed, 0 unchanged, 0 could not be resolved`, exit 0 |
| lock written | `bucket = "main"`, `commit = "4dc49cc8efd8ee879e25d010186f2c46c49aa151"`, `version = "1.8.2"` |
| `status` | `+ scoop jq 1.8.2 (install)`, `1 change(s), 0 skipped, 60 unmanaged`, exit 0 |
| `apply --prepare` | `1 of 1 changes ready, 0 failed, 0 skipped, 0 not locked`, **`Nothing has been changed.`**, exit 0 |
| after `--prepare` | `jq_app_dir=False`, `ph8_state_exists=False`, `default_state_exists=False` |

`--offline` on purpose: `latest` then means what the machine last pulled, so the
round reaches no network and is reproducible against the same bucket head
(`main` at `1e9a9ef933ea4cb28e90d6fdec8469584f2da39a`).

## 4. The mutating stages, verified on disk rather than by exit code

### 4.1 Install

`apply --config … --lock … --state … --yes` printed
`done scoop jq verified on disk` and `1 verified on disk, 0 failed, 0 held`,
exit 0. **The verdict is the disk:**

| | before | after |
|---|---|---|
| `scoop\apps\jq` | False | **True** |
| `scoop\shims\jq.exe` | False | **True** |
| installed `manifest.json` version | — | **1.8.2**, the pinned version |
| `jq --version` | — | **`jq-1.8.2`** |
| `ph8-state.json` | absent | `{ "scoop": { "jq": "installed" } }` |
| `%LOCALAPPDATA%\dotpkg\state.json` | absent | **still absent** |

### 4.2 The refusal counterweight, run *before* the prune

Without this, the removal below would prove only that `apply` removes things,
not that the guards were ever in the way. Same command, same state, **flags
withheld**:

```
pkg.toml declares no scoop packages but dotpkg owns 1. Refusing to prune
everything. If the file is right, pass --allow-empty-config.
```

exit **2** — *refused before anything was attempted* — and
`jq_still_here_after_refusal=True`. The mass-prune guard fired on a real machine.

### 4.3 Prune

With `--yes --allow-prune --allow-empty-config`, after `update --offline`
dropped the pin (`- scoop jq 1.8.2 (dropped, no longer declared)`, lock left
empty):

| | |
|---|---|
| plan line | `- scoop jq 1.8.2 (prune, owned)` |
| result | `done scoop jq verified on disk`, `1 verified on disk, 0 failed, 0 held`, exit 0 |
| `scoop\apps\jq` | **False** |
| `scoop\shims\jq.exe` | **False** |
| `ph8-state.json` | `{ "scoop": {} }` — ownership released, not merely forgotten |
| other packages | `scoop_apps_count=31`, unchanged; `actionlint`, `antigravity`, `zellij`, `kanata`, `neovim`, `ripgrep` each still present |

So three guards were exercised live in one run: the mass-prune guard,
`--allow-empty-config`, and `--allow-prune` gating the `--yes` fast path.

## 5. Left as found, including the parts that carry no prefix

Prefix discipline proves only that what *was named* is gone. A timestamp sweep
was run for the rest, and it found two marks that no prefix cleanup would have
caught:

- **`%LOCALAPPDATA%\dotpkg\manifests\jq\1.8.2\4dc49cc8…\jq.json`** (1681 bytes)
  — the staged manifest. All four entries in that tree were from this round
  (`entries_predating_this_round=0`), so the tree and its now-empty parent were
  removed.
- **`scoop\cache\jq#1.8.2#566ee08.exe`** (973312 bytes) — what `--prepare`
  fetched. Removed; the cache went 86 → **85** entries.

Final: `ph8` artefacts remaining **0**, `jq_app_dir=False`, `jq_cache_entries=0`,
`localappdata_dotpkg=False`, `scoop_apps_count=31`.

**kanata was observed at pid 3976 before and after both mutating stages, and at
8424 during the final sweep.** Recorded as a landmark, never asserted: the
standing rule for this machine is that its pid changes between rounds. No
command in this round targeted it, no plan in this round contained it — the
config declared one package and the state file named one package — and its scoop
app directory was verified present after the prune.

## 6. What this round did NOT settle, and it is most of the surface

- **The version-change path was never exercised.** Only `Install` and `Remove`.
  A scoop `Replace` is uninstall-then-install and opens the window in which a
  package is absent; that is the most dangerous path this tool has and it still
  has no live evidence from a release binary.
- **No winget mutation.** The config declared no winget package deliberately.
  `install`, `upgrade` and `remove` on the winget side remain evidenced only by
  the Phase 4b rounds and their own trees.
- **No x86_64.** Open item 22 is untouched: no x64 Windows machine has run
  dotpkg at all.
- **One package, one bucket, one architecture, one session** — and that session
  was **elevated** (`IsInRole(Administrator)` True), so nothing here says how
  `apply` behaves from an ordinary session.
- `--keep-going`, `--clone-missing-buckets` and `--show-unmanaged` were not
  exercised.

## 7. Three things this round found that nobody was looking for

### 7.1 Three installed scoop packages cannot be read at all

`status`, `apply` and every scan warned:

```
warning: scoop: actionlint: cannot read manifest.json: The path cannot be
traversed because it contains an untrusted mount point. (os error 448)
```

— the same for `antigravity` and `zellij`. This is why the machine reports **24**
unmanaged scoop packages against **31** installed app directories: 31 − 3
unreadable − `scoop` itself − 3 helpers (`7zip`, `dark`, `innounp`) = 24.

dotpkg does the right thing here, and visibly: it says which package it could
not read rather than counting it as absent, which is the distinction
`ScanOutcome::Unscannable` exists to keep. **What is new is that the condition
occurs at all on this machine, and that it is silent to scoop itself.** Nothing
in `docs/` records os error 448 before this round.

### 7.2 winget is reachable by dotpkg and not by PowerShell, in the same session

`& winget --version` from the ssh session fails with `Access is denied`; the
alias at `AppData\Local\Microsoft\WindowsApps\winget.exe` is a **0-byte**
execution-alias stub. In the same session, dotpkg's own scan read **36**
source-backed winget ids and produced its full warning set (disagreeing
versions, collapsed duplicate rows, `> x.y.z` lower bounds, and the aggregate
line for 85 sourceless entries).

So the limitation is PowerShell's `&` against a 0-byte alias, **not** winget
being unavailable. Stated because the first probe in this round read it the
other way round, and a wrong reading of the environment would have been carried
into the conclusions.

### 7.3 dotpkg leaves `.bak` files beside what it rewrites, and nothing says so

After the round, two files existed that this round never wrote:

| file | bytes | contents |
|---|---|---|
| `ph8-pkg.lock.bak` | 100 | the **previous** lock — jq pinned, before `update` emptied it |
| `ph8-state.json.bak` | 42 | the **previous** state — jq owned, before the prune released it |

This is the durable-save path doing its job, and keeping the prior content is
defensible. What is not recorded anywhere is that the files are **left behind**:
a user who runs `update` and `apply` in their dotfiles repository gets
`pkg.lock.bak` and a `state.json.bak` sitting next to their real files, with no
mention in the README and no cleanup. Carried forward as open item 27.

## 8. Method note

Every dotpkg invocation was run from a `.ps1` transferred with `scp`, one file
per invocation, and started with
`powershell -NoProfile -ExecutionPolicy Bypass -File`. Inline PowerShell over
ssh was attempted once, for a one-line `Get-Item … | ForEach-Object { $_.Length }`,
and the local shell ate `$_` exactly as this project's standing note says it
does; the error read as a script bug and was not one. No script in this round
contains a backtick, in code or in comments.
