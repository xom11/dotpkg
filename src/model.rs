#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub backend: String,
    pub name: String,
    pub version: String,
    /// Scoop records this in install.json; winget does not expose it.
    pub arch: Option<String>,
    /// Scoop only.
    pub bucket: Option<String>,
}

#[allow(dead_code)]
pub const SCOOP: &str = "scoop";
#[allow(dead_code)]
pub const WINGET: &str = "winget";
