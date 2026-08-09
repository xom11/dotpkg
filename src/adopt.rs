//! `dotpkg adopt` — bringing an already-installed package under management.
//!
//! Reaches no network and changes no installed software. Its whole job is to
//! find the commit whose manifest is the one this machine is actually running,
//! and then to write the three files that make the package managed rather than
//! merely known about.

use crate::bucket;
use crate::model::Name;
use anyhow::Result;
use std::path::Path;

/// Which rule found the commit. Reported, because the two are not equally
/// strong and a user is entitled to know which one answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Matched {
    /// The installed manifest and the bucket blob are the same file. Exact,
    /// and the only rule that can tell two same-version commits apart.
    Content,
    /// Only the version agreed. Weaker: measured, when a bucket amends a
    /// manifest without bumping the version, this picks the newer of the two.
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub commit: String,
    pub version: String,
    pub matched: Matched,
}

/// Find the commit that carries what is installed.
///
/// `Ok(None)` means no commit in this bucket's history for this app carries
/// the installed version -- an ordinary answer while searching, and the caller
/// turns it into a refusal that writes nothing.
///
/// Content is tried across the whole history before version is tried at all,
/// rather than per commit: an exact match anywhere beats an approximate match
/// higher up. Measured, the difference is which of two same-version commits
/// gets pinned, and the version rule picks the wrong one.
pub fn resolve_installed(
    bucket_dir: &Path,
    app: &Name,
    installed_version: &str,
    installed_manifest: &[u8],
    rev: &str,
) -> Result<Option<Found>> {
    let Some(path_in_repo) = bucket::manifest_path(bucket_dir, app, rev) else {
        return Ok(None);
    };
    // --full-history: measured, the default walk hides a version that reached
    // the bucket only on a branch whose change was superseded at merge time.
    let commits = bucket::history(bucket_dir, &path_in_repo, rev)?;
    let blobs = bucket::blobs(bucket_dir, &commits, &path_in_repo)?;

    let want = crate::verify::normalise(installed_manifest);
    for (commit, blob) in commits.iter().zip(blobs.iter()) {
        let Some(body) = blob else { continue };
        if crate::verify::normalise(body) == want {
            return Ok(Some(Found {
                commit: commit.clone(),
                version: blob_version(body).unwrap_or_else(|| installed_version.to_string()),
                matched: Matched::Content,
            }));
        }
    }
    for (commit, blob) in commits.iter().zip(blobs.iter()) {
        let Some(body) = blob else { continue };
        if blob_version(body).as_deref() == Some(installed_version) {
            return Ok(Some(Found {
                commit: commit.clone(),
                version: installed_version.to_string(),
                matched: Matched::Version,
            }));
        }
    }
    Ok(None)
}

fn blob_version(body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.get("version")?.as_str().map(str::to_string)
}

use crate::backend::Backend;
use crate::config::Config;
use crate::lock::{Lock, Pin};
use crate::state::{Ownership, State};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub adopted: Vec<(Name, Matched)>,
    pub refused: Vec<(Name, String)>,
    /// What `scan` could not read, carried out so the caller can print it.
    ///
    /// A package whose `manifest.json` cannot be read is absent from `scan`,
    /// so `adopt` refuses it with "<name> is not installed" -- which is false,
    /// and, without this, printed with no diagnostic at all. `status`, `apply`
    /// and `update` have each printed these warnings since Phase 2a; `adopt`
    /// was the one command that dropped them on the floor, and the Phase 3
    /// dogfood found it by adopting a package a junction made unreadable.
    pub warnings: Vec<String>,
    /// A write that failed part way through, and which of the three files it
    /// had already changed.
    ///
    /// This used to propagate with `?`, which skipped `render_adopt` entirely:
    /// the user was told `cannot create ...\state.json.tmp1234` and nothing
    /// anywhere said that `pkg.lock` and `pkg.toml` had already been rewritten.
    /// Fatal to the run -- the packages after it are not attempted -- but
    /// reported rather than swallowed.
    pub partial_write: Option<PartialWrite>,
}

/// What a write that stopped part way through left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialWrite {
    pub name: Name,
    /// The files that really were rewritten, in the order they were written.
    /// Never includes the one that failed.
    pub wrote: Vec<&'static str>,
    pub why: String,
}

