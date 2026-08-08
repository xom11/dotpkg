pub mod scoop;

use crate::model::Installed;
use anyhow::Result;

/// One package manager. `scan` reads state that is already on disk or already
/// known; nothing here reaches the network. Mutating methods arrive in Phase 2.
pub trait Backend {
    fn name(&self) -> &str;
    fn scan(&self) -> Result<Vec<Installed>>;
}
