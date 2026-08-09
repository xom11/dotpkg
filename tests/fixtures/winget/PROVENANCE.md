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
