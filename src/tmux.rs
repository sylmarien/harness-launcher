//! The window the app composes.
//!
//! The app drives tmux rather than embedding terminals. The multiplexer owns the
//! pty, so no terminal emulation is written here: resize, mouse, scrollback,
//! alternate screen and colour are its problem, and it solves them properly.
//!
//! What arrives from the harness is a recipe — a program, its arguments, its
//! environment, its working directory. Nothing in this module knows what that
//! program is, and the arguments are passed one per element, so no shell ever
//! sees them.

use std::env;

use crate::error::{Error, Result};
use crate::harness::LaunchRecipe;
use crate::process::{self, path_argument};

/// How much of the window the slot takes.
///
/// A share rather than a size: the window is whatever the terminal is, and a
/// maximised terminal must not be a bigger frame around the same small layout.
const SLOT_SHARE: &str = "66%";

/// tmux's name for "leave a pane behind when what ran in it stops".
const REMAIN_ON_EXIT: &str = "remain-on-exit";

/// Whether the app was started from inside tmux.
///
/// This is the whole of mode detection: inside, the app takes over the current
/// window; outside, it starts a session of its own.
pub fn inside_session() -> bool {
    env::var_os("TMUX").is_some_and(|value| !value.is_empty())
}

/// The pane the app is running in.
pub fn current_pane() -> Result<String> {
    env::var("TMUX_PANE").map_err(|_| {
        Error::new("$TMUX_PANE is not set, so the app cannot tell which pane it is in")
    })
}

/// A tmux server, and everything the app asks of one.
pub struct Server {
    /// The socket to talk over, or none for whichever server `tmux` finds by
    /// itself — which inside a session is the one the user is already in.
    socket: Option<String>,
}

impl Server {
    /// The server the terminal already belongs to.
    pub fn inherited() -> Self {
        Self { socket: None }
    }

    /// A server of its own, so a test never touches the user's.
    #[cfg(test)]
    fn on_socket(socket: &str) -> Self {
        Self {
            socket: Some(socket.to_string()),
        }
    }

    /// Split a pane and start the session beside it. Returns the new pane's id.
    ///
    /// The slot keeps `remain-on-exit`, so a session that stops leaves its last
    /// screen behind rather than vanishing — the app never reads a pane's
    /// output, so that screen is the only account of what went wrong with it.
    ///
    /// The option goes on the *window* first and is pinned to the pane
    /// afterwards, because a session that dies the instant it starts — bad
    /// credentials, a rejected flag — would be reaped before a second call could
    /// set anything, losing exactly the screen the option exists to keep. The
    /// window is then put back as it was found, which matters when the window is
    /// one the app took over from the user.
    pub fn open_slot(&self, target: &str, recipe: &LaunchRecipe) -> Result<String> {
        let previously = self.window_option(target, REMAIN_ON_EXIT)?;
        self.run(&["set-option", "-w", "-t", target, REMAIN_ON_EXIT, "on"])?;

        let slot = self.split(target, recipe);
        let restored = self.restore_window_option(target, REMAIN_ON_EXIT, previously);

        let slot = slot?;
        restored?;

        Ok(slot)
    }

    /// Put the keyboard on a pane.
    pub fn select_pane(&self, pane: &str) -> Result<()> {
        self.run(&["select-pane", "-t", pane])?;

        Ok(())
    }

    /// Build the window from nothing and hand the terminal over to it.
    pub fn open_window(
        &self,
        session: &str,
        list_pane: &[String],
        recipe: &LaunchRecipe,
        size: Option<(u16, u16)>,
    ) -> Result<()> {
        self.build_window(session, list_pane, recipe, size)?;

        process::hand_over(
            "tmux",
            &self.with_socket(&["attach-session", "-t", session]),
        )
    }

    /// A session holding the list pane and the slot, detached. Returns the slot.
    ///
    /// The session is created at the terminal's real size where that is known,
    /// so the layout the user first sees is the one their terminal deserves
    /// rather than tmux's 80×24 default stretched afterwards.
    fn build_window(
        &self,
        session: &str,
        list_pane: &[String],
        recipe: &LaunchRecipe,
        size: Option<(u16, u16)>,
    ) -> Result<String> {
        self.run(&session_arguments(session, list_pane, size))?;

        let slot = self.open_slot(&format!("{session}:"), recipe)?;
        self.select_pane(&slot)?;

        Ok(slot)
    }

