use dotpkg::backend::scoop::Scoop;
use dotpkg::lock::Pin;
use dotpkg::model::Name;
use std::fs;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Build a real git repository shaped like a scoop bucket: manifests under
/// `bucket/`, one commit per version. Returns a commit sha per version, in
/// the order given.
///
/// This is git, not a stand-in for git. `stage` runs the real binary here.
fn bucket_repo(
    scoop_root: &Path,
    bucket: &str,
    manifest_file: &str,
    versions: &[&str],
) -> Vec<String> {
    let dir = scoop_root.join("buckets").join(bucket);
    fs::create_dir_all(dir.join("bucket")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    let mut shas = Vec::new();
    for v in versions {
        // Escaped, because the path-escape tests below pin a version that is
        // itself a filesystem path and would otherwise emit invalid JSON on
        // Windows. No effect on an ordinary version string.
        let escaped = v.replace('\\', r"\\");
        fs::write(
            dir.join("bucket").join(manifest_file),
            format!(r#"{{"version":"{escaped}","bin":"tool.exe"}}"#),
        )
        .unwrap();
        git(&dir, &["add", "-A"]);
        git(
            &dir,
            &[
                "-c",
                "user.email=t@example.invalid",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                "bump",
            ],
        );
        shas.push(git(&dir, &["rev-parse", "HEAD"]).trim().to_string());
    }
    shas
}

fn pin(bucket: &str, commit: &str, version: &str) -> Pin {
    Pin::ScoopCommit {
        bucket: bucket.into(),
        commit: commit.into(),
        version: version.into(),
    }
}

#[test]
fn an_old_commit_recovers_the_old_manifest_not_the_current_one() {
    // The whole reproducibility claim in one test: the bucket has moved on to
    // 2.0.0, and the lock still gets 1.0.0.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "tool.json", &["1.0.0", "2.0.0"]);

    let staged = Scoop::new(root.path().to_path_buf())
        .stage(
            stage_dir.path(),
            &Name::new("tool"),
            &pin("main", &shas[0], "1.0.0"),
        )
        .unwrap();

    let text = fs::read_to_string(&staged).unwrap();
    assert!(text.contains("1.0.0"), "got {text}");
    assert!(
        !text.contains("2.0.0"),
        "recovered the current manifest, not the pinned one: {text}"
    );
}

#[test]
fn a_commit_the_bucket_does_not_have_fails_and_stages_nothing() {
    // The approved design's second mandatory test. A lock that quietly falls
    // back to latest is worse than no lock, because it makes a guarantee that
    // is not there.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    bucket_repo(root.path(), "main", "tool.json", &["1.0.0"]);

    let err = Scoop::new(root.path().to_path_buf())
        .stage(
            stage_dir.path(),
            &Name::new("tool"),
            &pin("main", "0000000000000000000000000000000000000000", "1.0.0"),
        )
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("0000000"), "name the commit: {msg}");
    assert!(msg.contains("main"), "name the bucket: {msg}");
    // The diagnosis, not just incidental substrings: without this, the
    // fallthrough "has no manifest for [...]" message also contains the sha
    // and the bucket name and would pass these assertions for the wrong
    // reason -- it would read as "no file matches", not "the commit itself
    // does not exist".
    assert!(
        msg.contains("is not in bucket"),
        "name why it failed, not just what: {msg}"
    );
    assert_eq!(
        fs::read_dir(stage_dir.path()).unwrap().count(),
        0,
        "nothing may be staged when the commit is missing"
    );
}

#[test]
fn a_lock_naming_a_branch_instead_of_a_hash_is_refused_and_stages_nothing() {
    // Measured against real git: `cat-file -e main^{commit}` accepts `main`,
    // `HEAD`, `@` and `refs/heads/main` -- it resolves any revision, not only
    // an object name -- and `git show main:bucket/tool.json` then returns the
    // TIP. When the tip carries the same version (a url/hash correction),
    // stage_text's version check passes too and the pin silently means latest.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    bucket_repo(root.path(), "main", "tool.json", &["1.0.0", "2.0.0"]);
    let scoop = Scoop::new(root.path().to_path_buf());

    for rev in ["main", "HEAD", "@", "refs/heads/main"] {
        let Err(err) = scoop.stage(
            stage_dir.path(),
            &Name::new("tool"),
            &pin("main", rev, "2.0.0"),
        ) else {
            panic!("{rev:?} must not be accepted as a pin");
        };
        let msg = format!("{err:#}");
        assert!(msg.contains(rev), "name the offending value: {msg}");
        assert!(
            msg.contains("hex"),
            "say what a commit must look like: {msg}"
        );
        // The neighbouring failure this must NOT be confused with. Without
        // this line, deleting the hex check leaves the test green whenever the
        // revision also happens to be missing from the bucket -- which is the
        // shape of negative control that has burned this project twice.
        assert!(
            !msg.contains("is not in bucket"),
            "refused for its shape, not for being absent: {msg}"
        );
    }
    assert!(
        !stage_dir.path().join("tool").exists(),
        "nothing may be staged for a refused pin"
    );
}

#[test]
fn a_real_commit_hash_is_still_accepted() {
    // The positive control. Without it, `ensure_commit_hash` returning Err
    // unconditionally passes the test above.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "tool.json", &["1.0.0", "2.0.0"]);
    let scoop = Scoop::new(root.path().to_path_buf());

    let staged = scoop
        .stage(
            stage_dir.path(),
            &Name::new("tool"),
            &pin("main", &shas[0], "1.0.0"),
        )
        .expect("a real 40-hex commit must still work");
    assert!(staged.exists());
}

#[test]
fn two_commits_of_one_version_stage_to_different_paths() {
    // install.json records the staged path verbatim -- measured 2026-08-08,
    // `{"architecture":"arm64","url":"<the staging path>"}`. Keyed on app and
    // version alone, re-pinning the same version to a different commit
    // overwrites the file an installed app is still pointing at, and the app
    // silently starts describing a manifest it was not installed from.
    //
    // Phase 3 makes that re-pin routine: a bucket amending a url or hash
    // without bumping the version is exactly what `update`'s `=` line reports.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let dir = root.path().join("buckets").join("main");
    fs::create_dir_all(dir.join("bucket")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.email", "t@example.invalid"]);
    git(&dir, &["config", "user.name", "t"]);

    let mut shas = Vec::new();
    for url in ["good", "amended"] {
        fs::write(
            dir.join("bucket").join("tool.json"),
            format!(r#"{{"version":"1.0.0","url":"https://example.invalid/{url}.zip"}}"#),
        )
        .unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", url]);
        shas.push(git(&dir, &["rev-parse", "HEAD"]).trim().to_string());
    }

    let scoop = Scoop::new(root.path().to_path_buf());
    let first = scoop
        .stage(
            stage_dir.path(),
            &Name::new("tool"),
            &pin("main", &shas[0], "1.0.0"),
        )
        .unwrap();
    let second = scoop
        .stage(
            stage_dir.path(),
            &Name::new("tool"),
            &pin("main", &shas[1], "1.0.0"),
        )
        .unwrap();

    // Ordered so the CONTENT assertion is the one that fires when the fix is
    // reverted, not the path-inequality assertion below it. With the commit
    // dropped from the path, `first` and `second` are the same PathBuf before
    // either `stage()` call returns, so `assert_ne!` would trip first and the
    // test would never reach the assertion that names the actual defect: the
    // second staging silently overwriting the file the first one wrote. Both
    // `first.exists()` and `assert_ne!` stay true even under the reverted
    // code (the shared path still exists; the two PathBuf values are compared
    // only here, further down), so they cannot mask this one.
    assert!(
        first.exists(),
        "the first staged manifest must survive the second staging"
    );
    assert!(
        fs::read_to_string(&first).unwrap().contains("good"),
        "the first path must still hold the FIRST commit's manifest"
    );
    assert!(fs::read_to_string(&second).unwrap().contains("amended"));
    assert_ne!(
        first, second,
        "one version at two commits must not share a path"
    );

    // The filename is still what scoop takes the app name from.
    assert_eq!(first.file_name().unwrap(), "tool.json");
    assert_eq!(second.file_name().unwrap(), "tool.json");
}

#[test]
fn a_manifest_absent_at_the_pinned_commit_does_not_fall_back_to_the_working_tree() {
    // The commit here IS real -- `cat-file -e` passes -- but the app's
    // manifest was not added until a LATER commit than the one the lock
    // pins. `cat-file -e` cannot catch this: the commit genuinely exists.
    // Only `git_show` refusing to read outside the pinned commit can. Under
    // a `git_show` that falls back to the working tree when the pinned path
    // is missing, this would silently stage whatever tool.json says right
    // now (2.0.0, added one commit later) and report success -- the exact
    // "quietly falls back to latest" failure the design forbids.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let dir = root.path().join("buckets").join("main");
    fs::create_dir_all(dir.join("bucket")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);

    fs::write(
        dir.join("bucket").join("other.json"),
        r#"{"version":"1.0.0","bin":"other.exe"}"#,
    )
    .unwrap();
    git(&dir, &["add", "-A"]);
    git(
        &dir,
        &[
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "other only",
        ],
    );
    let commit_before_tool_existed = git(&dir, &["rev-parse", "HEAD"]).trim().to_string();

    // tool.json shows up only here -- one commit later than the one pinned
    // below. The working tree has it from this point on; the pinned commit
    // never did.
    fs::write(
        dir.join("bucket").join("tool.json"),
        r#"{"version":"2.0.0","bin":"tool.exe"}"#,
    )
    .unwrap();
    git(&dir, &["add", "-A"]);
    git(
        &dir,
        &[
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "add tool",
        ],
    );

    let err = Scoop::new(root.path().to_path_buf())
        .stage(
            stage_dir.path(),
            &Name::new("tool"),
            &pin("main", &commit_before_tool_existed, "2.0.0"),
        )
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("tool"), "name the app: {msg}");
    // "tool" alone is satisfied by every error branch of stage(), including
    // the ones that would fire if this test's setup silently stopped
    // reproducing the situation it describes. The fallthrough message is the
    // only one that means "the commit is real and the file is not in it".
    assert!(
        msg.contains("has no manifest for"),
        "name why it failed, not just what: {msg}"
    );
    assert_eq!(
        fs::read_dir(stage_dir.path()).unwrap().count(),
        0,
        "nothing may be staged when the manifest is absent at the pinned commit"
    );
}

#[test]
fn the_staged_file_is_named_for_the_buckets_spelling_not_the_users() {
    // scoop takes the installed app name from the FILENAME, so this is what
    // makes the resulting app directory identical to what a plain
    // `scoop install tool` would create.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "Tool.json", &["1.0.0"]);

    let staged = Scoop::new(root.path().to_path_buf())
        .stage(
            stage_dir.path(),
            &Name::new("TOOL"),
            &pin("main", &shas[0], "1.0.0"),
        )
        .unwrap();

    assert_eq!(
        staged.file_name().unwrap(),
        "Tool.json",
        "got {}",
        staged.display()
    );
}

#[test]
fn a_manifest_whose_version_disagrees_with_the_lock_fails() {
    // The commit is right and the file is there, but the lock says something
    // else. Installing it would install a version nobody asked for.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "tool.json", &["1.0.0"]);

    let err = Scoop::new(root.path().to_path_buf())
        .stage(
            stage_dir.path(),
            &Name::new("tool"),
            &pin("main", &shas[0], "9.9.9"),
        )
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("9.9.9") && msg.contains("1.0.0"),
        "name both versions: {msg}"
    );
}

#[test]
fn a_missing_bucket_is_named_rather_than_guessed_at() {
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let err = Scoop::new(root.path().to_path_buf())
        .stage(
            stage_dir.path(),
            &Name::new("tool"),
            // A real commit shape (40 hex), not a placeholder: since
            // ensure_commit_hash now runs before the bucket-exists check, a
            // short dummy like the old "abc123" would fail there first and
            // this test would stop proving what it is named for.
            &pin("extras", &"a".repeat(40), "1.0.0"),
        )
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("extras"), "name the bucket: {msg}");
    // The diagnosis, not just the bucket name: every later error out of
    // stage() also contains "extras", so asserting only that passed with the
    // bucket-exists check deleted outright. This message is what the design
    // hands 2b-2 as the trigger for `scoop bucket add` -- losing it turns
    // "your bucket is missing" into "your commit is broken", which sends the
    // user to fix the wrong thing.
    assert!(
        msg.contains("not present at"),
        "name why it failed, not just what: {msg}"
    );
    assert_eq!(
        fs::read_dir(stage_dir.path()).unwrap().count(),
        0,
        "nothing may be staged when the bucket is missing"
    );
}

