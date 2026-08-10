//! Adding a package to `pkg.toml`.
//!
//! `pkg.toml` is the only file dotpkg writes that a human wrote by hand and
//! committed with comments in it. `pkg.lock` and `state.json` are dotpkg's own
//! and can be rendered from scratch; this one cannot, so it is edited in place
//! with `toml_edit` and every edit is verified before it replaces the original.

use crate::model::Name;
use anyhow::{Context, Result};
use std::path::Path;
use toml_edit::{Array, DocumentMut, Item, Value};

/// Append `name` to `packages`, matching the surrounding style: if the
/// existing entries are on their own lines, keep that (and give the new one
/// its own indented, trailing-comma-terminated line); otherwise append
/// inline. Shared by `add_scoop_package` and `add_winget_package` -- the two
/// call sites were carrying identical copies of this block until a reviewer
/// flagged the duplication.
///
/// **Carries the array's own trailing text forward as the new element's
/// prefix, rather than discarding it.** `toml_edit` (measured directly,
/// 0.22.27) stores a same-line comment on an array's LAST element as the
/// ARRAY's own `trailing` decor -- the text between the last comma and `]`
/// -- not as that element's `suffix`. A comment on any OTHER element instead
/// lands in the NEXT element's `prefix`, which nothing here ever touches, so
/// that case was always safe. Overwriting `trailing` unconditionally (this
/// function's first version) silently dropped a comment on the last element
/// on every append -- found by the Phase 4 dogfood running `dotpkg adopt
/// --backend winget` for real, not by review. When the old trailing was
/// unremarkable (no elements yet, or the plain `"\n"` a clean multiline array
/// leaves behind), `format!("{old_trailing}  ")` reduces to exactly the
/// `"\n  "` this function hardcoded before the fix, so the ordinary case is
/// byte-for-byte unchanged.
fn append_to_packages_array(packages: &mut Array, name: &Name) {
    let multiline = packages.iter().count() > 0 && packages.to_string().contains('\n');
    let old_trailing = packages.trailing().as_str().unwrap_or("").to_string();
    packages.push(name.to_string());
    if multiline {
        let last_idx = packages.len() - 1;
        let prefix = if old_trailing.is_empty() {
            "\n  ".to_string()
        } else {
            format!("{old_trailing}  ")
        };
        if let Some(last) = packages.get_mut(last_idx) {
            last.decor_mut().set_prefix(prefix);
        }
        packages.set_trailing_comma(true);
        packages.set_trailing("\n");
    }
}

