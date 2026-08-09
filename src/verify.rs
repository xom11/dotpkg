//! Did the mutation actually happen?
//!
//! Measured on a14, scoop 0.5.3: `scoop` exits **0** for a hash mismatch, a
//! dead URL, an install over a nonexistent manifest path, and an uninstall of
//! an app that is not installed. Only an unknown subcommand exits 1 — and this
//! is not the `.cmd` shim: `scoop.ps1` invoked directly reports
//! `$LASTEXITCODE=0` too.
//!
//! So this module is not a second safety net. It is the only signal there is.

use crate::model::Name;
use std::path::{Path, PathBuf};

/// What the executor asked scoop to make true.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    /// After an uninstall.
    Absent,
    /// After an install: the app's manifest must be the one that was staged.
    Present { staged: PathBuf },
}

/// How the disk disagrees. An enum rather than a string, because the retry
/// gate has to tell "nothing there" apart from "half-installed": retrying over
/// a half-install gets `WARN … is already installed`, exit 0, and no change —
/// manufacturing exactly the silent success this module exists to catch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disagreement {
    NotInstalled,
    HalfInstalled {
        leftover: PathBuf,
    },
    /// The installed manifest is a *different* manifest.
    ///
    /// There is deliberately no sibling variant for a line-ending-only
    /// difference: `verdict` accepts those, because scoop rewrites line
    /// endings when it copies the staged file and every successful install
    /// on Windows would otherwise be reported as a failure. See `verdict`.
    ContentDiffers,
    StillPresent {
        leftover: PathBuf,
    },
    Unreadable(String),
}

impl std::fmt::Display for Disagreement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Disagreement::NotInstalled => {
                write!(f, "the app directory is not there at all")
            }
            Disagreement::HalfInstalled { leftover } => write!(
                f,
                "a partial install is left at {} -- there is no current/manifest.json",
                leftover.display()
            ),
            Disagreement::ContentDiffers => {
                write!(f, "the installed manifest is not the one that was staged")
            }
            Disagreement::StillPresent { leftover } => {
                write!(f, "it is still on disk at {}", leftover.display())
            }
            Disagreement::Unreadable(why) => write!(f, "could not look: {why}"),
        }
    }
}

