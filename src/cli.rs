//! Reading the command line.
//!
//! The model and effort lists come from the harness, so neither the help text
//! nor what it accepts can drift from what the harness really takes. Spawns
//! are separated by [`SEPARATOR`] because bare pairs would read an unquoted
//! description as a second spawn.

use std::path::PathBuf;

use crate::creation::Wanted;
use crate::error::{Error, Result};
use crate::harness::{self, Choice};

/// What separates one spawn from the next. It keeps an unquoted description an
/// error rather than a guessed second spawn.
const SEPARATOR: &str = "--and";

/// What the app was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// Start a session on a worktree of its own, for each of these.
    Spawn(Vec<Wanted>),
    /// Open with nothing running and a draft in the slot, to be written there.
    Compose,
    /// Say how to use the app.
    Help,
}

/// Work out what the arguments asked for.
///
/// The line is cut into one group per spawn, and each group is read on its
/// own. A line with nothing on it opens the app on a blank form rather than
/// being a mistake.
pub fn parse(arguments: Vec<String>) -> Result<Invocation> {
    let given: Vec<String> = arguments.into_iter().skip(1).collect();
    if given.is_empty() {
        return Ok(Invocation::Compose);
    }

    let mut requests = Vec::new();
    for group in given.split(|argument| argument == SEPARATOR) {
        match asked_for(group)? {
            Asked::Help => return Ok(Invocation::Help),
            Asked::Spawn(request) => requests.push(request),
        }
    }

    Ok(Invocation::Spawn(requests))
}

/// What one group of arguments asked for.
enum Asked {
    /// One spawn, with the choices that group made.
    Spawn(Wanted),
    /// The usage text, which any group can ask for and which answers the lot.
    Help,
}

/// Read one spawn's worth of the command line.
fn asked_for(group: &[String]) -> Result<Asked> {
    let mut positional: Vec<&str> = Vec::new();
    let mut model: Option<String> = None;
    let mut effort: Option<String> = None;

    let mut arguments = group.iter().map(String::as_str);
    while let Some(argument) = arguments.next() {
        match argument {
            "-h" | "--help" => return Ok(Asked::Help),
            "--model" => model = Some(value_for("--model", arguments.next())?),
            "--level" => effort = Some(value_for("--level", arguments.next())?),
            flag if flag.starts_with('-') && flag != "-" => {
                return Err(Error::new(format!("unknown option `{flag}`")));
            }
            _ => positional.push(argument),
        }
    }

    let (repository, work) = match positional.as_slice() {
        [repository, work] => ((*repository).to_string(), (*work).to_string()),
        [] | [_] => {
            return Err(Error::new(
                "expected a repository and a description of the work",
            ));
        }
        too_many => {
            return Err(Error::new(format!(
                "expected a repository and a description of the work, but got {} arguments — \
                 the description is one argument, so it needs quoting, and `{SEPARATOR}` is \
                 what separates one spawn from the next",
                too_many.len()
            )));
        }
    };

    // The flags become anonymous ids here, in the order the harness offers its
    // lists — the same thing a form hands over.
    Ok(Asked::Spawn(Wanted {
        repository: PathBuf::from(repository),
        work,
        answers: vec![
            chosen(model, "model", &harness::models(), harness::default_model())?,
            chosen(
                effort,
                "effort level",
                &harness::effort_levels(),
                harness::default_effort_level(),
            )?,
        ],
    }))
}

