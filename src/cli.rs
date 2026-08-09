//! Reading the command line.
//!
//! Two modes, one binary. The mode a person uses starts a spawn; the other draws
//! the list pane, and exists because the app has to be a pane in the window it
//! composes — so when it builds that window from scratch it starts itself again
//! inside it. Not a second program: the same one, in a different mode.

use std::env;
use std::path::PathBuf;

use crate::app::View;
use crate::error::{Error, Result};
use crate::harness::{self, Choice};

/// The flag that selects the list pane mode. Internal: not in the usage text,
/// because it is the app talking to itself.
const LIST_PANE_FLAG: &str = "--list-pane";

/// What the app was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// Start a session on a worktree of its own.
    Spawn(Request),
    /// Draw the list pane, and nothing else.
    ListPane(View),
    /// Say how to use the app.
    Help,
}

/// Everything a person chose when starting a spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The repository to work on — or any directory inside it.
    pub repository: PathBuf,
    /// The work to be done, in the user's own words.
    pub work: String,
    /// The id of a [`harness::models`] choice.
    pub model: String,
    /// The id of a [`harness::effort_levels`] choice.
    pub effort: String,
}

/// Work out what the arguments asked for.
pub fn parse(arguments: Vec<String>) -> Result<Invocation> {
    let given: Vec<String> = arguments.into_iter().skip(1).collect();
    if given.first().is_some_and(|first| first == LIST_PANE_FLAG) {
        return list_pane(&given[1..]);
    }

    let mut positional: Vec<String> = Vec::new();
    let mut model: Option<String> = None;
    let mut effort: Option<String> = None;

    let mut arguments = given.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "-h" | "--help" => return Ok(Invocation::Help),
            "--model" => model = Some(value_for("--model", arguments.next())?),
            "--level" => effort = Some(value_for("--level", arguments.next())?),
            flag if flag.starts_with('-') && flag != "-" => {
                return Err(Error::new(format!("unknown option `{flag}`")));
            }
            _ => positional.push(argument),
        }
    }

    let (repository, work) = match positional.as_slice() {
        [repository, work] => (repository.clone(), work.clone()),
        [] | [_] => {
            return Err(Error::new(
                "expected a repository and a description of the work",
            ));
        }
        too_many => {
            return Err(Error::new(format!(
                "expected a repository and a description of the work, but got {} arguments — \
                 the description is one argument, so it needs quoting",
                too_many.len()
            )));
        }
    };

    Ok(Invocation::Spawn(Request {
        repository: PathBuf::from(repository),
        work,
        model: chosen(model, "model", &harness::models(), harness::default_model())?,
        effort: chosen(
            effort,
            "effort level",
            &harness::effort_levels(),
            harness::default_effort_level(),
        )?,
    }))
}

/// How to use the app. The choices come from the harness, so the help text
/// cannot drift from what the app will actually accept.
pub fn usage() -> String {
    format!(
        "harness-launcher — start a coding session on a worktree of its own\n\
         \n\
         usage:\n    \
             harness-launcher <repository> <work> [--model <id>] [--level <id>]\n\
         \n    \
             <repository>  a local git repository, or any directory inside one\n    \
             <work>        what the session should do, in your own words\n\
         \n\
         options:\n    \
             --model <id>  {}\n    \
             --level <id>  {}\n    \
             -h, --help    show this\n",
        offer(&harness::models(), harness::default_model()),
        offer(&harness::effort_levels(), harness::default_effort_level()),
    )
}

/// What a choice looks like in the usage text.
fn offer(choices: &[Choice], default: Choice) -> String {
    format!("{}; default {}", listed(choices), default.id)
}

/// How the app starts itself again as the list pane of a window it just built.
pub fn list_pane_command(view: &View) -> Result<Vec<String>> {
    let executable = env::current_exe()
        .map_err(|error| Error::new(format!("could not find the app's own binary: {error}")))?;
    let executable = executable
        .to_str()
        .ok_or_else(|| Error::new("the app's own path is not valid UTF-8"))?;

    Ok(vec![
        executable.to_string(),
        LIST_PANE_FLAG.to_string(),
        view.repository.clone(),
        view.spawn.clone(),
        view.branch.clone(),
        view.worktree.clone(),
    ])
}

/// The list pane mode's arguments, which the app wrote itself.
fn list_pane(arguments: &[String]) -> Result<Invocation> {
    match arguments {
        [repository, spawn, branch, worktree] => Ok(Invocation::ListPane(View {
            repository: repository.clone(),
            spawn: spawn.clone(),
            branch: branch.clone(),
            worktree: worktree.clone(),
        })),
        wrong => Err(Error::new(format!(
            "{LIST_PANE_FLAG} takes a repository, a spawn, a branch and a worktree, \
             but got {} arguments",
            wrong.len()
        ))),
    }
}

/// Read the value of an option, or say which option was left dangling.
fn value_for(flag: &str, value: Option<String>) -> Result<String> {
    value.ok_or_else(|| Error::new(format!("`{flag}` needs a value")))
}

/// Resolve what the user asked for against what the harness offers.
///
/// Left unsaid, it is whatever the harness would pick — the app has no opinion
/// about which model or how much effort, and inventing one here is exactly the
/// leak the seam exists to prevent.
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

    fn spawn(arguments: &[&str]) -> Request {
        match parse_arguments(arguments).unwrap() {
            Invocation::Spawn(request) => request,
            other => panic!("expected a spawn, got {other:?}"),
        }
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

        assert_eq!(request.model, harness::default_model().id);
        assert_eq!(request.effort, harness::default_effort_level().id);
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

        assert_eq!(request.model, "haiku");
        assert_eq!(request.effort, "max");
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
    }

    #[test]
    fn too_little_to_go_on_is_refused() {
        assert!(parse_arguments(&[]).is_err());
        assert!(parse_arguments(&["/code/project"]).is_err());
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
    fn the_list_pane_reads_back_what_the_app_wrote() {
        let view = View {
            repository: "project".to_string(),
            spawn: "add-retry-logic-a7f3".to_string(),
            branch: "spawn/add-retry-logic-a7f3".to_string(),
            worktree: "/data/worktrees/add-retry-logic-a7f3".to_string(),
        };
        let command = list_pane_command(&view).unwrap();

        let arguments: Vec<String> = std::iter::once("harness-launcher".to_string())
            .chain(command.into_iter().skip(1))
            .collect();
        assert_eq!(parse(arguments).unwrap(), Invocation::ListPane(view));
    }
}
