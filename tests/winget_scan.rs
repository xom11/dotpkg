mod common;

use common::fake_winget::FakeWinget;
use dotpkg::backend::winget::{parse_list, rows_to_scan, Winget, WingetRow, NO_APPLICATIONS_FOUND};
use dotpkg::backend::Backend;
use dotpkg::model::Name;

fn fixture(name: &str) -> String {
    // Rust does no newline translation, so this keeps the CRLF the fixture was
    // captured with -- which is the point: a parser tested only against \n
    // passes here and fails on the one platform this tool runs on.
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/winget")
            .join(name),
    )
    .unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

#[test]
fn the_full_table_parses_to_every_row_winget_printed() {
    let rows = parse_list(&fixture("list-full.txt")).unwrap();
    assert_eq!(rows.len(), 141, "141 rows were captured");
    let ids: std::collections::BTreeSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids.len(), 126, "126 of them are distinct");
    assert_eq!(
        rows.iter().filter(|r| r.source.is_none()).count(),
        84,
        "84 rows have no Source and cannot be compared against any index"
    );
}

#[test]
fn a_table_with_no_available_column_still_parses() {
    // The column SET is data-dependent: `Available` is absent whenever no row
    // has an upgrade. A parser keyed on column count instead of on header
    // names reads Source out of the Available slot and reports every package
    // as sourceless.
    let rows = parse_list(&fixture("list-duplicate-id.txt")).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.available.is_none()));
    assert!(
        rows.iter().all(|r| r.source.as_deref() == Some("winget")),
        "Source must not be read out of the missing Available column: {rows:?}"
    );
}

#[test]
fn one_id_can_appear_twice_with_two_different_versions() {
    let rows = parse_list(&fixture("list-duplicate-id.txt")).unwrap();
    let versions: Vec<&str> = rows.iter().map(|r| r.version.as_str()).collect();
    assert_eq!(rows[0].id, "7zip.7zip");
    assert_eq!(rows[1].id, "7zip.7zip");
    assert_eq!(versions, vec!["26.01.00.0", "26.02"]);
}

#[test]
fn a_version_winget_will_not_commit_to_is_kept_verbatim() {
    // "> 17.14.37" is winget saying *at least*: one machine-scoped install
    // whose exact version it cannot determine. Kept as written here; Task 10
    // is what refuses to treat it as a version.
    let rows = parse_list(&fixture("list-greater-prefix.txt")).unwrap();
    assert_eq!(rows.len(), 1, "ONE row -- not several installs");
    assert_eq!(rows[0].version, "> 17.14.37");
}

#[test]
fn the_available_column_is_read_when_it_is_there() {
    let rows = parse_list(&fixture("list-upgrade-available.txt")).unwrap();
    let chrome = rows
        .iter()
        .find(|r| r.id == "Google.Chrome")
        .expect("in the fixture");
    assert_eq!(chrome.version, "150.0.7871.187");
    assert_eq!(chrome.available.as_deref(), Some("151.0.7922.109"));
}

#[test]
fn a_not_found_message_is_not_a_table_and_is_not_silently_empty() {
    // list-not-found.txt and list-source-filter-empty.txt are BYTE-IDENTICAL
    // and came back with different exit codes. So the parser may not decide
    // "found nothing" from the text -- it must say "this is not a table" and
    // let the caller read the exit code.
    let r = parse_list(&fixture("list-not-found.txt"));
    assert!(r.is_err(), "no header row means the parser must refuse");
    let msg = format!("{:#}", r.unwrap_err());
    assert!(msg.contains("header"), "and say why: {msg}");
}

#[test]
fn a_header_that_is_not_the_shape_this_parser_measured_is_refused() {
    // The header is English and therefore locale-dependent. Guessing offsets
    // on an unrecognised header reports an empty machine -- and an empty
    // machine is what mass_prune_guard exists to catch far too late.
    let r = parse_list("Nom  Identifiant  Version\r\n----\r\nx  y  z\r\n");
    assert!(r.is_err(), "an unrecognised header must refuse, not guess");
}