/// How to use the app. The choices come from the harness, so the help text
/// cannot drift from what the app will actually accept.
pub fn usage() -> String {
    format!(
        "harness-launcher — start coding sessions on worktrees of their own\n\
         \n\
         usage:\n    \
             harness-launcher\n    \
             harness-launcher <repository> <work> [options]\n                     \
                              [{SEPARATOR} <repository> <work> [options]]...\n\
         \n    \
             with nothing at all, it opens on a blank form and starts what you\n    \
             write there; the arguments below are the same thing said up front\n\
         \n    \
             <repository>  a local git repository, or any directory inside one\n    \
             <work>        what the session should do, in your own words\n    \
             {SEPARATOR}         start another session beside it, on any repository\n\
         \n\
         options, chosen per session:\n    \
             --model <id>  {}\n    \
             --level <id>  {}\n    \
             -h, --help    show this\n\
         \n\
         everything you type goes to whatever is in the slot; F6 and F7 move\n\
         between them, F2 starts a draft of another session, F5 starts what a\n\
         draft describes, F3 throws a draft away — it asks first — F9 retires\n\
         the session you are on, and F10 quits and leaves every one of them\n\
         running.\n",
        offer(&harness::models(), harness::default_model()),
        offer(&harness::effort_levels(), harness::default_effort_level()),
    )
}

/// What a choice looks like in the usage text.
fn offer(choices: &[Choice], default: Choice) -> String {
    format!("{}; default {}", listed(choices), default.id)
}

/// Read the value of an option, or say which option was left dangling.
fn value_for(flag: &str, value: Option<&str>) -> Result<String> {
    value
        .map(str::to_string)
        .ok_or_else(|| Error::new(format!("`{flag}` needs a value")))
}

/// Resolve what the user asked for against what the harness offers. Left
/// unsaid, it is whatever the harness would pick.
fn chosen(
    asked_for: Option<String>,
    what: &str,
    offered: &[Choice],
    default: Choice,
) -> Result<String> {
    let Some(asked_for) = asked_for else {
        return Ok(default.id.to_string());
    };

    if offered.iter().any(|choice| choice.id == asked_for) {
        return Ok(asked_for);
    }

    Err(Error::new(format!(
        "`{asked_for}` is not a {what} this harness offers — {}",
        listed(offered)
    )))
}

