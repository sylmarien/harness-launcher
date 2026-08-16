//! The projects saved in the app's configuration file, and matching what was
//! typed against their names.
//!
//! A project is a name and a path, written by hand into one TOML file. The
//! draft form takes the name in place of the path. Having no file at all is
//! the ordinary case. A file that does not parse refuses at start-up.
//! See docs/users/saved-projects.md.

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::xdg;

/// A saved project: what it is called, and the repository the name stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// What the form's repository field takes in place of the path.
    pub name: String,
    /// The repository a spawn is made against.
    pub path: PathBuf,
}

/// The configuration file, as it is read.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    /// One entry per project: the name is the key, the path is the value.
    #[serde(default)]
    projects: BTreeMap<String, PathBuf>,
}

/// Every saved project, in the order their names sort.
pub fn saved() -> Result<Vec<Project>> {
    read(&file_from(
        env::var_os("XDG_CONFIG_HOME"),
        env::var_os("HOME"),
    )?)
}

/// The saved projects whose names match what was typed, best first.
///
/// A name matches when the typed characters appear in it in order, ignoring
/// case, with any number of characters between them. Nothing typed matches
/// nothing, so a blank field suggests no project.
pub fn matching<'a>(projects: &'a [Project], typed: &str) -> Vec<&'a Project> {
    if typed.is_empty() {
        return Vec::new();
    }

    let mut found: Vec<(usize, &Project)> = projects
        .iter()
        .filter_map(|project| Some((last_matched(&project.name, typed)?, project)))
        .collect();
    // Nearest the front of the name first. Names that tie are ordered by name.
    found.sort_by_key(|(last, project)| (*last, project.name.to_lowercase()));

    found.into_iter().map(|(_, project)| project).collect()
}

/// How far into `name` the last typed character falls, or nothing when the
/// typed characters do not appear in it in order. Each character is taken at
/// the first position left to it, so one number ranks the match.
fn last_matched(name: &str, typed: &str) -> Option<usize> {
    let name = name.to_lowercase();
    let mut rest = name.chars();
    let mut at = 0;

    for wanted in typed.to_lowercase().chars() {
        at += rest.by_ref().position(|held| held == wanted)? + 1;
    }

    Some(at - 1)
}

/// Resolve the configuration file from the environment, XDG first.
fn file_from(config_home: Option<OsString>, home: Option<OsString>) -> Result<PathBuf> {
    let base = xdg::under(config_home, home, ".config").ok_or_else(|| {
        Error::new(
            "neither $XDG_CONFIG_HOME nor $HOME is set, so there is nowhere to read the \
             configuration from",
        )
    })?;

    Ok(base.join("config.toml"))
}

