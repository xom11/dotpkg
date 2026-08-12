# Measurements: the version change, performed by the published binary

Taken 2026-08-12 on a14 (`zenbook-a14`, aarch64), against the artifact published
for 0.1.0 — sha256 `9daeae0cf5159d1096447340fda9e5534a38430b0eedaefc4e5ab8d7bd23d46f`,
the same bytes `docs/measurements-2026-08-12-release-0.1.0.md` §2 records as the
published build.

This closes the scoop half of open item 29. **A scoop version change is
uninstall-then-install** — installing over an installed app is a measured no-op —
so between the two halves the package is **absent**. The design calls that the
irreducible gap. CI had watched it happen with a locally built binary; nothing
had ever watched the artifact a user downloads do it.

## 1. Both directions, and the disk is the verdict

Subject: `jq`, absent before the round. Both versions are reachable in the
bucket's own history, and both commits were **resolved from the bucket by the
script** rather than typed:

```
commit_1_8_1 = 115388a583507a41d9e1a119d06fa98b7858417a
commit_1_8_2 = 4dc49cc8efd8ee879e25d010186f2c46c49aa151
```

| step | plan line | disk after |
|---|---|---|
| 1.8.2 → 1.8.1 | `v scoop jq 1.8.2 -> 1.8.1 (downgrade, from lock, arm64)` | **1.8.1** |
| 1.8.1 → 1.8.2 | `^ scoop jq 1.8.1 -> 1.8.2 (upgrade, 64bit)` | **1.8.2** |

Both reported `done scoop jq verified on disk` and `1 verified on disk, 0
failed, 0 held`, exit 0. `shims=jq.exe, jq.shim` after each — the shims survived
the window in which the package is absent — and ownership survived both, which
it must: ownership is intent, and the uninstall half is an implementation
detail.

## 2. The architecture changed underneath the version, and dotpkg said so

**`arm64` on the way down, `64bit` on the way up.** The same package, on the
same ARM64 machine, resolved to a different architecture at the two versions.

That is exactly the failure the design's `arch` option exists for: *"scoop
installs the wrong architecture silently … that combination left the author's
machine with 17 emulated packages and nothing anywhere saying why."* Here
something did say why — the architecture is on the plan line, before the user
consents. Recorded as the feature working rather than as a defect, and as the
first time it has been observed happening across a version change on real
hardware.

## 3. `ScanOutcome::Unscannable` fired on real hardware, for the first time

Every command in this round printed:

```
warning: winget: could not be scanned: winget list could not be run:
cannot run winget ["list", "--disable-interactivity"]: Access is denied. (os error 5)
```

**This is a change in the machine between rounds, not something this round did.**
Seven hours earlier, on the same machine in the same kind of session,
`docs/measurements-2026-08-12-phase8-release-apply.md` §7.2 records dotpkg
reading **36** source-backed winget ids while PowerShell's own `& winget` failed.
Now dotpkg cannot run it either. Nothing here touched winget.

What matters is what dotpkg did with it: the unmanaged count went from 60 to
**24 — scoop only** — and the winget backend was reported as unreadable rather
than as empty. That is `ScanOutcome::Unscannable` doing the one job it was added
for, live: *an empty `Scan` returned in place of a real failure is
indistinguishable from a genuinely empty machine*, and a genuinely empty machine
is what `mass_prune_guard` exists to catch far too late.

**And it bounds this round.** A winget mutation round was planned for the same
sitting and was **not attempted**, for two reasons that compound: dotpkg cannot
reach winget on that machine at present, and from this session nothing *else*
can read winget either — so any winget mutation would have been performed and
reported by the same tool, with no independent oracle. `src/execute.rs`'s own
module doc names that hazard: *"a fake that both performs and reports the
mutation proves only that it is self-consistent."*

## 4. Left as found, with the attribution that made it possible

`jq` pruned: `jq_app_dir=False`, `jq_shims=0`, `scoop_apps_count=31` (unchanged
from the start), state released to `{ "scoop": {} }`, three cache entries removed
(`jq#1.8.1#a8d6163`, `jq#1.8.2#566ee08`, `jq#1.8.2#abde28e`, cache back to 92),
and **0 `ph11` artefacts remaining**. kanata was observed at pid **13992** and
never signalled.

**The one deletion that was correctly NOT made.** `%LOCALAPPDATA%\dotpkg` held
staged manifests for `fastfetch`, `git`, `lazygit`, `nodejs`, `opencode`,
`tree-sitter` and `uv`, dated **18:21** — and the ph8 round removed that whole
directory at about **16:52**. So a dotpkg run that was not this session's
recreated it in between, and those entries were left alone; only the `jq`
entries, dated 23:04, were removed. Without ph8's timestamp lesson this round
would have deleted somebody else's state.

**`ph11-pkg.lock.bak` and `ph11-state.json.bak` appeared and were cleaned up.**
Worth stating rather than passing over: the installed binary is 0.1.0, and the
change that stops writing a `.bak` beside the two committed files landed after
it. The released artifact still writes all three. That is the expected gap
between `main` and the last tag, observed rather than assumed.

## 5. A method failure, and it is the fifth instance of one habit

**The first attempt at this round produced no version change at all**, and the
cause was the script's, not dotpkg's. It piped a *mutating* command into
`Select-Object -First 8`. PowerShell stops an upstream pipeline by throwing
`StopUpstreamCommandsException`, which **killed dotpkg mid-install**; the run
reported `install_exit=-1` and the disk read `ABSENT`. The next stage then saw
no installed package and planned an ordinary `(install)` rather than a version
change. Every figure in §1 comes from the re-run, where nothing that changes the
machine is piped into anything that can truncate it.

Two more of the same family, both caught by their own gate this time:

- **A fabricated sha.** A draft pasted a 12-character prefix from a survey and
  padded it to 40 characters by hand. The final script resolves both commits
  from the bucket and refuses to proceed unless each matches `^[0-9a-f]{40}$`.
- **Backticks in a `.ps1`, twice.** The standing rule is zero, in code and in
  comments, and both instances were markdown-style quoting of a command name in
  a comment — **the fourth and fifth instances of the class this project has
  recorded**. The first slipped through because the check *printed* its count
  and the transfer ran anyway; the check now **blocks** the transfer, and it
  caught the second one before it left the machine.

The rule that survives all three: a check that reports is not a gate. A gate
refuses.
