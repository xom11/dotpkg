//! winget's mutating argv, and the `WingetMutator` seam it runs through.
//!
//! Scoop's equivalent seam is `execute::Mutator`; this module is the winget
//! analogue, split into its own file rather than added to
//! `src/backend/winget.rs` because mutating is a different responsibility,
//! with different fixtures, from that file's scanning and resolving.
//!
//! **The central rule: winget's exit code is never the verdict.** Every
//! write verb reports through its exit code, like winget's read verbs and
//! unlike scoop's write verbs -- but a nonzero exit does not mean the
//! machine is wrong, and a zero exit does not always mean it moved.
//! `install --version <pin>` against a package already at `<pin>` exits
//! nonzero (`NO_AVAILABLE_UPGRADE`) and changes nothing, and the identical
//! code is returned when winget declines a downgrade instead. A caller must
//! re-scan the package after every mutation (`list_one_argv` is that
//! re-scan's argv) and judge from what is actually installed, never from
//! `CmdOut::code` alone. See
//! `docs/measurements-2026-08-10-winget-write-path.md`'s headline, and
//! `NO_AVAILABLE_UPGRADE`'s own doc comment below.
//!
//! **Why `set`, and not `install` plus `upgrade`.** Measured:
//! `winget install --version <pin>` performs an upgrade directly when the
//! package is already installed (0.24.1 -> 0.26.1, exit 0), and `winget
//! upgrade` goes to the *newest* version in the index rather than to a
//! requested one -- it took the guinea pig from 0.26.1 to 0.26.2 while the
//! pin was neither. A pinning tool cannot use a verb whose target is
//! "latest". One method, one measured argv, covers both a fresh install and
//! an upgrade; a downgrade is refused by winget itself (`NO_AVAILABLE_UPGRADE`
//! again), not decided by this crate.
//!
//! No test may spawn `winget.exe`. Every test goes through `WingetMutator`,
//! faked by `tests/common/fake_winget_mutator.rs` -- the sibling rule to
//! `WingetCmd`'s own seam in `src/backend/winget.rs`.

use crate::backend::winget::{CmdError, CmdOut};
use crate::model::Name;
use std::process::{Command, Stdio};

/// `winget install -e --id ducaale.xh --version <the version already
/// installed>` (and the same call with an *older* version than what is
/// installed) exits this code on a14 -- `0x8A15002B`, printing `Found an
/// existing package already installed. Trying to upgrade the installed
/// package...` followed by `No available upgrade found.`
/// (`docs/measurements-2026-08-10-winget-write-path.md` §2). `winget
/// upgrade` against an already-newest package exits it too (§9).
///
/// **Covers a success and a failure at once, the same shape
/// `NO_APPLICATIONS_FOUND`'s own doc comment already warns about for `list
/// -s msstore`.** winget returns this identical code both when the
/// installed package is already at exactly the version `set` asked for
/// (nothing to do -- a success) and when `set` asked for a version older
/// than what is installed (winget declines the downgrade -- a failure).
/// The exit code alone cannot tell these two apart; only a re-scan of the
/// installed version against the requested one can, which is why
/// `set`'s caller must never treat this code as "failed" on its own.
pub const NO_AVAILABLE_UPGRADE: i32 = -1978335189; // 0x8A15002B

/// `winget install --no-upgrade` against a package that is already
/// installed exits this code on a14 -- `0x8A150061`, printing `A package
/// version is already installed. Installation cancelled.`
/// (`docs/measurements-2026-08-10-winget-write-path.md` §9).
///
/// **Not produced by `set_argv`.** `set_argv` never passes `--no-upgrade`,
/// so nothing in this crate can trigger this code yet; it is recorded
/// alongside the two codes that are, so a later caller that does have a
/// reason to pass `--no-upgrade` does not have to re-measure it.
pub const ALREADY_INSTALLED: i32 = -1978335135; // 0x8A150061

/// `winget uninstall -e --id ducaale.xh --disable-interactivity`, run from a
/// process elevated to administrator, against a package installed for the
/// *user* scope, exits this code on a14 -- `0x8A15007D`, printing `The
/// package installed for user scope cannot be uninstalled when running with
/// administrator privileges.` (`docs/measurements-2026-08-10-winget-write-path.md`
/// §5). The identical package and argv, run de-elevated in the same
/// session, exits `0`. This is a property of the calling process's
/// integrity level, not of the package or the argv -- a scheduled `apply`
/// running at high integrity can install a user-scope package and then be
/// structurally unable to remove it, forever, until it is run de-elevated.
pub const CANNOT_UNINSTALL_ELEVATED: i32 = -1978335107; // 0x8A15007D

