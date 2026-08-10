//! `parse_show` and `parse_versions`: what `winget show` and
//! `winget show --versions` mean, read as pure text -- never spawning
//! `winget.exe`, matching the standing rule (`tests/winget_scan.rs`'s
//! `fixture` helper is duplicated here rather than shared, for the same
//! reason it is duplicated there: each integration test binary is its own
//! compilation unit).
//!
//! From Phase 4 Task 13 onward this file also carries `Winget::resolve_latest` and
//! `Winget::resolve_installed` -- the two `Backend` trait methods that make
//! `Backend` a real seam rather than decoration. Those tests DO use
//! `FakeWinget` (`tests/common/fake_winget.rs`), so `mod common;` is pulled
//! in below; the plain-text parser tests above it stay exactly as Task 12
//! left them.

mod common;

use common::fake_winget::FakeWinget;
use dotpkg::backend::winget::{
    parse_show, parse_versions, Winget, NO_APPLICATIONS_FOUND, NO_VERSION_FOUND,
};
use dotpkg::backend::{Backend, ResolveCtx};
use dotpkg::lock::Pin;
use dotpkg::model::{Installed, Name, WINGET};
use dotpkg::update::Resolution;

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

// -- Winget::resolve_latest / Winget::resolve_installed (Phase 4 Task 13) ---------
//
// The single check that says whether Phase 4 Task 13 worked lives in
// `src/update.rs`, not here: `update::run` must no longer name
// `bucket::resolve_latest` directly. These tests are what they became
// possible to write once that was true -- winget's own two resolvers,
// exercised through `Backend` with a `FakeWinget` so nothing here ever
// spawns `winget.exe`.

/// An `Installed` for winget, built by hand. `scan`'s own `rows_to_scan`
/// (Task 10) never puts an opaque name or a `"> "` version into `installed`
/// -- so this is the only way `resolve_installed`'s own refusal of one
/// (`an_opaque_package_is_refused_by_adopt_rather_than_pinned` below) is ever
/// reachable at all: a caller with a hand-built `Installed`, not `scan`.
fn installed_winget(id: &str, version: &str) -> Installed {
    Installed {
        backend: WINGET.to_string(),
        name: Name::new(id),
        version: version.to_string(),
        arch: None,
        bucket: None,
        bins: Vec::new(),
    }
}

#[test]
fn resolving_latest_asks_without_exact_and_pins_what_came_back() {
    let fake = FakeWinget::returning(0, fixture("show-canonical-echo.txt"));
    let w = Winget::new(fake.clone());
    let r = w.resolve_latest(&Name::new("git.git"), &ResolveCtx::offline());
    assert_eq!(
        fake.calls(),
        vec![vec!["show", "--id", "git.git", "--disable-interactivity"]],
        "no --exact: it is case-sensitive and would refuse this spelling"
    );
    let Resolution::Resolved { pin } = r else {
        panic!("got {r:?}")
    };
    assert_eq!(
        pin,
        Pin::WingetVersion {
            version: "2.55.0.3".into()
        }
    );
}

/// The positive sibling to both refusals below: an installed version that is
/// STILL in the index is confirmed -- via the exact `-v` probe
/// `show-old-version.txt` was captured from (`PROVENANCE.md`: `show -e --id
/// ajeetdsouza.zoxide -v 0.9.0 …`, minus the `-e` this crate never passes) --
/// and pinned, not merely assumed because it was already on disk.
#[test]
fn a_pin_whose_version_is_still_in_the_index_is_confirmed_and_pinned() {
    let fake = FakeWinget::returning(0, fixture("show-old-version.txt"));
    let w = Winget::new(fake.clone());
    let inst = installed_winget("ajeetdsouza.zoxide", "0.9.0");
    let r = w.resolve_installed(&inst, &ResolveCtx::offline());
    assert_eq!(
        fake.calls(),
        vec![vec![
            "show",
            "--id",
            "ajeetdsouza.zoxide",
            "-v",
            "0.9.0",
            "--disable-interactivity"
        ]]
    );
    let Resolution::Resolved { pin } = r else {
        panic!("got {r:?}")
    };
    assert_eq!(
        pin,
        Pin::WingetVersion {
            version: "0.9.0".into()
        }
    );
}