// -- rows_to_scan ---------------------------------------------------------

/// An ordinary, source-backed, single row for one id at one version.
/// `rows_to_scan`'s duplicate/disagreement tests build their own multi-row
/// cases on top of this; the sourceless and `"> "`-prefixed cases are
/// exercised through real fixtures instead, because both of those are about
/// what a real winget row looks like, not about grouping logic.
fn row(id: &str, version: &str) -> WingetRow {
    WingetRow {
        name: id.to_string(),
        id: id.to_string(),
        version: version.to_string(),
        available: None,
        source: Some("winget".to_string()),
    }
}

#[test]
fn the_whole_captured_machine_splits_into_exactly_these_counts() {
    // Computed from tests/fixtures/winget/list-full.txt, not estimated.
    // 141 rows -> 126 distinct ids -> 89 opaque + 37 installed.
    //
    //   84  ids with no Source        -- installed, comparable against nothing
    //    2  ids whose version is "> " -- Microsoft.VisualStudio.2022.BuildTools,
    //                                    Microsoft.WindowsAppRuntime.1.8
    //    3  ids whose duplicate rows disagree on a version --
    //                                    7zip.7zip, Microsoft.UI.Xaml.2.8,
    //                                    Microsoft.WindowsAppRuntime.2
    //   ---
    //   89  opaque        37 installed, 4 of them collapsed from duplicate rows
    //
    // 89 + 37 = 126 is the cross-check. If these numbers disagree with the
    // fixture, THE FIXTURE IS RIGHT and this comment is wrong: recompute,
    // fix the numbers, and say so in the report.
    let scan = rows_to_scan(parse_list(&fixture("list-full.txt")).unwrap());
    assert_eq!(scan.opaque.len(), 89);
    assert_eq!(scan.installed.len(), 37);
    assert_eq!(
        scan.opaque.len() + scan.installed.len(),
        126,
        "every id is one or the other"
    );
    assert!(scan
        .installed
        .iter()
        .all(|i| i.backend == dotpkg::model::WINGET));
    assert!(
        !scan
            .installed
            .iter()
            .any(|i| i.name.key().starts_with("msix\\") || i.name.key().starts_with("arp\\")),
        "no MSIX or ARP row may reach `installed`"
    );
}

#[test]
fn duplicate_ids_that_agree_on_a_version_collapse_to_one_entry_and_warn() {
    let rows = vec![
        row("WindowsAppRuntime.1.7", "1.7.9"),
        row("WindowsAppRuntime.1.7", "1.7.9"),
    ];
    let scan = rows_to_scan(rows);
    assert_eq!(scan.installed.len(), 1, "one package is one entry");
    assert_eq!(scan.installed[0].version, "1.7.9");
    assert_eq!(
        scan.warnings.len(),
        1,
        "winget's export collapses these silently; dotpkg may not"
    );
    assert!(scan.warnings[0].contains("WindowsAppRuntime.1.7"));
}

#[test]
fn duplicate_ids_that_disagree_on_a_version_are_opaque_rather_than_guessed() {
    // 7zip.7zip is installed twice, at 26.01.00.0 and 26.02. Two versions of
    // one package is a state dotpkg has no vocabulary for; picking one would
    // be inventing a fact. winget's own export picks 26.02 and says nothing.
    let scan = rows_to_scan(vec![
        row("7zip.7zip", "26.01.00.0"),
        row("7zip.7zip", "26.02"),
    ]);
    assert!(scan.installed.is_empty(), "got {:?}", scan.installed);
    assert_eq!(scan.opaque, vec![Name::new("7zip.7zip")]);
    assert!(
        scan.warnings[0].contains("26.01.00.0") && scan.warnings[0].contains("26.02"),
        "both versions must be named: {:?}",
        scan.warnings
    );
}

