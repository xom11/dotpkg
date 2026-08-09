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
use std::io::{Read, Write};
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

use crate::model::Name;

/// The bucket's own path for this app at `rev`, trying the cheap guesses
/// before paying for a tree listing. Mirrors `Scoop::stage`'s chain exactly,
/// so `update` records a path `stage` will later find.
pub fn manifest_path(dir: &Path, app: &Name, rev: &str) -> Option<String> {
    let mut tried: Vec<String> = Vec::new();
    for spelling in [app.to_string(), app.key().to_string()] {
        if tried.contains(&spelling) {
            continue;
        }
        tried.push(spelling.clone());
        let candidate = format!("bucket/{spelling}.json");
        if matches!(git_show(dir, rev, &candidate), Ok(Some(_))) {
            return Some(candidate);
        }
    }
    resolve_spelling(dir, rev, app.key()).map(|real| format!("bucket/{real}"))
}

/// What `update` records for one package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Latest {
    pub commit: String,
    pub version: String,
    pub path_in_repo: String,
    /// `git log -1` named a commit whose blob is not the tip's, so the tip was
    /// recorded instead.
    pub fell_back_to_tip: bool,
}

/// Resolve "latest" for one app at `rev`. `Ok(None)` means this bucket does
/// not have the app, which is an ordinary answer during a bucket search.
///
/// **Deliberately without `--full-history`.** Measured: that flag makes this
/// return the merge commit that carried a version rather than the one that
/// produced it. `adopt` needs the flag and this does not.
///
/// The blob comparison against `rev` is what makes "the recorded commit
/// carries the tip's content for this file" true by construction rather than
/// by trusting git's history simplification. It costs one extra `git show`.
pub fn resolve_latest(dir: &Path, app: &Name, rev: &str) -> Result<Option<Latest>> {
    let Some(path_in_repo) = manifest_path(dir, app, rev) else {
        return Ok(None);
    };
    let Some(tip_text) = git_show(dir, rev, &path_in_repo)? else {
        return Ok(None);
    };

    let out = Command::new("git")
        .current_dir(dir)
        .args(["log", "-1", "--format=%H", rev, "--", &path_in_repo])
        .output()
        .with_context(|| format!("cannot run git log in {}", dir.display()))?;
    let per_file = String::from_utf8_lossy(&out.stdout).trim().to_string();

    let (commit, fell_back_to_tip) = match git_show(dir, &per_file, &path_in_repo) {
        Ok(Some(t)) if !per_file.is_empty() && t == tip_text => (per_file, false),
        _ => {
            let sha = Command::new("git")
                .current_dir(dir)
                .args(["rev-parse", rev])
                .output()
                .with_context(|| format!("cannot run git rev-parse in {}", dir.display()))?;
            (
                String::from_utf8_lossy(&sha.stdout).trim().to_string(),
                true,
            )
        }
    };

    let parsed: serde_json::Value = serde_json::from_str(&tip_text)
        .with_context(|| format!("{app}: {path_in_repo} at {rev} is not valid JSON"))?;
    let version = parsed
        .get("version")
        .and_then(|v| v.as_str())
        .with_context(|| format!("{app}: {path_in_repo} at {rev} declares no version"))?
        .to_string();

    Ok(Some(Latest {
        commit,
        version,
        path_in_repo,
        fell_back_to_tip,
    }))
}

