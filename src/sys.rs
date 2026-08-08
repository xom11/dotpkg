use sysinfo::System;

/// Lowercased process base names, without extension: "kanata.exe" -> "kanata".
///
/// This is an input to the planner rather than something the planner discovers,
/// which is what lets `dotpkg status` say "skipped, running" before anything is
/// attempted.
pub fn running_process_names() -> Vec<String> {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let mut names: Vec<String> = sys
        .processes()
        .values()
        .map(|p| {
            let n = p.name().to_string_lossy().to_lowercase();
            n.strip_suffix(".exe").unwrap_or(&n).to_string()
        })
        .collect();
    names.sort();
    names.dedup();
    names
}
