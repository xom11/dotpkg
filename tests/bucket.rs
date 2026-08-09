mod common;

use common::*;
use dotpkg::bucket;

#[test]
fn the_tip_is_the_upstream_ref_when_there_is_one() {
    // `update` resolves against a remote-tracking ref so that a fetch is
    // visible without moving the branch scoop owns.
    let f = Fixture::new();
    let upstream = f.bucket("upstream");
    f.commit(&upstream, "tool.json", "1.0.0", "v100");

    let clone_dir = f.scoop_root().join("buckets").join("main");
    git(
        f.home.path(),
        &[
            "clone",
            "-q",
            &format!("file://{}", upstream.display()),
            &clone_dir.to_string_lossy(),
        ],
    );

    let tip = bucket::tip(&clone_dir);
    assert!(
        tip.rev.starts_with("origin/"),
        "a cloned bucket has an upstream and must resolve against it: {tip:?}"
    );
    assert_eq!(tip.stale, None, "an upstream ref is not stale");
}

#[test]
fn a_bucket_with_no_upstream_falls_back_to_head_and_says_why() {
    // A bucket created locally, or one whose remote was removed. Resolving is
    // still possible; claiming it is "latest" is not.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");

    let tip = bucket::tip(&dir);
    assert_eq!(tip.rev, "HEAD");
    let why = tip
        .stale
        .expect("falling back must be explained, not silent");
    assert!(why.contains("upstream"), "name what is missing: {why}");
}

#[test]
fn a_shallow_clone_is_detected() {
    // Measured: adopt's walk on a shallow clone finds nothing and git says
    // nothing about why, which is indistinguishable from "this version was
    // never in this bucket".
    let f = Fixture::new();
    let upstream = f.bucket("upstream");
    f.commit(&upstream, "tool.json", "1.0.0", "v100");
    f.commit(&upstream, "tool.json", "1.0.1", "v101");

    let full = f.home.path().join("full");
    let shallow = f.home.path().join("shallow");
    let url = format!("file://{}", upstream.display());
    git(
        f.home.path(),
        &["clone", "-q", &url, &full.to_string_lossy()],
    );
    git(
        f.home.path(),
        &[
            "clone",
            "-q",
            "--depth",
            "1",
            &url,
            &shallow.to_string_lossy(),
        ],
    );

    assert!(bucket::is_shallow(&shallow), "a --depth 1 clone is shallow");
    assert!(
        !bucket::is_shallow(&full),
        "a full clone must not be reported as shallow -- otherwise every adopt \
         failure blames shallowness"
    );
}

use dotpkg::model::Name;

#[test]
fn latest_is_the_per_file_commit_not_the_bucket_tip() {
    // Measured section A. The whole reason pkg.lock records a commit per
    // package rather than one commit per bucket.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "fzf.json", "1.0.0", "v100");
    let want = f.commit(&dir, "fzf.json", "1.0.2", "v102");
    f.commit(&dir, "bat.json", "9.9.9", "bat");
    let tip_sha = git(&dir, &["rev-parse", "HEAD"]).trim().to_string();

    let got = bucket::resolve_latest(&dir, &Name::new("fzf"), "HEAD")
        .unwrap()
        .expect("fzf is in this bucket");

    assert_eq!(got.commit, want, "the commit that last touched fzf.json");
    assert_ne!(got.commit, tip_sha, "not the bucket tip -- bat moved it on");
    assert_eq!(got.version, "1.0.2");
    assert_eq!(got.path_in_repo, "bucket/fzf.json");
    assert!(!got.fell_back_to_tip);
}

#[test]
fn latest_does_not_name_a_merge_commit() {
    // Measured section B'. Under --full-history this returns the MERGE, whose
    // blob is identical but which is not the commit that produced the version.
    // update must not have that flag; adopt must.
    let f = Fixture::new();
    let (_side, main_102) = merged_bucket(&f, "main");
    let dir = f.bucket_dir("main");

    let got = bucket::resolve_latest(&dir, &Name::new("tool"), "HEAD")
        .unwrap()
        .unwrap();

    assert_eq!(got.version, "1.0.2");
    assert_eq!(
        got.commit, main_102,
        "the commit that made 1.0.2, not the merge that carried it"
    );
}

