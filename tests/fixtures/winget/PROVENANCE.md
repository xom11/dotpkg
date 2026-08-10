# winget fixtures — where these bytes came from

Captured 2026-08-09 on a14, **raw stdout redirected straight to a file** by
`Start-Process -RedirectStandardOutput`. No `Out-String`, no re-encoding, no
reflow: these are the bytes `winget.exe` wrote.

```
winget       v1.29.280      Windows.Desktop v10.0.26200.8973      Arm64
culture      en-US          stdout redirected (Console.IsOutputRedirected = True)
```

**Line endings are CRLF and must stay CRLF.** `list-full.txt` has 143 `\r\n`
pairs. A parser tested only against `\n` passes here and fails on the one
platform this tool runs on. Do not let an editor or a `.gitattributes` rule
normalise this directory.

| file | argv (after `winget`) | exit | bytes | what it pins |
|---|---|---|---|---|
| `list-full.txt` | `list --disable-interactivity` | `0` | 30958 | 141 rows, 126 distinct ids, **8 duplicated**, **84 with no Source**, max line 218, columns at `Name(0) Id(64) Version(152) Available(182) Source(212)` |
| `list-duplicate-id.txt` | `list -e --id 7zip.7zip …` | `0` | 221 | one id, **two rows, two different versions**; **no `Available` column** |
| `list-single.txt` | `list -e --id ajeetdsouza.zoxide …` | `0` | 127 | the ordinary one-row case; no `Available` column |
| `list-greater-prefix.txt` | `list -e --id Microsoft.VisualStudio.2022.BuildTools …` | `0` | 268 | **one** row whose version is `> 17.14.37` |
| `list-upgrade-available.txt` | `list --upgrade-available …` | `0` | 1373 | the `Available` column present, **and a second table** under "require explicit targeting for upgrade", and a `9 upgrades available.` count that does not match the first table's 8 rows |
| `list-not-found.txt` | `list -e --id Xyzzy.NoSuch.Dotpkg …` | **`-1978335212`** | 53 | `No installed package found matching input criteria.` |
| `list-source-filter-empty.txt` | `list -s msstore …` | **`0`** | 53 | **byte-identical to the row above it, and a different exit code.** The exit code is a function of the filter, not of the output |
| `show-git.txt` | `show -e --id Git.Git …` | `0` | 1550 | `Found Git [Git.Git]`, `Version: 2.55.0.3`, and an `Installer SHA256:` |
| `show-canonical-echo.txt` | `show --id git.git …` (**no** `-e`) | `0` | 1550 | the same 1550 bytes as above from a **lowercased** id: dropping `--exact` folds case and echoes the canonical id in `Found … [Git.Git]` |
| `show-versions-zoxide.txt` | `show -e --id ajeetdsouza.zoxide --versions …` | `0` | 131 | **11** versions, `0.10.0` … `0.9.0` |
| `show-versions-ripgrep.txt` | `show -e --id BurntSushi.ripgrep.MSVC --versions …` | `0` | 128 | **8** versions — the shallowest retention measured |
| `show-old-version.txt` | `show -e --id ajeetdsouza.zoxide -v 0.9.0 …` | `0` | 1024 | a 2023-01-08 release recovered complete, **with its SHA256** |
| `show-version-gone.txt` | `show -e --id ajeetdsouza.zoxide -v 0.8.0 …` | **`-1978335209`** | 34 | `No version found matching: 0.8.0` |
| `show-package-gone.txt` | `show -e --id Xyzzy.NoSuch.Dotpkg …` | **`-1978335212`** | 43 | `No package found matching input criteria.` — a different code from the row above |
| `export-versions.json` | `export -o <path> --include-versions …` | `0` | 4420 | **42** `PackageIdentifier` entries against `list-full.txt`'s 57 source-backed rows: export collapses duplicates silently and drops all 84 sourceless rows. Keeps `26.02` for `7zip.7zip`, and **preserves the `> ` prefix** |

**stderr was 0 bytes for every one of the 15 captures, including all three
failures.** There is no stderr fixture because there is nothing to capture.

## How `list-full.txt` splits, computed from the file

141 rows collapse to 126 distinct ids, which split **89 opaque / 37 installed**
(89 + 37 = 126 is the cross-check):

