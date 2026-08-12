# Measurements: what the winget fence can see, what it cannot, and the sentence that says so

Round run 2026-08-12 on a14 (`zenbook-a14`) and on this macOS machine, starting
from `main` at **`7da8502`** and landing on the commit that carries this file.

**Which tree each figure describes, stated up front because the previous round
ended by having to add exactly this.** The machine-state probes (§1-§4) predate
any code change and describe a14 as it was on `7da8502`. The live fence figures
in §6 were first taken on `d0685c0`, corrected on `4054cda`, re-taken on
`b445482`, and **taken once more on `82b7bc0`** -- the tree this branch ends on
and the one every figure in §8c and §9 describes. They did not move: 27 and 25
both times, which is the answer the review fixes predicted, since quoting a key
in a message changes the text and not which packages the fence cannot see.

**Every probe on a14 is read-only**, or confined to this round's own `ph7-`
prefix: `winget export`, `winget show`, `winget list`, `Get-Process`,
`Get-ChildItem`, `Get-FileHash`, a byte-for-byte copy of two winget databases,
and `dotpkg status`, which mutates nothing. No winget write verb was invoked.
kanata was never started, stopped or signalled; its pid was **9644** at the
start of the round and **3976** by the fence runs, and both are recorded as
landmarks rather than asserted -- the record already warns that this pid changes
between rounds, and it changed inside this one.

Artefacts all carry the `ph7-` prefix: `ph7-build`, `ph7-target`, `ph7-fence`,
`ph7-dotpkg.tgz`, and the probe scripts and outputs. Phase 4b's `dotpkg-build`
and `p4b-dogfood` and `C:\Users\kln`'s unrelated session work were not touched.
a14's real `pkg.toml` was read and hashed, never written: sha256 `32a238ff…`
before and after.

## The headline

1. **The assumption the whole "4 of N" claim rests on is true, and had never
   been checked.** A directory under a winget package root exists for **exactly
   the 4 `portable (zip)` ids of the 41 installed, and for none of the other
   37** -- no exception in either direction, across all eight installer types
   present.
2. **The record's 36 and winget's 41 were never in conflict, and now the
   difference has a mechanism.** Diffed by name on one machine on one day:
   41 = 36 + 5, and the five are exactly the ids dotpkg refuses to read a
   version for. Each figure answers a different question, and each now says
   which it answers.
3. **The strongest lead the record deliberately left unopened was opened, and it
   is worth less than it looked and costs more.** `installed.db` really is
   SQLite and really does carry a populated `commands` table -- and it would
   raise coverage from 4 to **10 of 41**, while disagreeing with `winget list`
   on **31 names in both directions**, at least 11 of which are packages
   **scoop** installed.
4. **What ships adds no coverage at all, and says so.** It converts the silent
   half into a sentence: on a plan with 30 pending changes, **27** name the
   `[winget.guard]` entry the user is missing, and the 3 it stays silent about
   are exactly the ones the path signal covers.
5. **The live run corrected the code twice**, in ways no unit test could have,
   because the tests only ever fed the function the shape it was written for.

## 1. Does a non-portable package ever create a directory under `Packages\`?

This is the measurement that could have invalidated every direction at once. The
record's "4 of 36" stands on the assumption that it never does, and nothing had
checked it.

**Method.** The installed set is taken from `winget export -s winget` rather
than by parsing `winget list`'s columns: export emits `PackageIdentifier`
verbatim, so no column-width guessing can manufacture a wrong id. Installer type
comes from one `winget show` per id. Directory ownership is decided with the
**same rule production uses** -- the segment equals the id, or the id followed by
an underscore.

| installer type | ids | with a directory under a package root |
|---|---|---|
| `burn` | 4 | 0 |
| `exe` | 13 | 0 |
| `inno` | 3 | 0 |
| `msix` | 7 | 0 |
| `msix (zip)` | 3 | 0 |
| `nullsoft` | 1 | 0 |
| **`portable (zip)`** | **4** | **4** |
| `wix` | 6 | 0 |
| **total** | **41** | **4** |

**Non-portable ids owning a package directory: 0. Portable ids owning none: 0.**
The correspondence is exact on all 41.

