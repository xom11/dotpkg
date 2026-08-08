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

    let mut got = Scoop::new(dir.path().to_path_buf()).scan().unwrap();
    got.sort_by(|a, b| a.name.cmp(&b.name));

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

    let got = Scoop::new(dir.path().to_path_buf()).scan().unwrap();
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

    let got = Scoop::new(dir.path().to_path_buf()).scan().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "old");
    assert_eq!(got[0].arch, None);
    assert_eq!(got[0].bucket, None);
}

#[test]
fn a_directory_with_no_manifest_is_ignored_rather_than_failing_the_scan() {
    // A half-finished install must not take the whole run down.
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("apps").join("broken").join("current")).unwrap();
    app(dir.path(), "fzf", "0.74.2", "arm64", "main");

    let got = Scoop::new(dir.path().to_path_buf()).scan().unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "fzf");
}

#[test]
fn a_missing_scoop_root_scans_to_nothing() {
    let got = Scoop::new("/definitely/not/here".into()).scan().unwrap();
    assert!(got.is_empty());
}
