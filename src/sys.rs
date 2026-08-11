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

/// Given the two token signals, is this the state where winget refuses a
/// user-scope uninstall? `None` for either input means "could not tell".
///
/// Kept free of Win32 entirely (unlike `elevated` below) so it can be
/// unit-tested on every platform this crate builds on -- the combination
/// rule is what needs proving, and only Windows can produce the inputs, not
/// the rule. `cfg(any(windows, test))` rather than a bare `cfg(windows)`: the
/// only two callers are `elevated`'s Windows arm and this module's tests, so
/// on a non-Windows, non-test build it would otherwise be unreachable and
/// `-D warnings` would refuse the build over dead code that is not actually
/// dead anywhere this crate runs.
#[cfg(any(windows, test))]
fn verdict(is_elevated: Option<bool>, in_admins: Option<bool>) -> Option<bool> {
    match (is_elevated, in_admins) {
        // Not elevated at all: winget never refuses a user-scope uninstall
        // here, regardless of what group membership says.
        (Some(false), _) => Some(false),
        // Elevated and an enabled Administrators member: a real elevated
        // shell, the case the refusal exists for.
        (Some(true), Some(true)) => Some(true),
        // Elevated flag set but not an enabled Administrators member: THE
        // MEASURED restricted-token case (see `elevated`'s doc comment).
        // winget succeeded here, so dotpkg must not refuse.
        (Some(true), Some(false)) => Some(false),
        // Either signal unknown: could not tell, and "could not tell" must
        // never resolve to a refusal.
        (None, _) | (_, None) => None,
    }
}