Five directories exist under `%LOCALAPPDATA%\Microsoft\WinGet\Packages`; the
machine-scope root `%ProgramFiles%\WinGet\Packages` does not exist at all. The
fifth directory belongs to `PhatMT97.VKey.Classic`, which is **not installed** --
the uninstalled-but-still-present case the record already names, sitting one
directory away from `PhatMT97.VKey`, which is installed. That pair is why the
segment rule matters and is now pinned by a test.

**Caveat, because the instrument has one.** `winget show` reports the *available*
manifest's installer type, not the one a package was installed with. The
directory half of the table is read from the filesystem directly and needs no
such assumption; only the type labels do.

## 2. The denominator: three instruments, and they do not agree

| instrument | ids |
|---|---|
| `winget export -s winget` | **41** |
| `winget list`, rows whose Source is `winget`, parsed here | 40 |
| the record's figure, from `dotpkg status --show-unmanaged` plus one declared | 36 |

**The second number is this round's own parser and it is defective in two named
ways**, so it corroborates the first without independently confirming it: it
reads a VCRedist row's version as an id, misses `Warp.Warp`, and cannot parse one
`WinAppRuntime.Main.1.8` row whose Id column holds an MSIX package full name.
39 of the 41 are common to both.

### 2.1 Reconciled, by name, and the two instruments never disagreed

Run on the same machine, the same day, the same tree: `dotpkg status
--show-unmanaged` against a config declaring nothing reports **36** unmanaged
winget ids -- the record's figure, reproduced exactly.

**The difference set is one-sided and it has a mechanism:**

| | |
|---|---|
| `winget export -s winget` | 41 |
| dotpkg's own scan | 36 |
| common | **36** |
| in export, not in dotpkg's scan | **5** |
| in dotpkg's scan, not in export | **0** |

The five are `7zip.7zip`, `Microsoft.UI.Xaml.2.8`,
`Microsoft.VisualStudio.2022.BuildTools`, `Microsoft.WindowsAppRuntime.1.8` and
`Microsoft.WindowsAppRuntime.2` -- **exactly the five that print `installed, but
its state could not be read` in §6**, because winget reports each of them either
at two disagreeing versions or as a `> x.y.z` lower bound, and dotpkg refuses to
guess which.

So **41 = 36 + 5**, and the two counts are not a disagreement at all: one
instrument declines, by design, to count what it cannot establish a fact about.

**What this settles about the shipped claim.** Both denominators are right about
different questions, and each should now say which:

- **4 of 41** ids winget reports installed own a package directory.
- **4 of 36** ids dotpkg can establish a fact about, so **32** is the number of
  packages dotpkg could act on and could not see -- which is exactly the "32 of
  36" the record already used when it called a per-package line a flood.

The record's figures were never wrong; they were about dotpkg's view, and this
document's are about winget's.

## 3. `installed.db`: opened, measured, and priced

The record calls this "the strongest lead found, and deliberately not opened",
and the direction it belongs to was chosen with **measure first, design second**
attached to it. So it was opened, read-only, with `FileShare.ReadWrite` so a
winget process holding the file could not turn a lock into a false negative, and
copied out byte-for-byte with the sha256 verified equal on both machines.

**It is what the record guessed, and better:**

| | |
|---|---|
| bytes | **262144**, matching the record exactly |
| header | `SQLite format 3` |
| tables | **23** |
| `ids` rows | 42 |
| `manifest` rows | 57 |
| **`commands` rows** | **31** |
| **`commands_map` rows** | **35** |
| distinct ids with at least one command | **14** |

A second database beside it, `StoreEdgeFD\installed.db` (225280 bytes), carries
the identical schema and **2 ids, 0 commands** -- it is empty for this purpose.

### 3.1 The coverage it would actually buy

Of the **14** command-bearing ids, only **7** are installed according to
winget's own export. One of those seven, `OpenAI.Codex`, the path signal already
covers.

| | ids |
|---|---|
| covered today by the path signal | **4** |
| would gain a command name from `installed.db` | 7 |
| **union** | **10 of 41** |
| still uncovered | **31** |

The seven, with the commands the database holds for them:

| id | commands |
|---|---|
| `7zip.7zip` | `7z` |
| `Microsoft.PowerShell` | `pwsh` |
| `Microsoft.VisualStudio.2022.BuildTools` | `devenv` |
| `Microsoft.VisualStudioCode` | `code` |
| `OpenAI.Codex` | `codex` |
| `Rustlang.Rustup` | `cargo cargo-clippy cargo-fmt cargo-miri clippy-driver rls rust-analyzer rust-gdb rust-gdbgui rust-lldb rustc rustdoc rustfmt rustup` |
| `gerardog.gsudo` | `gsudo sudo` |

