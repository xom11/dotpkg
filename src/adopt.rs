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
