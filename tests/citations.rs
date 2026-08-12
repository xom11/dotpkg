//! The citation gate for `src/` and `tests/`.
//!
//! # Why this file exists
//!
//! A `file:line` citation in a comment is a claim about the Nth line of a file,
//! and any later commit that inserts a line above the target falsifies it
//! without touching the sentence. Nothing in the build can notice: the sentence
//! is still grammatical and the line number is still a number. This repository
//! recorded that class three times on one branch, fixed it twice in `src/` and
//! once in `tests/`, and **it came back anyway** -- a whole-tree re-check on
//! `3666d38` found six live drifted citations in shipped `.rs` files, two of
//! them the *same two citations* an earlier phase had already corrected once.
//!
//! The single gate that existed before this one asked "does the cited line
//! exist". All six survivors passed it, because a line that has drifted still
//! exists. A content check cannot replace it either: anchoring each citation to
//! what it pointed at in the commit that wrote it was measured across the whole
//! repository and **221 of 421 citations fire**, most of them legitimately --
//! `docs/plans/` cites code that had not been written yet, and `docs/phase3-notes.md`
//! is a closed record. A gate needing a 221-entry allowlist is a gate that dies.
//!
//! # The rule this enforces instead
//!
//! **A citation into code names a symbol, never a line.** A symbol does not
//! drift when a line is inserted above it, so there is no number left to go
//! stale -- and the gate is then total rather than heuristic: it does not have
//! to judge whether a citation is still right, because the shape that can go
//! wrong is simply absent. `docs/phase5-notes.md` had already found this by
//! hand ("named rather than cited by line on purpose ... a test name does not
//! drift"); it was a convention nobody enforced, and 32 line citations survived
//! it. A convention that is only ever asserted in prose is not a gate.
//!
//! Write `` `src/plan.rs::plan_backend` `` where a line number used to go, or
//! just name the symbol when the file is obvious from context.
//!
//! # Scope, stated because a sweep that does not state its scope reads as
//! covering everything
//!
//! This test covers `src/` and `tests/` only. Those two directories are exactly
//! what the Windows shipping tarball carries, so this gate runs on both
//! platforms and in the shipped tree. `docs/` is **not** covered here and is not
//! covered by any test -- see `scripts/check-citations.py`, which is the gate
//! for that directory and reports its own counts.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Extensions a citation can name. Deliberately not "anything with a dot": a
/// version like `sysinfo 0.32` must not read as a citation.
const CITED_EXTENSIONS: &[&str] = &["rs", "md", "toml", "txt", "json", "lock", "ps1", "cmd"];

/// A citation that is true about a NAMED PAST TREE and must not be re-pointed
/// to this one. `tests/cli.rs` carries the only instance today: mutation
/// survivors reported against Phase 3's `58c8e29`, kept so the report in
/// `docs/phase3-notes.md` stays findable. "Correcting" those to this tree's
/// numbers would turn a true statement about Phase 3 into a false one.
///
/// The marker must name the tree it is true about; a bare "historical" with no
/// sha is the same unfalsifiable sentence in a different costume.
const HISTORICAL_MARKER: &str = "HISTORICAL, DO NOT RE-POINT";

/// How far the marker's exemption reaches. A marker exempts the comment block
/// it introduces, not the rest of the file.
const HISTORICAL_WINDOW: usize = 12;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_and_text_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_and_text_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn covered_files() -> Vec<PathBuf> {
    let root = repo_root();
    let mut out = Vec::new();
    rust_and_text_files(&root.join("src"), &mut out);
    rust_and_text_files(&root.join("tests"), &mut out);
    out.sort();
    assert!(
        out.len() > 20,
        "the gate found only {} files under src/ and tests/ -- it is not looking where it thinks \
         it is, and a gate that scans nothing passes everything",
        out.len()
    );
    out
}

/// Split a candidate token into (path, rest) at the first extension boundary.
/// Returns `None` when the token does not end a known extension.
fn split_at_extension(token: &str) -> Option<(&str, &str)> {
    for ext in CITED_EXTENSIONS {
        let needle = format!(".{ext}");
        if let Some(idx) = token.find(&needle) {
            let (path, rest) = token.split_at(idx + needle.len());
            // `.rs` inside `.rstuff` is not an extension.
            let next = rest.chars().next();
            if next.is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_') {
                return Some((path, rest));
            }
        }
    }
    None
}

/// Every whitespace/punctuation-delimited token in a line that could be a
/// citation: it contains a dot and at least one of `:` after it.
fn tokens(line: &str) -> Vec<&str> {
    line.split(|c: char| {
        c.is_whitespace() || matches!(c, '`' | '(' | ')' | '[' | ']' | '"' | ',' | ';' | '*')
    })
    .filter(|t| t.contains('.') && t.contains(':'))
    .collect()
}

fn historical_lines(text: &str) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    for (i, line) in text.lines().enumerate() {
        if line.contains(HISTORICAL_MARKER) {
            for offset in 0..HISTORICAL_WINDOW {
                out.insert(i + offset);
            }
        }
    }
    out
}

