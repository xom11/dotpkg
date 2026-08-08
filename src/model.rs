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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_is_comparable_by_value() {
        let a = Installed {
            backend: SCOOP.into(),
            name: "fzf".into(),
            version: "0.74.2".into(),
            arch: Some("arm64".into()),
            bucket: Some("main".into()),
        };
        let b = a.clone();
        assert_eq!(a, b);
    }
}