| opaque because | ids | which |
|---|---|---|
| no `Source` | 84 | every `MSIX\…` and `ARP\…` row |
| version starts `> ` | 2 | `Microsoft.VisualStudio.2022.BuildTools`, `Microsoft.WindowsAppRuntime.1.8` |
| duplicate rows disagree on a version | 3 | `7zip.7zip` (`26.01.00.0`/`26.02`), `Microsoft.UI.Xaml.2.8` (`8.2511.26001.0`/`8.2501.31001.0`), `Microsoft.WindowsAppRuntime.2` (`2.3.1.0`/`2.2.0.0`) |

Of the 37 installed entries, **4 were collapsed from duplicate rows that agreed
on a version**: `Microsoft.DotNet.Native.Runtime`,
`Microsoft.VCLibs.Desktop.14`, `Microsoft.VCLibs.14`,
`Microsoft.WindowsAppRuntime.1.7`.

These numbers are what `tests/winget_scan.rs` asserts. **If code disagrees with
them, this file is right** — recompute from the fixture rather than adjusting an
assertion.

`export-versions.json` is the JSON file `-o` wrote, **not** the command's
stdout. The command also printed 7358 bytes of
`Installed package is not available from any source: <Name>` to stdout and
still exited `0`; that stream is recorded in
`docs/measurements-2026-08-09-winget.md` rather than kept as a fixture.

Two rows changed between the measurement rounds and this capture, both
explained in that document's section 9: `winget source update` added
`Windows Package Manager Source (winget-font) V2`, so `list-full.txt` has 141
rows where the measurement text says 140, 126 distinct ids where it says 125,
and 84 sourceless rows where it says 83. The eight duplicated ids are the same
eight.

Full record and method: [`docs/measurements-2026-08-09-winget.md`](../../../docs/measurements-2026-08-09-winget.md).

## 2026-08-10: the machine has drifted, and the fixtures above are no longer "numerically identical" to it

The "numerically identical" claim two paragraphs above was true when this file
was written (2026-08-09) and **is not true any more**. A live re-parse of
`winget list --disable-interactivity` on a14, done as part of Phase 4b's Task
1 (`docs/measurements-2026-08-10-winget-write-path.md` §15), gives:

| | this file's fixtures (`list-full.txt`, 2026-08-09) | a14, 2026-08-10 |
|---|---|---|
| rows | 141 | **140** |
| distinct ids | 126 | **125** |
| `installed` after `rows_to_scan` | 37 | **36** |
| `opaque` | 89 | 89 |

`wez.wezterm` has been uninstalled (accounting for the row/id/installed drop
by exactly one each); `tailscale.tailscale` moved `1.98.2` → `1.102.2`;
winget's own source MSIX row rotated. `opaque` is unchanged at 89 because the
package that left was itself in the `installed` bucket, not the `opaque` one.

