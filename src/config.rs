use crate::model::{fold_map, fold_names, Name};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub scoop: ScoopSection,
    pub winget: WingetSection,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ScoopSection {
    pub buckets: Vec<BucketDecl>,
    pub packages: Vec<Name>,
    pub opts: BTreeMap<Name, PkgOpts>,
}

/// One entry of `[scoop] buckets`.
///
/// `"main"` names a bucket scoop already knows; `"xom11=https://…"` names one
/// it does not and says where to get it. Until Phase 2b-2 this list was parsed
/// into `Vec<String>` and read by nothing, while the approved design described
/// cloning from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketDecl {
    pub name: Name,
    pub url: Option<String>,
}

fn parse_buckets(raw: Vec<String>) -> Result<Vec<BucketDecl>> {
    let mut seen: BTreeMap<Name, String> = BTreeMap::new();
    let mut out = Vec::with_capacity(raw.len());
    for entry in raw {
        // Trimmed on both sides of `=`: TOML gives no reason to write
        // `"xom11 = https://…"` over `"xom11=https://…"`, but a user who does
        // must not get silent corruption in return -- an untrimmed name never
        // matches a lock's plain spelling in `bucket_is_declared`, or the real
        // `buckets/<name>` directory, and an untrimmed URL is handed to `git`
        // verbatim.
        let (name_str, url) = match entry.split_once('=') {
            Some((n, u)) => (n.trim().to_string(), Some(u.trim().to_string())),
            None => (entry.trim().to_string(), None),
        };
        let name = Name::new(name_str.clone());
        // The bucket name becomes `$SCOOP/buckets/<name>` and a git argument.
        crate::backend::scoop::ensure_plain_component(
            &name,
            "pkg.toml [scoop] buckets",
            "bucket name",
            name.key(),
        )?;
        if let Some(u) = &url {
            anyhow::ensure!(
                // Deliberately loose: `contains('@')` accepts an `@` anywhere,
                // not only `user@host:`. SSH remotes are legitimately varied
                // (custom ports as `ssh://git@host:2222/…`, aliases from
                // `~/.ssh/config`), and this is the cheap check that lets all
                // of them through rather than a strict grammar that would
                // reject a real one.
                u.starts_with("https://") || u.starts_with("http://") || u.contains('@'),
                "[scoop] buckets: {u:?} does not look like a git remote"
            );
        }
        if let Some(first) = seen.get(&name) {
            anyhow::bail!(
                "[scoop] buckets names the same bucket twice: {first:?} and {name_str:?} \
                 (bucket names are compared without regard to case)"
            );
        }
        seen.insert(name.clone(), name_str);
        out.push(BucketDecl { name, url });
    }
    Ok(out)
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct WingetSection {
    pub packages: Vec<Name>,
    /// Process names the user says belong to a winget package, because winget
    /// exposes no way for dotpkg to find them out.
    ///
    /// **Measured** (`docs/measurements-2026-08-11-…` §2): `Tailscale.Tailscale`
    /// runs `tailscaled` and `tailscale-ipn`, `AutoHotkey.AutoHotkey` runs
    /// `autohotkey64`, and `Microsoft.WSL` runs `wslservice`. None is the id,
    /// the display name, or the id's last dotted segment, and none is a
    /// `portable` install, so neither `guard_names` nor
    /// `backend::winget::running_ids` reaches any of them.
    ///
    /// Values are normalised by `sys::normalize` at parse time, so they are
    /// directly comparable against `Running`'s `names`.
    pub guard: BTreeMap<Name, Vec<String>>,
}

/// The architectures scoop names in install.json, plus the opt-out.
///
/// A closed set on purpose: `arch = "arm"` used to parse and mean "installed
/// wrong, forever", because nothing ever equals it.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    #[serde(rename = "64bit")]
    X64,
    #[serde(rename = "32bit")]
    X86,
    Arm64,
    /// Never change whatever is installed.
    Keep,
}

