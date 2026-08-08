use super::{Backend, Scan};
use crate::model::{Installed, Name, SCOOP};
use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct Manifest {
    version: String,
}

#[derive(Debug, Default, Deserialize)]
struct Install {
    #[serde(default)]
    bucket: Option<String>,
    #[serde(default)]
    architecture: Option<String>,
}

pub struct Scoop {
    root: PathBuf,
}

impl Scoop {
    pub fn new(root: PathBuf) -> Scoop {
        Scoop { root }
    }

    /// `$SCOOP` if set, else `%USERPROFILE%\scoop`, matching scoop's own rule.
    pub fn discover() -> Scoop {
        let root = std::env::var_os("SCOOP")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("scoop")))
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join("scoop")))
            .unwrap_or_else(|| PathBuf::from("scoop"));
        Scoop { root }
    }
}

impl Backend for Scoop {
    fn name(&self) -> &str {
        SCOOP
    }

    fn scan(&self) -> Result<Scan> {
        let apps = self.root.join("apps");
        let entries = match std::fs::read_dir(&apps) {
            Ok(e) => e,
            // No scoop on this machine is a valid state, not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Scan::default()),
            Err(e) => return Err(e.into()),
        };

        let mut out = Scan::default();
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            // apps/scoop is scoop managing itself.
            if name == SCOOP {
                continue;
            }

            let current = entry.path().join("current");
            let manifest_path = current.join("manifest.json");
            let manifest_text = match std::fs::read_to_string(&manifest_path) {
                Ok(t) => t,
                // No manifest yet is the ordinary shape of a half-finished
                // install, or of `current` pointing at a version directory
                // scoop is still unpacking. Nothing to tell the user.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                // Anything else -- a permission denial, a dangling junction --
                // is a fact about this machine. Skipping it silently would make
                // an app the user *does* have look uninstalled, which in Phase 2
                // is an offer to reinstall it. Still not fatal: one unreadable
                // directory must not hide the other forty.
                Err(e) => {
                    out.warnings
                        .push(format!("{name}: cannot read manifest.json: {e}"));
                    continue;
                }
            };
            let manifest = match serde_json::from_str::<Manifest>(&manifest_text) {
                Ok(m) => m,
                Err(e) => {
                    out.warnings
                        .push(format!("{name}: manifest.json is not usable: {e}"));
                    continue;
                }
            };

            // install.json is absent on apps installed by older scoop versions.
            let install: Install = std::fs::read_to_string(current.join("install.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();

            out.installed.push(Installed {
                backend: SCOOP.to_string(),
                name: Name::new(name),
                version: manifest.version,
                arch: install.architecture,
                bucket: install.bucket,
                bins: Vec::new(),
            });
        }
        Ok(out)
    }
}
