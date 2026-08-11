use super::{Backend, ResolveCtx, Scan};
use crate::lock::Pin;
use crate::model::{Installed, Name, WINGET};
use crate::update::Resolution;
use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::process::{Command, Stdio};

/// One row of `winget list`'s fixed-width text table.
///
/// This is a row, not yet a fact about the machine: `available` and `source`
/// are exactly what winget printed (or omitted). Task 10 is what decides
/// which rows are `Installed` and which are `opaque`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WingetRow {
    pub name: String,
    pub id: String,
    pub version: String,
    pub available: Option<String>,
    pub source: Option<String>,
}

/// Header labels in the order winget prints them.
///
/// `Available` is absent from the header whenever no row in the table has an
/// upgrade -- measured on `list-duplicate-id.txt` and `list-single.txt`,
/// neither of which has that column at all. A layout keyed on column *count*
/// instead of these *names* would, on those files, read `Source` out of the
/// slot where `Available` would have been and report every package as
/// sourceless.
const COLUMN_NAMES: [&str; 5] = ["Name", "Id", "Version", "Available", "Source"];

/// Step back to the nearest UTF-8 char boundary at or before `idx`.
///
/// Column offsets come from `find`ing ASCII header labels, but the *data*
/// rows below that header are not guaranteed ASCII (a package name can carry
/// an accented character even under the en-US locale winget was measured
/// in). Slicing a `str` on a non-boundary byte offset panics; this makes that
/// impossible, at the cost of a field occasionally swallowing or exposing one
/// extra byte of a multi-byte character it split through the middle of --
/// better than aborting the whole scan over one package's name.
fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
    if idx > s.len() {
        idx = s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Parse the table `winget list` (with any filter/flag combination) prints to
/// stdout.
///
/// winget has no machine-readable `list` output. Its only JSON path,
/// `winget export`, was measured and rejected (see
/// `tests/fixtures/winget/PROVENANCE.md`): it silently collapsed 57
/// source-backed rows to 42 entries and dropped all 84 rows that have no
/// `Source`, naming those by *Name* rather than *Id*. `list` strictly
/// dominates it in information content, so this parses that fixed-width
/// table directly -- keyed on the header's column *names*, never on column
/// count (data-dependent) and never on hardcoded offsets (winget recomputes
/// them per invocation: `list-full.txt` puts `Source` at byte 212,
/// `list-upgrade-available.txt` puts it at byte 102).
pub fn parse_list(stdout: &str) -> Result<Vec<WingetRow>> {
    // winget's own line ending is CRLF (measured on every one of the 15
    // captured fixtures; `.gitattributes` pins the fixture bytes so nothing
    // normalises them in this repo). Splitting on '\n' alone and leaving the
    // '\r' attached glues it onto whichever column reaches the end of the
    // line -- see this file's sibling test that forgets to do this strip.
    let lines: Vec<&str> = stdout
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();

    let header_idx = lines
        .iter()
        .position(|line| line.starts_with("Name") && line.contains(" Id"))
        .ok_or_else(|| {
            let head: String = stdout.chars().take(120).collect();
            anyhow::anyhow!("winget list produced no header row: {head:?}")
        })?;
    let header = lines[header_idx];

    // Column starts: `find` on the header, searched left to right from the
    // end of the *previous column's label text* -- never a hardcoded offset.
    // A name absent from this particular header (most commonly `Available`)
    // is simply absent from `layout`.
    let mut layout: Vec<(&'static str, usize)> = Vec::new();
    let mut cursor = 0usize;
    for &name in COLUMN_NAMES.iter() {
        if cursor > header.len() {
            break;
        }
        if let Some(rel) = header[cursor..].find(name) {
            let start = cursor + rel;
            cursor = start + name.len();
            layout.push((name, start));
        }
    }

    // The header is English and therefore locale-dependent (a14 is en-US;
    // no other locale was measured). An unrecognised header must refuse
    // rather than guess offsets: guessed offsets would report an empty
    // machine, and an empty machine is what `mass_prune_guard` exists to
    // catch far too late.
    for required in ["Name", "Id", "Version"] {
        if !layout.iter().any(|&(name, _)| name == required) {
            bail!("winget list header is missing a {required} column: {header:?}");
        }
    }

    let has_available = layout.iter().any(|&(name, _)| name == "Available");
    let has_source = layout.iter().any(|&(name, _)| name == "Source");

    // `line[start..next_start]`, trimmed; empty when the line does not reach
    // `start` at all (the common case for a sourceless row: winget pads the
    // header but not every data line out to the last column).
    let field = |line: &str, name: &str| -> String {
        let idx = layout
            .iter()
            .position(|&(n, _)| n == name)
            .expect("caller checks the column is present before asking for it");
        let start = layout[idx].1;
        if start >= line.len() {
            return String::new();
        }
        let end = layout.get(idx + 1).map(|&(_, s)| s).unwrap_or(line.len());
        let end = floor_char_boundary(line, end.min(line.len()));
        let start = floor_char_boundary(line, start);
        line[start..end].trim().to_string()
    };

    let mut rows = Vec::new();
    for line in &lines[header_idx + 1..] {
        if line.trim().is_empty() {
            // A genuinely blank line: the one between the header block and
            // the table's first row never occurs in the fixtures, but the
            // trailing blank line every fixture ends on does.
            continue;
        }
        if line.chars().all(|c| c == '-') {
            // The `---` rule line printed directly under the header.
            continue;
        }

        let name = field(line, "Name");
        let id = field(line, "Id");
        let version = field(line, "Version");
        if name.is_empty() || id.is_empty() || version.is_empty() {
            // Not a table row. `list-upgrade-available.txt` prints
            // "9 upgrades available." after its first table, then a second
            // table under a different heading -- that count line disagrees
            // with the first table's 8 rows by winget's own design (it is
            // 9 counting a package the first table excludes because it needs
            // explicit targeting), not a parse error. Stop here; the second
            // table is not this call's job.
            break;
        }

        let available = has_available
            .then(|| field(line, "Available"))
            .filter(|s| !s.is_empty());
        let source = has_source
            .then(|| field(line, "Source"))
            .filter(|s| !s.is_empty());

        rows.push(WingetRow {
            name,
            id,
            version,
            available,
            source,
        });
    }

    Ok(rows)
}

/// Names a live process might plausibly report for a winget package.
///
/// winget exposes no executable list anywhere a scan can reach -- `winget
/// list` has no such column, and the aliases an install creates are announced
/// only on `install`'s own stdout ("Command line alias added: ..."), at
/// install time. So these are not executable names; they are the two guesses
/// measured to work, and they go into `Installed.bins` because that is the
/// field `Running::covers` consults.
///
/// Measured on a14 against the live process table, over the 36 source-backed
/// installed winget ids: the whole dotted id (`Installed.name.key()`, the
/// only winget signal that exists today) matched **0**; the id's last dotted
/// segment matched **4**; the folded display `Name` matched **2**. Both are
/// returned because they are different signals -- `Google.Chrome` is reached
/// only by the segment (`chrome`), and neither is reached by the id.
///
/// Over-matching is deliberate, per `Running::covers`'s own rule: "A false
/// positive costs one `!` line the user clears by closing an app; a false
/// negative costs the app."
///
/// **Known residual gap, measured:** installing `ducaale.xh` created TWO
/// aliases, `xh` and `xhs`, and `xhs` is neither the id, the display name,
/// nor the last segment of either. A package's second alias is invisible to
/// this, and no scan-time source for it exists.
///
/// That is one case, not the whole class -- a single-alias package is missed
/// too whenever the id's last segment is a build or vendor qualifier
/// (**measured:** `rg` / `BurntSushi.ripgrep.MSVC`). `rows_to_scan`'s doc
/// comment on `bins` carries the corrected framing and what narrows it; it is
/// not repeated here.
pub(crate) fn guard_names(id: &str, display: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let last = id.rsplit('.').next().unwrap_or(id);
    for raw in [last, display] {
        let folded = raw.trim().to_ascii_lowercase();
        if folded.is_empty() || out.contains(&folded) {
            continue;
        }
        out.push(folded);
    }
    out
}

/// The directories winget installs a `portable` package into, one per scope.
///
/// **Measured** (`docs/measurements-2026-08-11-phase5-guard-unmanaged-retry.md`
/// §3): the user-scope root held 5 package directories on a14, and the
/// machine-scope root did not exist at all -- as did neither
/// `%ProgramFiles%\WinGet\Links` nor its `(x86)` sibling. The machine-scope
/// entry below is therefore **reasoned, not measured**: it is where a
/// machine-scope portable would live, and no such install has been observed.
///
/// Returns an empty vector wherever these variables are unset, which is every
/// non-Windows platform. `running_ids` is a no-op on an empty root list, so
/// nothing needs a `cfg`.
///
/// `pub(crate)` is enough: the sole caller is `apply::sample_fence`, inside this
/// library. It was briefly `pub`, for a `main.rs` call site that read these roots
/// directly -- the `sample_fence` hoist removed that site, and the widening with
/// it.
pub(crate) fn package_roots() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        out.push(
            std::path::PathBuf::from(local)
                .join("Microsoft")
                .join("WinGet")
                .join("Packages"),
        );
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        out.push(std::path::PathBuf::from(pf).join("WinGet").join("Packages"));
    }
    out
}