/// Adopt every named package. Per package it is all or nothing.
///
/// Across packages, a **refusal** is reported and the rest proceed, the same
/// shape as `prepare`. A **write failure** is not a refusal and does not work
/// that way: the three files are re-read at the top of every iteration, so a
/// half-written set makes every later package's guards read a state dotpkg
/// itself does not understand. It stops the run, and it is recorded in
/// `Outcome::partial_write` -- naming which files really did change -- rather
/// than propagating out of a `?` that would skip the report entirely.
///
/// **Write order: `pkg.lock`, then `pkg.toml`, then `state.json`.** Every
/// prefix of that order is inert:
///
/// - lock only: an entry for an undeclared package. `plan()` never reads it
///   and the next whole-run `update` drops it.
/// - lock + `pkg.toml`: declared, locked, and installed at the locked version,
///   so `plan()` emits nothing at all.
/// - all three: adopted.
///
/// The dangerous order is `state.json` first, which makes the package
/// `installed ∧ ¬declared ∧ owned` -- a **prune candidate** (`src/plan.rs`).
/// This mirrors the executor's own reasoning about claiming ownership late.
pub fn run(
    scoop_root: &Path,
    names: &[Name],
    config_path: &Path,
    lock_path: &Path,
    state_path: &Path,
) -> Result<Outcome> {
    let scoop = crate::backend::scoop::Scoop::new(scoop_root.to_path_buf());
    let scan = Backend::scan(&scoop)?;
    let mut out = Outcome {
        warnings: scan.warnings.clone(),
        ..Outcome::default()
    };

    for name in names {
        // Re-read all three every iteration: each package's write must land
        // before the next one's guard reads it, or adopting two packages in
        // one command would lose the first.
        let declared = crate::config::load(config_path)?;
        let mut lock = crate::lock::load_or_empty(lock_path)?;
        // No special-casing here: a state.json this cannot read (a directory
        // sitting at that path, a permission denial, corrupt JSON) is a
        // condition dotpkg cannot understand, so the whole package refuses --
        // via this `?` -- before anything is written, rather than proceeding
        // on a guessed-empty ownership record. Defaulting to "nothing owned"
        // here would let `adopt` write pkg.lock and edit pkg.toml on a false
        // belief and discover the problem only at the final `state.save`.
        let mut state = State::load_or_empty(state_path)?;

        match adopt_one(
            scoop_root,
            &scan,
            &declared,
            &lock,
            &state,
            name,
            config_path,
        ) {
            Err(why) => out.refused.push((name.clone(), why)),
            Ok((bucket_name, found, config_text)) => {
                lock.scoop.insert(
                    name.clone(),
                    Pin::ScoopCommit {
                        // `key()`, matching `update`: `choose_bucket` opened
                        // `buckets/<key>` and `Scoop::stage` opens what the
                        // lock says verbatim, so the display spelling would
                        // name a directory nothing verified.
                        bucket: bucket_name.key().to_string(),
                        commit: found.commit.clone(),
                        version: found.version.clone(),
                    },
                );
                state.set(crate::model::SCOOP, name, Ownership::Adopted);
                if let Err(failure) = write_in_order(
                    WriteLock(|| crate::lock::save(&lock, lock_path)),
                    WritePkgToml(|| crate::config_edit::save(config_path, &config_text)),
                    WriteState(|| state.save(state_path)),
                ) {
                    out.partial_write = Some(PartialWrite {
                        name: name.clone(),
                        wrote: failure.wrote,
                        why: format!("{:#}", failure.error),
                    });
                    return Ok(out);
                }
                out.adopted.push((name.clone(), found.matched));
            }
        }
    }
    Ok(out)
}

/// One wrapper per write, so the three cannot be passed in the wrong order.
///
/// Without these, `write_in_order` takes three closures of indistinguishable
/// type, positionally, and swapping two of them at the call site compiles and
/// ships. That mistake is exactly the `state.json`-first ordering this
/// module's whole doc comment exists to forbid, and it was **measured** to be
/// invisible: with the arguments reversed, all 175 library tests passed --
/// including both seam tests below, which exercise `write_in_order` with their
/// own recorders and therefore cannot observe what `run` hands it. The only
/// test that caught it was `#[cfg(unix)]`, so on Windows -- this tool's only
/// real target -- the reversal was undetectable.
///
/// Same move `Name` makes in `crate::model`: the type exists so that the wrong
/// thing is not a bug to be caught but a program that cannot be written. It
/// needs no test, runs on every platform, and cannot rot.
struct WriteLock<F>(F);
struct WritePkgToml<F>(F);
struct WriteState<F>(F);

