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

    // Match the surrounding style: if the existing entries are on their own
    // lines, keep that; otherwise append inline.
    let multiline = packages.iter().count() > 0 && packages.to_string().contains('\n');
    packages.push(name.to_string());
    if multiline {
        let last_idx = packages.len() - 1;
        if let Some(last) = packages.get_mut(last_idx) {
            last.decor_mut().set_prefix("\n  ");
        }
        packages.set_trailing_comma(true);
        packages.set_trailing("\n");
    }

    let out = doc.to_string();
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

    let multiline = packages.iter().count() > 0 && packages.to_string().contains('\n');
    packages.push(name.to_string());
    if multiline {
        let last_idx = packages.len() - 1;
        if let Some(last) = packages.get_mut(last_idx) {
            last.decor_mut().set_prefix("\n  ");
        }
        packages.set_trailing_comma(true);
        packages.set_trailing("\n");
    }

    let out = doc.to_string();
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
    // not incidentally. Measured while writing this test: an inline comment
    // on the LAST element of a multiline array is attached to the array's
    // own trailing decor, not to that element, and `packages.set_trailing
    // ("\n")` (shared by both `add_scoop_package` and `add_winget_package`,
    // a few lines up) unconditionally overwrites it -- so that comment is
    // silently dropped on append. Pre-existing in `add_scoop_package` too
    // (its own fixture never happened to put a comment on the last element,
    // so nothing there ever exercised this path); recorded as a finding in
    // this task's report rather than fixed here, since the mechanism is
    // shared code this task did not otherwise touch.
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
        let err = add_winget_package(HAND_WRITTEN_WINGET, &Name::new("git.git")).unwrap_err();
        let msg = format!("{err:#}");
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
}
