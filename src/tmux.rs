//! tmux, as a process supervisor.
//!
//! It draws nothing. One detached session on a socket of the app's own, one
//! window per spawn, one pane per window, and not a single one of them ever
//! attached to a terminal a person is looking at. What the multiplexer is here
//! for is exactly one thing: **a process that outlives the app**. Quitting kills
//! nothing, because nothing the user started belongs to the app's own process
//! tree.
//!
//! Output leaves by the other road — the control-mode client in [`crate::control`],
//! which is attached before anything is started. This module is commands and
//! facts: make a window, start something in it, ask what is still alive.
//!
//! What arrives from the harness is a recipe — a program, its arguments, its
//! environment, its working directory. Nothing in this module knows what that
//! program is, and the arguments are passed one per element, so no shell ever
//! sees them.

use std::collections::HashMap;

use crate::error::Result;
use crate::harness::LaunchRecipe;
use crate::process::{self, path_argument};
use crate::screen::Size;

/// The socket the app's own server listens on.
///
/// A server of the app's own rather than whichever one `tmux` would find: the
/// session here is furniture, and it has no business appearing in the sessions
/// a person switches between. It also makes the app's behaviour identical
/// whether or not it happens to have been started from inside tmux.
const SOCKET: &str = "harness-launcher";

/// The one session every spawn is a window of.
///
/// Public because it is what the app is leaving behind: the reports on the way
/// in and the way out both name it, so that somebody who wants to go and look at
/// what is still running knows what to attach to.
pub const SESSION: &str = "spawns";

/// tmux's name for "leave a pane behind when what ran in it stops".
const REMAIN_ON_EXIT: &str = "remain-on-exit";

/// What the window that keeps the session alive is called.
///
/// Named rather than left to tmux, which would call it after whatever is
/// running in it. Every other window in the session is named after the spawn it
/// holds, so a name of the app's own here is what tells the furniture apart
/// from the spawns — and the reports that say what is still running are built
/// on exactly that distinction. Counting it would say one more spawn than there
/// has ever been, at every start-up and every exit.
const HOLDING: &str = "holding";

/// What tmux prints in `#{pane_dead}` for a pane whose command has exited and
/// which `remain-on-exit` kept.
///
/// **A value in somebody else's format, read in two places.** The listing every
/// tick parses and the survey of what a session is holding both turn on it, and
/// it is the one signal in the app that cannot go stale — so it is written down
/// once rather than sitting as a bare `"1"` in two parsers that would have to be
/// found together to be changed together.
const DEAD: &str = "1";

/// The live pane in `captured/tmux-list-panes.txt`, and the process tmux says is
/// holding it — both read off line 4 of that recording.
///
/// Named beside the parser that reads the recording, the way the recording
/// itself is: these are two more values that came out of a capture rather than
/// out of somebody's head, and a pane id and a pid transcribed by hand into
/// three test modules are three places to find when the capture is taken again.
#[cfg(test)]
pub(crate) const ALIVE_PANE: &str = "%3";
/// The process holding [`ALIVE_PANE`], from the same line of the same recording.
#[cfg(test)]
pub(crate) const ALIVE_PANE_PID: u32 = 14634;

/// What holds a window open until there is something real to run in it.
///
/// Two jobs. It keeps the session alive before the first spawn and after the
/// last one stops — a session with no windows is a session tmux discards, and
/// with it the control client's attachment. And it is what a spawn's window is
/// *born* running, so that the client can be watching the pane before the
/// harness writes its first byte: control mode streams only what is produced
/// while a client is attached, and a pane that draws itself before anyone is
/// listening stays permanently blank.
const HOLDER: [&str; 3] = ["sh", "-c", "while :; do sleep 3600; done"];

/// What one tick asks about every pane on the server.
///
/// Four fields, one call, however many spawns there are. `pane_dead` is only
/// meaningful because the server is left with `remain-on-exit` on: without it a
/// pane that stops disappears, and death would have to be inferred from absence
/// — which is a different thing, and reported differently.
const PANE_FORMAT: &str = "#{pane_id} #{pane_dead} #{pane_pid} #{pane_tty}";

/// What the reports ask about every window of the session the spawns live in.
///
/// The window's name, which is the spawn's name, and whether what ran in it has
/// stopped. Two fields, because a report says what is still running and names
/// it, and needs nothing else to do either.
const WINDOW_FORMAT: &str = "#{window_name} #{pane_dead}";

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

