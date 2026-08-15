//! The harness seam: the one place that knows what the app launches.
//!
//! Two invariants, both checked mechanically in CI:
//!
//! - nothing outside this module names the harness;
//! - this module performs no I/O — it translates a spec into plain data, and
//!   the app acts.
//!
//! There is deliberately no harness trait: one adapter makes a seam
//! hypothetical.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The program a spawn runs.
const PROGRAM: &str = "claude";

/// The glyph a row wears to say which harness its spawn is running.
///
/// One character, because rows are measured in characters and a wider glyph
/// would shorten every name in the list. Accepted cost: a terminal that draws
/// it double-width pushes its row one cell over.
pub const GLYPH: &str = "✻";

/// One option the harness offers, as the spawn form will show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// What the app passes back when this option is picked.
    pub id: &'static str,
    /// What the user reads.
    pub label: &'static str,
}

/// One list of options the harness offers, as the spawn form will show it.
/// The form is told titles and labels and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    /// What the form calls the list.
    pub title: &'static str,
    /// What can be picked, in the order the harness wants them read.
    pub options: Vec<Choice>,
    /// What is picked before the user picks anything.
    pub default: Choice,
}

/// Everything the harness lets you choose when starting a session, in the
/// order the form asks. An empty list means the form omits that control.
pub fn choices() -> Vec<Choices> {
    vec![
        Choices {
            title: "Model",
            options: models(),
            default: default_model(),
        },
        Choices {
            title: "Effort",
            options: effort_levels(),
            default: default_effort_level(),
        },
    ]
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

/// The spec a set of answers describes.
///
/// Answers arrive as anonymous ids; each list recognises its own, so their
/// order is not a contract, and anything unanswered gets the harness's own
/// default. This requires ids to be unique across every list [`choices`]
/// offers — a test below holds it.
pub fn spec_from(name: String, work: String, worktree: PathBuf, answers: &[String]) -> SpawnSpec {
    SpawnSpec {
        name,
        work,
        model: picked(&models(), answers, default_model()),
        effort: picked(&effort_levels(), answers, default_effort_level()),
        worktree,
    }
}

/// Whichever of these options was answered, or the one the harness would have
/// picked in the absence of an answer.
fn picked(offered: &[Choice], answers: &[String], default: Choice) -> String {
    answers
        .iter()
        .find(|answer| offered.iter().any(|choice| choice.id == answer.as_str()))
        .cloned()
        .unwrap_or_else(|| default.id.to_string())
}

/// What has to be installed before a spawn can be started at all, and the one
/// line that fixes it not being. Described here, checked by the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Requirement {
    /// The program that must be runnable — the same one [`launch_recipe`] names.
    pub program: &'static str,
    /// What the user can do about it not being there, in one line.
    pub fix: &'static str,
}

/// The one thing that has to be installed for this harness to run. The fix
/// names no installation command on purpose: it differs by machine, and a
/// confidently wrong command is worse than none.
pub fn requirement() -> Requirement {
    Requirement {
        program: PROGRAM,
        fix: "install Claude Code, or start this app from a shell that has it on PATH",
    }
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
pub fn default_model() -> Choice {
    OPUS
}

/// The effort a spawn spends when the user says nothing. Not `max`: that is a
/// price worth choosing deliberately rather than inheriting.
pub fn default_effort_level() -> Choice {
    HIGH
}

/// Turn a spec into the command line that starts the session.
///
/// The work is a positional argument in an argument vector, so nothing in it
/// needs quoting. `CLAUDE_CODE_NO_FLICKER=1` is a requirement, not a
/// preference: the grid the app holds per spawn is a screen and not a history,
/// and the alternate screen keeps the transcript out of a scrollback the app
/// does not implement. The variable is an internal detail of Claude Code
/// (verified at v2.1.226), so treat it as fallible: if a future version drops
/// it, the fullscreen requirement stops holding silently.
pub fn launch_recipe(spec: &SpawnSpec) -> LaunchRecipe {
    LaunchRecipe {
        program: PROGRAM.to_string(),
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

/// What the harness's own record says a session is doing, in the app's words.
/// `Unresolved` is not an `Err`: it is shown to a person, not logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// The agent is working.
    Working,
    /// The agent has stopped. Finished, waiting on an answer, or dead: the
    /// harness distinguishes those and the app deliberately does not.
    Stopped,
    /// The record did not resolve, and this is what was wrong with it.
    Unresolved(String),
}

/// Where the harness records what each of its live sessions is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusFiles {
    directory: PathBuf,
}

impl StatusFiles {
    /// The record the harness keeps for the session running as `pid`.
    pub fn of(&self, pid: u32) -> PathBuf {
        self.directory.join(format!("{pid}.json"))
    }
}

/// The environment variable that moves the harness's configuration elsewhere.
/// The app reads it and hands the answer to [`status_files`].
pub fn config_directory_variable() -> &'static str {
    "CLAUDE_CONFIG_DIR"
}

