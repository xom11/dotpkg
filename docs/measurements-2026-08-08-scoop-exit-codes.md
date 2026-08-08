# Measurements: scoop's exit codes, and what surrounds them

Measured 2026-08-08 on a14, scoop **0.5.3** (`b588a06e`, "chore(release): Bump
to version 0.5.3 (resync) (#6436)"), `main` bucket at `04bd5e1acb` ("timoni:
Update to version 0.30.0"). `where.exe git` resolves to
`C:\Users\kln\scoop\apps\git\current\cmd\git.exe` first — git on this machine
is itself scoop-managed — reporting `git version 2.55.0.windows.3`.

This document is the raw record behind
`docs/specs/2026-08-08-phase2b2-executor-design.md`, "The measurement that
rewrites this phase". Four rounds, run in this order:

| File | What it is |
|---|---|
| `m1-out.txt` | Read-only reconnaissance against the real `~/scoop`. No mutation. |
| `m2-out.txt` | A throwaway `$env:SCOOP` probe: flag acceptance, install/uninstall behaviour, cache, timings, bucket add. |
| `m3-out.txt` | The exit-code round, measured with `System.Diagnostics.Process.ExitCode`. |
| `m4-out.txt` | The exit-code *path* investigation — is it the `.cmd` shim eating the code, or scoop itself? — plus cleanup verification. |

The real `~/scoop` root was checked before and after every round and never
changed: **31 app directories, 75 cache entries**, `kanata`'s only live
process `kanata_windows_tty_winIOv2_arm64` at **PID 7868**
(start `08/08/2026 09:49:25`), `explorer` at **PID 9620**
(start `08/07/2026 19:18:09`). Those four numbers recur at the end of `m2`,
`m3`, and `m4` unchanged; they are reproduced verbatim below at each
checkpoint rather than assumed.

**Exit codes were read with `System.Diagnostics.Process.ExitCode`, not with
`Start-Process -PassThru`.** `m2-out.txt` predates that decision: every
`exit:` field in it is blank (e.g. `exit: ` followed immediately by
`wall: 5.56 s`), because `Start-Process -PassThru`'s `ExitCode` property was
never populated by the time it was read. **`m2` is not a source for any
exit-code claim** — its stdout, timings, and filesystem-tree observations are
still real and are reported below, but its blank `exit:` fields are the reason
`m3` re-ran the same questions through `System.Diagnostics.Process`.

## The headline

**Scoop reports no operational failure through its exit code.** A wrong hash,
a dead URL, installing over a manifest path that does not exist, uninstalling
an app that was never installed — every one of those exits `0`. The only
command in the whole sweep that exited non-zero was an unknown subcommand.
And it is not the `.cmd` shim swallowing the code: `scoop.ps1` invoked
directly, through `powershell -File` and through `powershell -Command` with
`$LASTEXITCODE` echoed explicitly, both report exit `0` too.

## 1. `m1-out.txt` — read-only reconnaissance against the real `~/scoop`

### Which git

```
where.exe git -> C:\Users\kln\scoop\apps\git\current\cmd\git.exe | C:\Users\kln\scoop\shims\git.exe
Get-Command git .Source -> C:\Users\kln\scoop\apps\git\current\cmd\git.exe
git --version -> git version 2.55.0.windows.3
```

### Baseline

```
root = C:\Users\kln\scoop  exists=True
app dir count = 31
cache entry count = 75
```

30 installed packages were enumerated junction-safe (`scoop` itself has no
resolvable version, shown as `?`, and is excluded from the 30); each carries
`depends=[]` — see "Falsified: `depends`" below.

### M6 — is the installed `manifest.json` byte-faithful to the bucket blob?

```
skip 7zip: HEAD is 26.02, installed is 26.01
actionlint @ 1.7.12: identical=True  instLen=1615 headLen=1615
age @ 1.3.1: identical=True  instLen=1077 headLen=1077
aichat @ 0.30.0: identical=True  instLen=1111 headLen=1111
antigravity @ 2.0.6: identical=True  instLen=3462 headLen=3462
bat @ 0.26.1: identical=True  instLen=2644 headLen=2644
skip beckon: HEAD is 0.2.10, installed is 0.2.9
dark @ 3.14.1: identical=True  instLen=364 headLen=364
```

Six apps compared, all byte-identical. Two were skipped, not failed: the
bucket's HEAD has moved past the installed version for `7zip` (26.02 vs.
26.01 installed) and `beckon` (0.2.10 vs. 0.2.9 installed), so there is no
same-version blob to compare against. This is what makes byte comparison a
usable verification primitive in the executor design — but the comparison
only has six positive data points, and both skips are recorded, not silently
dropped.

### Baseline processes

```
kanata : not running
kanata_windows_tty_winIOv2_arm64 PID=7868 start=08/08/2026 09:49:25
explorer PID=9620 start=08/07/2026 19:18:09
any *kanata* -> kanata_windows_tty_winIOv2_arm64:7868
```

### Scoop version and bucket HEAD

```
Current Scoop version:
b588a06e chore(release): Bump to version 0.5.3 (resync) (#6436)

'main' bucket:
04bd5e1acb timoni: Update to version 0.30.0
```

## 2. `m2-out.txt` — the throwaway `$env:SCOOP` probe

```
probe root  = C:\Users\kln\AppData\Local\Temp\dotpkg-probe-root
stage dir   = C:\Users\kln\AppData\Local\Temp\dotpkg-probe-stage
scoop.cmd   = C:\Users\kln\scoop\shims\scoop.cmd  exists=True
env:SCOOP   = C:\Users\kln\AppData\Local\Temp\dotpkg-probe-root
```

Real manifests for `fzf` 0.74.1 and `go` 1.26.4 were staged, plus two crafted
failure manifests: `deadurl` 9.9.9 (a URL that does not exist) and `badhash`
0.74.1 (fzf's real archive, wrong hash).

### Flag acceptance on a manifest path (M2 + M5)

`scoop download` on the fzf manifest with no flags, `-u`, `-a arm64`, and
`-a 64bit` all produced a successful download. Representative stdout (no
flags, cold cache):

```
INFO  Downloading 'fzf' [arm64]
Downloading https://github.com/junegunn/fzf/releases/download/v0.74.1/fzf-0.74.1-windows_arm64.zip (1.9 MB)...
Checking hash of fzf-0.74.1-windows_arm64.zip ... ok.
'fzf' (0.74.1) was downloaded successfully!
```

`-a 64bit` fetched a **different artifact** than the default (arm64):

```
INFO  Downloading 'fzf' [64bit]
Downloading https://github.com/junegunn/fzf/releases/download/v0.74.1/fzf-0.74.1-windows_amd64.zip (2.1 MB)...
Checking hash of fzf-0.74.1-windows_amd64.zip ... ok.
'fzf' (0.74.1) was installed successfully!
```

(Line above transcribed exactly as captured — "downloaded" is what the
`download` subcommand actually printed in the other three cases; this
confirms `-a` changes which artifact is fetched, which is the basis for
"`-a` is mandatory" in the executor design.)

`scoop install -u -a arm64` on the fzf manifest into the empty probe root
installed successfully:

```
Installing 'fzf' (0.74.1) [arm64] from 'C:\Users\kln\AppData\Local\Temp\dotpkg-probe-stage\fzf\0.74.1\fzf.json'
Loading fzf-0.74.1-windows_arm64.zip from cache
Checking hash of fzf-0.74.1-windows_arm64.zip ... ok.
Extracting fzf-0.74.1-windows_arm64.zip ... done.
Linking ~\AppData\Local\Temp\dotpkg-probe-root\apps\fzf\current => ~\AppData\Local\Temp\dotpkg-probe-root\apps\fzf\0.74.1
Creating shim for 'fzf'.
Adding ~\AppData\Local\Temp\dotpkg-probe-root\shims to your path.
'fzf' (0.74.1) was installed successfully!
```

stderr on every one of the above (and nearly everything else in this round)
carried the same non-fatal noise, ANSI colour codes included, from
`buckets.ps1:61`'s `Get-ChildItem` failing to find a `buckets` directory that
does not exist in a fresh probe root — never a sign that anything went wrong.

### M4 — what a successful install created

```
apps/fzf contents: 0.74.1, current [Junction]
current -> C:\Users\kln\AppData\Local\Temp\dotpkg-probe-root\apps\fzf\0.74.1
install.json = {     "architecture": "arm64",     "url": "C:\\Users\\kln\\AppData\\Local\\Temp\\dotpkg-probe-stage\\fzf\\0.74.1\\fzf.json" }
manifest.json version = 0.74.1
installed manifest identical to staged = True
shims/: fzf.shim
```

**Contaminated: shim creation was not measurable.** The probe root has no
`apps/scoop`, so scoop could not copy `shim.exe` into
`apps\scoop\current\supporting\shims\kiennq\shim.exe` (stderr shows exactly
that `Copy-Item` failure, "Cannot find path … because it does not exist").
Only `fzf.shim` was created; `fzf.exe` was not. **Nothing about real shim
behaviour on a properly-provisioned root follows from this run.**

### M2b — install over an already-installed app

```
--- install fzf AGAIN, same manifest
    argv: scoop install -u "C:\Users\kln\AppData\Local\Temp\dotpkg-probe-stage\fzf\0.74.1\fzf.json"
    STDOUT (93 chars):
      | WARN  'fzf' (0.74.1) is already installed.
      | Use 'scoop update fzf' to install a new version.
    STDERR (0 chars):
apps/fzf before = 0.74.1,current
apps/fzf after  = 0.74.1,current
unchanged = True
```

This is the first sighting of the `WARN` line — see "Corrections to
`docs/phase2b-notes.md`" below.

### M1 (internal label) — cache reuse

```
cache entries: fzf#0.74.1#54d353d.zip [1981965b] ; fzf#0.74.1#bd3be84.zip [2181266b]
```

Two distinct cached files for one version — the arm64 and 64bit artifacts —
confirming a prefetch that omits `-a` warms the wrong one.

### M3a (internal label) — residue of a successful uninstall

```
--- uninstall fzf
    argv: scoop uninstall fzf
    STDOUT (174 chars):
      | Uninstalling 'fzf' (0.74.1).
      | Removing shim 'fzf.shim'.
      | Removing shim 'fzf.exe'.
      | Unlinking ~\AppData\Local\Temp\dotpkg-probe-root\apps\fzf\current
      | 'fzf' was uninstalled.
apps/fzf still exists = False
shims/ after uninstall: (empty)
cache after uninstall: fzf#0.74.1#54d353d.zip, fzf#0.74.1#bd3be84.zip
persist/ exists = False
```

Uninstall is clean: `apps/<app>` is gone entirely, the cache is kept,
`persist/` was never created.

### M10a (internal label) — wall-clock timing, warm cache

| Step | Wall time |
|---|---|
| reinstall fzf from warm cache | 3.92 s (install-only: 6.01 s once, includes a `Checking hash` line) |
| uninstall fzf | 4.66 s |
| reinstall fzf again | 3.79 s |
| **full uninstall+install window** | **11.63 s** |

Spawn-dominated at this package size, not extraction-dominated.

### M11 — download failure modes, exit code vs. stdout/stderr

All three exited with a blank `exit:` field in this round (see the method
note above) — this is exactly the observation that motivated the `m3` redo.
The stdout content is still real:

```
--- download DEAD URL
    STDOUT (210 chars):
      | INFO  Downloading 'deadurl' [arm64]
      | The remote server returned an error: (404) Not Found.
      | ERROR URL https://github.com/xom11/definitely-not-a-real-repo-9f2a/releases/download/v9.9.9/nothing.zip is not valid

--- download BAD HASH
    STDOUT (633 chars):
      | INFO  Downloading 'badhash' [arm64]
      | Downloading https://github.com/junegunn/fzf/releases/download/v0.74.1/fzf-0.74.1-windows_arm64.zip (1.9 MB)...
      | Checking hash of fzf-0.74.1-windows_arm64.zip ... ERROR Hash check failed!
      | App:         badhash
      | URL:         https://github.com/junegunn/fzf/releases/download/v0.74.1/fzf-0.74.1-windows_arm64.zip
      | First bytes: 50 4B 03 04 14 00 08 00
      | Expected:    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
      | Actual:      b688ecafa2d1fdb0af3383f25d6d122866c13ad7cc996e9f735bf90e6c75f83f
      | ERROR Please contact the bucket maintainer!
      | 'badhash' (0.74.1) was downloaded successfully!
```

Note the `Expected:` hash above is scoop's own crafted-failure placeholder
(all `f`s) from the badhash manifest, not a real upstream hash — the manifest
was deliberately authored wrong for this probe.

### M8 — does `scoop bucket add` clone shallow?

```
--- bucket add xom11
    STDOUT (63 chars):
      | Checking repo... OK
      | The xom11 bucket was added successfully.
is-shallow = false
commit count = 16
```

Full clone, not shallow. Old pins in a bucket survive `scoop bucket add`.

### Probe state at end of `m2`, and the real root re-checked

```
[tree] final
  apps/ : 1 -> fzf
  cache/ : 2 -> fzf#0.74.1#54d353d.zip, fzf#0.74.1#bd3be84.zip
  shims/ : 1 -> fzf.shim
  buckets/ : 1 -> xom11
  persist/ : (absent)

real app dir count = 31
real cache count   = 75
kanata -> kanata_windows_tty_winIOv2_arm64:7868
```

## 3. `m3-out.txt` — the exit-code round

Reusing the tree left by `m2` (`apps/fzf` = `[0.74.1,current]`). A second fzf
manifest, version 0.74.2, was staged for the "install a different version
over an installed one" case. Exit codes here are `System.Diagnostics.Process`
values — real, not the blank fields from `m2`.

### E1 — download BAD HASH

```
EXIT CODE : 0
stdout    :
  | INFO  Downloading 'badhash' [arm64]
  | Downloading https://github.com/junegunn/fzf/releases/download/v0.74.1/fzf-0.74.1-windows_arm64.zip (1.9 MB)...
  | Checking hash of fzf-0.74.1-windows_arm64.zip ... ERROR Hash check failed!
  | App:         badhash
  | URL:         https://github.com/junegunn/fzf/releases/download/v0.74.1/fzf-0.74.1-windows_arm64.zip
  | First bytes: 50 4B 03 04 14 00 08 00
  | Expected:    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
  | Actual:      b688ecafa2d1fdb0af3383f25d6d122866c13ad7cc996e9f735bf90e6c75f83f
  | ERROR
  | Please try again or create a new issue by using the following link and paste your console output:
  | https:////
  | 'badhash' (0.74.1) was downloaded successfully!
```

That `https:////` line is exactly what scoop printed — a template with an
unfilled variable, verbatim, four slashes, no host. Reproduced here with its
exact capitalisation and punctuation because it is the clearest single
illustration of the headline: scoop's own crash-report link is broken, and it
still reports success on the very next line.

### E2 — download DEAD URL

```
EXIT CODE : 0
stdout    :
  | INFO  Downloading 'deadurl' [arm64]
  | The remote server returned an error: (404) Not Found.
  | ERROR URL https://github.com/xom11/definitely-not-a-real-repo-9f2a/releases/download/v9.9.9/nothing.zip is not valid
```

### E3 — download OK (cached)

```
EXIT CODE : 0
stdout    :
  | INFO  Downloading 'fzf' [arm64]
  | Loading fzf-0.74.1-windows_arm64.zip from cache
  | Checking hash of fzf-0.74.1-windows_arm64.zip ... ok.
  | 'fzf' (0.74.1) was downloaded successfully!
```

### E4 — install SAME version over installed

```
EXIT CODE : 0
stdout    :
  | WARN  'fzf' (0.74.1) is already installed.
  | Use 'scoop update fzf' to install a new version.
[state] after E4 : apps/fzf = [0.74.1,current]  current version = 0.74.1
```

### E5 — install DIFFERENT version over installed

The comment in the raw output names exactly what this test was checking:
"the notes claim: exit 0, no output, no change".

```
argv      : scoop install -u -a arm64 C:\...\dotpkg-probe-stage\fzf\0.74.2\fzf.json
EXIT CODE : 0
stdout    :
  | WARN  'fzf' (0.74.1) is already installed.
  | Use 'scoop update fzf' to install a new version.
[state] after E5 : apps/fzf = [0.74.1,current]  current version = 0.74.1
```

**This falsifies half of the old note and confirms the other half.** There
*is* output — a `WARN` line on stdout, not silence — and nothing changes. The
version the line names is **0.74.1, the version already installed**, even
though 0.74.2 was the manifest actually passed on the command line. See
"Corrections to `docs/phase2b-notes.md`" below.

### E6 — install BAD HASH manifest

```
EXIT CODE : 0
stdout    :
  | Installing 'badhash' (0.74.1) [arm64] from 'C:\...\dotpkg-probe-stage\badhash\0.74.1\badhash.json'
  | Downloading https://github.com/junegunn/fzf/releases/download/v0.74.1/fzf-0.74.1-windows_arm64.zip (1.9 MB)...
  | Checking hash of fzf-0.74.1-windows_arm64.zip ... ERROR Hash check failed!
  | App:         badhash
  | URL:         https://github.com/junegunn/fzf/releases/download/v0.74.1/fzf-0.74.1-windows_arm64.zip
  | First bytes: 50 4B 03 04 14 00 08 00
  | Expected:    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
  | Actual:      b688ecafa2d1fdb0af3383f25d6d122866c13ad7cc996e9f735bf90e6c75f83f
  | Please try again or create a new issue by using the following link and paste your console output:
  | https:////
[state] apps/badhash exists = True
```

An `install` with a wrong hash exits 0 **and leaves a directory behind**
(`apps/badhash` exists), unlike the clean "nothing changed" of E4/E5. What
that directory actually contains is answered in `m4`, section A, below.

### E7 — uninstall an app that is not installed

```
EXIT CODE : 0
stdout    :
  | ERROR 'definitely-not-installed-9f2a' isn't installed.
```

### E8 — uninstall fzf (installed)

```
EXIT CODE : 0
stdout    :
  | Uninstalling 'fzf' (0.74.1).
  | Removing shim 'fzf.shim'.
  | Removing shim 'fzf.exe'.
  | Unlinking ~\AppData\Local\Temp\dotpkg-probe-root\apps\fzf\current
  | 'fzf' was uninstalled.
[state] after E8 : apps/fzf ABSENT
```

### E9 — uninstall fzf AGAIN (now absent)

```
EXIT CODE : 0
stdout    :
  | ERROR 'fzf' isn't installed.
```

### E10 — install a manifest path that does not exist

```
EXIT CODE : 0
stdout    :
  | Couldn't find manifest for 'nope' at 'C:\...\dotpkg-probe-stage\nope\1.0.0\nope.json'.
```

### E11 — bucket add a bucket that is already added

```
EXIT CODE : 0
stdout    :
  | WARN  The 'xom11' bucket already exists. To add this bucket again, first remove it by running 'scoop bucket rm xom11'.
```

### Real root, re-checked at the end of `m3`

```
real app dir count = 31
real cache count   = 75
kanata -> kanata_windows_tty_winIOv2_arm64:7868
```

### Exit-code summary, this round

| Invocation | Exit | Notable stdout |
|---|---|---|
| download, bad hash | 0 | `ERROR Hash check failed!` … then "was downloaded successfully!" |
| download, dead URL | 0 | `ERROR URL … is not valid` |
| download, cached OK | 0 | success |
| install, same version, already installed | 0 | `WARN … is already installed.` — no change |
| install, different version, already installed | 0 | same `WARN`, names the *installed* version, not the requested one — no change |
| install, bad hash manifest | 0 | hash failure, **and `apps/badhash/` is created anyway** |
| uninstall, not installed | 0 | `ERROR '…' isn't installed.` |
| uninstall, installed | 0 | success |
| uninstall, now absent (again) | 0 | `ERROR '…' isn't installed.` |
| install, nonexistent manifest path | 0 | `Couldn't find manifest for '…' at '…'.` |
| bucket add, already added | 0 | `WARN The '…' bucket already exists.` |

Eleven invocations, eleven exit codes, all `0`. No failure mode tried in this
round produced anything else.

## 4. `m4-out.txt` — is it the `.cmd` shim, and what does a failed install leave?

### A. what does `apps/badhash` contain after the failed install (E6)?

```
exists = True
  \apps\badhash\0.74.1\
  \apps\badhash\0.74.1\fzf-0.74.1-windows_arm64.zip  [1981965b]
current exists = False
current\manifest.json exists = False
current\install.json exists = False
shims/ now: (empty)
```

Only the downloaded (bad-hash) archive is present. No `current` junction, no
`manifest.json`, no `install.json`. The raw output's own conclusion:

```
--- would Scoop::scan() SEE badhash as installed? (manifest.json readable under current, with a version) ---
no current\manifest.json -> scan SKIPS it silently (continue on NotFound)
```

A half-install is invisible to a scan — never mistaken for installed, but
also never reported as broken.

### B. is it `scoop.cmd` eating the exit code, or scoop itself?

```
scoop.cmd contents:
  @rem C:\Users\kln\scoop\apps\scoop\current\bin\scoop.ps1
  @echo off
  where /q pwsh.exe
  if %errorlevel% equ 0 (
      pwsh -noprofile -ex unrestricted -file "C:\Users\kln\scoop\apps\scoop\current\bin\scoop.ps1"  %*
  ) else (
      powershell -noprofile -ex unrestricted -file "C:\Users\kln\scoop\apps\scoop\current\bin\scoop.ps1"  %*
  )
```

Three ways of invoking the same failing `download` (bad-hash manifest) were
compared:

| Invocation | Exit code |
|---|---|
| `scoop.cmd download <badhash manifest>` (what dotpkg does today) | 0 |
| `powershell.exe -File scoop.ps1 download <badhash manifest>` | 0 |
| `powershell.exe -Command "& scoop.ps1 download <badhash manifest>; Write-Host ('LASTEXITCODE=' + $LASTEXITCODE)"` | 0, and the script printed `LASTEXITCODE=0` explicitly |

All three end with the same stdout tail:

```
Expected:    ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff
Actual:      b688ecafa2d1fdb0af3383f25d6d122866c13ad7cc996e9f735bf90e6c75f83f
ERROR
Please try again or create a new issue by using the following link and paste your console output:
https:////
'badhash' (0.74.1) was downloaded successfully!
```

**It is not the `.cmd` shim.** `scoop.ps1` itself, invoked directly by
PowerShell with no `.cmd` wrapper in the path at all, sets `$LASTEXITCODE` to
`0` after a hash failure. The behaviour is scoop's, not the shim's.

### C. does ANY scoop failure produce a non-zero exit? A sweep

```
exit=0   <- scoop download
exit=0   <- scoop install
exit=1   <- scoop thisisnotacommand
exit=0   <- scoop bucket rm definitely-not-a-bucket-9f2a
exit=0   <- scoop download nonexistent-app-9f2a
exit=0   <- scoop install nonexistent-app-9f2a
```

Six invocations. The **only** non-zero result in this entire measurement
effort — across all four files, every round — is an unrecognised subcommand.
Scoop reserves exit 1 for "I do not know that command", never for "what you
asked me to do failed".

### D. cleanup verification

```
removed C:\Users\kln\AppData\Local\Temp\dotpkg-probe-root -> still exists = False
removed C:\Users\kln\AppData\Local\Temp\dotpkg-probe-stage -> still exists = False
removed C:\Users\kln\AppData\Local\Temp\probe-stdout.txt -> still exists = False
removed C:\Users\kln\AppData\Local\Temp\probe-stderr.txt -> still exists = False
```

All four throwaway paths confirmed gone, not merely assumed gone.

### E. final: the real machine

```
real app dir count = 31
real cache count   = 75
kanata -> kanata_windows_tty_winIOv2_arm64:7868
explorer -> 9620
SCOOP env in this session = ''
```

Identical to the `m1` baseline. `SCOOP` was confirmed unset in the
measurement session itself, closing off "did a stray environment variable
point some command at the real root" as a lingering doubt.

## Falsified: `depends`

**Zero packages, in either survey, declare `depends`.** `m1-out.txt`:

```
=== M7: which installed manifests declare 'depends' ===
count with depends = 0

=== M7b: 'depends' across bucket HEAD for declared packages ===
pkg.toml exists = True
declared scoop packages = 25 : git,nodejs,gh,bat,ripgrep,fzf,fastfetch,neovim,tree-sitter,lazygit,lazydocker,yazi,zellij,opencode,shfmt,yamlfmt,stylua,actionlint,kanata,beckon,python,go,rustup,uv,age
bucket-HEAD manifests declaring depends = 0
```

0 of 30 installed manifests, and 0 of the 25 bucket-HEAD manifests for every
package `pkg.toml` declares. This is a **falsified concern**, not a
confirmed non-issue smoothed into silence: the hazard this project worried
about — a pinned manifest pulling a dependency at latest, over the network,
inside the mutation window — is not live on this machine, on this bucket, on
this day. It could still exist elsewhere; nothing here proves `depends`
support is unnecessary in general, only that it was never exercised in any
measurement this project has run. `docs/phase2b1-prepare-design.md`'s
dogfood and `docs/specs/2026-08-08-phase2b2-executor-design.md` both record
falsified predictions the same way, by name, rather than dropping them; this
is the third.

## Corrections to `docs/phase2b-notes.md`

`docs/phase2b-notes.md`'s "Measured: how a version change actually happens"
table records `scoop install <path>/app.json` (installed, different version)
as **"exit 0, no output, nothing changes"**. Measured here (E5, and M2b for
the same-version case), that is wrong in one detail:

- There **is** output: `WARN  '<app>' (<version>) is already installed.` /
  `Use 'scoop update <app>' to install a new version.`, on **stdout**.
- Installing a manifest for a **different** version than what is installed
  prints the **same** `WARN` line, and it names the version **already
  installed** (0.74.1), not the version just requested (0.74.2).

"Nothing changes" holds in both cases — confirmed on disk, not just inferred
from the exit code. Only "no output" was wrong.