/// The write order itself, behind a seam: lock, then pkg.toml, then
/// state.json, stopping at the first failure. `run` always calls this with
/// closures over the real `lock::save` / `config_edit::save` / `State::save`
/// -- the only reason this exists separately is so the ORDER is directly
/// observable in a test, by injecting closures that record each call, rather
/// than only inferable from what a real interrupted write leaves behind.
///
/// Three properties, held by three different things, deliberately:
///
/// - **Which closure goes in which position** -- held by the wrapper types
///   above, at compile time, on every platform.
/// - **That this function calls them in order and short-circuits** -- held by
///   the two seam tests below, portably.
/// - **That the sequence survives a real interrupted write** -- held by
///   `tests/adopt.rs`'s `a_failed_last_write_leaves_a_prefix_that_plan_does_
///   nothing_about` (`#[cfg(unix)]`, a real filesystem failure).
///
/// The failure carries the prefix that really did land. The error alone names
/// only the file that failed, and "which files did this leave changed" is the
/// one question a user whose `adopt` died half way through actually has.
fn write_in_order<L, P, S>(
    write_lock: WriteLock<L>,
    write_pkg_toml: WritePkgToml<P>,
    write_state: WriteState<S>,
) -> std::result::Result<(), WriteFailure>
where
    L: FnOnce() -> Result<()>,
    P: FnOnce() -> Result<()>,
    S: FnOnce() -> Result<()>,
{
    let mut wrote: Vec<&'static str> = Vec::new();
    if let Err(error) = (write_lock.0)() {
        return Err(WriteFailure { wrote, error });
    }
    wrote.push("pkg.lock");
    if let Err(error) = (write_pkg_toml.0)() {
        return Err(WriteFailure { wrote, error });
    }
    wrote.push("pkg.toml");
    if let Err(error) = (write_state.0)() {
        return Err(WriteFailure { wrote, error });
    }
    Ok(())
}

/// A write that stopped part way through, and the prefix it left behind.
#[derive(Debug)]
struct WriteFailure {
    wrote: Vec<&'static str>,
    error: anyhow::Error,
}