#[test]
fn a_version_that_climbs_out_of_the_staging_root_fails_and_stages_nothing() {
    // `staging_root.join(app).join(version)` is composed from strings that
    // come out of pkg.lock, which Phase 3's `update` copies verbatim from a
    // scoop bucket -- an arbitrary third-party git repository.
    //
    // Everything else about this lock entry is consistent: the manifest at
    // the pinned commit really does say "../escape", so the version-equality
    // check passes and is not what stops this. Without the guard the write
    // lands at `<home>/escape`, one level above the staging root.
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let staging_root = home.path().join("manifests");
    fs::create_dir_all(&staging_root).unwrap();
    let outside = home.path().join("escape");
    let shas = bucket_repo(root.path(), "main", "tool.json", &["../../escape"]);

    let err = Scoop::new(root.path().to_path_buf())
        .stage(
            &staging_root,
            &Name::new("tool"),
            &pin("main", &shas[0], "../../escape"),
        )
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("path component"),
        "refuse it as a path, not incidentally as something else: {msg}"
    );
    assert_eq!(
        fs::read_dir(&staging_root).unwrap().count(),
        0,
        "nothing may be staged when the version is not a plain path component"
    );
    assert!(
        !outside.exists(),
        "the write escaped the staging root: {}",
        outside.display()
    );
}

