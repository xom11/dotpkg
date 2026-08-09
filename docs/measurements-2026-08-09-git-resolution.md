# Measurements: what git actually answers when you ask a bucket a question

Measured 2026-08-09 on the macOS development machine, `git version 2.51.0`,
against **fabricated** repositories built by
`scratchpad/probe-git.sh`, `probe-git2.sh` and `probe-batch.sh`. Every repo in
this document was constructed commit by commit, so every answer below is
checked against ground truth this document's author built, not against what
git says it built.

This is the raw record behind
`docs/specs/2026-08-09-phase3-update-adopt-design.md`, "The measurement that
sets the definition of `commit`".

**What this document does not measure.** No real scoop bucket was used. The
`main` bucket has ~78,000 commits and manifests of 1–3 KB; the largest repo
here has 400 commits and manifests under 200 bytes. So **nothing here is a
timing result for a real bucket** — the spawn-count comparison in section H is
a count, and its wall-clock figures are only meaningful as a ratio. Real
timings are a dogfood question for a14, listed as such in the design.

## Method, and the first run that had to be thrown away

The first run of `probe-git.sh` called its repo-creating helper as
`D=$(newrepo linear)`. Command substitution runs in a subshell, so every `cd`
was discarded and every subsequent git command ran outside a repository. It
printed a full page of confidently formatted results — `update == HEAD? : yes`,
`expected 1.0.2 -> correct`, `byte-match picks: -> CORRECT, disambiguated` —
every one of them produced by comparing two empty strings, interleaved with
`fatal: not a git repository` on stderr.

Recorded rather than quietly fixed, because it is the same shape this project
has now hit three times: **a check that runs, reports success, and demonstrates
nothing.** The tell was not in the results; it was that the results were
*unanimous*.

Two further sections were discarded on the second run and redone:

- **Section D was contaminated.** It renamed `bucket/Tool.json` to
  `bucket/tool.json` to test rename handling — on a case-insensitive macOS
  filesystem, where git never saw a rename at all. Its "2 commits" answer
  measured nothing. Redone as D′ with two genuinely different filenames.
- **Section F was broken.** `git rm` removed the last file in `bucket/`, and
  therefore the directory, so the re-add commit failed and its result was read
  off a repository in a state the test did not intend. Redone as F′.

## A. Linear history — the base case

Four commits touching `bucket/fzf.json` (versions 1.0.0, 1.0.1, 1.0.2), with
unrelated `bucket/bat.json` churn interleaved, `bat` last.

```
update picked            : 3f3354ef  (version 1.0.2)   <- ground truth for 1.0.2
bucket HEAD              : 360e2e24                    <- the bat commit
update == HEAD?          : NO
blob(update) == blob(HEAD)? : YES
adopt walk for 1.0.0     : 8abcd3d0   correct
adopt walk for 1.0.1     : eb186dca   correct
adopt walk for 7.7.7     : correctly not found
```

`git log -1 --format=%H -- <path>` is per-file, not the bucket tip. That is the
whole reason `pkg.lock` records a commit per package rather than one commit per
bucket.

## B. A merge commit hides a version from the default walk

`tool.json` 1.0.0 on `main`; a side branch takes it to 1.0.1; `main`
independently takes it to 1.0.2; the merge resolves to main's content.

```
side commit (1.0.1)      : 87475985
merge commit             : 35184b8d
HEAD version             : 1.0.2

update picked            : 7ae35d6e (1.0.2)
blob(update) == blob(HEAD)? : YES

git log -- bucket/tool.json   (default simplification):
  7ae35d6  tool 1.0.2 on main
  676b9c9  tool 1.0.0

adopt walk for 1.0.1     : NOT FOUND
adopt walk for 1.0.1, --full-history : 87475985
```

**A version that reached the bucket only through a branch whose change was
superseded at merge time is invisible to `git log -- <path>`.** Default history
simplification follows a single TREESAME parent through the merge and never
walks the other side. `adopt` searching for a version a user actually has
installed would report "not in this bucket's history" about a commit that is a
genuine ancestor of HEAD.

## B′. …but `--full-history` is wrong for `update`

Same shape, rebuilt.

```
update, plain             : d62fdf1f (1.0.2)   <- ground truth: the commit that made 1.0.2
update, --full-history    : 0d8093e6 (1.0.2)   <- the MERGE commit
same commit?              : NO
blob(--full-history) == blob(HEAD)? : YES

adopt, plain,          1.0.1 : NOT FOUND
adopt, --full-history, 1.0.1 : d058f682
adopt, --full-history, 1.0.2 : 0d8093e6   (ground truth was d62fdf1f)
```

So the two flags cannot be shared. `--full-history` is **required** for `adopt`
(without it, a real version is unreachable) and **wrong** for `update` (it
names the merge rather than the commit that produced the version).

The last line is the important one: even for `adopt`, `--full-history` returns
a commit that merely *carries* 1.0.2, not the one that authored it. Its blob is
identical, so a manifest recovered from it is byte-for-byte the manifest the
authoring commit produced.

## C. Two commits, one version, different content

The rev-locking shape the Phase 2b-2 design is built around: a bucket amends a
manifest's url/hash without bumping the version.

```
older 2.0.0               : bad3d81f
newer 2.0.0               : ef91d3c8

adopt by VERSION picks    : ef91d3c8   (the newer one)
adopt by installed BYTES  : bad3d81f   (correct — disambiguated)
adopt by installed bytes, RAW, manifest rewritten to CRLF : NO MATCH
```

