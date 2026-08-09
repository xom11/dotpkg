mod common;

use common::*;
use dotpkg::adopt::{self, Matched};
use dotpkg::model::Name;

/// The installed manifest, as scoop leaves it: the bucket's bytes with CRLF.
fn as_scoop_installs_it(body: &str) -> Vec<u8> {
    body.replace('\n', "\r\n").into_bytes()
}

#[test]
fn the_installed_bytes_pick_the_right_commit_when_two_carry_one_version() {
    // Measured section C, and the reason adopt is strictly better than the
    // Phase 2b-1 rehearsal script it replaces. That script matched on version
    // and would pin this machine to the NEWER commit -- content it is not
    // running.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let older = f.commit(&dir, "tool.json", "2.0.0", "good");
    let newer = f.commit(&dir, "tool.json", "2.0.0", "amended");
    assert_ne!(older, newer);

    let installed = as_scoop_installs_it(&f.blob(&dir, &older, "tool.json"));
    let found = adopt::resolve_installed(&dir, &Name::new("tool"), "2.0.0", &installed, "HEAD")
        .unwrap()
        .expect("2.0.0 is in this history twice");

    assert_eq!(
        found.commit, older,
        "the commit whose content is actually installed"
    );
    assert_eq!(found.matched, Matched::Content);
}

#[test]
fn a_manifest_scoop_rewrote_still_matches_because_normalise_is_used() {
    // The control for the test above: without normalise the comparison finds
    // nothing and the fallback silently picks the newer commit instead.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let older = f.commit(&dir, "tool.json", "2.0.0", "good");
    f.commit(&dir, "tool.json", "2.0.0", "amended");

    let raw = f.blob(&dir, &older, "tool.json");
    assert!(
        raw.contains('\n') && !raw.contains("\r\n"),
        "the blob is LF"
    );
    let installed = as_scoop_installs_it(&raw);
    assert!(
        String::from_utf8_lossy(&installed).contains("\r\n"),
        "the fixture must actually differ from the blob"
    );

    let found = adopt::resolve_installed(&dir, &Name::new("tool"), "2.0.0", &installed, "HEAD")
        .unwrap()
        .unwrap();
    assert_eq!(found.matched, Matched::Content);
    assert_eq!(found.commit, older);
}

#[test]
fn a_manifest_that_matches_nothing_byte_for_byte_falls_back_to_the_version() {
    // A machine whose manifest was rewritten by something other than line
    // endings -- an older scoop, a hand edit. The version is a weaker answer
    // and it is recorded as such rather than presented as exact.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let c = f.commit(&dir, "tool.json", "3.1.0", "v310");

    let found = adopt::resolve_installed(
        &dir,
        &Name::new("tool"),
        "3.1.0",
        br#"{"version":"3.1.0","note":"rewritten by something else"}"#,
        "HEAD",
    )
    .unwrap()
    .unwrap();
    assert_eq!(found.commit, c);
    assert_eq!(found.matched, Matched::Version);
}

#[test]
fn adopt_finds_a_version_that_only_a_merged_branch_ever_had() {
    // Measured section B. Without --full-history this is unreachable and adopt
    // would refuse a package the user genuinely has installed.
    let f = Fixture::new();
    let (side_101, _main) = merged_bucket(&f, "main");
    let dir = f.bucket_dir("main");

    let found = adopt::resolve_installed(
        &dir,
        &Name::new("tool"),
        "1.0.1",
        br#"{"version":"1.0.1"}"#,
        "HEAD",
    )
    .unwrap()
    .expect("1.0.1 is an ancestor of HEAD even though the plain walk hides it");
    assert_eq!(found.commit, side_101);
    assert_eq!(found.matched, Matched::Version, "the byte-for-byte manifest passed here has no matching blob -- only the version can have answered");
}

#[test]
fn found_version_comes_from_the_matched_blob_not_the_callers_string() {
    // Found.version feeds the lock entry directly, and Scoop::stage refuses a
    // pin whose version disagrees with the blob at that commit -- so a wrong
    // value here is a lock that looks fine and fails at apply time. A Content
    // match must report the BLOB's version, never an echo of whatever string
    // the caller happened to pass in for lookup.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let c = f.commit(&dir, "tool.json", "2.0.0", "good");

    let installed = as_scoop_installs_it(&f.blob(&dir, &c, "tool.json"));
    // Deliberately not "2.0.0": if the matcher ever regresses to echoing the
    // caller's string, this is what makes it visible.
    let found = adopt::resolve_installed(&dir, &Name::new("tool"), "9.9.9", &installed, "HEAD")
        .unwrap()
        .expect("the bytes match a commit even though the caller's version string is wrong");
    assert_eq!(found.matched, Matched::Content);
    assert_eq!(
        found.version, "2.0.0",
        "the blob's version, not the caller's \"9.9.9\""
    );
}

#[test]
fn a_matched_blob_with_no_version_field_falls_back_to_the_callers_string() {
    // The unwrap_or_else branch, previously uncovered: a content match whose
    // blob has no parseable "version" field still has to report SOME
    // version, and the caller's string is the only one left to use.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let body = "{\n    \"url\": \"https://example.invalid/no-version.zip\"\n}\n";
    std::fs::write(dir.join("bucket").join("tool.json"), body).unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "tool.json, no version field"]);
    let c = git(&dir, &["rev-parse", "HEAD"]).trim().to_string();

    let installed = as_scoop_installs_it(body);
    let found = adopt::resolve_installed(&dir, &Name::new("tool"), "2.0.0", &installed, "HEAD")
        .unwrap()
        .expect("content matches even though the blob has no version field");
    assert_eq!(found.matched, Matched::Content);
    assert_eq!(found.commit, c);
    assert_eq!(
        found.version, "2.0.0",
        "falls back to the caller's version string when the blob has none"
    );
}

#[test]
fn a_version_no_commit_carries_resolves_to_none() {
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");

    assert_eq!(
        adopt::resolve_installed(&dir, &Name::new("tool"), "9.9.9", b"{}", "HEAD").unwrap(),
        None
    );
}

#[test]
fn an_app_the_bucket_has_never_had_resolves_to_none() {
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");

    assert_eq!(
        adopt::resolve_installed(&dir, &Name::new("nosuch"), "1.0.0", b"{}", "HEAD").unwrap(),
        None
    );
}