/// Choices, as a sentence.
fn listed(choices: &[Choice]) -> String {
    let listed: Vec<String> = choices
        .iter()
        .map(|choice| format!("{} ({})", choice.id, choice.label))
        .collect();

    listed.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_arguments(arguments: &[&str]) -> Result<Invocation> {
        let mut all = vec!["harness-launcher".to_string()];
        all.extend(arguments.iter().map(|argument| (*argument).to_string()));

        parse(all)
    }

    fn spawns(arguments: &[&str]) -> Vec<Wanted> {
        match parse_arguments(arguments).unwrap() {
            Invocation::Spawn(wanted) => wanted,
            otherwise => panic!("expected a spawn, got {otherwise:?}"),
        }
    }

    /// The one spawn, for the tests that ask for exactly one.
    fn spawn(arguments: &[&str]) -> Wanted {
        let mut wanted = spawns(arguments);

        assert_eq!(wanted.len(), 1, "expected one spawn: {wanted:?}");
        wanted.remove(0)
    }

    #[test]
    fn a_repository_and_the_work_are_all_it_takes() {
        let request = spawn(&["/code/project", "add retry logic"]);

        assert_eq!(request.repository, PathBuf::from("/code/project"));
        assert_eq!(request.work, "add retry logic");
    }

    #[test]
    fn unsaid_choices_fall_back_to_what_the_harness_would_pick() {
        let request = spawn(&["/code/project", "add retry logic"]);

        assert_eq!(
            request.answers,
            [
                harness::default_model().id,
                harness::default_effort_level().id
            ]
        );
    }

    #[test]
    fn the_choices_can_be_made_explicitly() {
        let request = spawn(&[
            "/code/project",
            "add retry logic",
            "--model",
            "haiku",
            "--level",
            "max",
        ]);

        assert_eq!(request.answers, ["haiku", "max"]);
    }

    #[test]
    fn a_choice_the_harness_does_not_offer_is_refused() {
        assert!(parse_arguments(&["/code/project", "work", "--model", "something-else"]).is_err());
        assert!(parse_arguments(&["/code/project", "work", "--level", "colossal"]).is_err());
    }

    #[test]
    fn work_that_looks_like_a_flag_is_still_work() {
        let request = spawn(&["/code/project", "make --model configurable"]);

        assert_eq!(request.work, "make --model configurable");
    }

    #[test]
    fn an_unquoted_description_is_refused_rather_than_half_read() {
        assert!(parse_arguments(&["/code/project", "add", "retry", "logic"]).is_err());
        assert!(parse_arguments(&["/code/project", "add", "retry"]).is_err());
    }

    #[test]
    fn several_spawns_can_be_started_in_one_breath() {
        let requests = spawns(&[
            "/code/project",
            "add retry logic",
            "--and",
            "/code/other",
            "fix the flake",
        ]);

        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].repository, PathBuf::from("/code/project"));
        assert_eq!(requests[0].work, "add retry logic");
        assert_eq!(requests[1].repository, PathBuf::from("/code/other"));
        assert_eq!(requests[1].work, "fix the flake");
    }

    #[test]
    fn several_spawns_can_share_one_repository() {
        let requests = spawns(&[
            "/code/project",
            "add retry logic",
            "--and",
            "/code/project",
            "fix the flake",
        ]);

        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].repository, requests[1].repository);
        assert_ne!(requests[0].work, requests[1].work);
    }

    #[test]
    fn each_spawn_makes_its_own_choices() {
        let requests = spawns(&[
            "/code/project",
            "add retry logic",
            "--model",
            "haiku",
            "--and",
            "/code/other",
            "fix the flake",
            "--level",
            "max",
        ]);

        assert_eq!(
            requests[0].answers,
            ["haiku", harness::default_effort_level().id]
        );
        assert_eq!(requests[1].answers, [harness::default_model().id, "max"]);
    }

    #[test]
    fn a_separator_with_nothing_after_it_is_refused_rather_than_ignored() {
        assert!(parse_arguments(&["/code/project", "add retry logic", "--and"]).is_err());
        assert!(parse_arguments(&["--and", "/code/project", "add retry logic"]).is_err());
    }

    #[test]
    fn one_spawn_that_makes_no_sense_refuses_the_whole_line() {
        assert!(
            parse_arguments(&[
                "/code/project",
                "add retry logic",
                "--and",
                "/code/other",
                "--model",
                "something-else",
            ])
            .is_err()
        );
    }

    #[test]
    fn nothing_at_all_opens_the_app_on_a_blank_form() {
        assert_eq!(parse_arguments(&[]).unwrap(), Invocation::Compose);
    }

    #[test]
    fn too_little_to_go_on_is_refused_rather_than_taken_as_nothing() {
        assert!(parse_arguments(&["/code/project"]).is_err());
        assert!(parse_arguments(&["--model", "haiku"]).is_err());
    }

    #[test]
    fn a_dangling_option_is_refused() {
        assert!(parse_arguments(&["/code/project", "work", "--model"]).is_err());
    }

    #[test]
    fn an_unknown_option_is_refused() {
        assert!(parse_arguments(&["/code/project", "work", "--turbo"]).is_err());
    }

    #[test]
    fn help_is_asked_for_either_way() {
        assert_eq!(parse_arguments(&["--help"]).unwrap(), Invocation::Help);
        assert_eq!(parse_arguments(&["-h"]).unwrap(), Invocation::Help);
    }

    #[test]
    fn the_usage_text_lists_what_the_harness_offers() {
        let usage = usage();

        for choice in harness::models() {
            assert!(usage.contains(choice.id), "{usage}");
        }
        for choice in harness::effort_levels() {
            assert!(usage.contains(choice.id), "{usage}");
        }
    }

    #[test]
    fn the_usage_text_says_how_to_leave() {
        assert!(usage().contains("F10"), "{}", usage());
    }

    #[test]
    fn the_usage_text_says_how_to_start_a_session_from_inside_the_app() {
        assert!(usage().contains("F2"), "{}", usage());
    }

    #[test]
    fn the_usage_text_says_how_to_ask_for_more_than_one() {
        assert!(usage().contains(SEPARATOR), "{}", usage());
    }

    #[test]
    fn the_usage_text_says_that_the_arguments_can_be_left_off_entirely() {
        assert!(usage().contains("with nothing at all"), "{}", usage());
    }
}