/// Read the projects out of a configuration file.
///
/// No file is no projects. A file that does not parse is a refusal, so a typo
/// in a hand-written file is never silently dropped.
fn read(file: &Path) -> Result<Vec<Project>> {
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(missing) if missing.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(unreadable) => {
            return Err(Error::new(format!(
                "could not read {}: {unreadable}",
                file.display()
            )));
        }
    };

    let held: Configuration = toml::from_str(&text).map_err(|mistake| {
        Error::new(format!(
            "{} is not readable as configuration: {mistake}",
            file.display()
        ))
    })?;

    Ok(held
        .projects
        .into_iter()
        .map(|(name, path)| Project { name, path })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The saved projects the fuzzy-matching tests run against. `Zap` and
    /// `bat` are there to tie, in an order only the case fold settles.
    fn some() -> Vec<Project> {
        [
            "Clade",
            "clang-tools",
            "codex-lab",
            "harness-launcher",
            "Zap",
            "bat",
        ]
        .into_iter()
        .map(|name| Project {
            name: name.to_string(),
            path: PathBuf::from("/code").join(name),
        })
        .collect()
    }

    /// The names `typed` matches, in the order they are suggested.
    fn matched(typed: &str) -> Vec<String> {
        matching(&some(), typed)
            .into_iter()
            .map(|project| project.name.clone())
            .collect()
    }

    #[test]
    fn typed_characters_match_a_name_they_appear_in_in_order() {
        assert!(
            matched("cld").contains(&"Clade".to_string()),
            "c, l and d appear in Clade in that order"
        );
    }

    #[test]
    fn typed_characters_that_are_not_in_order_match_nothing() {
        assert!(
            !matched("cled").contains(&"Clade".to_string()),
            "the only e in Clade comes after its d, so c, l, e, d is not in order"
        );
    }

    #[test]
    fn matching_ignores_case_in_both_directions() {
        assert!(matched("CLA").contains(&"Clade".to_string()));
        assert!(matched("Cla").contains(&"Clade".to_string()));
        assert!(matched("har").contains(&"harness-launcher".to_string()));
    }

    #[test]
    fn a_name_typed_whole_matches_itself() {
        assert_eq!(matched("harness-launcher"), ["harness-launcher"]);
    }

    #[test]
    fn nothing_typed_suggests_nothing() {
        assert!(matched("").is_empty());
    }

    #[test]
    fn the_name_the_last_character_lands_nearest_the_front_of_comes_first() {
        // The d is the third character of codex-lab and the fourth of Clade.
        assert_eq!(matched("cd"), ["codex-lab", "Clade"]);
    }

    #[test]
    fn names_that_tie_are_suggested_in_alphabetical_order_ignoring_case() {
        // The a is the third character of both Clade and clang-tools, and the
        // eighth of codex-lab.
        assert_eq!(matched("cla"), ["Clade", "clang-tools", "codex-lab"]);
        // The a is the second character of bat, harness-launcher and Zap, so
        // only their names separate them, compared with the case folded away.
        assert_eq!(
            matched("a"),
            [
                "bat",
                "harness-launcher",
                "Zap",
                "Clade",
                "clang-tools",
                "codex-lab"
            ]
        );
    }

    #[test]
    fn the_file_lands_under_the_configuration_directory() {
        let file = file_from(None, Some("/home/someone".into())).unwrap();

        assert_eq!(
            file,
            PathBuf::from("/home/someone/.config/harness-launcher/config.toml")
        );
    }

    #[test]
    fn with_nowhere_to_read_the_configuration_from_the_app_refuses() {
        assert!(file_from(None, None).is_err());
    }

    /// A configuration file holding this text.
    fn written(text: &str) -> (tempfile::TempDir, PathBuf) {
        let somewhere = tempfile::tempdir().unwrap();
        let file = somewhere.path().join("config.toml");
        fs::write(&file, text).unwrap();

        (somewhere, file)
    }

    #[test]
    fn a_project_is_a_name_and_a_path() {
        let (_somewhere, file) = written(
            "[projects]\n\
             clade = \"/code/clade\"\n\
             harness = \"/code/harness-launcher\"\n",
        );

        let saved = read(&file).unwrap();

        assert_eq!(
            saved,
            [
                Project {
                    name: "clade".to_string(),
                    path: PathBuf::from("/code/clade"),
                },
                Project {
                    name: "harness".to_string(),
                    path: PathBuf::from("/code/harness-launcher"),
                },
            ]
        );
    }

    #[test]
    fn no_configuration_file_is_no_projects_rather_than_a_refusal() {
        let somewhere = tempfile::tempdir().unwrap();

        assert_eq!(read(&somewhere.path().join("config.toml")).unwrap(), []);
    }

    #[test]
    fn a_file_with_no_projects_in_it_is_no_projects() {
        let (_somewhere, file) = written("");

        assert_eq!(read(&file).unwrap(), []);
    }

    #[test]
    fn a_file_that_does_not_parse_is_a_refusal_naming_the_file() {
        let (_somewhere, file) = written("[projects]\nclade = /code/clade\n");

        let refused = read(&file).expect_err("a file with a mistake in it was read anyway");

        let said = refused.to_string();
        assert!(
            said.contains(file.to_str().unwrap()),
            "the refusal does not say which file to fix: {said}"
        );
    }

    #[test]
    fn a_heading_that_is_not_the_one_the_file_takes_is_a_refusal() {
        let (_somewhere, file) = written("[project]\nclade = \"/code/clade\"\n");

        assert!(
            read(&file).is_err(),
            "a mistyped heading was read as a file with no projects in it"
        );
    }
}
