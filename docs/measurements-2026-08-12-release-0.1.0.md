# Measurements: what was actually verified before 0.1.0 was tagged

Taken 2026-08-12 against `main` at **`cbacdcd`**, on GitHub's runners and on a14
(`zenbook-a14`).

This is short on purpose. It records one thing: **whether the binary a user
would download has ever been started**, and by what.

## 1. The workflow was proved before a tag existed

`.github/workflows/release.yml` carries two triggers. A tag publishes;
`workflow_dispatch` builds the same artifacts and stops. The `publish` job is
gated on `startsWith(github.ref, 'refs/tags/v')`.

**Observed rather than assumed:** the dispatch run reports
`build (x86_64-pc-windows-msvc) success`,
`build (aarch64-pc-windows-msvc) success`, and **`publish` skipped**. So the
dry-run path does not publish, which is the whole reason for having two
triggers -- a tag is hard to take back, and a release whose binary nobody has
started is a claim rather than a build.

**One thing that was uncertain and is now measured:** `aarch64-pc-windows-msvc`
cross-compiles from the x64 `windows-latest` runner without extra setup beyond
`targets:` on the toolchain action. That was not obvious -- it needs the ARM64
MSVC libraries to be present on the image -- and it was going to be found out
either here or by a failed tag.

## 2. The checksums agree in three places

| | `dotpkg-aarch64-pc-windows-msvc.exe` | `dotpkg-x86_64-pc-windows-msvc.exe` |
|---|---|---|
| computed by CI, in the build job | `f00d78f2…` | `951456b4…` |
| recomputed on the developer machine after download | `f00d78f2…` | `951456b4…` |
| recomputed on a14 before running it | `f00d78f2…` | not run -- see §4 |

Three independent computations of the same bytes, which is what makes the
`SHA256SUMS` file beside the release mean something.

## 3. The aarch64 binary starts, and works, on real ARM64 hardware

**This is the leg CI structurally cannot do.** An x64 runner cannot execute
what it cross-compiled, so until this run nothing had ever started that
artifact.

Run on a14 from the downloaded bytes, read-only:

| | |
|---|---|
| `--version` | **`dotpkg 0.1.0`** |
| `--help` | lists **4** commands: `status`, `apply`, `update`, `adopt` |
| `status` against a config declaring nothing | **exit path reached, real output** |

The `status` run reports `? scoop 24 installed outside dotpkg`,
`? winget 36 installed outside dotpkg`, `0 change(s), 0 skipped, 60 unmanaged`.

**The 36 is a cross-check rather than a coincidence.** It is the same figure
`docs/measurements-2026-08-12-phase7-fence-coverage.md` §2.1 reconciled against
`winget export`'s 41, produced here by a *different binary* -- a release build,
cross-compiled by CI on a different machine -- reading the same machine.

kanata was not signalled; its pid was **3976** before and after, recorded as a
landmark.

## 4. What is still unverified, and it is not small

- **The x86_64 binary has never run on real hardware.** It ran on the
  `windows-latest` runner, which executed its own `--version` in the build job,
  and that is the whole of it. No x64 Windows machine has ever run dotpkg at
  all -- which the README's "Verified on" table says in as many words.
- **`apply` was not exercised from the release binary.** Only `status`, which
  mutates nothing. Every mutating path's evidence comes from the phase rounds
  and their own trees, not from this artifact.
- **One winget version, one scoop layout, one machine**, unchanged from what
  the README states.

## 5. Method failure, and it is the third instance of one habit

I wrote *"no backtick appears anywhere in this file, including in comments"*
into a PowerShell script and then put a backtick in it -- for the **third**
time in two days, every time in a comment, every time quoting a command name
the way one would in markdown.

`docs/measurements-2026-08-12-phase7-fence-coverage.md` §10 says "two". It was
true when written and this is the edit that falsifies it, so it is corrected
there rather than left to be discovered.

The gate added this round covers `scripts/*.ps1` and caught the repository's
own instance. It cannot reach a scratchpad script, and all three of mine were
scratchpad scripts -- so what stopped them from mattering was running an ad-hoc
checker before every upload, which is a habit and not a gate.
