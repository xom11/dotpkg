#!/usr/bin/env python3
"""The citation gate for `docs/`.

WHAT THIS DECIDES, AND WHAT IT DELIBERATELY DOES NOT.

It decides one thing, totally: every `file:line` citation in `docs/` names a
file that exists in the repository, names exactly one such file, and names a
line that file has. Before this gate ran for the first time, 38 citations named
a file no reader could open (`design.md` with no directory, the untracked
`.superpowers/` ledger, throwaway probe scripts) and 10 named a basename two
tracked files share -- `execute.rs:223` resolves to `src/execute.rs` or
`tests/execute.rs` depending only on who is guessing, and the two are 1300 lines
apart.

It does NOT decide whether a citation still points at what its sentence claims.
That was measured before it was rejected: anchoring every citation to the content
it pointed at in the commit that wrote it fires on **221 of 421** citations
repo-wide, and almost all of those are legitimate -- `docs/plans/` and
`docs/specs/` cite code that did not exist when the plan was written, and
`docs/phase3-notes.md` is a closed record whose citations were true about Phase
3's tree. A gate needing a 221-entry allowlist is a gate that gets switched off.

The class is closed at the source instead, in the one place it can be: `src/`
and `tests/` may no longer contain a line citation at all (`tests/citations.rs`
enforces that, in the suite, on both platforms). This gate keeps the remaining
435-odd historical citations honest about *which file* they mean, which is the
part a reader needs and the part a machine can settle.

Usage:  python3 scripts/check-citations.py
Exit 0 = every citation resolves.  Exit 1 = at least one does not, listed.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Extensions a citation can name. Not "anything with a dot": `sysinfo 0.32`
# must not read as a citation, and neither must a version in a lockfile.
EXTENSIONS = "rs|md|toml|txt|json|lock|ps1|cmd|yml|yaml"

CITATION = re.compile(
    r"(?<![-A-Za-z0-9_/])"
    rf"([A-Za-z0-9_][-A-Za-z0-9_./]*\.(?:{EXTENSIONS}))"
    r":(\d+)(?:-(\d+))?"
)


def tracked_files() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True
    )
    return [line for line in out.stdout.splitlines() if line]


def main() -> int:
    tracked = tracked_files()
    tracked_set = set(tracked)
    by_basename: dict[str, list[str]] = {}
    for path in tracked:
        by_basename.setdefault(Path(path).name, []).append(path)

    line_counts: dict[str, int] = {}

    def line_count(path: str) -> int:
        if path not in line_counts:
            text = (ROOT / path).read_text(encoding="utf-8", errors="replace")
            n = text.count("\n")
            if text and not text.endswith("\n"):
                n += 1
            line_counts[path] = n
        return line_counts[path]

    def resolve(cite: str) -> tuple[str | None, str]:
        if cite in tracked_set:
            return cite, "exact"
        if "/" in cite:
            hits = [p for p in tracked if p.endswith("/" + cite)]
        else:
            hits = by_basename.get(cite, [])
        if len(hits) == 1:
            return hits[0], "unique"
        if len(hits) > 1:
            return None, "ambiguous: " + ", ".join(sorted(hits))
        return None, "no such file in the repository"

    docs = [p for p in tracked if p.startswith("docs/")]
    problems: list[str] = []
    total = 0

    for doc in docs:
        text = (ROOT / doc).read_text(encoding="utf-8", errors="replace")
        for lineno, line in enumerate(text.splitlines(), start=1):
            for match in CITATION.finditer(line):
                total += 1
                cite, start, end = match.group(1), int(match.group(2)), match.group(3)
                highest = int(end) if end else start
                target, how = resolve(cite)
                if target is None:
                    problems.append(f"  {doc}:{lineno}  ->  {match.group(0)}  ({how})")
                    continue
                have = line_count(target)
                if highest > have:
                    problems.append(
                        f"  {doc}:{lineno}  ->  {match.group(0)}  "
                        f"(resolves to {target}, which has {have} lines)"
                    )

    # A gate whose output narrates its own result is this project's fourth
    # defect class, so the scope is printed whether it passes or fails, and a
    # zero-citation scan is treated as a broken gate rather than a clean tree.
    print(f"scanned {len(docs)} files under docs/, {total} file:line citations")
    if total == 0:
        print("FAIL: found no citations at all -- the gate is not looking where it thinks it is")
        return 1

    if problems:
        print(f"FAIL: {len(problems)} citation(s) do not resolve:")
        print("\n".join(problems))
        print(
            "\nFix by naming the directory (`src/execute.rs:223`, not `execute.rs:223`), "
            "or by dropping the line number when the file is not in the repository."
        )
        return 1

    print("OK: every citation names one tracked file and a line that file has")
    return 0


if __name__ == "__main__":
    sys.exit(main())
