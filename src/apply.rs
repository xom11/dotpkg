use crate::backend::scoop::Scoop;
use crate::config::Config;
use crate::lock::Lock;
use crate::lock::Pin;
use crate::model::SCOOP;
use crate::plan::{Action, Plan, SkipReason};
use crate::state::State;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Refuse a plan built from a config that declares nothing while dotpkg owns
/// something.
///
/// An empty or truncated `pkg.toml` parses successfully to zero packages —
/// every field is `#[serde(default)]` — and every owned package then becomes a
/// prune. Verified against the merged planner: five owned packages, empty
/// config, five prunes, no signal of any kind.
///
/// This is checked before anything else happens, and **`--yes` does not bypass
/// it**. `--yes` means "I have read the plan"; an empty config is file
/// corruption, so the plan itself is the thing that cannot be trusted.
/// Overriding takes its own flag.
///
/// Deliberately no ratio or count threshold. A user who genuinely deletes half
/// their `pkg.toml` is shown the plan and asked, which is the protection that
/// already exists.
///
/// TODO(phase-4-winget): scoop-only, both in what it reads and in what it
/// counts. A `pkg.toml` emptied of its `[winget]` section while dotpkg owns
/// winget packages passes this guard untouched. Deliberately left until the
/// winget backend exists — there is nothing that can prune a winget package
/// today, so widening it now would be an untested guard over an unimplemented
/// path — but it must be widened in the same change that adds one.
pub fn mass_prune_guard(declared: &Config, state: &State) -> Result<()> {
    if !declared.scoop.packages.is_empty() {
        return Ok(());
    }
    let owned = state.owned_count(SCOOP);
    anyhow::ensure!(
        owned == 0,
        "pkg.toml declares no scoop packages but dotpkg owns {owned}. \
         Refusing to prune everything. If the file is right, pass --allow-empty-config."
    );
    Ok(())
}

/// Refuse a lock that is incoherent in a way decidable without touching the
/// disk, before the plan is built and before anything is staged.
///
/// `Scoop::stage` re-checks the same rules, deliberately: this guard gives a
/// good whole-run message, and `stage` is a public API that Phase 3 will call
/// from somewhere else. Neither is allowed to be the only one.
///
/// Deliberately does not check the lock's bucket against `pkg.toml`'s
/// declared buckets: that is a per-package concern (Task 11 owns it, once a
/// parsed `BucketDecl` gives it a case-folded name to compare against), not a
/// whole-run one. A whole-run version of that check deadlocks -- drop a
/// package and its bucket line from `pkg.toml` together, and the stale lock
/// entry would fail every later `apply` before the plan is even built,
/// including the prune that would otherwise clear it.
pub fn lock_coherence_guard(lock: &Lock) -> Result<()> {
    for (name, pin) in &lock.scoop {
        let Pin::ScoopCommit {
            bucket,
            commit,
            version,
        } = pin
        else {
            anyhow::bail!(
                "pkg.lock [scoop.{name}] holds a winget pin. Run `dotpkg update` to rewrite it."
            );
        };
        crate::backend::scoop::ensure_plain_component(name, "bucket", bucket)
            .and_then(|()| crate::backend::scoop::ensure_plain_component(name, "version", version))
            .and_then(|()| {
                crate::backend::scoop::ensure_plain_component(name, "package name", name.key())
            })
            .and_then(|()| crate::backend::scoop::ensure_commit_hash(name, commit))
            .map_err(|e| e.context("pkg.lock is not usable. Run `dotpkg update` to rewrite it."))?;
    }
    Ok(())
}

/// `%LOCALAPPDATA%\dotpkg\manifests`, beside state.json.
///
/// Permanent, not temporary: `install.json` records this path, so a staging
/// directory that gets cleaned leaves the installed app pointing at a path
/// that no longer exists.
pub fn default_staging_root() -> PathBuf {
    match State::default_path().parent() {
        Some(dir) => dir.join("manifests"),
        None => PathBuf::from("manifests"),
    }
}

