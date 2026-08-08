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
fn skips_the_scoop_directory_itself_regardless_of_case() {
    // A case-different `apps/Scoop` is still scoop managing itself. Scanning
    // it as a package would make it a stray, and in Phase 2b a prune target.
    let dir = tempfile::tempdir().unwrap();
    app(dir.path(), "Scoop", "0.5.3", "64bit", "main");
    app(dir.path(), "fzf", "0.74.2", "arm64", "main");

    let got = Scoop::new(dir.path().to_path_buf())
        .scan()
        .unwrap()
        .installed;
    assert_eq!(got.len(), 1, "got {got:?}");
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

/// Install a real manifest from `tests/fixtures/scoop-manifests` as an app.
fn app_from_fixture(root: &Path, name: &str, arch: &str) {
    let dir = root.join("apps").join(name).join("current");
    fs::create_dir_all(&dir).unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/scoop-manifests")
        .join(format!("{name}.json"));
    fs::copy(&src, dir.join("manifest.json"))
        .unwrap_or_else(|e| panic!("copying {}: {e}", src.display()));
    fs::write(
        dir.join("install.json"),
        format!(r#"{{"bucket":"main","architecture":"{arch}"}}"#),
    )
    .unwrap();
}

fn bins_of(root: &Path, name: &str) -> Vec<String> {
    let scan = Scoop::new(root.to_path_buf()).scan().unwrap();
    assert!(scan.warnings.is_empty(), "got {:?}", scan.warnings);
    let inst = scan
        .installed
        .into_iter()
        .find(|i| i.name == name)
        .unwrap_or_else(|| panic!("{name} not scanned"));
    inst.bins
}

#[test]
fn a_bare_string_bin_yields_one_executable() {
    let dir = tempfile::tempdir().unwrap();
    app_from_fixture(dir.path(), "fzf", "arm64");
    assert_eq!(bins_of(dir.path(), "fzf"), vec!["fzf"]);
}

#[test]
fn a_list_of_strings_yields_all_of_them() {
    let dir = tempfile::tempdir().unwrap();
    app_from_fixture(dir.path(), "age", "64bit");
    assert_eq!(
        bins_of(dir.path(), "age"),
        vec!["age", "age-inspect", "age-keygen", "age-plugin-batchpass"]
    );
}

#[test]
fn a_path_is_reduced_to_its_basename_and_the_package_name_is_not_assumed() {
    // The finding: the package is `neovim`, the process is `nvim.exe`.
    let dir = tempfile::tempdir().unwrap();
    app_from_fixture(dir.path(), "neovim", "arm64");
    assert_eq!(bins_of(dir.path(), "neovim"), vec!["nvim", "xxd"]);
}

#[test]
fn a_mixed_list_of_strings_and_alias_pairs_yields_both_forms() {
    let dir = tempfile::tempdir().unwrap();
    app_from_fixture(dir.path(), "python", "64bit");
    assert_eq!(
        bins_of(dir.path(), "python"),
        vec!["idle", "idle3", "python", "python3"]
    );
}

#[test]
fn bins_under_every_architecture_and_shortcuts_are_all_collected() {
    // kanata is why this matters. It declares no top-level bin; its executable
    // is kanata_windows_tty_winIOv2_arm64.exe and only the shim alias is
    // `Kanata`. Reading just the installed architecture, or just `bin`, leaves
    // the keyboard remapper unprotected -- and losing it costs the keyboard on
    // the machine you would need to fix it.
    let dir = tempfile::tempdir().unwrap();
    app_from_fixture(dir.path(), "kanata", "arm64");
    assert_eq!(
        bins_of(dir.path(), "kanata"),
        vec![
            "kanata",
            "kanata-cmd",
            "kanata_windows_gui_winiov2_arm64",
            "kanata_windows_gui_winiov2_cmd_allowed_arm64",
            "kanata_windows_gui_winiov2_cmd_allowed_x64",
            "kanata_windows_gui_winiov2_x64",
            "kanata_windows_tty_winiov2_arm64",
            "kanata_windows_tty_winiov2_cmd_allowed_arm64",
            "kanata_windows_tty_winiov2_cmd_allowed_x64",
            "kanata_windows_tty_winiov2_x64",
        ]
    );
}

#[test]
fn a_manifest_naming_no_executable_yields_none_rather_than_guessing() {
    // nodejs uses env_add_path. Inventing `nodejs` here would be a guess that
    // never matches the real process, which is `node`.
    let dir = tempfile::tempdir().unwrap();
    app_from_fixture(dir.path(), "nodejs", "arm64");
    assert_eq!(bins_of(dir.path(), "nodejs"), Vec::<String>::new());
}

#[test]
fn a_dot_com_bin_is_stripped_the_same_as_dot_exe() {
    // `.com` is the one non-`.exe` extension that is ever a live Windows
    // process image (see sys::EXECUTABLE_SUFFIXES). If this side and the live
    // process side ever strip a different suffix list, a `.com` package's
    // manifest and its running process stop agreeing on its name.
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().join("apps").join("tool").join("current");
    fs::create_dir_all(&d).unwrap();
    fs::write(
        d.join("manifest.json"),
        r#"{"version":"1.0","bin":"tool.com"}"#,
    )
    .unwrap();

    let got = Scoop::new(dir.path().to_path_buf())
        .scan()
        .unwrap()
        .installed;
    assert_eq!(got.len(), 1, "got {got:?}");
    assert_eq!(got[0].bins, vec!["tool"]);
}

use dotpkg::model::Name;
use dotpkg::sys::Process;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn proc(name: &str, exe: Option<PathBuf>) -> Process {
    Process {
        name: name.to_string(),
        exe,
    }
}

#[test]
fn a_process_running_out_of_an_app_directory_names_that_app() {
    // nodejs is why this exists: its manifest names no executable at all, so
    // the path is the only signal there is.
    let root = PathBuf::from("/tmp/dpk-root");
    let got = Scoop::new(root.clone()).running_apps(&[proc(
        "node",
        Some(root.join("apps/nodejs/current/node.exe")),
    )]);
    assert_eq!(got, BTreeSet::from([Name::new("nodejs")]));
}

#[test]
fn the_persist_tree_counts_too_because_rustup_lives_there() {
    // rustup's env_add_path is `.cargo\bin`, which scoop puts under
    // persist/rustup, outside apps entirely.
    let root = PathBuf::from("/tmp/dpk-root");
    let got = Scoop::new(root.clone()).running_apps(&[proc(
        "cargo",
        Some(root.join("persist/rustup/.cargo/bin/cargo.exe")),
    )]);
    assert_eq!(got, BTreeSet::from([Name::new("rustup")]));
}

#[test]
fn a_process_with_no_readable_path_is_not_an_error() {
    // sysinfo reports None for a process at a higher integrity level. That is
    // the case name matching covers, so this must simply contribute nothing.
    let root = PathBuf::from("/tmp/dpk-root");
    let got = Scoop::new(root).running_apps(&[proc("kanata", None)]);
    assert!(got.is_empty());
}

#[test]
fn a_process_outside_the_scoop_tree_names_nothing() {
    let root = PathBuf::from("/tmp/dpk-root");
    let got =
        Scoop::new(root).running_apps(&[proc("node", Some(PathBuf::from("/usr/local/bin/node")))]);
    assert!(got.is_empty());
}

#[test]
fn a_sibling_directory_with_a_shared_prefix_is_not_the_apps_tree() {
    // `.../scoop/appsbackup/x.exe` must not read as app `backup`.
    let root = PathBuf::from("/tmp/dpk-root");
    let got = Scoop::new(root.clone())
        .running_apps(&[proc("x", Some(root.join("appsbackup/backup/x.exe")))]);
    assert!(got.is_empty(), "got {got:?}");
}

#[test]
fn path_matching_folds_case_like_the_filesystem() {
    let root = PathBuf::from("/tmp/DPK-Root");
    let got = Scoop::new(root).running_apps(&[proc(
        "node",
        Some(PathBuf::from("/tmp/dpk-root/Apps/NodeJS/current/node.exe")),
    )]);
    assert_eq!(got, BTreeSet::from([Name::new("nodejs")]));
}

#[test]
fn windows_paths_with_backslashes_and_a_resolved_version_dir_match() {
    // The only shape that occurs on the real machine, and the one a test
    // written on a Mac is most likely to miss: separators are backslashes, and
    // sysinfo may report the version directory the `current` junction resolves
    // to rather than `current` itself. Either way the segment after `apps` is
    // the app name.
    let root = PathBuf::from(r"C:\Users\kln\scoop");
    let got = Scoop::new(root).running_apps(&[proc(
        "nvim",
        Some(PathBuf::from(
            r"C:\Users\kln\scoop\apps\neovim\0.12.4\bin\nvim.exe",
        )),
    )]);
    assert_eq!(got, BTreeSet::from([Name::new("neovim")]));
}

#[test]
fn a_manifest_that_is_not_a_file_warns_rather_than_vanishing() {
    // The READ branch, as distinct from the parse branch already covered.
    // Reverting that branch to swallow every error left the whole suite green,
    // which is what makes this test worth its lines.
    //
    // Making manifest.json a DIRECTORY is the portable trigger: it yields a
    // non-NotFound error on every platform, unlike a permission denial.
    let dir = tempfile::tempdir().unwrap();
    app(dir.path(), "fzf", "0.74.2", "arm64", "main");
    fs::create_dir_all(
        dir.path()
            .join("apps")
            .join("unreadable")
            .join("current")
            .join("manifest.json"),
    )
    .unwrap();

    let scan = Scoop::new(dir.path().to_path_buf()).scan().unwrap();
    assert_eq!(scan.installed.len(), 1, "got {:?}", scan.installed);
    assert_eq!(scan.warnings.len(), 1, "got {:?}", scan.warnings);
    assert!(
        scan.warnings[0].contains("unreadable"),
        "the warning must name the app: {:?}",
        scan.warnings
    );
}
