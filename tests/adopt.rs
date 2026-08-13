mod common;

use common::fake_winget::FakeWinget;
use common::*;
use dotpkg::adopt::{self, Matched};
use dotpkg::backend::winget::Winget;
use dotpkg::model::{Name, SCOOP, WINGET};

/// `adopt::run` dispatches on a backend name and needs a `Winget<C>`
/// instance even for a scoop-only call (see its own doc comment). Every
/// test in this file predates winget adoption and exercises the scoop path
/// only, so this wraps the new 7-argument signature back down to the old
/// 5-argument shape every call site below already uses -- `FakeWinget::
/// unreachable()` turns "the winget path was somehow reached from a
/// scoop-backend call" into a loud panic rather than a silent pass.
fn run_scoop(
    scoop_root: &std::path::Path,
    names: &[Name],
    config_path: &std::path::Path,
    lock_path: &std::path::Path,
    state_path: &std::path::Path,
) -> anyhow::Result<adopt::Outcome> {
    let winget = Winget::new(FakeWinget::unreachable());
    adopt::run(
        scoop_root,
        &winget,
        SCOOP,
        names,
        config_path,
        lock_path,
        state_path,
    )
}

/// The winget twin of `run_scoop`, for the tests below Task 15's review
/// found missing entirely (`tests/adopt.rs` had exactly one winget test, a
/// refusal, and nothing exercising the write path at all). `scoop_root` is
/// still required by `adopt::run`'s dispatcher even though the winget
/// branch never reads it -- a tempdir stands in.
fn run_winget(
    winget: Winget<FakeWinget>,
    names: &[Name],
    config_path: &std::path::Path,
    lock_path: &std::path::Path,
    state_path: &std::path::Path,
) -> anyhow::Result<adopt::Outcome> {
    let unused_scoop_root = tempfile::tempdir().unwrap();
    adopt::run(
        unused_scoop_root.path(),
        &winget,
        WINGET,
        names,
        config_path,
        lock_path,
        state_path,
    )
}

/// Read a winget fixture, keeping the CRLF it was captured with -- see
/// `tests/winget_resolve.rs`'s identical helper for why.
fn winget_fixture(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/winget")
            .join(name),
    )
    .unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

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

use dotpkg::state::{Ownership, State};