/// Which of `scanned` has a live process running out of its own winget package
/// directory -- the winget analogue of `Scoop::running_apps`, and the signal
/// three documents said could never fire for winget.
///
/// **This is the only signal that would catch a process whose name resembles
/// nothing about its package.** Measured on a14: `kanata`'s process is
/// `kanata_windows_tty_winIOv2_arm64`, and scoop's fence catches it purely
/// because its executable lives under `$SCOOP/apps/kanata/`. `guard_names`
/// would miss it entirely. Nothing gave winget that protection until this
/// function.
///
/// **Coverage is bounded and the bound is measured, not guessed:** winget only
/// creates these directories for `portable` packages -- 4 of 36 installed ids
/// on a14 -- so every EXE/MSI application is invisible to THIS signal and can
/// only ever be caught by the fence's name half.
///
/// That bound is unmoved by Phase 5 Task 4, and what the name half can see is
/// wider because of it: `backend::apply_guard_overrides` merges a declared
/// `[winget.guard]` entry into the matching `Installed.bins`, which
/// `Running::covers` compares against `names`. So a non-portable package the
/// user has named does now reach the fence -- by name, never by path, and only
/// when pkg.toml names it. Nothing about that widens what this function
/// returns.
///
/// **Why a per-id prefix test rather than parsing the directory name.** The
/// segment is `<id>_<sourceIdentifier>` in all 5 measured cases, but splitting
/// on `_` assumes a winget id contains none, which is **unmeasured**, and the
/// failure direction is the dangerous one: a truncated segment matches no
/// installed id, so the fence misses and a running package can be replaced.
/// Testing `scanned` against the segment assumes nothing about winget's naming
/// and can only fail toward "no match".
///
/// The `_` boundary is load-bearing rather than decorative. a14 still carries
/// `PhatMT97.VKey.Classic_...` from an uninstalled package, whose folded
/// segment begins with installed `phatmt97.vkey`; a bare `starts_with` would
/// report a package running that is not installed. A bare `<id>` segment with
/// no suffix is accepted too, which is **reasoned, not measured** -- all 5
/// observed directories carry a suffix.
///
/// **Structural:** `backend::running_set` is the one non-test caller, and it is
/// the only producer of a `Running` outside tests -- so every winget entry that
/// ever reaches the fence's `dirs` half comes from here.
pub(crate) fn running_ids(
    roots: &[std::path::PathBuf],
    procs: &[crate::sys::Process],
    scanned: &[Name],
) -> std::collections::BTreeSet<Name> {
    fn fold(p: &std::path::Path) -> String {
        p.to_string_lossy().replace('\\', "/").to_ascii_lowercase()
    }

    let mut out = std::collections::BTreeSet::new();
    for root in roots {
        // `trim_end_matches` makes a root with or without its own trailing
        // separator equivalent. Without it, a root already ending in `/`
        // would fold to e.g. `c:/root/packages/`, the appended `/` below
        // would double it to `.../packages//`, and `strip_prefix` would then
        // match no real process path -- silently disabling this function for
        // every process under that root, which is the dangerous failure
        // direction this function exists to avoid.
        let prefix = format!("{}/", fold(root).trim_end_matches('/'));
        for p in procs {
            // A process whose path cannot be read is `names`' job, not this
            // function's: 22 of 223 on a14.
            let Some(exe) = p.exe.as_deref() else {
                continue;
            };
            let Some(rest) = fold(exe).strip_prefix(&prefix).map(str::to_string) else {
                continue;
            };
            let Some(seg) = rest.split('/').next().filter(|s| !s.is_empty()) else {
                continue;
            };
            for id in scanned {
                let key = id.key();
                let hit = seg == key
                    || seg
                        .strip_prefix(key)
                        .is_some_and(|tail| tail.starts_with('_'));
                if hit {
                    out.insert(id.clone());
                }
            }
        }
    }
    out
}