/// Add `name` to `[scoop] packages`, preserving comments, ordering and
/// formatting.
///
/// Refuses rather than guesses in three cases: the file does not parse, the
/// package is already declared (`config::parse` rejects a duplicate, so a
/// blind append would leave a `pkg.toml` that no longer loads), or the result
/// does not re-parse to exactly the original config plus this one name.
///
/// That last check is the reason this function returns a `String` instead of
/// writing: the verification has to happen before anything reaches disk.
pub fn add_scoop_package(text: &str, name: &Name) -> Result<String> {
    let before =
        crate::config::parse(text).context("refusing to edit a pkg.toml that does not parse")?;
    anyhow::ensure!(
        !before.scoop.packages.contains(name),
        "{name} is already declared in pkg.toml (package names are compared \
         without regard to case)"
    );

    let mut doc: DocumentMut = text
        .parse()
        .context("refusing to edit a pkg.toml that does not parse as TOML")?;

    let scoop = doc
        .entry("scoop")
        .or_insert_with(|| Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("pkg.toml's [scoop] is not a table")?;
    let packages = scoop
        .entry("packages")
        .or_insert_with(|| Item::Value(Value::Array(Array::new())))
        .as_array_mut()
        .context("pkg.toml's [scoop] packages is not an array")?;

    append_to_packages_array(packages, name);

    let out = doc.to_string();
    no_comment_was_lost(text, &out)?;
    verify_round_trip(&before, name, &out)?;
    Ok(out)
}

/// The guard. An edit to a hand-written committed file that cannot be
/// verified is refused, not written. `out` is re-parsed with `config::parse`
/// and compared field by field against `before` -- everything must be
/// unchanged except `[scoop] packages`, which must be exactly `before` plus
/// `name`.
///
/// Split out from `add_scoop_package` so it can be exercised directly: that
/// function only ever changes `[scoop] packages`, and its own
/// "already declared" check makes the packages-list comparison below
/// unreachable as a *failure* through the public function alone -- any input
/// for which `after.scoop.packages` could disagree with `before` plus `name`
/// is an input the earlier check already refused. The `buckets` / `opts` /
/// `winget` comparison here is what remains: defence against a future change
/// to the editing code above that touches something it should not.
///
/// This is the semantic half only: it re-parses `out` and compares `Config`
/// values, and `Config` has no field for comments, so a lost comment is
/// invisible here by construction. `no_comment_was_lost` is the text-level
/// sibling that catches that; it is called in `add_scoop_package`, beside
/// this function's own call, because only the caller holds both the
/// original and edited text. A future third `add_*_package` function must
/// call both, the way this one does.
fn verify_round_trip(before: &crate::config::Config, name: &Name, out: &str) -> Result<()> {
    let after = crate::config::parse(out)
        .context("the edit produced a pkg.toml that no longer parses; refusing to write it")?;
    anyhow::ensure!(
        after.scoop.buckets == before.scoop.buckets
            && after.scoop.opts == before.scoop.opts
            && after.winget.packages == before.winget.packages,
        "the edit changed something other than [scoop] packages; refusing to write it"
    );
    let mut want = before.scoop.packages.clone();
    want.push(name.clone());
    anyhow::ensure!(
        after.scoop.packages == want,
        "the edit did not add exactly {name} to [scoop] packages; refusing to write it"
    );
    Ok(())
}

/// Add `name` to `[winget] packages`, preserving comments, ordering and
/// formatting. `adopt`'s winget path.
///
/// The mirror of `add_scoop_package`, one section over: same three refusals
/// (does not parse, already declared, round trip disagrees), same
/// multiline-preserving append. `name` is whatever spelling the caller wants
/// written -- `adopt::adopt_one_winget` passes the spelling the user typed on
/// the command line, deliberately never the canonical id `winget` echoed
/// back: `pkg.toml` is the user's file, and the canonical-id rule
/// (`src/backend/winget.rs`) says that spelling is reported, not silently
/// rewritten. A pkg.toml that already declares the canonical spelling (or
/// any other case-fold of it) never reaches the "already declared" ensure
/// below as a failure, because `Config::winget.packages.contains` also folds
/// case.
pub fn add_winget_package(text: &str, name: &Name) -> Result<String> {
    let before =
        crate::config::parse(text).context("refusing to edit a pkg.toml that does not parse")?;
    anyhow::ensure!(
        !before.winget.packages.contains(name),
        "{name} is already declared in pkg.toml (package names are compared \
         without regard to case)"
    );

    let mut doc: DocumentMut = text
        .parse()
        .context("refusing to edit a pkg.toml that does not parse as TOML")?;

    let winget = doc
        .entry("winget")
        .or_insert_with(|| Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("pkg.toml's [winget] is not a table")?;
    let packages = winget
        .entry("packages")
        .or_insert_with(|| Item::Value(Value::Array(Array::new())))
        .as_array_mut()
        .context("pkg.toml's [winget] packages is not an array")?;

    append_to_packages_array(packages, name);

    let out = doc.to_string();
    no_comment_was_lost(text, &out)?;
    verify_round_trip_winget(&before, name, &out)?;
    Ok(out)
}

/// `verify_round_trip`'s mirror for the winget section: everything must be
/// unchanged except `[winget] packages`, compared as one `ScoopSection`
/// equality rather than field by field -- `ScoopSection` derives `PartialEq`
/// and nothing in it is what this function touches, unlike
/// `verify_round_trip` above, which has to compare three of `ScoopSection`'s
/// own fields individually because `[scoop] packages` -- part of the SAME
/// section -- is exactly what `add_scoop_package` changes.
///
/// Like `verify_round_trip`, this is the semantic half only -- it cannot see
/// a lost comment, because the `Config` it compares has no field for one.
/// `no_comment_was_lost` is the text-level sibling that catches that; it is
/// called in `add_winget_package`, beside this function's own call, for the
/// same reason: only the caller holds both texts. A future third
/// `add_*_package` function must call both, the way this one does.
fn verify_round_trip_winget(before: &crate::config::Config, name: &Name, out: &str) -> Result<()> {
    let after = crate::config::parse(out)
        .context("the edit produced a pkg.toml that no longer parses; refusing to write it")?;
    anyhow::ensure!(
        after.scoop == before.scoop,
        "the edit changed something other than [winget] packages; refusing to write it"
    );
    let mut want = before.winget.packages.clone();
    want.push(name.clone());
    anyhow::ensure!(
        after.winget.packages == want,
        "the edit did not add exactly {name} to [winget] packages; refusing to write it"
    );
    Ok(())
}

/// Every `#` comment present in `before` must still be present in `after` --
/// compared as a MULTISET of comment texts, not a set, so a comment that is
/// one of several identical ones and silently drops to fewer copies is
/// caught even though its text is still present elsewhere in the file. A
/// plain "is this text in `after` somewhere" check cannot tell those apart.
///
/// This function's first version, `exactly_one_line_added`, checked a
/// different and wrong invariant: that `after` has exactly one more line
/// than `before`, with every original line otherwise unchanged and in
/// order. That is only true of ONE of the shapes `add_scoop_package` and
/// `add_winget_package` produce -- appending to an existing multiline
/// array, the shape the Phase 4 dogfood bug lived in. It is false of two
/// other shapes those same functions are documented and tested to produce:
/// creating a section (`[scoop]` or `[winget]`) that did not exist yet adds
/// several lines, not one, and appending to a single-line or previously
/// empty array adds zero lines, because an existing line's content is
/// extended in place. Wiring the line-count version in made 16 pre-existing
/// tests fail for those two reasons -- none involving a lost comment -- so
/// it checked a description of one code path, not an invariant true of all
/// of them. Comment loss, not line count, is the actual defect class this
/// guard exists for, and comment loss is what it checks now.
///
/// What counts as "a comment" on a line, and the heuristic's actual scope
/// and remaining limit, are `line_comment`'s to document, not repeated here
/// to avoid the two drifting apart.
///
/// **Known, accepted weakness**: a comment that MOVES to a different line,
/// without being lost, is not caught -- a multiset remembers count, not
/// position. This is the same class of gap a plain `.contains` assertion
/// has (`docs/dogfood-phase4-2026-08-10.md`: "cannot tell 'still attached to
/// the right line' apart from 'moved'"). Loss is the defect class actually
/// measured (the dogfood bug); position is not checkable here without
/// reintroducing the line-count invariant this function replaced, which is
/// exactly what rejected legitimate edits. Deliberate, not an oversight.
fn no_comment_was_lost(before: &str, after: &str) -> Result<()> {
    let mut counts: std::collections::BTreeMap<&str, i32> = std::collections::BTreeMap::new();
    for line in before.lines() {
        if let Some(comment) = line_comment(line) {
            *counts.entry(comment).or_insert(0) += 1;
        }
    }
    for line in after.lines() {
        if let Some(comment) = line_comment(line) {
            *counts.entry(comment).or_insert(0) -= 1;
        }
    }
    if let Some((&comment, _)) = counts.iter().find(|&(_, &net)| net > 0) {
        anyhow::bail!(
            "the edit lost a comment: {comment:?} was in pkg.toml before and has fewer \
             copies (or none) now; refusing to write it"
        );
    }
    Ok(())
}

/// The comment on one `pkg.toml` line, if it has one: everything from the
/// first `#` that is not inside a quoted string to the end of the line.
///
/// A single left-to-right scan tracking whether the cursor is currently
/// inside a `"..."` string, so a `#` inside a string value (a bucket URL's
/// `#branch` fragment, which `src/config.rs` permits, is a real example) is
/// correctly skipped, AND a `#` that starts the comment is correctly found
/// even when the comment's OWN text goes on to contain a quote (`# see
/// "docs/x.md"`, `# replaces "extras" bucket`) -- both directions matter:
/// an earlier version of this function searched only after the line's LAST
/// `"`, which handled the first case but not the second. A comment whose
/// text contains a quote made that version's `rfind` land INSIDE the
/// comment, so it saw nothing after it and returned `None` -- in both
/// `before` and `after` alike, so the comment never entered
/// `no_comment_was_lost`'s multiset at all, and its total loss was silently
/// waved through rather than flagged. Fixed by scanning once, forward,
/// rather than searching backward from the end.
///
/// **Remaining heuristic limit, real but narrow**: TOML's basic strings
/// allow an escaped quote, `\"`, without ending the string. This scan does
/// not know that and toggles `in_string` on every `"` including an escaped
/// one, so a line whose STRING VALUE (a package name or bucket URL, not a
/// comment) contains `\"` followed later by a `#` could still be misread.
/// No shape `pkg.toml` actually writes does this -- package names and
/// bucket URLs never contain a literal quote -- so the limit is accepted
/// rather than handled.
fn line_comment(line: &str) -> Option<&str> {
    let mut in_string = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_string = !in_string,
            '#' if !in_string => return Some(line[i..].trim_end()),
            _ => {}
        }
    }
    None
}