#[test]
fn no_citation_in_src_or_tests_names_a_line_number() {
    let root = repo_root();
    let mut offenders = Vec::new();
    let mut exempted = 0usize;
    let mut scanned = 0usize;

    for file in covered_files() {
        let text = std::fs::read_to_string(&file).expect("a file the walker just listed");
        let historical = historical_lines(&text);
        for (i, line) in text.lines().enumerate() {
            for token in tokens(line) {
                let Some((path, rest)) = split_at_extension(token) else {
                    continue;
                };
                // A doubled colon is the required form and names a symbol; a
                // single colon followed by digits is the banned one. (The
                // banned shape is described rather than written out: this
                // comment is inside the directory the gate scans, and spelling
                // the example literally made the gate fail on itself -- which
                // is the cheapest possible proof that it bites.)
                if !rest.starts_with(':') || rest.starts_with("::") {
                    continue;
                }
                if !rest[1..].starts_with(|c: char| c.is_ascii_digit()) {
                    continue;
                }
                scanned += 1;
                if historical.contains(&i) {
                    exempted += 1;
                    continue;
                }
                let rel = file.strip_prefix(&root).unwrap_or(&file).display();
                offenders.push(format!("  {rel}:{}  ->  {path}{rest}", i + 1));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "{} line-number citation(s) in src/ or tests/. A line number is a claim a later commit \
         can falsify without touching the sentence; six such citations were already stale on \
         `3666d38`. Name the symbol instead -- `src/plan.rs::plan_backend` -- or mark the \
         citation `{HISTORICAL_MARKER}` and say which tree it is true about.\n{}\n\
         (scanned {scanned} line citations, {exempted} historical and exempt)",
        offenders.len(),
        offenders.join("\n"),
    );
}

#[test]
fn every_symbol_citation_into_code_resolves() {
    let root = repo_root();
    let mut offenders = Vec::new();
    let mut checked = 0usize;

    for file in covered_files() {
        let text = std::fs::read_to_string(&file).expect("a file the walker just listed");
        for (i, line) in text.lines().enumerate() {
            for token in tokens(line) {
                let Some((path, rest)) = split_at_extension(token) else {
                    continue;
                };
                let Some(symbol) = rest.strip_prefix("::") else {
                    continue;
                };
                let symbol =
                    symbol.trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
                if symbol.is_empty() {
                    continue;
                }
                // Only targets the shipping tarball carries. `docs/` is
                // `scripts/check-citations.py`'s scope, stated in this file's
                // header, and a check that silently skipped it here would read
                // as covering it.
                if !(path.starts_with("src/") || path.starts_with("tests/")) {
                    continue;
                }
                checked += 1;
                let rel = file.strip_prefix(&root).unwrap_or(&file).display();
                let target = root.join(path);
                let Ok(target_text) = std::fs::read_to_string(&target) else {
                    offenders.push(format!("  {rel}:{}  ->  {path} does not exist", i + 1));
                    continue;
                };
                if !target_text.contains(symbol) {
                    offenders.push(format!(
                        "  {rel}:{}  ->  {path} contains no `{symbol}`",
                        i + 1
                    ));
                }
            }
        }
    }

    // Deliberately NOT `assert!(checked > 0)`. That was the first version, and
    // it is the wrong guard: it couples the gate to the corpus happening to
    // contain the form, so a refactor that legitimately rewrote the last
    // fourteen symbol citations would turn this red with a message about the
    // parser. What needs proving is that the PARSER still recognises the form,
    // and `the_parser_recognises_both_citation_shapes` below proves that on
    // synthetic input, independently of what the tree currently holds.
    let _ = checked;
    assert!(
        offenders.is_empty(),
        "{} symbol citation(s) do not resolve, out of {checked} checked:\n{}",
        offenders.len(),
        offenders.join("\n"),
    );
}

/// The gate's own parser, proved on synthetic input rather than on whatever the
/// tree happens to contain.
///
/// Added by this branch's own review. The first version of
/// `every_symbol_citation_into_code_resolves` asserted that it had resolved at
/// least one citation, which reads like a self-check and is not one: it fails
/// when the corpus stops using the form, not when the parser stops recognising
/// it, and those are different events with the same red. A gate that cannot
/// distinguish them is the fourth defect class wearing a seatbelt.
#[test]
fn the_parser_recognises_both_citation_shapes() {
    let banned = |s: &str| {
        tokens(s).iter().any(|t| {
            split_at_extension(t)
                .is_some_and(|(_, rest)| rest.starts_with(':') && !rest.starts_with("::"))
        })
    };
    let symbol = |s: &str| {
        tokens(s).iter().any(|t| {
            split_at_extension(t).is_some_and(|(path, rest)| {
                rest.starts_with("::") && (path.starts_with("src/") || path.starts_with("tests/"))
            })
        })
    };

    // The banned shape, in the spellings this repository actually used.
    // The banned shape is BUILT rather than written out. This file lives inside
    // the directory the gate scans, so a literal example makes the gate fail on
    // itself -- which it duly did on the first run of this test. That is the
    // gate being total, not a hole in it, and the right place for the
    // workaround is here rather than in a new exemption.
    let c = ":";
    assert!(banned(&format!("(`tests/cli.rs{c}1000`) already drives")));
    assert!(banned(&format!("// (apply.rs{c}912) is the only thing")));
    assert!(banned(&format!("`src/backend/winget.rs{c}899`, `{c}988`")));

    // The required shape.
    assert!(symbol("(`src/plan.rs::plan_backend`), and `covers`"));
    assert!(symbol("`tests/cli.rs::path_without_winget` strips every"));

    // Neither shape, and each is something this repository really writes.
    for innocent in [
        "the same `sysinfo` 0.32 dotpkg links",
        "`Running::covers` third disjunct",
        "std::path::PathBuf::from(local)",
        "at 12:30 on a ratio of 1.5:1",
        "under C:\\Users\\kln\\AppData",
        "https://example.com/a.md and nothing else",
    ] {
        assert!(!banned(innocent), "false positive on: {innocent}");
        assert!(!symbol(innocent), "false positive on: {innocent}");
    }
}
