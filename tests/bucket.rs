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

#[test]
fn history_sees_a_version_that_only_a_merged_branch_ever_had() {
    // Measured section B, and the single reason adopt carries --full-history.
    // Without it a version a user genuinely has installed is unreachable, and
    // adopt would report "not in this bucket" about a commit that is a real
    // ancestor of HEAD.
    let f = Fixture::new();
    let (side_101, _main) = merged_bucket(&f, "main");
    let dir = f.bucket_dir("main");

    let commits = bucket::history(&dir, "bucket/tool.json", "HEAD").unwrap();
    assert!(
        commits.contains(&side_101),
        "the side branch's 1.0.1 commit must be reachable: {commits:?}"
    );

    // The control this test needs to mean anything: the DEFAULT walk cannot
    // see it, so a `history` that quietly dropped the flag would be caught.
    let plain = git(&dir, &["log", "--format=%H", "--", "bucket/tool.json"]);
    assert!(
        !plain.contains(&side_101),
        "if the plain walk also saw it, this fixture stopped reproducing the \
         shape it exists for"
    );
}

#[test]
fn blobs_reads_a_whole_history_in_one_process_and_keeps_the_order() {
    // Measured: 395 processes and 3.16s the naive way, 2 processes and 0.02s
    // this way, identical answer. The count is what transfers to Windows.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let c1 = f.commit(&dir, "tool.json", "1.0.0", "v100");
    let c2 = f.commit(&dir, "tool.json", "1.0.1", "v101");
    let c3 = f.commit(&dir, "tool.json", "1.0.2", "v102");

    let commits = vec![c3.clone(), c2.clone(), c1.clone()];
    let got = bucket::blobs(&dir, &commits, "bucket/tool.json").unwrap();

    assert_eq!(got.len(), 3, "one answer per commit, in order");
    for (i, want) in ["1.0.2", "1.0.1", "1.0.0"].iter().enumerate() {
        let body = got[i]
            .as_ref()
            .unwrap_or_else(|| panic!("blob {i} missing"));
        assert!(
            String::from_utf8_lossy(body).contains(want),
            "position {i} must belong to commit {}: got {}",
            commits[i],
            String::from_utf8_lossy(body)
        );
    }
}

#[test]
fn a_commit_where_the_path_is_absent_yields_none_and_does_not_shift_the_rest() {
    // `git cat-file --batch` answers a missing object with a one-line
    // "<spec> missing" and no body. A parser that assumed every request has a
    // body would consume the NEXT blob's bytes as this one's and mis-attribute
    // every commit after it -- silently, since the bytes still parse as JSON.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let before = f.commit(&dir, "other.json", "0.0.1", "other");
    let after = f.commit(&dir, "tool.json", "1.0.0", "v100");

    let got = bucket::blobs(&dir, &[before.clone(), after.clone()], "bucket/tool.json").unwrap();

    assert_eq!(got.len(), 2);
    assert!(got[0].is_none(), "tool.json did not exist at {before}");
    let body = got[1]
        .as_ref()
        .expect("tool.json exists at the later commit");
    assert!(
        String::from_utf8_lossy(body).contains("1.0.0"),
        "the second answer must not have been shifted: {}",
        String::from_utf8_lossy(body)
    );
}

#[test]
fn blobs_does_not_deadlock_on_a_history_too_large_for_the_pipe_buffer() {
    // CRITICAL, found in review: writing every spec to the child's stdin
    // before ever reading its stdout deadlocks once `git cat-file --batch`'s
    // own unread output fills the OS pipe buffer -- it then blocks on its own
    // write and stops reading stdin, and the writer blocks on stdin in turn.
    // Measured directly against `git cat-file --batch` on this machine with
    // ~1.4 KB bodies (this crate's realistic manifest size): the
    // write-then-read pattern completes at 2250 requests and hangs from 2300
    // on. `history`'s `--full-history` walk can hand `adopt` more commits
    // than that for a bucket with real traffic. This uses 5000, well past
    // that boundary, and was confirmed against the pre-fix implementation to
    // hang and require `timeout`/kill (see the task report).
    let f = Fixture::new();
    let count = 5000;
    let (dir, commits) = deep_history(&f, "main", count);
    assert_eq!(
        commits.len(),
        count,
        "fast-import must have built one commit per version"
    );

    let got = bucket::blobs(&dir, &commits, "bucket/tool.json").unwrap();
    assert_eq!(got.len(), count, "one answer per commit, in order");
    for (body, want_i) in got.iter().zip((1..=count).rev()) {
        let body = body
            .as_ref()
            .unwrap_or_else(|| panic!("every commit in this history carries tool.json"));
        assert!(
            String::from_utf8_lossy(body).contains(&format!("\"1.0.{want_i}\"")),
            "expected version 1.0.{want_i}, got a body that does not contain it"
        );
    }
}