    /// Create the slot's pane, and pin the option it was born with to it.
    fn split(&self, target: &str, recipe: &LaunchRecipe) -> Result<String> {
        let pane = self.run(&split_arguments(
            target,
            path_argument(&recipe.cwd)?,
            recipe,
        ))?;
        self.run(&["set-option", "-p", "-t", &pane, REMAIN_ON_EXIT, "on"])?;

        Ok(pane)
    }

    /// What a window option is set to on the window itself, if anything.
    fn window_option(&self, target: &str, name: &str) -> Result<Option<String>> {
        let shown = self.run(&["show-options", "-w", "-t", target, name])?;

        Ok(shown
            .strip_prefix(name)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()))
    }

    /// Put a window option back the way it was — including back to unset.
    fn restore_window_option(
        &self,
        target: &str,
        name: &str,
        previously: Option<String>,
    ) -> Result<()> {
        match previously {
            Some(value) => self.run(&["set-option", "-w", "-t", target, name, &value])?,
            None => self.run(&["set-option", "-w", "-t", target, "-u", name])?,
        };

        Ok(())
    }

    /// Ask tmux something, on this server.
    fn run<A: AsRef<str>>(&self, arguments: &[A]) -> Result<String> {
        process::run_ok("tmux", &self.with_socket(arguments))
    }

    /// The arguments, addressed to this server rather than whichever one tmux
    /// would pick.
    fn with_socket<'a, A: AsRef<str>>(&'a self, arguments: &'a [A]) -> Vec<&'a str> {
        let mut all = Vec::new();
        if let Some(socket) = &self.socket {
            all.push("-L");
            all.push(socket.as_str());
        }
        all.extend(arguments.iter().map(AsRef::as_ref));

        all
    }
}

/// The `new-session` call that creates the window and its list pane.
fn session_arguments(session: &str, list_pane: &[String], size: Option<(u16, u16)>) -> Vec<String> {
    let mut arguments = vec![
        "new-session".to_string(),
        "-d".to_string(),
        "-s".to_string(),
        session.to_string(),
    ];

    if let Some((columns, rows)) = size {
        arguments.push("-x".to_string());
        arguments.push(columns.to_string());
        arguments.push("-y".to_string());
        arguments.push(rows.to_string());
    }

    arguments.push("--".to_string());
    arguments.extend_from_slice(list_pane);

    arguments
}