**That last row is a behaviour warning, not decoration.** Fourteen commands for
one package, including `cargo` and `rustc`. Wired into the fence, a `cargo build`
in another window would block every rustup operation, permanently, on the
machine most likely to be running one.

### 3.2 Why it is a second source of truth, measured rather than argued

`installed.db` disagrees with `winget list` **in both directions**:

- **16 ids are in the database and appear in `winget list`'s output not at
  all.** The search was positive-controlled: the identical method finds every id
  the export does report (`Brave.Brave`, `Rustlang.Rustup`, `7zip.7zip`,
  `PhatMT97.VKey`, `OpenAI.Codex` all hit).
- **At least 11 of those 16 are packages *scoop* installed on this machine** --
  `Git.Git`, `OpenJS.NodeJS`, `sharkdp.bat`, `junegunn.fzf`, `Neovim.Neovim`,
  `mvdan.shfmt`, `sxyazi.yazi`, `JesseDuffield.lazygit`,
  `JesseDuffield.Lazydocker`, `Fastfetch-cli.Fastfetch`, `sigoden.AIChat` --
  matched against the 31 directories under scoop's `apps`. The matcher is a name
  heuristic, so **11 is a lower bound**; `GitHub.cli` is very likely scoop's `gh`
  and is not counted because the names do not line up mechanically.
- **15 installed ids are absent from the database entirely**, mostly the MSIX
  ones: `Microsoft.WindowsTerminal`, `Microsoft.Edge`, `Microsoft.Office`,
  `Microsoft.OneDrive` among them.

The risk is not hypothetical and it has a name: item 7's *"two sources of truth
about permitted versions is how a tool starts lying"*. Here the second source
would attribute a **winget** id, and a process name, to a package **scoop**
owns.

**The mitigation that survives the objection, recorded for whoever picks this
up:** never take the *id set* from `installed.db`, only look up *names* keyed by
an id `winget list` already reported. The 16 phantoms are then never consulted.
That reduces the risk to the schema dependency alone — and the price is still
bundling SQLite, cross-compiled to `aarch64-pc-windows-msvc`, against an
undocumented internal schema, for **+6 ids on one machine**.

### 3.3 There is no cheaper route to the same data

Two candidates, both measured dead:

- **`winget show` does not print commands.** Across three packages chosen
  because the database says they carry commands (`Rustlang.Rustup` 14,
  `gerardog.gsudo` 2, `OpenAI.Codex` 1), the fields printed are Author,
  Copyright, Dependencies, Description, Docs, Homepage, Installer, Installer
  SHA256, Installer Type, Installer Url, License, Moniker, Publisher, Release
  Notes, Tags and Version. **No Commands field.** `Moniker` is present and is
  sometimes the command (`gsudo`, `codex`) and sometimes absent (`Rustlang.Rustup`).
