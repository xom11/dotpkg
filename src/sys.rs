use std::path::PathBuf;
use sysinfo::System;

/// Suffixes stripped from both a live process's name (below) and a
/// manifest's declared executable (`declared_executables` in
/// `backend::scoop`), so the two are compared in the same form.
///
/// Only `.exe` and `.com` are ever real Windows process images —
/// `.cmd`/`.bat`/`.ps1` name a script, not the process that runs it — but
/// both call sites strip this exact list rather than their own subset, so
/// there is only one place left to get it wrong.
pub const EXECUTABLE_SUFFIXES: &[&str] = &["exe", "cmd", "bat", "ps1", "com"];

/// Lowercases `raw` and removes a trailing suffix in `EXECUTABLE_SUFFIXES`,
/// if the part after the last `.` is one: "Kanata.exe" -> "kanata", "tool.com"
/// -> "tool", "python3.11" -> "python3.11" (`11` is not a known suffix).
fn normalize(raw: &str) -> String {
    let n = raw.to_ascii_lowercase();
    match n.rsplit_once('.') {
        Some((stem, ext)) if EXECUTABLE_SUFFIXES.contains(&ext) => stem.to_string(),
        _ => n,
    }
}

/// One live process, as much of it as this session is allowed to see.
pub struct Process {
    /// Base name normalized by `normalize`: "Kanata.exe" -> "kanata".
    pub name: String,
    /// `None` when the executable path cannot be read — a process at a higher
    /// integrity level, or a kernel process. Name matching is what covers
    /// those, which is why the two signals are kept separate.
    pub exe: Option<PathBuf>,
}

/// The running process table.
///
/// This is an input to the planner rather than something the planner
/// discovers, which is what lets `dotpkg status` say "skipped, running" before
/// anything is attempted.
pub fn running_processes() -> Vec<Process> {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.processes()
        .values()
        .map(|p| Process {
            name: normalize(&p.name().to_string_lossy()),
            exe: p.exe().map(|e| e.to_path_buf()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dot_com_suffix_is_stripped_like_dot_exe() {
        // `.com` is the one non-`.exe` suffix that is ever a real Windows
        // process image. If this list and `backend::scoop`'s ever drift
        // apart, a `.com` package's live process stops matching its own
        // manifest and the package goes straight to Prune.
        assert_eq!(normalize("tool.com"), "tool");
        assert_eq!(normalize("Kanata.exe"), "kanata");
        assert_eq!(normalize("no-extension"), "no-extension");
    }
}