**A Phase 4b dogfood must re-derive the machine's numbers from a fresh `winget
list`, not reuse 141/126/37/89 (or the export/download counts quoted earlier
in this file) as expected values** — those are frozen properties of the
fixture *files*, not standing claims about the machine that produced them.
`docs/measurements-2026-08-10-winget-write-path.md` §14 first recorded this
drift; §15 independently re-derived the same 140/125/36/89 by parsing a fresh
`winget list` capture with the same header-name-keyed column logic
`src/backend/winget.rs` uses, rather than trusting §14's numbers unchecked.

## 2026-08-10: twelve write-path fixtures, checked in as part of Phase 4b's Task 1

`docs/measurements-2026-08-10-winget-write-path.md` §§1–9 record the write
path's stdout — the first time this crate measured `install`/`uninstall`/
`upgrade` rather than only `list`/`show`. These twelve fixtures turn that
prose into files `tests/*.rs` can read, with the same CRLF convention as every
fixture above (`.gitattributes` pins this whole directory `-text`).

**Confidence varies per fixture, and is recorded honestly below rather than
presented as uniform.** The measurements doc itself is not uniformly literal:
`install-version-fresh.txt`'s source block states `stdout 499 bytes` but only
quotes ~251 bytes of it (the `Downloading .../...` line elides the real URL,
and the gap suggests a progress indicator was never transcribed either) — so
even the doc's most detailed block is a curated excerpt, not a byte-for-byte
capture. Fixtures below are marked **verbatim** (the doc's own text, used
as-is), **reconstructed** (the doc gives the shape and the varying value, not
the bytes; templated from a verbatim sibling), or **substituted** (the doc
elides one value as a placeholder such as `<absent>`; a value this project has
already used for exactly this purpose elsewhere is reused rather than a new
one invented).

| fixture | argv | exit | source | confidence |
|---|---|---|---|---|
| `install-version-fresh.txt` | `install -e --id ducaale.xh --version 0.24.1 --silent --accept-package-agreements --accept-source-agreements --disable-interactivity` | `0` | doc §1's quoted block, verbatim including its own `...` URL elision | **verbatim** (doc's own excerpt is itself curated — see above) |
| `install-already-installed-no-upgrade.txt` | `install -e --id ducaale.xh --version 0.24.1 --disable-interactivity` | `0x8A15002B` | lines 1–2 verbatim-quoted twice in the doc (Headline, and §2's prose); line 3 (`No newer package versions are available from the configured sources.`) supplied by `task-1-brief.md` directly, not independently present in the committed doc | **verbatim** (line 3 trusted from the brief, per its own "exact values to use verbatim" instruction) |
| `install-upgraded.txt` | `install -e --id ducaale.xh --version 0.26.1 --disable-interactivity` | `0` | §2 row 2 records only the exit code and "full download + install"; no bytes were preserved for this specific call | **reconstructed** — §1's verbatim template with `0.24.1` replaced by `0.26.1` in both the version line and the download filename |
| `install-package-absent.txt` | `install -e --id Xyzzy.NoSuch.Dotpkg` | `0x8A150014` | §4's table, byte count (43) independently confirms the single line plus one CRLF terminator, nothing more | **verbatim**, byte-count cross-checked |
| `install-version-absent.txt` | `install -e --id sharkdp.hyperfine --version 99.99.99` | `0x8A150017` | §6 gives the message template (`No version found matching: …`) against `sharkdp.hyperfine` but elides the actual absent-version string as `<absent>` | **substituted** — `99.99.99` is not invented for this task; it is the same value `docs/measurements-2026-08-09-winget.md`'s `show -e --id ajeetdsouza.zoxide -v 99.99.99` already used for an identical "absent version" probe |
| `install-no-upgrade-available.txt` | `install -e --id ducaale.xh --no-upgrade --disable-interactivity` | `0x8A150061` | §9's terse log, the only line recorded for this call | **verbatim** of what's recorded; §9 states no byte count, so completeness (unlike §4's rows) cannot be cross-checked |
| `uninstall-refused-elevated.txt` | `uninstall -e --id ducaale.xh --disable-interactivity` (elevated) | `0x8A15007D` | §5's first quoted block, verbatim; matches `task-1-brief.md`'s own example exactly | **verbatim** |
| `uninstall-success.txt` | `uninstall -e --id ducaale.xh --disable-interactivity` (de-elevated) | `0` | §5's paired positive-control block, verbatim | **verbatim** |
| `uninstall-package-absent.txt` | `uninstall -e --id Xyzzy.NoSuch.Dotpkg` | `0x8A150014` | §4's table, byte count (53) independently confirms the single line plus one CRLF terminator | **verbatim**, byte-count cross-checked |
| `uninstall-version-absent.txt` | `uninstall -e --id ducaale.xh --version 99.99.99` | `0x8A150017` | §8's table has **no stdout column at all** — only exit codes and outcome | **substituted** — the message is inferred from the same `No version found matching: <version>` template §6 and `docs/measurements-2026-08-09-winget.md` establish for `install`/`show`; `uninstall`'s own wording was never captured. `99.99.99` reused for the same reason as `install-version-absent.txt` |
| `upgrade-nothing-available.txt` | `upgrade -e --id ducaale.xh --disable-interactivity` | `0x8A15002B` | the Headline section quotes `"No available upgrade found."` as winget's literal words for this exit code; §9's terse log confirms the same code for `upgrade (already newest)` | **verbatim** phrase, but its attachment to the `upgrade` verb specifically (rather than `install`) is inferred from the shared exit code, not an independent per-verb quote |
| `list-single-with-available.txt` | `list -e --id Discord.Discord --disable-interactivity` | `0` | **not** from `docs/measurements-2026-08-10-winget-write-path.md`. That doc's own equivalent (§1's second command, `ducaale.xh` with an `Available` column) was paraphrased as `EXIT 0 version 0.24.1 available 0.26.2`, not preserved as a raw table, and `ducaale.xh` no longer exists on a14 to safely (read-only) recapture. Sourced instead from this same task's live `--scope` probe (`docs/measurements-2026-08-10-winget-write-path.md` §15), captured the same way every other fixture in this directory was — raw stdout via `Start-Process -RedirectStandardOutput` | **verbatim**, real bytes, different round than §§1-9 |

`install-version-absent.txt` and `uninstall-version-absent.txt` are
byte-identical (`No version found matching: 99.99.99`) because both were built
from the same substitution against the same template; nothing measured
distinguishes winget's wording between the two verbs for this error.