#[test]
fn a_greater_than_version_is_opaque_because_it_is_not_a_version() {
    // Left in `installed`, `cur.version == want` is false forever and
    // is_older() picks Downgrade, so status prints a false down-arrow on
    // every run and apply --yes acts on it.
    let scan = rows_to_scan(parse_list(&fixture("list-greater-prefix.txt")).unwrap());
    assert!(scan.installed.is_empty());
    assert_eq!(
        scan.opaque,
        vec![Name::new("Microsoft.VisualStudio.2022.BuildTools")]
    );
}

#[test]
fn an_ordinary_single_row_becomes_an_ordinary_installed_entry() {
    // The positive sibling. Without it, a rows_to_scan that marked EVERYTHING
    // opaque would satisfy all four assertions above.
    let scan = rows_to_scan(parse_list(&fixture("list-single.txt")).unwrap());
    assert!(scan.opaque.is_empty());
    assert_eq!(scan.installed.len(), 1);
    assert_eq!(scan.installed[0].name, Name::new("ajeetdsouza.zoxide"));
    assert_eq!(scan.installed[0].version, "0.10.0");
    assert_eq!(
        scan.installed[0].arch, None,
        "winget does not expose an architecture"
    );
    assert_eq!(scan.installed[0].bucket, None);
    assert_eq!(
        scan.installed[0].bins,
        vec!["zoxide"],
        "guard_names(\"ajeetdsouza.zoxide\", \"zoxide\") folds the id's last \
         segment and the display Name to the same string and deduplicates \
         them to one entry -- bins is no longer empty for winget"
    );
}

#[test]
fn a_winget_installed_entry_carries_guard_names_so_the_running_check_can_fire() {
    // `Running::covers` has three signals and, for winget, exactly one of
    // them could ever fire before this: `dirs` is filled only from
    // `$SCOOP/apps` and `$SCOOP/persist` (so a winget id can never be in
    // it) and `bins` was always empty, leaving a process named after the
    // WHOLE dotted id -- which nothing is. Measured: 0 of 36 caught.
    use dotpkg::model::Running;
    let scan = rows_to_scan(vec![WingetRow {
        name: "Brave".to_string(),
        id: "Brave.Brave".to_string(),
        version: "151.1.93.132".to_string(),
        available: None,
        source: Some("winget".to_string()),
    }]);
    let inst = &scan.installed[0];
    assert_eq!(inst.bins, vec!["brave"], "got {:?}", inst.bins);

    // The real process name on a14, folded and suffix-stripped the way
    // `sys::running_processes` reports it.
    let running = Running::new(
        std::collections::BTreeSet::from(["brave".to_string()]),
        Default::default(),
    );
    assert!(
        running.covers(inst),
        "a running Brave must be covered; before this it was not"
    );

    // The control that must stay green: an unrelated process must NOT cover
    // it. Without this, a `guard_names` that returned every possible string
    // would satisfy the assertion above.
    let unrelated = Running::new(
        std::collections::BTreeSet::from(["notepad".to_string()]),
        Default::default(),
    );
    assert!(!unrelated.covers(inst), "must not over-match to anything");
}

#[test]
fn the_84_sourceless_ids_produce_one_aggregate_warning_not_84_lines() {
    // render.rs prints an opaque skip as "...see the warnings above" -- every
    // opaque push must be backed by a warning somewhere, but src/main.rs
    // prints every warning unconditionally on every run, so one warning per
    // sourceless id (84 of them here) would flood every single invocation.
    let scan = rows_to_scan(parse_list(&fixture("list-full.txt")).unwrap());
    let sourceless: Vec<&String> = scan
        .warnings
        .iter()
        .filter(|w| w.contains("no winget Source"))
        .collect();
    assert_eq!(
        sourceless.len(),
        1,
        "one aggregate warning, not one per id: {:?}",
        scan.warnings
    );
    assert!(
        sourceless[0].contains("84"),
        "must name the count: {:?}",
        sourceless[0]
    );
}