/// Everything that can refuse, before anything is written. Returns the pieces
/// the caller needs, so no partial state can exist between a check and a write.
#[allow(clippy::too_many_arguments)]
fn adopt_one(
    scoop_root: &Path,
    scan: &crate::backend::Scan,
    declared: &Config,
    lock: &Lock,
    state: &State,
    name: &Name,
    config_path: &Path,
) -> std::result::Result<(Name, Found, String), String> {
    let Some(inst) = scan
        .installed
        .iter()
        .find(|i| i.backend == crate::model::SCOOP && &i.name == name)
    else {
        return Err(format!(
            "{name} is not installed. `adopt` brings an existing package under \
             management; to install one, declare it and run `dotpkg update` then \
             `dotpkg apply`."
        ));
    };
    if state.owns(crate::model::SCOOP, name) {
        return Err(format!("{name} is already managed by dotpkg"));
    }

    let already = lock.scoop.get(name).and_then(|p| match p {
        Pin::ScoopCommit { bucket, .. } => Some(bucket.as_str()),
        Pin::WingetVersion { .. } => None,
    });
    // install.json's `bucket` is a legitimate hint here and nowhere else:
    // adopt targets packages dotpkg has never touched, and it is dotpkg's own
    // installs that lose the field.
    let hint = already.or(inst.bucket.as_deref());
    let (bucket_name, dir, rev) = match bucket::choose_bucket(scoop_root, declared, name, hint) {
        bucket::BucketChoice::Chosen { name: b, dir, tip } => (b, dir, tip.rev),
        bucket::BucketChoice::Ambiguous { candidates } => {
            let names: Vec<String> = candidates.iter().map(|c| c.to_string()).collect();
            return Err(format!(
                "{} declared buckets carry {name} ({}). Say which with \
                     `[scoop.opts] {name} = {{ bucket = \"...\" }}`.",
                candidates.len(),
                names.join(", ")
            ));
        }
        bucket::BucketChoice::NotCloned { name: b, dir } => {
            return Err(bucket::not_cloned_why(&name.to_string(), &b, &dir));
        }
        bucket::BucketChoice::NotFound { searched, missing } => {
            return Err(bucket::not_found_why(
                &name.to_string(),
                &searched,
                &missing,
            ));
        }
    };

    // Read, not `unwrap_or_default()`. An unreadable installed manifest used
    // to become an EMPTY one, which no bucket blob can match -- so the content
    // loop found nothing, the version loop answered, and the user was told
    // "matched by version only -- the installed manifest differs". That line
    // is false: the manifest was not compared at all. Low reachability (a
    // TOCTOU window after `scan` read the same file) but it was the one place
    // in the new code where an unreadable file became a benign default.
    let manifest_path = scoop_root
        .join("apps")
        .join(inst.name.to_string())
        .join("current")
        .join("manifest.json");
    let installed_manifest = std::fs::read(&manifest_path).map_err(|e| {
        format!(
            "cannot read the installed manifest at {}: {e}. Without it there is \
             nothing to match against, and matching on the version alone would \
             report a comparison that never happened.",
            manifest_path.display()
        )
    })?;

    let found = match resolve_installed(&dir, name, &inst.version, &installed_manifest, &rev) {
        Ok(Some(f)) => f,
        Ok(None) => {
            // Measured: a shallow clone gives exactly this answer with no
            // other signal, and the user cannot tell the two apart.
            let shallow = if bucket::is_shallow(&dir) {
                format!(
                    " -- and bucket {bucket_name} is a SHALLOW clone, so most of its \
                     history is not on this machine. `git -C {} fetch --unshallow` \
                     and try again.",
                    dir.display()
                )
            } else {
                String::new()
            };
            return Err(format!(
                "no commit in bucket {bucket_name} carries {name} {}{}",
                inst.version, shallow
            ));
        }
        Err(e) => return Err(format!("{e:#}")),
    };

    // Prepared, not written: the caller writes all three in order only once
    // every refusal above has been passed.
    let text = std::fs::read_to_string(config_path).map_err(|e| format!("{e}"))?;
    let config_text = if declared.scoop.packages.contains(name) {
        text
    } else {
        crate::config_edit::add_scoop_package(&text, name).map_err(|e| format!("{e:#}"))?
    };

    Ok((bucket_name, found, config_text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// The seam itself, exercised directly and portably: no filesystem, no
    /// `#[cfg(unix)]`, so this runs -- and would catch a regression -- on
    /// Windows, the one platform `a_failed_last_write_leaves_a_prefix_that_
    /// plan_does_nothing_about` (tests/adopt.rs) cannot reach.
    #[test]
    fn write_in_order_calls_lock_then_pkg_toml_then_state_and_propagates_the_last_failure() {
        // Named for what it can actually discriminate. The third write is the
        // last, so this test alone cannot tell "stopped after the failure"
        // from "recorded it and had nothing left to do" -- its sibling below,
        // where the FIRST write fails, is the short-circuit proof.
        let log: RefCell<Vec<&str>> = RefCell::new(Vec::new());
        let result = write_in_order(
            WriteLock(|| {
                log.borrow_mut().push("lock");
                Ok(())
            }),
            WritePkgToml(|| {
                log.borrow_mut().push("pkg.toml");
                Ok(())
            }),
            WriteState(|| {
                log.borrow_mut().push("state.json");
                anyhow::bail!("state.json write failed")
            }),
        );

        let failure = result.expect_err("the third write's failure must propagate");
        assert_eq!(
            failure.wrote,
            vec!["pkg.lock", "pkg.toml"],
            "the two writes that really landed must be named, and the one that \
             failed must not be: this list is what `render_adopt` tells the user \
             was changed"
        );
        assert_eq!(
            *log.borrow(),
            vec!["lock", "pkg.toml", "state.json"],
            "the recorded order must be exactly lock, then pkg.toml, then \
             state.json -- with the first two recorded (they ran) and the \
             third also recorded (it ran and failed), and nothing after it"
        );
    }

    /// A failure on the FIRST write must stop before the other two ever run
    /// -- the "all or nothing per package" promise, observed through the
    /// same seam rather than only through `Outcome`.
    #[test]
    fn write_in_order_stops_immediately_when_the_first_write_fails() {
        let log: RefCell<Vec<&str>> = RefCell::new(Vec::new());
        let result = write_in_order(
            WriteLock(|| {
                log.borrow_mut().push("lock");
                anyhow::bail!("lock write failed")
            }),
            WritePkgToml(|| {
                log.borrow_mut().push("pkg.toml");
                Ok(())
            }),
            WriteState(|| {
                log.borrow_mut().push("state.json");
                Ok(())
            }),
        );

        let failure = result.expect_err("the first write's failure must propagate");
        assert!(
            failure.wrote.is_empty(),
            "the write that failed changed nothing, so nothing may be reported as \
             written: {:?}",
            failure.wrote
        );
        assert_eq!(
            *log.borrow(),
            vec!["lock"],
            "pkg.toml and state.json must never have been called"
        );
    }
}