/// Where the harness keeps its session records, given the environment.
///
/// An undocumented internal detail of another program, present since v2.1.139,
/// and treated as fallible everywhere. `None` means nowhere to look, which the
/// caller treats the same as looking and finding nothing.
pub fn status_files(configured: Option<OsString>, home: Option<OsString>) -> Option<StatusFiles> {
    let configuration = match configured {
        Some(configured) if !configured.is_empty() => PathBuf::from(configured),
        _ => match home {
            Some(home) if !home.is_empty() => Path::new(&home).join(".claude"),
            _ => return None,
        },
    };

    Some(StatusFiles {
        directory: configuration.join("sessions"),
    })
}

/// Read one session record, as kept for the session running as `pid`.
///
/// `busy` and `waiting` are the agent working; `idle` and `shell` (written
/// from v2.1.197) are the agent stopped. The record's own pid is checked
/// because operating systems reuse process ids: a record left by a dead
/// session would otherwise report a spawn as working long after it stopped.
pub fn read_status(record: &str, pid: u32) -> Reading {
    let record: serde_json::Value = match serde_json::from_str(record) {
        Ok(record) => record,
        Err(error) => {
            return Reading::Unresolved(format!("its session record is not valid JSON: {error}"));
        }
    };

    match record["pid"].as_u64() {
        Some(named) if named == u64::from(pid) => {}
        Some(named) => {
            return Reading::Unresolved(format!(
                "its session record is for process {named}, not {pid} — \
                 it was left behind by a session that has gone"
            ));
        }
        None => {
            return Reading::Unresolved(
                "its session record does not say which process it belongs to".to_string(),
            );
        }
    }

    match record["status"].as_str() {
        Some("busy" | "waiting") => Reading::Working,
        Some("idle" | "shell") => Reading::Stopped,
        Some(unknown) => Reading::Unresolved(format!(
            "its session record says `{unknown}`, which is not a status this app knows"
        )),
        None => Reading::Unresolved(
            "its session record carries no status — the harness may be older than the \
             version that writes one"
                .to_string(),
        ),
    }
}

/// A real session record, copied from a real machine — see
/// `captured/README.md`. The session it belongs to ran as [`RECORDED_PID`].
#[cfg(test)]
const RECORDED: &str = include_str!("../../captured/session-record.json");

/// The process the captured record belongs to.
#[cfg(test)]
pub const RECORDED_PID: u32 = 531;

/// The captured record with a `status` field added — the machine it was taken
/// from writes none (see `captured/README.md`); everything around it is real.
#[cfg(test)]
pub fn recorded(status: &str) -> String {
    let rest = RECORDED
        .trim()
        .strip_prefix('{')
        .expect("the captured record is an object");

    format!("{{\"status\":\"{status}\",{rest}")
}

