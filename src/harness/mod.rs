//! The harness seam: the one place that knows what the app launches.
//!
//! Everything harness-specific lives here — the binary name, its flags, its
//! environment, the choices it offers. Two invariants keep it that way, and both
//! are checked mechanically in CI:
//!
//! - nothing outside this module names the harness;
//! - this module touches no process, filesystem or tmux API.
//!
//! The second is what makes the first hold: **this module performs no I/O.** It
//! translates a spec into plain data; the app is what acts. That also makes the
//! whole seam testable without a process, a terminal or a multiplexer.
//!
//! There is deliberately no harness *abstraction* here — no trait with one
//! implementation behind it. One adapter makes a seam hypothetical; two make it
//! real, and an interface shaped by a single harness is an interface shaped by
//! *this* harness.

use std::path::PathBuf;

/// One option the harness offers, as the spawn form will show it.
///
/// The question this answers is "what does this harness let you choose?", not
/// "what flags does this harness take?" — the first survives a harness whose
/// choices come from a config file, the second does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// What the app passes back when this option is picked.
    pub id: &'static str,
    /// What the user reads.
    pub label: &'static str,
}

/// Everything the user chose, plus where the app put the worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    /// The spawn's name — also the branch's and the worktree directory's.
    pub name: String,
    /// The work to be done, in the user's own words.
    pub work: String,
    /// The id of a [`models`] choice.
    pub model: String,
    /// The id of an [`effort_levels`] choice.
    pub effort: String,
    /// The worktree the session runs in.
    pub worktree: PathBuf,
}

/// A process to start, described rather than started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRecipe {
    /// The program to run, resolved on the caller's `PATH`.
    pub program: String,
    /// Its arguments, one per element — never a shell string.
    pub args: Vec<String>,
    /// Environment to add to the child's.
    pub env: Vec<(String, String)>,
    /// The working directory to start it in.
    pub cwd: PathBuf,
}

const OPUS: Choice = Choice {
    id: "opus",
    label: "Opus — the most capable",
};
const SONNET: Choice = Choice {
    id: "sonnet",
    label: "Sonnet — balanced",
};
const HAIKU: Choice = Choice {
    id: "haiku",
    label: "Haiku — the fastest",
};

const LOW: Choice = Choice {
    id: "low",
    label: "Low",
};
const MEDIUM: Choice = Choice {
    id: "medium",
    label: "Medium",
};
const HIGH: Choice = Choice {
    id: "high",
    label: "High",
};
const EXTRA_HIGH: Choice = Choice {
    id: "xhigh",
    label: "Extra high",
};
const MAXIMUM: Choice = Choice {
    id: "max",
    label: "Maximum",
};

/// The models a spawn can run on, most capable first.
pub fn models() -> Vec<Choice> {
    vec![OPUS, SONNET, HAIKU]
}

/// The effort levels a spawn can run at, least thinking first.
pub fn effort_levels() -> Vec<Choice> {
    vec![LOW, MEDIUM, HIGH, EXTRA_HIGH, MAXIMUM]
}

/// What a spawn runs on when the user says nothing.
///
/// Named rather than positional, because the two lists are not ordered the same
/// way: models run most-capable first, effort levels run by how much thinking
/// they buy. A default picked by index would follow whichever order it landed in
/// and change silently when one of them was re-ordered.
pub fn default_model() -> Choice {
    OPUS
}

/// The effort a spawn spends when the user says nothing.
///
/// Neither the least nor the most: a spawn is work handed over to run
/// unattended, which is the case for thinking about it — and `max` is a price
/// worth choosing deliberately rather than inheriting.
pub fn default_effort_level() -> Choice {
    HIGH
}

