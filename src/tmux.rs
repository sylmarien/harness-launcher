//! tmux, as a process supervisor: one detached session on the app's own
//! socket, one window per spawn, so spawned processes outlive the app.
//!
//! Output leaves via the control-mode client in [`crate::control`]; this
//! module is commands and facts only. A recipe's arguments are passed one per
//! element, so no shell ever sees them.
//! See docs/developers/the-tmux-session.md.

use std::collections::HashMap;

use crate::error::Result;
use crate::harness::LaunchRecipe;
use crate::process::{self, path_argument};
use crate::screen::Size;

/// The socket the app's own server listens on, so its session never appears
/// among the user's own.
const SOCKET: &str = "harness-launcher";

/// The one session every spawn is a window of. Public because the reports
/// name it, so the user knows what to attach to.
pub const SESSION: &str = "spawns";

/// tmux's name for "leave a pane behind when what ran in it stops".
const REMAIN_ON_EXIT: &str = "remain-on-exit";

/// The name of the window that keeps the session alive, so reports can tell
/// the furniture from the spawns.
const HOLDING: &str = "holding";

/// What tmux prints in `#{pane_dead}` for a pane kept by `remain-on-exit`.
/// Written down once; two parsers turn on it.
const DEAD: &str = "1";

/// The live pane in `captured/tmux-list-panes.txt`, read off line 4 of that
/// recording.
#[cfg(test)]
pub(crate) const ALIVE_PANE: &str = "%3";
/// The process holding [`ALIVE_PANE`], from the same line of the recording.
#[cfg(test)]
pub(crate) const ALIVE_PANE_PID: u32 = 14634;

/// Keeps the session alive when no spawn is running, and is what a spawn's
/// window is born running — so the control client is listening to the pane
/// before the harness writes its first byte, which control mode requires.
const HOLDER: [&str; 3] = ["sh", "-c", "while :; do sleep 3600; done"];

/// What one tick asks about every pane, in one call. `pane_dead` is only
/// meaningful because `remain-on-exit` is on; without it a stopped pane
/// disappears.
const PANE_FORMAT: &str = "#{pane_id} #{pane_dead} #{pane_pid} #{pane_tty}";

/// What both reports ask about every window: the spawn's name, the pane to
/// adopt or close, and whether what ran in it has stopped.
const WINDOW_FORMAT: &str = "#{window_name} #{pane_id} #{pane_dead}";

/// One spawn the session is holding, as tmux reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// The spawn's name, which the window carries.
    pub name: String,
    /// The pane it runs in.
    pub pane: String,
    /// Whether what ran in it has stopped.
    pub dead: bool,
}

/// One pane, as tmux reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    /// Whether what ran in it has stopped and left the pane behind.
    pub dead: bool,
    /// The process tmux started in it.
    pub pid: u32,
    /// The terminal it holds. Only valid while the pane is alive: a dead
    /// pane's terminal is handed to the next pane, so never probe a dead pane
    /// by tty.
    pub tty: String,
}

/// Every pane the server had, at one moment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Panes {
    panes: HashMap<String, Pane>,
}

impl Panes {
    /// Read what one `list-panes` printed. A line that does not parse is
    /// dropped, making its pane absent.
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
                        dead: dead == DEAD,
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

/// A tmux server, and everything the app asks of one. Cloning is free: this
/// is a socket name, never a connection.
#[derive(Clone)]
pub struct Server {
    /// The socket to talk over.
    socket: String,
}

impl Server {
    /// The app's own server.
    pub fn app() -> Self {
        Self::on_socket(SOCKET)
    }

    /// A server on a named socket — the app's, or a test's own.
    fn on_socket(socket: &str) -> Self {
        Self {
            socket: socket.to_string(),
        }
    }

    /// The socket this server is reached on, for the control client's own
    /// `tmux`.
    pub fn socket(&self) -> &str {
        &self.socket
    }

