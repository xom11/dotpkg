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

use dotpkg::backend::winget::{CmdOut, WingetCmd};
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
    /// `run` always errors, matching what happens when `winget.exe` is not on
    /// `PATH` at all.
    FailingToSpawn,
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

    /// `run` always returns `Err` -- winget.exe is not on `PATH`.
    pub fn failing_to_spawn() -> FakeWinget {
        FakeWinget(Rc::new(RefCell::new(Inner {
            plan: Plan::FailingToSpawn,
            calls: Vec::new(),
        })))
    }

    /// Every argv this fake was asked to run, in call order.
    pub fn calls(&self) -> Vec<Vec<String>> {
        self.0.borrow().calls.clone()
    }
}

impl WingetCmd for FakeWinget {
    fn run(&self, args: &[&str]) -> anyhow::Result<CmdOut> {
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
            Plan::FailingToSpawn => Err(anyhow::anyhow!("winget.exe not found on PATH (fake)")),
        }
    }
}
