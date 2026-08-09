use crate::backend::scoop::Scoop;
use crate::backend::Backend;
use crate::config::Config;
use crate::execute::Step;
use crate::lock::Lock;
use crate::lock::Pin;
use crate::model::{Name, SCOOP, WINGET};
use crate::plan::{Action, Plan, SkipReason};
// Only this file's own tests name `Divergence` directly (`classify` matches
// `SkipReason::ReportedOnly(divergence)` and calls `divergence.describe()`
// without needing the type in scope), so the import is `cfg(test)`-gated --
// otherwise it is unused in a normal build and the crate would no longer be
// warning-free.
#[cfg(test)]
use crate::plan::Divergence;
use crate::state::State;
use anyhow::Result;
use std::io::{BufRead, Write};
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
/// Checked per backend, independently: the old version returned from the
/// whole function on the first backend with any declared packages, so a
/// `pkg.toml` declaring one scoop package could drop its entire `[winget]`
/// section and prune every owned winget package with no guard at all. A
/// declared package in one backend must never vouch for an emptied section
/// of another.
pub fn mass_prune_guard(declared: &Config, state: &State) -> Result<()> {
    for (backend, declared_count) in [
        (SCOOP, declared.scoop.packages.len()),
        (WINGET, declared.winget.packages.len()),
    ] {
        if declared_count > 0 {
            continue;
        }
        let owned = state.owned_count(backend);
        anyhow::ensure!(
            owned == 0,
            "pkg.toml declares no {backend} packages but dotpkg owns {owned}. \
             Refusing to prune everything. If the file is right, pass --allow-empty-config."
        );
    }
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
/// declared buckets: that is a per-package concern, checked in
/// `stage_and_fetch` via a parsed `BucketDecl`'s case-folded `Name`, not a
/// whole-run one. A whole-run version of that check deadlocks -- drop a
/// package and its bucket line from `pkg.toml` together, and the stale lock
/// entry would fail every later `apply` before the plan is even built,
/// including the prune that would otherwise clear it.
pub fn lock_coherence_guard(lock: &Lock) -> Result<()> {
    for (name, pin) in &lock.scoop {
        entry_coherence(name, pin, crate::model::SCOOP)
            .map_err(|e| e.context("pkg.lock is not usable. Run `dotpkg update` to rewrite it."))?;
    }
    for (name, pin) in &lock.winget {
        entry_coherence(name, pin, crate::model::WINGET)
            .map_err(|e| e.context("pkg.lock is not usable. Run `dotpkg update` to rewrite it."))?;
    }
    Ok(())
}

/// One entry's share of the guard, with **no advice attached**.
///
/// Split out so that `update` -- which is itself the command "Run `dotpkg
/// update`" points at -- can name the blocking entries without repeating that
/// advice back at the user who is already running it. The rules live here
/// once; the two callers differ only in what they say afterwards.
///
/// One arm per `Pin` shape, not one function per backend, per the brief this
/// task followed -- but `in_map` (`SCOOP` or `WINGET`, the map this entry was
/// actually read from) is still a parameter, not dropped: `Task 7` changed
/// `Resolution::Resolved` to carry a `Pin` so a winget resolution carrying a
/// commit is a compile error, but nothing at the type level stops the
/// REVERSE mistake -- a `Pin::WingetVersion` landing in `lock.scoop` because
/// `src/update.rs`'s `fold_backend` (Task 15's own routing) has a bug.
/// Before this task, `entry_coherence` caught exactly that by bailing on any
/// non-`ScoopCommit` pin unconditionally; matching on `pin`'s shape alone
/// (dropping `in_map`) would silently accept a `Pin::WingetVersion` wherever
/// it happens to sit, including the map it must never reach. `in_map` is
/// what lets each arm refuse a pin that is coherent in itself but sitting in
/// the wrong map, without a second function and without duplicating that
/// check at each of this function's two callers.
///
/// The winget arm is Task 15's: a non-empty version, and
/// `ensure_plain_component` over both the version and the id -- **the id
/// spelled as winget spells it** (`name.to_string()`, the display form, not
/// `name.key()`). `ensure_plain_component`'s own rules (empty, `.`, `..`,
/// absolute, a leading `-`, a path separator) do not actually depend on
/// case, so this choice changes no verdict today -- it is made anyway
/// because the display spelling is what a winget lock entry's key genuinely
/// is (`update`/`adopt` write the canonical, cased id `winget show` echoed
/// back, never the folded form), and checking the string that is actually
/// stored is the correct thing to validate even where it happens not to
/// matter yet. Scoop's own arm keeps checking the folded `key()`, unchanged,
/// because scoop opens a directory on a case-folding filesystem.
fn entry_coherence(name: &Name, pin: &Pin, in_map: &'static str) -> Result<()> {
    match pin {
        Pin::ScoopCommit {
            bucket,
            commit,
            version,
        } => {
            anyhow::ensure!(
                in_map == crate::model::SCOOP,
                "pkg.lock [{in_map}.{name}] holds a scoop pin"
            );
            crate::backend::scoop::ensure_plain_component(name, "pkg.lock", "bucket", bucket)?;
            crate::backend::scoop::ensure_plain_component(name, "pkg.lock", "version", version)?;
            crate::backend::scoop::ensure_plain_component(
                name,
                "pkg.lock",
                "package name",
                name.key(),
            )?;
            crate::backend::scoop::ensure_commit_hash(name, commit)?;
        }
        Pin::WingetVersion { version } => {
            anyhow::ensure!(
                in_map == crate::model::WINGET,
                "pkg.lock [{in_map}.{name}] holds a winget pin"
            );
            crate::backend::scoop::ensure_plain_component(name, "pkg.lock", "version", version)?;
            let id = name.to_string();
            crate::backend::scoop::ensure_plain_component(name, "pkg.lock", "package id", &id)?;
        }
    }
    Ok(())
}

/// Every entry `lock_coherence_guard` would reject, with its reason.
///
/// The guard stops at the first, which is right for a guard: its product is
/// "refuse or don't". `update`'s failure message has a different product --
/// the user needs to know which entries to repair, and stopping at the first
/// turns one repair into N runs. Both maps, for the same reason
/// `lock_coherence_guard` above checks both: a winget entry has never been
/// coherence-checked before Task 15, and `update`'s repair message must be
/// able to name one.
pub fn incoherent_entries(lock: &Lock) -> Vec<(Name, String)> {
    lock.scoop
        .iter()
        .map(|(name, pin)| (name, pin, crate::model::SCOOP))
        .chain(
            lock.winget
                .iter()
                .map(|(name, pin)| (name, pin, crate::model::WINGET)),
        )
        .filter_map(|(name, pin, in_map)| {
            entry_coherence(name, pin, in_map)
                .err()
                .map(|e| (name.clone(), format!("{e:#}")))
        })
        .collect()
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
        Action::Skip { reason, .. } => match reason {
            SkipReason::NotLocked => Intent::NotLocked,
            SkipReason::Running => Intent::Skip("running -- stop it first".to_string()),
            // Unlike the old `BackendNotImplemented` arm this replaced, this
            // reads no `backend` field itself -- `Divergence::describe()`
            // already names "winget" in its own text, since every
            // `Capability::ReportsOnly` backend today is winget. That is a
            // known simplification (ledgered, not fixed here): if a second
            // `ReportsOnly` backend ever existed, `describe()` would need to
            // stop hardcoding the name and start reading it from somewhere.
            SkipReason::ReportedOnly(divergence) => Intent::Skip(divergence.describe()),
            // Same shape as Running: dotpkg cannot act on a state it could not
            // establish, and installing over it is exactly the mistake this
            // variant exists to prevent. The user fixes the read (usually by
            // rerunning without the elevated/restricted context) and reruns.
            SkipReason::Opaque => Intent::Skip(
                "installed, but its state could not be read -- see the warnings above".to_string(),
            ),
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
/// **What `is_ok() == false` means for `main`, as shipped in Phase 2b-2:** by
/// default, refuse the whole run and perform *none* of its actions -- not the
/// ready ones either. That is `main.rs`'s `!preparation.is_ok() && !keep_going`
/// gate: it prints how many packages could not be prepared, exits 2, and
/// `execute` is never called -- pinned by
/// `a_preparation_that_could_not_be_completed_refuses_before_execute_ever_runs`
/// in `tests/cli.rs`, which also proves it via the absence of the `scoop.cmd`
/// reachability sentinel that a real mutation attempt would leave behind.
///
/// `--keep-going` narrows that in exactly one direction: install and replace
/// whatever IS ready, and report the rest, instead of refusing outright.
/// Removals are never part of that narrowing. `gate_removals` holds every
/// `Step::Remove` back whenever `is_ok()` is false -- `--keep-going` included,
/// and no other flag opens that gate either -- because every newly typed
/// package name is `NotLocked` until `update` exists, which makes "installs
/// nothing, deletes something" the one shape a not-ok preparation can produce
/// today.
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

    /// Every skip that is a fact about *this run* and could be different on
    /// the next one, by name and reason -- not because `is_ok()` needs
    /// widening (it does not: none of these gate removals or refuse the
    /// run), but because this is still outstanding work the user asked for
    /// and did not get, and `main.rs` needs a way to say so both in the
    /// closing table and in the exit code.
    ///
    /// Still filtered through `is_outstanding` rather than counting every
    /// `Outcome::Skipped` outright, even though today the two queries agree:
    /// `NotLocked` -- the one `SkipReason` `is_outstanding` returns `false`
    /// for -- never reaches `Outcome::Skipped` at all (`classify` sends it to
    /// `Intent::NotLocked`, which becomes `Outcome::NotLocked`, and that
    /// already fails `is_ok()` on its own). `SkipReason::BackendNotImplemented`
    /// used to be the reason these two counts differed -- permanent and
    /// structural, reported identically on every run, and the one variant
    /// that was `Skipped` yet not outstanding -- and it was deleted along
    /// with the stub loop it existed for once Task 14 gave winget a real
    /// `Capability::ReportsOnly` view. Nothing has taken its place. The match
    /// in `is_outstanding` stays exhaustive with no wildcard for exactly this
    /// reason: the day a `Skipped`-but-not-outstanding `SkipReason` exists
    /// again, this narrows correctly without anyone having to remember to
    /// widen it back.
    ///
    /// Named for what the query answers, not for the first variant it
    /// recognised (`running_skips`, before review caught that `Opaque` had
    /// silently been left out of it): a skip's *kind* decides whether it
    /// belongs here, and that decision lives in `is_outstanding`, not in
    /// this method's name.
    pub fn outstanding_skips(&self) -> Vec<(Name, String)> {
        self.prepared
            .iter()
            .filter_map(|p| match (&p.action, &p.outcome) {
                (Action::Skip { reason, .. }, Outcome::Skipped { why })
                    if is_outstanding(reason) =>
                {
                    Some((action_name(&p.action), why.clone()))
                }
                _ => None,
            })
            .collect()
    }
}

/// Whether a skip is a fact about *this run* that could differ on the next
/// one (floors the exit code, appears in the closing table), as opposed to a
/// skip that is permanent and structural, reported identically on every run
/// forever (neither).
///
/// Exhaustive on purpose, with no wildcard arm: a `SkipReason` this match
/// does not name is a compile error, not a silent "does not float" default.
/// Task 14 adds `SkipReason::ReportedOnly` (a winget package that differs
/// from the lock) to this enum, and its own floor-or-not answer must be a
/// decision made here, not an oversight this match's shape would let slip
/// through.
fn is_outstanding(reason: &SkipReason) -> bool {
    match reason {
        // A winget package that differs from the lock is outstanding work
        // the user asked for and did not get, exactly like a running process
        // or an unreadable state: dotpkg reports it and, next run, might not
        // have to -- the package could converge, or the lock could change.
        // Unlike its predecessor `BackendNotImplemented`, this is not
        // permanent: it depends on the machine, not on what phase dotpkg is.
        SkipReason::Running | SkipReason::Opaque | SkipReason::ReportedOnly(_) => true,
        SkipReason::NotLocked => false,
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
///
/// `declared` is `pkg.toml`, parsed: `stage_and_fetch` uses it to check a
/// pin's bucket against `[scoop] buckets` before ever touching the disk.
pub fn prepare(
    plan: &Plan,
    lock: &Lock,
    scoop: &Scoop,
    mutator: &dyn crate::execute::Mutator,
    staging_root: &Path,
    declared: &Config,
) -> Preparation {
    let prepared = plan
        .actions
        .iter()
        .map(|action| Prepared {
            action: action.clone(),
            outcome: match classify(action) {
                Intent::NeedsArtifact => {
                    stage_and_fetch(action, lock, scoop, mutator, staging_root, declared)
                }
                Intent::NoArtifactNeeded => Outcome::ReadyToRemove,
                Intent::Skip(why) => Outcome::Skipped { why },
                Intent::NotLocked => Outcome::NotLocked,
                Intent::Report => Outcome::Report,
            },
        })
        .collect();
    Preparation { prepared }
}

/// Whether `bucket`, spelled as a lock's pin has it, names a bucket
/// `pkg.toml` declares. Compared via `Name`, which folds case:
/// `$SCOOP/buckets/Main` and `main` are the same directory on Windows, and a
/// byte-exact comparison would fail a lock that stages perfectly well.
fn bucket_is_declared(declared: &Config, bucket: &str) -> bool {
    let name = Name::new(bucket);
    declared.scoop.buckets.iter().any(|b| b.name == name)
}

/// The `NeedsArtifact` half of `prepare`, kept separate so the walk above
/// reads as a plain classify-and-dispatch.
///
/// Every error path returns `Outcome::Failed` rather than propagating a
/// `Result` or panicking: a `Preparation` must always be total over the
/// plan it was given, because the whole point of the phase is that one
/// package's problem is reported, not fatal to the run.
fn stage_and_fetch(
    action: &Action,
    lock: &Lock,
    scoop: &Scoop,
    mutator: &dyn crate::execute::Mutator,
    staging_root: &Path,
    declared: &Config,
) -> Outcome {
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
    // A per-package failure, not a whole-run abort: `lock_coherence_guard`
    // deliberately leaves this out (see its doc comment) because a whole-run
    // version deadlocks when a package and its bucket line are dropped from
    // `pkg.toml` together.
    if let Pin::ScoopCommit { bucket, .. } = pin {
        if !bucket_is_declared(declared, bucket) {
            return Outcome::Failed {
                why: format!(
                    "{name}: bucket {bucket:?} is not declared in pkg.toml -- \
                     add it to [scoop] buckets"
                ),
            };
        }
    }
    let staged = scoop.stage(staging_root, name, pin).and_then(|manifest| {
        // Behind `Mutator` since Phase 3, which is what finally lets a test on
        // any OS see both that this produces a real `Outcome::ReadyToFetch`
        // and that `arch` -- not `None` -- reaches the argv.
        let report = mutator.download(&manifest, arch.as_deref())?;
        crate::backend::scoop::download_outcome(&report.stdout, &manifest)?;
        Ok(manifest)
    });
    match staged {
        Ok(manifest) => Outcome::ReadyToFetch { manifest },
        Err(e) => Outcome::Failed {
            why: format!("{e:#}"),
        },
    }
}

/// Turn a finished `Preparation` into the steps the executor will run, plus
/// the packages that could not become steps and why.
///
/// `Outcome::Skipped` (today, only ever a running process at prepare time, or
/// a backend not yet implemented) is routed into the same `unusable` list as
/// `Failed` and `NotLocked`, rather than a third return: all three are the
/// same shape from a caller's point of view -- "no step, and a reason the
/// user must be shown" -- and `Preparation::is_ok` already keeps its own,
/// separate count of which of them refuse the whole run, so `unusable` does
/// not need to repeat that distinction to be useful to a caller that just
/// wants to report what didn't become a step and why.
pub fn plan_to_steps(prep: &Preparation) -> (Vec<Step>, Vec<(Name, String)>) {
    let mut steps = Vec::new();
    let mut unusable = Vec::new();
    for p in &prep.prepared {
        // Branch on the ACTION, never on the outcome: `Outcome::ReadyToRemove`
        // is still attachable to an `Install`, and nothing in the type system
        // binds the two.
        match (&p.action, &p.outcome) {
            (Action::Install { name, arch, .. }, Outcome::ReadyToFetch { manifest }) => {
                steps.push(Step::Install {
                    app: name.clone(),
                    staged: manifest.clone(),
                    arch: arch.clone(),
                })
            }
            (
                Action::Upgrade { name, arch, .. } | Action::Downgrade { name, arch, .. },
                Outcome::ReadyToFetch { manifest },
            ) => steps.push(Step::Replace {
                app: name.clone(),
                staged: manifest.clone(),
                arch: arch.clone(),
            }),
            (Action::Prune { name, .. }, Outcome::ReadyToRemove) => {
                steps.push(Step::Remove { app: name.clone() })
            }
            (a, Outcome::Failed { why }) => unusable.push((action_name(a), why.clone())),
            (a, Outcome::NotLocked) => unusable.push((
                action_name(a),
                "no lock entry -- run `dotpkg update`".to_string(),
            )),
            (a, Outcome::Skipped { why }) => unusable.push((action_name(a), why.clone())),
            _ => {}
        }
    }
    (steps, unusable)
}

/// Every `Action` variant names a backend and a package; mirrors
/// `render::action_backend_name`, but returns an owned `Name` since
/// `plan_to_steps` needs one it can put in an owned `Vec`.
fn action_name(action: &Action) -> Name {
    match action {
        Action::Install { name, .. }
        | Action::Upgrade { name, .. }
        | Action::Downgrade { name, .. }
        | Action::Prune { name, .. }
        | Action::Skip { name, .. }
        | Action::Unmanaged { name, .. }
        | Action::ArchDrift { name, .. } => name.clone(),
    }
}

/// Ask, and treat anything that is not an explicit yes as a no.
///
/// The question goes to `err` so that `apply | tee` still shows it while the
/// plan and preparation tables stay on stdout.
///
/// `Ok(0)` from `read_line` is what a child process with no console gets — the
/// medium-integrity scheduled task the dogfood runs under — and it means
/// **no**, loudly, naming `--yes`. `is_terminal()` is deliberately not
/// consulted: whether a terminal is attached is not the same question as
/// whether the user agreed.
pub fn confirm(question: &str, input: &mut dyn BufRead, err: &mut dyn Write) -> Result<bool> {
    write!(err, "{question}")?;
    err.flush()?;
    let mut line = String::new();
    match input.read_line(&mut line) {
        Ok(0) => {
            writeln!(
                err,
                "\napply needs an answer and stdin is not readable. Pass --yes if you \
                 have read the plan above."
            )?;
            Ok(false)
        }
        Ok(_) => {
            let a = line.trim().to_ascii_lowercase();
            Ok(a == "y" || a == "yes")
        }
        Err(e) => {
            writeln!(
                err,
                "\ncannot read an answer ({e}); treating that as no. Pass --yes."
            )?;
            Ok(false)
        }
    }
}

/// Everything a command needs, loaded once.
///
/// `main.rs` used to carry two inline copies of this sequence and `tests/`
/// reached neither.
pub struct Driver {
    pub declared: Config,
    pub locked: Lock,
    pub state: State,
    pub scoop: Scoop,
    pub scan: crate::backend::Scan,
    /// Winget's half of the same fact `scan` holds for scoop. Kept as its
    /// own field rather than merged into `scan`: `main.rs` prints each
    /// scan's `warnings` attributed to its own backend ("warning: scoop: …"
    /// / "warning: winget: …"), and a plain `Vec<String>` cannot be told
    /// apart by backend once merged -- merging first would silently mislabel
    /// every winget warning as scoop's. `main.rs` concatenates `installed`
    /// and `opaque` from both fields itself, right before calling `plan()`,
    /// which is backend-agnostic for exactly those two (see `Scan`'s own doc
    /// comment).
    pub winget_scan: crate::backend::Scan,
    pub running: crate::model::Running,
}

pub fn load_everything(config: &Path, lock: &Path, state_path: &Path) -> Result<Driver> {
    let declared = crate::config::load(config)?;
    let locked = crate::lock::load_or_empty(lock)?;
    let state = State::load_or_empty(state_path)?;
    let scoop = Scoop::discover();
    let scan = scoop.scan()?;
    let winget = crate::backend::winget::Winget::new(crate::backend::winget::RealWinget);
    // Not `?`: a winget hiccup must not abort scoop's entirely unrelated
    // half of this run. See `crate::backend::scan_or_warn`'s own doc comment.
    let winget_scan = crate::backend::scan_or_warn(&winget);
    let procs = crate::sys::running_processes();
    let running = scoop.running_set(&procs);
    Ok(Driver {
        declared,
        locked,
        state,
        scoop,
        scan,
        winget_scan,
        running,
    })
}

/// Split prepared steps into what may run and which removals are held back.
///
/// Removals are gated on the WHOLE preparation being ok, and no flag opens
/// this gate by itself -- not `--yes`, not `--keep-going`. `--keep-going`
/// only lets a run continue past packages that could not be prepared; it must
/// never also let a prune through on the strength of that same flag, because
/// every newly typed package name is `NotLocked` until `update` exists, which
/// makes "installs nothing, deletes something" the one shape reachable today
/// with a not-ok preparation.
///
/// Kept as its own function, rather than inline in `main.rs`, because this is
/// the one decision in the whole driver that a deleted line would make
/// silently permissive -- and a silently permissive prune is exactly the
/// mistake this project's tests exist to catch before a machine does.
pub fn gate_removals(steps: Vec<Step>, preparation_ok: bool) -> (Vec<Step>, Vec<Name>) {
    if preparation_ok {
        return (steps, Vec::new());
    }
    let mut kept = Vec::with_capacity(steps.len());
    let mut held = Vec::new();
    for step in steps {
        if matches!(step, Step::Remove { .. }) {
            held.push(step.app().clone());
        } else {
            kept.push(step);
        }
    }
    (kept, held)
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
    fn a_reported_only_winget_package_does_not_fail_a_scoop_run() {
        // Failing the run because dotpkg cannot act on winget yet would make
        // apply unusable for anyone whose pkg.toml has a [winget] section,
        // and the plan already prints a `!` line for it every single run.
        assert!(matches!(
            classify(&Action::Skip {
                backend: WINGET.into(),
                name: Name::new("Brave.Brave"),
                reason: SkipReason::ReportedOnly(Divergence::Change {
                    from: "151.1.93.132".into(),
                    to: "151.1.93.134".into(),
                }),
            }),
            Intent::Skip(_)
        ));
    }

    #[test]
    fn a_declared_unlocked_winget_package_is_skip_not_notlocked_so_it_cannot_fail_the_run() {
        // The regression this task's own review caught: `SkipReason::
        // ReportedOnly(Divergence::NotLocked)` must classify to
        // `Intent::Skip`, the same as every other ReportedOnly shape --
        // NOT to `Intent::NotLocked`, which is what `SkipReason::NotLocked`
        // itself gets a few lines above in this same match, and which fails
        // the whole run. Getting this one wrong is exactly how a declared,
        // unlocked winget package made `apply` refuse the entire plan --
        // scoop actions included -- with "N package(s) could not be
        // prepared, so nothing has been changed."
        assert!(matches!(
            classify(&Action::Skip {
                backend: WINGET.into(),
                name: Name::new("Git.Git"),
                reason: SkipReason::ReportedOnly(Divergence::NotLocked),
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

    #[test]
    fn a_config_that_declares_no_winget_packages_while_dotpkg_owns_some_is_refused() {
        let mut state = State::default();
        state.set(WINGET, &Name::new("Git.Git"), Ownership::Adopted);
        let cfg = crate::config::parse("[winget]\npackages = []\n").unwrap();
        let r = mass_prune_guard(&cfg, &state);
        assert!(
            r.is_err(),
            "an emptied [winget] section must not prune silently"
        );
        let msg = format!("{:#}", r.unwrap_err());
        assert!(
            msg.contains("winget"),
            "the message must name the backend: {msg}"
        );
        assert!(msg.contains('1'), "and how many are owned: {msg}");
    }

    #[test]
    fn a_declared_scoop_package_does_not_disable_the_winget_half_of_the_guard() {
        // THE bug. The old short-circuit returned from the whole function on the
        // first non-empty backend, so a pkg.toml with any scoop package at all
        // could drop its entire [winget] section and prune every owned winget
        // package with no guard.
        let mut state = State::default();
        state.set(WINGET, &Name::new("Git.Git"), Ownership::Adopted);
        let cfg = crate::config::parse("[scoop]\npackages = [\"fzf\"]\n").unwrap();
        let r = mass_prune_guard(&cfg, &state);
        assert!(
            r.is_err(),
            "a non-empty [scoop] must not vouch for an empty [winget]"
        );
    }

    #[test]
    fn a_config_that_declares_packages_for_every_owned_backend_is_allowed() {
        // The positive sibling: without it, a guard that always refused would
        // satisfy both assertions above.
        let mut state = State::default();
        state.set(SCOOP, &Name::new("fzf"), Ownership::Installed);
        state.set(WINGET, &Name::new("Git.Git"), Ownership::Adopted);
        let cfg = crate::config::parse(
            "[scoop]\npackages = [\"fzf\"]\n[winget]\npackages = [\"Git.Git\"]\n",
        )
        .unwrap();
        assert!(mass_prune_guard(&cfg, &state).is_ok());
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
        // Declared, so this test still proves stage()'s on-disk check --
        // the undeclared-in-pkg.toml check below has its own tests.
        let declared = crate::config::parse("[scoop]\nbuckets = [\"extras\"]\n").unwrap();

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

        let prep = prepare(&plan, &lock, &scoop, &scoop, stage_dir.path(), &declared);
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

    // -- the undeclared-bucket check --------------------------------------
    //
    // Task 4 deliberately left this out of lock_coherence_guard (see its doc
    // comment): a whole-run version deadlocks if a package and its bucket
    // line are dropped from pkg.toml together. It belongs here instead, as a
    // per-package failure.

    #[test]
    fn bucket_declared_check_folds_case() {
        // $SCOOP/buckets/Main and main are the same directory on Windows, so
        // a byte-exact comparison would refuse a lock that stages perfectly
        // well.
        let declared = crate::config::parse("[scoop]\nbuckets = [\"main\"]\n").unwrap();
        assert!(bucket_is_declared(&declared, "main"));
        assert!(bucket_is_declared(&declared, "Main"));
        assert!(bucket_is_declared(&declared, "MAIN"));
        assert!(!bucket_is_declared(&declared, "extras"));
    }

    #[test]
    fn a_lock_naming_a_bucket_pkg_toml_does_not_declare_fails_that_package_only() {
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        // "main" is declared; the lock below pins a different bucket.
        let declared = crate::config::parse("[scoop]\nbuckets = [\"main\"]\n").unwrap();

        let mut lock = Lock::default();
        lock.scoop.insert(
            Name::new("tool"),
            Pin::ScoopCommit {
                bucket: "extras".into(),
                commit: "a".repeat(40),
                version: "1.0.0".into(),
            },
        );
        // Install first, Prune second: proves this failure does not stop the
        // walk from reaching the next package either.
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

        let prep = prepare(&plan, &lock, &scoop, &scoop, stage_dir.path(), &declared);
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
        assert!(why.contains("[scoop] buckets"), "say how to fix it: {why}");
        // Disambiguates this from stage()'s own "bucket not present on disk"
        // failure, which also names the bucket but for a different reason
        // and would send the user to `scoop bucket add` instead of pkg.toml.
        assert!(
            !why.contains("not present at"),
            "this is the pkg.toml-declared check, not stage()'s on-disk check: {why}"
        );
    }

    #[test]
    fn the_undeclared_bucket_check_folds_case_like_windows_directories_do() {
        // A lock naming "Main" against a pkg.toml declaring "main" must NOT
        // trip the undeclared-bucket check -- that would refuse a lock that
        // stages perfectly well, over nothing but a case difference that does
        // not exist on the filesystem this is bound for.
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let declared = crate::config::parse("[scoop]\nbuckets = [\"main\"]\n").unwrap();

        let mut lock = Lock::default();
        lock.scoop.insert(
            Name::new("tool"),
            Pin::ScoopCommit {
                bucket: "Main".into(),
                commit: "a".repeat(40),
                version: "1.0.0".into(),
            },
        );
        let plan = Plan {
            actions: vec![Action::Install {
                backend: SCOOP.into(),
                name: Name::new("tool"),
                version: "1.0.0".into(),
                arch: None,
            }],
        };

        let prep = prepare(&plan, &lock, &scoop, &scoop, stage_dir.path(), &declared);
        let Outcome::Failed { why } = &prep.prepared[0].outcome else {
            panic!(
                "expected Failed -- there is no real bucket on disk -- got {:?}",
                prep.prepared[0].outcome
            );
        };
        // It still fails, because no real bucket exists on disk in this
        // test -- but for stage()'s reason, not because the case fold was
        // skipped.
        assert!(
            !why.contains("is not declared"),
            "the case fold must accept Main against declared main: {why}"
        );
        assert!(why.contains("not present at"), "got {why}");
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
        let declared = Config::default();
        let action = Action::Prune {
            backend: SCOOP.into(),
            name: Name::new("a"),
            version: "1".into(),
        };

        let outcome = stage_and_fetch(&action, &lock, &scoop, &scoop, stage_dir.path(), &declared);
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
        // The planner never actually produces this today -- winget's view has
        // `Capability::ReportsOnly`, so a version difference always comes
        // through as Skip{ReportedOnly(..)}, never an Install -- but
        // stage_and_fetch must not guess if that ever changes without
        // prepare() being updated in lockstep.
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let lock = Lock::default();
        let declared = Config::default();

        let plan = Plan {
            actions: vec![Action::Install {
                backend: WINGET.into(),
                name: Name::new("Git.Git"),
                version: "2.55.0".into(),
                arch: None,
            }],
        };

        let prep = prepare(&plan, &lock, &scoop, &scoop, stage_dir.path(), &declared);
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
                commit: commit.clone(),
                version: "1.0.0".into(),
            },
        );
        let scoop = Scoop::new(root.path().to_path_buf());
        let declared = crate::config::parse("[scoop]\nbuckets = [\"main\"]\n").unwrap();
        let plan = Plan {
            actions: vec![Action::Install {
                backend: SCOOP.into(),
                name: Name::new("tool"),
                version: "1.0.0".into(),
                arch: None,
            }],
        };

        let prep = prepare(&plan, &lock, &scoop, &scoop, stage_dir.path(), &declared);
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
            .join(&commit)
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
        let declared = Config::default();
        let plan = Plan {
            actions: vec![Action::Prune {
                backend: SCOOP.into(),
                name: Name::new("aichat"),
                version: "0.30.0".into(),
            }],
        };

        let prep = prepare(&plan, &lock, &scoop, &scoop, stage_dir.path(), &declared);
        assert_eq!(prep.prepared[0].outcome, Outcome::ReadyToRemove);
        assert!(prep.is_ok());
    }

    #[test]
    fn prepare_passes_through_running_skips_and_not_locked_failures_untouched() {
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let lock = Lock::default();
        let declared = Config::default();
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

        let prep = prepare(&plan, &lock, &scoop, &scoop, stage_dir.path(), &declared);
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
    fn a_declared_unlocked_winget_package_does_not_fail_the_whole_preparation() {
        // The end-to-end proof of the Critical fix, one level below the full
        // `apply` CLI: before it, this exact plan -- a declared winget
        // package with no lock entry, nothing else -- produced `Outcome::
        // NotLocked`, `not_locked_count() == 1`, and `is_ok() == false`,
        // which is what made `main.rs` print "N package(s) could not be
        // prepared, so nothing has been changed" and exit 2 for a plan that
        // has no scoop action in it at all, let alone a failing one.
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let lock = Lock::default();
        let declared = Config::default();
        let plan = Plan {
            actions: vec![Action::Skip {
                backend: WINGET.into(),
                name: Name::new("Git.Git"),
                reason: SkipReason::ReportedOnly(Divergence::NotLocked),
            }],
        };

        let prep = prepare(&plan, &lock, &scoop, &scoop, stage_dir.path(), &declared);
        assert_eq!(
            prep.not_locked_count(),
            0,
            "this must NOT be counted as a not-locked failure"
        );
        assert!(
            prep.is_ok(),
            "a backend that cannot act does not fail the run just because \
             it also has no lock entry"
        );
        assert_eq!(prep.skipped_count(), 1);
        let Outcome::Skipped { why } = &prep.prepared[0].outcome else {
            panic!(
                "expected Outcome::Skipped, got {:?} -- Outcome::NotLocked here is the \
                 regression",
                prep.prepared[0].outcome
            );
        };
        assert!(why.contains("not in pkg.lock"), "got {why}");
        assert!(
            !why.contains("dotpkg update"),
            "dotpkg update does not create a winget lock entry today, so this \
             advice would be false: {why}"
        );

        // The positive control: without it, a `prepare()` that always
        // reported `is_ok()` regardless of input would pass the assertion
        // above too.
        let unlocked_scoop_plan = Plan {
            actions: vec![Action::Skip {
                backend: SCOOP.into(),
                name: Name::new("zellij"),
                reason: SkipReason::NotLocked,
            }],
        };
        let unlocked_scoop_prep = prepare(
            &unlocked_scoop_plan,
            &lock,
            &scoop,
            &scoop,
            stage_dir.path(),
            &declared,
        );
        assert!(
            !unlocked_scoop_prep.is_ok(),
            "a scoop package really is not-locked-and-fatal in the same shape -- \
             the fix must not have widened NotLocked's meaning generally, only \
             narrowed it for a ReportsOnly backend"
        );
    }

    #[test]
    fn is_outstanding_floors_running_opaque_and_reported_only_but_not_not_locked() {
        // `is_outstanding` is private, so this calls it directly rather than
        // routing through `prepare()` -- deliberately, for `NotLocked`: that
        // is the one `SkipReason` left in the `false` arm, and it can no
        // longer be exercised through `outstanding_skips()` at all, because
        // `classify` never turns it into `Outcome::Skipped` in the first
        // place (it becomes `Intent::NotLocked` -> `Outcome::NotLocked`,
        // which fails `is_ok()` on its own). Until Task 14 deleted
        // `BackendNotImplemented`, that variant played this same "Skipped but
        // not outstanding" role and could be shown failing to float through
        // `prepare()`'s real pipeline; nothing has taken its place, so this
        // is now the only way to pin the `false` arm at all. The `true` arm
        // is still also proven end-to-end, below, by
        // `outstanding_skips_finds_running_opaque_and_reported_only_skips_together`.
        assert!(is_outstanding(&SkipReason::Running));
        assert!(is_outstanding(&SkipReason::Opaque));
        assert!(is_outstanding(&SkipReason::ReportedOnly(
            Divergence::Change {
                from: "1".into(),
                to: "2".into(),
            }
        )));
        assert!(
            !is_outstanding(&SkipReason::NotLocked),
            "permanent and structural for THIS run: apply cannot resolve a \
             version itself, and the run already fails on it via NotLocked, \
             not via a float"
        );
    }

    #[test]
    fn outstanding_skips_finds_running_opaque_and_reported_only_skips_together() {
        // The end-to-end proof that all three outstanding `SkipReason`s
        // really do carry through the whole pipeline -- `prepare()` ->
        // `classify()` -> `Outcome::Skipped` -> `outstanding_skips()` -- not
        // just that `is_outstanding` says so in isolation (see the direct
        // unit test above).
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let lock = Lock::default();
        let declared = Config::default();
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
                    reason: SkipReason::Opaque,
                },
                Action::Skip {
                    backend: WINGET.into(),
                    name: Name::new("Brave.Brave"),
                    reason: SkipReason::ReportedOnly(Divergence::Change {
                        from: "151.1.93.132".into(),
                        to: "151.1.93.134".into(),
                    }),
                },
            ],
        };

        let prep = prepare(&plan, &lock, &scoop, &scoop, stage_dir.path(), &declared);
        assert_eq!(
            prep.skipped_count(),
            3,
            "the positive control: all three really are Skipped outcomes"
        );

        let outstanding = prep.outstanding_skips();
        assert_eq!(
            outstanding,
            vec![
                (Name::new("kanata"), "running -- stop it first".to_string()),
                (
                    Name::new("zellij"),
                    "installed, but its state could not be read -- see the warnings above"
                        .to_string()
                ),
                (
                    Name::new("Brave.Brave"),
                    "151.1.93.132 -> 151.1.93.134 -- reported only, dotpkg cannot install \
                     or remove winget packages yet"
                        .to_string()
                ),
            ],
            "all three must float -- none of them is permanent and structural"
        );
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

    // -- entry_coherence's winget arm (Task 15) --------------------------
    //
    // Before this task `apply::incoherent_entries` iterated `lock.scoop`
    // only, so a winget pin had never been coherence-checked at all --
    // `lock::parse` accepts `[winget."Git.Git"] version = ""` just fine (see
    // `lock.rs`'s own `parse_accepts_a_commit_the_guards_reject_and_that_
    // split_is_deliberate` for why a too-broken-to-run lock must still be
    // READABLE), and only this guard is what stands between that and
    // `dotpkg update` writing it back out unrepaired.

    #[test]
    fn an_incoherent_winget_entry_is_named_by_the_same_guard_as_a_scoop_one() {
        let lock =
            crate::lock::parse("[winget.\"Git.Git\"]\nversion = \"\"\npin = \"version-only\"\n")
                .unwrap();
        let bad = incoherent_entries(&lock);
        assert_eq!(bad.len(), 1, "an empty version must be refused: {bad:?}");
        assert_eq!(bad[0].0.to_string(), "Git.Git");
        assert!(
            bad[0].1.contains("version"),
            "name what about it is wrong, not just that it is: {}",
            bad[0].1
        );
    }

    #[test]
    fn a_coherent_winget_entry_passes_the_same_guard() {
        // The positive sibling: without it, a guard that refuses every
        // winget entry unconditionally -- which would ALSO make the test
        // above pass -- goes undetected.
        let lock = crate::lock::parse(
            "[winget.\"Git.Git\"]\nversion = \"2.55.0\"\npin = \"version-only\"\n",
        )
        .unwrap();
        assert!(
            incoherent_entries(&lock).is_empty(),
            "a well-formed winget entry must not be reported as incoherent"
        );
        lock_coherence_guard(&lock).unwrap();
    }

    #[test]
    fn a_pin_shape_that_does_not_match_the_map_it_is_stored_in_is_still_refused() {
        // The property Task 15's own brief names as its job to preserve:
        // Task 7 made a winget resolution carrying a commit a compile error,
        // but nothing stops the OTHER mistake -- a `Pin::WingetVersion`
        // landing in `lock.scoop` because of a routing bug in
        // `src/update.rs`'s `fold_backend`. `entry_coherence`'s per-variant
        // match alone cannot catch this (a well-formed `WingetVersion` is
        // coherent by ITS OWN rules regardless of which map holds it) --
        // `in_map` is what still catches it. Built directly rather than
        // through `lock::parse`, which cannot produce this shape at all: a
        // `[scoop.*]` table always parses to `Pin::ScoopCommit` and a
        // `[winget.*]` table always parses to `Pin::WingetVersion` (see
        // `lock.rs`'s own `RawScoop`/`RawWinget`), so this is a state only
        // `resolve_into_lock` -- Rust code, not TOML -- could produce.
        use crate::lock::Pin;
        let mut lock = Lock::default();
        lock.scoop.insert(
            Name::new("tool"),
            Pin::WingetVersion {
                version: "1.0.0".into(),
            },
        );
        let result = lock_coherence_guard(&lock);
        assert!(
            result.is_err(),
            "a Pin::WingetVersion in lock.scoop must be refused: {result:?}"
        );
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("winget pin") && msg.contains("scoop"),
            "name what is wrong, not just that something is: {msg}"
        );

        let mut reversed = Lock::default();
        reversed.winget.insert(
            Name::new("tool"),
            Pin::ScoopCommit {
                bucket: "main".into(),
                commit: "a".repeat(40),
                version: "1.0.0".into(),
            },
        );
        let reversed_result = lock_coherence_guard(&reversed);
        assert!(
            reversed_result.is_err(),
            "the reverse mismatch (a Pin::ScoopCommit in lock.winget) must be refused too: \
             {reversed_result:?}"
        );
        assert!(
            format!("{:#}", reversed_result.unwrap_err()).contains("scoop pin"),
            "and name what is wrong about it"
        );
    }

    #[test]
    fn prepare_passes_reports_through_without_touching_the_verdict() {
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let lock = Lock::default();
        let declared = Config::default();
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

        let prep = prepare(&plan, &lock, &scoop, &scoop, stage_dir.path(), &declared);
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

    // -- plan_to_steps() -------------------------------------------------
    //
    // Unverified before this round: the reviewer enumerated all 42 (action x
    // outcome) pairs and found the behaviour correct but untested -- swapping
    // an arm so that an `Install` carrying a stray `ReadyToRemove` becomes a
    // `Step::Remove` kept every existing test green.

    #[test]
    fn the_three_ready_shapes_produce_their_matching_steps() {
        let prep = Preparation {
            prepared: vec![
                Prepared {
                    action: Action::Install {
                        backend: SCOOP.into(),
                        name: Name::new("fzf"),
                        version: "1.0.0".into(),
                        arch: None,
                    },
                    outcome: Outcome::ReadyToFetch {
                        manifest: PathBuf::from("/stage/fzf/1.0.0/fzf.json"),
                    },
                },
                Prepared {
                    action: Action::Upgrade {
                        backend: SCOOP.into(),
                        name: Name::new("bat"),
                        from: "1".into(),
                        to: "2".into(),
                        arch: Some("arm64".into()),
                    },
                    outcome: Outcome::ReadyToFetch {
                        manifest: PathBuf::from("/stage/bat/2/bat.json"),
                    },
                },
                Prepared {
                    action: Action::Downgrade {
                        backend: SCOOP.into(),
                        name: Name::new("ripgrep"),
                        from: "2".into(),
                        to: "1".into(),
                        arch: None,
                    },
                    outcome: Outcome::ReadyToFetch {
                        manifest: PathBuf::from("/stage/ripgrep/1/ripgrep.json"),
                    },
                },
                Prepared {
                    action: Action::Prune {
                        backend: SCOOP.into(),
                        name: Name::new("aichat"),
                        version: "0.30.0".into(),
                    },
                    outcome: Outcome::ReadyToRemove,
                },
            ],
        };

        let (steps, unusable) = plan_to_steps(&prep);
        assert!(unusable.is_empty(), "{unusable:?}");
        assert_eq!(
            steps,
            vec![
                Step::Install {
                    app: Name::new("fzf"),
                    staged: PathBuf::from("/stage/fzf/1.0.0/fzf.json"),
                    arch: None,
                },
                Step::Replace {
                    app: Name::new("bat"),
                    staged: PathBuf::from("/stage/bat/2/bat.json"),
                    arch: Some("arm64".into()),
                },
                Step::Replace {
                    app: Name::new("ripgrep"),
                    staged: PathBuf::from("/stage/ripgrep/1/ripgrep.json"),
                    arch: None,
                },
                Step::Remove {
                    app: Name::new("aichat"),
                },
            ]
        );
    }

    #[test]
    fn an_install_action_carrying_a_stray_readytoremove_produces_nothing() {
        // The invariant `plan_to_steps` exists to hold: branch on the ACTION,
        // never on the outcome alone. Nothing in the type system stops an
        // `Outcome::ReadyToRemove` from being attached to an `Install` --
        // this is exactly the pair a version that matched on the outcome by
        // itself would turn into a `Step::Remove` for a package nobody asked
        // to remove.
        let prep = Preparation {
            prepared: vec![Prepared {
                action: Action::Install {
                    backend: SCOOP.into(),
                    name: Name::new("fzf"),
                    version: "1.0.0".into(),
                    arch: None,
                },
                outcome: Outcome::ReadyToRemove,
            }],
        };

        let (steps, unusable) = plan_to_steps(&prep);
        assert!(steps.is_empty(), "{steps:?}");
        assert!(unusable.is_empty(), "{unusable:?}");
    }

    #[test]
    fn a_prune_action_carrying_a_stray_readytofetch_produces_nothing() {
        // The mirror image of the test above.
        let prep = Preparation {
            prepared: vec![Prepared {
                action: Action::Prune {
                    backend: SCOOP.into(),
                    name: Name::new("aichat"),
                    version: "0.30.0".into(),
                },
                outcome: Outcome::ReadyToFetch {
                    manifest: PathBuf::from("/stage/aichat/0.30.0/aichat.json"),
                },
            }],
        };

        let (steps, unusable) = plan_to_steps(&prep);
        assert!(steps.is_empty(), "{steps:?}");
        assert!(unusable.is_empty(), "{unusable:?}");
    }

    #[test]
    fn failed_and_not_locked_land_in_unusable_with_their_reasons() {
        let prep = Preparation {
            prepared: vec![
                Prepared {
                    action: Action::Install {
                        backend: SCOOP.into(),
                        name: Name::new("fzf"),
                        version: "1.0.0".into(),
                        arch: None,
                    },
                    outcome: Outcome::Failed {
                        why: "hash mismatch".into(),
                    },
                },
                Prepared {
                    action: Action::Skip {
                        backend: SCOOP.into(),
                        name: Name::new("bat"),
                        reason: SkipReason::NotLocked,
                    },
                    outcome: Outcome::NotLocked,
                },
            ],
        };

        let (steps, unusable) = plan_to_steps(&prep);
        assert!(steps.is_empty(), "{steps:?}");
        assert_eq!(
            unusable,
            vec![
                (Name::new("fzf"), "hash mismatch".to_string()),
                (
                    Name::new("bat"),
                    "no lock entry -- run `dotpkg update`".to_string()
                ),
            ]
        );
    }

    #[test]
    fn a_running_skip_lands_in_unusable_too() {
        // The fix for Important 5: `Outcome::Skipped` reached neither list --
        // it fell into the wildcard arm and vanished, even though it is
        // exactly "a package that could not become a step, and why", the
        // same shape `unusable` already exists to hold for `Failed` and
        // `NotLocked`.
        let prep = Preparation {
            prepared: vec![Prepared {
                action: Action::Skip {
                    backend: SCOOP.into(),
                    name: Name::new("kanata"),
                    reason: SkipReason::Running,
                },
                outcome: Outcome::Skipped {
                    why: "running -- stop it first".into(),
                },
            }],
        };

        let (steps, unusable) = plan_to_steps(&prep);
        assert!(steps.is_empty(), "{steps:?}");
        assert_eq!(
            unusable,
            vec![(Name::new("kanata"), "running -- stop it first".to_string())]
        );
    }

    #[test]
    fn a_report_outcome_produces_neither_a_step_nor_an_unusable_entry() {
        // `Report` (`Unmanaged`, `ArchDrift`) is the one outcome that is
        // neither ready nor an error dotpkg caused -- it must land in the
        // wildcard arm, not `unusable`, or a caller printing `unusable` as
        // "packages that need attention" would wrongly nag about packages
        // dotpkg never touched at all.
        let prep = Preparation {
            prepared: vec![Prepared {
                action: Action::Unmanaged {
                    backend: SCOOP.into(),
                    name: Name::new("antigravity"),
                    version: "2.0.6".into(),
                },
                outcome: Outcome::Report,
            }],
        };

        let (steps, unusable) = plan_to_steps(&prep);
        assert!(steps.is_empty(), "{steps:?}");
        assert!(unusable.is_empty(), "{unusable:?}");
    }

    // -- confirm() ---------------------------------------------------------

    #[test]
    fn no_answer_at_all_means_no_and_says_which_flag_would_have_helped() {
        // A scheduled task with no console gives a child process an immediately
        // closed stdin: read_line returns Ok(0). That is the exact shape the
        // a14 dogfood runs under, and it must never read as consent.
        let mut empty: &[u8] = b"";
        let mut err = Vec::new();
        let answered = confirm("Continue? [y/N] ", &mut empty, &mut err).unwrap();
        assert!(!answered, "an empty stdin must not be a yes");
        let text = String::from_utf8(err).unwrap();
        assert!(text.contains("--yes"), "say what to pass instead: {text}");
    }

    #[test]
    fn only_an_explicit_yes_is_a_yes() {
        for (input, expected) in [
            ("y\n", true),
            ("Y\n", true),
            ("yes\n", true),
            ("\n", false),
            ("n\n", false),
            ("no\n", false),
            ("Yes please\n", false),
            ("  y  \n", true),
        ] {
            let mut r = input.as_bytes();
            let mut err = Vec::new();
            assert_eq!(
                confirm("q", &mut r, &mut err).unwrap(),
                expected,
                "input {input:?}"
            );
        }
    }

    #[test]
    fn the_question_goes_to_stderr_so_a_piped_run_still_shows_it() {
        let mut r: &[u8] = b"y\n";
        let mut err = Vec::new();
        confirm("Continue? [y/N] ", &mut r, &mut err).unwrap();
        assert!(String::from_utf8(err).unwrap().contains("Continue?"));
    }

    #[test]
    fn a_read_error_is_also_treated_as_no() {
        // `Ok(0)` (no console at all) has its own tests above; this is the
        // OTHER way `read_line` can fail to produce an answer -- invalid
        // UTF-8 on the pipe -- and it must refuse the same way, not fall
        // through to whatever the half-filled `line` buffer happens to hold.
        struct Broken(bool);
        impl std::io::Read for Broken {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0 || buf.is_empty() {
                    return Ok(0);
                }
                buf[0] = 0xFF; // not valid UTF-8 on its own
                self.0 = true;
                Ok(1)
            }
        }
        let mut r = std::io::BufReader::new(Broken(false));
        let mut err = Vec::new();
        let answered = confirm("q", &mut r, &mut err).unwrap();
        assert!(!answered, "a read error must not be a yes");
        let text = String::from_utf8(err).unwrap();
        assert!(text.contains("--yes"), "say what to pass instead: {text}");
    }

    // -- gate_removals() -----------------------------------------------
    //
    // This is the whole halt-versus-proceed decision for a removal: no flag
    // -- not `--yes`, not `--keep-going` -- may open this gate on its own.

    #[test]
    fn gate_removals_holds_every_prune_back_when_the_preparation_is_not_ok() {
        let steps = vec![
            Step::Install {
                app: Name::new("fzf"),
                staged: PathBuf::from("/stage/fzf/1.0.0/fzf.json"),
                arch: None,
            },
            Step::Remove {
                app: Name::new("aichat"),
            },
            Step::Remove {
                app: Name::new("kanata"),
            },
        ];
        let (kept, held) = gate_removals(steps, false);
        assert_eq!(
            kept,
            vec![Step::Install {
                app: Name::new("fzf"),
                staged: PathBuf::from("/stage/fzf/1.0.0/fzf.json"),
                arch: None,
            }],
            "a non-removal step must still run"
        );
        assert_eq!(held, vec![Name::new("aichat"), Name::new("kanata")]);
    }

    #[test]
    fn gate_removals_lets_every_step_through_when_the_preparation_is_ok() {
        // The positive control: without it, a version that always holds
        // everything back would pass the test above too.
        let steps = vec![
            Step::Remove {
                app: Name::new("aichat"),
            },
            Step::Install {
                app: Name::new("fzf"),
                staged: PathBuf::from("/stage/fzf/1.0.0/fzf.json"),
                arch: None,
            },
        ];
        let (kept, held) = gate_removals(steps.clone(), true);
        assert_eq!(kept, steps);
        assert!(held.is_empty());
    }
}