/// Turn `parse_list`'s rows into a `Scan`: one fact per id, or an admission
/// that no fact could be established.
///
/// Every row reaching this function already has a non-empty `Name`, `Id` and
/// `Version` -- `parse_list` stops the table the moment one of those three
/// goes missing (see its own doc comment). That is correct against all 15
/// captured fixtures, but it means a genuine row with an empty `Id` or
/// `Version` would already be gone by the time it could get here, silently
/// truncating the rest of the table rather than surfacing as one bad row.
/// Nothing below this line can detect that; it is `parse_list`'s assumption,
/// recorded here because this is where the consequence would show up.
///
/// Rows are grouped by `Name` first, because one id can appear more than
/// once -- measured on `list-full.txt`, two, three or even four times for
/// one id. A group becomes `opaque` instead of an `Installed` for any of
/// three reasons, checked in this order:
///
///   1. **any row has no `Source`** -- measured 84 of 141 rows on a14, every
///      `MSIX\...` and `ARP\...` entry. Installed, but comparable against
///      nothing: there is no index to check it against. This is by far the
///      most common reason (84 of 126 ids on the captured machine), so it is
///      **not** warned about per id -- `src/main.rs` prints every warning
///      unconditionally on every run, and 84 lines for the ordinary shape of
///      a winget machine is exactly the false-positive flood that gets a
///      feature silenced and never turned back on (the same argument
///      `plan::SCOOP_HELPERS` was built on). Instead, one aggregate warning
///      after the loop below names the count.
///   2. **any row's version starts with `"> "`** -- measured on two ids
///      (`Microsoft.VisualStudio.2022.BuildTools`,
///      `Microsoft.WindowsAppRuntime.1.8`). This is winget saying *at
///      least*, for an install whose exact version it could not determine --
///      not "several versions installed": `list -e --id` returns exactly one
///      row for each. Left in `installed`, `cur.version == want` would be
///      `"> 17.14.37" == "17.14.37"`, false forever, and `plan::is_older`
///      splits on non-digits so both sides reduce to the same digit sequence
///      and the remaining arm is `Downgrade` -- a false `↓` on every `status`
///      run, and something `apply --yes` would act on. At two ids out of 126
///      a per-id warning costs nothing, and this is the one bucket where a
///      package the user actually declared can land in `opaque` -- so unlike
///      reason 1, this one is warned about individually, naming the id and
///      what winget reported.
///   3. **the group's rows disagree on version** -- measured on three ids
///      (`7zip.7zip`, `Microsoft.UI.Xaml.2.8`, `Microsoft.WindowsAppRuntime.2`).
///      Two genuinely different installed versions of one id is a state this
///      crate has no vocabulary for; picking one would be inventing a fact.
///      `winget export` does exactly that -- silently keeping the greatest --
///      and `PROVENANCE.md` is where that was measured and rejected as this
///      crate's own behaviour.
///
/// A group that clears all three becomes one `Installed`. If it had more
/// than one row -- measured on four ids, all agreeing on version -- a
/// warning records the collapse, because staying silent about it (as
/// `winget export` does for every duplicate, agreeing or not) is the exact
/// behaviour reason 3 above declines to copy.
///
/// Every `opaque` push here is paired with a warning that explains it --
/// either a per-id one (reasons 2 and 3) or the one aggregate warning
/// covering every reason-1 id -- because `render.rs` prints an opaque skip
/// as "installed, but its state could not be read -- see the warnings
/// above". An unpaired `opaque` push would make that sentence a promise this
/// function did not keep.
///
/// `arch` and `bucket` are always `None`: winget exposes neither. `bins` is
/// filled by `guard_names` (see its own doc comment) from `group[0]`'s
/// display `id` and `name` -- not a manifest, because winget has none, but
/// the two guesses measured to catch a live process.
///
/// **The residual gap is wider than this comment used to claim.** It said the
/// missed case was a package's *second* alias (`ducaale.xh`, whose install
/// created both `xh` and `xhs`). That example is real, but the framing was too
/// narrow: **measured**
/// (`docs/measurements-2026-08-11-phase5-guard-unmanaged-retry.md` §3) `rg` is
/// ripgrep's *only* command and `guard_names` misses it too, because
/// `BurntSushi.ripgrep.MSVC`'s last dotted segment is `MSVC` and the display
/// name folds to `ripgrep msvc`. Any id whose last segment is a build or
/// vendor qualifier rather than the command is in this class, not just an id
/// with two aliases. No scan-time source for the real command exists.
///
/// Two things narrow that gap, neither of them here. `running_ids` catches the
/// **portable** subset by path regardless of what the process is called -- 4 of
/// 36 installed ids on a14, so a minority -- and a declared `[winget.guard]`
/// entry is the only route open to the rest: since Phase 5 Task 4,
/// `backend::apply_guard_overrides` appends that table's names to the very
/// `bins` this function filled, in a second pass over the finished `Scan`.
/// **Deliberately not here:** this is a pure function of winget's own `list`
/// output -- `tests/winget_scan.rs` drives it with rows and nothing else -- and
/// taking a `Config` would end that.
///
/// **A route open is not a gap closed, and how much of the gap is covered is
/// not a property of this code at all** -- it is whatever the user wrote, for
/// the packages the user thought to write it for. An empty `[winget.guard]`
/// leaves the gap exactly as wide as this comment's first half describes. What
/// is **measured** (`docs/measurements-2026-08-11-phase5-guard-unmanaged-
/// retry.md` §2) is only that three real ids run processes no rule here could
/// have derived, which is why the route exists.
pub fn rows_to_scan(rows: Vec<WingetRow>) -> Scan {
    let mut groups: BTreeMap<Name, Vec<WingetRow>> = BTreeMap::new();
    for row in rows {
        let name = Name::new(row.id.clone());
        groups.entry(name).or_default().push(row);
    }

    let mut scan = Scan::default();
    let mut sourceless_count = 0usize;
    for (name, group) in groups {
        if group.iter().any(|r| r.source.is_none()) {
            scan.opaque.push(name);
            sourceless_count += 1;
            continue;
        }

        if let Some(unusable) = group.iter().find(|r| r.version.starts_with("> ")) {
            scan.warnings.push(format!(
                "{name}: winget reports the version as \"{}\" -- that is winget saying *at \
                 least*, for an install whose exact version it could not determine, not a \
                 version dotpkg can compare against",
                unusable.version
            ));
            scan.opaque.push(name);
            continue;
        }

        let mut versions: Vec<&str> = Vec::new();
        for r in &group {
            if !versions.contains(&r.version.as_str()) {
                versions.push(r.version.as_str());
            }
        }

        if versions.len() > 1 {
            scan.warnings.push(format!(
                "{name}: installed at {} disagreeing versions ({}) -- refusing to guess which \
                 one is current",
                versions.len(),
                versions.join(", ")
            ));
            scan.opaque.push(name);
            continue;
        }

        if group.len() > 1 {
            scan.warnings.push(format!(
                "{name}: {} rows from `winget list` all named version {} -- collapsed to one \
                 entry (winget's own export does this silently; dotpkg records it instead)",
                group.len(),
                versions[0]
            ));
        }

        scan.installed.push(Installed {
            backend: WINGET.to_string(),
            name,
            version: versions[0].to_string(),
            arch: None,
            bucket: None,
            bins: guard_names(&group[0].id, &group[0].name),
        });
    }

    if sourceless_count > 0 {
        scan.warnings.push(format!(
            "{sourceless_count} installed entries have no winget Source (every MSIX/ARP entry \
             `winget list` prints) and cannot be compared against any index -- not warned about \
             individually, to avoid one line per entry on every run"
        ));
    }

    scan
}