#[test]
fn the_recorded_commit_carries_the_tips_content_for_every_shape_measured() {
    // The design promise stated directly: whatever commit resolve_latest
    // names, that commit's blob for the resolved path must equal the tip's.
    // Checked against every shape the measurements produced, rather than
    // trusting one clone type to stand in for all of them -- a prior version
    // of this test asserted the property only on a shallow clone, where a
    // parentless boundary commit makes `git log -1` return the tip trivially
    // regardless of whether the self-check runs at all. See the commit
    // message for the analysis and the negative control that replaces it.
    let assert_property = |dir: &std::path::Path, app_name: &str, rev: &str| {
        let got = bucket::resolve_latest(dir, &Name::new(app_name), rev)
            .unwrap()
            .unwrap_or_else(|| panic!("{app_name} must be in this bucket"));
        assert_eq!(
            git(
                dir,
                &["show", &format!("{}:{}", got.commit, got.path_in_repo)]
            ),
            git(dir, &["show", &format!("{rev}:{}", got.path_in_repo)]),
            "resolved commit's blob must equal {rev}'s for {app_name}"
        );
    };

    // Linear history: several commits touch the same file.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "fzf.json", "1.0.0", "v100");
    f.commit(&dir, "fzf.json", "1.0.2", "v102");
    assert_property(&dir, "fzf", "HEAD");

    // A merge that carried an older side-branch version forward.
    let f = Fixture::new();
    merged_bucket(&f, "main");
    let dir = f.bucket_dir("main");
    assert_property(&dir, "tool", "HEAD");

    // A shallow clone: exactly one commit, and it is the tip.
    let f = Fixture::new();
    let upstream = f.bucket("upstream");
    f.commit(&upstream, "tool.json", "1.0.0", "v100");
    f.commit(&upstream, "tool.json", "1.0.1", "v101");
    let shallow = f.home.path().join("shallow");
    git(
        f.home.path(),
        &[
            "clone",
            "-q",
            "--depth",
            "1",
            &format!("file://{}", upstream.display()),
            &shallow.to_string_lossy(),
        ],
    );
    assert_property(&shallow, "tool", "HEAD");

    // Delete then re-add: the file's history has a gap rather than a
    // continuous rename, built inline since no existing fixture has this shape.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");
    std::fs::remove_file(dir.join("bucket").join("tool.json")).unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "remove tool.json"]);
    f.commit(&dir, "tool.json", "1.0.1", "v101");
    assert_property(&dir, "tool", "HEAD");
}

#[test]
fn an_app_the_bucket_does_not_have_resolves_to_none_rather_than_erroring() {
    // "not in this bucket" is an ordinary answer during a bucket search, not a
    // failure: update tries every declared bucket before giving up.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "tool.json", "1.0.0", "v100");

    assert_eq!(
        bucket::resolve_latest(&dir, &Name::new("nosuch"), "HEAD").unwrap(),
        None
    );
}

#[test]
fn latest_finds_a_manifest_the_bucket_spells_with_different_case() {
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "MixedCase.json", "1.0.0", "v100");

    let got = bucket::resolve_latest(&dir, &Name::new("MIXEDCASE"), "HEAD")
        .unwrap()
        .unwrap();
    assert_eq!(got.path_in_repo, "bucket/MixedCase.json");
}

#[test]
fn the_case_rename_fixture_really_produces_two_spellings() {
    // Task 3 review: nothing checked in currently asserts that
    // `case_renamed_bucket` actually produces two spellings. Tasks 4, 5, 11
    // and 12 build on it, so its own behaviour needs one direct witness.
    let f = Fixture::new();
    let (old_commit, new_commit) = case_renamed_bucket(&f, "main");
    let dir = f.bucket_dir("main");

    let old_listing = git(&dir, &["ls-tree", "--name-only", "-r", &old_commit]);
    assert_eq!(
        old_listing.trim(),
        "bucket/Tool.json",
        "the older commit must spell the file with an uppercase T"
    );

    let head_listing = git(&dir, &["ls-tree", "--name-only", "-r", "HEAD"]);
    assert_eq!(
        head_listing.trim(),
        "bucket/tool.json",
        "HEAD must spell the file with a lowercase t"
    );
    assert_eq!(
        git(&dir, &["rev-parse", "HEAD"]).trim(),
        new_commit,
        "HEAD must be the second commit `case_renamed_bucket` built"
    );
}
