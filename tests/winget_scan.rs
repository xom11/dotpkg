// `WingetRow` isn't named directly below (every test binds through
// `parse_list`'s inferred return type) but is imported per the brief's
// verbatim interface -- it documents which type these rows are, and Task 10
// names it directly. Silenced rather than dropped so the import still says
// that, and the global "warnings are findings" rule still holds.
#[allow(unused_imports)]
use dotpkg::backend::winget::{parse_list, WingetRow};

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
    assert!(rows.iter().all(|r| r.source.as_deref() == Some("winget")),
            "Source must not be read out of the missing Available column: {rows:?}");
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
    let chrome = rows.iter().find(|r| r.id == "Google.Chrome").expect("in the fixture");
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