/// Strip winget's line ending from every line without disturbing anything
/// else on it.
///
/// Sibling to the identical three lines in `parse_list` above: every fixture
/// is CRLF (pinned by `.gitattributes`; see `PROVENANCE.md`), and a
/// `trim`-based defence would only absorb the `\r` by accident -- Task 9's
/// review found exactly that gap. `show` and `show --versions` get their own
/// copy rather than sharing `parse_list`'s, so this task's scope stays
/// confined to the two new functions.
fn strip_cr(stdout: &str) -> Vec<&str> {
    stdout
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

/// The id winget echoes back in `Found <name> [<Id>]` -- the text between the
/// last `[` and the trailing `]` on the first line that starts `Found `.
///
/// Both `show` and `show --versions` open their stdout with this line, so
/// both `parse_show` and `parse_versions` need it. `<name>` (`Git`, `RipGrep
/// MSVC`) is the display/marketing name and is worthless as a `winget --id`
/// argument; `<Id>` (`Git.Git`, `BurntSushi.ripgrep.MSVC`) is the canonical
/// spelling -- see `docs/measurements-2026-08-09-winget.md` §3 and this
/// module's `parse_show` doc comment for why recording it, rather than the
/// spelling the caller asked with, is the whole point of this call.
fn found_id(lines: &[&str]) -> Option<String> {
    let line = lines.iter().find(|l| l.starts_with("Found "))?;
    let open = line.rfind('[')?;
    let close = line.rfind(']')?;
    if close <= open {
        return None;
    }
    Some(line[open + 1..close].to_string())
}

/// One package `winget show` resolved to: the canonical id it echoed back,
/// and the version it printed.
///
/// `id` is never the spelling the caller asked `show` with -- see
/// `parse_show`'s doc comment below and `src/model.rs`'s `Name` doc comment,
/// which this measurement corrected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub id: String,
    pub version: String,
}

/// Parse `winget show`'s stdout into the canonical id and the version it
/// reports.
///
/// Written against a call made WITHOUT `-e`/`--exact`. Measured
/// (`docs/measurements-2026-08-09-winget.md` §3): `--exact` is what makes
/// `--id` case-sensitive --
///
/// | argv | exit | stdout |
/// |---|---|---|
/// | `show -e --id Git.Git` | `0` | `Found Git [Git.Git]` |
/// | `show -e --id git.git` | `0x8A150014` | `No package found matching input criteria.` |
/// | `show --id git.git` (no `-e`) | `0` | `Found Git [Git.Git]` |
///
/// -- so a caller that put `Name::key()` (the folded form dotpkg's own
/// comparisons use) into `--exact --id` would get "not found" for a package
/// that exists. Dropping `--exact` both folds case on the way in AND hands
/// back the canonical spelling on the way out, in the same `Found <name>
/// [<Id>]` line: one self-verifying call. `<Id>` -- the brackets, never
/// `<name>` -- is what this function returns as `id`; `<name>` is a
/// display/marketing name (`Found RipGrep MSVC [BurntSushi.ripgrep.MSVC]`)
/// and is worthless as a `winget --id` argument.
///
/// Refuses -- naming the first 120 characters of stdout, matching
/// `parse_list`'s style -- if either the `Found` line or the `Version:` line
/// is missing, rather than returning a `Found` with an empty field: an empty
/// `Found` would silently be a package named `""` at version `""`, and
/// Phase 4 Task 13's `resolve_installed`/`resolve_latest` would go on to compare that
/// against real data.
pub fn parse_show(stdout: &str) -> Result<Found> {
    let lines = strip_cr(stdout);

    let id = found_id(&lines).ok_or_else(|| {
        let head: String = stdout.chars().take(120).collect();
        anyhow::anyhow!("winget show produced no \"Found <name> [<id>]\" line: {head:?}")
    })?;

    let version = lines
        .iter()
        .find_map(|line| line.strip_prefix("Version:"))
        .map(|rest| rest.trim().to_string())
        .ok_or_else(|| {
            let head: String = stdout.chars().take(120).collect();
            anyhow::anyhow!("winget show produced no \"Version:\" line: {head:?}")
        })?;

    Ok(Found { id, version })
}

/// Parse `winget show --versions`' stdout into the canonical id (the same
/// `Found <name> [<Id>]` line `parse_show` reads) and every version the index
/// still holds for it, in the order winget printed them.
///
/// Retention is a publisher policy, not a winget guarantee -- measured from
/// 8 (`BurntSushi.ripgrep.MSVC`) to 828 (`JanDeDobbeleer.OhMyPosh`) -- so
/// `vs.len()` is itself information: Phase 4 Task 13 uses it to say how deep the
/// index goes when a pin has fallen off the end ("this publisher keeps eight
/// releases" is more help than "the manifest is gone").
///
/// After the `Found` line: skip the `Version` header and the `---` rule
/// directly under it, then take every remaining non-blank line, trimmed, in
/// order. Nothing here re-sorts the list -- newest-first is winget's own
/// ordering (measured: `show`'s `Version:` line agreed with row 0 of
/// `--versions` on 6 of 6 packages tried, `docs/measurements-2026-08-09-winget.md`
/// §6), and re-sorting it would silently launder a winget ordering change
/// into a dotpkg bug instead of surfacing it.
pub fn parse_versions(stdout: &str) -> Result<(String, Vec<String>)> {
    let lines = strip_cr(stdout);

    let id = found_id(&lines).ok_or_else(|| {
        let head: String = stdout.chars().take(120).collect();
        anyhow::anyhow!("winget show --versions produced no \"Found <name> [<id>]\" line: {head:?}")
    })?;

    let header_idx = lines
        .iter()
        .position(|line| line.trim() == "Version")
        .ok_or_else(|| {
            let head: String = stdout.chars().take(120).collect();
            anyhow::anyhow!("winget show --versions produced no \"Version\" header: {head:?}")
        })?;

    let mut idx = header_idx + 1;
    if lines
        .get(idx)
        .map(|l| !l.is_empty() && l.chars().all(|c| c == '-'))
        .unwrap_or(false)
    {
        idx += 1;
    }

    let versions: Vec<String> = lines[idx..]
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();

    if versions.is_empty() {
        let head: String = stdout.chars().take(120).collect();
        bail!("winget show --versions listed no versions after its header: {head:?}");
    }

    Ok((id, versions))
}

/// Why a winget invocation could not be made at all -- distinct from a
/// winget that ran and reported failure through its exit code.
///
/// `anyhow::Error` erases `io::ErrorKind`, and `Winget::scan` needs exactly
/// that one bit: a machine with no `winget.exe` is a legitimate, empty
/// machine, while a `winget.exe` that exists and cannot be run is a machine
/// whose state dotpkg does not know. Before this split both reached the same
/// arm -- `Scoop::scan` had always distinguished them.
#[derive(Debug)]
pub enum CmdError {
    /// `winget.exe` is not on `PATH`.
    NotFound,
    Other(anyhow::Error),
}

impl std::fmt::Display for CmdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CmdError::NotFound => write!(f, "winget.exe is not on PATH"),
            CmdError::Other(e) => write!(f, "{e:#}"),
        }
    }
}

