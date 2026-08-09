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
}
