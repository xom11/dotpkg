use super::{Backend, Scan};
use crate::model::{Installed, Name, WINGET};
use anyhow::{bail, Context, Result};
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
            bins: Vec::new(),
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
/// Task 13's `resolve_installed`/`resolve_latest` would go on to compare that
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
/// `vs.len()` is itself information: Task 13 uses it to say how deep the
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
    fn run(&self, args: &[&str]) -> Result<CmdOut>;
}

/// The real `winget.exe`, invoked as a subprocess. Only production code
/// (`main.rs`, once a later task wires it up) may construct this -- every
/// test uses a fake that implements `WingetCmd` instead.
pub struct RealWinget;

impl WingetCmd for RealWinget {
    fn run(&self, args: &[&str]) -> Result<CmdOut> {
        let out = Command::new("winget")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .with_context(|| format!("cannot run winget {args:?}"))?;
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
/// **Not used by this task.** Task 13's pin-liveness check
/// (`Winget::resolve_installed`) is the caller: it runs `show ... -v <the
/// version pkg.lock pins>` and needs to tell "this exact version fell out of
/// the index" (this code) apart from "the package itself is no longer in any
/// index at all" (`NO_APPLICATIONS_FOUND`), because the two lead to different
/// advice for the user.
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
            // otherwise be indistinguishable to the user, and the caller
            // named in `e` is worth keeping.
            Err(e) => {
                let mut scan = Scan::default();
                scan.warnings
                    .push(format!("winget could not be run: {e:#}"));
                return Ok(scan);
            }
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
}