- **`WinGet\Links` is portable-only.** All **5** shims resolve to targets under
  `Packages\`, so they name aliases for exactly the packages the path signal
  already covers -- and `PhatMT97.VKey`, itself portable, has no shim at all. At
  best 3 of the 4.

So direction 2 is SQLite plus an undocumented schema, or nothing.

## 4. The declared surface, which is what direction 1 is measured against

a14's real `pkg.toml` (sha256 `32a238ff…`, the file the record has been talking
about for three phases) declares **25 scoop packages and zero winget packages**.

That matters twice. It is why the "a warning for 32 of 36 ids would be a flood"
fear is about the wrong denominator -- a warning attached to *declared* packages
fires **zero** times on a14 as configured. And it is why measuring the shipped
behaviour needs a synthetic `pkg.toml`, exactly as the residual round needed one
to isolate item 21.

## 5. What ships

**One line, on stderr, for a winget package that has a pending change, no
`[winget.guard]` entry, and no package directory** -- which is precisely the
condition under which both halves of the fence are dark. It names the package,
the guesses it would otherwise be matching on, and the entry to add.

Three decisions worth stating, each with the reason it is not the other way:

- **Keyed on a pending change, not on being installed.** A line per unguarded
  installed package is 37 lines on this machine. Phase 5 spent itself deleting
  lines from `status`; a package dotpkg is not about to touch cannot be hurt by
  a fence that cannot see it.
- **`Install` is excluded**: nothing is installed yet, so nothing of it can be
  running.
- **The directory rule is one function, shared with `running_ids`.** Two copies
  that drifted apart would make dotpkg warn about a package the fence covers,
  or stay silent about one it does not. Pinned: loosening it to a bare prefix
  makes `PhatMT97.VKey.Classic`'s directory silence the warning for
  `PhatMT97.VKey`, and **both of those directories exist on a14**.

## 6. Measured live on a14, on the shipping tree

`pkg.toml` declaring all **41** installed ids, `pkg.lock` pinning every one to
`99.0.0` so each is a pending change. a14's real `pkg.toml` was never the input.

**The plan `status` produced: 30 change(s), 9 skipped, 2 winget downgrade(s)
that will be refused, 24 unmanaged.**

| | ids | |
|---|---|---|
| **warned** -- change pending, no guard entry, no package directory | **27** | |
| silent: portable, so the path signal covers it | 3 | `BurntSushi.ripgrep.MSVC`, `OpenAI.Codex`, `ajeetdsouza.zoxide` |
| silent: state could not be read, so there is no action at all | 5 | two disagreeing versions, or a `> x.y.z` version |
| silent: skipped as running -- **the fence working** | 4 | `Brave.Brave`, `Microsoft.PowerShell`, `Microsoft.WindowsTerminal`, `PhatMT97.VKey` |
| silent: refused downgrade | 2 | `Google.Chrome`, `Microsoft.Edge` |
| **total** | **41** | |

Every id is accounted for, and the accounting closes against the plan's own
summary: 27 + 3 = the 30 changes, 5 + 4 = the 9 skipped, 2 = the 2 refusals.

**Positive control, because a warning only ever observed firing is not a
measurement.** Adding `[winget.guard]` entries for three of them drops **27 to
25** -- and the third, `Brave.Brave`, was already silent because it was skipped
as running, which is the same explanation arrived at independently.

**So the number this direction is worth, said plainly: it raises coverage from 4
of 41 to 4 of 41.** It adds none. What it changes is that 27 of the 30 changes
dotpkg was about to make without protection now name the entry that would
provide it, and the 3 it stays quiet about are exactly the ones that need no
entry.

## 7. The live run corrected the code, twice, in ways the tests could not

**A winget downgrade is refused, not performed.** The first version warned on
`Action::Downgrade` and fired on `Google.Chrome` and `Microsoft.Edge`, both
installed ahead of their pin. The sentence it printed -- "dotpkg may downgrade it
while it is running" -- describes something dotpkg is measured unable to do:
the step reaches `execute`, fires `winget install --version <pin>`, and that
command only ever moves a package **up**, returning `NO_AVAILABLE_UPGRADE` with
the step ending `touched: false`. `render`'s summary already counts a winget
downgrade separately from `change_count` for exactly this reason.

**Why no test caught it:** every unit test fed the function an `Upgrade`. The
`Downgrade` arm was written, reviewed and green without any test ever
disagreeing with it. It took a machine with two packages installed ahead of
their pins.

**And the second correction came from the mutation run**, in §8.

## 8. The mutation run, and the machine state beside it

**Machine gated first, and the gate is not a rubber stamp here.** This round's
macOS baseline over three windows reports a range of **11.62 .. 13.62 %** (the
range is what the script prints; the individual rounds are not quoted here
because only the third, 13.62, was read back), so the threshold was set to
**41**, which is what `scripts/idle-baseline.sh` instructs (roughly three times
the observed maximum). That is lower than the 14.42-15.68 % the previous round
measured on this same machine, and the difference is not investigated. At that
same threshold the gate **refused**
this machine twice during the round -- `machine_busy_pct` 20.65 with `node` at
56.2 % of one core, and 19.35 -- and passed at **14.22, VERDICT: IDLE**, which is
the run below.

**First run, `-j 2`: 23 mutants, 4 missed, 17 caught, 2 unviable, 0 TIMEOUT.**
All four were real gaps rather than equivalent mutants:

| mutant | why it survived |
|---|---|
| `unprotected_winget_changes -> vec![]` | the production wrapper reads the environment and **every test drove the roots seam instead** -- the same shape `package_roots()` carried for two phases |
| `unprotected_winget_changes -> vec![String::new()]` | same |
| `unprotected_winget_changes -> vec!["xyzzy".into()]` | same |
| `backend == WINGET` match guard `-> true` | the `Upgrade` arm's copy was pinned by the scoop-upgrade test; **the `Prune` arm's was not**, and both arms carry the same predicate |

Closed the same way `package_roots()` was: one assertion that goes *through* the
wrapper, using a synthetic id so the answer cannot depend on the machine -- with
the variables unset the root list is empty, and with them set nothing is named
that, so both give the same verdict. And one fixture per arm rather than one per
predicate, which is what let the second guard hide behind the first.

**Second run, same scope, same tree: 23 mutants, 21 caught, 2 unviable, 0
missed, 0 TIMEOUT**, exit 0.

**And a third run, which had to move machines and is the one that counts.**
After §8c's fixes the macOS gate refused this machine twelve consecutive times
-- `mediaanalysisd` sat at ~197 % of one core, which is not this session's work
to wait out -- so the run went to a14, where the gate passed at **4.34 %**,
inside the 2.85-5.14 % baseline the record measures for that machine. There:
**23 mutants, 23 caught, 0 missed, 0 unviable, 0 TIMEOUT**, exit 0.

**The unviable count going 5 to 0 is the measured confirmation of §8c.3**, not
a coincidence of platform: the run before the import fix reported 18 caught and
5 unviable, and the run after reports 23 caught. Five mutants that had been
silently exempt are now tested, and all five die.

## 8b. Two findings from looking at Phase 6 rather than at this phase

The independent post-merge audit this round was supposed to open with is
queued and has not run. These two came from working through the prompt's own
list of suspect surfaces while waiting, and both are about the previous round.

### 8b.1 A "still outstanding" item that had been closed 31 minutes after it was written

`docs/measurements-2026-08-12-phase6-citations.md` §12 item 6 states, in the
present tense, that the `docs/` gate reads `git ls-files` and therefore does not
scan a file that has not been `git add`ed.

**It does scan it.** `scripts/check-citations.py`'s `tracked_files` reads a
second list, `git ls-files --others --exclude-standard`, and its own docstring
explains why. Positive control rather than reading: an untracked document
carrying one unresolvable citation was written into `docs/`, the gate was run,
and it reported **37 files scanned** instead of 36 and **failed on that file by
name**. The probe was then removed and the tree confirmed clean.

**The timing is the finding.** The claim was written in `417c798` at 09:22. The
fix landed in `7005168` at 09:53 -- 31 minutes later, in a commit titled *"Close
the docs gate's own blind spot"* -- and `7005168` is **not** an ancestor of
`417c798`. The still-open list was never updated, and the sentence then survived
a whole-branch review by the person who wrote both. That is defect class 1, in
the document whose subject is defect class 1.

### 8b.2 `build.rs`'s fix has no automatic gate, and CI cannot be one

The prompt lists `build.rs` as reviewed by nobody. Reading it turns up no defect;
what it turns up is a question the record does not answer: **what re-checks the
manifest?**

The answer was: nothing. One manual by-content read of twelve binaries, once, on
one machine. And **CI is structurally incapable of noticing**, which is
checkable rather than arguable:

| | |
|---|---|
| CI runs `cargo test --all` on `windows-latest` | yes, from the workflow file |
| `e91f4b1` is an ancestor of `8f08752`, the commit that added `build.rs` | **yes** |
| `tests/update.rs` existed at `e91f4b1` | **yes** |
| `windows-latest` on `e91f4b1` | **green** |

So `update-<hash>.exe` started fine on GitHub's runner with **no manifest at
all**. The runner is not subject to the installer detection that blocks an
ordinary a14 desktop session, so a regression in `build.rs` would leave every
automatic gate green and be noticed only by someone running the suite by hand
from a non-elevated window.

**Closed with a test in the suite**, in `tests/update.rs`, because that file is
the one whose compiled name trips the heuristic -- the binary that would fail to
launch is the one asserting it carries the fix. It runs everywhere the suite
runs, including on the runner that is blind to the symptom.

### 8b.3 And that test could not fail

Written, formatted, clippy-clean, green on macOS and on a14. Then built against
a **deliberately neutered `build.rs`** in an isolated directory on a14, and it
**passed**.

The check is a byte search of the running executable for the level string an
embedded manifest writes into its resources. The first version spelled that
string literally in its own assert message, so the search always found the
test's own copy. **A gate reporting success while the thing it guards was
gone** -- defect class 2, written inside the round whose document quotes that
class, in the test written to close a fourth-class hole.

The literal now appears nowhere in that file; it is rebuilt at run time from a
byte-shifted copy, and the reconstruction is itself pinned, because a typo in
the shifted table would produce a test that always fails for the wrong reason --
the back side of the same class. Re-run against the same neutered build: **FAILED
at the intended assertion, naming the binary.** Restored, `git diff build.rs`
empty, re-run: passes.

**Nothing about that test looked wrong.** It was caught only because a gate is
not accepted here until it has been watched failing.

### 8b.4 The three-runs claim: checked, and it is stronger than it was stated

The prompt ranks this one by consequence -- if `src/sys.rs` does not really need
three mutation runs, the conclusions recorded for still-open items 15 and 19 go
with it. Checked on two levels rather than taken on trust.

**The six mutants still exist, at the lines the record names.** `cargo mutants
--list --file src/sys.rs` on this tree reports 15 mutants, and the six the claim
is about are all present: three at `:139`, one at `:163`, two at `:216`.

**The arithmetic was re-derived from the recorded table by machine, not by
eye:**

| sessions | mutants covered | missed |
|---|---|---|
| elevated Windows alone | 3 of 6 | |
| ordinary Windows alone | 2 of 6 | |
| macOS alone | 2 of 6 | |
| elevated + ordinary | **4 of 6** | both `:216` |
| elevated + macOS | **5 of 6** | `:139 -> Some(true)` |
| ordinary + macOS | **4 of 6** | `:139 -> Some(false)`, `:163` |
| **all three** | **6 of 6** | nothing |

So *three runs suffice* and *no two suffice* both hold.

**And the claim is stronger than the record states it.** It is presented as an
empirical result about three particular runs. Two of the three gaps are
**structural** and could not have come out otherwise: `:216`'s pair is in the
`cfg(not(windows))` arm, so no Windows build ever *compiles* it, and
`:139 -> Some(true)` is an equivalent mutant in an elevated session because
`Some(true)` is the correct answer there. Only the third gap -- `:163` and
`:139 -> Some(false)` needing elevation -- rests purely on measurement.

**No finding here.** Recorded because "checked and found sound" is a result, and
because an audit that only ever reports problems tells a reader nothing about
what was looked at.

### 8b.5 A rule stated in a file, broken in the same file, gated by nothing

Three of the four PowerShell scripts open with the same sentence:

> *no backtick appears anywhere in this file, including in comments. A backtick
> inside a comment is not a parse error, so a parse-check passes a file a
> backtick-check would fail; **both gates exist and both must run**.*

**Both gates did not exist.** The parse-check did. Nothing anywhere in the
repository ever looked for a backtick -- not the suite, not CI, not a script.

**And the sentence was false about the file that states it most fully.**
`scripts/idle-gate.ps1` carried a backtick in a comment, which is precisely the
case its own header describes, and it survived the round that wrote that header
and the whole-branch review after it.

Found by accident, which is worth recording rather than dressing up: this round
copies that script to a14 under its own prefix, and the round's ad-hoc
parse-and-backtick checker globbed it along with its own files.

**Closed with a real gate**, CI-side rather than in the suite, for the reason
`scripts/check-citations.py` gives about `docs/`: the Windows shipping tarball
carries `Cargo.toml`, `Cargo.lock`, `build.rs`, `src/` and `tests/` and **not**
`scripts/`, so a suite test reading `scripts/` would either fail on every
Windows run or tolerate the directory being absent -- and tolerating that is how
a gate starts scanning nothing. Confirmed able to fail three ways: red on the
real pre-existing violation before it was fixed, red again on a backtick
injected into a different script, and it refuses a run that finds zero files.

## 8c. What the post-merge review found in this branch, and what was done

The cloud review of the Phase 7 branch returned two findings. Both are real;
one had been found independently by this branch's own whole-branch review
minutes earlier, which is corroboration rather than duplication.

### 8c.1 The advice the warning printed did not parse

**Severity as reported: nit. Consequence: the feature's entire output was
unusable.** The warning said `add Google.Chrome = ["chrome"] under
[winget.guard]`. Every real winget id contains a dot, and an unquoted dotted key
in TOML is a *table path*, so dotpkg's own parser rejects it with `invalid type:
map, expected a sequence`. The diagnosis was right and the paste failed on
essentially every id the warning could ever name.

**Why the three existing tests did not catch it:** all three asserted
*substrings* of the message. A substring assertion cannot tell valid TOML from
invalid TOML. The fix is pinned by a test that lifts the suggested line out of
the sentence and feeds it to `config::parse`, then asserts the guard entry lands
under the package the warning named -- the round trip is the check, not the
spelling.

**Confirmed on the machine rather than only in the suite.** Re-run on a14
against the same 41-id config, all **27** emitted suggestions carry a quoted
key and **0** carry a bare one. One of them, verbatim, and it happens to be the
case that justifies the whole feature:

> `pkg.toml [winget.guard] AutoHotkey.AutoHotkey: winget created no package
> directory for this id, so dotpkg cannot recognise its processes by path, and
> the only names it will match are guesses ("autohotkey"). If it runs under any
> other name, add "AutoHotkey.AutoHotkey" = ["<process name>"] under
> [winget.guard] -- otherwise dotpkg may upgrade it while it is running`

The guess it names is `autohotkey`. The record measured the real process as
**`autohotkey64`**. So on this package the warning is not hypothetical: the
fence is dark, the guess is wrong, and the sentence is the only thing that says
so.

### 8c.2 The root read was O(actions), and acting on it uncovered something worse

The directory scan ran once per pending change -- up to 60 `read_dir` calls on
the measured plan where 2 suffice. Hoisted, with the matching rule left in the
one shared function so the hoist could not put a second copy of it at the call
site, and the directory test confirmed to still bite through the new path.

**The review's reasoning for this one contains a factual slip**, recorded
because accepting a finding's conclusion is not the same as accepting its
argument: it says `has_package_dir` has an *"other caller `running_ids`"*.
It does not. `running_ids` shares only `segment_names_id`; `has_package_dir` had
exactly one caller. The observation stands; the justification for the shape it
proposed did not.

### 8c.3 And the mutation run's "0 missed" was partly an artifact

Acting on 8c.2 added a function, so the mutation run was repeated -- and reading
its **breakdown** rather than its summary line showed the three mutants for the
new function coming back **UNVIABLE, not caught**. cargo-mutants writes
`BTreeSet::new()` unqualified; the file spelled the type as
`std::collections::BTreeSet` with no import, so the replacements did not
compile.

**An unviable mutant is evidence about the build, not about the tests.** A
function written that way is silently exempt from mutation testing while the
summary still reports `0 missed`.

**And `running_ids` had the same signature shape**, so the same exemption
applied to the function the entire winget path signal rests on -- the one three
phases of documents describe as measured. One import, and both now use the short
name, so the generated replacements build and the runner can do its job.

## 9. Verification

**The tree is `b445482`**, and every figure here was derived on it unless
attributed otherwise above.

- **macOS**: `cargo test --all` **658 passed / 0 failed**, **15** `test result:`
  lines, `--list` agrees at **658**. `cargo fmt --check` clean,
  `cargo clippy --all-targets -D warnings` clean.
- **The `cfg` difference set is now five, not four**, and this is called out
  because "exactly four" is quoted in several documents and has been stable for
  three phases. §8b.2's gate is `#[cfg(windows)]` and, unlike the two elevation
  tests, is **not** `#[ignore]`d -- it runs. Two `#[cfg(unix)]` tests are absent
  on Windows and **three** `#[cfg(windows)]` tests are absent on macOS.
