//! `parse_show` and `parse_versions`: what `winget show` and
//! `winget show --versions` mean, read as pure text -- never spawning
//! `winget.exe`, matching the standing rule (`tests/winget_scan.rs`'s
//! `fixture` helper is duplicated here rather than shared, for the same
//! reason it is duplicated there: each integration test binary is its own
//! compilation unit).

use dotpkg::backend::winget::{parse_show, parse_versions};

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
fn show_yields_the_canonical_id_even_when_asked_in_the_wrong_case() {
    // MEASURED: `--exact` is what makes `--id` case-sensitive.
    //   show -e --id git.git  -> 0x8A150014, "No package found"
    //   show    --id git.git  -> 0,          "Found Git [Git.Git]"
    // src/model.rs's "scoop and winget both resolve names case-insensitively"
    // is false for --exact, and Name folds case -- so dotpkg can hold a name
    // that compares equal to the right package and is unusable against winget.
    // Asking without --exact and recording what came back is the fix.
    let f = parse_show(&fixture("show-canonical-echo.txt")).unwrap();
    assert_eq!(
        f.id, "Git.Git",
        "the canonical spelling, not the one we asked with"
    );
    assert_eq!(f.version, "2.55.0.3");
}

#[test]
fn show_of_the_canonical_spelling_gives_the_same_answer() {
    // The positive sibling: both fixtures are 1550 bytes for a reason.
    let a = parse_show(&fixture("show-git.txt")).unwrap();
    let b = parse_show(&fixture("show-canonical-echo.txt")).unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(a.version, b.version);
}

#[test]
fn a_not_found_body_is_refused_rather_than_parsed_into_an_empty_found() {
    let r = parse_show(&fixture("show-package-gone.txt"));
    assert!(
        r.is_err(),
        "an empty Found would be a package named \"\" at version \"\""
    );
}

#[test]
fn versions_come_back_newest_first_and_the_retention_depth_is_countable() {
    // ripgrep.MSVC keeps 8; zoxide keeps 11; OhMyPosh keeps 828. Retention is
    // a publisher policy, not a winget guarantee, so `update` can say how deep
    // the index is when a pin falls off the end.
    let (id, vs) = parse_versions(&fixture("show-versions-ripgrep.txt")).unwrap();
    assert_eq!(id, "BurntSushi.ripgrep.MSVC");
    assert_eq!(vs.len(), 8);
    assert_eq!(vs[0], "15.2.0", "row 1 is what `show` calls Version:");

    let (_, zs) = parse_versions(&fixture("show-versions-zoxide.txt")).unwrap();
    assert_eq!(zs.len(), 11);
    assert_eq!(zs.first().map(String::as_str), Some("0.10.0"));
    assert_eq!(zs.last().map(String::as_str), Some("0.9.0"));
}

// The brief's draft of this test (`show_and_show_versions_agree_on_the_newest`)
// compared `show-git.txt`'s Version: line against `show-versions-zoxide.txt`'s
// row 0 -- two DIFFERENT packages (Git.Git vs ajeetdsouza.zoxide). It never
// actually compared the two values to each other; it just re-asserted each
// fixture's already-known value in isolation (both already covered above and
// in `show_yields_the_canonical_id_even_when_asked_in_the_wrong_case`). A
// parser that read the wrong line in either function would leave every one of
// those four assertions green, so the test did not assert what its name
// claimed, and it is being replaced rather than transcribed verbatim.
//
// No fixture captures a plain `show` of zoxide's CURRENT version (only
// `show-old-version.txt`, which pins the historical 0.9.0), so the "agree on
// the NEWEST" measurement in `docs/measurements-2026-08-09-winget.md` section
// 6 (six packages, `show`'s Version: == `--versions` row 1 on all six) cannot
// be replayed from the 15 checked-in fixtures -- it is recorded there as a
// live measurement, not as a fixture pair. What CAN be replayed is the one
// pair of fixtures that share a package under a version `show --versions`
// also lists: `show-old-version.txt` (`show -v 0.9.0`) and
// `show-versions-zoxide.txt`, where 0.9.0 is the OLDEST retained version
// (`vs.last()`), not the newest. That is a real cross-check between the two
// parsers on the same package, and it doubles as the "pins what was asked,
// not the newest" fact the brief's version wanted to record.
#[test]
fn parse_show_and_parse_versions_agree_on_a_version_string_for_the_same_package() {
    let (id, vs) = parse_versions(&fixture("show-versions-zoxide.txt")).unwrap();
    let pinned = parse_show(&fixture("show-old-version.txt")).unwrap();

    assert_eq!(pinned.id, id, "both calls must resolve to the same package");
    assert_eq!(
        pinned.version,
        vs.last().unwrap().as_str(),
        "show -v 0.9.0 must name the same version show --versions lists last -- the actual \
         cross-check the two parsers owe each other on one package"
    );
    assert_eq!(
        pinned.version, "0.9.0",
        "show -v pins the version that was ASKED for, not the newest"
    );
    assert_ne!(
        pinned.version, vs[0],
        "0.9.0 is not the newest (0.10.0) -- a versioned pin must not silently read back as latest"
    );
}