#[test]
fn an_absolute_version_cannot_redirect_the_write_off_the_staging_root() {
    // The escape that needs no `..`: Path::join with an absolute component
    // discards the entire prefix. Measured before the guard: stage() returned
    // Ok and wrote the manifest under `elsewhere`, outside the staging root
    // entirely, with no `..` anywhere in the lock.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let absolute = elsewhere.path().join("planted");
    let version = absolute.to_string_lossy().to_string();
    let shas = bucket_repo(root.path(), "main", "tool.json", &[&version]);

    let err = Scoop::new(root.path().to_path_buf())
        .stage(
            stage_dir.path(),
            &Name::new("tool"),
            &pin("main", &shas[0], &version),
        )
        .unwrap_err();
    assert!(format!("{err:#}").contains("path component"), "got {err:#}");
    assert_eq!(
        fs::read_dir(stage_dir.path()).unwrap().count(),
        0,
        "nothing may be staged for an absolute version"
    );
    assert!(
        !absolute.exists(),
        "the manifest was written outside the staging root: {}",
        absolute.display()
    );
}

#[test]
fn a_bucket_name_that_is_a_path_cannot_point_git_at_another_repository() {
    // The bucket composes `$SCOOP/buckets/<bucket>`, and the result is the
    // directory `git` is then run in. An absolute bucket leaves the scoop
    // root entirely: without the guard this call SUCCEEDS, staging a manifest
    // recovered from a repository that is not one of this machine's buckets.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let shas = bucket_repo(elsewhere.path(), "main", "tool.json", &["1.0.0"]);
    let absolute_bucket = elsewhere
        .path()
        .join("buckets")
        .join("main")
        .to_string_lossy()
        .to_string();

    let err = Scoop::new(root.path().to_path_buf())
        .stage(
            stage_dir.path(),
            &Name::new("tool"),
            &pin(&absolute_bucket, &shas[0], "1.0.0"),
        )
        .unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("path component"),
        "an absolute bucket must be refused as a path, got: {msg}"
    );
    assert_eq!(fs::read_dir(stage_dir.path()).unwrap().count(), 0);
}