- **The `docs/` gate passes**: 35 files scanned, every citation resolving. The
  total is deliberately not quoted here, for §4's reason in the previous round's
  document -- this file adds citations of its own.
- **Windows**, on `82b7bc0`, shipped as a tarball carrying `SHIPPING-SHA.txt` and a
  `SHIPPING-MANIFEST.txt` naming a sha256 for all **74** files:
  `manifest entries : 74     verified equal : 74`,
  `mismatched : 0    missing : 0    unlisted on disk : 0`.

  **74 against the previous round's 73, reconciled rather than absorbed:** no
  file was added -- `git ls-tree` counts **73** shipped files at `8f087524` and
  73 today, and `git diff --diff-filter=A` between them is empty. This round's
  manifest also hashes `SHIPPING-SHA.txt`; the previous one did not.
- **Windows suite** on `82b7bc0`: `cargo test --no-fail-fast`
  **exit 0, 657 passed / 0 failed / 2 ignored**, **15** `test result:` lines,
  `--list` **659**.
- **The rebuild was proved by content, not by its own report.** `--list` on the
  machine returns a test name that exists only in this tree
  (`a_winget_change_dotpkg_cannot_see_by_path_and_has_no_guard_entry_for_is_reported`),
  which a skipped rebuild could not print.
- **Name by name, from `--list`, never by subtracting totals**: macOS **658**,
  Windows **659**, common **656**, difference set **5**. The two `#[cfg(unix)]`
  tests absent on Windows, and three absent on macOS: the two
  `#[cfg(windows)] #[ignore]` elevation tests and §8b.2's manifest gate, which
  is not `#[ignore]`d.
