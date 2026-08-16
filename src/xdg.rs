//! Where the app's own directories sit, resolved from the environment.
//!
//! One XDG variable, one fallback under `$HOME`, and one `harness-launcher`
//! directory under whichever of the two answers. An empty variable counts as
//! unset, because a shell that exports nothing sets it to that.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The app's directory under `variable`, or under `home` and `fallback` when
/// the variable says nothing. Nothing when neither says anything, which is for
/// the caller to refuse in its own words.
pub fn under(
    variable: Option<OsString>,
    home: Option<OsString>,
    fallback: &str,
) -> Option<PathBuf> {
    let base = match variable {
        Some(variable) if !variable.is_empty() => PathBuf::from(variable),
        _ => Path::new(&home.filter(|home| !home.is_empty())?).join(fallback),
    };

    Some(base.join("harness-launcher"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variable_is_where_the_directory_goes_when_it_is_set() {
        let under = under(Some("/data".into()), Some("/home/someone".into()), ".local");

        assert_eq!(under, Some(PathBuf::from("/data/harness-launcher")));
    }

    #[test]
    fn without_the_variable_it_falls_back_to_the_home_directory() {
        let under = under(None, Some("/home/someone".into()), ".local");

        assert_eq!(
            under,
            Some(PathBuf::from("/home/someone/.local/harness-launcher"))
        );
    }

    #[test]
    fn an_empty_variable_is_treated_as_unset() {
        let under = under(
            Some(String::new().into()),
            Some("/home/someone".into()),
            ".local",
        );

        assert_eq!(
            under,
            Some(PathBuf::from("/home/someone/.local/harness-launcher"))
        );
    }

    #[test]
    fn with_neither_the_variable_nor_home_set_there_is_nowhere_to_put_it() {
        assert_eq!(under(None, None, ".local"), None);
        assert_eq!(
            under(None, Some(String::new().into()), ".local"),
            None,
            "an empty HOME resolved to a path relative to wherever the app was started"
        );
    }
}