#[test]
fn a_spelling_neither_guess_finds_is_resolved_from_the_tree() {
    // `MIXEDCASE` and its folded form `mixedcase` both miss `MixedCase.json`.
    // One tree listing finds the real name -- and uses it, rather than only
    // reporting it. Without this third attempt the two cheap guesses only
    // work when the user's casing happens to match.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "MixedCase.json", &["1.0.0"]);

    let staged = Scoop::new(root.path().to_path_buf())
        .stage(
            stage_dir.path(),
            &Name::new("MIXEDCASE"),
            &pin("main", &shas[0], "1.0.0"),
        )
        .unwrap();
    assert_eq!(
        staged.file_name().unwrap(),
        "MixedCase.json",
        "got {}",
        staged.display()
    );
}

#[test]
fn an_app_the_bucket_simply_does_not_have_fails() {
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "tool.json", &["1.0.0"]);

    let err = Scoop::new(root.path().to_path_buf())
        .stage(
            stage_dir.path(),
            &Name::new("nosuch"),
            &pin("main", &shas[0], "1.0.0"),
        )
        .unwrap_err();
    assert!(format!("{err:#}").contains("nosuch"), "got {err:#}");
}

#[test]
fn a_winget_pin_in_the_scoop_map_is_an_error_not_a_panic() {
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let err = Scoop::new(root.path().to_path_buf())
        .stage(
            stage_dir.path(),
            &Name::new("tool"),
            &Pin::WingetVersion {
                version: "1.0.0".into(),
            },
        )
        .unwrap_err();
    assert!(format!("{err:#}").contains("inconsistent"));
}