/// Every commit touching `path_in_repo`, newest first.
///
/// **`--full-history`, deliberately.** Measured: default history
/// simplification follows one TREESAME parent through a merge, so a version
/// that reached the bucket only on a branch whose change was superseded is
/// invisible -- and `adopt` would report "not in this bucket" about a commit
/// that is a genuine ancestor of HEAD.
///
/// This is the opposite choice from `resolve_latest`, and both are right: see
/// `docs/measurements-2026-08-09-git-resolution.md`, sections B and B'.
pub fn history(dir: &Path, path_in_repo: &str, rev: &str) -> Result<Vec<String>> {
    let out = Command::new("git")
        .current_dir(dir)
        .args([
            "log",
            "--full-history",
            "--format=%H",
            rev,
            "--",
            path_in_repo,
        ])
        .output()
        .with_context(|| format!("cannot run git log in {}", dir.display()))?;
    anyhow::ensure!(
        out.status.success(),
        "git log failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// `<commit>:<path_in_repo>` for every commit, in **one** process.
///
/// Measured on a 400-commit history with the match near the bottom: 2
/// processes and 0.02 s against 395 processes and 3.16 s, identical answer.
/// The ratio is from a synthetic repository; the process count is what
/// transfers, and it transfers to Windows.
///
/// `git cat-file --batch` writes `<sha> <type> <size>\n<contents>\n` per
/// request, in order -- except for a request it cannot resolve, which gets a
/// single `<spec> missing\n` line and **no body**. Keying on the header shape
/// rather than assuming one body per request is what stops a missing path from
/// shifting every later answer onto the wrong commit.
///
/// **Writing and reading run concurrently, on purpose.** `git cat-file
/// --batch` answers each request before reading the next, so its own unread
/// stdout fills the OS pipe buffer once enough responses have queued up --
/// at which point it blocks on its own write and stops reading stdin. Write
/// every spec before ever reading stdout (what a single `wait_with_output()`
/// after the writes would do) and the parent then blocks on *its* write once
/// the stdin pipe also fills; neither side can move again. Measured directly
/// against `git cat-file --batch`, bypassing this function, with ~1.4 KB
/// bodies (this crate's realistic manifest size, matching the range recorded
/// in the measurements doc): the naive write-then-read pattern completes at
/// 2250 requests and hangs from 2300 on, confirmed hung up to 4000 with
/// `timeout`. `history`'s `--full-history` walk can easily hand this more
/// commits than that for a bucket with real traffic. A dedicated thread
/// feeds stdin while stdout (and stderr) are read on other threads, so no
/// pipe can back up while another is stalled.
pub fn blobs(dir: &Path, commits: &[String], path_in_repo: &str) -> Result<Vec<Option<Vec<u8>>>> {
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let mut child = Command::new("git")
        .current_dir(dir)
        .args(["cat-file", "--batch"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot run git cat-file in {}", dir.display()))?;
    let stdin = child.stdin.take().expect("stdin was piped");
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");

    let (data, stderr_bytes) = std::thread::scope(|scope| -> Result<(Vec<u8>, Vec<u8>)> {
        let writer = scope.spawn(move || -> std::io::Result<()> {
            let mut stdin = stdin;
            for c in commits {
                writeln!(stdin, "{c}:{path_in_repo}")?;
            }
            Ok(())
        });
        let stderr_reader = scope.spawn(move || -> std::io::Result<Vec<u8>> {
            let mut stderr = stderr;
            let mut buf = Vec::new();
            stderr.read_to_end(&mut buf)?;
            Ok(buf)
        });

        let mut data = Vec::new();
        stdout
            .read_to_end(&mut data)
            .with_context(|| format!("cannot read git cat-file in {}", dir.display()))?;

        let stderr_bytes = stderr_reader
            .join()
            .map_err(|_| {
                anyhow::anyhow!(
                    "git cat-file stderr reader thread panicked in {}",
                    dir.display()
                )
            })?
            .with_context(|| format!("cannot read git cat-file stderr in {}", dir.display()))?;

        writer
            .join()
            .map_err(|_| {
                anyhow::anyhow!("git cat-file writer thread panicked in {}", dir.display())
            })?
            .with_context(|| format!("cannot feed git cat-file in {}", dir.display()))?;

        Ok((data, stderr_bytes))
    })?;

    // Unlike every other function in this file, `blobs` used to skip this
    // check: `wait_with_output()` was called but `out.status` was never
    // read. A git failure (not a repo, a corrupt repository, a broken git
    // binary) then produced empty stdout with no diagnostic, and the parser
    // below read that as "every commit is missing" -- the same silent
    // mis-attribution the missing-object parsing exists to prevent,
    // arriving by a different route.
    let status = child
        .wait()
        .with_context(|| format!("cannot wait for git cat-file in {}", dir.display()))?;
    anyhow::ensure!(
        status.success(),
        "git cat-file failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&stderr_bytes).trim()
    );

    let mut answers = Vec::with_capacity(commits.len());
    let mut i = 0usize;
    for _ in commits {
        let Some(nl) = data[i..].iter().position(|b| *b == b'\n').map(|p| i + p) else {
            answers.push(None);
            continue;
        };
        let header = String::from_utf8_lossy(&data[i..nl]).into_owned();
        let fields: Vec<&str> = header.split_whitespace().collect();
        // "<spec> missing" -- two fields, no body. Anything that is not a
        // three-field "<sha> <type> <size>" header is treated the same way:
        // no body to consume, so the next answer starts on the next line.
        let size = match fields.as_slice() {
            [_, _, size] => size.parse::<usize>().ok(),
            _ => None,
        };
        match size {
            // `+ 1` beyond the body itself: every response, including the
            // last, is followed by a bare newline git adds. Requiring it to
            // be present too (rather than just the body) keeps `i` at or
            // below `data.len()` in every case, so the next loop iteration's
            // `data[i..]` is a valid (possibly empty) slice instead of a
            // panic on a stream truncated by, say, the failure above.
            Some(n) if nl + 1 + n < data.len() => {
                answers.push(Some(data[nl + 1..nl + 1 + n].to_vec()));
                i = nl + 1 + n + 1;
            }
            _ => {
                answers.push(None);
                i = nl + 1;
            }
        }
    }
    Ok(answers)
}