/// One `winget` invocation's outcome: the exit code and stdout, verbatim.
///
/// `code` is a plain `i32`, not the `Option<i32>` `execute::CommandReport`
/// uses for scoop -- every one of the ~45 measured invocations
/// (`docs/measurements-2026-08-09-winget.md`) exited with a code, none was
/// killed by a signal, so there is no signal case to model here.
///
/// stdout only: see `RealWinget::run`'s doc comment for why stderr never
/// reaches this struct at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdOut {
    pub code: i32,
    pub stdout: String,
}

/// The seam. Every winget invocation this crate makes goes through here, so
/// every test can fake it and none has to spawn `winget.exe` -- the sibling
/// rule to `tests/cli.rs`'s "no test may provide a fake scoop binary", and to
/// the standing rule that no test may create a file at `Scoop::scoop_exe()`'s
/// path either.
pub trait WingetCmd {
    fn run(&self, args: &[&str]) -> Result<CmdOut, CmdError>;
}

/// The real `winget.exe`, invoked as a subprocess. Only production code may
/// construct this -- `main.rs` does, in every arm that scans or resolves --
/// and every test uses a fake that implements `WingetCmd` instead.
pub struct RealWinget;

impl WingetCmd for RealWinget {
    fn run(&self, args: &[&str]) -> Result<CmdOut, CmdError> {
        let out = Command::new("winget")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    CmdError::NotFound
                } else {
                    CmdError::Other(
                        anyhow::Error::new(e).context(format!("cannot run winget {args:?}")),
                    )
                }
            })?;
        // `out.stderr` is captured (so it cannot leak to this process's own
        // stderr) and then never read -- deliberately discarded rather than
        // merged into `stdout`. Measured across ~45 invocations on a14
        // (docs/measurements-2026-08-09-winget.md): stderr was 0 bytes every
        // single time, including all three failure codes, including the
        // 0x80070003 export failure whose error text was itself printed to
        // STDOUT. So anything winget ever writes to stderr on a real machine
        // is a surprise this crate has never seen measured -- silently
        // folding it into stdout would hide that surprise instead of
        // surfacing it.
        Ok(CmdOut {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        })
    }
}

/// `winget list -e --id <a package that does not exist>` exits this code on
/// a14 -- `0x8A150014` read as a signed `i32`. `winget show -e --id <absent>`
/// exits it too.
///
/// **Not "found nothing".** `winget list -s msstore` against a source with no
/// matching package prints the byte-identical 53-byte sentence `list -e --id
/// <absent>` does, and exits `0` -- see `tests/fixtures/winget/PROVENANCE.md`,
/// where both fixtures are checked in side by side precisely so this is not
/// forgotten. The exit code is a function of the *filter shape* winget was
/// asked with, not of the output it printed, so this constant may only be
/// trusted against `code` for the exact argv shapes this crate uses -- pinned
/// by `scan_asks_winget_exactly_once_with_the_argv_this_phase_measured` in
/// `tests/winget_scan.rs`.
pub const NO_APPLICATIONS_FOUND: i32 = -1978335212;

/// `winget show -e --id <an id that exists> -v <a version that does not>`
/// exits this code on a14 -- `0x8A150017`, deliberately distinct from
/// `NO_APPLICATIONS_FOUND` above: one says the package itself is gone, this
/// one says a specific version of a package that still exists is gone.
///
/// `version_liveness` below is the one place it is read, on behalf of both
/// callers that ask whether a version is still in the index: it needs to tell
/// "this exact version fell out of the index" (this code) apart from "the
/// package itself is no longer in any index at all"
/// (`NO_APPLICATIONS_FOUND`), because the two lead to different advice for
/// the user.
pub const NO_VERSION_FOUND: i32 = -1978335209;

/// One package manager, `winget`, behind the `WingetCmd` seam -- generic so
/// that `RealWinget` and a test's fake are interchangeable, and so nothing
/// outside this module needs to know which one it is holding.
pub struct Winget<C: WingetCmd> {
    cmd: C,
}

impl<C: WingetCmd> Winget<C> {
    pub fn new(cmd: C) -> Winget<C> {
        Winget { cmd }
    }
}

