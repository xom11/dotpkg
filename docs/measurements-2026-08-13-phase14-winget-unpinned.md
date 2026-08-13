# Phase 14 — `pin = "none"` on real hardware, and one claim refuted

**Machine:** `zenbook-a14`, ARM64 (`PROCESSOR_ARCHITECTURE=ARM64`), Windows
10.0.26200.8973, winget **1.29.280** (`Microsoft.AppInstaller 1.29.280.0`).
**Date:** 2026-08-13, round start 09:31:06 local.
**Tree:** `main` at `9c2f9e7`, transferred as a `git archive` of that commit,
sha256 `2707df121e2d85ac1c461c0444c7cb735e2bcc6cc5a22f45030c7e7bfe804f51`,
verified byte-identical on arrival.
**Binary:** built on the machine with cargo 1.97.1 / rustc 1.97.1, release
profile, 89.7 s, sha256
`6577002cf69e6a683a47c2c82135577bbaaa37aba7642f7cb7620ac0f2e91d72`.
**Frozen record.** Do not edit.

Every run below carried `--prepare` and never `--yes` or `--allow-prune`, and
every `--state` pointed inside `ph14-work`, so dotpkg owned nothing during the
round and no prune could be planned. **Nothing was installed or removed.**
`C:\Users\kln\pkg.toml` — a symlink into the nix repo since 2026-08-12 — was
neither read nor written; every run used its own `--config`.

## 1. The refutation, and it is the headline

**`winget show --id OhMyPosh --disable-interactivity` does NOT match
`JanDeDobbeleer.OhMyPosh`.** Measured through dotpkg's own spawn, on a machine
where `JanDeDobbeleer.OhMyPosh 30.6.4.0` is installed:

```
FAILED  winget OhMyPosh   OhMyPosh: no longer in the winget index
                          (No package found matching input criteria.)
```

That is `NO_APPLICATIONS_FOUND` — the same code, and the byte-identical
sentence, `docs/measurements-2026-08-10-winget-write-path.md` §7 recorded for
five bare-word substrings.

**This contradicts a claim carried in four places** — `CHANGELOG.md`'s "A winget
id that matches a *different* id is refused instead of pinned", the same
sentence in `update::run` and in `adopt::run_winget`, and the commit message of
`c3517e7` — each stating that *"`--id` [is] a substring filter, so a declared
`OhMyPosh` matches `JanDeDobbeleer.OhMyPosh`."* **None of those four carried a
measurement**; §7 was the only measured statement about `--id` and it said the
opposite. This round asked the question directly and §7 is the one that holds.

§7 probed only bare words (`7zip`, `Microsoft`, `ripgrep`, `git`, `zoxide`).
`OhMyPosh` is a **trailing dotted segment**, which the Phase 14 design named as
the one shape nothing had tried. It behaves the same: `--id` requires the whole
id.

**What this does and does not change.**

- The different-id refusals in `update`, `adopt` and `apply::resolve_for_ensure`
  are **not removed**. They are defence at the point of use, this is one machine
  and one winget version, and a refusal that never fires costs nothing while a
  missing one costs a package installed under a name the plan does not carry.
  They should now be read as **defensive against a shape not observed here**,
  not as handling a shape that was.
- **Dropping `-e` from a write verb would probably have been safe after all.**
  The Phase 14 design refused that option on the strength of the disagreement,
  and the disagreement has now resolved toward "it would have been fine". The
  design's choice is nonetheless still correct, for the reason that always
  applied independently: `-e` makes `--id` **case-sensitive**, an unpinned
  package has no lock entry holding the canonical spelling, and §2 below shows
  the resolve-first path supplying it. The conclusion survives; one of its two
  supporting arguments does not.

## 2. The feature, end to end

### Declared, unpinned, and already installed — the case this exists for

`Brave.Brave` is installed at `151.1.93.136`. Declared `pin = "none"` with an
**empty `pkg.lock`**:

```
  0 change(s), 0 skipped, 60 unmanaged
  0 of 0 changes ready, 0 failed, 0 skipped, 0 not locked, 60 unmanaged.
  Nothing has been changed.
  [exit 0]
```