/// How the driver reads one planned action, before any work is attempted.
///
/// `NotLocked` and `Running` are both `Action::Skip` and `status` prints both
/// as `!`, but apply must treat them differently: the user can close a running
/// app and run again, whereas a missing lock entry is something apply may not
/// fix, because resolving a version itself is forbidden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Needs a manifest staged and fetched.
    NeedsArtifact,
    /// A removal: nothing to prepare, ready by definition.
    NoArtifactNeeded,
    /// Benign; does not fail the run.
    Skip(String),
    /// Fails the run.
    NotLocked,
    /// Informational line, passed through.
    Report,
}

/// Pure. The judgement call of the whole driver: everything downstream is
/// plumbing that acts on what this function decided.
pub fn classify(action: &Action) -> Intent {
    match action {
        Action::Install { .. } | Action::Upgrade { .. } | Action::Downgrade { .. } => {
            Intent::NeedsArtifact
        }
        Action::Prune { .. } => Intent::NoArtifactNeeded,
        Action::Skip {
            backend, reason, ..
        } => match reason {
            SkipReason::NotLocked => Intent::NotLocked,
            SkipReason::Running => Intent::Skip("running -- stop it first".to_string()),
            SkipReason::BackendNotImplemented => {
                Intent::Skip(format!("{backend} backend not implemented until phase 4"))
            }
        },
        Action::Unmanaged { .. } | Action::ArchDrift { .. } => Intent::Report,
    }
}

/// What became of one planned action once `prepare` tried it. Every variant
/// is the record of an attempt; none is the record of a mutation -- see
/// `prepare`'s own doc comment for why that split holds everywhere in this
/// phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Staged, fetched and hash-verified. The manifest is on disk at this
    /// path and an executor installs from it.
    ///
    /// Split from `ReadyToRemove` rather than carrying an
    /// `Option<PathBuf>`: as one variant, "no manifest" meant "this is a
    /// removal" only for values `prepare()` itself produced, and any other
    /// construction -- a test, a future caller -- could say `Ready { manifest:
    /// None }` about an `Install` without the compiler objecting. An executor
    /// branching on `manifest.is_none()` to decide whether to *uninstall*
    /// would then be right by luck.
    ReadyToFetch { manifest: PathBuf },
    /// A removal: ready by definition, because there is nothing to fetch.
    ReadyToRemove,
    /// A per-package failure. Reported; never stops the run.
    Failed { why: String },
    /// Benign: the user can fix this (usually by closing an app) and run
    /// again. Does not fail the run.
    Skipped { why: String },
    /// Declared with no lock entry. Fails the run, because resolving a
    /// version is `update`'s job, not `apply`'s.
    NotLocked,
    /// Passed through from the plan (`Unmanaged`, `ArchDrift`): informational,
    /// and affects neither a count nor the verdict.
    Report,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub action: Action,
    pub outcome: Outcome,
}

/// The result of one `apply --prepare` run, whole.
///
/// **What `is_ok() == false` obliges an executor to do:** treat the run as
/// refused and perform *none* of its actions -- not the ready ones either.
/// That is what Phase 2b-1 ships (`main` prints this and exits 1 without an
/// executor existing at all), and the conservative reading of a design that
/// does not say. Phase 2b-2 may decide to narrow it -- "install the ready
/// ones, report the rest" is a defensible product choice -- but it is a
/// decision that must be written down and tested there, not inherited by
/// accident from this type's shape.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Preparation {
    pub prepared: Vec<Prepared>,
}

impl Preparation {
    /// Both ready shapes: an install with its manifest fetched, and a removal
    /// that needed nothing fetched. Counted together because the user-facing
    /// number is "how much of this plan can go ahead".
    pub fn ready_count(&self) -> usize {
        self.prepared
            .iter()
            .filter(|p| {
                matches!(
                    p.outcome,
                    Outcome::ReadyToFetch { .. } | Outcome::ReadyToRemove
                )
            })
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.prepared
            .iter()
            .filter(|p| matches!(p.outcome, Outcome::Failed { .. }))
            .count()
    }

    pub fn skipped_count(&self) -> usize {
        self.prepared
            .iter()
            .filter(|p| matches!(p.outcome, Outcome::Skipped { .. }))
            .count()
    }

