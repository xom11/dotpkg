pub mod scoop;
pub mod winget;

use crate::model::{Installed, Name};
use anyhow::Result;

/// What one scan found, plus what it could not read.
///
/// The two fields exist because "this app is not installed" and "this app is
/// installed and I was not allowed to look at it" are different facts, and a
/// bare `Vec<Installed>` reports them identically. A scan must never abort on
/// one bad directory -- forty good ones would vanish with it -- but it must not
/// pretend the bad one was absent either.
#[derive(Debug, Default)]
pub struct Scan {
    pub installed: Vec<Installed>,
    /// Installed, but this backend could not establish its state.
    ///
    /// `plan()` must not read a name's absence from `installed` as "not
    /// installed". The scoop case is a manifest that cannot be traversed; the
    /// winget case is a row with no source, which cannot be compared against
    /// any index. Both would otherwise become `Install` and then, under
    /// `--yes`, an uninstall-and-reinstall of a package that was never absent.
    ///
    /// One field rather than two: the *cause* differs per backend and belongs
    /// in `warnings`, but the *consequence* for the planner is identical.
    pub opaque: Vec<Name>,
    /// One line per entry that was skipped for a reason the user should see.
    /// Expected-and-normal skips (a half-finished install with no manifest yet)
    /// do not appear here.
    pub warnings: Vec<String>,
}

/// One package manager. `scan` reads state that is already on disk or already
/// known; nothing here reaches the network. Mutating methods arrive in Phase 2.
pub trait Backend {
    fn name(&self) -> &'static str;
    fn scan(&self) -> Result<Scan>;
}