Two results, both load-bearing:

1. **Matching on version alone pins a machine to content it is not running.**
   A machine installed from the older commit gets a lock naming the newer one.
   The 2b-1 rehearsal script matched on version only, so this is a defect in
   the rehearsal, not in anything shipped.
2. **Matching on bytes requires normalisation.** scoop rewrites line endings
   when it copies a manifest into `apps/<app>/current` (measured separately,
   `docs/dogfood-phase2b2-2026-08-09.md`), so a raw byte comparison against a
   bucket blob finds nothing at all. `verify::normalise` already exists and
   already handles exactly this; it must be reused rather than re-derived.

## D′. A real rename, and where the walk actually lands

`bucket/old-name.json` at 1.0.0, renamed to `bucket/new-name.json`, then taken
to 1.0.1.

```
git log -- bucket/new-name.json :
  d74b20ea  new-name 1.0.1                        -> version 1.0.1
  998f0e3b  rename old-name.json -> new-name.json -> version 1.0.0

adopt walk for 1.0.0, plain      : FOUND at 998f0e3b   (the RENAME commit)
adopt walk for 1.0.0, --follow   : FOUND
ground truth (authoring commit)  : 03b41ed1, where the file was bucket/old-name.json
```

`--follow` is not needed: the rename commit itself carries the correct content
under the new path, so the plain walk already finds a usable commit. It is
**not** the commit that authored 1.0.0.

This is the third independent instance — with B′ and by construction in C — of
the same fact, which is why it becomes a definition rather than a caveat:

> **The commit a lock records is a commit at which the manifest has the pinned
> content. It is not a claim about which commit authored that version.**

`Scoop::stage` never needed the stronger claim; it does `git show
<commit>:bucket/<app>.json` and checks the version field.

## E. `ls-tree` at the locked commit returns the historical spelling

Same repository as D′'s case-only sibling, where the bucket's filename changed
case:

```
git ls-tree --name-only HEAD  bucket/  ->  bucket/tool.json
git ls-tree --name-only $OLD  bucket/  ->  bucket/Tool.json
```

`Scoop::stage`'s `resolve_spelling` passes the **locked commit**, not HEAD
(`src/backend/scoop.rs`), so it already resolves against the tree it is about
to read. The 2b-1 rehearsal script listed `git ls-tree ... HEAD` and would have
missed a historical spelling.

Shipped code is right here, for a reason no document previously stated.

## F′. Deleted, then re-added

```
update picked            : c7bf46a4 (2.0.0)  correct
blob(update) == blob(HEAD)? : YES
adopt walk for the pre-deletion 1.0.0 : c159866f  == ground truth
```

Neither algorithm is confused by a gap in the file's history.

## G. A shallow clone

`git clone --depth 1` of the section-A repository.

```
is-shallow-repository    : true
update picked            : 360e2e24 (1.0.2)   <- the grafted tip, not the per-file commit
adopt walk for 1.0.0     : NOT FOUND — and git printed nothing about why
```

`update` degrades gracefully: the tip is the only commit there is, and its blob
is the current content, so the recorded pin is still correct. `adopt` fails
**silently** — indistinguishable, from its output alone, from "this version was
never in this bucket".

`scoop bucket add` clones in full (measured 2026-08-08,
`docs/measurements-2026-08-08-scoop-exit-codes.md`, section M8), so this is not
the ordinary case — but a bucket a user cloned by hand is not covered by that
measurement, and `git rev-parse --is-shallow-repository` answers it for one
process spawn.

## H. Cost: `git show` per candidate versus one `git cat-file --batch`

A synthetic repository of 400 commits to one manifest, searching for a version
at **position 394** — deliberately near the bottom, the worst case for a walk
that stops at the first match.

| Method | Processes spawned | Wall | Answer |
|---|---|---|---|
| `git show <commit>:<path>`, once per candidate (the 2b-1 rehearsal) | **395** | 3.16 s | `09f1483a` |
| `git log` once, then **one** `git cat-file --batch` fed every `<commit>:<path>` | **2** | 0.02 s | `09f1483a` |

Identical answer, **153×** on this repository. The number that transfers is the
process count, not the ratio: these manifests are under 200 bytes and this
history is 400 commits, whereas the real `main` bucket is ~78,000 commits with
1–3 KB manifests, on Windows, where process creation is far more expensive than
on macOS.

`git cat-file --batch` reads `<object>` specs on stdin and writes
`<sha> <type> <size>\n<contents>\n` per spec, in order, so the Nth response
belongs to the Nth commit. A spec whose path is absent from that commit gets a
`<spec> missing` line instead, which is why the parser must key on the header
shape rather than assume one response per request has a body.

## What still has to be measured on a14

- How long `update` takes across 25 declared packages against the real
  `main`/`extras` buckets. Section H is synthetic.
- How deep the per-file history actually is for a real package, which is what
  decides whether `adopt`'s walk is instant or slow.
- Whether any of the 25 declared packages exists in more than one declared
  bucket. The design refuses on ambiguity; if the real answer is 0 of 25, that
  refusal never fires and the `[scoop.opts] bucket` field it points at is
  documentation rather than a working path.
- Whether a real bucket contains a merge of the shape section B constructs. B
  proves the walk *can* miss a version; it does not prove that a scoop bucket
  ever does.
