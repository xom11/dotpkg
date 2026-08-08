use dotpkg::backend::scoop::Scoop;
use dotpkg::backend::Backend;
use std::fs;
use std::path::Path;

/// Build the parts of a scoop install that `scan` reads. Mirrors the real
/// layout: apps/<name>/current/{manifest,install}.json
fn app(root: &Path, name: &str, version: &str, arch: &str, bucket: &str) {
    let dir = root.join("apps").join(name).join("current");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("manifest.json"),
        format!(r#"{{"version":"{version}","description":"x"}}"#),
    )
    .unwrap();
    fs::write(
        dir.join("install.json"),
        format!(r#"{{"bucket":"{bucket}","architecture":"{arch}"}}"#),
    )
    .unwrap();
}

#[test]
fn reads_name_version_arch_and_bucket_for_each_app() {
    let dir = tempfile::tempdir().unwrap();
    app(dir.path(), "fzf", "0.74.2", "arm64", "main");
    app(dir.path(), "bat", "0.26.1", "64bit", "main");

    let scan = Scoop::new(dir.path().to_path_buf()).scan().unwrap();
    let mut got = scan.installed;
    got.sort_by(|a, b| a.name.cmp(&b.name));

    assert!(scan.warnings.is_empty(), "got {:?}", scan.warnings);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].name, "bat");
    assert_eq!(got[0].version, "0.26.1");
    assert_eq!(got[0].arch.as_deref(), Some("64bit"));
    assert_eq!(got[1].name, "fzf");
    assert_eq!(got[1].bucket.as_deref(), Some("main"));
    assert!(got.iter().all(|i| i.backend == "scoop"));
}

#[test]
fn skips_the_scoop_directory_itself() {
    // ~/scoop/apps/scoop is scoop managing itself, not a package.
    let dir = tempfile::tempdir().unwrap();
    app(dir.path(), "scoop", "0.5.3", "64bit", "main");
    app(dir.path(), "fzf", "0.74.2", "arm64", "main");

    let got = Scoop::new(dir.path().to_path_buf())
        .scan()
        .unwrap()
        .installed;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "fzf");
}

#[test]
fn an_app_installed_by_an_older_scoop_has_no_install_json_and_still_scans() {
    // install.json only appeared in later scoop versions. Treating "unknown
    // architecture" as "wrong architecture" would make dotpkg want to reinstall
    // it on every run.
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().join("apps").join("old").join("current");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("manifest.json"), r#"{"version":"1.0"}"#).unwrap();

    let got = Scoop::new(dir.path().to_path_buf())
        .scan()
        .unwrap()
        .installed;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "old");
    assert_eq!(got[0].arch, None);
    assert_eq!(got[0].bucket, None);
}

#[test]
fn a_directory_with_no_manifest_is_ignored_rather_than_failing_the_scan() {
    // A half-finished install must not take the whole run down -- and it is the
    // ordinary shape of one, so it earns no warning either.
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("apps").join("broken").join("current")).unwrap();
    app(dir.path(), "fzf", "0.74.2", "arm64", "main");

    let scan = Scoop::new(dir.path().to_path_buf()).scan().unwrap();
    assert_eq!(scan.installed.len(), 1);
    assert_eq!(scan.installed[0].name, "fzf");
    assert!(
        scan.warnings.is_empty(),
        "a missing manifest is expected, not newsworthy: {:?}",
        scan.warnings
    );
}

#[test]
fn a_manifest_that_cannot_be_read_is_skipped_with_a_warning_not_in_silence() {
    // The failure this separates out: an app that IS installed but whose
    // manifest is corrupt or unreadable. Dropping it silently makes it look
    // uninstalled -- and "uninstalled" is what Phase 2 offers to fix by
    // installing over the top of it. The scan still completes: the healthy
    // apps beside it are exactly what the user needs to see.
    let dir = tempfile::tempdir().unwrap();
    app(dir.path(), "fzf", "0.74.2", "arm64", "main");

    // Valid JSON, but no `version` -- the one field Installed cannot do without.
    let d = dir.path().join("apps").join("halfwritten").join("current");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("manifest.json"), r#"{"description":"x"}"#).unwrap();

    let scan = Scoop::new(dir.path().to_path_buf()).scan().unwrap();
    assert_eq!(scan.installed.len(), 1, "got {:?}", scan.installed);
    assert_eq!(scan.installed[0].name, "fzf");
    assert_eq!(scan.warnings.len(), 1, "got {:?}", scan.warnings);
    assert!(
        scan.warnings[0].contains("halfwritten"),
        "the warning must name the app: {:?}",
        scan.warnings
    );
}

#[test]
fn a_missing_scoop_root_scans_to_nothing() {
    let scan = Scoop::new("/definitely/not/here".into()).scan().unwrap();
    assert!(scan.installed.is_empty());
    assert!(scan.warnings.is_empty());
}

#[test]
fn a_mixed_case_app_directory_keeps_its_exact_name_on_display() {
    // `assert_eq!(got[0].name, "ripgrep")` folds case (`PartialEq<&str> for
    // Name`), so it would not notice `scan` lowercasing the directory name on
    // the way in. `.to_string()` goes through `Display`, which does not fold.
    let dir = tempfile::tempdir().unwrap();
    app(dir.path(), "RipGrep", "14.1.0", "64bit", "main");

    let got = Scoop::new(dir.path().to_path_buf())
        .scan()
        .unwrap()
        .installed;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name.to_string(), "RipGrep");
}