- **Every test was confirmed able to fail**, by neutering the production clause
  it guards, one at a time -- the guard-table check, the package-directory
  check, the shared segment rule, the `Install` exclusion, the backend guard and
  the `Prune` arm. Both edited files were then restored and the restoration
  **verified by sha256 against a copy taken beforehand**, not asserted.
- **`build.rs` is in the manifest** and was checked by name at packaging time,
  because a missing build script is not an error -- cargo silently builds
  without it.

## 10. Method failures of my own, this round

1. **I wrote "no backtick appears anywhere in this file" into two PowerShell
   scripts and then put a backtick in each.** Once in a `.Append()` call, once
   inside a comment -- the exact shape the record warns about, in the exact
   round that quotes the warning. Both were caught by the separate
   backtick-check, which is why it exists as a second check beside the parse
   check.
2. **`grep -c` exits 1 when the count is zero**, so `grep -c … && scp …` silently
   skipped the upload and the script never reached the machine. Found because
   the remote checker listed one file fewer than expected -- output, not exit
   code, again.
3. **I reported `${PIPESTATUS[0]}` in zsh**, where it is empty, so an `scp`
   whose success I claimed to have checked was not checked at all. Verified
   afterwards by listing the far end.
4. **Three separate attempts at inline PowerShell over ssh died in a parser
   cascade** because the local shell ate `$` and the quotes. It is not a
   quoting problem to be solved; the working form is a `.ps1` file, and I went
   back to it three times before writing that down.
