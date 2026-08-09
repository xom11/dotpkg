use anyhow::{bail, Result};

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