/// Replace `pkg.toml`, keeping the file the user wrote as `pkg.toml.bak`.
///
/// Temp-then-rename, the same discipline as `State::save` and `lock::save`.
pub fn save(path: &Path, text: &str) -> Result<()> {
    let stem = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("pkg.toml");
    let tmp = path.with_file_name(format!("{stem}.tmp{}", std::process::id()));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)
            .with_context(|| format!("cannot create {}", tmp.display()))?;
        f.write_all(text.as_bytes())
            .with_context(|| format!("cannot write {}", tmp.display()))?;
        f.sync_all()
            .with_context(|| format!("cannot flush {}", tmp.display()))?;
    }
    if path.exists() {
        let _ = std::fs::copy(path, path.with_extension("toml.bak"));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow::Error::new(e).context(format!(
            "cannot move {} into place at {}",
            tmp.display(),
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HAND_WRITTEN: &str = r#"# what this machine should have
[scoop]
buckets  = ["main", "extras"]
packages = [
  "fzf",     # fuzzy finder
  "bat",
]

[scoop.opts]
python = { arch = "64bit" }   # force an architecture
"#;

    #[test]
    fn a_package_is_added_and_every_comment_survives() {
        let out = add_scoop_package(HAND_WRITTEN, &Name::new("ripgrep")).unwrap();

        assert!(out.contains("# what this machine should have"), "{out}");
        assert!(out.contains("# fuzzy finder"), "{out}");
        assert!(out.contains("# force an architecture"), "{out}");

        let cfg = crate::config::parse(&out).unwrap();
        assert!(cfg.scoop.packages.contains(&Name::new("ripgrep")));
        assert!(cfg.scoop.packages.contains(&Name::new("fzf")));
        assert!(cfg.scoop.packages.contains(&Name::new("bat")));
        assert_eq!(cfg.scoop.buckets.len(), 2);
        assert_eq!(
            cfg.scoop.opts[&Name::new("python")].arch,
            Some(crate::config::Arch::X64)
        );
    }

    #[test]
    fn a_trailing_comment_on_the_last_scoop_element_survives_an_append() {
        // The scoop-side sibling of `add_winget_package`'s test of the same
        // shape: `append_to_packages_array` is shared code, and the bug the
        // Phase 4 dogfood found in the winget path was pre-existing here too
        // -- `HAND_WRITTEN` above never happened to put its comment on the
        // LAST element (`"bat"` has none), so nothing exercised this path
        // before.
        const SRC: &str = "[scoop]\npackages = [\n  \"fzf\",  # fuzzy finder\n]\n";
        let out = add_scoop_package(SRC, &Name::new("ripgrep")).unwrap();
        assert!(
            out.contains("\"fzf\",  # fuzzy finder\n  \"ripgrep\",\n]"),
            "the comment must stay attached to fzf's own line, and the new \
             entry must land on its own line after it: {out}"
        );
        let cfg = crate::config::parse(&out).unwrap();
        assert_eq!(
            cfg.scoop.packages,
            vec![Name::new("fzf"), Name::new("ripgrep")]
        );
    }

    #[test]
    fn adding_a_package_that_is_already_declared_is_refused_rather_than_duplicated() {
        // `packages = ["fzf", "fzf"]` is refused by config::parse, so a blind
        // append would produce a pkg.toml that no longer loads at all.
        let err = add_scoop_package(HAND_WRITTEN, &Name::new("FZF")).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("fzf") || msg.contains("FZF"), "name it: {msg}");
        assert!(msg.contains("already"), "say why: {msg}");
    }

    // -- the LAYOUT the edit leaves behind -------------------------------
    //
    // Added by the Task 14 mutation run, which found six survivors in the
    // `multiline` decision (`src/config_edit.rs:49` and `:52`). Every test
    // above asserts that the result PARSES and that comments survive; none
    // asserted the shape of the text, so the whole `multiline` branch --
    // the reason this function uses `toml_edit` at all rather than
    // re-rendering the file -- could be computed any way at all and the
    // suite stayed green. Matching the surrounding style is the promise;
    // these are what hold it.

    #[test]
    fn a_multiline_packages_array_keeps_its_shape_and_the_new_entry_gets_its_own_line() {
        let out = add_scoop_package(HAND_WRITTEN, &Name::new("ripgrep")).unwrap();
        assert!(
            out.contains("  \"bat\",\n  \"ripgrep\",\n]"),
            "the new entry must land on its own indented line with a trailing \
             comma, and the closing bracket must stay on its own line: {out}"
        );
        crate::config::parse(&out).expect("and it must still parse");
    }

    #[test]
    fn a_single_line_packages_array_is_not_reflowed_onto_several_lines() {
        // The other side of the same decision. Reflowing a one-line array is
        // a diff the user did not ask for in a file they hand-wrote and
        // committed.
        let out =
            add_scoop_package("[scoop]\npackages = [\"fzf\"]\n", &Name::new("ripgrep")).unwrap();
        assert!(
            out.contains("packages = [\"fzf\", \"ripgrep\"]"),
            "a single-line array must stay on one line: {out}"
        );
        crate::config::parse(&out).expect("and it must still parse");
    }

    #[test]
    fn an_empty_packages_array_keeps_whichever_shape_it_already_had() {
        // `packages = []` and `packages = [\n]` are both empty, so the entry
        // count cannot decide this on its own -- only the existing text can.
        // This is also the empty-pkg.toml case that had no test at all.
        let flat = add_scoop_package("[scoop]\npackages = []\n", &Name::new("ripgrep")).unwrap();
        assert!(
            flat.contains("packages = [\"ripgrep\"]"),
            "an empty one-line array must stay on one line: {flat}"
        );

        let broken =
            add_scoop_package("[scoop]\npackages = [\n]\n", &Name::new("ripgrep")).unwrap();
        assert!(
            broken.contains("packages = [\"ripgrep\"\n]"),
            "an empty array with no entries is not a multiline array: {broken}"
        );

        for out in [&flat, &broken] {
            let cfg = crate::config::parse(out).expect("and it must still parse");
            assert_eq!(cfg.scoop.packages, vec![Name::new("ripgrep")]);
        }
    }

    #[test]
    fn a_file_with_no_scoop_section_grows_one() {
        let out =
            add_scoop_package("[winget]\npackages = [\"Git.Git\"]\n", &Name::new("fzf")).unwrap();
        let cfg = crate::config::parse(&out).unwrap();
        assert!(cfg.scoop.packages.contains(&Name::new("fzf")));
        assert!(cfg.winget.packages.contains(&Name::new("Git.Git")), "{out}");
    }

    #[test]
    fn an_edit_that_changes_anything_else_is_refused_rather_than_written() {
        // The guard, exercised through a document toml_edit and config::parse
        // disagree about. `parse` uses deny_unknown_fields, so a stray key
        // means the round trip cannot be checked -- and an unverifiable edit
        // to a hand-written committed file is refused, not guessed at.
        let err =
            add_scoop_package("[scoop]\npackagess = [\"fzf\"]\n", &Name::new("bat")).unwrap_err();
        assert!(
            format!("{err:#}").contains("packagess"),
            "the original file's own problem must be named: {err:#}"
        );
    }

    #[test]
    fn the_round_trip_guard_is_reached_and_compares_the_whole_config() {
        // A positive statement of the same guard: the result must parse to
        // exactly the original config plus one package, and nothing else.
        let out = add_scoop_package(HAND_WRITTEN, &Name::new("ripgrep")).unwrap();
        let before = crate::config::parse(HAND_WRITTEN).unwrap();
        let after = crate::config::parse(&out).unwrap();

        assert_eq!(after.scoop.buckets, before.scoop.buckets);
        assert_eq!(after.scoop.opts, before.scoop.opts);
        assert_eq!(after.winget.packages, before.winget.packages);
        assert_eq!(after.scoop.packages.len(), before.scoop.packages.len() + 1);
    }

    #[test]
    fn saving_keeps_the_displaced_file_alongside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pkg.toml");
        std::fs::write(&path, HAND_WRITTEN).unwrap();

        let out = add_scoop_package(HAND_WRITTEN, &Name::new("ripgrep")).unwrap();
        save(&path, &out).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), out);
        let bak = path.with_extension("toml.bak");
        assert_eq!(
            std::fs::read_to_string(&bak).unwrap(),
            HAND_WRITTEN,
            "the file the user wrote is kept at {bak:?}"
        );
    }

    #[test]
    fn the_round_trip_guard_rejects_a_result_that_changes_more_than_packages() {
        // `add_scoop_package` itself never produces this shape -- it only
        // ever touches [scoop] packages, and its own "already declared"
        // check makes a packages-list disagreement unreachable through the
        // public function (proven by measurement: an `an_edit_that_changes_
        // anything_else_is_refused_rather_than_written`-style fixture with a
        // stray key fails at the earlier `before = config::parse(text)`
        // check every time, never at this guard -- see the commit message).
        // `verify_round_trip` is exercised directly instead, with a `before`
        // and a hand-written `out` that disagree on `buckets`: the shape a
        // future bug in the editing code above could produce.
        let before = crate::config::parse(HAND_WRITTEN).unwrap();
        let out = HAND_WRITTEN
            .replacen(
                r#"buckets  = ["main", "extras"]"#,
                r#"buckets  = ["main"]"#,
                1,
            )
            .replacen(r#""bat","#, r#""bat", "ripgrep","#, 1);

        let r = verify_round_trip(&before, &Name::new("ripgrep"), &out);
        assert!(r.is_err(), "a buckets disagreement must be refused: {r:?}");
        assert!(
            format!("{:#}", r.unwrap_err()).contains("changed something other than"),
            "must say what kind of disagreement it is"
        );
    }

    // -- add_winget_package -----------------------------------------------
    //
    // `add_scoop_package`'s mirror, one section over. Not a full re-run of
    // every scoop-side test (the multiline/round-trip MECHANISM is shared
    // and already proven above) -- these pin the properties that are
    // genuinely different about the winget section: no `buckets`/`opts` to
    // disturb, and the spelling written is the CALLER's, never a
    // canonicalised one.

    // The comment sits on the FIRST of two elements, not the last, matching
    // `add_scoop_package`'s own `HAND_WRITTEN` fixture above -- deliberately,
    // not incidentally, and paired with `a_trailing_comment_on_the_last_
    // element_survives_an_append` below, which puts one on the last element
    // instead. A comment on a non-last element lands in the NEXT element's
    // `prefix` and nothing this module does ever touches it; a comment on
    // the LAST element lands in the ARRAY's own `trailing` decor, which
    // `append_to_packages_array` used to overwrite unconditionally on every
    // append -- found by the Phase 4 dogfood running `dotpkg adopt --backend
    // winget` for real, fixed in `append_to_packages_array`. Keeping both
    // fixtures means a fix that only moves the problem (e.g. one that always
    // preserves whichever comment happens to already be safe) is still
    // caught.
    const HAND_WRITTEN_WINGET: &str = r#"# what this machine should have
[scoop]
buckets  = ["main"]
packages = ["fzf"]

[winget]
packages = [
  "Git.Git",     # version control
  "OpenAI.Codex",
]
"#;

    #[test]
    fn a_winget_package_is_added_and_every_comment_survives() {
        let out = add_winget_package(HAND_WRITTEN_WINGET, &Name::new("7zip.7zip")).unwrap();

        assert!(out.contains("# what this machine should have"), "{out}");
        assert!(out.contains("# version control"), "{out}");

        let cfg = crate::config::parse(&out).unwrap();
        assert!(cfg.winget.packages.contains(&Name::new("7zip.7zip")));
        assert!(cfg.winget.packages.contains(&Name::new("Git.Git")));
        // The [scoop] section -- a different backend entirely -- must be
        // untouched.
        assert!(cfg.scoop.packages.contains(&Name::new("fzf")));
        assert_eq!(cfg.scoop.buckets.len(), 1);
    }

    #[test]
    fn a_trailing_comment_on_the_last_winget_element_survives_an_append() {
        // The dogfood-found shape, reproduced directly: a same-line comment
        // on the array's LAST element, appended to. An exact substring, not
        // a loose `.contains("# ...")` -- `.contains` alone is exactly what
        // let the original bug hide behind `a_winget_package_is_added_and_
        // every_comment_survives` above, since that only checks the comment
        // text appears SOMEWHERE, not that it stayed attached to the right
        // line.
        const SRC: &str =
            "[winget]\npackages = [\n  \"ajeetdsouza.zoxide\",  # kept for comment-survival check\n]\n";
        let out = add_winget_package(SRC, &Name::new("Vivaldi.Vivaldi")).unwrap();
        assert!(
            out.contains(
                "\"ajeetdsouza.zoxide\",  # kept for comment-survival check\n  \"Vivaldi.Vivaldi\",\n]"
            ),
            "the comment must stay attached to ajeetdsouza.zoxide's own line, \
             and the new entry must land on its own line after it: {out}"
        );
        let cfg = crate::config::parse(&out).unwrap();
        assert_eq!(
            cfg.winget.packages,
            vec![
                Name::new("ajeetdsouza.zoxide"),
                Name::new("Vivaldi.Vivaldi")
            ]
        );
    }

    #[test]
    fn a_winget_package_is_written_exactly_as_the_caller_spelled_it() {
        // `pkg.toml` is the user's file: `adopt`'s winget path writes the
        // spelling the user typed on the command line, not the canonical id
        // winget might echo back for a DIFFERENT purpose (the lock's key).
        // This is the property that would silently break if `add_winget_
        // package` were ever changed to write `name.key()` (folded) or some
        // canonicalised spelling instead of `name.to_string()`.
        let out =
            add_winget_package("[winget]\npackages = []\n", &Name::new("openai.codex")).unwrap();
        assert!(
            out.contains("\"openai.codex\""),
            "the exact spelling passed in must appear verbatim: {out}"
        );
        assert!(
            !out.contains("\"OpenAI.Codex\""),
            "nothing here may invent a different case: {out}"
        );
    }

    #[test]
    fn adding_a_winget_package_that_is_already_declared_is_refused_rather_than_duplicated() {
        let result = add_winget_package(HAND_WRITTEN_WINGET, &Name::new("git.git"));
        assert!(
            result.is_err(),
            "a package already declared under [winget] must be refused: {result:?}"
        );
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("git.git") || msg.contains("Git.Git"),
            "name it: {msg}"
        );
        assert!(msg.contains("already"), "say why: {msg}");
    }

    #[test]
    fn a_file_with_no_winget_section_grows_one() {
        let out =
            add_winget_package("[scoop]\npackages = [\"fzf\"]\n", &Name::new("Git.Git")).unwrap();
        let cfg = crate::config::parse(&out).unwrap();
        assert!(cfg.winget.packages.contains(&Name::new("Git.Git")));
        assert!(cfg.scoop.packages.contains(&Name::new("fzf")));
    }

    #[test]
    fn the_winget_round_trip_guard_rejects_a_result_that_changes_the_scoop_section() {
        let before = crate::config::parse(HAND_WRITTEN_WINGET).unwrap();
        // A hand-built "edit" that also touches [scoop] buckets -- the shape
        // a future bug in `add_winget_package` could produce.
        let out = HAND_WRITTEN_WINGET
            .replacen(r#"buckets  = ["main"]"#, r#"buckets  = []"#, 1)
            .replacen(r#""Git.Git","#, r#""Git.Git", "7zip.7zip","#, 1);

        let r = verify_round_trip_winget(&before, &Name::new("7zip.7zip"), &out);
        assert!(
            r.is_err(),
            "a scoop-section disagreement must be refused: {r:?}"
        );
        assert!(
            format!("{:#}", r.unwrap_err()).contains("changed something other than"),
            "must say what kind of disagreement it is"
        );
    }

    #[test]
    fn the_winget_round_trip_guard_accepts_a_clean_addition() {
        // The positive control for the test above: without it, a guard that
        // always refuses would satisfy it for the wrong reason.
        let before = crate::config::parse(HAND_WRITTEN_WINGET).unwrap();
        let out = add_winget_package(HAND_WRITTEN_WINGET, &Name::new("7zip.7zip")).unwrap();
        verify_round_trip_winget(&before, &Name::new("7zip.7zip"), &out).unwrap();
    }

    // -- no_comment_was_lost ------------------------------------------------
    //
    // Both guards above re-parse the edited text and compare `Config`
    // values, and `Config` has no field for comments -- so the comment-loss
    // bug the Phase 4 dogfood found (a same-line trailing comment on a
    // `[winget] packages` array's last element, silently dropped on append)
    // was invisible to either guard BY CONSTRUCTION. This is the text-level
    // half that catches it.
    //
    // An earlier version of this guard, `exactly_one_line_added`, checked
    // line COUNT instead of comment content and was wrong: creating a new
    // section legitimately adds several lines, and an inline array append
    // legitimately adds zero. `HAND_WRITTEN` and `HAND_WRITTEN_WINGET`
    // above, which both carry real comments and go through the real
    // `add_*_package` path, are this guard's live positive controls -- if
    // an append ever regresses, they go red without anyone writing a new
    // test.

    #[test]
    fn a_dropped_trailing_comment_is_caught_by_the_text_level_guard() {
        // The exact bytes of the Phase 4 dogfood's finding: a same-line comment
        // on the array's LAST element, before the closing bracket.
        let before = "[winget]\npackages = [\n  \"ajeetdsouza.zoxide\",  # keep me\n]\n";
        // What the bug produced: the comment gone, one element added.
        let buggy =
            "[winget]\npackages = [\n  \"ajeetdsouza.zoxide\",\n  \"Vivaldi.Vivaldi\",\n]\n";
        assert!(
            no_comment_was_lost(before, buggy).is_err(),
            "a lost comment must be caught"
        );

        // The positive control: the correct output must pass. Without it, a
        // guard that rejected everything would satisfy the assertion above.
        let good = "[winget]\npackages = [\n  \"ajeetdsouza.zoxide\",  # keep me\n  \"Vivaldi.Vivaldi\",\n]\n";
        assert!(
            no_comment_was_lost(before, good).is_ok(),
            "the correct edit must pass"
        );

        // A plain "is this text present anywhere in `after`" check is fooled
        // here: two elements happen to carry the identical comment "# keep",
        // so that text never disappears from the file -- a SET of comment
        // texts looks unchanged. Comparing as a MULTISET catches it anyway:
        // "# keep" goes from two copies to one, so one of them was silently
        // dropped even though its text survives on the other line.
        let two_copies = "[winget]\npackages = [\n  \"A.A\",  # keep\n  \"B.B\",  # keep\n]\n";
        let one_dropped = "[winget]\npackages = [\n  \"A.A\",\n  \"B.B\",  # keep\n]\n";
        assert!(
            no_comment_was_lost(two_copies, one_dropped).is_err(),
            "one of two identical comments vanishing must be caught, not waved \
             through because its text is still present on the other line"
        );

        // A comment whose OWN text carries a quote. `line_comment` must find
        // the `#` by scanning for one that is not inside a string, not by
        // looking only after the line's LAST `"` -- that would land inside
        // this comment itself (its closing quote is the last `"` on the
        // line), see nothing after it, and miss the comment entirely, in
        // BOTH `before` and `after` alike, so its total loss would go
        // unnoticed rather than flagged.
        let quoted_comment = "[winget]\npackages = [\n  \"A.A\",  # keeps \"extras\"\n]\n";
        let comment_dropped = "[winget]\npackages = [\n  \"A.A\",\n]\n";
        assert!(
            no_comment_was_lost(quoted_comment, comment_dropped).is_err(),
            "a comment containing a quote must still be tracked, and its loss caught"
        );
    }
}