/// Find `<root>/apps/<app>` by folding case, the way the filesystem that wrote
/// it does.
///
/// Not `join(app.key())`: scoop names the directory after the **bucket's**
/// spelling, Windows resolves `apps/tool` to `Tool` and macOS does not, so a
/// path join makes every fixture on this developer's machine mean something
/// different from production. Reproduced while prototyping.
fn app_dir(root: &Path, app: &Name) -> Result<Option<PathBuf>, String> {
    let apps = root.join("apps");
    let entries = match std::fs::read_dir(&apps) {
        Ok(e) => e,
        // A machine with no scoop is a valid state, and so is one mid-setup.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", apps.display())),
    };
    for entry in entries {
        let entry =
            entry.map_err(|e| format!("cannot read an entry of {}: {e}", apps.display()))?;
        // Same rule as `Scoop::scan` (src/backend/scoop.rs): a stray file
        // sharing an app's name is not an app directory. Deliberate, not an
        // oversight -- a file cannot hold `current/manifest.json`, so treating
        // it as installed would just relabel the next check's failure.
        if !entry.path().is_dir() {
            continue;
        }
        if Name::new(entry.file_name().to_string_lossy().to_string()) == *app {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

/// Collapse CRLF and drop trailing newlines.
///
/// Two files equal under this are the same JSON, so `verdict` treats them as
/// a match. The class is slightly wider than "line endings" -- `{"a":1}` vs.
/// `{"a":1}\n\n\n` lands here too -- and that is still safe: neither
/// transformation can change a url or a hash, which is what the byte
/// comparison exists to protect.
///
/// `adopt` is the second consumer as of Phase 3, and needs exactly this
/// function rather than one of its own: measured, comparing an installed
/// manifest against a bucket blob raw matches nothing at all, because scoop
/// rewrites line endings when it copies the file into `apps/<app>/current`.
pub(crate) fn normalise(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') {
            out.push(b'\n');
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    while out.last() == Some(&b'\n') {
        out.pop();
    }
    out
}

/// Compare what is on disk against what was asked for. No subprocess, no
/// network, no exit code.
pub fn verdict(root: &Path, app: &Name, want: &Expected) -> Result<(), Disagreement> {
    let dir = app_dir(root, app).map_err(Disagreement::Unreadable)?;
    match want {
        Expected::Absent => match dir {
            None => Ok(()),
            Some(leftover) => Err(Disagreement::StillPresent { leftover }),
        },
        Expected::Present { staged } => {
            let Some(dir) = dir else {
                return Err(Disagreement::NotInstalled);
            };
            let observed = dir.join("current").join("manifest.json");
            let got = match std::fs::read(&observed) {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(Disagreement::HalfInstalled { leftover: dir })
                }
                Err(e) => {
                    return Err(Disagreement::Unreadable(format!(
                        "cannot read {}: {e}",
                        observed.display()
                    )))
                }
            };
            let want_bytes = std::fs::read(staged).map_err(|e| {
                Disagreement::Unreadable(format!("cannot read {}: {e}", staged.display()))
            })?;
            if got == want_bytes {
                Ok(())
            } else if normalise(&got) == normalise(&want_bytes) {
                // Accepted, not reported. Measured on the dogfood machine:
                // scoop rewrites the manifest's line endings when it copies
                // the staged file into `apps/<app>/current`, so an exact byte
                // comparison fails on EVERY successful install on Windows.
                // Treating that as a failure made `apply` report every
                // upgrade it had just correctly performed as
                // "install did not happen", and left `state.json` empty, so
                // nothing dotpkg installed was ever recorded as owned.
                //
                // This is safe because the two files are the same JSON: the
                // check exists to catch a *different* manifest -- the
                // same-version content swap a lock naming a branch produces
                // -- and a difference only in line endings and trailing
                // newlines cannot change a url or a hash.
                Ok(())
            } else {
                Err(Disagreement::ContentDiffers)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const BODY_A: &str = r#"{"version":"1.0.0","url":"https://good/v1.zip","hash":"aaaa"}"#;
    const BODY_B: &str = r#"{"version":"1.0.0","url":"https://evil/v1.zip","hash":"bbbb"}"#;

    struct Tree(tempfile::TempDir);
    impl Tree {
        fn new() -> Tree {
            Tree(tempfile::tempdir().unwrap())
        }
        fn root(&self) -> &Path {
            self.0.path()
        }
        fn stage(&self, app: &str, version: &str, body: &str) -> PathBuf {
            let d = self.root().join("stage").join(app).join(version);
            std::fs::create_dir_all(&d).unwrap();
            let p = d.join(format!("{app}.json"));
            std::fs::write(&p, body).unwrap();
            p
        }
        /// A clean install: `current/manifest.json`, byte-identical to the
        /// staged file. Measured on a14 -- scoop copies the manifest verbatim.
        fn install(&self, dir_name: &str, body: &str) {
            let cur = self.root().join("apps").join(dir_name).join("current");
            std::fs::create_dir_all(&cur).unwrap();
            std::fs::write(cur.join("manifest.json"), body).unwrap();
        }
        /// The measured residue of a failed install: `apps/<app>/<version>/`
        /// holding only the archive, no `current`, no manifest.
        fn half_install(&self, dir_name: &str, version: &str) {
            let d = self.root().join("apps").join(dir_name).join(version);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("thing.zip"), b"PK\x03\x04").unwrap();
        }
        fn empty_apps(&self) {
            std::fs::create_dir_all(self.root().join("apps")).unwrap();
        }
    }

    #[test]
    fn a_clean_install_agrees() {
        let t = Tree::new();
        let staged = t.stage("fzf", "1.0.0", BODY_A);
        t.install("fzf", BODY_A);
        assert_eq!(
            verdict(t.root(), &Name::new("fzf"), &Expected::Present { staged }),
            Ok(())
        );
    }

    #[test]
    fn a_same_version_content_swap_is_caught_where_a_version_check_would_not_be() {
        // The `commit = "main"` hole: both manifests say version 1.0.0 and
        // only the url and hash differ. This is why the comparison is bytes.
        let t = Tree::new();
        let staged = t.stage("tool", "1.0.0", BODY_A);
        t.install("tool", BODY_B);
        assert_eq!(
            verdict(t.root(), &Name::new("tool"), &Expected::Present { staged }),
            Err(Disagreement::ContentDiffers)
        );
    }

    #[test]
    fn the_silent_no_op_install_scoop_was_measured_doing_is_caught() {
        let t = Tree::new();
        let staged = t.stage("fzf", "0.74.2", r#"{"version":"0.74.2"}"#);
        t.install("fzf", r#"{"version":"0.74.1"}"#);
        assert_eq!(
            verdict(t.root(), &Name::new("fzf"), &Expected::Present { staged }),
            Err(Disagreement::ContentDiffers)
        );
    }

    #[test]
    fn the_measured_failed_install_residue_is_its_own_diagnosis() {
        let t = Tree::new();
        let staged = t.stage("badhash", "0.74.1", BODY_A);
        t.half_install("badhash", "0.74.1");
        assert_eq!(
            verdict(
                t.root(),
                &Name::new("badhash"),
                &Expected::Present { staged }
            ),
            Err(Disagreement::HalfInstalled {
                leftover: t.root().join("apps").join("badhash")
            })
        );
    }

    #[test]
    fn nothing_at_all_is_not_installed() {
        let t = Tree::new();
        let staged = t.stage("fzf", "1.0.0", BODY_A);
        t.empty_apps();
        assert_eq!(
            verdict(t.root(), &Name::new("fzf"), &Expected::Present { staged }),
            Err(Disagreement::NotInstalled)
        );
    }

    #[test]
    fn absent_means_absent_and_a_leftover_is_named() {
        let t = Tree::new();
        t.empty_apps();
        assert_eq!(
            verdict(t.root(), &Name::new("fzf"), &Expected::Absent),
            Ok(())
        );
        t.half_install("fzf", "1.0.0");
        assert_eq!(
            verdict(t.root(), &Name::new("fzf"), &Expected::Absent),
            Err(Disagreement::StillPresent {
                leftover: t.root().join("apps").join("fzf")
            })
        );
    }

    #[test]
    fn the_app_directory_is_found_by_folding_case_not_by_the_platforms_rules() {
        // scoop names the directory after the BUCKET's spelling. Windows finds
        // `Tool` when asked for `tool`; macOS and Linux do not, so a path join
        // would make this fixture diverge from production. Found by a real
        // failure while prototyping.
        //
        // The `Present` half below is NOT a reliable control for that: on the
        // default (case-insensitive) APFS volume this developer's macOS uses,
        // and on Windows, `std::fs::read` on `apps/tool/current/manifest.json`
        // succeeds even under a path-join mutant, because the filesystem
        // itself folds the case before the join-based path ever gets there.
        // It only has teeth on the Linux CI leg, and even there a mutant
        // `app_dir` that returns `Some` unconditionally still slips through
        // it. The `Absent` half is the one that discriminates everywhere:
        // `PathBuf` equality is string equality, and `Name::key()` folds to
        // lowercase, so a path-join mutant produces `apps/tool` while the
        // real implementation returns the on-disk spelling `apps/Tool` --
        // an `assert_eq!` on the full leftover catches that on any
        // filesystem, case-sensitive or not. `assert!(matches!(.., { .. }))`
        // does not: it throws away the field carrying the evidence and
        // passes vacuously against a mutant that always returns `Some`.
        let t = Tree::new();
        let staged = t.stage("tool", "1.0.0", BODY_A);
        t.install("Tool", BODY_A);
        assert_eq!(
            verdict(t.root(), &Name::new("tool"), &Expected::Present { staged }),
            Ok(())
        );
        assert_eq!(
            verdict(t.root(), &Name::new("TOOL"), &Expected::Absent),
            Err(Disagreement::StillPresent {
                leftover: t.root().join("apps").join("Tool")
            })
        );
    }

    #[test]
    fn a_difference_only_in_line_endings_is_accepted_because_scoop_rewrites_them() {
        // Measured on the dogfood machine: scoop rewrites line endings when
        // it copies the staged manifest into `apps/<app>/current`, so this is
        // what EVERY successful install on Windows looks like. Reporting it
        // as a failure made `apply` call every upgrade it had just performed
        // correctly "install did not happen", and left `state.json` empty so
        // nothing was ever recorded as owned.
        let t = Tree::new();
        let staged = t.stage("tool", "1.0.0", "{\n  \"version\": \"1.0.0\"\n}");
        t.install("tool", "{\r\n  \"version\": \"1.0.0\"\r\n}");
        assert_eq!(
            verdict(t.root(), &Name::new("tool"), &Expected::Present { staged }),
            Ok(())
        );
    }

    #[test]
    fn a_trailing_newline_difference_is_also_accepted() {
        let t = Tree::new();
        let staged = t.stage("tool", "1.0.0", "{\"version\":\"1.0.0\"}");
        t.install("tool", "{\"version\":\"1.0.0\"}\n\n");
        assert_eq!(
            verdict(t.root(), &Name::new("tool"), &Expected::Present { staged }),
            Ok(())
        );
    }

    #[test]
    fn accepting_line_endings_does_not_accept_a_content_swap() {
        // The guard that must survive the fix above: two manifests carrying
        // the same version but a different url and hash -- what a lock naming
        // a branch instead of a commit produces -- must still be caught, even
        // when their line endings also differ.
        let t = Tree::new();
        let staged = t.stage("tool", "1.0.0", BODY_A);
        t.install("tool", &BODY_B.replace('\n', "\r\n"));
        assert_eq!(
            verdict(t.root(), &Name::new("tool"), &Expected::Present { staged }),
            Err(Disagreement::ContentDiffers)
        );
    }

    #[test]
    fn a_machine_with_no_apps_directory_is_absent_not_an_error() {
        let t = Tree::new();
        assert_eq!(
            verdict(t.root(), &Name::new("fzf"), &Expected::Absent),
            Ok(())
        );
    }

    #[test]
    fn an_apps_directory_that_cannot_be_read_is_reported_not_treated_as_absent() {
        // A regular file where `apps/` should be a directory: `read_dir`
        // fails with `NotADirectory` on macOS and Linux, and Windows maps its
        // equivalent the same way -- portable without `#[cfg(unix)]`. This
        // must surface as `Unreadable`, never as "nothing there": an
        // uninstall verifying itself against an error it never looked past
        // is exactly the silent success this module exists to catch.
        let t = Tree::new();
        std::fs::write(t.root().join("apps"), b"not a directory").unwrap();
        match verdict(t.root(), &Name::new("fzf"), &Expected::Absent) {
            Err(Disagreement::Unreadable(msg)) => {
                let apps = t.root().join("apps");
                assert!(
                    msg.contains(&apps.display().to_string()),
                    "message {msg:?} does not name {}",
                    apps.display()
                );
            }
            other => panic!("expected Unreadable, got {other:?}"),
        }
    }

    #[test]
    fn every_disagreement_says_something_a_user_can_act_on() {
        // `s.len() > 10` never fires: the shortest real message is 39
        // characters, and both path-carrying variants could drop
        // `leftover.display()` entirely and stay under no bound at all. Assert
        // the actionable content instead -- the same tightening as commit
        // 42c8b32, a third instance of the shape that review named twice.
        for d in [Disagreement::NotInstalled, Disagreement::ContentDiffers] {
            let s = d.to_string();
            assert!(!s.trim().is_empty(), "{d:?} renders empty");
        }
        for d in [
            Disagreement::HalfInstalled {
                leftover: PathBuf::from("/a/b"),
            },
            Disagreement::StillPresent {
                leftover: PathBuf::from("/a/b"),
            },
        ] {
            let s = d.to_string();
            assert!(
                s.contains("/a/b"),
                "{d:?} renders as {s:?} without the path"
            );
        }
        let s = Disagreement::Unreadable("boom".into()).to_string();
        assert!(s.contains("boom"), "renders as {s:?} without the reason");
    }
}
