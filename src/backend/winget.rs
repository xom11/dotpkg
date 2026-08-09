use super::Scan;
use crate::model::{Installed, Name, WINGET};
use anyhow::{bail, Result};
use std::collections::BTreeMap;

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
///      nothing: there is no index to check it against.
///   2. **any row's version starts with `"> "`** -- measured on two ids
///      (`Microsoft.VisualStudio.2022.BuildTools`,
///      `Microsoft.WindowsAppRuntime.1.8`). This is winget saying *at
///      least*, for an install whose exact version it could not determine --
///      not "several versions installed": `list -e --id` returns exactly one
///      row for each. Left in `installed`, `cur.version == want` would be
///      `"> 17.14.37" == "17.14.37"`, false forever, and `plan::is_older`
///      splits on non-digits so both sides reduce to the same digit sequence
///      and the remaining arm is `Downgrade` -- a false `↓` on every `status`
///      run, and something `apply --yes` would act on.
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
/// `arch` and `bucket` are always `None`: winget exposes neither. `bins` is
/// always empty -- there is no winget-side manifest to read executable names
/// from -- and that has a consequence: `Running::covers` (`src/model.rs`)
/// checks three signals (package directory, process name, declared
/// executables), and with `bins` empty only the first two can ever fire. The
/// running-process guard is therefore weaker for a winget package than for a
/// scoop one. Nothing depends on that today because `plan()` does not yet
/// act on winget packages; recorded here for whoever adds that.
pub fn rows_to_scan(rows: Vec<WingetRow>) -> Scan {
    let mut groups: BTreeMap<Name, Vec<WingetRow>> = BTreeMap::new();
    for row in rows {
        let name = Name::new(row.id.clone());
        groups.entry(name).or_default().push(row);
    }

    let mut scan = Scan::default();
    for (name, group) in groups {
        if group
            .iter()
            .any(|r| r.source.is_none() || r.version.starts_with("> "))
        {
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
            bins: Vec::new(),
        });
    }

    scan
}