use dotpkg::backend::scoop::download_argv;

#[test]
fn the_download_argv_never_skips_hash_verification() {
    // The approved design forbids --skip-hash-check and this is the one place
    // it would be tempting. Hash verification is scoop's, and dotpkg does not
    // opt out of it.
    //
    // This is the whole of what a test in this repository can honestly prove
    // about the download step: what scoop then does with the argv was
    // measured, not asserted, and is covered by the Windows dogfood.
    //
    // Asserted as the whole argv rather than as "no skip-hash flag appears":
    // a predicate that only looks for the flags thought of today cannot fail
    // for any argv that grows a new one, and this equality also pins the
    // manifest path that the sibling test checks.
    let argv = download_argv(Path::new("/stage/tool/1.0.0/tool.json"), None);
    assert_eq!(
        argv,
        vec![
            "download".to_string(),
            "/stage/tool/1.0.0/tool.json".to_string()
        ]
    );
}

#[test]
fn the_download_argv_carries_the_resolved_architecture() {
    // `install_argv` has had this coverage since the previous task;
    // `download_argv` never has, and it is the one `stage_and_fetch` actually
    // calls during prepare. Asserted as the whole argv, for the same reason
    // as the test above: a predicate that only checks "does -a appear
    // somewhere" cannot fail if it appears in the wrong position, or if
    // "arm64" is dropped while "-a" survives.
    let argv = download_argv(Path::new("/stage/tool/1.0.0/tool.json"), Some("arm64"));
    assert_eq!(
        argv,
        vec![
            "download".to_string(),
            "-a".to_string(),
            "arm64".to_string(),
            "/stage/tool/1.0.0/tool.json".to_string()
        ]
    );
}

