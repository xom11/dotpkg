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
            &pin("extras", "abc123", "1.0.0"),
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
    let argv = download_argv(Path::new("/stage/tool/1.0.0/tool.json"));
    assert_eq!(
        argv,
        vec![
            "download".to_string(),
            "/stage/tool/1.0.0/tool.json".to_string()
        ]
    );
}

#[test]
fn the_download_argv_names_the_staged_manifest() {
    let argv = download_argv(Path::new("/stage/tool/1.0.0/tool.json"));
    assert_eq!(argv[0], "download");
    assert!(
        argv.iter().any(|a| a.ends_with("tool.json")),
        "the staged path is the point: {argv:?}"
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
    assert!(exe.starts_with(std::fs::canonicalize(root.path()).unwrap()));
}