5. **I predicted 37 warnings and measured 29, then 27.** The prediction ignored
   that `status` produces no action at all for a package whose state cannot be
   read, and that a package the fence *catches* is skipped rather than warned
   about. Both are the code being right and the prediction being wrong -- but
   the gap is exactly the size that would have been comfortable to round away.

## 11. Still outstanding

1. ~~**The 41-versus-36 disagreement is not reconciled by name.**~~ --
   **closed inside this round, §2.1.** 41 = 36 + 5, the five are exactly the
   ones dotpkg refuses to read a version for, and dotpkg's scan holds nothing
   winget's export does not. Both denominators are correct about different
   questions and each now says which.
2. **The two `main.rs` call sites are unpinned.** `tests/cli.rs` hands every
   spawned binary a `PATH` with winget removed *by construction*, so no
   integration test can produce a winget action at all, and nothing goes red if
   someone deletes either call. This is the same recorded residual
   `sample_fence` carries for the same reason, and it is numbered here rather
   than left implicit.
3. **Direction 2 is priced but not decided.** +6 ids of 41, against SQLite
   cross-compiled to `aarch64-pc-windows-msvc` and an undocumented schema. The
   figures are from **one machine**; a second machine could move them either
   way, and the `Rustlang.Rustup` row suggests the fence would need a rule about
   how many commands are too many.
4. **Nothing re-checks `build.rs` outside the suite, and the suite's check is
   Windows-only.** §8b.2's gate runs wherever the suite runs, which now includes
   CI, but CI's runner cannot reproduce the symptom it guards, so the gate
   proves the manifest is *embedded* and never that the failure is *absent*.
   The two are different claims and only the first is tested.
5. **The warning has never been observed on a machine whose real `pkg.toml`
   declares a winget package**, because a14's declares none. The measurement
   above uses a synthetic config, which is the same standing this project gives
   any structurally-verified, live-unverified path.
6. **Whether `installed.db`'s `commands` table is populated the same way on a
   machine that installed those packages *through* winget is unmeasured.** On
   a14, 11 of the 16 phantom entries correlate to scoop installs, so the
   population mechanism observed here may be ARP correlation rather than winget
   installation.
7. **Item 17 and item 20 are untouched by this round**, and item 9 is closed
   only in its second half -- there is still no scan-time source for a
   package's process names that this round is willing to ship.
