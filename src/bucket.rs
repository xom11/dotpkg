//! Every git invocation in the crate.
//!
//! Until Phase 3 these lived inline in `backend::scoop`, which was fine while
//! `stage` was the only caller. `update` and `adopt` are two more, and a
//! third copy of "run git, decide what its silence meant" is how the three
//! drift apart.
//!
//! Nothing here writes to a bucket's working tree or moves a branch. A bucket
//! is scoop's directory; dotpkg reads it and fetches into it, and that is all.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// True when git exited 0. Used where the question is yes/no and the output
/// does not matter.
pub fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `Ok(None)` when the path is absent from that revision; `Err` only when git
/// itself could not be run.
pub fn git_show(dir: &Path, rev: &str, path_in_repo: &str) -> Result<Option<String>> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["show", &format!("{rev}:{path_in_repo}")])
        .output()
        .with_context(|| format!("cannot run git in {}", dir.display()))?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
}

/// The bucket's own filename for this app, found case-insensitively, **at the
/// given revision**.
///
/// Measured: `git ls-tree` at an old commit returns that commit's spelling
/// (`bucket/Tool.json`) while HEAD has another (`bucket/tool.json`). Listing
/// HEAD instead would miss a historical name -- which is what the Phase 2b-1
/// rehearsal script did.
pub fn resolve_spelling(dir: &Path, rev: &str, app_key: &str) -> Option<String> {
    let listing = Command::new("git")
        .current_dir(dir)
        .args(["ls-tree", "--name-only", rev, "bucket/"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let wanted = format!("{app_key}.json");
    listing
        .lines()
        .map(|l| l.rsplit('/').next().unwrap_or(l))
        .find(|f| f.to_ascii_lowercase() == wanted)
        .map(str::to_string)
}

/// Measured: `adopt`'s walk over a shallow clone finds nothing, and git prints
/// nothing to distinguish that from "this version was never here". `scoop
/// bucket add` clones in full, but a bucket the user cloned by hand is not
/// covered by that measurement.
pub fn is_shallow(dir: &Path) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "--is-shallow-repository"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

/// The revision resolution reads from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tip {
    /// `origin/main`, or `HEAD` when there is no upstream.
    pub rev: String,
    /// Why this is not a remote-tracking ref, when it is not. `None` means the
    /// answer is as current as the last fetch made it.
    pub stale: Option<String>,
}

/// Where to resolve "latest" from, without moving anything.
///
/// The upstream of the bucket's current branch, so a `fetch` is visible
/// without a `pull`: the fetched objects are reachable from `refs/remotes/`,
/// which is all `git show` needs at `apply` time.
pub fn tip(dir: &Path) -> Tip {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "--abbrev-ref", "@{u}"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let rev = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if rev.is_empty() {
                Tip {
                    rev: "HEAD".into(),
                    stale: Some("the bucket's branch names no upstream".into()),
                }
            } else {
                Tip { rev, stale: None }
            }
        }
        _ => Tip {
            rev: "HEAD".into(),
            stale: Some("the bucket's branch has no upstream to fetch from".into()),
        },
    }
}

/// `git fetch`. Never `pull`, never a checkout: the branch and working tree
/// belong to scoop.
pub fn fetch(dir: &Path) -> Result<()> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["fetch", "--quiet"])
        .output()
        .with_context(|| format!("cannot run git fetch in {}", dir.display()))?;
    anyhow::ensure!(
        out.status.success(),
        "git fetch failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(())
}
