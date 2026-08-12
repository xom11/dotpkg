#!/usr/bin/env python3
"""The backtick gate for `scripts/*.ps1`.

WHY THIS EXISTS, AND WHY IT IS NOT A TEST IN THE SUITE.

Three PowerShell scripts in this repository open with the same sentence: *"no
backtick appears anywhere in this file, including in comments. A backtick inside
a comment is not a parse error, so a parse-check passes a file a backtick-check
would fail; both gates exist and both must run."*

Both gates did not exist. The parse-check did; nothing anywhere in the
repository ever looked for a backtick. And the sentence was false about the file
that states it most fully: `scripts/idle-gate.ps1` carried one in a comment,
which is exactly the case its own header describes, and it survived the round
that wrote the header plus a whole-branch review.

That is the shape this repository has paid for four times: a convention asserted
in prose, believed because it is written down, and enforced by nobody.

NOT A TEST IN THE SUITE, and the reason is the same one `tests/citations.rs`
gives for its own scope. The Windows shipping tarball carries `Cargo.toml`,
`Cargo.lock`, `build.rs`, `src/` and `tests/` -- it does not carry `scripts/`.
A suite test reading `scripts/` would either fail on every Windows run or have
to tolerate the directory being absent, and "tolerate it being absent" is how a
gate quietly starts scanning nothing. CI has a full checkout, so the check lives
here, beside `check-citations.py`, which made the same call for `docs/`.

WHY THE SELF-REFERENCE TRAP DOES NOT APPLY HERE. A gate that searches for a
string it also contains can be satisfied by its own copy -- this branch shipped
exactly that bug once, in a test looking for a manifest string it spelled in its
own failure message. This file contains the character it searches for and is
safe from that, because it scans `.ps1` files and is not one. It is stated
rather than left to be noticed.

Usage:  python3 scripts/check-ps1-style.py
Exit 0 = no backtick in any scanned file.  Exit 1 = at least one, listed.
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BACKTICK = chr(96)


def main() -> int:
    files = sorted(ROOT.glob("scripts/*.ps1"))

    problems: list[str] = []
    for path in files:
        text = path.read_text(encoding="utf-8", errors="replace")
        for lineno, line in enumerate(text.splitlines(), start=1):
            if BACKTICK in line:
                rel = path.relative_to(ROOT)
                problems.append(f"  {rel}:{lineno}  {line.strip()[:100]}")

    # A gate whose output narrates its own result is this project's fourth
    # defect class, so the scope is printed either way, and finding no files at
    # all is treated as a broken gate rather than a clean tree.
    print(f"scanned {len(files)} PowerShell script(s) under scripts/")
    if not files:
        print("FAIL: found no .ps1 files at all -- the gate is not looking where it thinks it is")
        return 1

    if problems:
        print(f"FAIL: {len(problems)} backtick(s), in files whose own headers forbid them:")
        print("\n".join(problems))
        print(
            "\nA backtick is PowerShell's line-continuation and escape character, so one that "
            "drifts out of a comment changes what the script means without changing how it "
            "parses. Remove it, or say the word without the quoting."
        )
        return 1

    print("OK: no backtick in any scanned script")
    return 0


if __name__ == "__main__":
    sys.exit(main())