    /// The session every spawn lives in, made detached if it is not there
    /// yet. Sized to the slot, which later windows inherit. `remain-on-exit`
    /// goes on globally: the server belongs to the app alone, so there is no
    /// user setting to preserve.
    pub fn session(&self, size: Size) -> Result<String> {
        if !process::run("tmux", &self.with_socket(&["has-session", "-t", SESSION]))?.ok {
            self.run(&held(vec![
                "new-session".to_string(),
                "-d".to_string(),
                "-s".to_string(),
                SESSION.to_string(),
                "-n".to_string(),
                HOLDING.to_string(),
                "-x".to_string(),
                size.columns.to_string(),
                "-y".to_string(),
                size.rows.to_string(),
            ]))?;
            self.run(&["set-option", "-g", "-w", REMAIN_ON_EXIT, "on"])?;
        }

        Ok(SESSION.to_string())
    }

    /// Open a window running only the holder, returning its pane's id. The
    /// caller can put a grid behind the pane before [`Server::start`].
    pub fn open_window(&self, session: &str, name: &str) -> Result<String> {
        self.run(&held(vec![
            "new-window".to_string(),
            "-d".to_string(),
            "-t".to_string(),
            session.to_string(),
            "-n".to_string(),
            name.to_string(),
            "-P".to_string(),
            "-F".to_string(),
            "#{pane_id}".to_string(),
        ]))
    }

    /// Start the harness in a pane the holder is keeping warm.
    pub fn start(&self, pane: &str, recipe: &LaunchRecipe) -> Result<()> {
        self.run(&respawn_arguments(
            pane,
            path_argument(&recipe.cwd)?,
            recipe,
        ))?;

        Ok(())
    }

    /// Take a pane away, with its window: the backstop for a session that
    /// will not stop, and the tidy-up `remain-on-exit` requires — a pane
    /// nobody closes stays in every listing.
    pub fn close(&self, pane: &str) -> Result<()> {
        self.run(&["kill-pane", "-t", pane])?;

        Ok(())
    }

    /// Every spawn the session holds, stopped ones included, or `None` when
    /// there is no session at all.
    ///
    /// `has-session` is checked first so asking never creates the session it
    /// asks about; no server is an answer, not a refusal. Scoped with `-s` to
    /// the app's own session.
    pub fn windows(&self) -> Result<Option<Vec<Window>>> {
        if !process::run("tmux", &self.with_socket(&["has-session", "-t", SESSION]))?.ok {
            return Ok(None);
        }

        Ok(Some(windows_in(&self.run(&[
            "list-panes",
            "-s",
            "-t",
            SESSION,
            "-F",
            WINDOW_FORMAT,
        ])?)))
    }

    /// Every pane on the server, in one call — the whole of a tick's
    /// subprocess cost.
    pub fn panes(&self) -> Result<Panes> {
        Ok(Panes::parse(&self.run(&[
            "list-panes",
            "-a",
            "-F",
            PANE_FORMAT,
        ])?))
    }

    /// Ask tmux something, on this server.
    fn run<A: AsRef<str>>(&self, arguments: &[A]) -> Result<String> {
        process::run_ok("tmux", &self.with_socket(arguments))
    }

    /// The arguments, addressed to this server rather than whichever one tmux
    /// would pick.
    fn with_socket<'a, A: AsRef<str>>(&'a self, arguments: &'a [A]) -> Vec<&'a str> {
        let mut all = vec!["-L", self.socket.as_str()];
        all.extend(arguments.iter().map(AsRef::as_ref));

        all
    }
}

/// Which spawns a window listing holds. The holding window is furniture and
/// is dropped; a line that does not parse is dropped too, as in
/// [`Panes::parse`].
fn windows_in(listing: &str) -> Vec<Window> {
    listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?;
            let pane = fields.next()?;
            let dead = fields.next()?;
            if name == HOLDING {
                return None;
            }

            Some(Window {
                name: name.to_string(),
                pane: pane.to_string(),
                dead: dead == DEAD,
            })
        })
        .collect()
}

/// A pane-creating command with the holder appended after `--`, so no shell
/// reads it.
fn held(mut arguments: Vec<String>) -> Vec<String> {
    arguments.push("--".to_string());
    arguments.extend(HOLDER.iter().map(|word| (*word).to_string()));

    arguments
}