/// The `split-window` call that creates the slot.
fn split_arguments(target: &str, cwd: &str, recipe: &LaunchRecipe) -> Vec<String> {
    let mut arguments: Vec<String> = [
        "split-window",
        "-h",
        "-l",
        SLOT_SHARE,
        "-t",
        target,
        "-c",
        cwd,
        "-P",
        "-F",
        "#{pane_id}",
    ]
    .iter()
    .map(|argument| (*argument).to_string())
    .collect();

    for (name, value) in &recipe.env {
        arguments.push("-e".to_string());
        arguments.push(format!("{name}={value}"));
    }

    arguments.push("--".to_string());
    arguments.push(recipe.program.clone());
    arguments.extend(recipe.args.iter().cloned());

    arguments
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::thread::sleep;
    use std::time::{Duration, Instant};
    use tempfile::{TempDir, tempdir};

    fn recipe() -> LaunchRecipe {
        LaunchRecipe {
            program: "some-harness".to_string(),
            args: vec!["--flag".to_string(), "do the work".to_string()],
            env: vec![("SOME_VARIABLE".to_string(), "1".to_string())],
            cwd: PathBuf::from("/worktrees/add-retry-logic-a7f3"),
        }
    }

    fn list_pane() -> Vec<String> {
        ["/usr/local/bin/harness-launcher", "--list-pane", "project"]
            .iter()
            .map(|argument| (*argument).to_string())
            .collect()
    }

    /// The argument after `flag`, which is how tmux options are written.
    fn value_after(arguments: &[String], flag: &str) -> String {
        let flag = arguments
            .iter()
            .position(|argument| argument == flag)
            .unwrap_or_else(|| panic!("no {flag} in {arguments:?}"));

        arguments[flag + 1].clone()
    }

    #[test]
    fn the_slot_starts_the_harness_in_its_worktree() {
        let arguments = split_arguments("%3", "/worktrees/add-retry-logic-a7f3", &recipe());

        assert_eq!(
            value_after(&arguments, "-c"),
            "/worktrees/add-retry-logic-a7f3"
        );
    }

    #[test]
    fn the_recipes_environment_reaches_the_pane() {
        let arguments = split_arguments("%3", "/worktrees/x", &recipe());

        assert_eq!(value_after(&arguments, "-e"), "SOME_VARIABLE=1");
    }

    #[test]
    fn the_command_is_passed_one_argument_at_a_time_so_no_shell_sees_it() {
        let arguments = split_arguments("%3", "/worktrees/x", &recipe());
        let command = &arguments[arguments.iter().position(|a| a == "--").unwrap() + 1..];

        assert_eq!(command, ["some-harness", "--flag", "do the work"]);
    }

    #[test]
    fn the_slot_is_sized_as_a_share_of_the_window() {
        let arguments = split_arguments("%3", "/worktrees/x", &recipe());

        assert_eq!(value_after(&arguments, "-l"), SLOT_SHARE);
        assert!(
            SLOT_SHARE.ends_with('%'),
            "the slot must not be a fixed size"
        );
    }

    #[test]
    fn the_slot_is_split_off_the_pane_it_is_told_to_split() {
        let arguments = split_arguments("%3", "/worktrees/x", &recipe());

        assert_eq!(value_after(&arguments, "-t"), "%3");
    }

    #[test]
    fn a_new_window_is_born_the_size_of_the_terminal_it_replaces() {
        let arguments = session_arguments("some-session", &list_pane(), Some((130, 40)));

        assert_eq!(value_after(&arguments, "-x"), "130");
        assert_eq!(value_after(&arguments, "-y"), "40");
    }

    #[test]
    fn a_terminal_that_will_not_say_how_big_it_is_leaves_the_sizing_to_tmux() {
        let arguments = session_arguments("some-session", &list_pane(), None);

        assert!(!arguments.contains(&"-x".to_string()), "{arguments:?}");
        assert!(!arguments.contains(&"-y".to_string()), "{arguments:?}");
    }

    #[test]
    fn the_new_window_runs_the_list_pane_in_it() {
        let arguments = session_arguments("some-session", &list_pane(), Some((130, 40)));
        let command = &arguments[arguments.iter().position(|a| a == "--").unwrap() + 1..];

        assert_eq!(command, list_pane());
    }

    // The rest is the real thing: a real tmux, on a socket of this test's own,
    // so nothing here can reach the server the user is sitting in front of.
    // There is no fake and no abstraction over tmux — the real one is cheap and
    // hermetic enough not to need either.

    /// A tmux server that belongs to one test and dies with it.
    struct PrivateTmux {
        server: Server,
        socket: String,
        worktree: TempDir,
    }

    impl PrivateTmux {
        fn start(name: &str) -> Self {
            let socket = format!("harness-launcher-{name}");
            let private = Self {
                server: Server::on_socket(&socket),
                socket,
                worktree: tempdir().unwrap(),
            };
            private.kill();

            private
        }

        /// A window with one pane in it, minding its own business.
        fn window(&self, session: &str) -> String {
            self.server
                .run(&session_arguments(session, &sleeping(), Some((120, 30))))
                .unwrap();

            format!("{session}:")
        }

        /// A recipe for a harmless stand-in: no harness is ever really started
        /// in a test, because the real one costs tokens and needs credentials.
        fn recipe(&self, script: &str) -> LaunchRecipe {
            LaunchRecipe {
                program: "sh".to_string(),
                args: vec!["-c".to_string(), script.to_string()],
                env: vec![("PROBE".to_string(), "probe-value".to_string())],
                cwd: self.worktree.path().to_path_buf(),
            }
        }

        fn panes(&self, format: &str) -> String {
            self.server
                .run(&["list-panes", "-a", "-F", format])
                .unwrap()
        }

        fn kill(&self) {
            let _ = process::run("tmux", &["-L", &self.socket, "kill-server"]);
        }

        /// Wait for something tmux only knows once a child has got going.
        fn until(&self, what: &str, ready: impl Fn(&str) -> bool) -> String {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let seen = self.panes(what);
                if ready(&seen) {
                    return seen;
                }
                assert!(Instant::now() < deadline, "gave up waiting; saw {seen:?}");
                sleep(Duration::from_millis(25));
            }
        }
    }

    impl Drop for PrivateTmux {
        fn drop(&mut self) {
            self.kill();
        }
    }

    /// A pane's worth of doing nothing.
    fn sleeping() -> Vec<String> {
        ["sh", "-c", "sleep 120"]
            .iter()
            .map(|argument| (*argument).to_string())
            .collect()
    }

    #[test]
    fn a_slot_runs_the_harness_where_and_how_the_recipe_said() {
        let tmux = PrivateTmux::start("slot-runs-the-recipe");
        let window = tmux.window("work");

        let slot = tmux
            .server
            .open_slot(&window, &tmux.recipe("printenv PROBE; sleep 120"))
            .unwrap();

        let shown = tmux.until("#{pane_id} #{pane_current_path}", |seen| {
            seen.contains(&format!("{slot} "))
        });
        assert!(
            shown.contains(tmux.worktree.path().to_str().unwrap()),
            "the slot did not start in the worktree: {shown}"
        );
        let printed = tmux
            .server
            .run(&["capture-pane", "-p", "-t", &slot])
            .unwrap();
        assert!(
            printed.contains("probe-value"),
            "the recipe's environment did not reach the pane: {printed:?}"
        );
    }

    #[test]
    fn a_session_that_stops_at_once_still_leaves_its_pane_behind() {
        let tmux = PrivateTmux::start("stopping-leaves-the-pane");
        let window = tmux.window("work");

        let slot = tmux
            .server
            .open_slot(&window, &tmux.recipe("exit 3"))
            .unwrap();

        let panes = tmux.until("#{pane_id} dead=#{pane_dead}", |seen| {
            seen.contains("dead=1")
        });
        assert!(
            panes.contains(&format!("{slot} dead=1")),
            "the slot was reaped instead of kept: {panes}"
        );
    }

    #[test]
    fn the_window_is_left_as_the_app_found_it() {
        let tmux = PrivateTmux::start("window-left-as-found");
        let window = tmux.window("work");

        let slot = tmux
            .server
            .open_slot(&window, &tmux.recipe("sleep 120"))
            .unwrap();

        assert_eq!(
            tmux.server.window_option(&window, REMAIN_ON_EXIT).unwrap(),
            None,
            "the app left its own setting on the user's window"
        );
        let pinned = tmux
            .server
            .run(&["show-options", "-p", "-t", &slot, REMAIN_ON_EXIT])
            .unwrap();
        assert_eq!(pinned, "remain-on-exit on");
    }

    #[test]
    fn a_window_that_already_had_a_setting_keeps_it() {
        let tmux = PrivateTmux::start("window-keeps-its-setting");
        let window = tmux.window("work");
        tmux.server
            .run(&["set-option", "-w", "-t", &window, REMAIN_ON_EXIT, "off"])
            .unwrap();

        tmux.server
            .open_slot(&window, &tmux.recipe("sleep 120"))
            .unwrap();

        assert_eq!(
            tmux.server.window_option(&window, REMAIN_ON_EXIT).unwrap(),
            Some("off".to_string())
        );
    }

    #[test]
    fn a_window_built_from_nothing_has_the_list_on_the_left_and_the_slot_on_the_right() {
        let tmux = PrivateTmux::start("window-from-nothing");

        let slot = tmux
            .server
            .build_window(
                "built",
                &sleeping(),
                &tmux.recipe("sleep 120"),
                Some((120, 30)),
            )
            .unwrap();

        let panes =
            tmux.panes("#{pane_id} left=#{pane_left} width=#{pane_width} active=#{pane_active}");
        let rows: Vec<&str> = panes.lines().collect();
        assert_eq!(rows.len(), 2, "expected a list pane and a slot: {panes}");
        let list = rows.iter().find(|row| !row.starts_with(&slot)).unwrap();
        let slot_row = rows.iter().find(|row| row.starts_with(&slot)).unwrap();
        assert!(
            list.contains("left=0"),
            "the list is not on the left: {panes}"
        );
        assert!(
            !slot_row.contains("left=0"),
            "the slot is not on the right: {panes}"
        );
        assert!(
            slot_row.contains("active=1"),
            "the keyboard was left off the slot: {panes}"
        );
        assert!(
            slot_row.contains("width=79"),
            "the slot did not take its share of 120 columns: {panes}"
        );
    }
}