/// The three-file write, and the property that every prefix of it is inert.
#[test]
fn adopt_writes_the_lock_then_pkg_toml_then_state_and_each_prefix_is_safe() {
    let f = Fixture::new();
    let dir = f.bucket("main");
    let c = f.commit(&dir, "aichat.json", "0.30.0", "v030");

    let config_path = f.home.path().join("pkg.toml");
    let lock_path = f.home.path().join("pkg.lock");
    let state_path = f.home.path().join("state.json");
    std::fs::write(
        &config_path,
        "# hand written\n[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
    )
    .unwrap();

    // An installed, unowned aichat.
    let cur = f.scoop_root().join("apps").join("aichat").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(
        cur.join("manifest.json"),
        f.blob(&dir, "HEAD", "aichat.json"),
    )
    .unwrap();

    let out = run_scoop(
        &f.scoop_root(),
        &[Name::new("aichat")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();
    assert_eq!(out.adopted.len(), 1, "{out:?}");
    assert_eq!(
        out.adopted[0],
        (Name::new("aichat"), Matched::Content, None),
        "the installed manifest is the bucket's own bytes: the matched rule \
         reported to the caller must say so, not merely say 'adopted'; and \
         there was no previous pin to replace"
    );

    // All three files, and only the intended change in each. Not just
    // "an entry exists" -- the exact bucket/commit/version dotpkg's next
    // `apply` will stage from.
    let lock = dotpkg::lock::load_or_empty(&lock_path).unwrap();
    assert_eq!(
        lock.scoop[&Name::new("aichat")],
        dotpkg::lock::Pin::ScoopCommit {
            bucket: "main".to_string(),
            commit: c,
            version: "0.30.0".to_string(),
        },
        "the exact pin `apply` will later stage from"
    );

    let cfg_text = std::fs::read_to_string(&config_path).unwrap();
    assert!(
        cfg_text.contains("# hand written"),
        "comments survive: {cfg_text}"
    );
    let cfg = dotpkg::config::parse(&cfg_text).unwrap();
    assert!(cfg.scoop.packages.contains(&Name::new("aichat")));
    assert!(cfg.scoop.packages.contains(&Name::new("fzf")));

    let state = State::load_or_empty(&state_path).unwrap();
    assert_eq!(
        state.ownership(dotpkg::model::SCOOP, &Name::new("aichat")),
        Some(Ownership::Adopted),
        "adopt is the first writer of this variant"
    );
}

/// The reachable sequence the audit named: hand-write `pkg.toml` for what is
/// installed, run `update` (which pins the newest commit `pkg.lock` has ever
/// seen), then `adopt` to hold what is actually on disk instead. `adopt_one`
/// refuses when a package is not installed or is already owned, but not when
/// `pkg.lock` already carries a pin for it -- so this replaces a committed
/// pin, and both halves of that must be visible: the new pin must be the one
/// that is actually installed, and the outcome must say a pin was replaced
/// and what it was.
#[test]
fn adopting_over_an_existing_unowned_pin_replaces_it_and_reports_the_previous_version() {
    let f = Fixture::new();
    let dir = f.bucket("main");
    let installed_commit = f.commit(&dir, "fzf.json", "0.74.1", "installed");
    // The commit `update` would have resolved as newest -- not what is
    // actually installed.
    let latest_commit = f.commit(&dir, "fzf.json", "0.74.2", "latest");
    assert_ne!(installed_commit, latest_commit);

    let config_path = f.home.path().join("pkg.toml");
    let lock_path = f.home.path().join("pkg.lock");
    let state_path = f.home.path().join("state.json");
    std::fs::write(
        &config_path,
        "[scoop]\nbuckets = [\"main\"]\npackages = [\"fzf\"]\n",
    )
    .unwrap();
    let config_before = std::fs::read_to_string(&config_path).unwrap();

    // `update`'s own write: fzf pinned to the newer commit, before `adopt`
    // ever runs. No state.json entry -- `update` never claims ownership.
    let mut stale_lock = dotpkg::lock::Lock::default();
    stale_lock.scoop.insert(
        Name::new("fzf"),
        dotpkg::lock::Pin::ScoopCommit {
            bucket: "main".to_string(),
            commit: latest_commit,
            version: "0.74.2".to_string(),
        },
    );
    dotpkg::lock::save(&stale_lock, &lock_path).unwrap();

    // What is actually installed: the OLDER commit's manifest, byte for
    // byte.
    let cur = f.scoop_root().join("apps").join("fzf").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(
        cur.join("manifest.json"),
        f.blob(&dir, &installed_commit, "fzf.json"),
    )
    .unwrap();

    let out = run_scoop(
        &f.scoop_root(),
        &[Name::new("fzf")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();

    assert_eq!(out.adopted.len(), 1, "{out:?}");
    assert_eq!(
        out.adopted[0],
        (
            Name::new("fzf"),
            Matched::Content,
            Some("0.74.2".to_string())
        ),
        "the previous pin's version must be carried out so the caller can \
         report it: {out:?}"
    );

    let lock = dotpkg::lock::load_or_empty(&lock_path).unwrap();
    assert_eq!(
        lock.scoop[&Name::new("fzf")],
        dotpkg::lock::Pin::ScoopCommit {
            bucket: "main".to_string(),
            commit: installed_commit,
            version: "0.74.1".to_string(),
        },
        "the pin must now match what is actually installed, not what update \
         last resolved"
    );

    // Secondary fix: fzf was already declared, so pkg.toml's text does not
    // change -- the write must be skipped entirely, not repeated with
    // identical bytes.
    let config_after = std::fs::read_to_string(&config_path).unwrap();
    assert_eq!(
        config_before, config_after,
        "pkg.toml's content must be untouched"
    );
    assert!(
        !config_path.with_extension("toml.bak").exists(),
        "a skipped write must not leave a .bak behind"
    );

    let rendered = dotpkg::render::render_adopt(SCOOP, &out);
    assert!(
        rendered.contains("replaced the existing pin 0.74.2"),
        "the end-to-end render must say a pin was replaced and what it was: \
         {rendered}"
    );
}

#[test]
fn an_adopted_package_is_not_a_prune_candidate_and_not_notlocked() {
    // The two failure modes the three-file rule exists to prevent, asserted
    // through the shipped planner rather than by reasoning about it.
    //
    // state.json alone => installed, owned, undeclared => Prune.
    // state.json + pkg.toml => declared, unlocked => Skip{NotLocked}, which
    // makes the next apply refuse the whole run at exit 2.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "aichat.json", "0.30.0", "v030");

    let config_path = f.home.path().join("pkg.toml");
    let lock_path = f.home.path().join("pkg.lock");
    let state_path = f.home.path().join("state.json");
    std::fs::write(
        &config_path,
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
    )
    .unwrap();
    let cur = f.scoop_root().join("apps").join("aichat").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(
        cur.join("manifest.json"),
        f.blob(&dir, "HEAD", "aichat.json"),
    )
    .unwrap();

    let out = run_scoop(
        &f.scoop_root(),
        &[Name::new("aichat")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();
    // Without this the rest of the test passes VACUOUSLY. `run` returns
    // `Ok(Outcome)` for a refusal too, and a refused adopt writes nothing --
    // leaving aichat installed, undeclared and UNOWNED, which is not a Prune
    // candidate either. The loop below would then find nothing to complain
    // about for entirely the wrong reason. This is the same failure shape the
    // Task 12 fixture bug produced, so it is pinned here.
    assert_eq!(out.adopted.len(), 1, "adopt must have succeeded: {out:?}");
    assert!(out.refused.is_empty(), "{out:?}");

    let declared = dotpkg::config::load(&config_path).unwrap();
    let lock = dotpkg::lock::load_or_empty(&lock_path).unwrap();
    let state = State::load_or_empty(&state_path).unwrap();
    let scoop = dotpkg::backend::scoop::Scoop::new(f.scoop_root());
    let scan = dotpkg::backend::Backend::scan(&scoop).unwrap();
    let plan = dotpkg::plan::plan(
        &declared,
        &lock,
        &scan.installed,
        &scan.opaque,
        &state,
        &dotpkg::model::Running::default(),
        &[],
    );

    for a in &plan.actions {
        match a {
            dotpkg::plan::Action::Prune { name, .. } => {
                panic!("an adopted package must never be a prune candidate: {name}")
            }
            dotpkg::plan::Action::Skip { name, reason, .. }
                if *reason == dotpkg::plan::SkipReason::NotLocked =>
            {
                panic!("an adopted package must not be NotLocked: {name}")
            }
            _ => {}
        }
    }
    // The positive counterweight to the two panics above, which on their own
    // are satisfied by any plan that happens to contain neither variant --
    // including an empty plan produced for the wrong reason.
    assert!(
        plan.actions.is_empty(),
        "a declared, locked, correctly-installed package needs no action: {:?}",
        plan.actions
    );
}

#[test]
fn a_package_whose_version_is_not_in_the_bucket_writes_nothing_at_all() {
    // All-or-nothing per package. A partial adopt is the shape the write order
    // is designed around, and the refusal path must not produce one.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "aichat.json", "0.30.0", "v030");

    let config_path = f.home.path().join("pkg.toml");
    let lock_path = f.home.path().join("pkg.lock");
    let state_path = f.home.path().join("state.json");
    let original = "[scoop]\nbuckets = [\"main\"]\npackages = []\n";
    std::fs::write(&config_path, original).unwrap();

    // Installed at a version the bucket has never had.
    let cur = f.scoop_root().join("apps").join("aichat").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(cur.join("manifest.json"), r#"{"version":"9.9.9"}"#).unwrap();

    let out = run_scoop(
        &f.scoop_root(),
        &[Name::new("aichat")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();

    assert_eq!(out.adopted.len(), 0);
    assert_eq!(out.refused.len(), 1);
    let (name, why) = &out.refused[0];
    assert_eq!(name, &Name::new("aichat"));
    assert!(why.contains("9.9.9"), "name the version: {why}");
    assert!(why.contains("main"), "name the bucket searched: {why}");
    // `f.bucket("main")` is a full clone, not a shallow one -- the shallow
    // hint must not fire when shallowness is not actually the cause.
    assert!(
        !why.contains("shallow"),
        "a full clone must not be misdiagnosed as shallow: {why}"
    );

    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        original,
        "pkg.toml untouched"
    );
    assert!(!lock_path.exists(), "no lock written");
    assert!(!state_path.exists(), "no state written");
}

#[test]
fn a_refusal_names_shallowness_when_that_is_the_likely_cause() {
    // Measured: a shallow clone produces exactly the same "not found" with no
    // other signal, and the user has no way to tell the two apart.
    let f = Fixture::new();
    let upstream = f.bucket("upstream");
    f.commit(&upstream, "aichat.json", "0.29.0", "v029");
    f.commit(&upstream, "aichat.json", "0.30.0", "v030");
    let shallow = f.scoop_root().join("buckets").join("main");
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

    let config_path = f.home.path().join("pkg.toml");
    std::fs::write(
        &config_path,
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
    )
    .unwrap();
    let cur = f.scoop_root().join("apps").join("aichat").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(cur.join("manifest.json"), r#"{"version":"0.29.0"}"#).unwrap();

    let out = run_scoop(
        &f.scoop_root(),
        &[Name::new("aichat")],
        &config_path,
        &f.home.path().join("pkg.lock"),
        &f.home.path().join("state.json"),
    )
    .unwrap();

    assert_eq!(out.adopted.len(), 0, "{out:?}");
    assert_eq!(out.refused.len(), 1, "{out:?}");
    let (_, why) = &out.refused[0];
    assert!(
        why.contains("shallow"),
        "a shallow bucket is the likely cause and must be named: {why}"
    );
    assert!(
        why.contains("unshallow"),
        "naming the cause is only useful with the command that fixes it: {why}"
    );
}

#[test]
fn a_declared_bucket_that_is_not_on_this_machine_is_named_as_absent_rather_than_as_manifestless() {
    // `adopt`'s own arm of the CRITICAL. `install.json`'s `bucket` hint names
    // `extras`, which pkg.toml declares and which was never cloned. Before the
    // fix, `choose_bucket`'s `stated` branch had no `.git` check, so `extras`
    // was opened as though it were there and the whole history walk came back
    // empty -- refusing with "no commit in bucket extras carries aichat
    // 0.30.0", which blames the bucket's contents for the bucket's absence.
    let f = Fixture::new();
    let main = f.bucket("main");
    f.commit(&main, "aichat.json", "0.30.0", "v030");

    let config_path = f.home.path().join("pkg.toml");
    std::fs::write(
        &config_path,
        "[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = []\n",
    )
    .unwrap();
    let cur = f.scoop_root().join("apps").join("aichat").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(cur.join("manifest.json"), r#"{"version":"0.30.0"}"#).unwrap();
    std::fs::write(cur.join("install.json"), r#"{"bucket":"extras"}"#).unwrap();

    let lock_path = f.home.path().join("pkg.lock");
    let state_path = f.home.path().join("state.json");
    let out = run_scoop(
        &f.scoop_root(),
        &[Name::new("aichat")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();

    assert_eq!(out.adopted.len(), 0, "{out:?}");
    assert_eq!(out.refused.len(), 1, "{out:?}");
    let (_, why) = &out.refused[0];
    assert!(why.contains("extras"), "name the bucket: {why}");
    assert!(
        why.contains("not present at"),
        "say that the bucket itself is absent from this machine: {why}"
    );
    assert!(
        !why.contains("no commit in bucket"),
        "nothing was searched, so nothing may be reported as not found: {why}"
    );
    assert!(
        why.contains("--clone-missing-buckets"),
        "point at the command that fixes it: {why}"
    );
    assert!(!lock_path.exists(), "no lock written");
    assert!(!state_path.exists(), "no state written");
}

#[test]
fn install_json_naming_a_bucket_pkg_toml_does_not_declare_is_named_and_told_to_declare_it() {
    // `adopt`'s arm of the `Undeclared` branch. `install.json`'s `bucket`
    // hint names `extras`, which is cloned on disk but which pkg.toml does
    // not declare. Before the fix this was `NotFound { searched:
    // vec![extras] }`, rendered as "no declared bucket has aichat (searched:
    // extras)" -- naming a bucket that was neither declared nor searched.
    let f = Fixture::new();
    let main = f.bucket("main");
    f.commit(&main, "other.json", "1.0.0", "v100");
    let extras = f.bucket("extras");
    f.commit(&extras, "aichat.json", "0.30.0", "v030");

    let config_path = f.home.path().join("pkg.toml");
    std::fs::write(
        &config_path,
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
    )
    .unwrap();
    let cur = f.scoop_root().join("apps").join("aichat").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(cur.join("manifest.json"), r#"{"version":"0.30.0"}"#).unwrap();
    std::fs::write(cur.join("install.json"), r#"{"bucket":"extras"}"#).unwrap();

    let lock_path = f.home.path().join("pkg.lock");
    let state_path = f.home.path().join("state.json");
    let out = run_scoop(
        &f.scoop_root(),
        &[Name::new("aichat")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();

    assert_eq!(out.adopted.len(), 0, "{out:?}");
    assert_eq!(out.refused.len(), 1, "{out:?}");
    let (_, why) = &out.refused[0];
    assert!(why.contains("extras"), "name the bucket: {why}");
    assert!(
        why.contains("does not declare"),
        "say it is not declared, not that it is absent from disk or that a \
         search found nothing: {why}"
    );
    assert!(
        !why.contains("searched"),
        "nothing was searched -- the bucket was never even declared: {why}"
    );
    assert!(why.contains("[scoop] buckets"), "point at the fix: {why}");
    assert!(!lock_path.exists(), "no lock written");
    assert!(!state_path.exists(), "no state written");
}

#[test]
fn a_package_that_is_not_installed_is_refused_rather_than_invented() {
    let f = Fixture::new();
    f.bucket("main");
    let config_path = f.home.path().join("pkg.toml");
    std::fs::write(
        &config_path,
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
    )
    .unwrap();

    let lock_path = f.home.path().join("pkg.lock");
    let state_path = f.home.path().join("state.json");
    let out = run_scoop(
        &f.scoop_root(),
        &[Name::new("nothere")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();
    assert_eq!(out.adopted.len(), 0, "{out:?}");
    assert_eq!(out.refused.len(), 1, "{out:?}");
    let (_, why) = &out.refused[0];
    assert!(
        why.contains("not installed"),
        "adopt brings an EXISTING package under management: {why}"
    );
    // The counterweight the message assertion alone does not carry: an
    // implementation that refuses with the right words AFTER writing is
    // indistinguishable from this one without it.
    assert!(!lock_path.exists(), "no lock written");
    assert!(!state_path.exists(), "no state written");
    assert!(
        !dotpkg::config::load(&config_path)
            .unwrap()
            .scoop
            .packages
            .contains(&Name::new("nothere")),
        "pkg.toml must not declare a package that is not installed"
    );
}

// Gated `#[cfg(unix)]`, not because the mechanism is a Windows/Unix
// distinction in principle, but because inducing a write failure while
// leaving the *read* of the same path untouched needs a permission change,
// and this crate already has precedent (tests/cli.rs, tests/scoop_scan.rs)
// for keeping that kind of fixture unix-only rather than guessing at a
// Windows equivalent that might flake or silently no-op in CI.
//
// A directory placed AT `state_path` itself (the first version of this test)
// does not isolate "only the last write fails": `State::load_or_empty` hard
// errors on `ErrorKind::IsADirectory`, the same as any other unreadable
// state.json, and that read happens before ANY of the three writes are
// attempted, in every ordering. So that fixture cannot tell "the write
// order is safe" apart from "adopt correctly refuses when it cannot even
// read state.json" -- see `an_unreadable_state_file_refuses_the_whole_
// package_before_anything_is_written` below for that second, real property.
// Making ONLY the write fail requires state_path's PARENT to reject the
// write while state_path itself stays absent (so the read is a plain,
// successful "not found").
#[cfg(unix)]
#[test]
fn a_failed_last_write_leaves_a_prefix_that_plan_does_nothing_about() {
    // The write order is lock -> pkg.toml -> state.json, and the claim is that
    // every PREFIX of it is inert. That claim is testable, not merely
    // arguable: force the last write to fail and look at what survives.
    //
    // Under this order the survivor is lock + pkg.toml: declared, locked, and
    // installed at the locked version, so plan() emits nothing at all.
    //
    // Under the state-first order the survivor would be state.json + pkg.lock
    // -- owned and undeclared -- which src/plan.rs turns into a Prune.
    // `the_forbidden_write_order_leaves_a_shape_plan_turns_into_a_prune`
    // (portable, no cfg(unix) needed) covers that consequence directly.
    use std::os::unix::fs::PermissionsExt;

    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "aichat.json", "0.30.0", "v030");

    let config_path = f.home.path().join("pkg.toml");
    let lock_path = f.home.path().join("pkg.lock");
    // state_path itself does not exist yet -- reading it returns NotFound,
    // i.e. an ordinary empty state, exactly as a first-ever adopt would see.
    // Its PARENT is what is made unwritable, right before the call, so only
    // `State::save`'s temp-file creation fails -- not the read above it.
    let state_dir = f.home.path().join("statedir");
    std::fs::create_dir_all(&state_dir).unwrap();
    let state_path = state_dir.join("state.json");

    std::fs::write(
        &config_path,
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
    )
    .unwrap();
    let cur = f.scoop_root().join("apps").join("aichat").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(
        cur.join("manifest.json"),
        f.blob(&dir, "HEAD", "aichat.json"),
    )
    .unwrap();

    std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = run_scoop(
        &f.scoop_root(),
        &[Name::new("aichat")],
        &config_path,
        &lock_path,
        &state_path,
    );
    // Restored before any assertion can early-return/panic and leave a
    // read-only directory behind for the OS to clean up.
    std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    // Reported, not propagated. This used to be `result.is_err()`, which is
    // what the `?` in `adopt::run` produced -- and that `?` skipped
    // `render_adopt` entirely, so the user saw `cannot create
    // .../state.json.tmpNNN` and no line anywhere saying pkg.lock and pkg.toml
    // had already been rewritten. The files that changed are the one thing a
    // user whose adopt died half way through needs to be told.
    let out = result.expect("a partial write is reported through the outcome, not through `?`");
    let partial = out
        .partial_write
        .as_ref()
        .expect("the state write must genuinely have failed");
    assert_eq!(partial.name, Name::new("aichat"));
    assert_eq!(
        partial.wrote,
        vec!["pkg.lock", "pkg.toml"],
        "exactly the two writes that landed, and not the one that failed"
    );
    assert!(
        partial.why.contains("state.json"),
        "name the write that failed: {}",
        partial.why
    );
    assert!(
        out.adopted.is_empty(),
        "a package whose write failed part way was not adopted: {:?}",
        out.adopted
    );
    let text = dotpkg::render::render_adopt(SCOOP, &out);
    assert!(
        text.contains("pkg.lock") && text.contains("pkg.toml"),
        "the report must name what really changed on disk: {text}"
    );

    // The first two writes stand.
    let lock = dotpkg::lock::load_or_empty(&lock_path).unwrap();
    assert!(
        lock.scoop.contains_key(&Name::new("aichat")),
        "the lock was written first"
    );
    let declared = dotpkg::config::load(&config_path).unwrap();
    assert!(
        declared.scoop.packages.contains(&Name::new("aichat")),
        "pkg.toml was written second"
    );
    assert!(
        !state_path.exists(),
        "state.json must NOT have been written"
    );

    // And what survives is inert.
    let scoop = dotpkg::backend::scoop::Scoop::new(f.scoop_root());
    let scan = dotpkg::backend::Backend::scan(&scoop).unwrap();
    let plan = dotpkg::plan::plan(
        &declared,
        &lock,
        &scan.installed,
        &scan.opaque,
        &State::default(),
        &dotpkg::model::Running::default(),
        &[],
    );
    for a in &plan.actions {
        if let dotpkg::plan::Action::Prune { name, .. } = a {
            panic!(
                "an interrupted adopt left a PRUNE candidate -- the write order is wrong: {name}"
            );
        }
    }
    assert!(
        plan.actions.is_empty(),
        "a declared, locked, correctly-installed package needs no action: {:?}",
        plan.actions
    );
}

#[test]
fn an_unreadable_state_file_refuses_the_whole_package_before_anything_is_written() {
    // The property CRITICAL review found missing: `State::load_or_empty`
    // hard-errors on a directory at state_path (measured: IsADirectory, not
    // NotFound), and `adopt::run` must let that propagate rather than
    // default to "nothing owned" -- a default would let it write pkg.lock
    // and edit pkg.toml on a false belief and discover the problem only at
    // the final `state.save`. This is a permanent regression test for that:
    // it goes red if a guard like that is ever reintroduced, because then
    // `out.adopted.len()` would be 1 and pkg.lock/pkg.toml would exist.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "aichat.json", "0.30.0", "v030");

    let config_path = f.home.path().join("pkg.toml");
    let lock_path = f.home.path().join("pkg.lock");
    let original = "[scoop]\nbuckets = [\"main\"]\npackages = []\n";
    std::fs::write(&config_path, original).unwrap();
    let cur = f.scoop_root().join("apps").join("aichat").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(
        cur.join("manifest.json"),
        f.blob(&dir, "HEAD", "aichat.json"),
    )
    .unwrap();

    // A directory where state.json should be: unreadable as state, not
    // merely absent.
    let state_path = f.home.path().join("state.json");
    std::fs::create_dir_all(state_path.join("occupied")).unwrap();

    let result = run_scoop(
        &f.scoop_root(),
        &[Name::new("aichat")],
        &config_path,
        &lock_path,
        &state_path,
    );
    assert!(
        result.is_err(),
        "an unreadable state.json must refuse, not proceed on a guessed default"
    );
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        original,
        "pkg.toml untouched"
    );
    assert!(!lock_path.exists(), "no lock written");
}

#[test]
fn the_forbidden_write_order_leaves_a_shape_plan_turns_into_a_prune() {
    // This is why `adopt::run` writes state.json LAST, after pkg.lock and
    // pkg.toml. If the order were reversed -- state.json before pkg.lock and
    // pkg.toml -- an interruption right after state.json lands leaves this
    // exact shape: owned (state.json says so), locked (as if pkg.lock had
    // also landed), but never declared (pkg.toml never got its turn). That
    // shape is not hypothetical -- `plan()`, the same planner `apply` uses,
    // turns it into a Prune, which would UNINSTALL a package the user just
    // told dotpkg to adopt.
    //
    // Built directly here, through each file's own writer (`lock::save`,
    // `State::save`), not through `adopt::run` -- which now always writes in
    // the safe order and can no longer produce this shape on its own. This
    // is what makes the consequence covered rather than merely argued in a
    // comment: `a_failed_last_write_leaves_a_prefix_that_plan_does_nothing_
    // about` only proves the SHIPPED order is safe; this proves what the
    // FORBIDDEN order would cost if it ever came back.
    let f = Fixture::new();
    let dir = f.bucket("main");
    let commit = f.commit(&dir, "aichat.json", "0.30.0", "v030");

    let config_path = f.home.path().join("pkg.toml");
    let lock_path = f.home.path().join("pkg.lock");
    let state_path = f.home.path().join("state.json");

    // pkg.toml: does NOT declare aichat -- the write the forbidden order
    // never reaches.
    std::fs::write(
        &config_path,
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
    )
    .unwrap();

    // pkg.lock: has an entry for aichat, as if the forbidden order's second
    // write had already landed.
    let mut lock = dotpkg::lock::Lock::default();
    lock.scoop.insert(
        Name::new("aichat"),
        dotpkg::lock::Pin::ScoopCommit {
            bucket: "main".to_string(),
            commit,
            version: "0.30.0".to_string(),
        },
    );
    dotpkg::lock::save(&lock, &lock_path).unwrap();

    // state.json: owns aichat, as if the forbidden order's FIRST write had
    // landed and then something interrupted the run before pkg.toml's turn.
    let mut state = State::default();
    state.set(
        dotpkg::model::SCOOP,
        &Name::new("aichat"),
        Ownership::Adopted,
    );
    state.save(&state_path).unwrap();

    // The package really is installed, at the locked version -- otherwise
    // this would be a stale ghost entry, not the shape under test.
    let cur = f.scoop_root().join("apps").join("aichat").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(
        cur.join("manifest.json"),
        f.blob(&dir, "HEAD", "aichat.json"),
    )
    .unwrap();

    let declared = dotpkg::config::load(&config_path).unwrap();
    let locked = dotpkg::lock::load_or_empty(&lock_path).unwrap();
    let loaded_state = State::load_or_empty(&state_path).unwrap();
    let scoop = dotpkg::backend::scoop::Scoop::new(f.scoop_root());
    let scan = dotpkg::backend::Backend::scan(&scoop).unwrap();

    let plan = dotpkg::plan::plan(
        &declared,
        &locked,
        &scan.installed,
        &scan.opaque,
        &loaded_state,
        &dotpkg::model::Running::default(),
        &[],
    );

    let pruned = plan.actions.iter().any(|a| {
        matches!(
            a,
            dotpkg::plan::Action::Prune { name, .. } if *name == Name::new("aichat")
        )
    });
    assert!(
        pruned,
        "owned + locked + undeclared must plan a Prune for aichat -- this is \
         why adopt writes state.json LAST: {:?}",
        plan.actions
    );
}

#[test]
fn adopting_an_already_managed_package_again_is_refused_not_repeated() {
    // The guard `an un-fireable negative control is a plan failure` exists
    // to check: this is the test Step 6.2 names to confirm the
    // `state.owns` early return actually does something. Adopt the same
    // package twice in separate calls -- the second must be refused, not
    // silently re-adopted or re-pinned.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "aichat.json", "0.30.0", "v030");

    let config_path = f.home.path().join("pkg.toml");
    let lock_path = f.home.path().join("pkg.lock");
    let state_path = f.home.path().join("state.json");
    std::fs::write(
        &config_path,
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
    )
    .unwrap();
    let cur = f.scoop_root().join("apps").join("aichat").join("current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(
        cur.join("manifest.json"),
        f.blob(&dir, "HEAD", "aichat.json"),
    )
    .unwrap();

    let first = run_scoop(
        &f.scoop_root(),
        &[Name::new("aichat")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();
    assert_eq!(first.adopted.len(), 1, "{first:?}");

    let second = run_scoop(
        &f.scoop_root(),
        &[Name::new("aichat")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();
    assert_eq!(second.adopted.len(), 0, "{second:?}");
    assert_eq!(second.refused.len(), 1, "{second:?}");
    let (name, why) = &second.refused[0];
    assert_eq!(name, &Name::new("aichat"));
    assert!(
        why.contains("already managed"),
        "say why the second call refused: {why}"
    );
}

#[test]
fn adopting_two_packages_in_one_command_does_not_lose_the_first() {
    // The reason `run` re-reads all three files at the top of every loop
    // iteration rather than caching them once: an in-memory `Lock`/`Config`/
    // `State` built before the loop and never refreshed would still hold
    // last package's write in memory, but a caller re-reading straight from
    // disk after the whole call returns would see only the SECOND package if
    // each iteration's on-disk write were not landed before the next
    // iteration started forming its own updated copy from a stale read.
    let f = Fixture::new();
    let dir = f.bucket("main");
    f.commit(&dir, "aichat.json", "0.30.0", "v030");
    f.commit(&dir, "widget.json", "1.2.3", "w123");

    let config_path = f.home.path().join("pkg.toml");
    let lock_path = f.home.path().join("pkg.lock");
    let state_path = f.home.path().join("state.json");
    std::fs::write(
        &config_path,
        "[scoop]\nbuckets = [\"main\"]\npackages = []\n",
    )
    .unwrap();

    for (app, file) in [("aichat", "aichat.json"), ("widget", "widget.json")] {
        let cur = f.scoop_root().join("apps").join(app).join("current");
        std::fs::create_dir_all(&cur).unwrap();
        std::fs::write(cur.join("manifest.json"), f.blob(&dir, "HEAD", file)).unwrap();
    }

    let out = run_scoop(
        &f.scoop_root(),
        &[Name::new("aichat"), Name::new("widget")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();
    assert_eq!(out.adopted.len(), 2, "{out:?}");

    let lock = dotpkg::lock::load_or_empty(&lock_path).unwrap();
    assert!(
        lock.scoop.contains_key(&Name::new("aichat")),
        "the first package's lock entry must survive the second's write: {lock:?}"
    );
    assert!(lock.scoop.contains_key(&Name::new("widget")), "{lock:?}");

    let declared = dotpkg::config::load(&config_path).unwrap();
    assert!(
        declared.scoop.packages.contains(&Name::new("aichat")),
        "the first package's pkg.toml entry must survive the second's write: {declared:?}"
    );
    assert!(declared.scoop.packages.contains(&Name::new("widget")));

    let state = State::load_or_empty(&state_path).unwrap();
    assert_eq!(
        state.ownership(dotpkg::model::SCOOP, &Name::new("aichat")),
        Some(Ownership::Adopted),
        "the first package's state entry must survive the second's write"
    );
    assert_eq!(
        state.ownership(dotpkg::model::SCOOP, &Name::new("widget")),
        Some(Ownership::Adopted)
    );
}

// -- adopt --backend winget (Task 15 review, Important 2) -------------------
//
// Before this addition, `tests/adopt.rs` had exactly one winget test, a
// refusal (`tests/cli.rs`'s `adopt_backend_winget_refuses_gracefully_when_
// the_package_is_not_installed`), and nothing at all exercised the success
// path: ~90 lines that write three files to the user's disk. Fixtures are
// the same rule `tests/winget_scan.rs`/`tests/winget_resolve.rs` already
// follow: no hand-built `winget list`/`winget show` text, only the checked-in
// a14 captures (`tests/fixtures/winget/PROVENANCE.md`). `list-single.txt`
// carries exactly one installed package, `ajeetdsouza.zoxide` at `0.10.0`;
// `show-old-version.txt` is a real `show -v 0.9.0` reply for that same
// package -- the version mismatch between the two (scan says 0.10.0,
// resolve_installed's canned reply says 0.9.0) is harmless here, the same
// way `tests/winget_resolve.rs`'s own tests never try to make a `FakeWinget`
// script internally consistent across calls, only correct per call.

#[test]
fn adopt_backend_winget_brings_an_installed_package_under_management() {
    let fake = FakeWinget::script(vec![
        (0, winget_fixture("list-single.txt")),
        (0, winget_fixture("show-old-version.txt")),
    ]);
    let winget = Winget::new(fake);

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("pkg.toml");
    let lock_path = dir.path().join("pkg.lock");
    let state_path = dir.path().join("state.json");
    std::fs::write(&config_path, "# hand written\n[scoop]\npackages = []\n").unwrap();

    let out = run_winget(
        winget,
        &[Name::new("ajeetdsouza.zoxide")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();

    assert_eq!(out.adopted.len(), 1, "{out:?}");
    assert_eq!(
        out.adopted[0],
        (
            Name::new("ajeetdsouza.zoxide"),
            Matched::WingetConfirmed,
            None
        ),
        "winget's own confirmation rule, not one of scoop's two, and no \
         previous pin to replace"
    );
    assert!(out.refused.is_empty(), "{out:?}");

    // The lock, keyed by the canonical id, holding the version `show -v`
    // actually confirmed.
    let lock = dotpkg::lock::load_or_empty(&lock_path).unwrap();
    assert_eq!(
        lock.winget[&Name::new("ajeetdsouza.zoxide")],
        dotpkg::lock::Pin::WingetVersion {
            version: "0.9.0".to_string(),
        }
    );

    // pkg.toml, comments preserved, now declaring the package.
    let cfg_text = std::fs::read_to_string(&config_path).unwrap();
    assert!(cfg_text.contains("# hand written"), "{cfg_text}");
    let cfg = dotpkg::config::parse(&cfg_text).unwrap();
    assert!(cfg
        .winget
        .packages
        .contains(&Name::new("ajeetdsouza.zoxide")));
    assert!(
        cfg.scoop.packages.is_empty(),
        "a different backend's section must be untouched"
    );

    // state.json, owned under the winget backend.
    let state = State::load_or_empty(&state_path).unwrap();
    assert_eq!(
        state.ownership(WINGET, &Name::new("ajeetdsouza.zoxide")),
        Some(Ownership::Adopted)
    );
    assert_eq!(
        state.ownership(SCOOP, &Name::new("ajeetdsouza.zoxide")),
        None,
        "adopted under winget, not scoop"
    );
}

#[test]
fn adopt_backend_winget_refuses_a_package_already_managed_rather_than_reconfirming() {
    // `FakeWinget::script` with only the `list` response: if the `state.owns`
    // refusal below were ever deleted, `adopt_one_winget` would go on to call
    // `resolve_installed`, which would try to consume a second scripted
    // response that does not exist and panic -- a stronger, structural
    // counterweight than asserting the refused count alone.
    let fake = FakeWinget::script(vec![(0, winget_fixture("list-single.txt"))]);
    let winget = Winget::new(fake);

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("pkg.toml");
    let lock_path = dir.path().join("pkg.lock");
    let state_path = dir.path().join("state.json");
    std::fs::write(&config_path, "[winget]\npackages = []\n").unwrap();

    let mut state = State::default();
    state.set(WINGET, &Name::new("ajeetdsouza.zoxide"), Ownership::Adopted);
    state.save(&state_path).unwrap();

    let out = run_winget(
        winget,
        &[Name::new("ajeetdsouza.zoxide")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();

    assert_eq!(out.adopted.len(), 0, "{out:?}");
    assert_eq!(out.refused.len(), 1, "{out:?}");
    let (name, why) = &out.refused[0];
    assert_eq!(name, &Name::new("ajeetdsouza.zoxide"));
    assert!(why.contains("already managed"), "{why}");
    assert!(!lock_path.exists(), "no lock written for a refusal");
}

#[test]
fn adopt_backend_winget_refuses_when_show_echoes_a_different_id_rather_than_a_different_case() {
    // The boundary of the test below. `show` runs without `--exact` -- that is
    // what folds case on the way in -- which also leaves `--id` a substring
    // filter, so it can answer about a different package entirely.
    //
    // **The two fixtures are each a real capture; pairing them is synthetic
    // and deliberate.** `list-single.txt` really lists `ajeetdsouza.zoxide`
    // and `show-canonical-echo.txt` really echoes `Git.Git`; no machine would
    // ever answer both about one package. What is under test is dotpkg's
    // comparison of the two ids, not winget's behaviour, and no fixture pairs
    // a list with a foreign `show` echo because nobody has captured one.
    //
    // Refusing matters because the alternative was silent and unusable:
    // `pkg.lock` and `state.json` keyed `Git.Git`, `pkg.toml` keyed what was
    // typed, and `plan` looking the pin up under the declared name -- so
    // `adopt` printed success and the next `apply` refused the whole run at
    // exit 2. `update` refuses the same shape.
    let fake = FakeWinget::script(vec![
        (0, winget_fixture("list-single.txt")),
        (0, winget_fixture("show-canonical-echo.txt")),
    ]);
    let winget = Winget::new(fake);

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("pkg.toml");
    let lock_path = dir.path().join("pkg.lock");
    let state_path = dir.path().join("state.json");
    std::fs::write(&config_path, "[winget]\npackages = []\n").unwrap();

    let out = run_winget(
        winget,
        &[Name::new("ajeetdsouza.zoxide")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();

    assert!(out.adopted.is_empty(), "nothing may be adopted: {out:?}");
    assert_eq!(out.refused.len(), 1, "{out:?}");
    let (refused_name, why) = &out.refused[0];
    assert_eq!(refused_name, &Name::new("ajeetdsouza.zoxide"));
    assert!(
        why.contains("Git.Git") && why.contains("ajeetdsouza.zoxide"),
        "name both the id winget matched and the one that was typed, since the \
         fix is to retype it: {why}"
    );

    // And nothing may have been written under either spelling. This is the
    // half that makes the refusal worth having: a warning would have left all
    // three files on disk, disagreeing.
    let lock = dotpkg::lock::load_or_empty(&lock_path).unwrap();
    assert!(lock.winget.is_empty(), "no pin: {:?}", lock.winget);
    let state = State::load_or_empty(&state_path).unwrap();
    assert_eq!(state.ownership(WINGET, &Name::new("Git.Git")), None);
    assert_eq!(
        state.ownership(WINGET, &Name::new("ajeetdsouza.zoxide")),
        None
    );
    let cfg_text = std::fs::read_to_string(&config_path).unwrap();
    assert!(
        !cfg_text.contains("Git.Git") && !cfg_text.contains("zoxide"),
        "pkg.toml must be untouched: {cfg_text}"
    );
}

#[test]
fn adopt_backend_winget_reports_a_canonical_case_difference_the_same_way_update_does() {
    // The other half of Task 15 review's Important 1: `update` warns when
    // the spelling it resolved differs from what pkg.toml declared;
    // `adopt` did not. Typed as "AjeetDSouza.Zoxide" -- `scan.installed`
    // still finds it (`Name::Eq` folds case), but winget's own `show -v`
    // reply (`show-old-version.txt`) echoes back the real, lowercase id.
    let fake = FakeWinget::script(vec![
        (0, winget_fixture("list-single.txt")),
        (0, winget_fixture("show-old-version.txt")),
    ]);
    let winget = Winget::new(fake);

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("pkg.toml");
    let lock_path = dir.path().join("pkg.lock");
    let state_path = dir.path().join("state.json");
    std::fs::write(&config_path, "[winget]\npackages = []\n").unwrap();

    let out = run_winget(
        winget,
        &[Name::new("AjeetDSouza.Zoxide")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();

    assert_eq!(out.adopted.len(), 1, "{out:?}");
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("AjeetDSouza.Zoxide") && w.contains("ajeetdsouza.zoxide")),
        "name both spellings: {:?}",
        out.warnings
    );

    // pkg.toml keeps what the user typed...
    let cfg_text = std::fs::read_to_string(&config_path).unwrap();
    assert!(cfg_text.contains("AjeetDSouza.Zoxide"), "{cfg_text}");
    assert!(
        !cfg_text.contains("ajeetdsouza.zoxide"),
        "pkg.toml must not be silently corrected: {cfg_text}"
    );

    // ...but the lock and state.json record the canonical spelling.
    let lock = dotpkg::lock::load_or_empty(&lock_path).unwrap();
    let (stored_key, _) = lock
        .winget
        .get_key_value(&Name::new("AjeetDSouza.Zoxide"))
        .unwrap();
    assert_eq!(stored_key.to_string(), "ajeetdsouza.zoxide");

    let state = State::load_or_empty(&state_path).unwrap();
    assert_eq!(
        state.ownership(WINGET, &Name::new("ajeetdsouza.zoxide")),
        Some(Ownership::Adopted)
    );
}

#[test]
fn adopt_backend_winget_does_not_warn_when_the_typed_spelling_already_matches_the_canonical_one() {
    // The positive counterweight to the test above: without it, a version
    // that warns on every winget adoption regardless of case would satisfy
    // the case-difference test on its own.
    let fake = FakeWinget::script(vec![
        (0, winget_fixture("list-single.txt")),
        (0, winget_fixture("show-old-version.txt")),
    ]);
    let winget = Winget::new(fake);

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("pkg.toml");
    let lock_path = dir.path().join("pkg.lock");
    let state_path = dir.path().join("state.json");
    std::fs::write(&config_path, "[winget]\npackages = []\n").unwrap();

    let out = run_winget(
        winget,
        &[Name::new("ajeetdsouza.zoxide")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();

    assert_eq!(out.adopted.len(), 1, "{out:?}");
    assert!(
        !out.warnings.iter().any(|w| w.contains("you typed this as")),
        "an exact-case match has nothing to report: {:?}",
        out.warnings
    );
}

// -- adopt --backend winget for a package declared `pin = "none"` -----------
//
// Design: `docs/specs/2026-08-13-winget-unpinned-design.md` §9. This path is
// the ONLY way an already-installed unpinned package ever becomes prunable:
// `apply` never installs a package that is already present, so it never comes
// to own one, and `prune` can only reach what dotpkg owns. It therefore has to
// work rather than refuse.
//
// `list-single.txt` is the same a14 capture the test above uses -- one
// installed package, `ajeetdsouza.zoxide` at `0.10.0`.

#[test]
fn adopting_an_unpinned_package_records_ownership_and_writes_no_lock_entry() {
    // One scripted response, not two: the scan, and nothing else. A second
    // entry would let a stray `show -v` call pass unnoticed -- see the
    // sibling test below, which turns that into a panic.
    let winget = Winget::new(FakeWinget::script(vec![(
        0,
        winget_fixture("list-single.txt"),
    )]));

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("pkg.toml");
    let lock_path = dir.path().join("pkg.lock");
    let state_path = dir.path().join("state.json");
    std::fs::write(
        &config_path,
        "# hand written\n[winget]\npackages = [\"ajeetdsouza.zoxide\"]\n\
         [winget.opts]\n\"ajeetdsouza.zoxide\" = { pin = \"none\" }\n",
    )
    .unwrap();

    let out = run_winget(
        winget,
        &[Name::new("ajeetdsouza.zoxide")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();

    assert_eq!(out.adopted.len(), 1, "{out:?}");
    assert_eq!(
        out.adopted[0],
        (Name::new("ajeetdsouza.zoxide"), Matched::Unpinned, None),
        "a fourth rule, not one of the other three: nothing was confirmed \
         because nothing is being pinned, and there is no previous pin to \
         replace because none is ever written"
    );
    assert!(out.refused.is_empty(), "{out:?}");

    // **The point of the whole path.** Ownership really is recorded, which is
    // what lets a later `prune` reach this package at all.
    let state = State::load_or_empty(&state_path).unwrap();
    assert_eq!(
        state.ownership(WINGET, &Name::new("ajeetdsouza.zoxide")),
        Some(Ownership::Adopted)
    );

    // And no lock entry exists -- not an empty one, not a stub. `pkg.lock`
    // records what a declaration resolved to, and an unpinned declaration
    // resolves to nothing.
    let lock = dotpkg::lock::load_or_empty(&lock_path).unwrap();
    assert!(
        lock.winget.is_empty(),
        "an unpinned adopt must write no winget pin: {lock:?}"
    );
    assert!(
        lock.scoop.is_empty(),
        "and nothing on the other side either"
    );

    // pkg.toml is already correct -- the package is declared, with its opts
    // entry -- so it must come back byte-identical, comments and all.
    let cfg_text = std::fs::read_to_string(&config_path).unwrap();
    assert!(cfg_text.contains("# hand written"), "{cfg_text}");
    assert_eq!(
        dotpkg::config::parse(&cfg_text).unwrap().winget.unpinned(),
        std::collections::BTreeSet::from([Name::new("ajeetdsouza.zoxide")]),
        "the opts entry must survive the adopt: losing it would silently turn \
         the package back into a pinned one"
    );
}

#[test]
fn adopting_an_unpinned_package_asks_winget_nothing_beyond_the_scan() {
    // There is no version to confirm, so `resolve_installed` -- one ~1 s
    // `winget show -v` -- must not run at all. `FakeWinget::script` panics
    // when a call has no scripted response, so the single entry below IS the
    // assertion: a second spawn turns this test red rather than slow.
    //
    // The canonical id needs no lookup either. `inst.name` is `winget list`'s
    // own `Id` column, canonical by construction, unlike a hand-typed
    // `pkg.toml` spelling.
    let fake = FakeWinget::script(vec![(0, winget_fixture("list-single.txt"))]);
    let winget = Winget::new(fake.clone());

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("pkg.toml");
    let lock_path = dir.path().join("pkg.lock");
    let state_path = dir.path().join("state.json");
    std::fs::write(
        &config_path,
        "[winget]\npackages = [\"ajeetdsouza.zoxide\"]\n\
         [winget.opts]\n\"ajeetdsouza.zoxide\" = { pin = \"none\" }\n",
    )
    .unwrap();

    let out = run_winget(
        winget,
        &[Name::new("ajeetdsouza.zoxide")],
        &config_path,
        &lock_path,
        &state_path,
    )
    .unwrap();
    assert_eq!(out.adopted.len(), 1, "{out:?}");

    let calls = fake.calls();
    assert_eq!(
        calls.len(),
        1,
        "exactly one winget invocation -- the scan. Got: {calls:?}"
    );
    assert_eq!(
        calls[0][0], "list",
        "and it is the scan, not a show: {calls:?}"
    );

    // The counterweight, in the same test so the two cannot drift: the PINNED
    // adopt of the same package really does spawn a second call. Without this,
    // a change that broke adoption entirely would leave the assertion above
    // green for the wrong reason.
    let fake2 = FakeWinget::script(vec![
        (0, winget_fixture("list-single.txt")),
        (0, winget_fixture("show-old-version.txt")),
    ]);
    let dir2 = tempfile::tempdir().unwrap();
    let cfg2 = dir2.path().join("pkg.toml");
    std::fs::write(&cfg2, "[winget]\npackages = [\"ajeetdsouza.zoxide\"]\n").unwrap();
    let out2 = run_winget(
        Winget::new(fake2.clone()),
        &[Name::new("ajeetdsouza.zoxide")],
        &cfg2,
        &dir2.path().join("pkg.lock"),
        &dir2.path().join("state.json"),
    )
    .unwrap();
    assert_eq!(out2.adopted.len(), 1, "{out2:?}");
    assert_eq!(
        fake2.calls().len(),
        2,
        "a pinned adopt asks twice -- scan, then `show -v` to confirm the \
         version it is about to pin: {:?}",
        fake2.calls()
    );
}
