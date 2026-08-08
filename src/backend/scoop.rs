use super::{Backend, Scan};
use crate::model::{Installed, Name, SCOOP};
use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

/// Every executable this manifest declares, normalised to the form
/// `sysinfo` reports a process under: basename, known extension removed,
/// lowercased.
///
/// This walks for the keys instead of modelling the schema. Measured across
/// the author's thirty installed manifests, `bin` appears as a bare string, a
/// list of strings, a mixed list of strings and `[path, alias]` pairs, and
/// nested under `architecture.<arch>`. A depth-first collect handles all four
/// and cannot be broken by a fifth shape nobody has seen.
///
/// Every architecture branch is collected, not just the installed one:
/// `kanata` declares its executables per architecture, and reading only one
/// branch is how the app that costs you the keyboard goes unguarded.
///
/// `shortcuts` is collected alongside `bin` because for `antigravity` it is
/// the only field in the manifest that names an executable at all.
///
/// Over-collection is the safe direction: a spurious entry can only ever
/// cause a package to be skipped.
fn declared_executables(manifest: &serde_json::Value) -> Vec<String> {
    const EXECUTABLE_SUFFIXES: &[&str] = &["exe", "cmd", "bat", "ps1", "com"];

    fn add(v: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
        match v {
            serde_json::Value::String(s) => {
                // Later elements of a bin tuple can be arguments, not names.
                if s.starts_with('-') {
                    return;
                }
                let base = s.rsplit(['\\', '/']).next().unwrap_or(s);
                let stem = base
                    .rsplit_once('.')
                    .filter(|(_, ext)| {
                        EXECUTABLE_SUFFIXES.contains(&ext.to_ascii_lowercase().as_str())
                    })
                    .map(|(stem, _)| stem)
                    .unwrap_or(base);
                if !stem.is_empty() {
                    out.insert(stem.to_ascii_lowercase());
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|e| add(e, out)),
            _ => {}
        }
    }

    fn walk(v: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, val) in m {
                    if k == "bin" || k == "shortcuts" {
                        add(val, out);
                    } else {
                        walk(val, out);
                    }
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|e| walk(e, out)),
            _ => {}
        }
    }

    let mut out = std::collections::BTreeSet::new();
    walk(manifest, &mut out);
    out.into_iter().collect()
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
            let manifest: serde_json::Value = match serde_json::from_str(&manifest_text) {
                Ok(m) => m,
                Err(e) => {
                    out.warnings
                        .push(format!("{name}: manifest.json is not usable: {e}"));
                    continue;
                }
            };
            let Some(version) = manifest.get("version").and_then(|v| v.as_str()) else {
                out.warnings
                    .push(format!("{name}: manifest.json has no version"));
                continue;
            };
            let bins = declared_executables(&manifest);

            // install.json is absent on apps installed by older scoop versions.
            let install: Install = std::fs::read_to_string(current.join("install.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();

            out.installed.push(Installed {
                backend: SCOOP.to_string(),
                name: Name::new(name),
                version: version.to_string(),
                arch: install.architecture,
                bucket: install.bucket,
                bins,
            });
        }
        Ok(out)
    }
}