impl<C: WingetCmd> Backend for Winget<C> {
    fn name(&self) -> &'static str {
        WINGET
    }

    /// Runs exactly `["list", "--disable-interactivity"]` -- no filter at
    /// all, deliberately: the argv is part of this function's contract (see
    /// `NO_APPLICATIONS_FOUND`'s doc comment for why), and it is the one
    /// shape the exit-code trust below is measured against.
    fn scan(&self) -> Result<Scan> {
        let out = match self.cmd.run(&["list", "--disable-interactivity"]) {
            Ok(out) => out,
            // Symmetric with `Scoop::scan`'s `NotFound` arm for a missing
            // `~/scoop/apps`: a machine with no `winget.exe` on `PATH` is a
            // legitimate machine -- not every Windows install has it -- and
            // an empty `Scan` is the right answer. But unlike that arm this
            // records a warning rather than staying silent, because "winget
            // could not be run" and "winget ran and found nothing" would
            // otherwise be indistinguishable to the user.
            Err(CmdError::NotFound) => {
                let mut scan = Scan::default();
                scan.warnings
                    .push(format!("winget could not be run: {}", CmdError::NotFound));
                return Ok(scan);
            }
            // The OTHER failure shape, which `CmdError::NotFound` above is
            // deliberately not: `winget.exe` exists and could not be run for
            // some other reason (permissions, a corrupt install, ...). This
            // machine's winget state is unknown, not empty -- an empty `Scan`
            // here would read as "nothing is installed", which is exactly the
            // wrong answer `mass_prune_guard` exists to catch too late. `Err`
            // propagates so `scan_or_warn` can turn it into
            // `ScanOutcome::Unscannable` instead.
            Err(CmdError::Other(e)) => return Err(e.context("winget list could not be run")),
        };
        // winget signals failure through its exit code -- the opposite of
        // scoop, which was measured to exit 0 for a hash mismatch, a dead
        // URL, and an uninstall of an app that was never installed (see
        // `execute::CommandReport`'s doc comment). Here the code is
        // trustworthy for this exact argv, and a nonzero code must become an
        // error, never an empty `Scan`: an empty machine is exactly what
        // `mass_prune_guard` exists to catch, and it catches it far too late
        // -- after a plan full of prunes has already been built.
        anyhow::ensure!(
            out.code == 0,
            "winget list exited {}: {}",
            out.code,
            out.stdout.lines().next().unwrap_or("(no output)")
        );
        Ok(rows_to_scan(parse_list(&out.stdout)?))
    }

    /// Ask winget what the declared spelling resolves to right now.
    ///
    /// `["show", "--id", <the declared spelling>, "--disable-interactivity"]`
    /// -- **no `--exact`**. Measured (`PROVENANCE.md`, `parse_show`'s own doc
    /// comment): `--exact` is what makes `--id` case-sensitive, so a folded
    /// or wrong-case spelling gets `NO_APPLICATIONS_FOUND` for a package that
    /// exists. Dropping it both folds case on the way in and hands back the
    /// canonical id on the way out, in the same `Found <name> [<Id>]` line
    /// `parse_show` reads -- ask with what the user wrote, `parse_show`
    /// records what winget matched.
    ///
    /// `declared`, `scoop_root`, `old` and `offline` are unused: unlike
    /// `Scoop::resolve_latest`, nothing here reads any of them -- winget has
    /// no bucket to choose and this call is not gated on network
    /// reachability, matching the brief's own test (`ResolveCtx::offline()`
    /// still resolves). `ctx.canonical` IS used, on the success path: the
    /// canonical id `parse_show` read out of `Found <name> [<Id>]` is what
    /// Task 15's `update` writes into `pkg.lock`'s key, not the spelling this
    /// method was asked with.
    fn resolve_latest(&self, name: &Name, ctx: &ResolveCtx) -> Resolution {
        let id = name.to_string();
        let out = match self
            .cmd
            .run(&["show", "--id", &id, "--disable-interactivity"])
        {
            Ok(out) => out,
            Err(e) => {
                return Resolution::Failed {
                    why: format!("winget show could not be run: {e:#}"),
                }
            }
        };
        if out.code == NO_APPLICATIONS_FOUND {
            // The package itself is gone -- not "no longer at this version",
            // which is `NO_VERSION_FOUND`'s fact, not this one's.
            return Resolution::Failed {
                why: format!(
                    "{name}: no longer in the winget index ({})",
                    out.stdout.lines().next().unwrap_or("(no output)")
                ),
            };
        }
        if out.code != 0 {
            return Resolution::Failed {
                why: format!(
                    "winget show {name} exited {}: {}",
                    out.code,
                    out.stdout.lines().next().unwrap_or("(no output)")
                ),
            };
        }
        match parse_show(&out.stdout) {
            Ok(found) => {
                *ctx.canonical.borrow_mut() = Some(Name::new(found.id));
                Resolution::Resolved {
                    pin: Pin::WingetVersion {
                        version: found.version,
                    },
                }
            }
            Err(e) => Resolution::Failed {
                why: format!("{e:#}"),
            },
        }
    }

    /// Confirm that `inst`'s installed version is still in winget's index --
    /// the installed version *is* the pin, but only if `show -v` still finds
    /// it.
    ///
    /// Refuses a version starting `"> "` before spawning anything: `rows_to_scan`
    /// already keeps those out of `installed` (see its own doc comment,
    /// reason 2), but this method is public API and a caller with a
    /// hand-built `Installed` must still be refused. Measured: `> 17.14.37`
    /// is winget saying *at least*, for an install whose exact version it
    /// cannot determine -- pinning it would write a lock entry that can never
    /// match.
    ///
    /// Otherwise the whole question is `version_liveness`'s -- see its own doc
    /// comment for the argv, for why it carries no `--exact`, and for how it
    /// tells "this version is gone" apart from "this package is gone". The
    /// only thing this method adds on top is what to do with the answer: the
    /// `Found` becomes the pin, and the canonical id it read becomes
    /// `ctx.canonical`.
    fn resolve_installed(&self, inst: &Installed, ctx: &ResolveCtx) -> Resolution {
        if inst.version.starts_with("> ") {
            return Resolution::Failed {
                why: format!(
                    "{}: winget reports the installed version as {:?} -- that is winget \
                     saying *at least*, for an install whose exact version it cannot \
                     determine, not a version dotpkg can pin",
                    inst.name, inst.version
                ),
            };
        }

        match version_liveness(&self.cmd, &inst.name, &inst.version) {
            Ok(found) => {
                *ctx.canonical.borrow_mut() = Some(Name::new(found.id));
                Resolution::Resolved {
                    pin: Pin::WingetVersion {
                        version: found.version,
                    },
                }
            }
            Err(why) => Resolution::Failed { why },
        }
    }
}

/// Is `version` of `id` still in winget's index?
///
/// Shared body of the two questions that ask it, so the argv and every error
/// sentence are decided once: `Winget::resolve_installed` above, which needs
/// the `Found` back because the installed version *is* the pin it is about to
/// write, and `apply::prepare`'s winget branch, which needs only to know that
/// the version `pkg.lock` pins can still be installed before it lets a
/// `WingetStep::Set` be built for it. A free function taking `&dyn WingetCmd`
/// rather than a `Winget<C>` method, because `prepare` holds the seam, not a
/// backend.
///
/// `["show", "--id", <id>, "-v", <version>, "--disable-interactivity"]` --
/// **no `--exact`**, for the same measured reason as `resolve_latest`
/// (`PROVENANCE.md`, `parse_show`'s own doc comment): `--exact` is what makes
/// `--id` case-sensitive, so a folded or wrong-case spelling gets
/// `NO_APPLICATIONS_FOUND` for a package that exists. Phase 4b Task 13's brief wrote
/// this call as `show -e --id ... -v ...`; the `-e` is deliberately **not**
/// here, because this is the argv shape the crate's exit-code trust was
/// measured against and adding a flag no measurement covers would hide a
/// case-sensitivity bug behind a liveness check.
///
/// `NO_VERSION_FOUND` means the package is still in the index but this exact
/// version is not: a second call to `--versions` answers how deep that index
/// goes, because retention is a publisher policy that spans three orders of
/// magnitude (8 for `BurntSushi.ripgrep.MSVC`, 828 for
/// `JanDeDobbeleer.OhMyPosh`) and "this publisher keeps N releases" is more
/// help than "the manifest is gone". `NO_APPLICATIONS_FOUND` means the
/// package itself is gone -- a different fact, so a different message, never
/// conflated with the version-only one.
///
/// **Cost: one subprocess on the happy path, two on that `NO_VERSION_FOUND`
/// branch.** Measured on a14, a `show` invocation is ~1.09 s
/// (`docs/measurements-2026-08-09-winget.md`), which matters because
/// `apply::check_pin_is_live` calls this once per winget action in a plan,
/// serially. The second call is bought deliberately: it turns "the manifest is
/// gone" into "this publisher currently keeps 8 versions (15.0.0..15.2.0)",
/// and it only happens on a path that has already failed.
///
/// `Err` is already the whole sentence a user reads, and already names `id`
/// where naming it helps: every caller puts it straight into a `Resolution`
/// or an `Outcome` without adding to it.
pub(crate) fn version_liveness(
    cmd: &dyn WingetCmd,
    id: &Name,
    version: &str,
) -> Result<Found, String> {
    let id_arg = id.to_string();
    let out = match cmd.run(&[
        "show",
        "--id",
        &id_arg,
        "-v",
        version,
        "--disable-interactivity",
    ]) {
        Ok(out) => out,
        Err(e) => return Err(format!("winget show could not be run: {e:#}")),
    };

    if out.code == NO_VERSION_FOUND {
        let depth = match cmd.run(&[
            "show",
            "--id",
            &id_arg,
            "--versions",
            "--disable-interactivity",
        ]) {
            Ok(versions_out) if versions_out.code == 0 => parse_versions(&versions_out.stdout).ok(),
            _ => None,
        };
        return Err(match depth {
            Some((_, versions)) => format!(
                "{}: version {} is no longer in the winget index -- this publisher \
                 currently keeps {} version(s) ({}..{})",
                id,
                version,
                versions.len(),
                versions.first().map(String::as_str).unwrap_or("?"),
                versions.last().map(String::as_str).unwrap_or("?"),
            ),
            None => format!("{id}: version {version} is no longer in the winget index"),
        });
    }
    if out.code == NO_APPLICATIONS_FOUND {
        return Err(format!(
            "{}: no longer in the winget index ({})",
            id,
            out.stdout.lines().next().unwrap_or("(no output)")
        ));
    }
    if out.code != 0 {
        return Err(format!(
            "winget show {} exited {}: {}",
            id,
            out.code,
            out.stdout.lines().next().unwrap_or("(no output)")
        ));
    }
    parse_show(&out.stdout).map_err(|e| format!("{e:#}"))
}