/// The exact argv for pinning `id` to `version`: a fresh install, an
/// upgrade, or a declined downgrade, decided by winget itself rather than
/// by this crate. See this module's own doc comment for why one call
/// covers both directions instead of separate `install`/`upgrade` argvs.
///
/// Measured (`docs/measurements-2026-08-10-winget-write-path.md` §1-§2):
/// `install -e --id <id> --version <version> --silent
/// --accept-package-agreements --accept-source-agreements
/// --disable-interactivity` installs exactly `<version>` on a fresh install
/// (hash-verified: `Successfully verified installer hash`), and on an
/// already-installed package silently reinterprets itself as an upgrade
/// toward `<version>` -- moving the package there when it is newer, and
/// refusing with `NO_AVAILABLE_UPGRADE` otherwise (already there, or a
/// downgrade). `--disable-interactivity` matters specifically because a
/// scheduled, unattended `apply` has no operator present to answer a
/// prompt.
///
/// `id.to_string()`, never `id.key()`. `--exact` (`-e`) is what makes
/// `--id` case-sensitive on the write verbs, same as the read verbs:
/// `install -e --id SHARKDP.HYPERFINE` reaches `NO_APPLICATIONS_FOUND` for
/// a package that exists, where the correctly-cased call reaches
/// `NO_VERSION_FOUND` instead (§6). `Name::key()` is the ASCII-folded form
/// every other comparison in this crate uses; putting it on the wire here
/// means "not found" for a package that is there. The lock holds the
/// canonical spelling winget itself echoed back, which is why `-e` is safe
/// here at all.
pub fn set_argv(id: &Name, version: &str) -> Vec<String> {
    vec![
        "install".to_string(),
        "-e".to_string(),
        "--id".to_string(),
        id.to_string(),
        "--version".to_string(),
        version.to_string(),
        "--silent".to_string(),
        "--accept-package-agreements".to_string(),
        "--accept-source-agreements".to_string(),
        "--disable-interactivity".to_string(),
    ]
}

/// The exact argv for removing an installed package, refused unless
/// `version` matches what is actually installed.
///
/// Measured (`docs/measurements-2026-08-10-winget-write-path.md` §8):
/// `uninstall --version` resolves against what is INSTALLED, not the
/// index, and refuses with `NO_VERSION_FOUND` rather than removing a
/// different version -- passing the version this crate believes is
/// installed is what makes a removal fail closed instead of silently
/// removing the wrong one. `--disable-interactivity` and
/// `--accept-source-agreements` are the same unattended-run flags `set_argv`
/// carries, needed here for the same reason.
///
/// `id.to_string()`, never `id.key()` -- same reasoning as `set_argv`.
pub fn remove_argv(id: &Name, version: &str) -> Vec<String> {
    vec![
        "uninstall".to_string(),
        "-e".to_string(),
        "--id".to_string(),
        id.to_string(),
        "--version".to_string(),
        version.to_string(),
        "--disable-interactivity".to_string(),
        "--accept-source-agreements".to_string(),
    ]
}

/// The exact argv for the single-package re-scan a step's verdict is built
/// from, once a later task writes the code that judges it.
///
/// **`-e`/`--exact` here, but deliberately not in `resolve_latest` or
/// `resolve_installed` (`src/backend/winget.rs:696`, `:772`) -- the opposite
/// choice, for the same underlying measured reason, not the same choice for
/// a shared one.** Those two resolvers run *before* the canonical spelling
/// is known: they must omit `--exact` so winget folds case on the way in,
/// and they read the canonical id back out of the `Found <name> [<Id>]`
/// line that self-verifying call produces. `list_one_argv` runs *after*
/// resolution, against the canonical spelling `pkg.lock` already holds --
/// the exact spelling winget itself echoed back.
///
/// Copying `-e` into a resolver call is a measured hazard: `--exact`
/// together with a folded or wrong-case spelling returns
/// `NO_APPLICATIONS_FOUND` for a package that exists (`show -e --id
/// git.git` -> `0x8A150014`, `docs/measurements-2026-08-09-winget.md` §3,
/// line 166) -- the defect `Name`'s own doc comment describes.
///
/// **Dropping `-e` from THIS call is not known to break anything, and this
/// comment must not claim otherwise.** That defect needs `--exact` AND a
/// wrong-case spelling together; `list_one_argv` only ever runs against a
/// spelling already confirmed correct, and measured, dropping `-e` there
/// still finds it (`show --id Git.Git`, no `-e`, still returns `Found Git
/// [Git.Git]`, `docs/measurements-2026-08-09-winget.md` §3, line 169). The
/// over-matching worry that might otherwise justify keeping `-e` -- a bare
/// substring like `git` accidentally matching `Git.Git` -- is also measured
/// not to happen for a full id: `docs/measurements-2026-08-10-winget-write
/// -path.md` §7 probed `--id 7zip`/`Microsoft`/`ripgrep`/`git`/`zoxide`,
/// each a real substring of a real installed id, all without `--exact`, and
/// every one came back "no package found" -- `--id` always requires the
/// whole id, `--exact` only ever controls case. `-e` is kept here because
/// it is the stricter form against a spelling already known canonical and
/// costs nothing, not because dropping it is known to be dangerous.
///
/// `id.to_string()`, never `id.key()` -- same reasoning as `set_argv`.
pub fn list_one_argv(id: &Name) -> Vec<String> {
    vec![
        "list".to_string(),
        "-e".to_string(),
        "--id".to_string(),
        id.to_string(),
        "--disable-interactivity".to_string(),
    ]
}