#[test]
fn a_pin_whose_version_left_the_index_is_refused_and_says_how_deep_the_index_is() {
    let fake = FakeWinget::script(vec![
        (NO_VERSION_FOUND, fixture("show-version-gone.txt")),
        (0, fixture("show-versions-zoxide.txt")),
    ]);
    let w = Winget::new(fake);
    let inst = installed_winget("ajeetdsouza.zoxide", "0.8.0");
    let Resolution::Failed { why } = w.resolve_installed(&inst, &ResolveCtx::offline()) else {
        panic!("0.8.0 is not in the index")
    };
    assert!(why.contains("0.8.0"), "name the version: {why}");
    assert!(
        why.contains("11"),
        "and how many the publisher keeps: {why}"
    );
}

#[test]
fn a_package_that_left_the_index_entirely_is_a_different_message() {
    // 0x8A150014 and 0x8A150017 are distinct codes for distinct facts.
    let fake = FakeWinget::returning(NO_APPLICATIONS_FOUND, fixture("show-package-gone.txt"));
    let w = Winget::new(fake);
    let Resolution::Failed { why } =
        w.resolve_latest(&Name::new("Xyzzy.NoSuch"), &ResolveCtx::offline())
    else {
        panic!("absent package")
    };
    assert!(
        why.contains("no longer") || why.contains("not in"),
        "got {why}"
    );
    assert!(
        !why.contains("version"),
        "this is not a version problem: {why}"
    );
}

/// The same distinct-code principle, on `resolve_installed`'s own path
/// rather than `resolve_latest`'s: `NO_APPLICATIONS_FOUND` here must read as
/// "the package is gone", not "this version is gone" -- the message
/// `NO_VERSION_FOUND` earns in the sibling test above.
#[test]
fn resolve_installed_also_tells_a_gone_package_apart_from_a_gone_version() {
    let fake = FakeWinget::returning(NO_APPLICATIONS_FOUND, fixture("show-package-gone.txt"));
    let w = Winget::new(fake);
    let inst = installed_winget("Xyzzy.NoSuch", "1.0.0");
    let Resolution::Failed { why } = w.resolve_installed(&inst, &ResolveCtx::offline()) else {
        panic!("absent package")
    };
    assert!(
        why.contains("no longer") || why.contains("not in"),
        "got {why}"
    );
}

#[test]
fn an_opaque_package_is_refused_by_adopt_rather_than_pinned() {
    // rows_to_scan never puts an opaque name into `installed`, so
    // resolve_installed cannot be reached for one through scan -- but it is
    // public, and a caller with a hand-built Installed must still be refused.
    let fake = FakeWinget::returning(0, fixture("show-git.txt"));
    let w = Winget::new(fake.clone());
    let inst = installed_winget("Microsoft.VisualStudio.2022.BuildTools", "> 17.14.37");
    let Resolution::Failed { why } = w.resolve_installed(&inst, &ResolveCtx::offline()) else {
        panic!("a version dotpkg cannot vouch for must not be pinned")
    };
    assert!(why.contains("> 17.14.37"), "got {why}");
    // The zero-count pairing for this refusal: "before spawning anything" is
    // only true if nothing was spawned, and `fake.returning(0, ...)` would
    // silently make this pass even if the refusal check were deleted, since
    // then resolve_installed would go on to call the fake and still get a
    // success back. Only this count actually catches that.
    assert_eq!(
        fake.calls().len(),
        0,
        "refused before spawning anything: {:?}",
        fake.calls()
    );
}

#[test]
fn resolve_latest_reports_when_winget_cannot_be_spawned_at_all() {
    let fake = FakeWinget::failing_to_spawn();
    let w = Winget::new(fake);
    let Resolution::Failed { why } =
        w.resolve_latest(&Name::new("Git.Git"), &ResolveCtx::offline())
    else {
        panic!("a spawn failure must refuse")
    };
    assert!(why.contains("PATH") || why.contains("winget"), "{why}");
}

#[test]
fn an_unrecognised_nonzero_exit_is_reported_with_the_code_and_first_line() {
    // Neither of the two named codes this crate trusts -- a generic failure
    // still refuses, and still says something a user can act on rather than
    // silently becoming a resolved pin for the wrong reason.
    let fake = FakeWinget::returning(7, "boom, something else failed\nmore detail\n".to_string());
    let w = Winget::new(fake);
    let Resolution::Failed { why } =
        w.resolve_latest(&Name::new("Some.Pkg"), &ResolveCtx::offline())
    else {
        panic!("nonzero exit must refuse")
    };
    assert!(why.contains('7'), "name the exit code: {why}");
    assert!(
        why.contains("boom, something else failed"),
        "name the first line: {why}"
    );
}
