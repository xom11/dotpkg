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

/// Whether this process holds an elevated token.
///
/// `None` means "could not tell", and every caller must treat that as "do not
/// refuse". Measured on a14 (docs/measurements-2026-08-10-winget-write-path.md
/// §5): `winget install` succeeds elevated and `winget uninstall` of that same
/// user-scope package is then refused with 0x8A15007D. dotpkg runs as a
/// scheduled `apply`, so an elevated run can install a package and be
/// structurally unable to remove it -- every prune failing forever.
#[cfg(windows)]
pub fn elevated() -> Option<bool> {
    use std::mem;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return None;
        }
        let mut info = TOKEN_ELEVATION::default();
        let mut written = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut info as *mut _ as *mut _),
            mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut written,
        )
        .is_ok();
        let _ = CloseHandle(token);
        if ok {
            Some(info.TokenIsElevated != 0)
        } else {
            None
        }
    }
}

/// No elevation concept to report. `None`, not `Some(false)`: a caller that
/// refuses on `Some(false)` would be wrong here, and one that refuses on
/// `None` is wrong everywhere -- see the Windows arm's doc comment.
#[cfg(not(windows))]
pub fn elevated() -> Option<bool> {
    None
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
        // A dot that is NOT an executable suffix must survive. `no-extension`
        // above does not cover this: it has no dot at all, so it never reaches
        // the suffix test. Without this line the whole
        // `EXECUTABLE_SUFFIXES.contains(&ext)` guard could be `true` and the
        // suite stayed green -- measured, a surviving mutant in the Task 14
        // run. The consequence is a live `python3.11` normalising to
        // `python3` and matching a package it is not, which is a running-skip
        // decided by a version number.
        assert_eq!(normalize("python3.11"), "python3.11");
    }

    #[test]
    fn the_real_process_table_yields_at_least_one_readable_executable_path() {
        // The only test in the crate that touches the OS. Everything else
        // about the running-process machinery is exercised with fabricated
        // `Process` values, which means nothing catches this function
        // returning `exe: None` for every process -- and that single change
        // silently disables path matching, the *only* running signal that
        // `nodejs` and `rustup` have, because neither names an executable
        // anywhere in its manifest. In Phase 2b that is a missed guard on an
        // uninstall.
        //
        // A test process can always read its own image path on macOS, Linux
        // and Windows, so one readable path is a safe floor to assert; a
        // stricter count would depend on what else happens to be running.
        let procs = running_processes();
        assert!(!procs.is_empty(), "the process table cannot be empty");
        assert!(
            procs.iter().any(|p| p.exe.is_some()),
            "no process reported a readable executable path -- path matching is dead"
        );
    }

    #[test]
    fn elevated_answers_or_admits_it_does_not_know() {
        // The only assertion that is true on all three platforms this crate is
        // built on. The VALUE cannot be asserted: it depends on how the test
        // runner was launched. What must hold is that the function is total and
        // never panics -- because its caller (`apply`'s winget removal
        // pre-check) treats `None` as "do not refuse", and a panic here would
        // take down a run that was about to do useful work.
        // The call itself is the assertion on Windows: `#[test]` fails on a panic,
        // and "does not panic" is the property the caller depends on -- `apply`'s
        // winget-removal pre-check treats `None` as "do not refuse", so a panic
        // here would take down a run that was about to do useful work.
        let answer = elevated();
        #[cfg(not(windows))]
        assert_eq!(answer, None, "there is no elevation concept to report here");
        #[cfg(windows)]
        let _ = answer;
    }
}
