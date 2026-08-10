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

## 2026-08-10: thirteen write-path fixtures, checked in as part of Phase 4b's Task 1

`docs/measurements-2026-08-10-winget-write-path.md` §§1–9 records, in prose,
the write path's stdout — the first time this crate measured
`install`/`uninstall`/`upgrade` rather than only `list`/`show`. That prose is
a curated excerpt of a longer recording session, not the recording itself:
the raw per-probe capture for every W1 (`list -e --id <package>` write-verb
failure paths) and W2 (write-path positive controls, one guinea pig,
`ducaale.xh`) invocation survived on the machine that ran the session, as
`Start-Process -RedirectStandardOutput` wrote it — CRLF, unedited, one file
per probe. These thirteen fixtures are copies of those files (`cp`, not
retyped), not transcriptions of the doc's prose. An initial pass at this task
*did* transcribe from the doc's prose for six of these files, producing
content that was either an outright reconstruction (`install-upgraded.txt`,
built by hand-substituting a version number into a sibling's text),
substituted a placeholder this project uses elsewhere
(`install-version-absent.txt`, `uninstall-version-absent.txt`), or came from
the right shape but the wrong probe (`list-single-with-available.txt`, real
bytes but from this task's own `--scope` probe rather than the write-path
round). All four are replaced below with the actual captured bytes; the
caveats that described them as reconstructed no longer apply to anything in
this directory and are removed rather than left to describe files they no
longer describe.

| fixture | source file | argv | exit | bytes |
|---|---|---|---|---|
| `install-version-fresh.txt` | `w2/01-stdout.txt` | `install -e --id ducaale.xh --version 0.24.1 --disable-interactivity --accept-source-agreements --accept-package-agreements --silent` | `0` | 499 |
| `install-already-installed-no-upgrade.txt` | `w2/02-stdout.txt` | `install -e --id ducaale.xh --version 0.24.1 --disable-interactivity --accept-source-agreements --accept-package-agreements --silent` (run again, same version, already installed) | `0x8A15002B` | 188 |
| `install-upgraded.txt` | `w2/03-stdout.txt` | `install -e --id ducaale.xh --version 0.26.1 --disable-interactivity --accept-source-agreements --accept-package-agreements --silent` (0.24.1 installed, asked for 0.26.1: a real upgrade, not a template) | `0` | 588 |
| `install-package-absent.txt` | `w1/01-stdout.txt` | `install -e --id Xyzzy.NoSuch.Dotpkg --disable-interactivity --accept-source-agreements` | `0x8A150014` | 43 |
| `install-version-absent.txt` | `w1/05-stdout.txt` | `install -e --id sharkdp.hyperfine --version 0.0.0-dotpkg-w1-does-not-exist --disable-interactivity --accept-source-agreements` | `0x8A150017` | 59 |
| `install-no-upgrade-available.txt` | `w2/05-stdout.txt` | `install -e --id ducaale.xh --no-upgrade --disable-interactivity --accept-source-agreements --accept-package-agreements --silent` | `0x8A150061` | 65 |
| `uninstall-refused-elevated.txt` | `w2/12-stdout.txt` | `uninstall -e --id ducaale.xh --disable-interactivity --accept-source-agreements` (elevated) | `0x8A15007D` | 127 |
| `uninstall-success.txt` | `w2/inner-stdout.txt` | `uninstall -e --id ducaale.xh --disable-interactivity --accept-source-agreements` (de-elevated: `runas /trustlevel:0x20000`, inner session `elevated = False`) | `0` | 80 |
| `uninstall-package-absent.txt` | `w1/03-stdout.txt` | `uninstall -e --id Xyzzy.NoSuch.Dotpkg --disable-interactivity --accept-source-agreements` | `0x8A150014` | 53 |
| `uninstall-version-absent.txt` | `w2/10-stdout.txt` | `uninstall -e --id ducaale.xh --version 0.0.0-dotpkg-w2-does-not-exist --disable-interactivity --accept-source-agreements` | `0x8A150017` | 59 |
| `upgrade-nothing-available.txt` | `w2/07-stdout.txt` | `upgrade -e --id ducaale.xh --disable-interactivity --accept-source-agreements --accept-package-agreements --silent` (run again, already at the newest version) | `0x8A15002B` | 99 |
| `list-single-with-available.txt` | `w2/verify-01.txt` | `list -e --id ducaale.xh --disable-interactivity`, the verify call taken immediately after W2 step 01 (`install --version 0.24.1`) | `0` | 126 |
| `list-single-ahead-of-pin.txt` | `w2/verify-07.txt` | `list -e --id ducaale.xh --disable-interactivity`, the verify call taken immediately after W2 step 07 (`upgrade`, already newest — package sits at `0.26.2`) | `0` | 97 |

Two things worth naming about the last two rows, because a later task's
assertions depend on them:

- `list-single-with-available.txt` is **`ducaale.xh` at `0.24.1` with
  `Available` `0.26.2`** — the exact row `docs/measurements-2026-08-10
  -winget-write-path.md` §1's second command paraphrases as `EXIT 0 version
  0.24.1 available 0.26.2`, now present as the raw table rather than a
  paraphrase.
- `list-single-ahead-of-pin.txt` is **`ducaale.xh` at `0.26.2` with no
  `Available` column at all** — winget drops the column once nothing is
  upgradable (the same behaviour `PROVENANCE.md`'s `list-single.txt`
  and `list-duplicate-id.txt` entries above already document for other
  packages), and that absence is itself the signal, not an omission.

`w1-transcript.txt` and `w2-transcript.txt` (kept outside this repo, in the
session's own scratch directory, not committed — same status as
`scratch/w3-scope.ps1`) number every probe and record its argv/exit/wall time
against the same bytes these fixtures now hold; the argv column above is
copied from those transcripts, not reconstructed from the fixture content.
The one exception is `verify-01`/`verify-07`: those two files have no
transcript line of their own (the transcript numbers the 15 write steps, not
the verify call after each one), so their argv and exit code are inferred
from content and position — a `list -e --id ducaale.xh` table with no error
text is only ever exit `0` anywhere else in this dataset, and their byte
counts (126, then dropping to 97 once the `Available` column disappears
starting at verify-06) track step 01's install and step 06/07's convergence
to the newest version exactly.
