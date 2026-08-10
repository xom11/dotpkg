//! A recording fake for `WingetCmd`, shared by every winget-adjacent
//! integration test binary (`winget_scan.rs` today; `winget_resolve.rs` from
//! Task 13 onward) so that none of them ever spawns `winget.exe` -- the whole
//! reason `WingetCmd` exists as a seam.
//!
//! Not every binary that pulls this in via `mod common;` uses every
//! constructor here (`script`, in particular, is Task 13's), so this module
//! carries its own `#![allow(dead_code)]` the same way `tests/common/mod.rs`
//! does for its own git helpers.
#![allow(dead_code)]

use dotpkg::backend::winget::{CmdError, CmdOut, WingetCmd};
use std::cell::RefCell;
use std::rc::Rc;

/// What `run` hands back, and how many times it may be asked.
enum Plan {
    /// The same `(code, stdout)` answered on every call, however many there
    /// are -- the ordinary case, where a test cares about one response.
    Constant(i32, String),
    /// One entry consumed per call, in the order given. A call past the end
    /// panics: a test that scripts N responses and then triggers call N+1 has
    /// an expectation mismatched with its fake, not a case to paper over with
    /// a default answer that would mean something else entirely.
    Script(Vec<(i32, String)>, usize),
    /// `run` errors exactly once with the held `CmdError`, then panics. A
    /// second call is a test/fake mismatch, matching `Plan::Script`'s own
    /// rule -- and unlike `Constant`, this cannot simply hand back a clone,
    /// because `CmdError` holds an `anyhow::Error` and is not `Clone`.
    Failing(Option<CmdError>),
    /// `run` panics. See `FakeWinget::unreachable`'s own doc comment.
    Unreachable,
}

struct Inner {
    plan: Plan,
    calls: Vec<Vec<String>>,
}

/// `Rc<RefCell<_>>`, not a deep copy: `Winget::new(fake.clone())` hands the
/// clone to the `Winget` under test while the original `fake` binding stays
/// in the caller's hands for `fake.calls()` afterward, and both must see the
/// same call log for that to mean anything.
#[derive(Clone)]
pub struct FakeWinget(Rc<RefCell<Inner>>);

impl FakeWinget {
    /// Answer every call with `(code, stdout)`.
    pub fn returning(code: i32, stdout: String) -> FakeWinget {
        FakeWinget(Rc::new(RefCell::new(Inner {
            plan: Plan::Constant(code, stdout),
            calls: Vec::new(),
        })))
    }

    /// Answer calls in order from `responses`, one entry per call.
    pub fn script(responses: Vec<(i32, String)>) -> FakeWinget {
        FakeWinget(Rc::new(RefCell::new(Inner {
            plan: Plan::Script(responses, 0),
            calls: Vec::new(),
        })))
    }

    /// `run` returns `Err(CmdError::NotFound)` exactly once -- winget.exe is
    /// not on `PATH`. Neither existing caller of this constructor
    /// (`tests/winget_scan.rs`, `tests/winget_resolve.rs`) may change
    /// meaning, so this keeps constructing `CmdError::NotFound` rather than
    /// becoming a thin wrapper over `failing_with` with a different default.
    pub fn failing_to_spawn() -> FakeWinget {
        FakeWinget(Rc::new(RefCell::new(Inner {
            plan: Plan::Failing(Some(CmdError::NotFound)),
            calls: Vec::new(),
        })))
    }

    /// `run` returns `Err(e)` exactly once, then panics on any further call
    /// -- see `Plan::Failing`'s own doc comment for why "exactly once" rather
    /// than repeatable.
    pub fn failing_with(e: CmdError) -> FakeWinget {
        FakeWinget(Rc::new(RefCell::new(Inner {
            plan: Plan::Failing(Some(e)),
            calls: Vec::new(),
        })))
    }

    /// `run` panics if ever called. Task 15's `update`/`adopt` tests that
    /// declare no winget packages at all must never touch winget in any
    /// way -- this is what turns "the winget loop was accidentally reached
    /// anyway" into a loud, immediate test failure instead of a silent
    /// pass-for-the-wrong-reason.
    pub fn unreachable() -> FakeWinget {
        FakeWinget(Rc::new(RefCell::new(Inner {
            plan: Plan::Unreachable,
            calls: Vec::new(),
        })))
    }

    /// Every argv this fake was asked to run, in call order.
    pub fn calls(&self) -> Vec<Vec<String>> {
        self.0.borrow().calls.clone()
    }
}

impl WingetCmd for FakeWinget {
    fn run(&self, args: &[&str]) -> Result<CmdOut, CmdError> {
        let mut inner = self.0.borrow_mut();
        inner
            .calls
            .push(args.iter().map(|s| s.to_string()).collect());
        match &mut inner.plan {
            Plan::Constant(code, stdout) => Ok(CmdOut {
                code: *code,
                stdout: stdout.clone(),
            }),
            Plan::Script(responses, idx) => {
                let (code, stdout) = responses.get(*idx).unwrap_or_else(|| {
                    panic!(
                        "FakeWinget::script exhausted: call {} has no scripted response \
                         (only {} were given)",
                        *idx,
                        responses.len()
                    )
                });
                let out = CmdOut {
                    code: *code,
                    stdout: stdout.clone(),
                };
                *idx += 1;
                Ok(out)
            }
            Plan::Failing(err) => Err(err.take().unwrap_or_else(|| {
                panic!(
                    "FakeWinget::failing_to_spawn/failing_with was called with {args:?} after \
                     its one error was already consumed -- this fake answers with its error \
                     exactly once, matching Plan::Script's own rule that a fake asked past its \
                     script is a test/fake mismatch, not a case to paper over with a default \
                     answer"
                )
            })),
            Plan::Unreachable => panic!(
                "FakeWinget::unreachable was called with {args:?} -- this test declared \
                 no winget packages and must never touch winget at all"
            ),
        }
    }
}