/// The `respawn-pane` call that replaces the holder with the harness. `-k`
/// kills only the holder; arguments are passed one at a time, so no shell
/// sees them.
fn respawn_arguments(pane: &str, cwd: &str, recipe: &LaunchRecipe) -> Vec<String> {
    let mut arguments: Vec<String> = ["respawn-pane", "-k", "-t", pane, "-c", cwd]
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
pub(crate) mod tests {
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
    fn the_harness_starts_in_the_pane_the_spawn_was_given() {
        let arguments = respawn_arguments("%3", "/worktrees/add-retry-logic-a7f3", &recipe());

        assert_eq!(value_after(&arguments, "-t"), "%3");
    }

    #[test]
    fn the_harness_starts_in_its_worktree() {
        let arguments = respawn_arguments("%3", "/worktrees/add-retry-logic-a7f3", &recipe());

        assert_eq!(
            value_after(&arguments, "-c"),
            "/worktrees/add-retry-logic-a7f3"
        );
    }

    #[test]
    fn the_recipes_environment_reaches_the_pane() {
        let arguments = respawn_arguments("%3", "/worktrees/x", &recipe());

        assert_eq!(value_after(&arguments, "-e"), "SOME_VARIABLE=1");
    }

    #[test]
    fn the_command_is_passed_one_argument_at_a_time_so_no_shell_sees_it() {
        let arguments = respawn_arguments("%3", "/worktrees/x", &recipe());
        let command = &arguments[arguments.iter().position(|a| a == "--").unwrap() + 1..];

        assert_eq!(command, ["some-harness", "--flag", "do the work"]);
    }

    /// A real `list-panes` from a real tmux — see `captured/README.md`.
    const CAPTURED: &str = include_str!("../captured/tmux-list-panes.txt");

    #[test]
    fn a_live_pane_is_read_with_the_process_and_terminal_it_holds() {
        let panes = Panes::parse(CAPTURED);

        assert_eq!(
            panes.get(ALIVE_PANE),
            Some(&Pane {
                dead: false,
                pid: ALIVE_PANE_PID,
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

    /// A real `list-panes -s` from a real tmux — see `captured/README.md`.
    const CAPTURED_IN_SESSION: &str = include_str!("../captured/tmux-list-panes-in-session.txt");

    #[test]
    fn the_spawns_a_session_is_holding_are_read_off_the_windows_they_are_in() {
        let spawns = windows_in(CAPTURED_IN_SESSION);

        assert_eq!(
            spawns[0],
            Window {
                name: "add-retry-logic-a7f3".to_string(),
                pane: "%1".to_string(),
                dead: false,
            }
        );
        assert_eq!(spawns[1].name, "fix-the-flake-b2c9");
    }

    #[test]
    fn the_window_holding_the_session_open_is_not_one_of_the_spawns() {
        let spawns = windows_in(CAPTURED_IN_SESSION);

        assert!(
            !spawns.iter().any(|window| window.name == HOLDING),
            "{spawns:?}"
        );
    }

    /// A stopped spawn stays in the listing: it is the pane adoption closes.
    #[test]
    fn a_spawn_that_has_stopped_is_listed_as_stopped_rather_than_dropped() {
        let spawns = windows_in(CAPTURED_IN_SESSION);

        let stopped = spawns
            .iter()
            .find(|window| window.name == "drop-the-cache-d4e1")
            .expect("a spawn kept by remain-on-exit was dropped from the listing");
        assert!(stopped.dead, "{stopped:?}");
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

    // From here on: a real tmux on a private socket, so nothing here can
    // reach the user's own server. No fake, no abstraction over tmux.

    /// A tmux server that belongs to one test and dies with it.
    pub struct PrivateTmux {
        pub server: Server,
        socket: String,
        worktree: TempDir,
    }

    impl PrivateTmux {
        pub fn start(name: &str) -> Self {
            let socket = format!("harness-launcher-{name}");
            let private = Self {
                server: Server::on_socket(&socket),
                socket,
                worktree: tempdir().unwrap(),
            };
            private.kill();

            private
        }

        /// A harmless stand-in: no real harness runs in a test, because it
        /// costs tokens and needs credentials.
        pub fn recipe(&self, script: &str) -> LaunchRecipe {
            LaunchRecipe {
                program: "sh".to_string(),
                args: vec!["-c".to_string(), script.to_string()],
                env: vec![("PROBE".to_string(), "probe-value".to_string())],
                cwd: self.worktree.path().to_path_buf(),
            }
        }

        pub fn worktree(&self) -> &std::path::Path {
            self.worktree.path()
        }

        pub fn panes(&self, format: &str) -> String {
            self.server
                .run(&["list-panes", "-a", "-F", format])
                .unwrap()
        }

        fn kill(&self) {
            let _ = process::run("tmux", &["-L", &self.socket, "kill-server"]);
        }

        /// Wait for something tmux only knows once a child has got going.
        pub fn until(&self, what: &str, ready: impl Fn(&str) -> bool) -> String {
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

        /// [`Self::until`], but on what a pane has drawn: `capture-pane`
        /// reports the screen *now*, and an unscheduled child has drawn
        /// nothing yet, so a bare capture would depend on machine load.
        pub fn shown_until(&self, pane: &str, ready: impl Fn(&str) -> bool) -> String {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let seen = self
                    .server
                    .run(&["capture-pane", "-p", "-t", pane])
                    .unwrap();
                if ready(&seen) {
                    return seen;
                }
                assert!(
                    Instant::now() < deadline,
                    "gave up waiting; the pane showed {seen:?}"
                );
                sleep(Duration::from_millis(25));
            }
        }
    }

    impl Drop for PrivateTmux {
        fn drop(&mut self) {
            self.kill();
        }
    }

    /// The shape of a slot, for the tests that care what a pane was born as.
    const SLOT: Size = Size {
        columns: 61,
        rows: 17,
    };

    #[test]
    fn the_session_is_detached_and_stays_that_way() {
        let tmux = PrivateTmux::start("session-is-detached");

        let session = tmux.server.session(SLOT).unwrap();

        let sessions = tmux
            .server
            .run(&[
                "list-sessions",
                "-F",
                "#{session_name} attached=#{session_attached}",
            ])
            .unwrap();
        assert_eq!(sessions, format!("{session} attached=0"));
    }

    #[test]
    fn asking_for_the_session_twice_does_not_make_a_second_one() {
        let tmux = PrivateTmux::start("session-is-made-once");
        tmux.server.session(SLOT).unwrap();

        tmux.server.session(SLOT).unwrap();

        assert_eq!(
            tmux.server.run(&["list-sessions"]).unwrap().lines().count(),
            1
        );
    }

    #[test]
    fn a_spawns_window_is_born_the_size_of_the_slot() {
        let tmux = PrivateTmux::start("window-is-born-slot-sized");
        let session = tmux.server.session(SLOT).unwrap();

        let pane = tmux
            .server
            .open_window(&session, "add-retry-logic-a7f3")
            .unwrap();

        let sizes = tmux.panes("#{pane_id} #{pane_width}x#{pane_height}");
        assert!(
            sizes.contains(&format!("{pane} {}x{}", SLOT.columns, SLOT.rows)),
            "the pane is not the shape of the slot: {sizes}"
        );
    }

    #[test]
    fn several_spawns_are_several_windows_of_the_one_detached_session() {
        let tmux = PrivateTmux::start("several-spawns-several-windows");
        let session = tmux.server.session(SLOT).unwrap();

        let one = tmux
            .server
            .open_window(&session, "add-retry-logic-a7f3")
            .unwrap();
        let other = tmux
            .server
            .open_window(&session, "fix-the-flake-b2c9")
            .unwrap();

        assert_ne!(one, other, "two spawns were given the same pane");
        let windows = tmux
            .server
            .run(&[
                "list-windows",
                "-t",
                &session,
                "-F",
                "#{window_name} panes=#{window_panes}",
            ])
            .unwrap();
        assert!(
            windows.contains("add-retry-logic-a7f3 panes=1"),
            "{windows}"
        );
        assert!(windows.contains("fix-the-flake-b2c9 panes=1"), "{windows}");
        let sessions = tmux
            .server
            .run(&["list-sessions", "-F", "#{session_name}"])
            .unwrap();
        assert_eq!(
            sessions, session,
            "a second session appeared to hold the spawns that are not in the slot"
        );
    }

    #[test]
    fn a_machine_holding_nothing_has_no_session_to_report() {
        let tmux = PrivateTmux::start("nothing-is-held");

        assert_eq!(tmux.server.windows().unwrap(), None);
    }

    #[test]
    fn a_session_holding_spawns_names_each_one_with_the_pane_it_is_in() {
        let tmux = PrivateTmux::start("reports-what-is-running");
        let session = tmux.server.session(SLOT).unwrap();
        let going = tmux
            .server
            .open_window(&session, "add-retry-logic-a7f3")
            .unwrap();
        tmux.server
            .start(&going, &tmux.recipe("sleep 120"))
            .unwrap();
        let stopped = tmux
            .server
            .open_window(&session, "drop-the-cache-d4e1")
            .unwrap();
        tmux.server.start(&stopped, &tmux.recipe("exit 3")).unwrap();
        tmux.until("#{pane_dead}", |seen| seen.contains('1'));

        let windows = tmux.server.windows().unwrap().expect("a session to report");

        assert_eq!(
            windows,
            [
                Window {
                    name: "add-retry-logic-a7f3".to_string(),
                    pane: going,
                    dead: false,
                },
                Window {
                    name: "drop-the-cache-d4e1".to_string(),
                    pane: stopped,
                    dead: true,
                },
            ]
        );
    }

    #[test]
    fn asking_what_the_session_holds_leaves_it_exactly_as_it_was() {
        let tmux = PrivateTmux::start("asking-changes-nothing");
        let shape = "#{window_name} #{pane_id} #{pane_dead}";

        assert_eq!(tmux.server.windows().unwrap(), None);
        assert_eq!(
            tmux.server.windows().unwrap(),
            None,
            "asking what the session held is what brought it into existence"
        );

        let session = tmux.server.session(SLOT).unwrap();
        let pane = tmux
            .server
            .open_window(&session, "add-retry-logic-a7f3")
            .unwrap();
        tmux.server.start(&pane, &tmux.recipe("sleep 120")).unwrap();
        let before = tmux.panes(shape);

        tmux.server.windows().unwrap();

        assert_eq!(
            tmux.panes(shape),
            before,
            "asking what the session held changed what is running"
        );
    }

    /// A command that lost its `-L` would find whichever server `$TMUX`
    /// names, which on a spawn's window is the user's own.
    #[test]
    fn every_command_is_addressed_to_a_server_of_the_apps_own() {
        let app = Server::app();

        assert_eq!(app.socket(), SOCKET);
        assert_eq!(
            app.with_socket(&["list-panes", "-a"]),
            ["-L", SOCKET, "list-panes", "-a"],
            "a command went out without saying which server it was for"
        );
        assert_eq!(
            app.with_socket::<String>(&[]),
            ["-L", SOCKET],
            "the socket is not the first thing every command says"
        );
        assert_eq!(
            app.with_socket(&["has-session", "-t", SESSION]),
            ["-L", SOCKET, "has-session", "-t", SESSION],
            "the report asked the user's own server what it was holding"
        );
        assert_eq!(
            app.with_socket(&["list-panes", "-s", "-t", SESSION, "-F", WINDOW_FORMAT]),
            [
                "-L",
                SOCKET,
                "list-panes",
                "-s",
                "-t",
                SESSION,
                "-F",
                WINDOW_FORMAT
            ],
            "the report read the user's own server's windows"
        );
    }

    #[test]
    fn a_spawn_runs_the_harness_where_and_how_the_recipe_said() {
        let tmux = PrivateTmux::start("spawn-runs-the-recipe");
        let session = tmux.server.session(SLOT).unwrap();
        let pane = tmux.server.open_window(&session, "spawn").unwrap();

        tmux.server
            .start(&pane, &tmux.recipe("printenv PROBE; sleep 120"))
            .unwrap();

        let shown = tmux.until("#{pane_id} #{pane_current_path}", |seen| {
            seen.contains(tmux.worktree().to_str().unwrap())
        });
        assert!(
            shown.contains(&format!("{pane} ")),
            "the spawn did not start in the worktree: {shown}"
        );
        // Waited for: `printenv` may not have been scheduled yet on a loaded
        // machine.
        let printed = tmux.shown_until(&pane, |seen| seen.contains("probe-value"));
        assert!(
            printed.contains("probe-value"),
            "the recipe's environment did not reach the pane: {printed:?}"
        );
    }

    #[test]
    fn a_session_that_stops_at_once_still_leaves_its_pane_behind() {
        let tmux = PrivateTmux::start("stopping-leaves-the-pane");
        let session = tmux.server.session(SLOT).unwrap();
        let pane = tmux.server.open_window(&session, "spawn").unwrap();

        tmux.server.start(&pane, &tmux.recipe("exit 3")).unwrap();

        let panes = tmux.until("#{pane_id} dead=#{pane_dead}", |seen| {
            seen.contains("dead=1")
        });
        assert!(
            panes.contains(&format!("{pane} dead=1")),
            "the spawn was reaped instead of kept: {panes}"
        );
    }

    #[test]
    fn the_last_spawn_stopping_does_not_take_the_session_with_it() {
        let tmux = PrivateTmux::start("session-outlives-its-spawns");
        let session = tmux.server.session(SLOT).unwrap();
        let pane = tmux.server.open_window(&session, "spawn").unwrap();
        tmux.server.start(&pane, &tmux.recipe("exit 3")).unwrap();
        tmux.until("#{pane_dead}", |seen| seen.contains('1'));

        assert!(
            tmux.server.run(&["has-session", "-t", &session]).is_ok(),
            "the session went with the spawn that stopped"
        );
    }

    #[test]
    fn closing_a_pane_takes_it_off_the_server_whether_or_not_it_was_still_running() {
        let tmux = PrivateTmux::start("closing-takes-the-pane-away");
        let session = tmux.server.session(SLOT).unwrap();
        let running = tmux.server.open_window(&session, "running").unwrap();
        tmux.server
            .start(&running, &tmux.recipe("sleep 120"))
            .unwrap();
        let stopped = tmux.server.open_window(&session, "stopped").unwrap();
        tmux.server.start(&stopped, &tmux.recipe("exit 3")).unwrap();
        tmux.until("#{pane_dead}", |seen| seen.contains('1'));

        tmux.server.close(&running).unwrap();
        tmux.server.close(&stopped).unwrap();

        let panes = tmux.server.panes().unwrap();
        assert_eq!(panes.get(&running), None, "the live pane is still listed");
        assert_eq!(panes.get(&stopped), None, "the dead pane is still listed");
    }

    #[test]
    fn one_call_reads_the_liveness_of_every_pane_there_is() {
        let tmux = PrivateTmux::start("one-call-reads-every-pane");
        let session = tmux.server.session(SLOT).unwrap();
        let alive = tmux.server.open_window(&session, "alive").unwrap();
        tmux.server
            .start(&alive, &tmux.recipe("sleep 120"))
            .unwrap();
        let stopped = tmux.server.open_window(&session, "stopped").unwrap();
        tmux.server.start(&stopped, &tmux.recipe("exit 3")).unwrap();
        tmux.until("#{pane_dead}", |seen| seen.contains('1'));

        let panes = tmux.server.panes().unwrap();

        let alive = panes.get(&alive).expect("the live spawn was not listed");
        assert!(!alive.dead);
        assert!(alive.pid > 0, "no process id for a live pane: {alive:?}");
        assert!(alive.tty.starts_with("/dev/"), "{alive:?}");
        assert!(
            panes
                .get(&stopped)
                .expect("the stopped spawn was not listed")
                .dead,
            "the stopped spawn did not read as dead"
        );
    }
}