/// Is `id` installed **at user scope**, as far as winget will say?
///
/// The one production answer for `apply::refuse_elevated_winget_removal`'s
/// injected `is_user_scope`, so the whole pre-check is only as good as this
/// argv. A free function taking `&dyn WingetCmd` rather than a `Winget<C>`
/// method, for the same reason `version_liveness` above is one: `main.rs` holds
/// the seam at that point, not a backend.
///
/// `["list", "-e", "--id", <id>, "--scope", "user", "--disable-interactivity"]`
/// -- the exact argv measured on a14
/// (`docs/measurements-2026-08-10-winget-write-path.md` §15), and `-e` here for
/// the same reason as in `list_one_argv`: it is what makes `--id` exact, and
/// the id handed in comes from `pkg.lock`, which holds the spelling winget
/// itself produced.
///
/// **Three answers, not two.** `Some(true)` and `Some(false)` are the two
/// shapes §15 measured, in both directions: 19 of the 36 source-backed
/// installed ids exit `0` under `--scope user` and `0x8A150014` under `--scope
/// machine`, and `Microsoft.VisualStudio.2022.BuildTools` does the exact
/// reverse. `None` is "could not tell", and it must not become a refusal --
/// the same rule `sys::elevated()`'s own `None` follows. A refusal has to be
/// caused by a measured hazard, never by a missing answer, and
/// `execute::run_winget_step`'s `CANNOT_UNINSTALL_ELEVATED` translation is
/// what catches whatever this lets through.
///
/// **Exit `0` is not trusted on its own.** `list -s msstore` against a source
/// with no match prints the byte-identical 53-byte `No installed package found
/// matching input criteria.` and exits `0` (`NO_APPLICATIONS_FOUND`'s own doc
/// comment; both fixtures are checked in side by side for exactly this
/// reason), so a bare `code == 0` would read a sentence saying "not installed"
/// as "installed at user scope" and refuse a removal on the strength of it.
/// The row for *this* id has to be there, in the `Id` column, parsed the same
/// way every other list in this module is.
///
/// **Cost: one ~1 s subprocess per call** (`docs/measurements-2026-08-09-winget.md`).
/// `refuse_elevated_winget_removal` is what keeps that off every other path:
/// it asks only for a `WingetStep::Remove`, and only when the process is known
/// to be elevated.
pub fn installed_at_user_scope(cmd: &dyn WingetCmd, id: &Name) -> Option<bool> {
    let id_arg = id.to_string();
    let out = cmd
        .run(&[
            "list",
            "-e",
            "--id",
            &id_arg,
            "--scope",
            "user",
            "--disable-interactivity",
        ])
        .ok()?;
    if out.code == NO_APPLICATIONS_FOUND {
        return Some(false);
    }
    if out.code != 0 {
        return None;
    }
    // Compared as a `Name`, not as bytes. `winget_exec::winget_verdict` -- the
    // other place a `list` answer is matched back against the id that asked for
    // it -- does `&i.name == id`, which folds case, and these two must not
    // disagree about what "this row is that package" means. A byte comparison
    // here would return `None` ("could not tell") rather than `Some(true)` for a
    // row whose case differs from the caller's spelling, which fails this
    // pre-check **open**: `main.rs` reads `None` as "not blocked" and lets an
    // elevated user-scope removal through to winget's own refusal. Not reachable
    // from today's one caller -- `WingetStep::Remove`'s id is `winget list`'s own
    // `Id` column, so the two spellings are the same bytes by construction -- but
    // the guard must not depend on that staying true.
    if parse_list(&out.stdout)
        .ok()?
        .iter()
        .any(|r| Name::new(r.id.as_str()) == *id)
    {
        Some(true)
    } else {
        None
    }
}

