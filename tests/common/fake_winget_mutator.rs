//! A recording fake for `WingetMutator`, modelled directly on
//! `tests/common/fake_winget.rs` -- the write-side sibling of that seam's
//! read-side fake, so that no test in `tests/winget_execute.rs` (or any
//! later winget-executor test binary) ever spawns `winget.exe`.
//!
//! Not every binary that pulls this in via `mod common;` uses every
//! constructor here, so this module carries its own `#![allow(dead_code)]`
//! the same way `tests/common/mod.rs` does for its own git helpers.
#![allow(dead_code)]

use dotpkg::backend::winget::{CmdError, CmdOut};
use dotpkg::backend::winget_exec::WingetMutator;
use dotpkg::model::Name;
use std::cell::RefCell;
use std::rc::Rc;

/// What `set`/`remove`/`list_one` hand back, and how many times they may be
/// asked. Identical shape to `fake_winget::Plan`; see that type's own doc
/// comments for why each variant behaves the way it does.
enum Plan {
    /// The same `(code, stdout)` answered on every call, however many there
    /// are -- the ordinary case, where a test cares about one response.
    Constant(i32, String),
    /// One entry consumed per call, in the order given. A call past the end
    /// panics: a test that scripts N responses and then triggers call N+1 has
    /// an expectation mismatched with its fake, not a case to paper over with
    /// a default answer that would mean something else entirely.
    Script(Vec<(i32, String)>, usize),
    /// The mutator errors exactly once with the held `CmdError`, then panics.
    /// A second call is a test/fake mismatch, matching `Plan::Script`'s own
    /// rule -- and unlike `Constant`, this cannot simply hand back a clone,
    /// because `CmdError` holds an `anyhow::Error` and is not `Clone`.
    Failing(Option<CmdError>),
    /// One entry consumed per call as `Script` does, and then the held
    /// `CmdError` for the call *after* the last entry -- once, then a panic.
    ///
    /// **The only way to reach `run_winget_step`'s two rescan-`Err` arms.**
    /// `winget_verdict` returns `Err` from exactly one place, `list_one`, so a
    /// test for "the mutation ran and then the rescan could not be spawned"
    /// needs an `Ok` answer followed by an `Err`. Neither existing variant can
    /// produce that: `Script` answers `Ok` forever, and `Failing` errors on the
    /// *first* call, which is the mutation itself and a different arm entirely.
    ScriptThenFailing(Vec<(i32, String)>, usize, Option<CmdError>),
    /// Every call panics. A test that declares no winget packages must make
    /// any winget mutation a loud panic, not a silent pass -- the same rule
    /// `FakeWinget::unreachable` exists for on the read side.
    Unreachable,
}

struct Inner {
    plan: Plan,
    calls: Vec<Vec<String>>,
}

/// `Rc<RefCell<_>>`, not a deep copy: a clone handed to the code under test
/// shares the call log with the caller's own binding, and both must see the
/// same log for `calls()` to mean anything afterward.
#[derive(Clone)]
pub struct FakeWingetMutator(Rc<RefCell<Inner>>);

impl FakeWingetMutator {
    /// Answer every call with `(code, stdout)`.
    pub fn returning(code: i32, stdout: String) -> FakeWingetMutator {
        FakeWingetMutator(Rc::new(RefCell::new(Inner {
            plan: Plan::Constant(code, stdout),
            calls: Vec::new(),
        })))
    }

    /// Answer calls in order from `responses`, one entry per call.
    pub fn script(responses: Vec<(i32, String)>) -> FakeWingetMutator {
        FakeWingetMutator(Rc::new(RefCell::new(Inner {
            plan: Plan::Script(responses, 0),
            calls: Vec::new(),
        })))
    }

    /// The mutator returns `Err(e)` exactly once, then panics on any further
    /// call -- see `Plan::Failing`'s own doc comment for why "exactly once"
    /// rather than repeatable.
    pub fn failing_with(e: CmdError) -> FakeWingetMutator {
        FakeWingetMutator(Rc::new(RefCell::new(Inner {
            plan: Plan::Failing(Some(e)),
            calls: Vec::new(),
        })))
    }

    /// Answer calls in order from `responses`, then error once with `e` on the
    /// call after the last of them. See `Plan::ScriptThenFailing`'s own doc
    /// comment for the arm this exists to reach.
    pub fn script_then_failing(responses: Vec<(i32, String)>, e: CmdError) -> FakeWingetMutator {
        FakeWingetMutator(Rc::new(RefCell::new(Inner {
            plan: Plan::ScriptThenFailing(responses, 0, Some(e)),
            calls: Vec::new(),
        })))
    }

    /// Every call panics. See `Plan::Unreachable`'s own doc comment.
    pub fn unreachable() -> FakeWingetMutator {
        FakeWingetMutator(Rc::new(RefCell::new(Inner {
            plan: Plan::Unreachable,
            calls: Vec::new(),
        })))
    }

    /// Every argv this fake was asked to run, in call order.
    pub fn calls(&self) -> Vec<Vec<String>> {
        self.0.borrow().calls.clone()
    }

    fn record_and_answer(&self, argv: Vec<String>) -> Result<CmdOut, CmdError> {
        let mut inner = self.0.borrow_mut();
        inner.calls.push(argv.clone());
        match &mut inner.plan {
            Plan::Constant(code, stdout) => Ok(CmdOut {
                code: *code,
                stdout: stdout.clone(),
            }),
            Plan::Script(responses, idx) => {
                let (code, stdout) = responses.get(*idx).unwrap_or_else(|| {
                    panic!(
                        "FakeWingetMutator::script exhausted: call {} has no scripted response \
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
            Plan::ScriptThenFailing(responses, idx, err) => {
                if let Some((code, stdout)) = responses.get(*idx) {
                    let out = CmdOut {
                        code: *code,
                        stdout: stdout.clone(),
                    };
                    *idx += 1;
                    return Ok(out);
                }
                Err(err.take().unwrap_or_else(|| {
                    panic!(
                        "FakeWingetMutator::script_then_failing was called again after its \
                         {} scripted responses and its one error were all consumed -- same \
                         rule as Plan::Script's own: a fake asked past its script is a \
                         test/fake mismatch, not a case to paper over",
                        responses.len()
                    )
                }))
            }
            Plan::Failing(err) => Err(err.take().unwrap_or_else(|| {
                panic!(
                    "FakeWingetMutator::failing_with was called again after its one error was \
                     already consumed -- this fake answers with its error exactly once, \
                     matching Plan::Script's own rule that a fake asked past its script is a \
                     test/fake mismatch, not a case to paper over with a default answer"
                )
            })),
            Plan::Unreachable => panic!(
                "FakeWingetMutator::unreachable was called with {argv:?} -- this test declared \
                 no winget packages and must never touch a winget mutation at all"
            ),
        }
    }
}

impl WingetMutator for FakeWingetMutator {
    fn set(&self, id: &Name, version: &str) -> Result<CmdOut, CmdError> {
        self.record_and_answer(dotpkg::backend::winget_exec::set_argv(id, version))
    }
    fn remove(&self, id: &Name, version: &str) -> Result<CmdOut, CmdError> {
        self.record_and_answer(dotpkg::backend::winget_exec::remove_argv(id, version))
    }
    fn list_one(&self, id: &Name) -> Result<CmdOut, CmdError> {
        self.record_and_answer(dotpkg::backend::winget_exec::list_one_argv(id))
    }
}