    pub fn not_locked_count(&self) -> usize {
        self.prepared
            .iter()
            .filter(|p| matches!(p.outcome, Outcome::NotLocked))
            .count()
    }

    /// The run's verdict. `NotLocked` fails it for the same reason `Failed`
    /// does -- apply has no way to fix either one itself -- unlike `Skipped`,
    /// which the user can clear from outside dotpkg and rerun.
    pub fn is_ok(&self) -> bool {
        self.failed_count() == 0 && self.not_locked_count() == 0
    }
}

/// Walk the plan and try to make every `NeedsArtifact` action ready: recover
/// its pinned manifest, stage it under `staging_root`, then fetch and
/// hash-verify it. This is the entire phase: nothing here installs,
/// uninstalls, or otherwise changes anything already on the machine. The only
/// filesystem writes are inside `staging_root`, and the only command ever run
/// is `scoop download`, which never mutates installed software.
///
/// A per-package failure is recorded in that package's `Outcome` and the walk
/// continues -- one bad package must never hide, or stop, the others.
pub fn prepare(plan: &Plan, lock: &Lock, scoop: &Scoop, staging_root: &Path) -> Preparation {
    let prepared = plan
        .actions
        .iter()
        .map(|action| Prepared {
            action: action.clone(),
            outcome: match classify(action) {
                Intent::NeedsArtifact => stage_and_fetch(action, lock, scoop, staging_root),
                Intent::NoArtifactNeeded => Outcome::ReadyToRemove,
                Intent::Skip(why) => Outcome::Skipped { why },
                Intent::NotLocked => Outcome::NotLocked,
                Intent::Report => Outcome::Report,
            },
        })
        .collect();
    Preparation { prepared }
}

