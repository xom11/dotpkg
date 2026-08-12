use dotpkg::backend::scoop::Scoop;
use dotpkg::backend::Backend;
use std::fs;
use std::path::Path;

#[test]
fn the_backend_reports_the_name_every_map_and_guard_is_keyed_by() {
    // state.json is a map keyed by this string, plan() compares against
    // model::SCOOP, and owned_count(SCOOP) is what mass_prune_guard reads.
    // Mutating it to "" or "xyzzy" left the whole suite green.
    let s = Scoop::new(std::path::PathBuf::from("/nonexistent"));
    assert_eq!(Backend::name(&s), dotpkg::model::SCOOP);
    assert_eq!(Backend::name(&s), "scoop");
}

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
fn a_half_finished_install_with_no_manifest_yet_is_silent_not_opaque() {
    // The NotFound arm is the benign one and must stay benign: this is the
    // `Err(e) if e.kind() == NotFound => <benign default>` idiom whose other
    // error kinds went untested in three places across this crate.
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("apps").join("fzf").join("current")).unwrap();
    let scan = Backend::scan(&Scoop::new(root.path().to_path_buf())).unwrap();
    assert!(scan.installed.is_empty());
    assert!(
        scan.opaque.is_empty(),
        "an absent manifest is not an unreadable one"
    );
    assert!(scan.warnings.is_empty(), "and says nothing");
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
fn an_app_whose_manifest_cannot_be_read_is_reported_as_opaque_not_as_absent() {
    let root = tempfile::tempdir().unwrap();
    let current = root.path().join("apps").join("zellij").join("current");
    std::fs::create_dir_all(current.join("manifest.json")).unwrap(); // a DIRECTORY
    let scan = Backend::scan(&Scoop::new(root.path().to_path_buf())).unwrap();
    assert!(scan.installed.is_empty(), "got {:?}", scan.installed);
    assert_eq!(
        scan.opaque,
        vec![Name::new("zellij")],
        "the name must survive"
    );
    assert_eq!(scan.warnings.len(), 1, "and still be explained to the user");
    assert!(
        scan.warnings[0].contains("zellij"),
        "got {:?}",
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

#[test]
fn a_bin_nested_inside_a_json_array_is_still_collected() {
    // `declared_executables::walk`'s `Value::Array` arm exists for a shape
    // `bin`/`shortcuts` matching alone cannot reach: a manifest key whose
    // value is itself an array of objects, each carrying its own `bin`.
    // Deleting that arm drops straight into the `_ => {}` wildcard and this
    // executable vanishes -- and nothing before this test ever put a `bin`
    // behind an array, so the whole suite stayed green while it did.
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().join("apps").join("tool").join("current");
    fs::create_dir_all(&d).unwrap();
    fs::write(
        d.join("manifest.json"),
        r#"{"version":"1.0","variants":[{"bin":"foo.exe"}]}"#,
    )
    .unwrap();

    let got = Scoop::new(dir.path().to_path_buf())
        .scan()
        .unwrap()
        .installed;
    assert_eq!(got.len(), 1, "got {got:?}");
    assert_eq!(got[0].bins, vec!["foo"]);
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

use dotpkg::config::{BucketDecl, Config, ScoopSection};
use dotpkg::execute::{CommandReport, Mutator};

fn one_bucket_config(name: &str) -> Config {
    Config {
        scoop: ScoopSection {
            buckets: vec![BucketDecl {
                name: Name::new(name),
                url: None,
            }],
            ..Default::default()
        },
        ..Default::default()
    }
}

/// A fake `bucket_add`. `clone_missing_buckets` never calls `uninstall`,
/// `install` or `download`, so those panic if reached rather than silently
/// returning something plausible.
///
/// The two `Ok` shapes mirror what a real `scoop bucket add` was measured to
/// do (see `clone_missing_buckets`'s own doc comment): it can really clone
/// (`.git` appears under the bucket directory) or it can exit 0 having done
/// nothing (`.git` does not appear) -- the silent-success trap the post-run
/// `.git` re-check exists to catch. Before this seam existed, no test could
/// produce the second shape at all: `self.run` against a nonexistent
/// `scoop.cmd` can only ever return `Err`.
struct FakeBucketAdd {
    root: PathBuf,
    clones_for_real: bool,
    fails: bool,
}

impl Mutator for FakeBucketAdd {
    fn uninstall(&self, _app: &Name) -> anyhow::Result<CommandReport> {
        unreachable!("clone_missing_buckets never calls uninstall")
    }
    fn install(&self, _manifest: &Path, _arch: Option<&str>) -> anyhow::Result<CommandReport> {
        unreachable!("clone_missing_buckets never calls install")
    }
    fn download(&self, _manifest: &Path, _arch: Option<&str>) -> anyhow::Result<CommandReport> {
        unreachable!("clone_missing_buckets never calls download")
    }
    fn bucket_add(&self, bucket: &BucketDecl) -> anyhow::Result<CommandReport> {
        if self.fails {
            anyhow::bail!("cannot run fake scoop");
        }
        if self.clones_for_real {
            let git = self
                .root
                .join("buckets")
                .join(bucket.name.key())
                .join(".git");
            fs::create_dir_all(git).unwrap();
        }
        Ok(CommandReport {
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

#[test]
fn a_declared_bucket_already_on_disk_is_left_alone() {
    // A bucket directory that already has `.git` must be skipped outright --
    // no clone attempt, no failure entry -- before `bucket_add` is ever
    // called. The mutator here panics if it is reached at all, which is a
    // stronger guarantee than merely checking `failed` ends up empty: it
    // proves the pre-run skip guard, not a coincidence downstream of it.
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("buckets").join("main").join(".git")).unwrap();
    let cfg = one_bucket_config("main");
    struct PanicsIfCalled;
    impl Mutator for PanicsIfCalled {
        fn uninstall(&self, _app: &Name) -> anyhow::Result<CommandReport> {
            unreachable!()
        }
        fn install(&self, _m: &Path, _a: Option<&str>) -> anyhow::Result<CommandReport> {
            unreachable!()
        }
        fn download(&self, _m: &Path, _a: Option<&str>) -> anyhow::Result<CommandReport> {
            unreachable!()
        }
        fn bucket_add(&self, bucket: &BucketDecl) -> anyhow::Result<CommandReport> {
            panic!("must not be called: {} already has .git", bucket.name)
        }
    }

    let failed = Scoop::new(dir.path().to_path_buf()).clone_missing_buckets(&cfg, &PanicsIfCalled);

    assert!(failed.is_empty(), "got {failed:?}");
}

#[test]
fn a_bucket_add_that_really_clones_is_not_recorded_as_failed() {
    // The positive sibling to the silent-success test below: an `Ok` whose
    // `.git` genuinely appears afterwards is a real clone and must not be
    // reported. Without this test, a mutant that always takes the
    // empty-arm (`Ok(_) if true`) would look identical to correct behaviour.
    let dir = tempfile::tempdir().unwrap();
    let cfg = one_bucket_config("main");
    let mutator = FakeBucketAdd {
        root: dir.path().to_path_buf(),
        clones_for_real: true,
        fails: false,
    };

    let failed = Scoop::new(dir.path().to_path_buf()).clone_missing_buckets(&cfg, &mutator);

    assert!(failed.is_empty(), "got {failed:?}");
}

#[test]
fn a_bucket_add_that_reports_success_without_cloning_is_recorded_as_failed() {
    // The trap `clone_missing_buckets`'s own doc comment measures: `scoop
    // bucket add` can exit 0 having done nothing. Mutated to `true`, the
    // post-run guard would accept this `Ok` on its own and record nothing --
    // a clone that silently did nothing would look identical to one that
    // worked. Paired with the test above (same `Ok`, opposite `.git`
    // outcome), a mutation that always takes one branch cannot satisfy both.
    let dir = tempfile::tempdir().unwrap();
    let cfg = one_bucket_config("main");
    let mutator = FakeBucketAdd {
        root: dir.path().to_path_buf(),
        clones_for_real: false,
        fails: false,
    };

    let failed = Scoop::new(dir.path().to_path_buf()).clone_missing_buckets(&cfg, &mutator);

    assert_eq!(failed.len(), 1, "got {failed:?}");
    assert_eq!(failed[0].0, Name::new("main"));
}

#[test]
fn a_bucket_add_that_cannot_run_at_all_is_recorded_with_its_error_text() {
    // The `Err` arm, distinct from the two `Ok` shapes above: a bucket add
    // that could not even be attempted is recorded with a real, non-empty
    // reason rather than a stand-in default.
    let dir = tempfile::tempdir().unwrap();
    let cfg = one_bucket_config("main");
    let mutator = FakeBucketAdd {
        root: dir.path().to_path_buf(),
        clones_for_real: false,
        fails: true,
    };

    let failed = Scoop::new(dir.path().to_path_buf()).clone_missing_buckets(&cfg, &mutator);

    assert_eq!(failed.len(), 1, "got {failed:?}");
    assert_eq!(failed[0].0, Name::new("main"));
    assert!(
        failed[0].1.contains("cannot run fake scoop"),
        "got {:?}",
        failed[0].1
    );
}

use dotpkg::backend::{Scan, ScanOutcome};
use dotpkg::model::{Installed, SCOOP, WINGET};

fn installed_pkg(name: &str, bins: &[&str]) -> Installed {
    Installed {
        backend: SCOOP.to_string(),
        name: Name::new(name),
        version: "0".to_string(),
        arch: None,
        bucket: None,
        bins: bins.iter().map(|b| b.to_string()).collect(),
    }
}

fn installed_winget_pkg(name: &str, bins: &[&str]) -> Installed {
    Installed {
        backend: WINGET.to_string(),
        name: Name::new(name),
        version: "0".to_string(),
        arch: None,
        bucket: None,
        bins: bins.iter().map(|b| b.to_string()).collect(),
    }
}

// `running_set` unions name matching and path matching -- the design's central
// claim is that the two have non-overlapping blind spots and only their union
// covers every package. It used to be assembled inline in `main.rs`, which no
// test could reach; these three exercise the union itself, with fabricated
// `Process` values, on any OS.

#[test]
fn the_running_set_detects_a_package_reachable_only_by_path() {
    // nodejs's manifest names no executable at all; the live path under
    // apps/nodejs/current/ is the only signal there is.
    let root = PathBuf::from("/tmp/dpk-root");
    let scoop = Scoop::new(root.clone());
    let procs = [proc(
        "node",
        Some(root.join("apps/nodejs/current/node.exe")),
    )];

    // The two empty slices are the winget half -- ids and package roots --
    // which this test deliberately does not exercise.
    let running = dotpkg::backend::running_set(&scoop, &[], &[], &procs);

    assert!(running.covers(&installed_pkg("nodejs", &[])));
}

#[test]
fn the_running_set_detects_a_package_reachable_only_by_name() {
    // An elevated kanata: sysinfo cannot read its exe path (`exe: None`), so
    // the only signal is the live process name matching one of the
    // manifest's declared executables.
    let root = PathBuf::from("/tmp/dpk-root");
    let scoop = Scoop::new(root);
    let procs = [proc("kanata_windows_tty_winiov2_arm64", None)];

    // The two empty slices are the winget half -- ids and package roots --
    // which this test deliberately does not exercise.
    let running = dotpkg::backend::running_set(&scoop, &[], &[], &procs);

    assert!(running.covers(&installed_pkg(
        "kanata",
        &["kanata_windows_tty_winiov2_arm64"]
    )));
}

#[test]
fn the_running_set_detects_both_signals_at_once() {
    let root = PathBuf::from("/tmp/dpk-root");
    let scoop = Scoop::new(root.clone());
    let procs = [
        proc("node", Some(root.join("apps/nodejs/current/node.exe"))),
        proc("kanata_windows_tty_winiov2_arm64", None),
    ];

    // The two empty slices are the winget half -- ids and package roots --
    // which this test deliberately does not exercise.
    let running = dotpkg::backend::running_set(&scoop, &[], &[], &procs);

    assert!(running.covers(&installed_pkg("nodejs", &[])));
    assert!(running.covers(&installed_pkg(
        "kanata",
        &["kanata_windows_tty_winiov2_arm64"]
    )));
}

// `apply::sample_fence` is the ONE place production chooses what the fence sees,
// and it must union three signals, not two. This drives its tested seam,
// `sample_fence_with_roots`, so the assertions cover BOTH halves of the choice
// the three call sites used to make for themselves: extracting the winget ids
// out of a `ScanOutcome`, and unioning winget's package dirs into scoop's.
//
// A fabricated root is why the seam is split in two. `sample_fence` reads
// `package_roots()`, which returns empty on every non-Windows platform, so a
// test calling it could never exercise the winget path half at all -- see
// `sample_fence`'s own doc comment.
//
// Proven red twice, in both directions: dropping the winget `extend` from
// `backend::running_set` fails on "winget path half lost", and replacing
// `scoop.running_apps(procs)` with an empty set fails on "scoop path half lost".
#[test]
fn the_fence_unions_scoop_paths_with_winget_package_dirs() {
    let root = PathBuf::from("/tmp/dpk-root");
    let wg_root = PathBuf::from("/tmp/dpk-winget/Packages");
    let scoop = Scoop::new(root.clone());
    let procs = [
        // Caught only by its scoop path. Measured on a14: this is kanata's real
        // process name, and it resembles neither the package name nor any
        // prefix or suffix of it.
        proc(
            "kanata_windows_tty_winiov2_arm64",
            Some(root.join("apps/kanata/current/kanata_windows_tty_winIOv2_arm64.exe")),
        ),
        // Caught only by its winget package dir. Measured: the one live process
        // under WinGet\Packages on a14.
        proc(
            "vkey",
            Some(
                wg_root
                    .join("PhatMT97.VKey_Microsoft.Winget.Source_8wekyb3d8bbwe")
                    .join("VKey.exe"),
            ),
        ),
    ];
    // A real `ScanOutcome`, not a bare id list: the extraction from `installed`
    // is part of what this pins. `bins` deliberately EMPTY here too, so no
    // guard name can leak into the `names` half by way of the scan.
    let winget_scan = ScanOutcome::Scanned(Scan {
        installed: vec![installed_winget_pkg("PhatMT97.VKey", &[])],
        ..Scan::default()
    });
    let running = dotpkg::apply::sample_fence_with_roots(&scoop, &winget_scan, &[wg_root], &procs);

    assert!(
        running.covers(&installed_pkg("kanata", &[])),
        "scoop path half lost"
    );
    // `bins` deliberately EMPTY: with "vkey" in it this would pass on the
    // `names` half alone and prove nothing about the path half.
    assert!(
        running.covers(&installed_winget_pkg("PhatMT97.VKey", &[])),
        "winget path half lost"
    );
}

// The other direction of the same seam: a winget scan that FAILED contributes no
// fence entries, so the id is not held merely because a process happens to run
// under a winget package directory bearing its name. Without this, a
// `sample_fence_with_roots` that ignored `winget_scan` entirely and fabricated
// its own id list would pass the test above.
#[test]
fn a_winget_scan_that_failed_contributes_no_fence_entries() {
    let root = PathBuf::from("/tmp/dpk-root");
    let wg_root = PathBuf::from("/tmp/dpk-winget/Packages");
    let scoop = Scoop::new(root);
    let procs = [proc(
        "vkey",
        Some(
            wg_root
                .join("PhatMT97.VKey_Microsoft.Winget.Source_8wekyb3d8bbwe")
                .join("VKey.exe"),
        ),
    )];
    let winget_scan = ScanOutcome::Unscannable("list exited 1".to_string());

    let running = dotpkg::apply::sample_fence_with_roots(&scoop, &winget_scan, &[wg_root], &procs);

    assert!(
        !running.covers(&installed_winget_pkg("PhatMT97.VKey", &[])),
        "an Unscannable winget scan must contribute no ids to the fence"
    );
}

// `std::os::windows::fs::symlink_dir` needs Developer Mode or an elevated
// process. GitHub's `windows-latest` runners normally allow it, but that
// cannot be confirmed from a macOS development machine, and this suite's
// history includes tests that passed for reasons unrelated to what they
// claimed. Gating to unix is the honest choice: skipped-with-a-stated-reason
// on Windows CI, rather than a symlink call that might flake -- or silently
// no-op -- there.
#[cfg(unix)]
#[test]
fn a_root_reached_through_a_symlink_still_matches_running_processes() {
    // The hole: sysinfo reports resolved paths. A root reached through a
    // junction, a subst drive or a symlink prefix-compares against the wrong
    // string, running_apps silently returns nothing, and nodejs and rustup --
    // which have no other running signal -- become prunable while running.
    //
    // A symlink is the portable stand-in for a Windows junction.
    let real = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(real.path().join("apps/nodejs/current")).unwrap();

    let link_parent = tempfile::tempdir().unwrap();
    let link = link_parent.path().join("aliased-root");
    std::os::unix::fs::symlink(real.path(), &link).unwrap();

    // The process reports the REAL path, as sysinfo would -- fully resolved.
    // `real.path()` is not that by itself: on macOS it is `/var/folders/...`,
    // itself an alias for `/private/var/folders/...` (`/var` -> `/private/var`),
    // so it has to be canonicalised here for the same reason `resolve_root`
    // canonicalises the scoop root.
    let real_resolved = std::fs::canonicalize(real.path()).unwrap();
    let got = Scoop::new(link).running_apps(&[proc(
        "node",
        Some(real_resolved.join("apps/nodejs/current/node.exe")),
    )]);
    assert_eq!(
        got,
        BTreeSet::from([Name::new("nodejs")]),
        "aliased root must still match"
    );
}

// -- The `NotFound` idiom: unreadable is not empty -------------------------
//
// `Scoop::scan` maps one io error to a valid empty machine and every other to
// a failure:
//
//     Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Scan::default()),
//     Err(e) => return Err(e.into()),
//
// Replacing that guard with `true` makes **every** read failure read as "this
// machine has no scoop packages". A mutation run found it surviving, and no
// previous phase had recorded it, because nothing in the suite could produce a
// read_dir error that is not `NotFound`.
//
// **Why it is the most dangerous of the survivors rather than one more of
// them.** An empty scan is not a wrong number, it is the input that makes every
// owned package undeclared-and-absent, and `mass_prune_guard` is the only thing
// left between that and a plan full of prunes -- a guard the design itself
// describes as catching the case "far too late". So this mutant converts a
// permissions problem into a proposal to uninstall everything dotpkg owns.
//
// `#[cfg(unix)]` for the same reason as the symlink test above: mode bits are
// how a directory is made unreadable here, and asserting the Windows
// equivalent from a macOS machine would be claiming something nobody measured.
#[cfg(unix)]
#[test]
fn an_apps_directory_that_cannot_be_read_is_an_error_and_never_an_empty_machine() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    // A package inside, so "empty" is not merely the wrong answer, it is a
    // false statement about a directory that has something in it.
    app(dir.path(), "fzf", "0.74.2", "arm64", "main");
    let apps = dir.path().join("apps");

    fs::set_permissions(&apps, fs::Permissions::from_mode(0o000)).unwrap();
    // The control that keeps the assertion honest: root ignores mode bits
    // entirely, so a suite running as root would read this directory fine and
    // everything below would pass while measuring nothing.
    let readable_anyway = fs::read_dir(&apps).is_ok();
    let scanned = Scoop::new(dir.path().to_path_buf()).scan();
    // Restored before any assertion fires, or a failure here is buried under a
    // TempDir cleanup panic about a directory it cannot traverse.
    fs::set_permissions(&apps, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(
        !readable_anyway,
        "this test needs a directory it cannot read, and this process could read \
         it anyway -- running as root defeats the mode bits, so nothing below \
         would be measuring what it claims"
    );

    let err = scanned.expect_err(
        "an unreadable apps/ must fail the scan. Returning an empty Scan here \
         says 'no scoop packages are installed', which is the one input that \
         turns every owned package into a prune candidate",
    );
    let msg = format!("{err:#}");
    assert!(
        msg.to_lowercase().contains("permission"),
        "the failure must name what actually went wrong: {msg}"
    );
}