#[test]
fn the_two_greater_than_prefixed_ids_are_each_named_in_their_own_warning() {
    // Unlike the 84 sourceless ids, these are rare (2 of 126) and are the one
    // bucket where a package the user actually declared can land in
    // `opaque` -- so each gets its own warning naming the id and what
    // winget reported, not folded into an aggregate.
    let scan = rows_to_scan(parse_list(&fixture("list-full.txt")).unwrap());
    let unusable: Vec<&String> = scan
        .warnings
        .iter()
        .filter(|w| w.contains("at least"))
        .collect();
    assert_eq!(
        unusable.len(),
        2,
        "one per id, not folded together: {:?}",
        scan.warnings
    );
    assert!(
        scan.warnings
            .iter()
            .any(|w| w.contains("Microsoft.VisualStudio.2022.BuildTools") && w.contains("17.14.37")),
        "{:?}",
        scan.warnings
    );
    assert!(
        scan.warnings
            .iter()
            .any(|w| w.contains("Microsoft.WindowsAppRuntime.1.8") && w.contains("1.8.9")),
        "{:?}",
        scan.warnings
    );
}

#[test]
fn a_scan_with_nothing_unusual_produces_no_warnings_at_all() {
    // The absence counterweight the two tests above need: without this, an
    // implementation that always emits both warning shapes -- regardless of
    // whether either condition actually occurred -- would satisfy every
    // `contains(...)` assertion above by coincidence.
    let scan = rows_to_scan(parse_list(&fixture("list-single.txt")).unwrap());
    assert!(scan.warnings.is_empty(), "got {:?}", scan.warnings);
}

// -- Winget::scan (the WingetCmd seam) -------------------------------------
//
// Every test below uses `FakeWinget` (tests/common/fake_winget.rs). No test
// in this crate may spawn `winget.exe` -- that is the entire reason
// `WingetCmd` exists as a seam, the sibling rule to `tests/cli.rs`'s "no test
// may provide a fake scoop binary".

#[test]
fn scan_asks_winget_exactly_once_with_the_argv_this_phase_measured() {
    // The exit code is a function of the FILTER, not of the output:
    // `list -s msstore` returns the same 53-byte sentence as
    // `list -e --id <absent>` and exits 0 where the other exits 0x8A150014.
    // So the argv is part of the contract and is pinned here.
    let fake = FakeWinget::returning(0, fixture("list-single.txt"));
    let scan = Backend::scan(&Winget::new(fake.clone())).unwrap();
    assert_eq!(fake.calls(), vec![vec!["list", "--disable-interactivity"]]);
    assert_eq!(scan.installed.len(), 1);
}

#[test]
fn a_machine_without_winget_is_an_empty_scan_and_a_warning_not_an_error() {
    // Symmetric with Scoop::scan, where a missing ~/scoop/apps is a valid
    // empty state rather than a failure.
    let fake = FakeWinget::failing_to_spawn();
    let scan = Backend::scan(&Winget::new(fake)).unwrap();
    assert!(scan.installed.is_empty() && scan.opaque.is_empty());
    assert_eq!(scan.warnings.len(), 1);
    assert!(
        scan.warnings[0].contains("winget"),
        "got {:?}",
        scan.warnings
    );
}

#[test]
fn a_nonzero_exit_from_list_is_an_error_not_an_empty_machine() {
    // An empty machine is exactly what mass_prune_guard exists to catch too
    // late. A `list` that fails must never look like "nothing is installed".
    let fake = FakeWinget::returning(NO_APPLICATIONS_FOUND, fixture("list-not-found.txt"));
    let r = Backend::scan(&Winget::new(fake));
    assert!(
        r.is_err(),
        "a failed list must not read as an empty machine"
    );
}

#[test]
fn the_backend_reports_the_name_the_lock_and_state_are_keyed_by() {
    assert_eq!(
        Backend::name(&Winget::new(FakeWinget::returning(0, String::new()))),
        dotpkg::model::WINGET
    );
}