/// Whether a process in a pane's foreground process group is the harness.
///
/// Two names because neither is reliable alone: `command` is truncated to
/// fifteen characters by the kernel, and `argv0` is often a path. Accepted
/// blind spot: a harness behind a wrapper (an interpreter, say) is read as
/// stopped — searching the whole argument vector would instead read
/// `vim /etc/claude` as a live agent.
pub fn names_the_harness(command: &str, argv0: &str) -> bool {
    let program = argv0.rsplit('/').next().unwrap_or_default();

    command == PROGRAM || program == PROGRAM
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
    fn the_glyph_a_row_wears_is_one_character_wide() {
        assert_eq!(GLYPH.chars().count(), 1);
    }

    #[test]
    fn what_has_to_be_installed_is_the_program_a_spawn_actually_runs() {
        assert_eq!(requirement().program, launch_recipe(&spec()).program);
    }

    #[test]
    fn the_requirement_says_what_to_do_about_it_not_being_met() {
        assert!(!requirement().fix.is_empty());
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
    fn answers_become_a_spec_whatever_order_they_arrive_in() {
        let spec = spec_from(
            "add-retry-logic-a7f3".to_string(),
            "add retry logic to the client".to_string(),
            PathBuf::from("/data/harness-launcher/worktrees/add-retry-logic-a7f3"),
            &["max".to_string(), "haiku".to_string()],
        );

        assert_eq!(spec.model, "haiku");
        assert_eq!(spec.effort, "max");
    }

    #[test]
    fn an_unanswered_list_is_what_the_harness_would_have_picked() {
        let spec = spec_from(
            "add-retry-logic-a7f3".to_string(),
            "add retry logic".to_string(),
            PathBuf::from("/w/add-retry-logic-a7f3"),
            &["haiku".to_string()],
        );

        assert_eq!(spec.model, "haiku");
        assert_eq!(spec.effort, default_effort_level().id);
    }

    #[test]
    fn no_id_is_offered_under_two_headings() {
        let offered: Vec<&str> = choices()
            .iter()
            .flat_map(|choices| choices.options.iter().map(|choice| choice.id))
            .collect();

        let distinct: std::collections::HashSet<&str> = offered.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            offered.len(),
            "an id is offered under more than one heading: {offered:?}"
        );
    }

    #[test]
    fn an_answer_this_harness_never_offered_leaves_its_own_default() {
        let spec = spec_from(
            "add-retry-logic-a7f3".to_string(),
            "add retry logic".to_string(),
            PathBuf::from("/w/add-retry-logic-a7f3"),
            &["from-another-harness-entirely".to_string()],
        );

        assert_eq!(spec.model, default_model().id);
        assert_eq!(spec.effort, default_effort_level().id);
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
    fn every_list_the_form_is_offered_has_a_title_and_a_default_it_contains() {
        for choices in choices() {
            assert!(!choices.title.is_empty());
            assert!(
                choices.options.contains(&choices.default),
                "{} defaults to something it does not offer",
                choices.title
            );
        }
    }

    #[test]
    fn the_form_is_offered_the_choices_the_command_line_takes() {
        let offered: Vec<Vec<Choice>> = choices()
            .into_iter()
            .map(|choices| choices.options)
            .collect();

        assert_eq!(offered, [models(), effort_levels()]);
    }

    #[test]
    fn every_choice_has_something_to_show_the_user() {
        for choice in models().iter().chain(effort_levels().iter()) {
            assert!(!choice.id.is_empty());
            assert!(!choice.label.is_empty());
        }
    }

    #[test]
    fn a_working_agent_is_what_busy_and_waiting_both_mean() {
        assert_eq!(
            read_status(&recorded("busy"), RECORDED_PID),
            Reading::Working
        );
        assert_eq!(
            read_status(&recorded("waiting"), RECORDED_PID),
            Reading::Working
        );
    }

    #[test]
    fn a_stopped_agent_is_what_idle_and_shell_both_mean() {
        assert_eq!(
            read_status(&recorded("idle"), RECORDED_PID),
            Reading::Stopped
        );
        assert_eq!(
            read_status(&recorded("shell"), RECORDED_PID),
            Reading::Stopped
        );
    }

    #[test]
    fn a_status_this_app_does_not_know_resolves_to_nothing_rather_than_a_guess() {
        let reading = read_status(&recorded("compacting"), RECORDED_PID);

        let Reading::Unresolved(why) = reading else {
            panic!("a status from a later harness was mapped anyway: {reading:?}");
        };
        assert!(why.contains("compacting"), "{why}");
    }

    #[test]
    fn the_record_this_machine_really_writes_carries_no_status_and_says_so() {
        let reading = read_status(RECORDED, RECORDED_PID);

        let Reading::Unresolved(why) = reading else {
            panic!("a record with no status resolved to one: {reading:?}");
        };
        assert!(why.contains("no status"), "{why}");
    }

    #[test]
    fn a_record_left_behind_by_another_process_is_not_believed() {
        let reading = read_status(&recorded("busy"), RECORDED_PID + 1);

        let Reading::Unresolved(why) = reading else {
            panic!("a record for another process was read as this one's: {reading:?}");
        };
        assert!(why.contains(&RECORDED_PID.to_string()), "{why}");
    }

    #[test]
    fn something_that_is_not_a_record_at_all_resolves_to_nothing() {
        assert!(matches!(
            read_status("", RECORDED_PID),
            Reading::Unresolved(_)
        ));
        assert!(matches!(
            read_status("{\"status\": \"bu", RECORDED_PID),
            Reading::Unresolved(_)
        ));
        assert!(matches!(
            read_status("[]", RECORDED_PID),
            Reading::Unresolved(_)
        ));
    }

    #[test]
    fn the_records_live_under_the_configuration_directory_the_harness_was_given() {
        let files = status_files(
            Some("/elsewhere/config".into()),
            Some("/home/someone".into()),
        )
        .expect("a configured directory is somewhere to look");

        assert_eq!(
            files.of(4321),
            PathBuf::from("/elsewhere/config/sessions/4321.json")
        );
    }

    #[test]
    fn unconfigured_the_records_live_under_the_home_directory() {
        let files = status_files(None, Some("/home/someone".into()))
            .expect("a home directory is somewhere to look");

        assert_eq!(
            files.of(4321),
            PathBuf::from("/home/someone/.claude/sessions/4321.json")
        );
    }

    #[test]
    fn an_empty_configuration_variable_is_treated_as_unset() {
        let files = status_files(Some("".into()), Some("/home/someone".into()))
            .expect("a home directory is somewhere to look");

        assert_eq!(
            files.of(4321),
            PathBuf::from("/home/someone/.claude/sessions/4321.json")
        );
    }

    #[test]
    fn with_no_home_and_no_configuration_there_is_nowhere_to_look() {
        assert_eq!(status_files(None, None), None);
    }

    #[test]
    fn the_harness_is_recognised_by_a_truncated_name_or_the_path_it_was_started_as() {
        assert!(names_the_harness("claude", "claude"));
        assert!(names_the_harness(
            "stand-in-harnes",
            "/usr/local/bin/claude"
        ));
        assert!(!names_the_harness("bash", "-bash"));
        assert!(!names_the_harness("sleep", "sleep"));
        assert!(!names_the_harness("", ""));
    }

    #[test]
    fn a_harness_hidden_behind_a_wrapper_is_not_recognised_and_that_is_recorded() {
        assert!(!names_the_harness("node", "/usr/bin/node"));
    }
}
