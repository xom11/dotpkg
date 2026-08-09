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

    Ok(parse_batch(&data, commits.len()))
}

/// Split `git cat-file --batch`'s response stream into one answer per request.
///
/// Separate from `blobs` so the shapes that matter can be exercised without a
/// repository: a well-formed stream, a `missing` reply in the middle (which
/// carries no body and must not shift every later answer onto the wrong
/// commit), and a stream that ends early. That last one is why the length
/// check below is strict: `blobs` only reaches this after `git cat-file`
/// exited 0, so a truncated stream should be impossible -- but "should be
/// impossible" is exactly the reasoning that made the un-drained-pipe deadlock
/// and the unchecked exit status ship, and the cost of being wrong here is an
/// index panic inside an unattended `adopt`.
fn parse_batch(data: &[u8], count: usize) -> Vec<Option<Vec<u8>>> {
    let mut answers = Vec::with_capacity(count);
    let mut i = 0usize;
    for _ in 0..count {
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
    answers
}

/// Which bucket a package comes from, or why that cannot be decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BucketChoice {
    Chosen {
        name: Name,
        dir: std::path::PathBuf,
        tip: Tip,
    },
    /// More than one declared bucket carries it. Never resolved by declaration
    /// order: reordering `buckets` would silently move a pin.
    Ambiguous { candidates: Vec<Name> },
    /// The bucket the lock or `[scoop.opts]` names is declared, but is not on
    /// this machine, so **nothing** about this package could be read at all.
    ///
    /// A separate variant rather than a `NotFound` with an empty `searched`,
    /// because it is a different fact: `NotFound` means a search happened and
    /// came back empty, and this one means no search was possible. Reported as
    /// `NotFound` it read as `bucket <name> has no manifest for it` -- a flat
    /// lie about a bucket that has the manifest and simply is not cloned.
    NotCloned { name: Name, dir: std::path::PathBuf },
    /// The search ran and found nothing.
    ///
    /// `searched` is the buckets that were really opened and looked in;
    /// `missing` is the declared buckets that are not on this machine and so
    /// were never looked at. The two lists are separate because reporting the
    /// full declared list as "searched" is a false line, and it is the false
    /// line a fresh machine gets for **every** package.
    NotFound {
        searched: Vec<Name>,
        missing: Vec<Name>,
    },
}

