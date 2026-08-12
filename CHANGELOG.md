# Changelog

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