/// The `NeedsArtifact` half of `prepare`, kept separate so the walk above
/// reads as a plain classify-and-dispatch.
///
/// Every error path returns `Outcome::Failed` rather than propagating a
/// `Result` or panicking: a `Preparation` must always be total over the
/// plan it was given, because the whole point of the phase is that one
/// package's problem is reported, not fatal to the run.
fn stage_and_fetch(action: &Action, lock: &Lock, scoop: &Scoop, staging_root: &Path) -> Outcome {
    let (Action::Install {
        backend,
        name,
        arch,
        ..
    }
    | Action::Upgrade {
        backend,
        name,
        arch,
        ..
    }
    | Action::Downgrade {
        backend,
        name,
        arch,
        ..
    }) = action
    else {
        // classify() is the only caller, and it returns Intent::NeedsArtifact
        // only for the three variants matched above. Kept as a Failed
        // outcome, not a panic or an unreachable!(), so a future mismatch
        // between the two functions is a reported failure instead of a
        // crashed run.
        return Outcome::Failed {
            why: format!("{action:?} was classified as needing an artifact but names none"),
        };
    };
    if backend != SCOOP {
        return Outcome::Failed {
            why: format!("{backend}: no backend implementation can stage an artifact yet"),
        };
    }
    let Some(pin) = lock.scoop.get(name) else {
        // The planner itself would have produced Skip{NotLocked} instead of
        // an Install/Upgrade/Downgrade if the lock had no entry for this
        // name, so this is unreachable given a plan that actually came from
        // plan::plan(). Still handled rather than assumed.
        return Outcome::Failed {
            why: format!("{name}: no lock entry (the planner should have caught this)"),
        };
    };
    let staged = scoop.stage(staging_root, name, pin).and_then(|manifest| {
        scoop.download(&manifest, arch.as_deref())?;
        Ok(manifest)
    });
    match staged {
        Ok(manifest) => Outcome::ReadyToFetch { manifest },
        Err(e) => Outcome::Failed {
            why: format!("{e:#}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Name, WINGET};
    use crate::state::Ownership;

    #[test]
    fn a_version_change_needs_an_artifact() {
        for a in [
            Action::Install {
                backend: SCOOP.into(),
                name: Name::new("a"),
                version: "1".into(),
                arch: None,
            },
            Action::Upgrade {
                backend: SCOOP.into(),
                name: Name::new("a"),
                from: "1".into(),
                to: "2".into(),
                arch: None,
            },
            Action::Downgrade {
                backend: SCOOP.into(),
                name: Name::new("a"),
                from: "2".into(),
                to: "1".into(),
                arch: None,
            },
        ] {
            assert!(matches!(classify(&a), Intent::NeedsArtifact), "{a:?}");
        }
    }

    #[test]
    fn a_prune_needs_nothing_fetched() {
        assert!(matches!(
            classify(&Action::Prune {
                backend: SCOOP.into(),
                name: Name::new("a"),
                version: "1".into()
            }),
            Intent::NoArtifactNeeded
        ));
    }

    #[test]
    fn a_running_package_is_a_skip_but_an_unlocked_one_fails_the_run() {
        // The distinction that is easy to collapse: both are Action::Skip and
        // status prints both as `!`.
        assert!(matches!(
            classify(&Action::Skip {
                backend: SCOOP.into(),
                name: Name::new("a"),
                reason: SkipReason::Running
            }),
            Intent::Skip(_)
        ));
        assert!(matches!(
            classify(&Action::Skip {
                backend: SCOOP.into(),
                name: Name::new("a"),
                reason: SkipReason::NotLocked
            }),
            Intent::NotLocked
        ));
    }

    #[test]
    fn a_declared_winget_package_does_not_fail_a_scoop_run() {
        // Failing the run because Phase 4 has not happened would make apply
        // unusable for anyone whose pkg.toml has a [winget] section, and the
        // plan already prints a `!` line for it every single run.
        assert!(matches!(
            classify(&Action::Skip {
                backend: WINGET.into(),
                name: Name::new("Git.Git"),
                reason: SkipReason::BackendNotImplemented
            }),
            Intent::Skip(_)
        ));
    }

    #[test]
    fn reports_pass_through_without_affecting_the_verdict() {
        for a in [
            Action::Unmanaged {
                backend: SCOOP.into(),
                name: Name::new("a"),
                version: "1".into(),
            },
            Action::ArchDrift {
                backend: SCOOP.into(),
                name: Name::new("a"),
                have: "64bit".into(),
                want: "arm64".into(),
            },
        ] {
            assert!(matches!(classify(&a), Intent::Report), "{a:?}");
        }
    }

    #[test]
    fn a_preparation_with_a_failure_is_not_ok_and_one_without_is() {
        let ok = Preparation {
            prepared: vec![Prepared {
                action: Action::Install {
                    backend: SCOOP.into(),
                    name: Name::new("a"),
                    version: "1".into(),
                    arch: None,
                },
                outcome: Outcome::ReadyToFetch {
                    manifest: PathBuf::from("/stage/a/1/a.json"),
                },
            }],
        };
        assert!(ok.is_ok());

        let bad = Preparation {
            prepared: vec![Prepared {
                action: Action::Install {
                    backend: SCOOP.into(),
                    name: Name::new("a"),
                    version: "1".into(),
                    arch: None,
                },
                outcome: Outcome::Failed {
                    why: "hash mismatch".into(),
                },
            }],
        };
        assert!(!bad.is_ok());

        let unlocked = Preparation {
            prepared: vec![Prepared {
                action: Action::Skip {
                    backend: SCOOP.into(),
                    name: Name::new("a"),
                    reason: SkipReason::NotLocked,
                },
                outcome: Outcome::NotLocked,
            }],
        };
        assert!(!unlocked.is_ok(), "an unlocked package must fail the run");
    }

    fn owning(names: &[&str]) -> State {
        let mut s = State::default();
        for n in names {
            s.set(SCOOP, &Name::new(*n), Ownership::Installed);
        }
        s
    }

    #[test]
    fn an_empty_config_with_owned_packages_is_refused() {
        let err = mass_prune_guard(
            &crate::config::parse("").unwrap(),
            &owning(&["fzf", "bat", "ripgrep", "neovim", "kanata"]),
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains('5'), "the count is the whole point: {msg}");
        assert!(
            msg.contains("--allow-empty-config"),
            "say how to override: {msg}"
        );
    }

    #[test]
    fn an_empty_config_on_a_machine_that_owns_nothing_is_fine() {
        // A fresh machine. status should report everything as unmanaged and
        // apply should do nothing -- not error.
        mass_prune_guard(&crate::config::parse("").unwrap(), &State::default()).unwrap();
    }

    #[test]
    fn a_config_that_declares_anything_is_not_the_corruption_case() {
        mass_prune_guard(
            &crate::config::parse("[scoop]\npackages = [\"fzf\"]\n").unwrap(),
            &owning(&["fzf", "bat", "ripgrep"]),
        )
        .unwrap();
    }

    // -- default_staging_root ------------------------------------------

    #[test]
    fn the_staging_root_lives_beside_state_json() {
        assert_eq!(
            default_staging_root(),
            State::default_path().parent().unwrap().join("manifests")
        );
    }

    // -- prepare(): the assembly, not just classify() in isolation ------
    //
    // `classify` is where the judgement is and gets the dedicated tests
    // above. These exercise the wiring: that `prepare` actually calls
    // `classify`, dispatches on it, and turns a real `stage`/`download`
    // error into a per-package `Outcome::Failed` without stopping the walk.

    use crate::lock::Pin;

    fn git_output(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
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

    #[test]
    fn prepare_turns_a_missing_bucket_into_a_failed_outcome_without_stopping_the_run() {
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());

        let mut lock = Lock::default();
        lock.scoop.insert(
            Name::new("tool"),
            Pin::ScoopCommit {
                bucket: "extras".into(),
                // A real commit shape (40 hex), not a placeholder: since
                // ensure_commit_hash now runs before the bucket-exists check,
                // a short dummy like the old "abc123" would fail there first
                // and this test would stop proving what it is named for.
                commit: "a".repeat(40),
                version: "1.0.0".into(),
            },
        );

        // Install first, Prune second: proves one package's failure does not
        // stop the walk from reaching the next.
        let plan = Plan {
            actions: vec![
                Action::Install {
                    backend: SCOOP.into(),
                    name: Name::new("tool"),
                    version: "1.0.0".into(),
                    arch: None,
                },
                Action::Prune {
                    backend: SCOOP.into(),
                    name: Name::new("old"),
                    version: "0.1.0".into(),
                },
            ],
        };

        let prep = prepare(&plan, &lock, &scoop, stage_dir.path());
        assert_eq!(prep.prepared.len(), 2);
        assert_eq!(prep.failed_count(), 1);
        assert_eq!(prep.ready_count(), 1, "the prune must still go through");
        assert!(!prep.is_ok());

        let Outcome::Failed { why } = &prep.prepared[0].outcome else {
            panic!(
                "expected a Failed outcome, got {:?}",
                prep.prepared[0].outcome
            );
        };
        assert!(why.contains("extras"), "name the bucket: {why}");
        // The same tightening as the two integration tests in
        // tests/prepare.rs, for the same measured reason: with the
        // bucket-exists check in `stage()` deleted outright, `contains
        // ("extras")` alone stayed green here, because the next error down
        // (a commit-not-in-bucket message) also names the bucket -- while
        // telling the user their commit is broken when what they actually
        // need is `scoop bucket add`.
        assert!(
            why.contains("not present at"),
            "name why it failed, not just what: {why}"
        );
    }

    #[test]
    fn stage_and_fetch_is_total_even_if_ever_called_with_the_wrong_action_shape() {
        // prepare() only ever reaches stage_and_fetch via classify() returning
        // Intent::NeedsArtifact, which today means exactly Install/Upgrade/
        // Downgrade -- so this path is unreachable through prepare() itself.
        // Called directly, bypassing that guarantee, it must still fail
        // rather than panic: a `Preparation` must be total over whatever it
        // is given, not just over today's callers.
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let lock = Lock::default();
        let action = Action::Prune {
            backend: SCOOP.into(),
            name: Name::new("a"),
            version: "1".into(),
        };

        let outcome = stage_and_fetch(&action, &lock, &scoop, stage_dir.path());
        let Outcome::Failed { why } = outcome else {
            panic!("expected a Failed outcome, got {outcome:?}");
        };
        assert!(
            why.contains("needing an artifact"),
            "name the actual mismatch: {why}"
        );
    }

    #[test]
    fn prepare_refuses_to_stage_a_non_scoop_backend_defensively() {
        // The planner never actually produces this today -- winget always
        // comes through as Skip{BackendNotImplemented} -- but stage_and_fetch
        // must not guess if that ever changes without prepare() being
        // updated in lockstep.
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let lock = Lock::default();

        let plan = Plan {
            actions: vec![Action::Install {
                backend: WINGET.into(),
                name: Name::new("Git.Git"),
                version: "2.55.0".into(),
                arch: None,
            }],
        };

        let prep = prepare(&plan, &lock, &scoop, stage_dir.path());
        let Outcome::Failed { why } = &prep.prepared[0].outcome else {
            panic!(
                "expected a Failed outcome, got {:?}",
                prep.prepared[0].outcome
            );
        };
        assert!(why.contains("winget"), "name the backend: {why}");
        assert!(!prep.is_ok());
    }

    #[test]
    fn prepare_stages_for_real_but_reports_a_failure_when_there_is_no_scoop_to_download_with() {
        // stage() is fully testable against real git; download() needs a
        // real scoop binary this suite cannot assume exists. This test
        // proves prepare() calls both, in order: the manifest lands on disk
        // for real (stage succeeded) even though the overall outcome is
        // Failed, because there is no shims/scoop.cmd here to run.
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let bucket_dir = root.path().join("buckets").join("main");
        std::fs::create_dir_all(bucket_dir.join("bucket")).unwrap();
        git_output(&bucket_dir, &["init", "-q", "-b", "main"]);
        std::fs::write(
            bucket_dir.join("bucket").join("tool.json"),
            r#"{"version":"1.0.0","bin":"tool.exe"}"#,
        )
        .unwrap();
        git_output(&bucket_dir, &["add", "-A"]);
        git_output(
            &bucket_dir,
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
        let commit = git_output(&bucket_dir, &["rev-parse", "HEAD"])
            .trim()
            .to_string();

        let mut lock = Lock::default();
        lock.scoop.insert(
            Name::new("tool"),
            Pin::ScoopCommit {
                bucket: "main".into(),
                commit,
                version: "1.0.0".into(),
            },
        );
        let scoop = Scoop::new(root.path().to_path_buf());
        let plan = Plan {
            actions: vec![Action::Install {
                backend: SCOOP.into(),
                name: Name::new("tool"),
                version: "1.0.0".into(),
                arch: None,
            }],
        };

        let prep = prepare(&plan, &lock, &scoop, stage_dir.path());
        assert_eq!(prep.prepared.len(), 1);
        let Outcome::Failed { why } = &prep.prepared[0].outcome else {
            panic!(
                "expected download to fail with no scoop binary present, got {:?}",
                prep.prepared[0].outcome
            );
        };
        // Disambiguates which of the two steps failed: a stage() failure
        // would say "is not in bucket" or "not present at". This says
        // neither, because stage() already succeeded and wrote the file
        // asserted below.
        assert!(
            !why.contains("is not in bucket") && !why.contains("not present at"),
            "this must fail at download, not at stage: {why}"
        );

        let staged = stage_dir
            .path()
            .join("tool")
            .join("1.0.0")
            .join("tool.json");
        assert!(
            staged.exists(),
            "stage() must have run for real and written the manifest"
        );
        assert!(
            !root.path().join("apps").exists(),
            "nothing outside staging_root may be written -- no install-like tree may appear"
        );
    }

    #[test]
    fn prepare_treats_a_prune_as_ready_to_remove() {
        // Nothing to fetch for a removal -- 2b-2 does the uninstalling, and
        // the variant itself is what tells it so.
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let lock = Lock::default();
        let plan = Plan {
            actions: vec![Action::Prune {
                backend: SCOOP.into(),
                name: Name::new("aichat"),
                version: "0.30.0".into(),
            }],
        };

        let prep = prepare(&plan, &lock, &scoop, stage_dir.path());
        assert_eq!(prep.prepared[0].outcome, Outcome::ReadyToRemove);
        assert!(prep.is_ok());
    }

    #[test]
    fn prepare_passes_through_running_skips_and_not_locked_failures_untouched() {
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let lock = Lock::default();
        let plan = Plan {
            actions: vec![
                Action::Skip {
                    backend: SCOOP.into(),
                    name: Name::new("kanata"),
                    reason: SkipReason::Running,
                },
                Action::Skip {
                    backend: SCOOP.into(),
                    name: Name::new("zellij"),
                    reason: SkipReason::NotLocked,
                },
            ],
        };

        let prep = prepare(&plan, &lock, &scoop, stage_dir.path());
        assert_eq!(prep.skipped_count(), 1);
        assert_eq!(prep.not_locked_count(), 1);
        assert!(!prep.is_ok(), "a not-locked package must fail the run");
        assert_eq!(prep.prepared[1].outcome, Outcome::NotLocked);
        let Outcome::Skipped { why } = &prep.prepared[0].outcome else {
            panic!("expected Skipped, got {:?}", prep.prepared[0].outcome);
        };
        assert!(why.contains("running"), "got {why}");
    }

    #[test]
    fn the_lock_coherence_guard_refuses_every_shape_that_is_decidable_without_io() {
        use crate::lock::Pin;

        let bad_commit = {
            let mut l = Lock::default();
            l.scoop.insert(
                Name::new("tool"),
                Pin::ScoopCommit {
                    bucket: "main".into(),
                    commit: "main".into(),
                    version: "1.0.0".into(),
                },
            );
            l
        };
        let msg = format!("{:#}", lock_coherence_guard(&bad_commit).unwrap_err());
        assert!(msg.contains("tool") && msg.contains("main"), "{msg}");
        assert!(msg.contains("dotpkg update"), "say how to fix it: {msg}");
        assert!(
            msg.contains("hex"),
            "say what a commit must look like, not just that it's wrong: {msg}"
        );
        // Without this, "tool" and "main" are also satisfied by the
        // undeclared-bucket-style message this guard used to produce, so the
        // two asserts above would pass for the wrong reason.
        assert!(
            !msg.contains("is not in bucket"),
            "refused for its shape, not for being absent: {msg}"
        );

        let winget_pin_in_scoop_map = {
            let mut l = Lock::default();
            l.scoop.insert(
                Name::new("tool"),
                Pin::WingetVersion {
                    version: "1".into(),
                },
            );
            l
        };
        assert!(lock_coherence_guard(&winget_pin_in_scoop_map).is_err());

        let path_escaping_bucket = {
            let mut l = Lock::default();
            l.scoop.insert(
                Name::new("tool"),
                Pin::ScoopCommit {
                    bucket: "../evil".into(),
                    commit: "a".repeat(40),
                    version: "1.0.0".into(),
                },
            );
            l
        };
        assert!(lock_coherence_guard(&path_escaping_bucket).is_err());
    }

    #[test]
    fn a_coherent_lock_passes_the_guard() {
        // Positive control: without it, a guard that always errors passes the
        // test above.
        use crate::lock::Pin;
        let mut lock = Lock::default();
        lock.scoop.insert(
            Name::new("tool"),
            Pin::ScoopCommit {
                bucket: "main".into(),
                commit: "a".repeat(40),
                version: "1.0.0".into(),
            },
        );
        lock_coherence_guard(&lock).unwrap();
    }

    #[test]
    fn prepare_passes_reports_through_without_touching_the_verdict() {
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let lock = Lock::default();
        let plan = Plan {
            actions: vec![
                Action::Unmanaged {
                    backend: SCOOP.into(),
                    name: Name::new("antigravity"),
                    version: "2.0.6".into(),
                },
                Action::ArchDrift {
                    backend: SCOOP.into(),
                    name: Name::new("python"),
                    have: "64bit".into(),
                    want: "arm64".into(),
                },
            ],
        };

        let prep = prepare(&plan, &lock, &scoop, stage_dir.path());
        assert_eq!(prep.prepared.len(), 2);
        assert_eq!(prep.ready_count(), 0);
        assert_eq!(prep.failed_count(), 0);
        assert_eq!(prep.skipped_count(), 0);
        assert_eq!(prep.not_locked_count(), 0);
        assert!(prep.is_ok());
        for p in &prep.prepared {
            assert_eq!(p.outcome, Outcome::Report);
        }
    }
}