No line, no change, **no `NotLocked`**, exit 0. Before this branch that same
declaration was the refusal that made a real dotfiles repository delete five GUI
packages from its config.

**The counterweight, same package, same empty lock, no `[winget.opts]` entry:**

```
  ! winget Brave.Brave    no lock entry -- run `dotpkg update`
  0 of 0 changes ready, 0 failed, 0 skipped, 1 not locked, 60 unmanaged.
  [exit 1]
```

So the run above passed because of the opts entry and not because the
`NotLocked` rule went missing. (Exit 1 rather than 2 is `--prepare`'s own
documented choice — `main.rs` states that "2 would be wrong regardless, since
`--prepare` genuinely changed nothing". The exit-2 refusal is the full `apply`
path.)

### Declared, unpinned, and absent

`ducaale.xh`, absent from this machine. `status`, which spawns nothing:

```
  + winget ducaale.xh     -                        (install, unpinned -- whatever winget's index has now)
```

`apply --prepare`, which asks winget:

```
  ready   winget ducaale.xh     0.26.2            (install, unpinned)
  1 of 1 changes ready, 0 failed, 0 skipped, 0 not locked
  [exit 0]
```

The two lines differ on purpose: at plan time no version exists to print, and
`0.26.2` is what winget's index answered. This is the canonical-id resolution
working against real winget.

### `update` writes no entry and does not churn

Declared: `Brave.Brave` unpinned, `BurntSushi.ripgrep.MSVC` pinned.

```
  = winget Brave.Brave    unpinned    (no pin -- pkg.toml declares pin = "none")
  + winget BurntSushi.ripgrep.MSVC 15.2.0   (new pin)
  1 changed, 0 unchanged, 0 could not be resolved.
```

`pkg.lock` afterwards, in full:

```toml
[winget."BurntSushi.ripgrep.MSVC"]
version = "15.2.0"
pin     = "version-only"
```

Brave has no entry. Run immediately again:

```
  0 changed, 1 unchanged, 0 could not be resolved.
  pkg.lock is already current -- not rewritten.
```

**No churn.** This is the half that would otherwise rewrite a committed file on
every run for an empty diff.

### A stale pin is warned about, then cleared

`pkg.lock` seeded with `Brave.Brave = 151.1.93.100` while `pkg.toml` declares it
`pin = "none"`:

```
warning: pkg.lock still pins Brave.Brave at 151.1.93.100, but pkg.toml declares
it pin = "none" -- that entry is read by nothing. `dotpkg update` removes it.
```

`dotpkg update`:

```
  - winget Brave.Brave    151.1.93.100    (pin dropped -- pkg.toml declares pin = "none")
  1 changed, 0 unchanged, 0 could not be resolved.
```

and `pkg.lock` is then **empty**. Both directions of the `wrote_anything` split
are therefore observed on real hardware: the steady state writes nothing, and a
dropped pin really lands.

## 3. What this round did NOT do

- **No winget mutation.** `docs/OPEN-ITEMS.md` item 29's "no winget mutation has
  run anywhere" is **unchanged by this round**. Every run stopped at
  `--prepare`. An unpinned `Ensure` has never been executed against real winget;
  only its preparation has.
- **No scoop staging.** No config here declared a scoop package and dotpkg owned
  nothing, so `%LOCALAPPDATA%\dotpkg` and `scoop\cache` were untouched — the
  post-round timestamp sweep of both returned empty.
- **Nothing about the `[winget.guard]` fence**, which an unpinned package
  reaches only on the prune path, and no prune ran.

## 4. Machine left as found

Round-start baseline listing of `C:\Users\kln` taken before anything was
written. Post-round sweep by timestamp against 09:31:00 across `C:\Users\kln`,
`%LOCALAPPDATA%\dotpkg` and `scoop\cache`: the only hits were the eight
`ph14-*` artefacts this round created, all removed. Second sweep after deletion
returned **empty**, and no `ph14*` name remains.

`kanata` was pid **11040** at probe time and pid **11040** at cleanup. It was
never started or stopped.