/// Why a search that looked in `searched` and could not look in `missing`
/// found nothing, as one sentence.
///
/// `subject` is what was being looked for, spelled as the caller's sentence
/// needs it: `update` says "it" (its lines are already keyed by package name),
/// `adopt` names the package. Shared rather than written twice because the two
/// call sites are the same fact, and the version of this that *was* written
/// twice is how "searched: main, extras" got printed about a machine that had
/// only `main`.
pub fn not_found_why(subject: &str, searched: &[Name], missing: &[Name]) -> String {
    let list = |ns: &[Name]| {
        ns.iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match (searched.is_empty(), missing.is_empty()) {
        (false, true) => format!(
            "no declared bucket has {subject} (searched: {})",
            list(searched)
        ),
        (false, false) => format!(
            "no declared bucket on this machine has {subject} (searched: {}). \
             Declared but not on this machine, so not searched: {} -- \
             `dotpkg apply --clone-missing-buckets` clones them.",
            list(searched),
            list(missing)
        ),
        (true, false) => format!(
            "nothing was searched for {subject}: every declared bucket ({}) is \
             missing from this machine -- `dotpkg apply --clone-missing-buckets` \
             clones them.",
            list(missing)
        ),
        // `pkg.toml` declares packages but no buckets at all. Reachable, and
        // "searched: " with an empty list is the least useful thing to print
        // about it.
        (true, true) => {
            format!("pkg.toml declares no buckets, so there was nowhere to look for {subject}")
        }
    }
}

/// Why a bucket that was named rather than searched for could not be opened.
///
/// Deliberately does not say *what* named it: the lock, `[scoop.opts]` and (in
/// `adopt`) `install.json` all land here, and naming the wrong one would be
/// exactly the kind of confidently-wrong line this fix exists to remove.
/// Shared by `update` and `adopt` for the same reason as `not_found_why`.
pub fn not_cloned_why(subject: &str, name: &Name, dir: &Path) -> String {
    format!(
        "{subject} resolves to bucket {name}, which is declared but is not present \
         at {} -- `dotpkg apply --clone-missing-buckets` clones it.",
        dir.display()
    )
}

/// Decide which declared bucket a package comes from.
///
/// Precedence, strongest first: the existing lock entry (so `update`
/// re-resolves a version and never a provenance), then `[scoop.opts] <pkg>
/// = { bucket = "..." }`, then a search of every declared bucket.
pub fn choose_bucket(
    scoop_root: &Path,
    declared: &crate::config::Config,
    app: &Name,
    already_locked: Option<&str>,
) -> BucketChoice {
    let open = |name: &Name| -> BucketChoice {
        let dir = scoop_root.join("buckets").join(name.key());
        BucketChoice::Chosen {
            name: name.clone(),
            tip: tip(&dir),
            dir,
        }
    };

    let declared_names: Vec<Name> = declared
        .scoop
        .buckets
        .iter()
        .map(|b| b.name.clone())
        .collect();

    if let Some(stated) = [
        already_locked.map(Name::new),
        declared
            .scoop
            .opts
            .get(app)
            .and_then(|o| o.bucket.as_deref())
            .map(Name::new),
    ]
    .into_iter()
    .flatten()
    .next()
    {
        if !declared_names.contains(&stated) {
            return BucketChoice::NotFound {
                searched: vec![stated],
                missing: Vec::new(),
            };
        }
        // The same `.git` check the search loop below makes, which this branch
        // did not have. Without it an absent bucket is opened as if it were
        // there: `tip()` falls to its `_` arm, every `git_show` fails, and
        // `resolve_latest` returns `Ok(None)` -- which `update` renders as
        // "bucket <name> has no manifest for it" about a bucket that has the
        // manifest and simply is not cloned.
        let dir = scoop_root.join("buckets").join(stated.key());
        return if dir.join(".git").exists() {
            open(&stated)
        } else {
            BucketChoice::NotCloned { name: stated, dir }
        };
    }

    let mut found = Vec::new();
    let mut searched = Vec::new();
    let mut missing = Vec::new();
    for name in &declared_names {
        let dir = scoop_root.join("buckets").join(name.key());
        // Recorded, not discarded. A declared bucket that is not on disk is
        // the ordinary case on a fresh machine and on a `pkg.toml` that just
        // grew a bucket line, and reporting it inside `searched` told every
        // user that a bucket which was never opened had been looked in.
        if !dir.join(".git").exists() {
            missing.push(name.clone());
            continue;
        }
        searched.push(name.clone());
        let at = tip(&dir);
        if manifest_path(&dir, app, &at.rev).is_some() {
            found.push(name.clone());
        }
    }
    match found.len() {
        0 => BucketChoice::NotFound { searched, missing },
        1 => open(&found[0]),
        _ => BucketChoice::Ambiguous { candidates: found },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One `<sha> blob <n>\n<body>\n` response, as `git cat-file --batch`
    /// writes it.
    fn response(sha: &str, body: &str) -> Vec<u8> {
        format!("{sha} blob {}\n{body}\n", body.len()).into_bytes()
    }

    #[test]
    fn a_well_formed_stream_yields_one_body_per_request_in_order() {
        let mut data = response("aaa", "hello");
        data.extend(response("bbb", "bye"));

        assert_eq!(
            parse_batch(&data, 2),
            vec![Some(b"hello".to_vec()), Some(b"bye".to_vec())]
        );
    }

    #[test]
    fn a_missing_reply_has_no_body_and_does_not_shift_the_answers_after_it() {
        // The property the whole header-keyed parse exists for. A parser that
        // assumed one body per request would consume the NEXT commit's header
        // as this one's body and hand every later answer back against the
        // wrong commit -- `adopt` would then pin a commit whose manifest is
        // not the one that matched.
        let mut data = response("aaa", "hello");
        data.extend_from_slice(b"deadbeef missing\n");
        data.extend(response("bbb", "bye"));

        assert_eq!(
            parse_batch(&data, 3),
            vec![Some(b"hello".to_vec()), None, Some(b"bye".to_vec())],
            "the answer after a missing object must still be its own commit's"
        );
    }

    #[test]
    fn a_stream_that_ends_at_the_body_is_none_because_the_trailing_newline_is_missing() {
        // `git cat-file --batch` always follows a body with a bare newline, so
        // a stream ending exactly at the body end is truncated output, not a
        // complete answer. The check is strict (`<`, not `<=`) precisely so
        // this case is refused rather than reported as a blob -- and so `i`
        // stays at or below `data.len()` for the next iteration.
        let data = b"aaa blob 5\nhello".to_vec();
        assert_eq!(parse_batch(&data, 1), vec![None]);
    }

    #[test]
    fn a_body_shorter_than_its_header_claims_is_none_rather_than_a_panic() {
        // The bounds check is what keeps this from being an index panic
        // inside an unattended `adopt`.
        let data = b"aaa blob 100\nhello\n".to_vec();
        assert_eq!(parse_batch(&data, 1), vec![None]);
    }

    #[test]
    fn a_truncated_stream_still_returns_one_answer_per_request() {
        // Fewer responses than requests: every request must still get an
        // answer, or `commits.iter().zip(blobs.iter())` in `adopt` would
        // silently drop the tail of the history.
        let data = response("aaa", "hello");
        assert_eq!(
            parse_batch(&data, 3),
            vec![Some(b"hello".to_vec()), None, None]
        );
    }
}
