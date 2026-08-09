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

use std::collections::HashMap;
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

/// What one tick asks about every pane on the server.
///
/// Four fields, one call, however many spawns there are. `pane_dead` is only
/// meaningful because the slot is created with `remain-on-exit`: without it a
/// pane that stops disappears, and death would have to be inferred from absence
/// — which is a different thing, and reported differently.
const PANE_FORMAT: &str = "#{pane_id} #{pane_dead} #{pane_pid} #{pane_tty}";

/// Whether the app was started from inside tmux.
///
/// It has to be: the app takes over the window it is already in, and there is
/// no window to take over otherwise. Started anywhere else, it refuses and says
/// so rather than building a window and moving the user into it.
pub fn inside_session() -> bool {
    env::var_os("TMUX").is_some_and(|value| !value.is_empty())
}

/// The pane the app is running in.
pub fn current_pane() -> Result<String> {
    env::var("TMUX_PANE").map_err(|_| {
        Error::new("$TMUX_PANE is not set, so the app cannot tell which pane it is in")
    })
}

/// One pane, as tmux reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    /// Whether what ran in it has stopped and left the pane behind.
    pub dead: bool,
    /// The process tmux started in it.
    pub pid: u32,
    /// The terminal it holds — only worth reading while it is alive, because a
    /// dead pane's terminal is released and handed to the next pane that asks.
    pub tty: String,
}

/// Every pane the server had, at one moment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Panes {
    panes: HashMap<String, Pane>,
}

impl Panes {
    /// Read what one `list-panes` printed.
    ///
    /// A line that does not parse is dropped rather than guessed at, which
    /// makes the pane it described *absent* — and absence is a thing the app
    /// already has an honest answer for.
    pub fn parse(listing: &str) -> Self {
        let panes = listing
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let id = fields.next()?;
                let dead = fields.next()?;
                let pid = fields.next()?.parse().ok()?;
                let tty = fields.next()?;

                Some((
                    id.to_string(),
                    Pane {
                        dead: dead == "1",
                        pid,
                        tty: tty.to_string(),
                    },
                ))
            })
            .collect();

        Self { panes }
    }

    /// What the server said about one pane, if it had it at all.
    pub fn get(&self, pane: &str) -> Option<&Pane> {
        self.panes.get(pane)
    }
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

    /// Every pane on the server, in one call.
    ///
    /// This is the whole of a tick's subprocess cost: twenty spawns are twenty
    /// rows of one listing, not twenty questions.
    pub fn panes(&self) -> Result<Panes> {
        Ok(Panes::parse(&self.run(&[
            "list-panes",
            "-a",
            "-F",
            PANE_FORMAT,
        ])?))
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

    /// A real `list-panes` from a real tmux — see `captured/README.md`. Its
    /// window is the shape the app composes: a list pane, a live slot, and a
    /// slot whose session stopped and was kept by `remain-on-exit`.
    const CAPTURED: &str = include_str!("../captured/tmux-list-panes.txt");

    #[test]
    fn a_live_pane_is_read_with_the_process_and_terminal_it_holds() {
        let panes = Panes::parse(CAPTURED);

        assert_eq!(
            panes.get("%3"),
            Some(&Pane {
                dead: false,
                pid: 14634,
                tty: "/dev/pts/3".to_string(),
            })
        );
    }

    #[test]
    fn a_pane_whose_session_stopped_is_read_as_dead() {
        let panes = Panes::parse(CAPTURED);

        assert!(panes.get("%2").unwrap().dead);
        assert!(!panes.get("%0").unwrap().dead);
    }

    #[test]
    fn a_pane_the_server_does_not_have_is_simply_absent() {
        let panes = Panes::parse(CAPTURED);

        assert_eq!(panes.get("%99"), None);
    }

    #[test]
    fn a_line_that_makes_no_sense_takes_only_its_own_pane_with_it() {
        let panes = Panes::parse("%0 0 not-a-pid /dev/pts/0\n%1 0 14627 /dev/pts/1\n\n");

        assert_eq!(panes.get("%0"), None);
        assert_eq!(panes.get("%1").unwrap().pid, 14627);
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

        /// A window with one pane in it, minding its own business — which is
        /// what the app finds when it is started: a window somebody else made.
        fn window(&self, session: &str) -> String {
            self.server
                .run(&[
                    "new-session",
                    "-d",
                    "-s",
                    session,
                    "-x",
                    "120",
                    "-y",
                    "30",
                    "--",
                    "sh",
                    "-c",
                    "sleep 120",
                ])
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
    fn the_app_keeps_the_left_and_the_slot_takes_the_right() {
        let tmux = PrivateTmux::start("the-app-keeps-the-left");
        tmux.window("work");
        let here = tmux.panes("#{pane_id}").trim().to_string();

        let slot = tmux
            .server
            .open_slot(&here, &tmux.recipe("sleep 120"))
            .unwrap();
        tmux.server.select_pane(&slot).unwrap();

        let panes =
            tmux.panes("#{pane_id} left=#{pane_left} width=#{pane_width} active=#{pane_active}");
        let rows: Vec<&str> = panes.lines().collect();
        assert_eq!(rows.len(), 2, "expected the app's pane and a slot: {panes}");
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

    #[test]
    fn one_call_reads_the_liveness_of_every_pane_there_is() {
        let tmux = PrivateTmux::start("one-call-reads-every-pane");
        let window = tmux.window("work");
        let alive = tmux
            .server
            .open_slot(&window, &tmux.recipe("sleep 120"))
            .unwrap();
        let stopped = tmux
            .server
            .open_slot(&window, &tmux.recipe("exit 3"))
            .unwrap();
        tmux.until("#{pane_dead}", |seen| seen.contains('1'));

        let panes = tmux.server.panes().unwrap();

        let alive = panes.get(&alive).expect("the live slot was not listed");
        assert!(!alive.dead);
        assert!(alive.pid > 0, "no process id for a live pane: {alive:?}");
        assert!(alive.tty.starts_with("/dev/"), "{alive:?}");
        assert!(
            panes
                .get(&stopped)
                .expect("the stopped slot was not listed")
                .dead,
            "the stopped slot did not read as dead"
        );
    }
}
