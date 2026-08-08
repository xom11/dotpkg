use crate::config::Config;
use crate::model::SCOOP;
use crate::state::State;
use anyhow::Result;

/// Refuse a plan built from a config that declares nothing while dotpkg owns
/// something.
///
/// An empty or truncated `pkg.toml` parses successfully to zero packages —
/// every field is `#[serde(default)]` — and every owned package then becomes a
/// prune. Verified against the merged planner: five owned packages, empty
/// config, five prunes, no signal of any kind.
///
/// This is checked before anything else happens, and **`--yes` does not bypass
/// it**. `--yes` means "I have read the plan"; an empty config is file
/// corruption, so the plan itself is the thing that cannot be trusted.
/// Overriding takes its own flag.
///
/// Deliberately no ratio or count threshold. A user who genuinely deletes half
/// their `pkg.toml` is shown the plan and asked, which is the protection that
/// already exists.
pub fn mass_prune_guard(declared: &Config, state: &State) -> Result<()> {
    if !declared.scoop.packages.is_empty() {
        return Ok(());
    }
    let owned = state.owned_count(SCOOP);
    anyhow::ensure!(
        owned == 0,
        "pkg.toml declares no scoop packages but dotpkg owns {owned}. \
         Refusing to prune everything. If the file is right, pass --allow-empty-config."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Name;
    use crate::state::Ownership;

    fn owning(names: &[&str]) -> State {
        let mut s = State::default();
        for n in names {
            s.set(SCOOP, &Name::new(*n), Ownership::Installed);
        }
        s
    }

    #[test]
    fn an_empty_config_with_owned_packages_is_refused() {
        let err = mass_prune_guard(
            &crate::config::parse("").unwrap(),
            &owning(&["fzf", "bat", "ripgrep", "neovim", "kanata"]),
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains('5'), "the count is the whole point: {msg}");
        assert!(
            msg.contains("--allow-empty-config"),
            "say how to override: {msg}"
        );
    }

    #[test]
    fn an_empty_config_on_a_machine_that_owns_nothing_is_fine() {
        // A fresh machine. status should report everything as unmanaged and
        // apply should do nothing -- not error.
        mass_prune_guard(&crate::config::parse("").unwrap(), &State::default()).unwrap();
    }

    #[test]
    fn a_config_that_declares_anything_is_not_the_corruption_case() {
        mass_prune_guard(
            &crate::config::parse("[scoop]\npackages = [\"fzf\"]\n").unwrap(),
            &owning(&["fzf", "bat", "ripgrep"]),
        )
        .unwrap();
    }
}
