# Changelog

## Unreleased

Since `v0.1.0` (`7ab9413`). **One behaviour change, and it is a deletion:**
`update` and `apply` no longer write `pkg.lock.bak` and `pkg.toml.bak`. Nothing
else a user would notice is different — no command, flag, exit code or output
line moved, and the one refactor was measured to be behaviour-preserving rather
than asserted to be. Everything else that changed is the evidence behind 0.1.0
and the shape of the code underneath it.

### The one behaviour change

- **`pkg.toml.bak` and `pkg.lock.bak` are no longer written.** Both files are
  **committed**, so the user's own history already holds every version of them
  and `git checkout` recovers strictly more than a copy of the last one ever
  did; what the copies produced in practice was a permanently dirty
  `git status` beside files people commit. **`state.json.bak` stays**, because
  `state.json` is deliberately not committed — it is the truth of one machine —
  so nothing else can recover it, and it lives in the platform state directory
  where no version control is watching. Existing `.bak` files are safe to
  delete. The rule this leaves is in the README: a `.bak` is for a file nothing
  else can recover.

### The claims got stronger without the features moving

- **`apply` has now run from the published binary**, sha256 `9daeae0c…` and not
  a rebuild of it, installing and then pruning a real package on real hardware
  and verifying both on disk. Until this it had only ever run `status`, which
  mutates nothing. The prune was preceded by its own counterweight: the same
  command with the flags withheld refused at exit 2 and left the package
  installed.
- **The design's third test layer exists at last.** "Real scoop in a throwaway
  `$env:SCOOP` on a Windows runner" was specified on 2026-08-08 and never built;
  `tests/cli.rs` was never it, being hermetic by design. A CI job now installs
  scoop into a throwaway root, builds a bucket that is a real git repository,
  and serves a real archive over HTTP so scoop does its own hash check.
- **What that still does not cover is stated rather than left to be assumed:** a
  version change — scoop's uninstall-then-install window — and any winget
  mutation. Neither has release-binary or CI evidence. See `docs/OPEN-ITEMS.md`
  item 29.

### Code

- **`execute::Mutates`** names the write half of a backend, which had no
  contract at all: the two per-backend seams were threaded through `execute` and
  `run_step` as a hand-written pair of parameters, so a third backend meant a
  third parameter at 27 call sites. `execute::Backends` carries both sides in one
  value. Measured name by name: identical 658-name test sets before and after,
  0 lost, 0 added.
- One test added, joining `Backend::scan` to the `[winget.guard]` opaque warning
  — two halves that were each pinned and never connected.

### Record

- **`docs/OPEN-ITEMS.md`** is the one live list, keeping every item under the
  number it already had.
- Twenty documents removed, about 28,200 lines: the phase narratives and then
  all eight task-breakdown plans. Every one still reads with `git show`; both
  waves name their commit in `OPEN-ITEMS.md`.
- **`LICENSE` now exists.** `Cargo.toml` had claimed MIT since the first commit
  with no such file in the tree.
- A flake was added and removed the same day; the reasoning is in the README.
- **`flake.lock` was left behind by that removal and is now gone too** — a lock
  file for a flake that does not exist, in a project whose whole thesis is that a
  lock records what was actually resolved.

## 0.1.0 — 2026-08-12

First release. 321 commits over five days, and the whole of it is one idea:
**declare Windows packages in a file, resolve them into a lock, and let a tool
bring the machine to that state without ever guessing.**

### What it does

Four commands, both backends (winget and scoop) in each:

- **`status`** — prints the plan it would execute and changes nothing. No
  install, no uninstall, no network.
- **`apply`** — brings the machine to what `pkg.toml` and `pkg.lock` describe.
  Stages and fetches everything first, asks once, then mutates — and **verifies
  every result afterwards rather than believing the package manager's exit
  code**, because neither manager's exit code says whether anything happened.
- **`update`** — moves the lock forward, not the machine.
- **`adopt`** — records an already-installed package as dotpkg's, so `prune`
  can reach it.

### The parts worth naming

- **A real lock file.** `pkg.lock` records what each declaration resolved to —
  for scoop, the bucket commit as well as the version — so the same `pkg.toml`
  produces the same machine.
- **Prune can never reach a package dotpkg did not install.** Ownership lives
  in `state.json`, written when dotpkg installs something and never inferred.
- **A running-process fence.** dotpkg refuses to replace or remove a package
  whose process is alive, by two independent signals: a live process running
  out of the package's own directory, and a name match. `[winget.guard]` lets
  you name the processes a winget id really runs, and **dotpkg now tells you
  which entry you are missing** rather than failing silently.
- **Exit codes that mean something.** `2` is "refused before anything was
  attempted; nothing changed."

### What is measured, and what is not

This project's rule is that a claim carries the measurement that settles it,
and the same rule applies to the release. **See the "Verified on" section of
the README for the exact scope**, which is narrower than the feature list
suggests: one real Windows machine, one architecture, one winget version.

Every number in the documents under `docs/` names the machine and the tree it
was taken on. Where something ships structurally verified and live-unverified,
it is numbered in a still-open list rather than described as done.

### Known gaps, decided rather than forgotten

- **Downgrading a winget package** — decided against, not deferred. Measured:
  `winget install --version <older>` only ever moves a package up.
- **Dependency handling** — a winget install that pulls in a second package
  leaves that package unmanaged. Measured and reported, not silently absorbed.
- **`add`** — `pkg.toml` plus `update` plus `apply` composes to the same thing.
- **Chocolatey** — nothing beyond the two backends.