impl<C: WingetCmd> Winget<C> {
    /// Refresh winget's own index for the one source dotpkg reads --
    /// `["source", "update", "--name", "winget", "--disable-interactivity"]`.
    ///
    /// Measured (`docs/measurements-2026-08-09-winget.md` §9, "Repeated,
    /// scoped"): scoped to `--name winget`, this exits `0` and changes
    /// nothing on the machine it was run against twice -- 141 rows before and
    /// after, the `(Name, Id, Version, Source)` multiset identical, zero
    /// `Available`-column moves. That measurement is what makes it safe to
    /// call unconditionally, unlike the **bare** `winget source update` (no
    /// `--name`), which installed winget's own `winget-font` source MSIX and
    /// is never used by this crate.
    ///
    /// This is winget's analogue of `bucket::fetch` -- the thing `--offline`
    /// skips -- and it is not a `Backend` trait method for the same reason
    /// `bucket::fetch` is not one: it is not "resolve one package", it is a
    /// whole-run, once-per-invocation refresh that only ever makes sense for
    /// winget, the same way per-bucket fetching only ever makes sense for
    /// scoop.
    pub fn update_source(&self) -> Result<()> {
        let out = match self.cmd.run(&[
            "source",
            "update",
            "--name",
            "winget",
            "--disable-interactivity",
        ]) {
            Ok(out) => out,
            // Only the new type changes here: any `WingetCmd::run` failure
            // already turned into an `Err` for this caller, same as before.
            Err(e) => bail!("winget source update could not be run: {e}"),
        };
        anyhow::ensure!(
            out.code == 0,
            "winget source update exited {}: {}",
            out.code,
            out.stdout.lines().next().unwrap_or("(no output)")
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::Process;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    #[test]
    fn running_ids_catches_a_package_whose_process_runs_from_its_winget_package_dir() {
        // Measured on a14 (measurements-2026-08-11 §1): exactly one live
        // process ran from under WinGet\Packages, and this is its real path.
        let roots = vec![PathBuf::from(
            r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages",
        )];
        let procs = vec![Process {
            name: "vkey".to_string(),
            exe: Some(PathBuf::from(
                r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages\PhatMT97.VKey_Microsoft.Winget.Source_8wekyb3d8bbwe\VKey.exe",
            )),
        }];
        let scanned = vec![Name::new("PhatMT97.VKey")];
        assert_eq!(
            running_ids(&roots, &procs, &scanned),
            BTreeSet::from([Name::new("PhatMT97.VKey")])
        );
    }

    #[test]
    fn running_ids_treats_a_root_with_a_trailing_separator_the_same_as_without() {
        // Same fixture as the test above, except the caller's root already
        // ends in a separator. Structural: `package_roots()` never produces
        // one (it builds paths with `.join()`), so nothing in this crate hits
        // this today -- but `running_ids` is a general-purpose pure function
        // a later task wires up, and its next caller is not obliged to know
        // that. Before the fix this went the dangerous direction: the
        // trailing `\` doubled the appended separator, `strip_prefix` matched
        // no real path, and the set came back empty.
        let roots = vec![PathBuf::from(
            r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages\",
        )];
        let procs = vec![Process {
            name: "vkey".to_string(),
            exe: Some(PathBuf::from(
                r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages\PhatMT97.VKey_Microsoft.Winget.Source_8wekyb3d8bbwe\VKey.exe",
            )),
        }];
        let scanned = vec![Name::new("PhatMT97.VKey")];
        assert_eq!(
            running_ids(&roots, &procs, &scanned),
            BTreeSet::from([Name::new("PhatMT97.VKey")])
        );
    }

    #[test]
    fn running_ids_requires_the_underscore_boundary_so_a_dead_sibling_dir_matches_nothing() {
        // Measured: PhatMT97.VKey.Classic_... still exists on disk holding only
        // a config.toml, has no <id>_<hash>.db and no ARP key, and is absent
        // from `winget list`. Its folded segment starts with the folded id
        // "phatmt97.vkey" and must NOT match, because what follows is '.' and
        // not '_'. Without the boundary check this test goes green while the
        // fence claims a package is running that is not even installed.
        let roots = vec![PathBuf::from(
            r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages",
        )];
        let procs = vec![Process {
            name: "whatever".to_string(),
            exe: Some(PathBuf::from(
                r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages\PhatMT97.VKey.Classic_Microsoft.Winget.Source_8wekyb3d8bbwe\config.exe",
            )),
        }];
        let scanned = vec![Name::new("PhatMT97.VKey")];
        assert_eq!(running_ids(&roots, &procs, &scanned), BTreeSet::new());
    }

    #[test]
    fn running_ids_ignores_a_process_whose_path_cannot_be_read() {
        // Measured: 22 of 223 live processes reported no readable path. That is
        // the blind spot `Running.names` covers and this function must not
        // pretend to; a path-only implementation that unwrapped `exe` would
        // panic, and one that treated None as a match would be worse.
        let roots = vec![PathBuf::from(
            r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages",
        )];
        let procs = vec![Process {
            name: "vkey".to_string(),
            exe: None,
        }];
        let scanned = vec![Name::new("PhatMT97.VKey")];
        assert_eq!(running_ids(&roots, &procs, &scanned), BTreeSet::new());
    }

    #[test]
    fn running_ids_only_answers_for_ids_the_scan_actually_found() {
        // The dead-directory case from the other side: a live process under a
        // package dir for an id that is not installed produces nothing,
        // because `covers` is only ever asked about an `Installed`.
        let roots = vec![PathBuf::from(
            r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages",
        )];
        let procs = vec![Process {
            name: "zoxide".to_string(),
            exe: Some(PathBuf::from(
                r"C:\Users\kln\AppData\Local\Microsoft\WinGet\Packages\ajeetdsouza.zoxide_Microsoft.Winget.Source_8wekyb3d8bbwe\zoxide.exe",
            )),
        }];
        assert_eq!(running_ids(&roots, &procs, &[]), BTreeSet::new());
    }

    #[test]
    fn running_ids_folds_case_on_both_sides() {
        // The real directory is mixed case ("PhatMT97.VKey_...") and
        // `Name::key()` is the lowercased form. A comparison that folds only
        // one side silently never matches -- the exact trap `guard_names`' own
        // doc comment records for process names.
        let roots = vec![PathBuf::from(r"C:\ROOT\Packages")];
        let procs = vec![Process {
            name: "x".to_string(),
            exe: Some(PathBuf::from(
                r"c:\root\packages\AJEETDSOUZA.ZOXIDE_Microsoft.Winget.Source_x\zoxide.exe",
            )),
        }];
        let scanned = vec![Name::new("ajeetdsouza.zoxide")];
        assert_eq!(
            running_ids(&roots, &procs, &scanned),
            BTreeSet::from([Name::new("ajeetdsouza.zoxide")])
        );
    }

    #[test]
    fn running_ids_returns_nothing_when_no_root_exists() {
        // Off Windows `package_roots()` finds no environment variables and
        // returns an empty vector; the function must be a no-op, not a panic.
        let procs = vec![Process {
            name: "vkey".to_string(),
            exe: Some(PathBuf::from("/usr/bin/vkey")),
        }];
        assert_eq!(
            running_ids(&[], &procs, &[Name::new("PhatMT97.VKey")]),
            BTreeSet::new()
        );
    }

    #[test]
    fn guard_names_are_the_two_signals_measured_to_catch_a_real_process() {
        // Measured on a14 against the live process table: of 36 source-backed
        // installed winget ids, the whole dotted id caught 0, the id's LAST
        // dotted segment caught 4, and the display Name column caught 2.
        // Brave.Brave was running at the time and today's guard missed it.
        assert_eq!(guard_names("Brave.Brave", "Brave"), vec!["brave"]);
        // Chrome is the case the display name cannot reach and the last segment
        // can: the process is chrome.exe, the display name is "Google Chrome".
        assert_eq!(
            guard_names("Google.Chrome", "Google Chrome"),
            vec!["chrome", "google chrome"]
        );
        // An id with no dot at all must still yield its own name, not nothing.
        assert_eq!(guard_names("xh", "xh"), vec!["xh"]);
        // Case is folded, because `sys::running_processes` lowercases what it
        // reports and a comparison against unfolded text silently never matches.
        assert_eq!(guard_names("PhatMT97.VKey", "VKey"), vec!["vkey"]);
        // An empty display Name must not produce an empty guard name: `names`
        // is a BTreeSet<String> that could contain "" and match nothing, but a
        // future caller comparing against it would be comparing against noise.
        assert_eq!(guard_names("Some.Thing", ""), vec!["thing"]);
    }

    #[test]
    fn the_two_error_codes_decimal_and_hex_forms_still_agree() {
        // Measured (`docs/measurements-2026-08-10-winget-write-path.md`,
        // `PROVENANCE.md`): each constant's decimal value came off a real a14
        // exit code, and the hex in the trailing comment beside its own
        // definition is winget's `0x8A1500..` spelling of that identical
        // code, read as a signed `i32`. This is not a restatement of the
        // same number -- it is the one place anything checks the decimal
        // against the hex rather than against itself. Every other test in
        // this crate builds its `CmdOut::code` from the constant, so a sign
        // flip on the constant's own definition flips every one of those
        // tests right along with it and the suite stays green; only cross-
        // checking against the hex recorded beside it catches that.
        assert_eq!(NO_APPLICATIONS_FOUND as u32, 0x8A150014);
        assert_eq!(NO_VERSION_FOUND as u32, 0x8A150017);
    }
}
