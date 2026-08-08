use super::Backend;
use crate::model::{Installed, SCOOP};
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

    fn scan(&self) -> Result<Vec<Installed>> {
        let apps = self.root.join("apps");
        let entries = match std::fs::read_dir(&apps) {
            Ok(e) => e,
            // No scoop on this machine is a valid state, not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        let mut out = Vec::new();
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
            let Ok(manifest_text) = std::fs::read_to_string(current.join("manifest.json")) else {
                // A half-finished or broken install must not fail the whole scan.
                continue;
            };
            let Ok(manifest) = serde_json::from_str::<Manifest>(&manifest_text) else {
                continue;
            };

            // install.json is absent on apps installed by older scoop versions.
            let install: Install = std::fs::read_to_string(current.join("install.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();

            out.push(Installed {
                backend: SCOOP.to_string(),
                name,
                version: manifest.version,
                arch: install.architecture,
                bucket: install.bucket,
            });
        }
        Ok(out)
    }
}
