use crate::backend::scoop::Scoop;
use crate::backend::winget::WingetCmd;
use crate::backend::Backend;
use crate::config::Config;
use crate::execute::{ScoopStep, Step, WingetStep};
use crate::lock::Lock;
use crate::lock::Pin;
use crate::model::{Installed, Name, SCOOP, WINGET};
use crate::plan::{Action, Plan, SkipReason};
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
    /// Needs nothing staged, because winget has no local manifest to stage --
    /// but the version `pkg.lock` pins still has to be *in winget's index*
    /// before `apply` may promise to install it. What `--prepare` can check
    /// for a winget action, and all it can check: `winget show --id <id> -v
    /// <pin>` (`backend::winget::version_liveness`).
    ///
    /// Its own `Intent` rather than a second reading of `NeedsArtifact`: the
    /// two do different work, produce different `Outcome`s, and a caller that
    /// confused them would ask `Scoop::stage` for a manifest that does not
    /// exist.
    NeedsLiveness,
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
        // The one place the two backends' preparations part company, and it
        // is a fact about what each backend *has*, not about what dotpkg may
        // do to it: scoop pins a manifest in a bucket, so preparing means
        // staging and fetching that manifest and hash-verifying it; winget
        // holds the manifest itself and hands out nothing, so there is
        // nothing to stage and the only thing `--prepare` can establish is
        // that the pinned version is still in winget's index.
        //
        // A third backend falls to `NeedsArtifact` and then to
        // `stage_and_fetch`'s `backend != SCOOP` guard, which reports it as a
        // per-package failure rather than staging something meaningless.
        Action::Install { backend, .. }
        | Action::Upgrade { backend, .. }
        | Action::Downgrade { backend, .. } => {
            if backend == WINGET {
                Intent::NeedsLiveness
            } else {
                Intent::NeedsArtifact
            }
        }
        Action::Prune { .. } => Intent::NoArtifactNeeded,
        Action::Skip { reason, .. } => match reason {
            SkipReason::NotLocked => Intent::NotLocked,
            SkipReason::Running => Intent::Skip("running -- stop it first".to_string()),
            // Same shape as Running: dotpkg cannot act on a state it could not
            // establish, and installing over it is exactly the mistake this
            // variant exists to prevent. The user fixes the read (usually by
            // rerunning without the elevated/restricted context) and reruns.
            SkipReason::Opaque => Intent::Skip(
                "installed, but its state could not be read -- see the warnings above".to_string(),
            ),
            // The whole-backend sibling of `Opaque`: nothing about this
            // package specifically failed to read, its entire backend's scan
            // did, so `installed`'s emptiness for it proves nothing either
            // way. Installing over an unknown state is exactly the mistake
            // this variant exists to prevent, same as `Opaque`.
            SkipReason::Unscannable => Intent::Skip(
                "this backend could not be scanned -- see the warnings above; nothing was \
                 attempted for it"
                    .to_string(),
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
    /// Ready, and nothing was fetched: winget has no local manifest to stage.
    /// The pinned version was confirmed still present in winget's index.
    ///
    /// A separate variant rather than `ReadyToFetch { manifest: None }`, for
    /// the reason `ReadyToFetch` and `ReadyToRemove` were split from one
    /// another in Phase 2b-2: as one variant, "no manifest" would mean "this
    /// is a winget action" only for values `prepare` itself produced, and an
    /// executor branching on `manifest.is_none()` would be right by luck.
    ///
    /// Carries the version so `plan_to_steps` builds `WingetStep::Set` from
    /// the version preparation actually confirmed, not from a second reading
    /// of the action or the lock.
    ///
    /// **And it carries `id`, which is NOT the action's `name`.** The action's
    /// name is `pkg.toml`'s spelling -- whatever the user typed, a supported
    /// state rather than a typo the tool rejects (`update` warns about a case
    /// mismatch and says "pkg.toml is left as you wrote it"; `adopt`
    /// deliberately writes the user's spelling). `id` is the canonical spelling
    /// winget itself echoed back in `Found <name> [<Id>]`, which
    /// `version_liveness` obtained by deliberately omitting `--exact` so winget
    /// would fold case on the way in.
    ///
    /// Two spellings, and only one of them may reach a mutating argv:
    /// `set_argv` puts `-e` beside `--id`, and `--exact` is what makes `--id`
    /// case-sensitive on the write verbs too (measured,
    /// `docs/measurements-2026-08-10-winget-write-path.md` §6: `install -e --id
    /// SHARKDP.HYPERFINE --version <x>` returns `0x8A150014` "No package found
    /// matching input criteria." where the correctly-cased call reaches
    /// `0x8A150017`). A `Set` built from the declared spelling therefore can
    /// never install the package, and the rescan -- `list -e --id <the same
    /// wrong spelling>` -- misses too, so the run reports the package does not
    /// exist. Every run, forever. The design named this outright: *"a mutating
    /// call may use `-e --id` only with a spelling winget itself produced."*
    ///
    /// Splitting the two here rather than rewriting the action is deliberate:
    /// the plan the user reads, and `pkg.toml`, keep their own spelling.
    ReadyToSet { id: Name, version: String },
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
/// removal step (`Step::is_remove()`) back whenever `is_ok()` is false --
/// `--keep-going` included, and no other flag opens that gate either --
/// because every newly typed package name is `NotLocked` until `update`
/// exists, which makes "installs nothing, deletes something" the one shape a
/// not-ok preparation can produce today.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Preparation {
    pub prepared: Vec<Prepared>,
}

impl Preparation {
    /// Every ready shape: a scoop install with its manifest fetched, a removal
    /// that needed nothing fetched, and a winget `Set` whose pinned version
    /// was confirmed still in winget's index. Counted together because the
    /// user-facing number is "how much of this plan can go ahead".
    ///
    /// Also the left-hand side of the invariant `main.rs` checks after
    /// routing: every ready outcome must become exactly one `Step`, so
    /// `ready_count()` and `steps + held` agreeing is what says no ready
    /// action was silently dropped by a missing `plan_to_steps` arm.
    ///
    /// **Deliberately still counts a winget `Downgrade`'s `ReadyToSet`.** The
    /// pin really is live in winget's index -- `check_pin_is_live` has no way
    /// to know it is a downgrade, and must not guess -- and `plan_to_steps`
    /// really does build a `WingetStep::Set` for it and fire it, so excluding
    /// it here would make this method's own invariant with `plan_to_steps`
    /// false for exactly this case. The user-facing "ready" number
    /// `render_preparation` prints subtracts it separately, at the point
    /// that number is actually built -- see `refused_winget_downgrade_count`.
    pub fn ready_count(&self) -> usize {
        self.prepared
            .iter()
            .filter(|p| {
                matches!(
                    p.outcome,
                    Outcome::ReadyToFetch { .. }
                        | Outcome::ReadyToRemove
                        | Outcome::ReadyToSet { .. }
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

    /// Winget downgrades whose pin was confirmed still live in winget's
    /// index -- so `check_pin_is_live` produced `Outcome::ReadyToSet` for
    /// them, and `plan_to_steps` will build a `WingetStep::Set` and fire
    /// `install --version <pin>` -- but that call is winget's own measured
    /// refusal every time (`Plan::change_count`'s own doc comment), not a
    /// change going ahead. The post-merge audit's I2: `--prepare` printed
    /// `ready` for exactly this shape and exited 0, and the consent prompt
    /// counted it as "1 installed" one line below the plan's own "0
    /// change(s)" for the same package.
    ///
    /// Deliberately gated on `Outcome::ReadyToSet`, not on the action alone:
    /// a winget `Downgrade` whose liveness check itself failed is already
    /// `Outcome::Failed` and already correctly counted as a failure, not a
    /// refusal. Counting it here too would double one package into two
    /// explanations, and -- at `main.rs`'s one call site that subtracts this
    /// from an `installs` count built from real `Step`s -- would subtract a
    /// package that produced no step at all, undercounting an unrelated
    /// install sharing the run.
    ///
    /// Its own method, mirroring `Plan::refused_downgrade_count` one layer
    /// up, rather than folded into `ready_count`: excluding it there instead
    /// would make `ready_count`'s own invariant with `plan_to_steps` (every
    /// ready outcome becomes exactly one step) false for this one case, for
    /// a number `main.rs` uses to catch a routing bug, not to inform a user.
    /// See `ready_count`'s own doc comment.
    pub fn refused_winget_downgrade_count(&self) -> usize {
        self.prepared
            .iter()
            .filter(|p| {
                matches!(p.outcome, Outcome::ReadyToSet { .. })
                    && matches!(&p.action, Action::Downgrade { backend, .. } if backend == WINGET)
            })
            .count()
    }

    /// The run's verdict. `NotLocked` fails it for the same reason `Failed`
    /// does -- apply has no way to fix either one itself -- unlike `Skipped`,
    /// which the user can clear from outside dotpkg and rerun.
    pub fn is_ok(&self) -> bool {
        self.failed_count() == 0 && self.not_locked_count() == 0
    }

    /// How many packages **could not be prepared** -- exactly the set that
    /// makes `is_ok()` false, and exactly the number `main.rs` prints in "N
    /// package(s) could not be prepared, so nothing has been changed".
    ///
    /// Its own method because that sentence used to be printed with
    /// `plan_to_steps`'s `unusable.len()`, which is a different and wider set:
    /// `unusable` is every package that did not become a step, so it also
    /// counts every benign `Outcome::Skipped` (a running app the user can
    /// close, which does not fail the run) and, in principle, a routing bug (an
    /// action that WAS prepared, and so certainly not one that could not be).
    /// One real failure alongside two running apps printed "3 package(s) could
    /// not be prepared" -- a false number in a refusal message, which is the
    /// same defect class as a false number in the confirmation prompt.
    pub fn unpreparable_count(&self) -> usize {
        self.failed_count() + self.not_locked_count()
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
    /// already fails `is_ok()` on its own). Two variants have been the reason
    /// these counts differed and both are gone:
    /// `SkipReason::BackendNotImplemented` (deleted with the stub loop it
    /// existed for when Phase 4 Task 14 gave winget a real planner view) and
    /// `SkipReason::ReportedOnly` (deleted by Phase 4b Task 13 when winget got an
    /// executor and stopped having anything to report-only about). Nothing has
    /// taken their place. The match in `is_outstanding` stays exhaustive with
    /// no wildcard for exactly this reason: the day a
    /// `Skipped`-but-not-outstanding `SkipReason` exists again, this narrows
    /// correctly without anyone having to remember to widen it back.
    ///
    /// Named for what the query answers, not for the first variant it
    /// recognised (`running_skips`, before review caught that `Opaque` had
    /// silently been left out of it): a skip's *kind* decides whether it
    /// belongs here, and that decision lives in `is_outstanding`, not in
    /// this method's name.
    ///
    /// Each entry carries its **backend** as well as its name, because
    /// `main.rs` pushes these into `Execution::results` for the closing table
    /// and that table names a backend per line. A winget package skipped for
    /// `Running` or `Opaque` has been reachable since Phase 4 Task 14, so
    /// without the backend here the table has been calling those packages
    /// `scoop` ever since -- see `execute::ItemOutcome`.
    pub fn outstanding_skips(&self) -> Vec<(String, Name, String)> {
        self.prepared
            .iter()
            .filter_map(|p| match (&p.action, &p.outcome) {
                (Action::Skip { reason, .. }, Outcome::Skipped { why })
                    if is_outstanding(reason) =>
                {
                    Some((
                        action_backend(&p.action).to_string(),
                        action_name(&p.action),
                        why.clone(),
                    ))
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
/// That shape has already earned itself twice -- `SkipReason::ReportedOnly`
/// arrived (Phase 4 Task 14) and was deleted again (Phase 4b Task 13, when winget got an
/// executor), and on both days the floor-or-not answer had to be a decision
/// made here rather than an oversight the match would have let slip through.
fn is_outstanding(reason: &SkipReason) -> bool {
    match reason {
        // Both are facts about this run that could differ on the next one:
        // the process could be closed, the unreadable state could become
        // readable. Outstanding work the user asked for and did not get.
        SkipReason::Running | SkipReason::Opaque => true,
        SkipReason::NotLocked => false,
        // A scan failure is a fact about this run, not a permanent property
        // of the machine: the next run's `winget.exe` might not be
        // permission-denied, or might be back on `PATH`. Floors the exit
        // code and appears in the closing table exactly like `Running` and
        // `Opaque` -- outstanding work the user asked for and did not get.
        SkipReason::Unscannable => true,
    }
}

/// Walk the plan and try to make every action that needs preparing ready:
/// for scoop, recover its pinned manifest, stage it under `staging_root`,
/// then fetch and hash-verify it; for winget, confirm the pinned version is
/// still in winget's index. This is the entire phase: nothing here installs,
/// uninstalls, or otherwise changes anything already on the machine. The only
/// filesystem writes are inside `staging_root`, and the only commands ever
/// run are `scoop download` and `winget show`, neither of which mutates
/// installed software. `winget show` is run **once per winget action, twice
/// when a pin has fallen out of the index** -- see `check_pin_is_live` for what
/// that costs.
///
/// A per-package failure is recorded in that package's `Outcome` and the walk
/// continues -- one bad package must never hide, or stop, the others.
///
/// `declared` is `pkg.toml`, parsed: `stage_and_fetch` uses it to check a
/// pin's bucket against `[scoop] buckets` before ever touching the disk.
///
/// `winget` is the read-only half of winget's seam (`WingetCmd`, not
/// `WingetMutator`): the type alone says this function cannot install or
/// remove a winget package even by mistake, which is the same argument
/// `Step`/`ScoopStep` makes for `execute`.
pub fn prepare(
    plan: &Plan,
    lock: &Lock,
    scoop: &Scoop,
    mutator: &dyn crate::execute::Mutator,
    winget: &dyn WingetCmd,
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
                Intent::NeedsLiveness => check_pin_is_live(action, winget),
                Intent::NoArtifactNeeded => Outcome::ReadyToRemove,
                Intent::Skip(why) => Outcome::Skipped { why },
                Intent::NotLocked => Outcome::NotLocked,
                Intent::Report => Outcome::Report,
            },
        })
        .collect();
    Preparation { prepared }
}

/// The `NeedsLiveness` half of `prepare`: winget's whole preparation.
///
/// The version asked about is the one the **plan** resolved from `pkg.lock`
/// (`Install`'s `version`, `Upgrade`/`Downgrade`'s `to`), not a second read of
/// the lock -- the plan is the thing the user is shown and says yes to, so it
/// is the thing preparation must confirm.
///
/// **Cost: one `winget show` subprocess per winget action in the plan, run
/// serially, and two when the pin has fallen out of the index** --
/// `version_liveness`'s `NO_VERSION_FOUND` branch asks `show --versions` as
/// well, to say how deep the publisher's retention goes. The nearest measured
/// figure for a `show` invocation on a real machine is ~1.09 s
/// (`docs/measurements-2026-08-09-winget.md`), so a plan with ten winget
/// changes spends roughly ten seconds in `--prepare` before anything is
/// attempted. Nothing here is parallelised or cached, and no measurement of
/// this loop itself exists yet.
///
/// **A transient winget failure refuses the whole run, scoop included.**
/// `version_liveness` returns `Err` for *any* nonzero exit code, not only the
/// two it names, so a winget that is momentarily unhappy -- a locked index, a
/// source mid-update -- becomes `Outcome::Failed`, which fails
/// `Preparation::is_ok`, which exits 2 with no scoop action performed either.
/// That is fail-closed on purpose: the alternative is installing a version
/// dotpkg could not confirm, and `--keep-going` is the documented way to let
/// the ready packages through. It is nonetheless a **new failure mode** this
/// task introduced, and a retry policy is the obvious future refinement.
///
/// **A missing `winget.exe` lands here, and lands as `Failed` -- deliberately.
/// This is the whole answer for the three mutating actions; a `Prune` is safe
/// for a different reason, named at the end of this comment.** `Winget::scan`
/// routes `CmdError::NotFound` to an empty `Scan` plus a warning, because a machine
/// with no winget is a legitimate machine; the cost is that "winget is not
/// installed" and "winget found nothing installed" are spelled the same way in
/// `installed`, so a declared *and locked* winget package on such a machine
/// plans as an `Install`. While winget only reported, that `Install` was a
/// report line. Now that winget acts it would be a real `winget install`
/// against a binary that is not there -- except that it never gets that far:
/// this liveness check runs `winget show` first, `WingetCmd::run` returns
/// `CmdError::NotFound`, and the package becomes `Outcome::Failed` ("winget
/// show could not be run: winget.exe is not on PATH"). It therefore never
/// becomes a `Step`, it fails `Preparation::is_ok`, and by default the whole
/// run refuses before `execute` is ever called -- `--keep-going` does not
/// change that for this package either, since a `Failed` outcome produces no
/// step to keep going with. Reported per package rather than as one
/// whole-backend error because that is what the rest of this function already
/// is, and because the same run's scoop half is none of winget's business.
///
/// **The `Prune` direction never reaches this function, and is safe for a
/// different reason** -- so do not read the paragraph above as covering it. A
/// `Prune` is `Intent::NoArtifactNeeded`, ready by definition, and on a machine
/// with no `winget.exe` no winget `Prune` is ever emitted in the first place:
/// the scan is empty, and `plan_backend`'s undeclared loop only iterates
/// `installed`. That is `backend::scan_or_warn`'s own argument, in its own doc
/// comment, and it is the half of the problem `ScanOutcome::Unscannable`
/// deliberately does *not* need to cover.
fn check_pin_is_live(action: &Action, winget: &dyn WingetCmd) -> Outcome {
    let (name, version) = match action {
        Action::Install { name, version, .. } => (name, version),
        Action::Upgrade { name, to, .. } | Action::Downgrade { name, to, .. } => (name, to),
        // `classify` returns `Intent::NeedsLiveness` only for the three
        // variants above. Kept as a `Failed` outcome, not a panic, for the
        // same reason `stage_and_fetch`'s own version of this arm is: a
        // future mismatch between the two functions must be a reported
        // failure, not a crashed run.
        _ => {
            return Outcome::Failed {
                why: format!("{action:?} needs a liveness check but names no version"),
            }
        }
    };
    match crate::backend::winget::version_liveness(winget, name, version) {
        // `found.id`, never `name`. This call was made WITHOUT `--exact`
        // precisely so winget would fold case on the way in and hand the
        // canonical spelling back on the way out, in the same self-verifying
        // `Found <name> [<Id>]` line -- so the answer to "what may go on a
        // mutating wire" is already in hand here and must not be thrown away.
        // `name` is `pkg.toml`'s spelling and stays in the action, the plan and
        // the file; see `Outcome::ReadyToSet`'s own doc comment for what putting
        // it on the wire beside `-e` costs.
        Ok(found) => Outcome::ReadyToSet {
            id: Name::new(found.id),
            version: version.clone(),
        },
        Err(why) => Outcome::Failed { why },
    }
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
    // Winget no longer reaches here at all -- `classify` sends it to
    // `Intent::NeedsLiveness`, because it has no manifest to stage rather than
    // because dotpkg may not act on it. So this now guards exactly one thing:
    // a *third* backend, added without a preparation of its own, must be
    // reported rather than handed to `Scoop::stage`, which would look its name
    // up in a scoop bucket and fail with a sentence about buckets.
    if backend != SCOOP {
        return Outcome::Failed {
            why: format!("{backend}: dotpkg has no way to prepare a {backend} package"),
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
pub fn plan_to_steps(
    prep: &Preparation,
    installed: &[Installed],
) -> (Vec<Step>, Vec<(Name, String)>) {
    let mut steps = Vec::new();
    let mut unusable = Vec::new();
    for p in &prep.prepared {
        // Branch on the ACTION, never on the outcome: `Outcome::ReadyToRemove`
        // is still attachable to an `Install`, and nothing in the type system
        // binds the two. Every ready arm also checks `backend`, so a ready
        // outcome paired with the wrong backend routes to no arm at all and
        // falls to the routing-bug arm below rather than being executed by the
        // wrong package manager.
        match (&p.action, &p.outcome) {
            (
                Action::Install {
                    backend,
                    name,
                    arch,
                    ..
                },
                Outcome::ReadyToFetch { manifest },
            ) if backend == SCOOP => steps.push(Step::Scoop(ScoopStep::Install {
                app: name.clone(),
                staged: manifest.clone(),
                arch: arch.clone(),
            })),
            (
                Action::Upgrade {
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
                },
                Outcome::ReadyToFetch { manifest },
            ) if backend == SCOOP => steps.push(Step::Scoop(ScoopStep::Replace {
                app: name.clone(),
                staged: manifest.clone(),
                arch: arch.clone(),
            })),
            (Action::Prune { backend, name, .. }, Outcome::ReadyToRemove) if backend == SCOOP => {
                steps.push(Step::Scoop(ScoopStep::Remove { app: name.clone() }))
            }
            // One `Set` for all three, because a winget version change is one
            // `install --version` call in either direction -- see
            // `WingetStep`'s own doc comment for the measurement, and note
            // that dotpkg does not decide the direction: winget's own refusal
            // of a downgrade is translated where it is measured, not
            // pre-judged here.
            //
            // **`id` comes from the outcome, not from the action.** The action's
            // `name` is `pkg.toml`'s spelling; the outcome's `id` is the
            // canonical one winget echoed back during the liveness check, and
            // that is the only spelling `set_argv`'s `-e --id` may carry. See
            // `Outcome::ReadyToSet`'s own doc comment. `guard_for` still takes
            // `name`: it looks the scan up by `Name`, which folds case, so
            // either spelling finds the same row, and `name` is what every other
            // arm here passes.
            //
            // **Uncovered residual, *reasoned only*: the two spellings also
            // meet in the mid-run fence, and nothing measures whether they
            // ever differ.** `Running.dirs` holds `winget list`'s `Id`
            // (`backend::winget_fence_ids` -> `running_ids`, which inserts the
            // scanned id itself), while the `id` stored here -- what
            // `Step::app()` returns for this step -- is `winget show`'s `Id`.
            // `execute`'s per-step re-sampler asks `Running::covers_any(app,
            // guard)`, whose `dirs` half is `dirs.contains(app)`. `Name`
            // folds case, so a case-only difference is absorbed; only a
            // difference that is NOT case would make that half silently
            // answer "not running" mid-run for a package the plan-time fence
            // could see. The `bins` half would not compensate: `guard_for`
            // supplies plausible PROCESS names, a different signal that is
            // empty unless `[winget.guard]` names the package or
            // `guard_names` guesses right. Nothing in this phase's
            // measurement document compares a `winget list` `Id` against the
            // `winget show` `Id` for the same package, so there is no number
            // either way: recorded as an unmeasured residual, not as a bug.
            (
                Action::Install { backend, name, .. }
                | Action::Upgrade { backend, name, .. }
                | Action::Downgrade { backend, name, .. },
                Outcome::ReadyToSet { id, version },
            ) if backend == WINGET => steps.push(Step::Winget(WingetStep::Set {
                id: id.clone(),
                version: version.clone(),
                guard: guard_for(name, installed),
            })),
            (
                Action::Prune {
                    backend,
                    name,
                    version,
                },
                Outcome::ReadyToRemove,
            ) if backend == WINGET => steps.push(Step::Winget(WingetStep::Remove {
                id: name.clone(),
                version: version.clone(),
                guard: guard_for(name, installed),
            })),
            (a, Outcome::Failed { why }) => unusable.push((action_name(a), why.clone())),
            (a, Outcome::NotLocked) => unusable.push((
                action_name(a),
                "no lock entry -- run `dotpkg update`".to_string(),
            )),
            (a, Outcome::Skipped { why }) => unusable.push((action_name(a), why.clone())),
            // A prepared action nobody routed. Reachable only by a
            // backend/outcome pairing no real `prepare()` call produces -- an
            // action whose outcome does not match its own kind, or either
            // backend's action paired with the other's ready outcome. Loud
            // beats silently dropped: a deleted step here is a package the
            // plan promised and the machine never got.
            //
            // **This is not "could not be prepared", and `main.rs` does not
            // count it as such.** A ready outcome does not raise
            // `failed_count()` or `not_locked_count()`, so `is_ok()` stays
            // true and the refusal path that prints "N package(s) could not be
            // prepared" is not even reached by this on its own -- which is
            // exactly why that sentence counts `failed_count() +
            // not_locked_count()` and not `unusable.len()`. What makes this
            // loud instead is `main.rs`'s own invariant check: every ready
            // outcome must become exactly one step, so `ready_count()`
            // exceeding `steps + held` is reported there as the routing bug
            // this arm's text calls it.
            (a, Outcome::ReadyToFetch { .. })
            | (a, Outcome::ReadyToRemove)
            | (a, Outcome::ReadyToSet { .. }) => unusable.push((
                action_name(a),
                format!(
                    "{}: prepared, but no executor claimed it -- this is a routing bug, \
                     not a package problem",
                    action_backend(a)
                ),
            )),
            _ => {}
        }
    }
    (steps, unusable)
}

/// The names a live process might report for a winget package, for
/// `execute`'s per-step re-sampler (`Step::guard_names`).
///
/// Read out of the scan's own `Installed.bins`, which
/// `backend::winget::rows_to_scan` filled with `guard_names(id, display)` --
/// the display Name column is only available at scan time, so re-deriving it
/// here would silently drop the one guess that catches Google Chrome.
///
/// Falls back to `guard_names(name, name)` when nothing is installed under
/// this name, which is every `Install`: there is no `Installed` to read a
/// display name from, and a package that is not installed cannot be running,
/// so the fallback's job is only to be defined rather than to be right --
/// `execute` will sample it, find nothing, and proceed. Matched by `Name`, so
/// a lock keyed `brave.brave` still finds a scan row spelled `Brave.Brave`.
fn guard_for(name: &Name, installed: &[Installed]) -> Vec<String> {
    installed
        .iter()
        .find(|i| i.backend == WINGET && &i.name == name)
        .map(|i| i.bins.clone())
        .unwrap_or_else(|| crate::backend::winget::guard_names(name.key(), name.key()))
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

/// Every `Action` variant's `backend` field, for a routing-bug message that
/// must name which backend was prepared but never claimed. Mirrors
/// `action_name` above, one field over.
fn action_backend(action: &Action) -> &str {
    match action {
        Action::Install { backend, .. }
        | Action::Upgrade { backend, .. }
        | Action::Downgrade { backend, .. }
        | Action::Prune { backend, .. }
        | Action::Skip { backend, .. }
        | Action::Unmanaged { backend, .. }
        | Action::ArchDrift { backend, .. } => backend,
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
    /// Winget's half of the same fact `scan` holds for scoop -- a
    /// `ScanOutcome` rather than a bare `Scan` since Task 6, so a genuine
    /// scan failure (`ScanOutcome::Unscannable`) cannot be confused with a
    /// scan that succeeded and simply found nothing. Kept as its own field
    /// rather than merged into `scan`: `main.rs` prints each scan's
    /// `warnings` attributed to its own backend ("warning: scoop: …" /
    /// "warning: winget: …"), and a plain `Vec<String>` cannot be told apart
    /// by backend once merged -- merging first would silently mislabel every
    /// winget warning as scoop's. `main.rs` concatenates `installed` and
    /// `opaque` from both fields itself, right before calling `plan()`,
    /// which is backend-agnostic for exactly those two (see `Scan`'s own doc
    /// comment).
    pub winget_scan: crate::backend::ScanOutcome,
    pub running: crate::model::Running,
}

/// The whole fence, sampled: which winget ids count, which winget roots to look
/// under, and the union with scoop's half. **Every production path calls this
/// and passes nothing but the three things it already holds.**
///
/// **This is here rather than inline in `main.rs` because the INPUTS were the
/// part no test could reach.** `backend::running_set` takes `winget_ids` and
/// `winget_roots` as parameters, so before this function existed, each of the
/// three call sites chose them for itself -- and one of the three lives in
/// `main.rs`'s per-step re-sampler closure.
///
/// **Why no test reached that closure's winget half, precisely.** Not because
/// `main.rs` is unobservable: `tests/cli.rs` spawns the real binary through
/// `CARGO_BIN_EXE_dotpkg`, and
/// `a_declared_package_skipped_as_running_is_outstanding_not_success`
/// (`tests/cli.rs`) already drives this very fence end to end -- it copies
/// the binary into `<scoop_root>/apps/aichat/current/`, runs it live, and asserts
/// the `held … running -- stop it first` line. What that test cannot reach is the
/// **winget** half, and the reason is narrower: `path_without_winget`
/// (`tests/cli.rs::path_without_winget`) strips every `PATH` directory holding `winget.exe` or
/// `winget` from every `cli.rs` fixture by design, so `Winget::scan` there always
/// returns an empty `Scan`, `winget_fence_ids` always returns an empty vector,
/// and `running_ids` has no scanned id to match a package directory against.
/// Overriding `LOCALAPPDATA` changes which roots `package_roots()` reports but
/// cannot help: with no scanned ids there is nothing for the path half to match.
///
/// **Measured, suite-wide, on Phase 5 Task 2's tree** -- `01df082`, the commit
/// that created this function, where the suite totalled 598 and still did at
/// Task 2's last commit `ea13a00`. With `main.rs`'s closure calling
/// `sample_fence_with_roots(&d.scoop, &d.winget_scan, &[], …)` -- the winget
/// roots dropped at the call site -- `cargo test --no-fail-fast` reported 598
/// passed, 0 failed across all fourteen binaries, and no compiler warning of any
/// kind. That is the whole suite of that tree, not a filtered run. **The total
/// is deliberately not restated for the tree this comment ships on**, which is
/// larger: naming the measured tree is what lets a reader tell "this number is
/// from an older tree" from "this number is wrong", and re-running the mutation
/// here would be a new measurement, not a re-labelling of this one.
///
/// This is the same remedy, for the same finding, that `gate_the_run` above
/// applies: Task 15's review found four driver lines whose *order* and
/// *arguments* no test pinned, and hoisting them into one library function
/// "leaves the driver one call to get wrong instead of three separately-placed
/// ones". Here it is not an order but a pair of inputs; the shape of the fix is
/// identical.
///
/// **Why two functions**, given that one of them has to take roots for a test to
/// fabricate them:
///
/// - If `winget_roots` were a parameter of the **production** entry point, every
///   call site would have to name it, and naming it is where the three sites
///   previously disagreed. Passing `&[]` would be an ordinary argument mistake on
///   the ordinary path.
/// - If `winget_roots` were only ever read inside one function with no
///   root-taking variant at all, no test could exercise the winget path half,
///   because `package_roots()` reads `LOCALAPPDATA` / `ProgramFiles` and returns
///   an empty vector on every non-Windows platform -- including the machine this
///   crate is developed on.
///
/// So `sample_fence` takes no roots and is what production calls;
/// `sample_fence_with_roots` takes fabricated ones and is what
/// `the_fence_unions_scoop_paths_with_winget_package_dirs` drives.
///
/// **Residual, stated plainly rather than claimed closed.** The split moves the
/// mistake off the default path; it does not make it unwritable, and it is worth
/// being exact about what remains, because an earlier draft of this comment
/// claimed "the choice of ids and roots is no longer expressible at a call site"
/// and that is false:
///
/// - `sample_fence_with_roots` is `pub`, so `main.rs` could call it with `&[]`.
///   **Measured on Task 2's tree** (`01df082`, 598 passed / 0 failed): doing
///   exactly that leaves the whole suite green with no warning -- see the
///   measurement above, including why that total is not the total here. Nothing
///   but review catches it.
/// - `backend::running_set` is also still `pub`, because three
///   `tests/scoop_scan.rs` tests call it directly, so that lower-level door is
///   open too.
/// - Each of the three sites still contains one call, and `main.rs`'s two are
///   unpinned: nothing goes red if someone deletes the re-sampler closure's call
///   outright or swaps it for `Running::default()`.
///
/// What the hoist *does* buy is real but narrower than "unwritable": writing the
/// mistake now requires deliberately reaching past the production entry point for
/// a differently-named function, rather than mis-filling one of its ordinary
/// parameters, and the correct inputs live in one place that a test does pin. That
/// is the same class of residual `gate_the_run` accepted and recorded ("one call
/// to get wrong instead of three separately-placed ones").
pub fn sample_fence(
    scoop: &Scoop,
    winget_scan: &crate::backend::ScanOutcome,
    procs: &[crate::sys::Process],
) -> crate::model::Running {
    sample_fence_with_roots(
        scoop,
        winget_scan,
        &crate::backend::winget::package_roots(),
        procs,
    )
}

/// `sample_fence`'s tested seam: the same union, against roots the caller
/// supplies. See `sample_fence`'s own doc comment for why this pair is two
/// functions and not one, and why production must never call this one directly.
pub fn sample_fence_with_roots(
    scoop: &Scoop,
    winget_scan: &crate::backend::ScanOutcome,
    winget_roots: &[PathBuf],
    procs: &[crate::sys::Process],
) -> crate::model::Running {
    let winget_ids = crate::backend::winget_fence_ids(winget_scan);
    crate::backend::running_set(scoop, &winget_ids, winget_roots, procs)
}

/// Which winget packages this run would change while dotpkg has no way to tell
/// whether they are running.
///
/// **The hole this names, in numbers rather than in principle.** The fence has
/// two halves. The path half (`backend::winget::running_ids`) can only ever fire
/// for a package that owns a directory under a winget package root, and winget
/// creates one only for a `portable` installer: measured on a14 on 2026-08-12,
/// **4 of 41** installed ids, with no exception in either direction across all
/// eight installer types present. The name half depends on
/// `backend::winget::guard_names`' two guesses, which are guesses -- for
/// `BurntSushi.ripgrep.MSVC` they are `msvc` and `ripgrep msvc`, and the process
/// is `rg`. A user who writes no `[winget.guard]` entry for one of the other 37
/// therefore gets no protection, **and until this function nothing said so**:
/// `docs/phase5-notes.md`'s still-open item 9 records exactly that, that dotpkg
/// "cannot tell them which entry they are missing".
///
/// **Why it is keyed on a pending change rather than on being installed.** A
/// line per unguarded installed package would be **32** lines on the measured
/// machine -- 4 of the 36 ids dotpkg can establish a fact about own a package
/// directory, and 32 do not. (Winget itself reports 41; the extra 5 are the
/// ones dotpkg refuses to read a version for, and it can raise no line about a
/// package it has no facts about, so 32 rather than 37 is the ceiling that
/// matters here.) That is a flood and not information -- and Phase 5 spent
/// itself
/// deleting lines from `status` for that reason. A package dotpkg is not about
/// to touch cannot be damaged by a fence that cannot see it, so the moment the
/// sentence is worth printing is the moment there is a change pending for it.
/// `Action::Install` is deliberately not one of those moments: nothing is
/// installed yet, so nothing of it can be running.
///
/// The two shapes that *are* moments -- `Upgrade` and `Prune` -- each replace
/// or remove a live installation, and each names itself in the message, because
/// "may upgrade it while it is running" is actionable in a way that "is
/// unprotected" is not. It was three until `Downgrade` was measured to be
/// refused rather than performed; this sentence said "three" for one commit
/// after that stopped being true, which is the class this repository keeps
/// paying for.
pub fn unprotected_winget_changes(
    plan: &Plan,
    guard: &std::collections::BTreeMap<Name, Vec<String>>,
    installed: &[Installed],
) -> Vec<String> {
    unprotected_winget_changes_with_roots(
        plan,
        guard,
        installed,
        &crate::backend::winget::package_roots(),
    )
}

/// `unprotected_winget_changes`' tested seam, against roots the caller
/// supplies. The same split, and for the same reason, as
/// `sample_fence` / `sample_fence_with_roots`: production reads the environment
/// once, and a test can hand this one a directory it built.
pub fn unprotected_winget_changes_with_roots(
    plan: &Plan,
    guard: &std::collections::BTreeMap<Name, Vec<String>>,
    installed: &[Installed],
    roots: &[PathBuf],
) -> Vec<String> {
    let mut out = Vec::new();
    let mut said = std::collections::BTreeSet::new();

    // Read the roots once. The directories cannot change while a plan is being
    // described, and asking per action made this O(actions) `read_dir` calls.
    // The matching rule stays in `backend::winget::segment_names_id`, which
    // `running_ids` uses too -- hoisting the I/O must not put a second copy of
    // that comparison here.
    let dir_segments = crate::backend::winget::package_dir_segments(roots);

    for action in &plan.actions {
        // Only the shapes that can actually replace or remove a live
        // installation, and the list is shorter than "everything that acts" by
        // two measured exclusions rather than by taste:
        //
        // - `Install` has nothing installed to be running.
        // - `Downgrade` reaches `execute` and fires `winget install --version`,
        //   but that command only ever moves a package *up*: it returns
        //   `NO_AVAILABLE_UPGRADE`, the step ends `touched: false`, and
        //   `render`'s summary counts a winget downgrade separately from
        //   `change_count` for exactly that reason. Warning about it would
        //   print a sentence dotpkg has been measured unable to carry out.
        //
        // Matching on the variant rather than on a "does it change something"
        // helper is deliberate: both exclusions differ from `Prune`/`Upgrade`
        // by what reaches the disk, not by whether an action exists.
        let (name, verb) = match action {
            Action::Upgrade { backend, name, .. } if backend == WINGET => (name, "upgrade"),
            Action::Prune { backend, name, .. } if backend == WINGET => (name, "remove"),
            _ => continue,
        };

        // The user named the process themselves: that is the whole protection
        // this warning asks for, so asking twice would be noise.
        if guard.contains_key(name) {
            continue;
        }
        // The path half can fire for this package, so it is covered by the
        // signal that needs no declaration at all.
        if dir_segments
            .iter()
            .any(|seg| crate::backend::winget::segment_names_id(seg, name.key()))
        {
            continue;
        }
        if !said.insert(name.clone()) {
            continue;
        }

        // Read out of the scan row rather than re-derived, for the reason
        // `guard_for`'s own doc comment gives: the display name is only
        // available at scan time, and re-deriving here would silently drop the
        // guess that catches Google Chrome.
        let guesses = guard_for(name, installed);
        let guesses = if guesses.is_empty() {
            "none at all".to_string()
        } else {
            guesses
                .iter()
                .map(|g| format!("{g:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        };

        out.push(format!(
            "pkg.toml [winget.guard] {name}: winget created no package directory for this id, \
             so dotpkg cannot recognise its processes by path, and the only names it will \
             match are guesses ({guesses}). If it runs under any other name, add \
             \"{name}\" = [\"<process name>\"] under [winget.guard] -- otherwise dotpkg \
             may {verb} it \
             while it is running"
        ));
    }
    out
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
    let mut winget_scan = crate::backend::scan_or_warn(&winget);
    // Before anything reads `winget_scan`: `main.rs` clones its `installed` into
    // the list `plan()` and `plan_to_steps` both see, so a guard name merged
    // here reaches the plan-time fence and the mid-run re-sampler alike. See
    // `apply_guard_overrides`' own doc comment for that chain.
    for w in &crate::backend::apply_guard_overrides(
        &mut winget_scan,
        &declared.winget.guard,
        &declared.winget.packages,
    ) {
        eprintln!("warning: {w}");
    }
    let procs = crate::sys::running_processes();
    let running = sample_fence(&scoop, &winget_scan, &procs);
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
///
/// Held removals come back with their **backend**, taken off the `Step` itself
/// so it cannot disagree with what would have run: `main.rs` pushes each into
/// `Execution::results`, and that table names a backend per line. A held
/// *winget* removal became reachable the moment Phase 4b Task 13 let `plan_to_steps`
/// emit `WingetStep::Remove`.
pub fn gate_removals(steps: Vec<Step>, preparation_ok: bool) -> (Vec<Step>, Vec<(String, Name)>) {
    if preparation_ok {
        return (steps, Vec::new());
    }
    let mut kept = Vec::with_capacity(steps.len());
    let mut held = Vec::new();
    for step in steps {
        if step.is_remove() {
            held.push((step.backend().to_string(), step.app().clone()));
        } else {
            kept.push(step);
        }
    }
    (kept, held)
}

/// Refuse a run that would attempt a winget removal this crate has *measured*
/// it cannot perform.
///
/// Measured on a14 (`docs/measurements-2026-08-10-winget-write-path.md` §5):
/// `winget install` of a user-scope package succeeds from an elevated session,
/// and `winget uninstall` of that same package is then refused with
/// `0x8A15007D` -- `The package installed for user scope cannot be uninstalled
/// when running with administrator privileges.` -- repeatably, `--all-versions`
/// included. The paired positive control, same machine and same argv at medium
/// integrity, exited `0` and removed it. dotpkg's whole shape is a scheduled
/// `apply`, so an elevated run can install a package and then be
/// *structurally* unable to remove it: every prune failing forever, not
/// transiently.
///
/// Called after `gate_removals` and before `execute`, so a refusal happens
/// before the recovery file is written and before one single step runs -- the
/// same "fail closed at the point of use" shape as
/// `execute::root_looks_like_scoop`.
///
/// **`elevated` is a parameter, not a `sys::elevated()` call in here.** That
/// function reads the real process token, so a test's verdict would otherwise
/// depend on how `cargo test` itself was launched -- the non-discriminating
/// shape Phase 4's `resolve_root` test had. `is_user_scope` is injected for the
/// same reason, and because it is a `winget list` subprocess.
///
/// **`None` must not refuse.** `sys::elevated()` returns it for "could not
/// tell", and unconditionally on every non-Windows target. A machine whose
/// token query failed is a machine dotpkg knows nothing about, and refusing
/// every winget removal there would be a refusal caused by a missing answer
/// rather than by a measured hazard. `run_winget_step` still translates
/// `CANNOT_UNINSTALL_ELEVATED` into a named failure if it happens anyway: a
/// pre-check plus a translation, not either alone, because the pre-check
/// cannot be perfect.
///
/// **Narrow on purpose: user-scope packages only.** Whether a *machine*-scope
/// package can be removed while elevated is **unmeasured** -- §5's trio is a
/// user-scope package throughout -- so refusing on elevation alone would
/// invent a refusal, and would break the removal an elevated scheduled `apply`
/// is most likely to be for. The scope query's own basis is measured in both
/// directions (§15: 19 ids exit `0` under `--scope user` and non-zero under
/// `--scope machine`; `Microsoft.VisualStudio.2022.BuildTools` does the
/// reverse).
///
/// **Cost: `is_user_scope` is asked once per winget removal, and only on an
/// elevated run.** Each ask is a ~1 s `winget list` subprocess
/// (`docs/measurements-2026-08-09-winget.md`), so the two cheap answers come
/// first: a run that is not elevated (or cannot tell) returns before asking
/// anything at all, and no scoop step and no `WingetStep::Set` is ever asked
/// about. Every winget removal in an elevated run *is* asked, without
/// short-circuiting on the first hit -- to let the run through, every one of
/// them has to be asked anyway, so stopping early would only ever buy time on
/// a run that is about to refuse, at the price of naming one package when the
/// operator needs the whole list to know what de-elevating will fix.
///
/// **The refusal says "no package was installed, upgraded or removed", not
/// "nothing has been changed".** Post-merge audit Minor 5: by the time this
/// runs, `main.rs` has already git-cloned any missing declared bucket (if
/// `--clone-missing-buckets` was passed) and run `scoop download` for every
/// scoop step, populating scoop's cache -- both write to the machine outside
/// dotpkg's staging root. Narrowed rather than moved earlier: moving this
/// check ahead of that work is a *behaviour* change (this function only knows
/// which removals are affected from `steps`, which does not exist until
/// after staging has already happened), where narrowing the sentence only
/// asks it to stop claiming more than it can honestly claim -- no package
/// was installed, upgraded or removed, which is true and is also this
/// sentence's meaning everywhere else it appears (`--prepare`'s identical
/// line, printed after the identical staging work).
pub fn refuse_elevated_winget_removal(
    steps: &[Step],
    elevated: Option<bool>,
    is_user_scope: &dyn Fn(&Name) -> bool,
) -> Result<(), String> {
    if elevated != Some(true) {
        return Ok(());
    }
    let blocked: Vec<String> = steps
        .iter()
        .filter_map(|s| match s {
            Step::Winget(WingetStep::Remove { id, .. }) => Some(id),
            // Wildcard-free on winget's side, so a new `WingetStep` variant
            // has to come back here and say whether it removes anything.
            // Scoop's three collapse: none of them ever goes near winget.
            Step::Winget(WingetStep::Set { .. }) | Step::Scoop(_) => None,
        })
        .filter(|id| is_user_scope(id))
        .map(|id| id.to_string())
        .collect();
    if blocked.is_empty() {
        return Ok(());
    }
    Err(format!(
        "this run is elevated, and winget refuses to uninstall a package installed for \
         user scope from an elevated process -- measured exit 0x8A15007D, \"The package \
         installed for user scope cannot be uninstalled when running with administrator \
         privileges.\" {} removal(s) in this run are affected: {}. The same removal was \
         measured to succeed from a session this check reads as not elevated, so re-run \
         `dotpkg apply` without elevation. No package was installed, upgraded or removed.",
        blocked.len(),
        blocked.join(", ")
    ))
}

/// What the driver does next, once the steps are prepared and gated.
#[derive(Debug, PartialEq, Eq)]
pub enum RunGate {
    /// Confirm (unless `--yes`) and run the steps.
    Proceed,
    /// The machine already matches the files. `main.rs` owns the sentence it
    /// prints for this, because that exact text is what a scheduled run's log
    /// is read for and `tests/cli.rs` pins it end to end.
    NothingToDo,
    /// The whole sentence a user reads. `main.rs` hands it to `refuse`, which
    /// exits 2 -- this project's exit codes mean that as "not touched".
    Refuse(String),
}

/// Everything that stands between a prepared, removal-gated step list and
/// `execute`, in the order it has to happen.
///
/// **This is here rather than inline in the `apply` arm because the ORDER is
/// the part that had no test.** Task 15's review found that the four driver
/// lines calling `refuse_elevated_winget_removal` could be moved after the
/// confirmation prompt, or after `recovery_path` was built, or handed a
/// constant `Some(false)`, and the entire suite stayed green: the tests pinned
/// the pre-check *function*, never its position or its arguments. Hoisting the
/// three checks into one function moves the ordering into code a test can
/// reach, and leaves the driver one call to get wrong instead of three
/// separately-placed ones. The same reasoning that put `gate_removals` here:
/// the decisions a deleted line would make silently permissive do not belong
/// in `main.rs`.
///
/// The order, and why it is this one:
///
/// 1. **A converged machine returns first.** Nothing to install, nothing to
///    remove, nothing held, nothing unusable -- asking "0 installed, 0
///    removed, continue?" has no meaningful answer, and an unreadable stdin
///    would refuse it anyway: exit 2, "go look", every night, about nothing.
///    All three of `steps`, `unusable` and `held` must be empty, because a
///    held prune is outstanding work whose closing-table row is the only place
///    the user learns about it.
/// 2. **Then the `--allow-prune` gate**, which reads the step list and nothing
///    else.
/// 3. **Then the elevation pre-check**, last because it is the only one that
///    spawns subprocesses -- one `winget list` per winget removal, ~1 s each
///    (see `refuse_elevated_winget_removal`'s own cost note). A guard that
///    needs nothing must not queue behind one that needs a subprocess; the
///    same reasoning already puts `mass_prune_guard` and
///    `lock_coherence_guard` ahead of the machine scan entirely.
///
/// Every one of the three is still ahead of the confirmation prompt, ahead of
/// the recovery file being written, and ahead of every mutation. That, not the
/// relative order, is the property a refusal depends on -- and it is why this
/// function returns a decision instead of doing anything itself.
///
/// `elevated` and `is_user_scope` are passed straight through to
/// `refuse_elevated_winget_removal`; see that function for why neither may be
/// resolved in here.
pub fn gate_the_run(
    steps: &[Step],
    unusable: &[(Name, String)],
    held: &[(String, Name)],
    yes: bool,
    allow_prune: bool,
    elevated: Option<bool>,
    is_user_scope: &dyn Fn(&Name) -> bool,
) -> RunGate {
    if steps.is_empty() && unusable.is_empty() && held.is_empty() {
        return RunGate::NothingToDo;
    }

    let removals = steps.iter().filter(|s| s.is_remove()).count();
    if removals > 0 && yes && !allow_prune {
        return RunGate::Refuse(format!(
            "this run would remove {removals} package(s) and --yes was passed. \
             Removals need --allow-prune as well."
        ));
    }

    if let Err(why) = refuse_elevated_winget_removal(steps, elevated, is_user_scope) {
        return RunGate::Refuse(why);
    }

    RunGate::Proceed
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

    /// A `WingetCmd` that answers every `show` with one canned `CmdOut`, and
    /// records the argv it was asked with.
    ///
    /// The whole seam `prepare` now takes, faked: no test may spawn
    /// `winget.exe`, and none of these has one to spawn anyway.
    struct FakeWinget {
        out: std::cell::RefCell<Vec<Result<crate::backend::winget::CmdOut, ()>>>,
        calls: std::cell::RefCell<Vec<Vec<String>>>,
    }

    impl FakeWinget {
        /// `show -v` succeeds: the pinned version is still in the index.
        fn live(version: &str) -> FakeWinget {
            FakeWinget {
                out: std::cell::RefCell::new(vec![Ok(crate::backend::winget::CmdOut {
                    code: 0,
                    stdout: format!("Found Brave Browser [Brave.Brave]\r\nVersion: {version}\r\n"),
                })]),
                calls: std::cell::RefCell::new(Vec::new()),
            }
        }
        /// `show -v` succeeds and echoes back a canonical id that differs from
        /// the spelling it was asked with -- the ordinary supported state, not a
        /// typo: `update` warns about the mismatch and says "pkg.toml is left as
        /// you wrote it", and `adopt` deliberately writes the user's spelling.
        /// `-v` is answered without `--exact`, so winget folds case on the way
        /// in and hands the canonical spelling back in the `Found <name>
        /// [<Id>]` line -- which is the only place that spelling exists.
        fn live_echoing(canonical: &str, version: &str) -> FakeWinget {
            FakeWinget {
                out: std::cell::RefCell::new(vec![Ok(crate::backend::winget::CmdOut {
                    code: 0,
                    stdout: format!(
                        "Found Some Display Name [{canonical}]\r\nVersion: {version}\r\n"
                    ),
                })]),
                calls: std::cell::RefCell::new(Vec::new()),
            }
        }
        /// `show -v` says this exact version has fallen out of the index, and
        /// the follow-up `--versions` call cannot be answered either.
        fn version_gone() -> FakeWinget {
            FakeWinget {
                out: std::cell::RefCell::new(vec![
                    Ok(crate::backend::winget::CmdOut {
                        code: crate::backend::winget::NO_VERSION_FOUND,
                        stdout: "No package found matching input criteria.\r\n".into(),
                    }),
                    Ok(crate::backend::winget::CmdOut {
                        code: 1,
                        stdout: String::new(),
                    }),
                ]),
                calls: std::cell::RefCell::new(Vec::new()),
            }
        }
        /// There is no `winget.exe` on this machine at all.
        fn absent() -> FakeWinget {
            FakeWinget {
                out: std::cell::RefCell::new(vec![Err(())]),
                calls: std::cell::RefCell::new(Vec::new()),
            }
        }
        /// Refuses every call: proof that a code path never asked winget
        /// anything.
        fn never_called() -> FakeWinget {
            FakeWinget {
                out: std::cell::RefCell::new(Vec::new()),
                calls: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl crate::backend::winget::WingetCmd for FakeWinget {
        fn run(
            &self,
            args: &[&str],
        ) -> Result<crate::backend::winget::CmdOut, crate::backend::winget::CmdError> {
            self.calls
                .borrow_mut()
                .push(args.iter().map(|a| a.to_string()).collect());
            let mut out = self.out.borrow_mut();
            assert!(
                !out.is_empty(),
                "winget was asked {args:?} and this fake has no answer left for it"
            );
            match out.remove(0) {
                Ok(o) => Ok(o),
                Err(()) => Err(crate::backend::winget::CmdError::NotFound),
            }
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
    fn a_winget_version_change_needs_a_liveness_check_not_an_artifact() {
        // The one place the two backends' preparations part company, and the
        // reason it is a `classify` decision rather than a `stage_and_fetch`
        // one: winget has no local manifest, so asking `Scoop::stage` for one
        // is not "not implemented yet", it is meaningless. Before Phase 4b Task 13 this
        // never came up -- a winget difference was a `Skip`.
        for a in [
            Action::Install {
                backend: WINGET.into(),
                name: Name::new("Brave.Brave"),
                version: "151.1.93.134".into(),
                arch: None,
            },
            Action::Upgrade {
                backend: WINGET.into(),
                name: Name::new("Brave.Brave"),
                from: "151.1.93.132".into(),
                to: "151.1.93.134".into(),
                arch: None,
            },
            Action::Downgrade {
                backend: WINGET.into(),
                name: Name::new("Brave.Brave"),
                from: "151.1.93.134".into(),
                to: "151.1.93.132".into(),
                arch: None,
            },
        ] {
            assert!(matches!(classify(&a), Intent::NeedsLiveness), "{a:?}");
        }
        // A winget prune is still ready by definition, like scoop's: there is
        // nothing to check the liveness of, because nothing is being fetched
        // OR installed.
        assert!(matches!(
            classify(&Action::Prune {
                backend: WINGET.into(),
                name: Name::new("Brave.Brave"),
                version: "151.1.93.134".into(),
            }),
            Intent::NoArtifactNeeded
        ));
    }

    #[test]
    fn a_declared_unlocked_winget_package_now_fails_the_run_exactly_as_a_scoop_one_does() {
        // **This reverses a test, on purpose.** Until Phase 4b Task 13 a declared,
        // unlocked winget package was `SkipReason::ReportedOnly(Divergence::
        // NotLocked)` and classified to `Intent::Skip`, deliberately: `apply`
        // could not have acted on it even *with* a pin, so failing the whole
        // run -- every unrelated scoop action included -- over a lock entry
        // that did not exist helped nobody.
        //
        // That reasoning was entirely about winget not having an executor.
        // It has one now, so the scoop rule applies unchanged: resolving a
        // version is `update`'s job and not `apply`'s, `dotpkg update` really
        // does resolve winget (Task 15), and a run that silently installed
        // nothing for a package the user declared would be the "degrade
        // silently" failure the spec forbids. The planner emits plain
        // `SkipReason::NotLocked` for it now, and this is what that becomes.
        assert!(matches!(
            classify(&Action::Skip {
                backend: WINGET.into(),
                name: Name::new("Git.Git"),
                reason: SkipReason::NotLocked,
            }),
            Intent::NotLocked
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

        let prep = prepare(
            &plan,
            &lock,
            &scoop,
            &scoop,
            &FakeWinget::never_called(),
            stage_dir.path(),
            &declared,
        );
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

        let prep = prepare(
            &plan,
            &lock,
            &scoop,
            &scoop,
            &FakeWinget::never_called(),
            stage_dir.path(),
            &declared,
        );
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

        let prep = prepare(
            &plan,
            &lock,
            &scoop,
            &scoop,
            &FakeWinget::never_called(),
            stage_dir.path(),
            &declared,
        );
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
    fn prepare_refuses_to_stage_a_backend_it_has_no_preparation_for() {
        // This test used `WINGET` as its unpreparable backend until Phase 4b Task 13,
        // when winget got a preparation of its own (`Intent::NeedsLiveness`)
        // and stopped being one. The guard it pins is unchanged and still
        // needed, so the backend is now a third one that does not exist: a
        // backend added to `Action` without a `classify` branch must be
        // reported per package, not handed to `Scoop::stage`, which would look
        // its name up in a scoop bucket and fail with a sentence about
        // buckets.
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let lock = Lock::default();
        let declared = Config::default();

        let plan = Plan {
            actions: vec![Action::Install {
                backend: "chocolatey".into(),
                name: Name::new("git"),
                version: "2.55.0".into(),
                arch: None,
            }],
        };

        let prep = prepare(
            &plan,
            &lock,
            &scoop,
            &scoop,
            // And it must not reach for winget either: an unknown backend is
            // not winget's problem to answer for.
            &FakeWinget::never_called(),
            stage_dir.path(),
            &declared,
        );
        let Outcome::Failed { why } = &prep.prepared[0].outcome else {
            panic!(
                "expected a Failed outcome, got {:?}",
                prep.prepared[0].outcome
            );
        };
        assert!(why.contains("chocolatey"), "name the backend: {why}");
        assert!(
            !why.contains("bucket"),
            "the diagnosis must be the missing preparation, not a scoop \
             bucket lookup it should never have attempted: {why}"
        );
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
        // Prophylactic, not observed failing here: this test asserts no disk
        // snapshot, so it is not currently exposed. See `tests/cli.rs`'s
        // `write_lock_and_bucket_for` for the measured case where the same
        // unmanaged temp repo shape aborted a `cargo mutants` run.
        //
        // THE ONE SITE THAT CANNOT USE `common::init_repo`, and it is written
        // out by hand for a structural reason rather than an oversight: that
        // constructor lives in `tests/common/mod.rs`, which is compiled into
        // the integration-test binaries, and this test lives inside the
        // library crate, which cannot depend on them. Every one of the other
        // five temp-repo sites in this workspace goes through the constructor,
        // so this is the only place the three lines can drift apart -- if a
        // sixth site ever appears in `src/`, it belongs next to this one and
        // needs the same two `config` calls.
        git_output(&bucket_dir, &["config", "gc.auto", "0"]);
        git_output(&bucket_dir, &["config", "maintenance.auto", "0"]);
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

        let prep = prepare(
            &plan,
            &lock,
            &scoop,
            &scoop,
            &FakeWinget::never_called(),
            stage_dir.path(),
            &declared,
        );
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

        let prep = prepare(
            &plan,
            &lock,
            &scoop,
            &scoop,
            &FakeWinget::never_called(),
            stage_dir.path(),
            &declared,
        );
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

        let prep = prepare(
            &plan,
            &lock,
            &scoop,
            &scoop,
            &FakeWinget::never_called(),
            stage_dir.path(),
            &declared,
        );
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
    fn a_declared_unlocked_winget_package_now_fails_the_whole_preparation_like_a_scoop_one() {
        // **This reverses a test, on purpose**, and its old name said so:
        // `a_declared_unlocked_winget_package_does_not_fail_the_whole_
        // preparation`. That was correct while winget only reported -- `apply`
        // could not have installed the package even with a pin, so refusing
        // the run over a lock entry that did not exist punished every
        // unrelated scoop action for nothing. Winget has an executor now, so
        // the reason is gone and the scoop rule applies: `apply` may not
        // resolve a version itself, `dotpkg update` can (Task 15 taught it
        // winget), and the run must say so rather than quietly installing
        // nothing.
        //
        // The scoop half stays as the symmetry check it always was -- except
        // that it now asserts the two backends AGREE, where before it asserted
        // they differed.
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let lock = Lock::default();
        let declared = Config::default();
        for backend in [WINGET, SCOOP] {
            let plan = Plan {
                actions: vec![Action::Skip {
                    backend: backend.into(),
                    name: Name::new("Git.Git"),
                    reason: SkipReason::NotLocked,
                }],
            };
            let prep = prepare(
                &plan,
                &lock,
                &scoop,
                &scoop,
                // Nothing is asked of winget: a package with no pin has no
                // version whose liveness could be checked.
                &FakeWinget::never_called(),
                stage_dir.path(),
                &declared,
            );
            assert_eq!(prep.not_locked_count(), 1, "{backend}");
            assert!(!prep.is_ok(), "{backend}: this must fail the run");
            assert_eq!(prep.skipped_count(), 0, "{backend}: not a benign skip");
            assert!(
                matches!(prep.prepared[0].outcome, Outcome::NotLocked),
                "{backend}: got {:?}",
                prep.prepared[0].outcome
            );
        }
    }

    #[test]
    fn a_locked_winget_install_becomes_ready_to_set_and_asks_winget_for_exactly_one_liveness_check()
    {
        // The whole new preparation path, end to end at the `prepare()` level:
        // no manifest is staged (winget has none), the pinned version is
        // confirmed live, and the outcome carries that version forward so
        // `plan_to_steps` does not have to re-derive it.
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let plan = Plan {
            actions: vec![Action::Install {
                backend: WINGET.into(),
                name: Name::new("Brave.Brave"),
                version: "151.1.93.134".into(),
                arch: None,
            }],
        };
        let winget = FakeWinget::live("151.1.93.134");
        let prep = prepare(
            &plan,
            &Lock::default(),
            &scoop,
            &scoop,
            &winget,
            stage_dir.path(),
            &Config::default(),
        );
        assert_eq!(
            prep.prepared[0].outcome,
            Outcome::ReadyToSet {
                // `FakeWinget::live` echoes `Found Brave Browser [Brave.Brave]`,
                // and this is that echo -- the canonical spelling, read off
                // winget's own answer rather than copied from the action.
                id: Name::new("Brave.Brave"),
                version: "151.1.93.134".into()
            }
        );
        assert_eq!(prep.ready_count(), 1, "ReadyToSet is a ready shape");
        assert!(prep.is_ok());
        // The argv, pinned: `--exact` is deliberately absent (see
        // `version_liveness`), and the version asked about is the one the PLAN
        // resolved -- the thing the user was shown -- not a second read of the
        // lock, which is empty here precisely so a lock read could not pass.
        let calls = winget.calls.borrow();
        assert_eq!(
            *calls,
            vec![vec![
                "show".to_string(),
                "--id".into(),
                "Brave.Brave".into(),
                "-v".into(),
                "151.1.93.134".into(),
                "--disable-interactivity".into(),
            ]],
            "exactly one `show`, with this argv"
        );
    }

    #[test]
    fn the_winget_write_argv_carries_the_canonical_id_not_the_declared_spelling() {
        // **The one place the two spellings must not be confused.** `pkg.toml`
        // holds whatever the user typed -- a supported state, not a typo the
        // tool rejects: `update` warns about a case mismatch and says "pkg.toml
        // is left as you wrote it", and `adopt` deliberately writes the user's
        // spelling. `plan_backend` builds the action from that declared string,
        // so the action's `name` here is `git.git`.
        //
        // Measured (`docs/measurements-2026-08-10-winget-write-path.md` §6):
        // `install -e --id SHARKDP.HYPERFINE --version <x>` returns 0x8A150014
        // "No package found matching input criteria." where the correctly-cased
        // call reaches 0x8A150017 -- `--exact` is what makes `--id`
        // case-sensitive, on the write verbs too. So a `set_argv` built from the
        // declared spelling can NEVER install the package, and the rescan
        // (`list -e --id <same wrong spelling>`) misses too, so the user is told
        // the package does not exist. Every run, forever.
        //
        // `version_liveness` omits `--exact` deliberately and therefore already
        // has the answer: winget echoes the canonical id back in `Found <name>
        // [<Id>]`. That echo is what must reach the wire -- while the plan the
        // user reads keeps their own spelling, which is the deliberate choice.
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(stage_dir.path().to_path_buf());
        let plan = Plan {
            actions: vec![Action::Upgrade {
                backend: WINGET.into(),
                name: Name::new("git.git"),
                from: "2.51.0".into(),
                to: "2.52.0".into(),
                arch: None,
            }],
        };
        let winget = FakeWinget::live_echoing("Git.Git", "2.52.0");
        let prep = prepare(
            &plan,
            &Lock::default(),
            &scoop,
            &scoop,
            &winget,
            stage_dir.path(),
            &Config::default(),
        );
        let (steps, unusable) = plan_to_steps(&prep, &[]);
        assert!(unusable.is_empty(), "{unusable:?}");
        assert_eq!(steps.len(), 1, "one Upgrade, one Set: {steps:?}");

        // Asserted through `set_argv` rather than against the step's `id`
        // field, because the argv is the thing winget actually receives and
        // `set_argv` is what puts `-e` beside it.
        let Step::Winget(WingetStep::Set { id, version, .. }) = &steps[0] else {
            panic!("expected a winget Set: {:?}", steps[0]);
        };
        let argv = crate::backend::winget_exec::set_argv(id, version);
        assert!(
            argv.contains(&"Git.Git".to_string()),
            "the canonical spelling winget itself echoed back must be on the wire: {argv:?}"
        );
        assert!(
            !argv.contains(&"git.git".to_string()),
            "the declared spelling must NOT be on the wire beside `-e`: {argv:?}"
        );
        // And the plan's own display is untouched: `pkg.toml` and the line the
        // user reads keep the spelling they wrote.
        assert_eq!(
            prep.prepared[0].action, plan.actions[0],
            "preparation must not rewrite the action the user was shown"
        );
    }

    #[test]
    fn a_winget_pin_that_fell_out_of_the_index_is_a_failed_preparation_not_a_ready_one() {
        // The negative control for the test above: without it, a
        // `check_pin_is_live` that ignored winget's answer entirely would pass
        // it. A pin winget can no longer serve is exactly the case `--prepare`
        // exists to find before anything is installed.
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let plan = Plan {
            actions: vec![Action::Upgrade {
                backend: WINGET.into(),
                name: Name::new("Brave.Brave"),
                from: "151.1.93.132".into(),
                to: "151.1.93.134".into(),
                arch: None,
            }],
        };
        let prep = prepare(
            &plan,
            &Lock::default(),
            &scoop,
            &scoop,
            &FakeWinget::version_gone(),
            stage_dir.path(),
            &Config::default(),
        );
        let Outcome::Failed { why } = &prep.prepared[0].outcome else {
            panic!("expected Failed, got {:?}", prep.prepared[0].outcome);
        };
        assert!(
            why.contains("151.1.93.134") && why.contains("no longer in the winget index"),
            "name the version that is gone, not just that something failed: {why}"
        );
        assert!(!prep.is_ok(), "the run must refuse by default");
        assert_eq!(prep.ready_count(), 0);
    }

    #[test]
    fn a_refused_winget_downgrade_is_ready_to_set_but_counted_separately() {
        // I2 (post-merge audit): a winget package installed ahead of its pin
        // reaches `check_pin_is_live` exactly like an install or an upgrade
        // does -- the pin really is live in winget's index, so the outcome
        // really is `ReadyToSet`, and `plan_to_steps` really does build a
        // `WingetStep::Set` and fire it. dotpkg does not decide the
        // direction; winget's own refusal is the gate (`Plan::change_count`'s
        // own doc comment). `refused_winget_downgrade_count` is what a
        // caller must consult before calling this "ready" to a user --
        // `ready_count` itself still counts it, deliberately (see that
        // method's own doc comment).
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let plan = Plan {
            actions: vec![Action::Downgrade {
                backend: WINGET.into(),
                name: Name::new("Brave.Brave"),
                from: "151.1.93.134".into(),
                to: "151.1.93.132".into(),
                arch: None,
            }],
        };
        let prep = prepare(
            &plan,
            &Lock::default(),
            &scoop,
            &scoop,
            &FakeWinget::live("151.1.93.132"),
            stage_dir.path(),
            &Config::default(),
        );
        assert!(
            matches!(prep.prepared[0].outcome, Outcome::ReadyToSet { .. }),
            "the pin really is live -- the liveness check cannot know this \
             is a downgrade, and must not guess: {:?}",
            prep.prepared[0].outcome
        );
        assert_eq!(
            prep.ready_count(),
            1,
            "the invariant with plan_to_steps holds"
        );
        assert_eq!(
            prep.refused_winget_downgrade_count(),
            1,
            "but the user-facing count must know it is a refusal"
        );
        assert!(
            prep.is_ok(),
            "a refused downgrade must not block the rest of the run -- that \
             would be a behaviour change, not a reporting fix"
        );

        // The counterweight: a genuine winget install or upgrade must not be
        // caught by this count.
        let genuine_plan = Plan {
            actions: vec![Action::Upgrade {
                backend: WINGET.into(),
                name: Name::new("Brave.Brave"),
                from: "151.1.93.132".into(),
                to: "151.1.93.134".into(),
                arch: None,
            }],
        };
        let genuine = prepare(
            &genuine_plan,
            &Lock::default(),
            &scoop,
            &scoop,
            &FakeWinget::live("151.1.93.134"),
            stage_dir.path(),
            &Config::default(),
        );
        assert_eq!(
            genuine.refused_winget_downgrade_count(),
            0,
            "an upgrade must never be counted as a refused downgrade"
        );
    }

    #[test]
    fn a_machine_with_no_winget_exe_fails_preparation_instead_of_installing_into_thin_air() {
        // The one hole the capability flip could have opened, closed here.
        // `Winget::scan` answers `CmdError::NotFound` with an empty `Scan`
        // plus a warning, because a machine with no winget is legitimate --
        // which means "winget is not installed" and "winget found nothing" are
        // spelled the same way in `installed`, so a declared AND LOCKED winget
        // package on such a machine plans as an `Install`. While winget only
        // reported, that `Install` was a report line; now it would be a real
        // `winget install`.
        //
        // It never gets there: preparation asks `winget show` first, so the
        // missing binary is found at prepare time, reported per package, and
        // the package produces no step at all.
        let root = tempfile::tempdir().unwrap();
        let stage_dir = tempfile::tempdir().unwrap();
        let scoop = Scoop::new(root.path().to_path_buf());
        let plan = Plan {
            actions: vec![Action::Install {
                backend: WINGET.into(),
                name: Name::new("BurntSushi.ripgrep.MSVC"),
                version: "15.2.0".into(),
                arch: None,
            }],
        };
        let prep = prepare(
            &plan,
            &Lock::default(),
            &scoop,
            &scoop,
            &FakeWinget::absent(),
            stage_dir.path(),
            &Config::default(),
        );
        let Outcome::Failed { why } = &prep.prepared[0].outcome else {
            panic!(
                "an absent winget.exe must not be ready: {:?}",
                prep.prepared[0].outcome
            );
        };
        assert!(
            why.contains("not on PATH"),
            "say what is actually wrong with the machine: {why}"
        );
        assert!(
            !prep.is_ok(),
            "by default the whole run refuses before execute is reached"
        );
        let (steps, unusable) = plan_to_steps(&prep, &[]);
        assert!(
            steps.is_empty(),
            "no step may be built against a binary that does not exist: {steps:?}"
        );
        assert_eq!(unusable.len(), 1, "and it is reported, not dropped");
    }

    #[test]
    fn is_outstanding_floors_running_and_opaque_and_unscannable_but_not_not_locked() {
        // `is_outstanding` is private, so this calls it directly rather than
        // routing through `prepare()` -- deliberately, for `NotLocked`: that
        // is the one `SkipReason` left in the `false` arm, and it can no
        // longer be exercised through `outstanding_skips()` at all, because
        // `classify` never turns it into `Outcome::Skipped` in the first
        // place (it becomes `Intent::NotLocked` -> `Outcome::NotLocked`,
        // which fails `is_ok()` on its own). Two variants have played the
        // "Skipped but not outstanding" role and could be shown failing to
        // float through `prepare()`'s real pipeline -- `BackendNotImplemented`
        // (deleted by Phase 4 Task 14) and `ReportedOnly` (deleted by Phase 4b Task 13, when
        // winget got an executor); nothing has taken their place, so this is
        // now the only way to pin the `false` arm at all.
        assert!(is_outstanding(&SkipReason::Running));
        assert!(is_outstanding(&SkipReason::Opaque));
        // Task 6's addition: a scan failure floors the same way, and for the
        // same reason -- outstanding work the user asked for and did not
        // get, that could differ on the next run. Nothing in this suite
        // would notice `SkipReason::Unscannable => true` flipping to `false`
        // without this line: the two planner tests Task 6 added only inspect
        // `Plan`, never `prepare`/`outstanding_skips`.
        assert!(is_outstanding(&SkipReason::Unscannable));
        assert!(
            !is_outstanding(&SkipReason::NotLocked),
            "permanent and structural for THIS run: apply cannot resolve a \
             version itself, and the run already fails on it via NotLocked, \
             not via a float"
        );
    }

    #[test]
    fn outstanding_skips_finds_running_opaque_and_unscannable_skips_together() {
        // The end-to-end proof that every outstanding `SkipReason` really does
        // carry through the whole pipeline -- `prepare()` -> `classify()` ->
        // `Outcome::Skipped` -> `outstanding_skips()` -- not just that
        // `is_outstanding` says so in isolation (see the direct unit test
        // above).
        //
        // Three, not four: `ReportedOnly` was the fourth until Phase 4b Task 13 deleted
        // it along with the rest of winget's report-only path. The count lives
        // in this comment and in the assertion below rather than only in the
        // test's name, because a stale count in prose is exactly the failure
        // class this project exists to catch -- this test's own history has it
        // twice now, once when `Unscannable` was added and once here.
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
                    name: Name::new("Discord.Discord"),
                    reason: SkipReason::Unscannable,
                },
            ],
        };

        let prep = prepare(
            &plan,
            &lock,
            &scoop,
            &scoop,
            &FakeWinget::never_called(),
            stage_dir.path(),
            &declared,
        );
        assert_eq!(
            prep.skipped_count(),
            3,
            "the positive control: all three really are Skipped outcomes"
        );

        let outstanding = prep.outstanding_skips();
        assert_eq!(
            outstanding,
            vec![
                (
                    SCOOP.to_string(),
                    Name::new("kanata"),
                    "running -- stop it first".to_string()
                ),
                (
                    SCOOP.to_string(),
                    Name::new("zellij"),
                    "installed, but its state could not be read -- see the warnings above"
                        .to_string()
                ),
                // The backend is asserted, not just the name and the reason:
                // `main.rs` pushes these into `Execution::results`, whose
                // closing table names a backend per line, and it printed
                // "scoop" for this winget package until Phase 4b Task 13 -- see
                // `execute::ItemOutcome`.
                (
                    WINGET.to_string(),
                    Name::new("Discord.Discord"),
                    "this backend could not be scanned -- see the warnings above; nothing \
                     was attempted for it"
                        .to_string()
                ),
            ],
            "all three must float -- none of them is permanent and structural"
        );
    }

    #[test]
    fn outstanding_skips_excludes_a_not_locked_skip_sitting_beside_an_outstanding_one() {
        // `is_outstanding` returns `false` for exactly one `SkipReason`:
        // `NotLocked`. In real `prepare()` output that reason never reaches
        // `Outcome::Skipped` -- `classify` routes it to `Outcome::NotLocked`
        // instead (see `is_outstanding`'s doc comment) -- so, like
        // `could_not_be_prepared_counts_failures_not_every_package_that_
        // became_no_step` below, this builds the `Preparation` directly
        // rather than through `prepare()`, to put a `Skipped`-with-
        // `NotLocked` entry in front of `outstanding_skips` at all.
        //
        // Under `replace match guard is_outstanding(reason) with true`, this
        // entry would come back too. Asserted by identity, not merely by
        // count, so a mutant that returned the wrong package (or both) would
        // not slip past a bare `len() == 1`.
        let prep = Preparation {
            prepared: vec![
                Prepared {
                    action: Action::Skip {
                        backend: SCOOP.into(),
                        name: Name::new("kanata"),
                        reason: SkipReason::Running,
                    },
                    outcome: Outcome::Skipped {
                        why: "running -- stop it first".into(),
                    },
                },
                Prepared {
                    action: Action::Skip {
                        backend: SCOOP.into(),
                        name: Name::new("zellij"),
                        reason: SkipReason::NotLocked,
                    },
                    outcome: Outcome::Skipped {
                        why: "not locked".into(),
                    },
                },
            ],
        };
        assert_eq!(
            prep.outstanding_skips(),
            vec![(
                SCOOP.to_string(),
                Name::new("kanata"),
                "running -- stop it first".to_string()
            )],
            "the not-locked skip must not come back, even sitting right beside one that does"
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

        let prep = prepare(
            &plan,
            &lock,
            &scoop,
            &scoop,
            &FakeWinget::never_called(),
            stage_dir.path(),
            &declared,
        );
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
    // `ScoopStep::Remove` kept every existing test green.

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

        let (steps, unusable) = plan_to_steps(&prep, &[]);
        assert!(unusable.is_empty(), "{unusable:?}");
        assert_eq!(
            steps,
            vec![
                Step::Scoop(ScoopStep::Install {
                    app: Name::new("fzf"),
                    staged: PathBuf::from("/stage/fzf/1.0.0/fzf.json"),
                    arch: None,
                }),
                Step::Scoop(ScoopStep::Replace {
                    app: Name::new("bat"),
                    staged: PathBuf::from("/stage/bat/2/bat.json"),
                    arch: Some("arm64".into()),
                }),
                Step::Scoop(ScoopStep::Replace {
                    app: Name::new("ripgrep"),
                    staged: PathBuf::from("/stage/ripgrep/1/ripgrep.json"),
                    arch: None,
                }),
                Step::Scoop(ScoopStep::Remove {
                    app: Name::new("aichat"),
                }),
            ]
        );
    }

    #[test]
    fn an_install_action_carrying_a_stray_readytoremove_produces_no_step_but_is_reported() {
        // The invariant `plan_to_steps` exists to hold: branch on the ACTION,
        // never on the outcome alone. Nothing in the type system stops an
        // `Outcome::ReadyToRemove` from being attached to an `Install` --
        // this is exactly the pair a version that matched on the outcome by
        // itself would turn into a `ScoopStep::Remove` for a package nobody
        // asked to remove.
        //
        // Since Task 4 narrowed the trailing wildcard, this stray pairing no
        // longer disappears silently: no scoop arm above claims a
        // `ReadyToRemove` for an `Install`, so it falls into the
        // routing-bug arm and is reported, even though the backend named is
        // scoop and a real `prepare()` call could never produce this pairing.
        // `steps` staying empty is still the invariant under test; `unusable`
        // gaining an entry is the new, deliberately louder consequence of
        // "an unrouted-but-ready action is always a bug worth reporting".
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

        let (steps, unusable) = plan_to_steps(&prep, &[]);
        assert!(steps.is_empty(), "{steps:?}");
        assert_eq!(
            unusable,
            vec![(
                Name::new("fzf"),
                "scoop: prepared, but no executor claimed it -- this is a routing bug, not a \
                 package problem"
                    .to_string()
            )],
            "an unrouted-but-ready action must be loud, not silently dropped: {unusable:?}"
        );
    }

    #[test]
    fn a_prune_action_carrying_a_stray_readytofetch_produces_no_step_but_is_reported() {
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

        let (steps, unusable) = plan_to_steps(&prep, &[]);
        assert!(steps.is_empty(), "{steps:?}");
        assert_eq!(
            unusable,
            vec![(
                Name::new("aichat"),
                "scoop: prepared, but no executor claimed it -- this is a routing bug, not a \
                 package problem"
                    .to_string()
            )],
            "an unrouted-but-ready action must be loud, not silently dropped: {unusable:?}"
        );
    }

    #[test]
    fn could_not_be_prepared_counts_failures_not_every_package_that_became_no_step() {
        // The number in "N package(s) could not be prepared" was
        // `plan_to_steps`'s `unusable.len()`, and this input is where the two
        // disagree: one package really could not be prepared, and two more
        // simply have a live process the user can close. `unusable` carries all
        // three, because none of them became a step; only one of them is what
        // the sentence is about.
        //
        // Both numbers are asserted, so a "fix" that made them equal again by
        // widening `unpreparable_count` would fail too.
        let prep = Preparation {
            prepared: vec![
                Prepared {
                    action: Action::Install {
                        backend: SCOOP.into(),
                        name: Name::new("neovim"),
                        version: "0.10.1".into(),
                        arch: None,
                    },
                    outcome: Outcome::Failed {
                        why: "download failed: hash mismatch".into(),
                    },
                },
                Prepared {
                    action: Action::Skip {
                        backend: SCOOP.into(),
                        name: Name::new("kanata"),
                        reason: SkipReason::Running,
                    },
                    outcome: Outcome::Skipped {
                        why: "running -- stop it first".into(),
                    },
                },
                Prepared {
                    action: Action::Skip {
                        backend: WINGET.into(),
                        name: Name::new("Brave.Brave"),
                        reason: SkipReason::Running,
                    },
                    outcome: Outcome::Skipped {
                        why: "running -- stop it first".into(),
                    },
                },
            ],
        };
        let (_, unusable) = plan_to_steps(&prep, &[]);
        assert_eq!(unusable.len(), 3, "all three became no step: {unusable:?}");
        assert_eq!(
            prep.unpreparable_count(),
            1,
            "but only one could not be PREPARED"
        );
    }

    #[test]
    fn unpreparable_count_is_zero_when_nothing_failed_and_nothing_is_unlocked() {
        // The zero side of the pin: a run made entirely of a benign skip
        // (which does not fail the run -- see `is_ok()`) must read 0, not
        // the constant `replace unpreparable_count -> usize with 1` would
        // leave behind.
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
        assert_eq!(prep.unpreparable_count(), 0);
    }

    #[test]
    fn unpreparable_count_sums_failed_and_not_locked_rather_than_returning_either_alone() {
        // One failed and two not-locked, deliberately different non-zero
        // numbers: the sum is 3, which is neither operand and not the
        // constant 1 either. A fixture built as 1-and-0 (like the test
        // above this one) would still read 1 under `replace
        // unpreparable_count -> usize with 1` and prove nothing; this one
        // cannot pass under that mutation, nor under a body that returned
        // `failed_count()` or `not_locked_count()` alone.
        let prep = Preparation {
            prepared: vec![
                Prepared {
                    action: Action::Install {
                        backend: SCOOP.into(),
                        name: Name::new("neovim"),
                        version: "0.10.1".into(),
                        arch: None,
                    },
                    outcome: Outcome::Failed {
                        why: "download failed: hash mismatch".into(),
                    },
                },
                Prepared {
                    action: Action::Skip {
                        backend: SCOOP.into(),
                        name: Name::new("kanata"),
                        reason: SkipReason::NotLocked,
                    },
                    outcome: Outcome::NotLocked,
                },
                Prepared {
                    action: Action::Skip {
                        backend: WINGET.into(),
                        name: Name::new("Discord.Discord"),
                        reason: SkipReason::NotLocked,
                    },
                    outcome: Outcome::NotLocked,
                },
                // A benign skip, present so the sum is proven to ignore it
                // rather than merely happening to equal 3 without it.
                Prepared {
                    action: Action::Skip {
                        backend: SCOOP.into(),
                        name: Name::new("zellij"),
                        reason: SkipReason::Running,
                    },
                    outcome: Outcome::Skipped {
                        why: "running -- stop it first".into(),
                    },
                },
            ],
        };
        assert_eq!(
            prep.failed_count(),
            1,
            "the positive control on one operand"
        );
        assert_eq!(
            prep.not_locked_count(),
            2,
            "the positive control on the other operand"
        );
        assert_eq!(
            prep.unpreparable_count(),
            3,
            "the sum of the two, not either alone and not the constant 1"
        );
    }

    #[test]
    fn a_winget_prune_becomes_a_winget_remove_and_never_a_scoop_one() {
        // **This test's assertion is inverted, and the risk it guards is not.**
        // It was `a_winget_prune_produces_no_scoop_step_and_is_reported_as_
        // unrouted`, and while winget had no executor the only safe answer to a
        // winget `ReadyToRemove` was "no step at all". Winget has one now, so
        // the right step exists -- but the hazard is exactly the same one, and
        // now it is reachable in production: `backend == SCOOP` on the scoop
        // Prune arm is a runtime value check no type defends.
        // `Step::Scoop(ScoopStep::Remove)` compiles perfectly well for a winget
        // `Action::Prune` and would reach `run_scoop_step` and invoke `scoop
        // uninstall` against a winget package if the guard were deleted, and
        // nothing upstream stops it: `classify` maps every `Prune` to
        // `NoArtifactNeeded` -> `ReadyToRemove` with no backend check at all.
        // So the assertion is on the step's exact identity, not merely on its
        // count.
        let prep = Preparation {
            prepared: vec![Prepared {
                action: Action::Prune {
                    backend: WINGET.into(),
                    name: Name::new("Vivaldi.Vivaldi"),
                    version: "8.1.4087.62".into(),
                },
                outcome: Outcome::ReadyToRemove,
            }],
        };

        // The guard names come from the scan; here there is no scan row for
        // this id, which is the fallback `guard_for` documents.
        let (steps, unusable) = plan_to_steps(&prep, &[]);
        assert_eq!(
            steps,
            vec![Step::Winget(WingetStep::Remove {
                id: Name::new("Vivaldi.Vivaldi"),
                version: "8.1.4087.62".into(),
                guard: vec!["vivaldi".to_string(), "vivaldi.vivaldi".to_string()],
            })],
            "a winget prune must become winget's own removal, never scoop's"
        );
        assert!(unusable.is_empty(), "{unusable:?}");
    }

    #[test]
    fn a_prune_from_a_backend_that_is_neither_winget_nor_scoop_does_not_become_a_winget_removal() {
        // `backend == WINGET` (`plan_to_steps`) is the only thing standing
        // between this arm's `Step::Winget(WingetStep::Remove)` and a
        // routing bug -- but `Action::Prune` + `Outcome::ReadyToRemove`
        // structurally matches BOTH this arm and the scoop Prune arm above
        // it (`plan_to_steps`), so a literal `backend: SCOOP` action never even
        // reaches THIS arm's guard: the scoop arm's own `backend == SCOOP`
        // claims it first, every time, mutated or not. Only a THIRD backend
        // -- one that matches neither guard -- actually exercises this arm's
        // own check, which is why "chocolatey" (this file's own stand-in for
        // a backend that is neither, also used by
        // `prepare_refuses_to_stage_a_backend_it_has_no_preparation_for`
        // above) is used here rather than "scoop". Under `backend ==
        // WINGET -> true`, this becomes a `WingetStep::Remove` that would
        // run `winget uninstall` against a package no winget backend ever
        // declared.
        let prep = Preparation {
            prepared: vec![Prepared {
                action: Action::Prune {
                    backend: "chocolatey".into(),
                    name: Name::new("some-package"),
                    version: "1.0.0".into(),
                },
                outcome: Outcome::ReadyToRemove,
            }],
        };
        let (steps, unusable) = plan_to_steps(&prep, &[]);
        assert!(steps.is_empty(), "{steps:?}");
        assert_eq!(
            unusable,
            vec![(
                Name::new("some-package"),
                "chocolatey: prepared, but no executor claimed it -- this is a routing bug, \
                 not a package problem"
                    .to_string()
            )],
            "a non-winget, non-scoop backend's prune must never become a winget removal: \
             {unusable:?}"
        );
    }

    #[test]
    fn a_winget_step_takes_its_guard_names_from_the_scan_not_from_the_id_alone() {
        // The whole reason `plan_to_steps` takes `installed` at all. `Google.
        // Chrome`'s live process is `chrome.exe` and its display Name is
        // "Google Chrome" -- the display Name exists only in the scan, so
        // re-deriving guards here from the id would silently drop it, and
        // `execute`'s per-step re-sampler would stop catching a running Chrome.
        let mut inst = Installed {
            backend: WINGET.to_string(),
            // Deliberately a different spelling from the action's: `guard_for`
            // matches by `Name`, which folds case, because a lock key and a
            // scan row are two independently-cased sources for one package.
            name: Name::new("google.chrome"),
            version: "141.0.7390.123".to_string(),
            arch: None,
            bucket: None,
            bins: Vec::new(),
        };
        inst.bins = crate::backend::winget::guard_names("Google.Chrome", "Google Chrome");
        assert_eq!(
            inst.bins,
            vec!["chrome".to_string(), "google chrome".to_string()],
            "the fixture itself must carry the measured pair"
        );

        let prep = Preparation {
            prepared: vec![Prepared {
                action: Action::Upgrade {
                    backend: WINGET.into(),
                    name: Name::new("Google.Chrome"),
                    from: "141.0.7390.100".into(),
                    to: "141.0.7390.123".into(),
                    arch: None,
                },
                outcome: Outcome::ReadyToSet {
                    id: Name::new("Google.Chrome"),
                    version: "141.0.7390.123".into(),
                },
            }],
        };

        let (steps, unusable) = plan_to_steps(&prep, &[inst]);
        assert_eq!(
            steps,
            vec![Step::Winget(WingetStep::Set {
                id: Name::new("Google.Chrome"),
                version: "141.0.7390.123".into(),
                guard: vec!["chrome".to_string(), "google chrome".to_string()],
            })],
            "an Upgrade is a `Set`, and its guards come from the scan row"
        );
        assert!(unusable.is_empty(), "{unusable:?}");
        // The negative control for the lookup itself: with no scan row, the
        // display-name guard is unavailable and cannot be invented.
        let (fallback, _) = plan_to_steps(&prep, &[]);
        assert_eq!(
            fallback[0].guard_names(),
            ["chrome".to_string(), "google.chrome".to_string()],
            "the fallback derives from the id alone, and says so"
        );
    }

    #[test]
    fn guard_for_needs_both_the_right_backend_and_the_right_name_not_either_alone() {
        // `guard_for`'s `.find(|i| i.backend == WINGET && &i.name == name)`
        // (`guard_for`) survived mutation to `||` because both tests above
        // pass an `installed` slice with exactly one winget row -- with only
        // one candidate, "is winget" and "is this name" agree on the same
        // row, and `&&` vs `||` cannot be told apart. Two fixtures close that,
        // one per half of the guard.
        let target = || Preparation {
            prepared: vec![Prepared {
                action: Action::Upgrade {
                    backend: WINGET.into(),
                    name: Name::new("Google.Chrome"),
                    from: "141.0.7390.100".into(),
                    to: "141.0.7390.123".into(),
                    arch: None,
                },
                outcome: Outcome::ReadyToSet {
                    id: Name::new("Google.Chrome"),
                    version: "141.0.7390.123".into(),
                },
            }],
        };

        // Half 1: a DECOY winget row for a different package, ordered BEFORE
        // the target's own row. Under `||`, `backend == WINGET` alone is
        // already true for the decoy, so `find` stops there and hands back
        // the decoy's `bins` -- another package's process names -- for
        // `execute`'s per-step re-sampler to check a RUNNING guard against.
        let decoy_winget = Installed {
            backend: WINGET.to_string(),
            name: Name::new("Vivaldi.Vivaldi"),
            version: "8.1.4087.62".to_string(),
            arch: None,
            bucket: None,
            bins: vec!["vivaldi".to_string()],
        };
        let target_winget = Installed {
            backend: WINGET.to_string(),
            name: Name::new("Google.Chrome"),
            version: "141.0.7390.123".to_string(),
            arch: None,
            bucket: None,
            bins: vec!["chrome".to_string(), "google chrome".to_string()],
        };
        let (steps, _) = plan_to_steps(&target(), &[decoy_winget, target_winget]);
        assert_eq!(
            steps,
            vec![Step::Winget(WingetStep::Set {
                id: Name::new("Google.Chrome"),
                version: "141.0.7390.123".into(),
                guard: vec!["chrome".to_string(), "google chrome".to_string()],
            })],
            "a decoy winget row for a different package must not win just for \
             being winget and ordered first: {steps:?}"
        );

        // Half 2: a SCOOP row whose NAME collides with the target, and no
        // matching winget row at all. Under `||`, `&i.name == name` alone is
        // already true for the scoop row, so `find` would hand back a scoop
        // package's `bins` for a winget guard. The correct answer with no
        // winget row present is `guard_for`'s own id-derived fallback.
        let colliding_scoop = Installed {
            backend: SCOOP.to_string(),
            name: Name::new("Google.Chrome"),
            version: "1.0.0".to_string(),
            arch: None,
            bucket: None,
            bins: vec!["not-the-winget-bins".to_string()],
        };
        let (fallback, _) = plan_to_steps(&target(), &[colliding_scoop]);
        assert_eq!(
            fallback,
            vec![Step::Winget(WingetStep::Set {
                id: Name::new("Google.Chrome"),
                version: "141.0.7390.123".into(),
                guard: vec!["chrome".to_string(), "google.chrome".to_string()],
            })],
            "a same-named SCOOP row must not be mistaken for the winget scan \
             row; with no matching winget row, guard_for must fall back to \
             the id-derived guess: {fallback:?}"
        );
    }

    /// A helper for the fence-coverage tests below: one winget `Installed`
    /// carrying whatever `guard_names` would have guessed for it.
    fn winget_row(id: &str, guesses: &[&str]) -> Installed {
        Installed {
            backend: WINGET.to_string(),
            name: Name::new(id),
            version: "1.0.0".to_string(),
            arch: None,
            bucket: None,
            bins: guesses.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn upgrade_of(id: &str) -> Action {
        Action::Upgrade {
            backend: WINGET.into(),
            name: Name::new(id),
            from: "1.0.0".into(),
            to: "2.0.0".into(),
            arch: None,
        }
    }

    #[test]
    fn a_winget_change_dotpkg_cannot_see_by_path_and_has_no_guard_entry_for_is_reported() {
        // The measured hole this exists to close: on a14, winget creates a
        // package directory for 4 of 41 installed ids -- every `portable` one
        // and no other -- so for the other 37 the path signal can never fire.
        // With no `[winget.guard]` entry either, the only names left are
        // `guard_names`' guesses, and those are measured wrong for the one
        // package anybody checked: `BurntSushi.ripgrep.MSVC` guesses `msvc`
        // and `ripgrep msvc`, and the process is `rg`.
        //
        // Before this, that user got no protection and no sentence saying so.
        let empty_root = tempfile::tempdir().unwrap();
        let roots = vec![empty_root.path().to_path_buf()];

        let plan = Plan {
            actions: vec![upgrade_of("BurntSushi.ripgrep.MSVC")],
        };
        let installed = vec![winget_row(
            "BurntSushi.ripgrep.MSVC",
            &["msvc", "ripgrep msvc"],
        )];

        let out = unprotected_winget_changes_with_roots(
            &plan,
            &std::collections::BTreeMap::new(),
            &installed,
            &roots,
        );

        assert_eq!(out.len(), 1, "expected exactly one warning, got {out:?}");
        let w = &out[0];
        assert!(
            w.contains("BurntSushi.ripgrep.MSVC"),
            "the warning must name the package the user has to act on: {w}"
        );
        assert!(
            w.contains("[winget.guard]"),
            "the warning must name the section the entry goes in: {w}"
        );
        assert!(
            w.contains("msvc"),
            "the warning must name the guesses, so the user can judge whether \
             they are right rather than take dotpkg's word for it: {w}"
        );
    }

    #[test]
    fn the_guard_entry_the_warning_suggests_is_valid_toml_that_dotpkg_itself_parses() {
        // Found by this branch's own review and, independently, by the
        // post-merge audit. The warning's whole purpose is to hand the user a
        // line they can paste, and **every real winget id contains a dot**: an
        // unquoted `Google.Chrome = ["chrome"]` under `[winget.guard]` is a
        // TOML *table path*, not a key, so `config::parse` fails with
        // `invalid type: map, expected a sequence`. Correct diagnosis, unusable
        // advice.
        //
        // Asserting a substring cannot catch that, which is why this test
        // extracts the suggested line and feeds it to the real parser. The
        // check is the round trip, not the spelling.
        let empty_root = tempfile::tempdir().unwrap();
        let id = "Google.Chrome";
        let out = unprotected_winget_changes_with_roots(
            &Plan {
                actions: vec![upgrade_of(id)],
            },
            &std::collections::BTreeMap::new(),
            &[winget_row(id, &["chrome", "google chrome"])],
            &[empty_root.path().to_path_buf()],
        );
        assert_eq!(out.len(), 1, "{out:?}");

        // Lift the suggestion out of the sentence the way a reader would.
        let msg = &out[0];
        let start = msg
            .find("add ")
            .expect("the message offers something to add")
            + "add ".len();
        let end = msg[start..]
            .find(" under [winget.guard]")
            .expect("the message says where the line goes")
            + start;
        let suggested = msg[start..end].replace("<process name>", "chrome");

        let toml = format!("[winget.guard]\n{suggested}\n");
        let cfg = crate::config::parse(&toml).unwrap_or_else(|e| {
            panic!(
                "dotpkg's own parser rejects the line it told the user to \
                 add.\n  suggested: {suggested}\n  error: {e:#}"
            )
        });

        assert_eq!(
            cfg.winget.guard.get(&Name::new(id)).map(Vec::as_slice),
            Some(["chrome".to_string()].as_slice()),
            "the suggested line must land as a guard entry for the package the \
             warning named, not as a nested table: {toml}"
        );
    }

    #[test]
    fn a_winget_change_the_path_signal_can_see_is_not_reported() {
        // The first of the two ways a package IS protected, and the one that
        // needs no declaration: winget gave it a package directory, so
        // `running_ids` can match a process running out of it. This is the
        // control that stops the warning from firing on all 41 ids instead of
        // the 37 it means.
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(
            root.path()
                .join("burntsushi.ripgrep.msvc_Microsoft.Winget.Source_8wekyb3d8bbwe"),
        )
        .unwrap();
        let roots = vec![root.path().to_path_buf()];

        let out = unprotected_winget_changes_with_roots(
            &Plan {
                actions: vec![upgrade_of("BurntSushi.ripgrep.MSVC")],
            },
            &std::collections::BTreeMap::new(),
            &[winget_row("BurntSushi.ripgrep.MSVC", &["msvc"])],
            &roots,
        );
        assert!(
            out.is_empty(),
            "a package the path signal covers must not be reported as invisible: {out:?}"
        );
    }

    #[test]
    fn a_winget_change_with_a_declared_guard_entry_is_not_reported() {
        // The other way a package is protected: the user answered the question
        // this warning asks. Asking again would be the flood.
        let empty_root = tempfile::tempdir().unwrap();
        let mut guard = std::collections::BTreeMap::new();
        guard.insert(Name::new("BurntSushi.ripgrep.MSVC"), vec!["rg".to_string()]);

        let out = unprotected_winget_changes_with_roots(
            &Plan {
                actions: vec![upgrade_of("BurntSushi.ripgrep.MSVC")],
            },
            &guard,
            &[winget_row("BurntSushi.ripgrep.MSVC", &["msvc"])],
            &[empty_root.path().to_path_buf()],
        );
        assert!(
            out.is_empty(),
            "a package with a declared guard entry must not be warned about: {out:?}"
        );
    }

    #[test]
    fn a_winget_install_is_not_reported_because_nothing_of_it_can_be_running_yet() {
        // `Install` is the one acting shape that cannot damage a live process:
        // there is no installation to replace. Keyed on the variant rather than
        // on "does it change something", which would sweep this in.
        let empty_root = tempfile::tempdir().unwrap();
        let out = unprotected_winget_changes_with_roots(
            &Plan {
                actions: vec![Action::Install {
                    backend: WINGET.into(),
                    name: Name::new("Obsidian.Obsidian"),
                    version: "1.10.0".into(),
                    arch: None,
                }],
            },
            &std::collections::BTreeMap::new(),
            &[],
            &[empty_root.path().to_path_buf()],
        );
        assert!(
            out.is_empty(),
            "an install has nothing installed to be running: {out:?}"
        );
    }

    #[test]
    fn a_winget_downgrade_is_not_reported_because_winget_refuses_it_rather_than_performing_it() {
        // Found by running the check against a real machine rather than by
        // reading it. Declaring all 41 of a14's installed ids pinned above
        // their installed version produced two `Action::Downgrade`s --
        // `Google.Chrome` and `Microsoft.Edge`, both installed ahead of the pin
        // -- and the first version of this check warned about both.
        //
        // The sentence it printed was false. A winget downgrade does reach
        // `execute` and does fire `winget install --version <pin>`, but
        // measured, that command only ever moves a package *up*: it comes back
        // `NO_AVAILABLE_UPGRADE`, the step ends `touched: false`, and
        // `render`'s summary counts it separately from `change_count` for
        // exactly this reason. So "dotpkg may downgrade it while it is running"
        // describes something dotpkg has been measured unable to do, and a
        // warning that sends the user to guard against it is asking for work
        // that buys nothing.
        //
        // `Prune` stays in: `winget uninstall` does remove a running package.
        let empty_root = tempfile::tempdir().unwrap();
        let out = unprotected_winget_changes_with_roots(
            &Plan {
                actions: vec![Action::Downgrade {
                    backend: WINGET.into(),
                    name: Name::new("Google.Chrome"),
                    from: "150.0.7871.187".into(),
                    to: "99.0.0".into(),
                    arch: None,
                }],
            },
            &std::collections::BTreeMap::new(),
            &[winget_row("Google.Chrome", &["chrome", "google chrome"])],
            &[empty_root.path().to_path_buf()],
        );
        assert!(
            out.is_empty(),
            "a winget downgrade is refused rather than performed, so there is nothing \
             for a guard entry to protect: {out:?}"
        );
    }

    #[test]
    fn a_winget_removal_is_still_reported_because_uninstall_really_does_remove_it() {
        // The other half of the pair above, and the reason `Prune` cannot be
        // dropped alongside `Downgrade`: `winget uninstall` is not refused.
        let empty_root = tempfile::tempdir().unwrap();
        let out = unprotected_winget_changes_with_roots(
            &Plan {
                actions: vec![Action::Prune {
                    backend: WINGET.into(),
                    name: Name::new("Obsidian.Obsidian"),
                    version: "1.12.7".into(),
                }],
            },
            &std::collections::BTreeMap::new(),
            &[winget_row("Obsidian.Obsidian", &["obsidian"])],
            &[empty_root.path().to_path_buf()],
        );
        assert_eq!(
            out.len(),
            1,
            "a removal can still hit a running package: {out:?}"
        );
        assert!(
            out[0].contains("remove"),
            "the message must name the action it is warning about: {}",
            out[0]
        );
    }

    #[test]
    fn a_scoop_removal_is_not_reported_as_a_winget_removal() {
        // The `Upgrade` arm's `backend == WINGET` guard was already pinned by
        // the scoop-upgrade test below; the `Prune` arm's was not, and an
        // `--in-diff` mutation run found it: replacing that guard with `true`
        // survived the whole suite. Both arms carry the same guard, so both
        // need their own fixture -- one test per arm, not one per predicate.
        let empty_root = tempfile::tempdir().unwrap();
        let out = unprotected_winget_changes_with_roots(
            &Plan {
                actions: vec![Action::Prune {
                    backend: SCOOP.into(),
                    name: Name::new("ripgrep"),
                    version: "15.2.0".into(),
                }],
            },
            &std::collections::BTreeMap::new(),
            &[],
            &[empty_root.path().to_path_buf()],
        );
        assert!(
            out.is_empty(),
            "a scoop removal must not be reported as a missing [winget.guard] entry: {out:?}"
        );
    }

    #[test]
    fn the_production_entry_point_reports_through_the_roots_it_reads_for_itself() {
        // `unprotected_winget_changes` is the thin wrapper that reads the
        // environment, and `_with_roots` is the seam every other test drives.
        // An `--in-diff` mutation run found the wrapper completely unpinned --
        // `vec![]`, `vec![String::new()]` and `vec!["xyzzy".into()]` all
        // survived -- which is the same shape `package_roots()` carried for two
        // phases before one assertion closed it.
        //
        // It READS the environment and never sets it: `std::env::set_var` is
        // process-global and this suite runs in parallel, which is why the
        // roots seam exists at all.
        //
        // The id is synthetic so the answer cannot depend on which machine this
        // runs on: with the variables unset the root list is empty, and with
        // them set no winget package directory is named this, so both give the
        // same verdict.
        let id = "Dotpkg.NoSuchPackage.ForTheWrapperTest";
        let out = unprotected_winget_changes(
            &Plan {
                actions: vec![upgrade_of(id)],
            },
            &std::collections::BTreeMap::new(),
            &[winget_row(id, &["forthewrappertest"])],
        );
        assert_eq!(
            out.len(),
            1,
            "the wrapper must report through real roots: {out:?}"
        );
        assert!(
            out[0].contains(id) && out[0].contains("[winget.guard]"),
            "and it must be the real message, not any one-element vector: {}",
            out[0]
        );
    }

    #[test]
    fn a_scoop_change_is_not_reported_by_the_winget_fence_coverage_check() {
        // Scoop's path signal is a different mechanism with a different root,
        // and it covers every scoop package rather than 4 of 41. A warning
        // about a missing `[winget.guard]` entry for a scoop package would send
        // the user to edit a table that cannot affect it.
        let empty_root = tempfile::tempdir().unwrap();
        let out = unprotected_winget_changes_with_roots(
            &Plan {
                actions: vec![Action::Upgrade {
                    backend: SCOOP.into(),
                    name: Name::new("ripgrep"),
                    from: "14.1.0".into(),
                    to: "15.2.0".into(),
                    arch: None,
                }],
            },
            &std::collections::BTreeMap::new(),
            &[],
            &[empty_root.path().to_path_buf()],
        );
        assert!(
            out.is_empty(),
            "scoop is not this check's business: {out:?}"
        );
    }

    #[test]
    fn the_package_directory_check_matches_a_directory_the_fence_would_match_and_no_other() {
        // The rule is shared with `running_ids` through
        // `backend::winget::segment_names_id`, and this pins both ends of it
        // from the directory side. A directory that merely STARTS with the id
        // belongs to a different package -- `PhatMT97.VKey.Classic_…` is a real
        // one on a14, sitting beside `PhatMT97.VKey_…` -- and treating it as a
        // match would silence the warning for the wrong package.
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(
            root.path()
                .join("phatmt97.vkey.classic_Microsoft.Winget.Source"),
        )
        .unwrap();
        let roots = vec![root.path().to_path_buf()];

        let out = unprotected_winget_changes_with_roots(
            &Plan {
                actions: vec![upgrade_of("PhatMT97.VKey")],
            },
            &std::collections::BTreeMap::new(),
            &[winget_row("PhatMT97.VKey", &["vkey"])],
            &roots,
        );
        assert_eq!(
            out.len(),
            1,
            "`PhatMT97.VKey.Classic_…` is a different package's directory and must not \
             be read as covering `PhatMT97.VKey`: {out:?}"
        );

        // And the exact-name form, which winget does not currently produce but
        // `segment_names_id` accepts, does cover it.
        std::fs::create_dir(root.path().join("phatmt97.vkey")).unwrap();
        let covered = unprotected_winget_changes_with_roots(
            &Plan {
                actions: vec![upgrade_of("PhatMT97.VKey")],
            },
            &std::collections::BTreeMap::new(),
            &[winget_row("PhatMT97.VKey", &["vkey"])],
            &roots,
        );
        assert!(
            covered.is_empty(),
            "an exactly-named directory covers it: {covered:?}"
        );
    }

    #[test]
    fn a_winget_install_and_a_winget_upgrade_produce_no_scoop_step_and_are_reported_as_unrouted() {
        // Mirrors the test above for the other two `backend == SCOOP`
        // guards (`plan_to_steps`): a winget action ready to fetch
        // must not fall through into a `ScoopStep::Install` or
        // `ScoopStep::Replace` either.
        let prep = Preparation {
            prepared: vec![
                Prepared {
                    action: Action::Install {
                        backend: WINGET.into(),
                        name: Name::new("Brave.Brave"),
                        version: "151.1.93.134".into(),
                        arch: None,
                    },
                    outcome: Outcome::ReadyToFetch {
                        manifest: PathBuf::from("/stage/Brave.Brave/151.1.93.134/manifest.json"),
                    },
                },
                Prepared {
                    action: Action::Upgrade {
                        backend: WINGET.into(),
                        name: Name::new("7zip.7zip"),
                        from: "26.01.00.0".into(),
                        to: "26.02".into(),
                        arch: None,
                    },
                    outcome: Outcome::ReadyToFetch {
                        manifest: PathBuf::from("/stage/7zip.7zip/26.02/manifest.json"),
                    },
                },
            ],
        };

        let (steps, unusable) = plan_to_steps(&prep, &[]);
        assert!(steps.is_empty(), "{steps:?}");
        assert_eq!(
            unusable,
            vec![
                (
                    Name::new("Brave.Brave"),
                    "winget: prepared, but no executor claimed it -- this is a routing bug, \
                     not a package problem"
                        .to_string()
                ),
                (
                    Name::new("7zip.7zip"),
                    "winget: prepared, but no executor claimed it -- this is a routing bug, \
                     not a package problem"
                        .to_string()
                ),
            ],
            "a winget install/upgrade must never fall through to a scoop step: {unusable:?}"
        );
    }

    #[test]
    fn a_scoop_action_carrying_a_readytoset_does_not_become_a_winget_step() {
        // `backend == WINGET` (`plan_to_steps`) is the only thing standing
        // between this arm's `Step::Winget(WingetStep::Set)` and a routing
        // bug: nothing in the type system stops a SCOOP action from
        // carrying an `Outcome::ReadyToSet` -- scoop's own arms above this
        // one only match `ReadyToFetch`, so this pairing reaches THIS arm's
        // guard directly, with no earlier arm to intercept it first (unlike
        // the Prune/ReadyToRemove shape below, where the scoop arm always
        // claims a literal `backend: SCOOP` first). Under `backend ==
        // WINGET -> true`, this becomes a `WingetStep::Set` that would run
        // `winget install` against a scoop package.
        let prep = Preparation {
            prepared: vec![Prepared {
                action: Action::Install {
                    backend: SCOOP.into(),
                    name: Name::new("fzf"),
                    version: "1.0.0".into(),
                    arch: None,
                },
                outcome: Outcome::ReadyToSet {
                    id: Name::new("fzf"),
                    version: "1.0.0".into(),
                },
            }],
        };
        let (steps, unusable) = plan_to_steps(&prep, &[]);
        assert!(steps.is_empty(), "{steps:?}");
        assert_eq!(
            unusable,
            vec![(
                Name::new("fzf"),
                "scoop: prepared, but no executor claimed it -- this is a routing bug, not a \
                 package problem"
                    .to_string()
            )],
            "a scoop action must never become a winget step, even carrying an outcome no real \
             prepare() call pairs it with: {unusable:?}"
        );
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

        let (steps, unusable) = plan_to_steps(&prep, &[]);
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

        let (steps, unusable) = plan_to_steps(&prep, &[]);
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

        let (steps, unusable) = plan_to_steps(&prep, &[]);
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
            Step::Scoop(ScoopStep::Install {
                app: Name::new("fzf"),
                staged: PathBuf::from("/stage/fzf/1.0.0/fzf.json"),
                arch: None,
            }),
            Step::Scoop(ScoopStep::Remove {
                app: Name::new("aichat"),
            }),
            // A winget removal in the same list: `WingetStep::Remove` became
            // reachable at Phase 4b Task 13, and `is_remove()` must gate it exactly as
            // it gates scoop's -- a gate that only recognised one backend's
            // removals would be silently permissive for the other, which is the
            // one mistake this function's own doc comment is about.
            Step::Winget(WingetStep::Remove {
                id: Name::new("OpenAI.Codex"),
                version: "0.145.0".into(),
                guard: vec!["codex".into()],
            }),
        ];
        let (kept, held) = gate_removals(steps, false);
        assert_eq!(
            kept,
            vec![Step::Scoop(ScoopStep::Install {
                app: Name::new("fzf"),
                staged: PathBuf::from("/stage/fzf/1.0.0/fzf.json"),
                arch: None,
            })],
            "a non-removal step must still run"
        );
        // Each held removal carries the backend that would have performed it,
        // because `main.rs` prints it and puts it in the closing table.
        assert_eq!(
            held,
            vec![
                (SCOOP.to_string(), Name::new("aichat")),
                (WINGET.to_string(), Name::new("OpenAI.Codex")),
            ]
        );
    }

    #[test]
    fn gate_removals_lets_every_step_through_when_the_preparation_is_ok() {
        // The positive control: without it, a version that always holds
        // everything back would pass the test above too.
        let steps = vec![
            Step::Scoop(ScoopStep::Remove {
                app: Name::new("aichat"),
            }),
            Step::Scoop(ScoopStep::Install {
                app: Name::new("fzf"),
                staged: PathBuf::from("/stage/fzf/1.0.0/fzf.json"),
                arch: None,
            }),
        ];
        let (kept, held) = gate_removals(steps.clone(), true);
        assert_eq!(kept, steps);
        assert!(held.is_empty());
    }
}