/// Whether this process is in the state where winget refuses a user-scope
/// uninstall. Not "is this token elevated" -- that question turned out to
/// need two signals, not one, to answer.
///
/// `None` means "could not tell", and every caller must treat that as "do not
/// refuse".
///
/// Measured on a14 (docs/measurements-2026-08-10-winget-write-path.md §5):
/// `winget install` succeeds elevated and `winget uninstall` of that same
/// user-scope package is then refused with 0x8A15007D. That measurement is
/// what first justified answering with `TOKEN_ELEVATION.TokenIsElevated`
/// alone.
///
/// The Phase 4b dogfood on that same machine overturned the single-signal
/// version, on one restricted token: from a `runas /trustlevel:0x20000`
/// child of an elevated PowerShell, .NET's `IsInRole(Administrators)`
/// reported `False` while `TOKEN_ELEVATION.TokenIsElevated` still read 1
/// (inherited from the elevated parent), and `winget uninstall -e --id
/// ducaale.xh` **succeeded** (exit 0) from that same child shell. So exactly
/// two states have been observed: a real elevated session, where winget
/// refuses, and this restricted token, where winget allows. An ordinary
/// non-elevated interactive session was NOT exercised in that round (the
/// dogfood's "medium integrity" run did not actually de-elevate -- a bug in
/// the test script, recorded in progress.md) -- it is expected to behave
/// like the restricted token (`TokenIsElevated` false, so `verdict` answers
/// `Some(false)` without needing the second signal at all), but that is
/// reasoned, not measured.
///
/// `runas /trustlevel:0x20000` is understood to build a *restricted* token by
/// marking the Administrators SID DENY_ONLY rather than removing it
/// (`SaferComputeTokenFromLevel`'s documented mechanism) -- nobody dumped
/// this token's group attributes to confirm it, so that is the reasoned
/// explanation for the `IsInRole` result, not itself an observation.
/// `CheckTokenMembership` honours DENY_ONLY the same way `IsInRole` does.
/// That was reasoned when this function was written -- `WindowsPrincipal.
/// IsInRole` calls it internally -- and it has since been **measured**, which
/// is the only reason the sentence is stated this plainly. On a14, from a
/// `runas /trustlevel:0x20000` child of an elevated shell, `dotpkg apply
/// --yes --allow-prune` of a user-scope `ducaale.xh` exited 0, printed `done
/// winget ducaale.xh verified on disk`, and the package really was gone. The
/// single-signal version returned exit 2 and a refusal for that same shape.
/// So `verdict` has now been observed answering `Some(false)` on a token
/// whose `TokenIsElevated` is 1 -- the direction this whole two-signal
/// design exists for, and the one an assertion of `Some(true)` can never
/// reach.
///
/// One gap stays open: an ordinary non-elevated interactive session, with no
/// `runas` at all, is still unmeasured. It is expected to answer `Some(false)`
/// on the first signal alone, never consulting the second, but that is
/// reasoned.
#[cfg(windows)]
pub fn elevated() -> Option<bool> {
    use std::mem;
    use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, PSID};
    use windows::Win32::Security::{
        CheckTokenMembership, CreateWellKnownSid, GetTokenInformation, TokenElevation,
        WinBuiltinAdministratorsSid, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return None;
        }
        let mut info = TOKEN_ELEVATION::default();
        let mut written = 0u32;
        let elevation_ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut info as *mut _ as *mut _),
            mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut written,
        )
        .is_ok();
        let _ = CloseHandle(token);
        let is_elevated = elevation_ok.then_some(info.TokenIsElevated != 0);

        // `CheckTokenMembership` honours DENY_ONLY groups, which is the
        // signal `TOKEN_ELEVATION` misses in the restricted-token case
        // documented above. Build the well-known Administrators SID into a
        // stack buffer -- 68 bytes is `SECURITY_MAX_SID_SIZE` from the
        // Windows SDK, large enough for any SID `CreateWellKnownSid` can
        // produce; that constant is not bound by the `windows` crate itself,
        // hence the literal. `u32` elements, not `u8`: a SID's subauthorities
        // are a `DWORD` array and the kernel writes it by that struct layout,
        // so the buffer needs 4-byte alignment -- `[u8; N]` only guarantees
        // alignment 1.
        let mut sid_buf = [0u32; 17];
        let mut sid_len = mem::size_of_val(&sid_buf) as u32;
        let sid = PSID(sid_buf.as_mut_ptr() as *mut _);
        let in_admins = if CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            PSID::default(),
            sid,
            &mut sid_len,
        )
        .is_ok()
        {
            // NULL token handle, not the (already-closed) handle opened
            // above: `CheckTokenMembership` requires an impersonation-level
            // token when non-NULL, but `OpenProcessToken` above returns a
            // primary token. NULL asks it to duplicate the calling thread's
            // own effective token internally instead. That is not quite the
            // token this question is about -- winget runs as a *child
            // process*, which inherits the process token, not a thread's
            // impersonation token -- but the two agree here because this
            // crate never impersonates, so the process token and the
            // thread's effective token are the same token. If that ever
            // changes, this NULL must change with it.
            let mut is_member = BOOL::default();
            if CheckTokenMembership(HANDLE::default(), sid, &mut is_member).is_ok() {
                Some(is_member.as_bool())
            } else {
                None
            }
        } else {
            None
        };

        verdict(is_elevated, in_admins)
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

    // `verdict` is plain Rust with no Win32 in it and compiles under
    // `cfg(test)` on every platform, so these run on macOS and Linux too --
    // the point of extracting it out of `elevated()` in the first place.

    #[test]
    fn elevated_and_in_admins_group_is_a_real_elevated_shell_and_must_refuse() {
        // Both signals true: this is the actual "run apply from an elevated
        // shell" case the refusal exists for.
        assert_eq!(verdict(Some(true), Some(true)), Some(true));
    }

    #[test]
    fn elevated_but_deny_only_admins_is_the_measured_restricted_token_case_and_must_not_refuse() {
        // THE MEASURED CASE: a `runas /trustlevel:0x20000` child reports
        // `TokenIsElevated = 1` while its Administrators SID is DENY_ONLY, so
        // `CheckTokenMembership` reports `false`. `winget uninstall`
        // succeeded from that same shell in the Phase 4b dogfood, so dotpkg
        // must not refuse it either.
        assert_eq!(verdict(Some(true), Some(false)), Some(false));
    }

    #[test]
    fn not_elevated_is_an_ordinary_user_session_and_must_not_refuse() {
        // Not elevated at all: group membership is moot, winget never
        // refuses a user-scope uninstall from an unelevated process. All
        // three membership answers are covered, including "could not tell"
        // -- `is_elevated: Some(false)` is conclusive on its own, so an
        // unknown `in_admins` must not turn it into `None`.
        assert_eq!(verdict(Some(false), Some(true)), Some(false));
        assert_eq!(verdict(Some(false), Some(false)), Some(false));
        assert_eq!(verdict(Some(false), None), Some(false));
    }

    #[test]
    fn an_unknown_signal_on_either_side_means_could_not_tell_and_must_not_refuse() {
        // `None` anywhere the elevation signal is not already conclusively
        // `Some(false)` must resolve to `None`, never to `Some(true)`: the
        // caller treats `None` as "do not refuse", so a failure of either
        // underlying Win32 call must degrade to that, not to a guess.
        assert_eq!(verdict(None, Some(true)), None);
        assert_eq!(verdict(None, Some(false)), None);
        assert_eq!(verdict(None, None), None);
        assert_eq!(verdict(Some(true), None), None);
    }
}