/// A tmux server, and everything the app asks of one.
///
/// **Cloning one is free and means nothing more than a second thing addressing
/// the same socket.** This is a name and a way of writing an argument list,
/// never a connection — the supervisor's thread and the thread that makes
/// spawns each hold one, and there is nothing between them to share.
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

    /// The socket this server is reached on, for anyone spawning their own
    /// `tmux` — which is the control client, and nobody else.
    pub fn socket(&self) -> &str {
        &self.socket
    }

    /// The session every spawn lives in, made if it is not there yet.
    ///
    /// Detached, and never otherwise: `-d` is what keeps it out of the terminal
    /// the user is looking at. The size given here is the slot's, because the
    /// slot is what a spawn draws into; it is also what a window created later
    /// inherits, so the first spawn is the right shape before it starts.
    ///
    /// `remain-on-exit` goes on globally rather than being set and put back
    /// around each window. The server belongs to the app alone, so there is no
    /// user's setting here to preserve — which is the whole of what the old
    /// save-and-restore dance was for.
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

    /// Open a window for a spawn, with nothing of the spawn's in it yet.
    ///
    /// Returns the id of its one pane. Nothing runs there but the holder, which
    /// is the point: the caller has the pane's id before anything draws, and can
    /// put a grid behind it before starting the harness with [`Server::start`].
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

    /// Take a pane away, and the window it is the only pane of.
    ///
    /// Two jobs, both of them retirement's. It is the **backstop** for a
    /// session that will not stop when it is asked, and it is the tidying-up
    /// after one that did: `remain-on-exit` is what makes a stopped spawn
    /// readable rather than absent, and the price of it is that a pane nobody
    /// takes away is a row of every `list-panes` from then on.
    pub fn close(&self, pane: &str) -> Result<()> {
        self.run(&["kill-pane", "-t", pane])?;

        Ok(())
    }

    /// The spawns still running in the one session they are windows of, or
    /// nothing at all if there is no session to ask about.
    ///
    /// **There is no *holding session*** — that was parking, and parking is
    /// gone. What is held is a single *window*, [`HOLDING`], which is furniture
    /// keeping the session alive and is filtered out below.
    ///
    /// **This is the question both reports are**, and it is asked without
    /// making anything: `has-session` first, so that a report taken before the
    /// app has started anything does not itself create the session it is
    /// reporting on. A report that brought its own subject into existence would
    /// be a lie on every first run.
    ///
    /// **Nothing found is `None`, not a refusal.** A machine that has never run
    /// this — or has rebooted since — has no server at all, and `has-session`
    /// says so by exiting non-zero rather than by failing to run. That is an
    /// answer, and the only honest one.
    ///
    /// Scoped with `-s` to the session rather than `-a` across the server, so
    /// what comes back is the app's own spawns and not whatever else happens to
    /// share the socket.
    pub fn running(&self) -> Result<Option<Vec<String>>> {
        if !process::run("tmux", &self.with_socket(&["has-session", "-t", SESSION]))?.ok {
            return Ok(None);
        }

        Ok(Some(running_in(&self.run(&[
            "list-panes",
            "-s",
            "-t",
            SESSION,
            "-F",
            WINDOW_FORMAT,
        ])?)))
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

/// Which spawns a listing of the session's windows says are still running.
///
/// Two things are dropped, and for different reasons. The **holding** window is
/// furniture — it is what keeps the session alive with no spawn in it. A window
/// whose pane is **dead** is a spawn that has stopped, kept on the server by
/// `remain-on-exit` so the list can say so; it is a leftover, but it is not
/// something still running, and a report that conflated them would tell somebody
/// twenty agents were mid-turn when none was.
///
/// A line that does not parse is dropped rather than guessed at, exactly as in
/// [`Panes::parse`]: the alternative is a report naming a spawn that is not
/// there.
fn running_in(listing: &str) -> Vec<String> {
    listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?;
            let dead = fields.next()?;
            if name == HOLDING || dead == DEAD {
                return None;
            }

            Some(name.to_string())
        })
        .collect()
}

/// A command that creates a pane, with the holder as what it starts.
///
/// The `--` matters as much as the holder does: everything after it is an
/// argument vector rather than something for a shell to read.
fn held(mut arguments: Vec<String>) -> Vec<String> {
    arguments.push("--".to_string());
    arguments.extend(HOLDER.iter().map(|word| (*word).to_string()));

    arguments
}