impl Arch {
    /// The string scoop writes into install.json. `Keep` names no
    /// architecture: it is the absence of an opinion, not a value.
    pub fn as_scoop(self) -> Option<&'static str> {
        match self {
            Arch::X64 => Some("64bit"),
            Arch::X86 => Some("32bit"),
            Arch::Arm64 => Some("arm64"),
            Arch::Keep => None,
        }
    }
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PkgOpts {
    #[serde(default)]
    pub arch: Option<Arch>,
    /// Which declared bucket this package comes from.
    ///
    /// Needed only when two declared buckets both carry the app. Nothing else
    /// can answer it: a new package has no lock entry, and `install.json`
    /// records `bucket` only for packages dotpkg has never installed.
    #[serde(default)]
    pub bucket: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    scoop: RawScoopSection,
    #[serde(default)]
    winget: RawWingetSection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScoopSection {
    #[serde(default)]
    buckets: Vec<String>,
    #[serde(default)]
    packages: Vec<String>,
    #[serde(default)]
    opts: BTreeMap<String, PkgOpts>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWingetSection {
    #[serde(default)]
    packages: Vec<String>,
    #[serde(default)]
    guard: BTreeMap<String, Vec<String>>,
}

pub fn parse(text: &str) -> Result<Config> {
    let raw: RawConfig = toml::from_str(text).context("pkg.toml is not valid")?;
    let cfg = Config {
        scoop: ScoopSection {
            buckets: parse_buckets(raw.scoop.buckets)?,
            packages: fold_names(raw.scoop.packages, "[scoop]")?,
            opts: fold_map(raw.scoop.opts, "[scoop.opts]")?,
        },
        winget: WingetSection {
            packages: fold_names(raw.winget.packages, "[winget]")?,
            guard: fold_map(raw.winget.guard, "[winget.guard]")?
                .into_iter()
                .map(|(id, raw_names)| {
                    let mut names = Vec::new();
                    for raw_name in raw_names {
                        let folded = crate::sys::normalize(raw_name.trim());
                        if folded.is_empty() {
                            anyhow::bail!(
                                "pkg.toml [winget.guard] {id}: a guard name is empty after \
                                 folding. An empty name matches no process while reading here \
                                 as protection."
                            );
                        }
                        if !names.contains(&folded) {
                            names.push(folded);
                        }
                    }
                    Ok((id, names))
                })
                .collect::<Result<BTreeMap<Name, Vec<String>>>>()?,
        },
    };
    // `fold_map` does not look inside values, so the bucket opt -- which
    // becomes `$SCOOP/buckets/<it>` and a git argument -- is validated here,
    // explicitly, with the same check `[scoop] buckets` uses.
    for (name, opts) in &cfg.scoop.opts {
        if let Some(b) = &opts.bucket {
            crate::backend::scoop::ensure_plain_component(
                name,
                "pkg.toml [scoop.opts]",
                "bucket name",
                b,
            )?;
        }
    }
    Ok(cfg)
}

pub fn load(path: &Path) -> Result<Config> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_example() {
        let cfg = parse(
            r#"
[scoop]
buckets  = ["main", "extras", "xom11=https://github.com/xom11/scoop-bucket"]
packages = ["fzf", "Bat"]

[scoop.opts]
python = { arch = "64bit" }
kanata = { arch = "keep" }

[winget]
packages = ["Git.Git"]
"#,
        )
        .unwrap();

        assert_eq!(cfg.scoop.packages, vec!["fzf", "bat"]);
        assert_eq!(cfg.scoop.buckets.len(), 3);
        assert_eq!(cfg.scoop.opts[&Name::new("python")].arch, Some(Arch::X64));
        assert_eq!(cfg.scoop.opts[&Name::new("kanata")].arch, Some(Arch::Keep));
        assert_eq!(cfg.winget.packages, vec!["Git.Git"]);

        // The two checks above fold case (`PartialEq<&str> for Name`), so they
        // would not notice `parse` lowercasing a package name on the way in.
        // `.to_string()` goes through `Display`, which does not fold.
        assert_eq!(cfg.scoop.packages[1].to_string(), "Bat");
        assert_eq!(cfg.winget.packages[0].to_string(), "Git.Git");
    }

    #[test]
    fn an_empty_file_is_valid_and_declares_nothing() {
        let cfg = parse("").unwrap();
        assert!(cfg.scoop.packages.is_empty());
        assert!(cfg.winget.packages.is_empty());
    }

    #[test]
    fn a_misspelled_key_is_an_error_not_a_silent_ignore() {
        // deny_unknown_fields: a typo like `packagess` must not read as "you
        // declared nothing", which would make status report every package as a
        // stray and, in Phase 2, offer to remove them.
        let err = parse("[scoop]\npackagess = [\"fzf\"]\n").unwrap_err();
        assert!(
            format!("{err:#}").contains("packagess"),
            "error should name the bad key, got: {err:#}"
        );
    }

    #[test]
    fn a_misspelled_architecture_is_an_error_not_a_permanent_drift() {
        // `arch = "arm"` used to parse cleanly and mean "always wrong", which
        // in Phase 2b is "reinstall on every run".
        let err = parse("[scoop.opts]\npython = { arch = \"arm\" }\n").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("arm64"),
            "the error must list the real values: {msg}"
        );
    }

    #[test]
    fn two_declared_names_differing_only_in_case_are_rejected() {
        // Name folds case, so these are one package -- but `packages` is a Vec
        // and the declared loop iterates it twice, producing two Install
        // actions for one app and a change_count of 2. Verified against the
        // merged planner.
        let err = parse("[scoop]\npackages = [\"fzf\", \"FZF\"]\n").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("fzf") && msg.contains("FZF"),
            "name both spellings: {msg}"
        );
    }

    #[test]
    fn an_exact_repeat_is_rejected_without_blaming_a_case_difference() {
        // `["fzf", "fzf"]` lands on the same collision path as `["fzf",
        // "FZF"]`, and the message used to end "differ only in case" -- which
        // for this pair is simply false, and sends the reader looking for a
        // capital letter that is not there.
        let err = parse("[scoop]\npackages = [\"fzf\", \"fzf\"]\n").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("fzf"), "name the spelling: {msg}");
        assert!(
            !msg.contains("differ only in case"),
            "these do not differ in case at all: {msg}"
        );
    }

    #[test]
    fn a_duplicate_scoop_opts_key_is_rejected_rather_than_silently_clobbered() {
        // TOML cannot express a literal duplicate key, so serde never sees a
        // collision -- the collision is created by Name's folding. Measured
        // behaviour before this fix: one entry, the FIRST key, the LAST value.
        let err =
            parse("[scoop.opts]\npython = { arch = \"64bit\" }\nPython = { arch = \"arm64\" }\n")
                .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("python") && msg.contains("Python"),
            "got: {msg}"
        );
    }

    #[test]
    fn a_duplicate_winget_name_is_rejected_too() {
        let err = parse("[winget]\npackages = [\"Git.Git\", \"git.git\"]\n").unwrap_err();
        assert!(format!("{err:#}").contains("Git.Git"));
    }

    #[test]
    fn distinct_names_are_still_accepted() {
        // The guard must not reject a legitimate config.
        let cfg = parse("[scoop]\npackages = [\"fzf\", \"bat\", \"ripgrep\"]\n").unwrap();
        assert_eq!(cfg.scoop.packages.len(), 3);
    }

    #[test]
    fn a_bucket_declaration_splits_into_a_name_and_an_optional_url() {
        let cfg = parse(
            "[scoop]\nbuckets = [\"main\", \"extras\", \
             \"xom11=https://github.com/xom11/scoop-bucket\"]\n",
        )
        .unwrap();
        let b = &cfg.scoop.buckets;
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].name, Name::new("main"));
        assert_eq!(b[0].url, None);
        assert_eq!(b[2].name, Name::new("xom11"));
        assert_eq!(
            b[2].url.as_deref(),
            Some("https://github.com/xom11/scoop-bucket")
        );
    }

    #[test]
    fn a_bucket_name_that_could_leave_its_directory_is_refused_at_parse_time() {
        for bad in ["../evil", "a/b", "-oops", "", "c:\\x"] {
            let text = format!("[scoop]\nbuckets = [\"{bad}=https://example.invalid/x\"]\n");
            assert!(parse(&text).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn a_bucket_url_must_look_like_a_url() {
        assert!(parse("[scoop]\nbuckets = [\"x=not a url\"]\n").is_err());
        assert!(parse("[scoop]\nbuckets = [\"x=https://example.invalid/b\"]\n").is_ok());
        assert!(parse("[scoop]\nbuckets = [\"x=git@example.invalid:b.git\"]\n").is_ok());
    }

    #[test]
    fn two_bucket_declarations_naming_the_same_bucket_are_refused() {
        let err =
            parse("[scoop]\nbuckets = [\"main\", \"MAIN=https://x.invalid/y\"]\n").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("main") && msg.contains("MAIN"), "{msg}");
    }

    #[test]
    fn whitespace_around_the_equals_sign_is_trimmed_from_both_sides() {
        // Measured before the trim: "xom11 = https://…" was wrongly REJECTED
        // (the leading space made starts_with("https://") false), and
        // "xom11 = git@…" was wrongly ACCEPTED with a trailing space baked
        // into the name and a leading space baked into the url -- silent
        // corruption, worse than either a clean accept or a clean refuse.
        let cfg = parse("[scoop]\nbuckets = [\"xom11 = https://example.invalid/b\"]\n").unwrap();
        assert_eq!(cfg.scoop.buckets[0].name.to_string(), "xom11");
        assert_eq!(
            cfg.scoop.buckets[0].url.as_deref(),
            Some("https://example.invalid/b")
        );

        let cfg = parse("[scoop]\nbuckets = [\"xom11 = git@example.invalid:b.git\"]\n").unwrap();
        assert_eq!(cfg.scoop.buckets[0].name.to_string(), "xom11");
        assert_eq!(
            cfg.scoop.buckets[0].url.as_deref(),
            Some("git@example.invalid:b.git")
        );
    }

    #[test]
    fn a_package_can_name_the_bucket_it_comes_from() {
        // The only place this information can live. Two declared buckets can
        // both carry an app, and neither pkg.lock (which does not exist yet
        // for a new package) nor the machine (install.json loses `bucket` for
        // anything dotpkg installed) can answer which one the user meant.
        let cfg = parse(
            "[scoop]\nbuckets = [\"main\", \"extras\"]\npackages = [\"tool\"]\n\
             [scoop.opts]\ntool = { bucket = \"extras\" }\n",
        )
        .unwrap();
        assert_eq!(
            cfg.scoop.opts[&Name::new("tool")].bucket.as_deref(),
            Some("extras")
        );
        // arch and bucket are independent, and neither may require the other.
        let cfg = parse("[scoop.opts]\ntool = { arch = \"arm64\" }\n").unwrap();
        assert_eq!(cfg.scoop.opts[&Name::new("tool")].bucket, None);
    }

    #[test]
    fn a_bucket_opt_that_could_leave_its_directory_is_refused_at_parse_time() {
        // Same rule as `[scoop] buckets`: this string becomes
        // `$SCOOP/buckets/<it>` and a git argument.
        for bad in ["../evil", "a/b", "-oops", "", "c:\\x"] {
            let text = format!("[scoop.opts]\ntool = {{ bucket = \"{bad}\" }}\n");
            assert!(parse(&text).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn winget_guard_names_are_normalised_the_way_running_processes_reports_them() {
        // Measured on a14: `Tailscale.Tailscale` is installed and its live
        // processes are `tailscaled` and `tailscale-ipn`, neither of which is
        // the id, the display name, or the last dotted segment. This table is
        // the only mechanism that reaches them -- winget creates no package
        // directory for a non-portable install.
        //
        // The value is written with an extension and mixed case on purpose:
        // `sys::running_processes` lowercases and strips a known executable
        // suffix, so an unfolded comparison silently never matches.
        let cfg = parse(
            r#"
[winget]
packages = ["Tailscale.Tailscale"]

[winget.guard]
"Tailscale.Tailscale" = ["Tailscaled.EXE", "tailscale-ipn"]
"#,
        )
        .unwrap();
        assert_eq!(
            cfg.winget.guard.get(&Name::new("Tailscale.Tailscale")),
            Some(&vec!["tailscaled".to_string(), "tailscale-ipn".to_string()])
        );
    }

    #[test]
    fn a_winget_guard_name_that_is_empty_after_folding_is_a_parse_error() {
        // An empty string in the guard list would sit in the comparison set
        // matching nothing, while reading in pkg.toml as protection.
        let err = parse(
            r#"
[winget.guard]
"Tailscale.Tailscale" = ["  "]
"#,
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("[winget.guard]"), "message was: {msg}");
        assert!(msg.contains("Tailscale.Tailscale"), "message was: {msg}");
    }

    #[test]
    fn a_typo_in_the_winget_guard_table_name_is_refused_not_ignored() {
        // deny_unknown_fields, for the reason this file's `packagess` test
        // already gives: a typo must not read as "you declared nothing".
        assert!(parse(
            r#"
[winget]
packages = ["Tailscale.Tailscale"]
guards = { }
"#
        )
        .is_err());
    }

    #[test]
    fn an_absent_winget_guard_table_is_an_empty_map_not_a_failure() {
        let cfg = parse("[winget]\npackages = [\"Git.Git\"]\n").unwrap();
        assert!(cfg.winget.guard.is_empty());
    }

    #[test]
    fn two_guard_values_that_fold_to_the_same_process_name_collapse_to_one() {
        // `Tailscaled.EXE`, `TAILSCALED`, and `tailscaled` all fold to the
        // same string through `sys::normalize`. Without the `.contains()`
        // guard in `parse`, the guard list would carry that name three times
        // over -- a phantom duplicate nothing written in pkg.toml led a reader
        // to expect.
        //
        // **What it is not:** a matching fix. That was a forecast when this
        // test was written and is now checkable -- the value's only consumer,
        // `backend::apply_guard_overrides`, appends each name to
        // `Installed.bins` only when it is not already there, and `Running`'s
        // matchers only ask whether a string is in a set. So this pins the
        // parsed value's own shape, and a reader must not take it for a claim
        // that a duplicate would have broken the fence.
        let cfg = parse(
            r#"
[winget.guard]
"Tailscale.Tailscale" = ["Tailscaled.EXE", "TAILSCALED", "tailscaled"]
"#,
        )
        .unwrap();
        assert_eq!(
            cfg.winget.guard.get(&Name::new("Tailscale.Tailscale")),
            Some(&vec!["tailscaled".to_string()])
        );
    }

    #[test]
    fn two_winget_guard_keys_differing_only_in_case_are_rejected() {
        // The same hazard `a_duplicate_scoop_opts_key_is_rejected_rather_than_silently_clobbered`
        // guards against, for `[winget.guard]`'s own keys: TOML cannot
        // express a literal duplicate key, so serde never sees a collision
        // -- it is created by `Name`'s folding, inside `fold_map`, same as
        // `[scoop.opts]`.
        let err = parse(
            r#"
[winget.guard]
"Tailscale.Tailscale" = ["tailscaled"]
"tailscale.tailscale" = ["tailscale-ipn"]
"#,
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("Tailscale.Tailscale") && msg.contains("tailscale.tailscale"),
            "got: {msg}"
        );
    }

    #[test]
    fn a_bad_bucket_name_is_blamed_on_pkg_toml_not_on_a_lock_that_is_not_involved() {
        // ensure_plain_component's message used to hardcode "the lock's",
        // which is simply false here -- there is no lock, and the offending
        // text is sitting in pkg.toml.
        let err =
            parse("[scoop]\nbuckets = [\"../evil=https://example.invalid/x\"]\n").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("pkg.toml"), "name the actual file: {msg}");
        assert!(
            !msg.contains("lock"),
            "must not send the reader to the wrong file: {msg}"
        );
    }
}
