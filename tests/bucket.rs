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
