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

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The program a spawn runs.
const PROGRAM: &str = "claude";

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

/// One list of options the harness offers, as the spawn form will show it.
///
/// The form draws a title and some labels and can say which one is picked. It
/// is told nothing else — not that one of these lists is about models, not what
/// any id means, not that the harness has exactly two of them. That is what
/// keeps the form a form: the question it asks is always *"which of these?"*,
/// and every answer to it comes from here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    /// What the form calls the list.
    pub title: &'static str,
    /// What can be picked, in the order the harness wants them read.
    pub options: Vec<Choice>,
    /// What is picked before the user picks anything.
    pub default: Choice,
}

/// Everything the harness lets you choose when starting a session.
///
/// In the order the form asks: what runs the work, then how much of itself it
/// spends on it.
///
/// A harness with nothing to offer under one of these headings returns an empty
/// list rather than a placeholder, and the form omits that control entirely —
/// which is why this returns the lists as they are rather than promising each
/// of them has something in it.
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
/// Whatever asked the questions — a form in the app, or a command line — asked
/// *"which of these?"* and was told nothing else, so what comes back is a bag
/// of ids with no headings on it. This is where they become the harness's own
/// vocabulary again: **each list recognises its own**, so the order the answers
/// arrive in is not a contract, and a list dropped for having nothing in it
/// leaves no hole to line up against.
///
/// Anything unanswered is what the harness would have picked itself. The caller
/// cannot tell one list from another — that is the whole point of asking the
/// question the way [`choices`] does — so it cannot be asked to check them
/// either.
///
/// *Accepted cost, and it is this module's to keep true:* recognising an answer
/// by looking for it means **an id offered under two headings would be
/// ambiguous**, and the first list asked would win it. Nothing outside here can
/// see that, let alone prevent it, so the rule lives with the lists: ids are
/// unique across every list [`choices`] offers. A test below holds it.
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
/// line that fixes it not being.
///
/// **Described here and checked by the app**, like everything else across this
/// seam: the module knows *what* must be there and what to say about it missing,
/// and does none of the finding out — no lookup, no process, no filesystem.
/// `PATH` is named once, inside the sentence handed to the user, because that is
/// where they will have to go and fix it; naming it is not consulting it, and
/// nothing here ever reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Requirement {
    /// The program that must be runnable — the same one [`launch_recipe`] names.
    pub program: &'static str,
    /// What the user can do about it not being there, in one line.
    pub fix: &'static str,
}

/// The one thing that has to be installed for this harness to run.
///
/// The fix names no installation command on purpose: how Claude Code is
/// installed differs by machine and changes between versions, and a refusal
/// confidently telling somebody to run the wrong one is worse than a refusal
/// that says exactly what is wrong and leaves the how to them.
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
/// alternate screen. It is a requirement rather than a preference, and the
/// reasons are the app's own rather than the multiplexer's: the grid the app
/// holds per spawn is a **screen and not a history**, so a spawn costs one
/// screenful of cells and twenty cost megabytes; and the alternate screen is
/// what keeps a transcript out of a scrollback **the app does not implement** —
/// under the classic renderer output scrolls off the top of the grid and is
/// simply gone.
///
/// *Its two original reasons have both expired, and are recorded here because
/// the superseded reasoning is the part that would otherwise be silently
/// reinvented.* It was chosen for tmux's server memory, which stopped mattering
/// when the app took the grids over, and reinforced by a redraw test about
/// parking a pane — a mechanism that no longer exists. A decision's reasons can
/// expire before the decision does; these were re-derived rather than
/// inherited.
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
///
/// Two answers and an admission. The admission is here rather than as an
/// `Err` because a record that will not resolve is not a failure of the app —
/// it is one rung of the ladder handing over to the next, and the sentence it
/// carries is shown to a person rather than logged.
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
///
/// The app reads the environment and hands the answer back to [`status_files`];
/// the name of the variable is a harness fact, so it lives here.
pub fn config_directory_variable() -> &'static str {
    "CLAUDE_CONFIG_DIR"
}

/// Where the harness keeps its session records, given the environment.
///
/// **This is an undocumented internal detail of another program**, present
/// since v2.1.139, and the app treats it as fallible everywhere: a missing
/// directory, a missing file, or a record it cannot read is a rung of the
/// ladder that did not answer, never a refusal and never a status.
///
/// `None` means there is nowhere to look at all, which is the same answer as
/// looking and finding nothing — the caller has one case, not two.
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
/// `busy` and `waiting` are both the agent working. `idle` is the agent
/// stopped, and so is `shell` — a turn that ended with a background shell still
/// alive, written by versions from v2.1.197.
///
/// The record names the process it belongs to, and that is checked against the
/// process it was looked up by: the files are keyed by process id, and an
/// operating system hands out process ids again. Believing a record left behind
/// by a session that has gone would report a spawn as working long after it
/// stopped, which is the one wrong answer this app is not allowed to give.
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

/// The captured record, with a status in it.
///
/// **The capture plus one field, not a recording.** The machine this was taken
/// from writes no `status` at all — see `captured/README.md` — so a record
/// carrying one could not be recorded here, though the field itself has been
/// confirmed present on an ordinary install. Everything around it is real.
///
/// It lives in the module that knows what a record is, so that the one place a
/// test departs from a recording is one place rather than several, and so that
/// replacing the capture one day fixes every test at once.
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
/// Two names, because neither is reliable alone: `command` is what the kernel
/// reports, **truncated to fifteen characters**, and `argv0` is whatever the
/// program was started as, which is often a path.
///
/// *Accepted blind spot:* a harness started through a wrapper that replaces
/// both — an interpreter invoked with the harness as its first argument, say —
/// is not recognised, because the harness's name appears in neither of the two
/// places looked at. The cost of that is a spawn read as stopped while it
/// works, which is the direction the app is content to be wrong in; widening
/// the search to the whole argument vector would trade it for reading `vim
/// /etc/claude` as a live agent, which is the direction it is not.
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

    /// The check and the launch must be about the same program, or the app
    /// would refuse over one binary and then try to start another.
    #[test]
    fn what_has_to_be_installed_is_the_program_a_spawn_actually_runs() {
        assert_eq!(requirement().program, launch_recipe(&spec()).program);
    }

    /// A refusal nobody can act on is barely better than a crash: this is the
    /// one place that knows what installing this harness means.
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

    /// The rule that makes an anonymous answer resolvable at all: an id
    /// belongs to exactly one of the lists this harness offers. Two lists
    /// sharing one would make an answer ambiguous, and whichever list was asked
    /// first would silently win it.
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