#[test]
fn the_download_argv_names_the_staged_manifest() {
    let argv = download_argv(Path::new("/stage/tool/1.0.0/tool.json"), None);
    assert_eq!(argv[0], "download");
    assert!(
        argv.iter().any(|a| a.ends_with("tool.json")),
        "the staged path is the point: {argv:?}"
    );
}

#[test]
fn cloning_is_only_offered_for_a_bucket_pkg_toml_declares_with_a_url() {
    // Never a guessed URL: a lock naming an undeclared bucket is a failure
    // that says so.
    let cfg = dotpkg::config::parse(
        "[scoop]\nbuckets = [\"main\", \"xom11=https://example.invalid/b\"]\n",
    )
    .unwrap();
    let argvs: Vec<Vec<String>> = cfg
        .scoop
        .buckets
        .iter()
        .map(dotpkg::backend::scoop::bucket_add_argv)
        .collect();
    assert_eq!(argvs[0], vec!["bucket", "add", "main"]);
    assert_eq!(
        argvs[1],
        vec!["bucket", "add", "xom11", "https://example.invalid/b"]
    );
}

#[test]
fn the_scoop_entry_point_is_the_cmd_shim() {
    // Measured: scoop.ps1 cannot be exec'd by Command, and relying on PATH
    // picks up whatever the user's shell resolves. shims/scoop.cmd runs
    // non-interactively and exits 0.
    let root = tempfile::tempdir().unwrap();
    let exe = Scoop::new(root.path().to_path_buf()).scoop_exe();
    assert_eq!(exe.file_name().unwrap(), "scoop.cmd");
    assert_eq!(exe.parent().unwrap().file_name().unwrap(), "shims");

    // Both sides are canonicalised before comparing, and the comparison is on
    // the resolved directory rather than on a path prefix. `Scoop::new`
    // deliberately strips Windows' `\\?\` extended-length prefix -- that is
    // what `strip_extended_prefix` exists for -- so the stored root is NOT
    // what `fs::canonicalize` returns on Windows. The old
    // `exe.starts_with(canonicalize(root))` therefore passed on macOS and
    // Linux and failed on Windows, the one platform this tool runs on. Found
    // by building the branch on the dogfood machine.
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    assert_eq!(
        std::fs::canonicalize(exe.parent().unwrap().parent().unwrap()).unwrap(),
        std::fs::canonicalize(root.path()).unwrap(),
        "the shim must sit directly under the scoop root"
    );
}

use dotpkg::apply::{prepare, Outcome};
use dotpkg::execute::{CommandReport, Mutator};
use std::cell::RefCell;

/// A fake scoop that only ever downloads. It records the argv it was handed
/// and reports scoop's measured success shape.
///
/// It deliberately cannot uninstall or install: `prepare` must never reach
/// those, and a fake that silently permits them could not prove it.
struct Downloader {
    calls: RefCell<Vec<(std::path::PathBuf, Option<String>)>>,
    verified: bool,
}

impl Downloader {
    fn ok() -> Downloader {
        Downloader {
            calls: RefCell::new(Vec::new()),
            verified: true,
        }
    }
    fn hash_failure() -> Downloader {
        Downloader {
            calls: RefCell::new(Vec::new()),
            verified: false,
        }
    }
}