/// Turn a spec into the command line that starts the session.
///
/// The work is a positional argument, so it cannot be swallowed by a text box
/// and needs no verification afterwards — and because the app hands the child an
/// argument vector rather than a shell string, nothing in it needs quoting.
///
/// `CLAUDE_CODE_NO_FLICKER=1` forces the fullscreen renderer, which draws on the
/// alternate screen. It is a requirement rather than a preference: parking and
/// unparking a pane resizes it, and only the fullscreen renderer repaints
/// cleanly when that happens. It also keeps a transcript out of tmux's
/// scrollback, which is what makes twenty panes affordable.
///
/// The variable is the one Claude Code itself documents as the equivalent of the
/// `tui: "fullscreen"` setting — *"the flicker-free alt-screen renderer with
/// virtualized scrollback"* — read from the shipped binary's own settings schema
/// at v2.1.226, and confirmed by running a spawn and reading the child's
/// environment. It is an internal detail of another program, so treat it as
/// fallible: if a future version drops it, the fullscreen requirement stops
/// holding **silently**, and no test here can tell you that. Only starting one
/// and looking can.
pub fn launch_recipe(spec: &SpawnSpec) -> LaunchRecipe {
    LaunchRecipe {
        program: "claude".to_string(),
        args: vec![
            "--model".to_string(),
            spec.model.clone(),
            "--effort".to_string(),
            spec.effort.clone(),
            "-n".to_string(),
            spec.name.clone(),
            spec.work.clone(),
        ],
        env: vec![("CLAUDE_CODE_NO_FLICKER".to_string(), "1".to_string())],
        cwd: spec.worktree.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SpawnSpec {
        SpawnSpec {
            name: "add-retry-logic-a7f3".to_string(),
            work: "add retry logic to the client".to_string(),
            model: "opus".to_string(),
            effort: "high".to_string(),
            worktree: PathBuf::from("/data/harness-launcher/worktrees/add-retry-logic-a7f3"),
        }
    }

    #[test]
    fn a_spec_becomes_one_command_line() {
        let recipe = launch_recipe(&spec());

        assert_eq!(recipe.program, "claude");
        assert_eq!(
            recipe.args,
            [
                "--model",
                "opus",
                "--effort",
                "high",
                "-n",
                "add-retry-logic-a7f3",
                "add retry logic to the client",
            ]
        );
    }

    #[test]
    fn the_work_is_the_last_argument_so_flags_cannot_swallow_it() {
        let recipe = launch_recipe(&spec());

        assert_eq!(recipe.args.last().unwrap(), "add retry logic to the client");
    }

    #[test]
    fn the_work_reaches_the_session_exactly_as_typed() {
        let mut spec = spec();
        spec.work = "fix the \"worktree\" cleanup; $HOME & --model sonnet\nsecond line".to_string();

        let recipe = launch_recipe(&spec);

        assert_eq!(recipe.args.last().unwrap(), &spec.work);
    }

    #[test]
    fn the_session_starts_in_the_worktree() {
        let spec = spec();

        assert_eq!(launch_recipe(&spec).cwd, spec.worktree);
    }

    #[test]
    fn the_fullscreen_renderer_is_forced_rather_than_inherited() {
        let recipe = launch_recipe(&spec());

        assert_eq!(
            recipe.env,
            [("CLAUDE_CODE_NO_FLICKER".to_string(), "1".to_string())]
        );
    }

    #[test]
    fn every_effort_level_the_harness_offers_is_one_it_accepts() {
        let offered: Vec<&str> = effort_levels().iter().map(|choice| choice.id).collect();

        assert_eq!(offered, ["low", "medium", "high", "xhigh", "max"]);
    }

    #[test]
    fn the_defaults_are_choices_the_harness_actually_offers() {
        assert!(models().contains(&default_model()));
        assert!(effort_levels().contains(&default_effort_level()));
    }

    #[test]
    fn every_choice_has_something_to_show_the_user() {
        for choice in models().iter().chain(effort_levels().iter()) {
            assert!(!choice.id.is_empty());
            assert!(!choice.label.is_empty());
        }
    }
}