#[test]
fn blobs_errors_rather_than_silently_calling_every_commit_missing() {
    // IMPORTANT, found in review: `blobs` used to never look at the child's
    // exit status. A git failure (not a repo, a corrupt repository) produced
    // empty stdout with no diagnostic, and the parser read that as "every
    // commit is missing" -- returning `Ok(vec![None; n])` instead of an
    // error. Confirmed against the pre-fix implementation: this exact
    // scenario returned `Ok([None])` there; it must return `Err` here.
    let f = Fixture::new();
    let not_a_repo = f.home.path().join("not-a-repo");
    std::fs::create_dir_all(&not_a_repo).unwrap();

    let err = bucket::blobs(
        &not_a_repo,
        &["deadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string()],
        "bucket/tool.json",
    )
    .unwrap_err();
    // Either message is a pass, and pinning one of them made this test
    // INTERMITTENT -- measured, not supposed: it failed once on
    // `ubuntu-latest` with `cannot feed git cat-file in …: Broken pipe (os
    // error 32)` while passing on the commit before and the commit after,
    // with only documentation between them.
    //
    // `blobs` can report this failure by two routes and which one wins is a
    // race. `git cat-file --batch` in a directory that is not a repository
    // exits immediately, so either the writer thread finishes feeding stdin
    // first and the exit-status check reports `git cat-file failed`, or git
    // is already gone and the write gets EPIPE, reported as `cannot feed`.
    // On a loaded CI runner the child wins more often.
    //
    // What this test is FOR is the line above: `unwrap_err()`. The defect it
    // guards against is `blobs` returning `Ok(vec![None; n])` -- "every commit
    // is missing" -- for a git failure it never looked at. Both messages are
    // that failure being reported. Asserting one of them was asserting
    // something narrower than the test means, which is how a test starts
    // failing for a reason unrelated to what it claims.
    let text = format!("{err:#}");
    assert!(
        text.contains("cat-file") && text.contains("not-a-repo"),
        "the failure must be reported, and must name what failed and where, \
         not be swallowed as every commit missing: {text}"
    );
}

#[test]
fn blobs_returns_bytes_not_a_string_because_line_endings_are_the_evidence() {
    // adopt compares these against an installed manifest under
    // verify::normalise. A String round trip through lossy UTF-8 would be
    // lossless for JSON but the signature would invite someone to trim.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let c = f.commit(&dir, "tool.json", "1.0.0", "v100");
    let got = bucket::blobs(&dir, std::slice::from_ref(&c), "bucket/tool.json").unwrap();
    let body = got[0].as_ref().unwrap();
    assert_eq!(
        body.as_slice(),
        f.blob(&dir, &c, "tool.json").as_bytes(),
        "the blob must come back byte for byte, trailing newline included"
    );
}

use dotpkg::bucket::BucketChoice;

fn cfg(text: &str) -> dotpkg::config::Config {
    dotpkg::config::parse(text).unwrap()
}

#[test]
fn a_package_in_exactly_one_declared_bucket_is_chosen_without_asking() {
    let f = Fixture::new();
    let main = f.bucket("main");
    f.commit(&main, "tool.json", "1.0.0", "v100");
    let extras = f.bucket("extras");
    f.commit(&extras, "other.json", "1.0.0", "v100");

    let choice = bucket::choose_bucket(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n"),
        &Name::new("tool"),
        None,
    );
    match choice {
        BucketChoice::Chosen { name, .. } => assert_eq!(name, Name::new("main")),
        other => panic!("expected a clean choice, got {other:?}"),
    }
}

#[test]
fn a_package_in_two_declared_buckets_refuses_and_names_both() {
    // Never a guess based on declaration order: reordering `buckets` would
    // silently move a pin.
    let f = Fixture::new();
    for b in ["main", "extras"] {
        let dir = f.bucket(b);
        f.commit(&dir, "tool.json", "1.0.0", "v100");
    }

    let choice = bucket::choose_bucket(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n"),
        &Name::new("tool"),
        None,
    );
    match choice {
        BucketChoice::Ambiguous { candidates } => {
            assert!(candidates.contains(&Name::new("main")), "{candidates:?}");
            assert!(candidates.contains(&Name::new("extras")), "{candidates:?}");
        }
        other => panic!("two buckets have it; guessing is forbidden: {other:?}"),
    }
}

#[test]
fn scoop_opts_bucket_settles_an_ambiguity() {
    let f = Fixture::new();
    for b in ["main", "extras"] {
        let dir = f.bucket(b);
        f.commit(&dir, "tool.json", "1.0.0", "v100");
    }

    let choice = bucket::choose_bucket(
        &f.scoop_root(),
        &cfg(
            "[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n\
              [scoop.opts]\ntool = { bucket = \"extras\" }\n",
        ),
        &Name::new("tool"),
        None,
    );
    match choice {
        BucketChoice::Chosen { name, .. } => assert_eq!(name, Name::new("extras")),
        other => panic!("the opt must settle it: {other:?}"),
    }
}

#[test]
fn an_existing_lock_entry_pins_the_bucket_so_update_never_moves_a_package() {
    // update re-resolves a version, not a provenance. A package silently
    // migrating from extras to main would change its url and hash with no line
    // in the diff saying so.
    let f = Fixture::new();
    for b in ["main", "extras"] {
        let dir = f.bucket(b);
        f.commit(&dir, "tool.json", "1.0.0", "v100");
    }

    let choice = bucket::choose_bucket(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n"),
        &Name::new("tool"),
        Some("extras"),
    );
    match choice {
        BucketChoice::Chosen { name, .. } => assert_eq!(name, Name::new("extras")),
        other => panic!("the lock's bucket wins: {other:?}"),
    }
}

#[test]
fn a_package_no_declared_bucket_has_names_what_was_searched() {
    let f = Fixture::new();
    f.bucket("main");
    let choice = bucket::choose_bucket(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n"),
        &Name::new("tool"),
        None,
    );
    match choice {
        BucketChoice::NotFound { searched, missing } => {
            assert_eq!(searched, vec![Name::new("main")]);
            assert!(
                missing.is_empty(),
                "every declared bucket is on disk here: {missing:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_declared_bucket_that_is_not_on_disk_is_reported_as_missing_not_as_searched() {
    // The whole-branch review's CRITICAL. `main` is on disk and does not carry
    // `tool`; `extras` is declared and was never cloned. The search loop used
    // to `continue` past `extras` in silence and then report `searched: main,
    // extras` -- naming a bucket it never opened. On a fresh machine, or on a
    // pkg.toml that just grew a bucket line, that is what EVERY declared
    // package gets, and nothing anywhere says an uncloned bucket is the cause.
    //
    // Note that `a_package_no_declared_bucket_has_names_what_was_searched`
    // above declares `main` and creates `main`, so no test before this one
    // could see any of it.
    let f = Fixture::new();
    let main = f.bucket("main");
    f.commit(&main, "other.json", "1.0.0", "v100");

    let choice = bucket::choose_bucket(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n"),
        &Name::new("tool"),
        None,
    );
    match choice {
        BucketChoice::NotFound { searched, missing } => {
            assert_eq!(
                searched,
                vec![Name::new("main")],
                "only the bucket that is actually on this machine was searched"
            );
            assert_eq!(
                missing,
                vec![Name::new("extras")],
                "a declared bucket that is not on this machine must be named as \
                 missing rather than folded into the searched list"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_bucket_named_by_the_lock_or_by_an_opt_that_is_not_on_disk_is_reported_as_absent() {
    // The same defect on the OTHER branch of `choose_bucket`. The `stated`
    // branch had no `.git` check at all, unlike the search loop four lines
    // below it, so an absent bucket was opened as if it were there: `tip()`
    // fell to its `_` arm, every `git_show` failed, `resolve_latest` returned
    // `Ok(None)`, and `update` rendered that as "bucket extras has no manifest
    // for it" -- a flat lie about a bucket that has the manifest and is simply
    // not cloned.
    let f = Fixture::new();
    let main = f.bucket("main");
    f.commit(&main, "tool.json", "1.0.0", "v100");
    let config = cfg("[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n");

    // Named by the lock.
    match bucket::choose_bucket(&f.scoop_root(), &config, &Name::new("tool"), Some("extras")) {
        BucketChoice::NotCloned { name, dir } => {
            assert_eq!(name, Name::new("extras"));
            assert_eq!(dir, f.scoop_root().join("buckets").join("extras"));
        }
        other => panic!(
            "a bucket that is not on this machine must not be opened as though it were: {other:?}"
        ),
    }

    // Named by `[scoop.opts]`, which reaches the same branch by the other
    // route. Without this the fix could be made on the lock path alone.
    match bucket::choose_bucket(
        &f.scoop_root(),
        &cfg(
            "[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n\
              [scoop.opts]\ntool = { bucket = \"extras\" }\n",
        ),
        &Name::new("tool"),
        None,
    ) {
        BucketChoice::NotCloned { name, .. } => assert_eq!(name, Name::new("extras")),
        other => panic!("{other:?}"),
    }

    // The counterweight: the same stated bucket, present on disk, must still
    // be chosen. Without this a `.git` check that always failed would satisfy
    // both assertions above.
    let extras = f.bucket("extras");
    f.commit(&extras, "tool.json", "2.0.0", "v200");
    match bucket::choose_bucket(&f.scoop_root(), &config, &Name::new("tool"), Some("extras")) {
        BucketChoice::Chosen { name, .. } => assert_eq!(name, Name::new("extras")),
        other => panic!("a bucket that IS on this machine must still be opened: {other:?}"),
    }
}

#[test]
fn a_bucket_named_by_the_lock_or_by_an_opt_that_pkg_toml_does_not_declare_is_reported_as_undeclared(
) {
    // The fourth exit of `choose_bucket`, and the one the previous fix wave
    // routed through `NotFound` without addressing: `extras` here is not
    // merely absent from disk (that is `NotCloned`, above) -- it is not in
    // `[scoop] buckets` at all. Before this fix the `stated` branch returned
    // `NotFound { searched: vec![extras], missing: [] }`, which
    // `not_found_why`'s `(false, true)` arm rendered as `no declared bucket
    // has <pkg> (searched: extras)` -- naming a bucket that was neither
    // declared nor searched. `apply` already tells this state apart
    // (`src/apply.rs`'s `bucket_is_declared` check, "bucket \"extras\" is not
    // declared in pkg.toml -- add it to [scoop] buckets"); `choose_bucket`
    // did not.
    //
    // `extras` is cloned on disk here -- proving the branch fires on
    // declaration, not presence. If it were also absent from disk, a test
    // could not tell `Undeclared` apart from `NotCloned`.
    let f = Fixture::new();
    let main = f.bucket("main");
    f.commit(&main, "tool.json", "1.0.0", "v100");
    let extras = f.bucket("extras");
    f.commit(&extras, "tool.json", "2.0.0", "v200");
    let config = cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n");

    // Named by the lock.
    match bucket::choose_bucket(&f.scoop_root(), &config, &Name::new("tool"), Some("extras")) {
        BucketChoice::Undeclared { name } => assert_eq!(name, Name::new("extras")),
        other => panic!(
            "a bucket pkg.toml does not declare must not be reported as searched or \
             not-cloned: {other:?}"
        ),
    }

    // Named by `[scoop.opts]`, the other route into the same branch. Without
    // this the fix could be made on the lock path alone.
    match bucket::choose_bucket(
        &f.scoop_root(),
        &cfg("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n\
              [scoop.opts]\ntool = { bucket = \"extras\" }\n"),
        &Name::new("tool"),
        None,
    ) {
        BucketChoice::Undeclared { name } => assert_eq!(name, Name::new("extras")),
        other => panic!("{other:?}"),
    }
}