impl Mutator for Downloader {
    fn uninstall(&self, app: &dotpkg::model::Name) -> anyhow::Result<CommandReport> {
        panic!("prepare must never uninstall anything, but it asked for {app}");
    }
    fn install(&self, m: &Path, _a: Option<&str>) -> anyhow::Result<CommandReport> {
        panic!(
            "prepare must never install anything, but it asked for {}",
            m.display()
        );
    }
    fn download(&self, manifest: &Path, arch: Option<&str>) -> anyhow::Result<CommandReport> {
        self.calls
            .borrow_mut()
            .push((manifest.to_path_buf(), arch.map(str::to_string)));
        // Both branches exit 0. Measured on a14: scoop reports a hash failure
        // through stdout and nothing else.
        let stdout = if self.verified {
            "Checking hash of tool-1.0.0.zip ... ok.\n'tool' (1.0.0) was downloaded successfully!\n"
        } else {
            "Checking hash of tool-1.0.0.zip ... ERROR Hash check failed!\n\
             'tool' (1.0.0) was downloaded successfully!\n"
        };
        Ok(CommandReport {
            code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
        })
    }
}

fn one_install_plan(name: &str, version: &str, arch: Option<&str>) -> dotpkg::plan::Plan {
    dotpkg::plan::Plan {
        actions: vec![dotpkg::plan::Action::Install {
            backend: dotpkg::model::SCOOP.into(),
            name: Name::new(name),
            version: version.into(),
            arch: arch.map(str::to_string),
        }],
    }
}

#[test]
fn a_real_ready_to_fetch_is_produced_by_production_code_and_carries_the_architecture() {
    // Two things at once, both of which Phase 2b-2 left unproven on every
    // platform: that `Outcome::ReadyToFetch` is reachable from real code at
    // all (every value of it in the suite was hand-built), and that the
    // architecture the planner resolved actually reaches the download argv.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "tool.json", &["1.0.0"]);
    let scoop = Scoop::new(root.path().to_path_buf());
    let declared =
        dotpkg::config::parse("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n").unwrap();
    let mut lock = dotpkg::lock::Lock::default();
    lock.scoop
        .insert(Name::new("tool"), pin("main", &shas[0], "1.0.0"));

    let fake = Downloader::ok();
    let prep = prepare(
        &one_install_plan("tool", "1.0.0", Some("arm64")),
        &lock,
        &scoop,
        &fake,
        stage_dir.path(),
        &declared,
    );

    let staged = match &prep.prepared[0].outcome {
        Outcome::ReadyToFetch { manifest } => manifest.clone(),
        other => panic!("expected ReadyToFetch from real code, got {other:?}"),
    };
    assert!(staged.exists(), "the manifest must really be on disk");

    let calls = fake.calls.borrow();
    assert_eq!(calls.len(), 1, "exactly one download: {calls:?}");
    assert_eq!(
        calls[0].0, staged,
        "download must be handed the staged path"
    );
    assert_eq!(
        calls[0].1.as_deref(),
        Some("arm64"),
        "the architecture the plan resolved must reach the download argv"
    );
}

#[test]
fn a_hash_failure_that_exits_zero_is_still_a_failed_outcome() {
    // The positive control's sibling. Without it, a `download` that ignored
    // its stdout entirely would pass the test above.
    let root = tempfile::tempdir().unwrap();
    let stage_dir = tempfile::tempdir().unwrap();
    let shas = bucket_repo(root.path(), "main", "tool.json", &["1.0.0"]);
    let scoop = Scoop::new(root.path().to_path_buf());
    let declared =
        dotpkg::config::parse("[scoop]\nbuckets = [\"main\"]\npackages = [\"tool\"]\n").unwrap();
    let mut lock = dotpkg::lock::Lock::default();
    lock.scoop
        .insert(Name::new("tool"), pin("main", &shas[0], "1.0.0"));

    let prep = prepare(
        &one_install_plan("tool", "1.0.0", None),
        &lock,
        &scoop,
        &Downloader::hash_failure(),
        stage_dir.path(),
        &declared,
    );

    match &prep.prepared[0].outcome {
        Outcome::Failed { why } => assert!(
            why.contains("hash"),
            "name the diagnosis, not just that it failed: {why}"
        ),
        other => panic!("a hash failure must not be ready: {other:?}"),
    }
}
