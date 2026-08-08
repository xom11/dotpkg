use std::path::PathBuf;
use sysinfo::System;

/// One live process, as much of it as this session is allowed to see.
pub struct Process {
    /// Lowercased base name without a trailing `.exe`: "Kanata.exe" -> "kanata".
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
        .map(|p| {
            let n = p.name().to_string_lossy().to_ascii_lowercase();
            Process {
                name: n.strip_suffix(".exe").unwrap_or(&n).to_string(),
                exe: p.exe().map(|e| e.to_path_buf()),
            }
        })
        .collect()
}