/// Every mutating winget invocation this crate makes, behind one seam so
/// every test can fake it and none has to spawn `winget.exe` -- the sibling
/// rule to `WingetCmd` (`src/backend/winget.rs`) and to `execute::Mutator`
/// (scoop's own seam).
///
/// `Err` means the process could not be run at all, exactly as
/// `WingetCmd::run`'s own doc comment says. It does not mean the operation
/// failed -- see this module's own doc comment on why `CmdOut::code` is
/// never the verdict either.
pub trait WingetMutator {
    fn set(&self, id: &Name, version: &str) -> Result<CmdOut, CmdError>;
    fn remove(&self, id: &Name, version: &str) -> Result<CmdOut, CmdError>;
    fn list_one(&self, id: &Name) -> Result<CmdOut, CmdError>;
}

/// The real `winget.exe`, invoked as a subprocess. Only production code
/// (`main.rs`, once a later task wires it up) may construct this -- every
/// test uses a fake that implements `WingetMutator` instead.
pub struct RealWingetMutator;

impl WingetMutator for RealWingetMutator {
    fn set(&self, id: &Name, version: &str) -> Result<CmdOut, CmdError> {
        self.run(&set_argv(id, version))
    }
    fn remove(&self, id: &Name, version: &str) -> Result<CmdOut, CmdError> {
        self.run(&remove_argv(id, version))
    }
    fn list_one(&self, id: &Name) -> Result<CmdOut, CmdError> {
        self.run(&list_one_argv(id))
    }
}

impl RealWingetMutator {
    /// Shells out to `winget` exactly as `RealWinget::run` does
    /// (`src/backend/winget.rs`), including discarding stderr for the same
    /// measured reason, carried over rather than re-justified: stderr was 0
    /// bytes across all 27 write-verb invocations captured for
    /// `docs/measurements-2026-08-10-winget-write-path.md`, every failure
    /// included. Anything winget ever writes to stderr on a real machine is
    /// a surprise this crate has never measured; silently folding it into
    /// stdout would hide that surprise instead of surfacing it.
    fn run(&self, argv: &[String]) -> Result<CmdOut, CmdError> {
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        let out = Command::new("winget")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    CmdError::NotFound
                } else {
                    CmdError::Other(
                        anyhow::Error::new(e).context(format!("cannot run winget {argv:?}")),
                    )
                }
            })?;
        Ok(CmdOut {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Step 1: the failing argv test (TDD RED) --------------------------

    #[test]
    fn the_mutating_argv_is_exactly_what_was_measured() {
        // Every flag here has a measured reason, and the argv is part of this
        // module's contract: docs/measurements-2026-08-10-winget-write-path.md
        // §§1-9 are the only invocations winget's exit codes are trusted for.
        assert_eq!(
            set_argv(&Name::new("Brave.Brave"), "151.1.93.134"),
            vec![
                "install",
                "-e",
                "--id",
                "Brave.Brave",
                "--version",
                "151.1.93.134",
                "--silent",
                "--accept-package-agreements",
                "--accept-source-agreements",
                "--disable-interactivity",
            ]
        );
        assert_eq!(
            remove_argv(&Name::new("Vivaldi.Vivaldi"), "8.1.4087.62"),
            vec![
                "uninstall",
                "-e",
                "--id",
                "Vivaldi.Vivaldi",
                "--version",
                "8.1.4087.62",
                "--disable-interactivity",
                "--accept-source-agreements",
            ]
        );
        assert_eq!(
            list_one_argv(&Name::new("Brave.Brave")),
            vec![
                "list",
                "-e",
                "--id",
                "Brave.Brave",
                "--disable-interactivity"
            ]
        );
    }

    #[test]
    fn the_id_on_the_wire_is_the_display_spelling_never_the_folded_key() {
        // Measured: `--exact` is what makes `--id` case-sensitive, on the WRITE
        // verbs too -- `install -e --id SHARKDP.HYPERFINE` returns 0x8A150014
        // ("no package") for a package that exists, where the correctly-cased
        // call reaches 0x8A150017. `Name::key()` is the folded form, so putting
        // it on the wire means "not found" for a package that is there. The lock
        // holds the canonical spelling winget itself echoed back, which is why
        // `-e` is safe here at all.
        let n = Name::new("Git.Git");
        assert!(set_argv(&n, "1").contains(&"Git.Git".to_string()));
        assert!(!set_argv(&n, "1").contains(&"git.git".to_string()));
        assert!(remove_argv(&n, "1").contains(&"Git.Git".to_string()));
        assert!(list_one_argv(&n).contains(&"Git.Git".to_string()));
    }
}