/// The `respawn-pane` call that replaces the holder with the harness.
///
/// `-k` kills what is there, which is the holder and never a spawn: this runs
/// once, on a pane opened moments earlier. The command is passed one argument
/// at a time, so no shell sees the work the user typed.
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
        let spawns = running_in(CAPTURED_IN_SESSION);

        assert!(
            spawns.contains(&"add-retry-logic-a7f3".to_string()),
            "{spawns:?}"
        );
        assert!(
            spawns.contains(&"fix-the-flake-b2c9".to_string()),
            "{spawns:?}"
        );
    }

    /// The window that keeps the session alive is furniture, and counting it
    /// would say one more spawn than there has ever been.
    #[test]
    fn the_window_holding_the_session_open_is_not_one_of_the_spawns() {
        let spawns = running_in(CAPTURED_IN_SESSION);

        assert!(!spawns.contains(&HOLDING.to_string()), "{spawns:?}");
    }

    /// `remain-on-exit` keeps a stopped spawn's window on the server so the
    /// list can still say it stopped. A report about what is *running* must not
    /// read that as one still going.
    #[test]
    fn a_spawn_that_has_stopped_is_not_one_of_the_ones_still_running() {
        let spawns = running_in(CAPTURED_IN_SESSION);

        assert!(
            !spawns.contains(&"drop-the-cache-d4e1".to_string()),
            "a spawn whose window is only being kept by remain-on-exit was \
             counted as running: {spawns:?}"
        );
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

        /// A recipe for a harmless stand-in: no harness is ever really started
        /// in a test, because the real one costs tokens and needs credentials.
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

    /// Several spawns at once, which is several windows and never several
    /// sessions. There is no holding session and nothing is parked: a spawn is
    /// a window of the one detached session from the moment it is made until it
    /// stops, whether or not it is the one in the slot.
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

    /// The start-up report on a machine that has never run this, or has not run
    /// it since a reboot: there is no server at all, which is *nothing found*
    /// rather than a refusal. Asking must not be what brings a server into
    /// existence.
    #[test]
    fn a_machine_holding_nothing_has_no_session_to_report() {
        let tmux = PrivateTmux::start("nothing-is-held");

        assert_eq!(tmux.server.running().unwrap(), None);
    }

    /// The whole of what a report is built from, against a real tmux: the
    /// format string, the parse and what tmux actually prints, agreeing end to
    /// end. The captured listing pins the parse; this pins that the app is
    /// asking the question the capture was taken of.
    #[test]
    fn a_session_holding_spawns_reports_the_ones_still_running() {
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

        let running = tmux.server.running().unwrap().expect("a session to report");

        assert_eq!(
            running,
            ["add-retry-logic-a7f3"],
            "the report is not the spawns that are still going"
        );
    }

    /// **The report is a statement about the world, not a feature.** Asking
    /// what is running must leave the world exactly as it found it — no session
    /// made, no window opened, nothing attached, nothing adopted. Worth pinning
    /// rather than assuming: `new-session -A` and an attaching client are both
    /// one flag away from here, and either would make the act of reporting the
    /// thing that changed the answer.
    #[test]
    fn asking_what_is_running_leaves_it_exactly_as_it_was() {
        let tmux = PrivateTmux::start("asking-changes-nothing");
        let shape = "#{window_name} #{pane_id} #{pane_dead}";

        // With nothing there, asking twice must still find nothing: the first
        // ask is the one that could have created a session to find.
        assert_eq!(tmux.server.running().unwrap(), None);
        assert_eq!(
            tmux.server.running().unwrap(),
            None,
            "asking what was running is what brought a session into existence"
        );

        let session = tmux.server.session(SLOT).unwrap();
        let pane = tmux
            .server
            .open_window(&session, "add-retry-logic-a7f3")
            .unwrap();
        tmux.server.start(&pane, &tmux.recipe("sleep 120")).unwrap();
        let before = tmux.panes(shape);

        tmux.server.running().unwrap();

        assert_eq!(
            tmux.panes(shape),
            before,
            "asking what was running changed what is running"
        );
    }

    /// How the server the user is sitting in front of is left alone: every
    /// command this module runs is addressed to a socket of the app's own.
    ///
    /// This is the whole of the mechanism, and it is worth pinning here rather
    /// than only observing its effect. [`Server::run`] is the single road out
    /// of this module, and it goes through [`Server::with_socket`] — so a
    /// command that lost its `-L` would find whichever server `$TMUX` named,
    /// which on a spawn's window is the user's own. There is no run of the
    /// tests that could notice that, because a test's server is private for the
    /// same reason.
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
        // The reports go out the same road. `has-session` is the one that
        // matters most here: it is asked before the app has made anything, so
        // one that lost its `-L` would answer about whichever session the
        // user's own server happens to call `spawns` — and the start-up report
        // would describe somebody else's work as this app's litter.
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
        let printed = tmux
            .server
            .run(&["capture-pane", "-p", "-t", &pane])
            .unwrap();
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

    /// Both the states a pane can be closed from: still running something, and
    /// kept behind by `remain-on-exit` after what ran in it stopped.
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
