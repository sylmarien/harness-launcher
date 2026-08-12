//! Twenty spawns at once, driven end to end and measured.
//!
//! The tranche's headline claim is a number — comfortably past the four or five
//! concurrent sessions a person handles by hand — and a number is a thing to
//! observe rather than to assert. So this is not a test of a behaviour: it is a
//! rig that starts the real app on a real terminal, gives it twenty real
//! worktrees over four real repositories, and writes down what happened. What it
//! asserts is only the handful of things that would make the numbers meaningless
//! if they were not true.
//!
//! **It is `#[ignore]`d**, because it takes a minute, wants four cores to itself
//! and prints a report nobody reads on the way past. Run it by hand:
//!
//! ```text
//! cargo test --release --test twenty_spawns -- --ignored --nocapture --test-threads=1
//! ```
//!
//! What it observed on the machine it was run on is written down in
//! `docs/evidence/scale-at-twenty.md`, which is where the numbers belong: a
//! measurement is about a machine, and this file is only how it was taken.
//!
//! **Nothing here starts a real harness.** Twenty Claude Code sessions cost
//! tokens and need credentials, which the design rules out of every test. What
//! runs in the panes is a stand-in that does the two things this measurement is
//! about: it draws a whole screen on the alternate buffer several times a
//! second, and it keeps a session record where the real harness keeps one. Where
//! that stand-in is unlike the real thing is recorded in the evidence document
//! rather than papered over.
//!
//! **Everything is private to the run.** Its own `HOME`, its own worktree root,
//! its own harness configuration directory, and — through `TMUX_TMPDIR` — its
//! own tmux socket directory, so the server the app talks to is this rig's and
//! not the one the person running it is sitting in front of.
//!
//! **This file names the harness, and that is not the invariant breaking.** The
//! rule is that nothing in `src/` outside the harness module may mention Claude
//! Code, and the grep that checks it is scoped there for a reason: this rig is
//! not part of the app, it is the *world the app runs in*. A world that is going
//! to be asked for a harness has to have one in it, under the name the app will
//! reach for — so the stand-in is called `claude` and the models it answers to
//! are the harness's own. Nothing here is compiled into the program.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tempfile::TempDir;

/// The terminal the app is given, in columns and rows.
///
/// A large one, because the list is a third of it and the point is what twenty
/// rows look like beside a session.
const COLUMNS: u16 = 200;
const ROWS: u16 = 50;

/// How long anything waited for is waited for.
const PATIENCE: Duration = Duration::from_secs(30);

/// How often the supervisor looks, which is what a tick's cost is measured
/// against. The app's own number, written again here because a test cannot see
/// it: this file drives the binary rather than linking to it.
const TICK: Duration = Duration::from_millis(200);

/// How long the app gives a spawn before it will call it unaccounted for, and a
/// little more — so a screen taken after this has all three statuses on it.
const GRACE: Duration = Duration::from_secs(12);

/// The repositories the spawns are spread over. Several, because "spawns on
/// different repositories" is what the tranche promises and a single repository
/// would measure something easier.
const REPOSITORIES: [&str; 4] = ["harness-launcher", "acme-api", "dotfiles", "infra"];

/// Twenty pieces of work, five per repository, written the way somebody would
/// write them — the list's readability is one of the things being looked at, and
/// `task-07` would flatter it.
const WORK: [&str; 20] = [
    "fix worktree cleanup",
    "add retry logic",
    "status ladder grace period",
    "spawn form choices",
    "control mode backpressure",
    "rate limit headers",
    "drop legacy auth",
    "openapi drift check",
    "pagination cursors",
    "idempotency keys",
    "tidy the shell prompt",
    "neovim lsp config",
    "ssh agent forwarding",
    "font fallback for emoji",
    "prune stale symlinks",
    "terraform state locking",
    "rotate the deploy keys",
    "alert on disk pressure",
    "cheaper log retention",
    "blue green cutover",
];

/// The spawns whose stand-in reports itself stopped, and the one whose stand-in
/// writes no record at all — so the list has all three statuses on it at once.
///
/// Carried by the model the spawn is started with rather than by its
/// description, so nothing about the arrangement leaks into the names the list
/// shows.
const STOPPED: [usize; 3] = [2, 9, 16];
const SILENT: usize = 13;

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

/// Everywhere the run keeps something, and everything it made.
struct Rig {
    /// Everything below is under here, and goes with it.
    root: TempDir,
}

impl Rig {
    fn new() -> Self {
        let rig = Self {
            root: tempfile::tempdir().unwrap(),
        };
        for directory in ["bin", "home", "config", "data", "sockets", "repositories"] {
            fs::create_dir_all(rig.at(directory)).unwrap();
        }
        write_program(&rig.at("bin").join("claude"), &stand_in());
        write_program(&rig.at("bin").join("git"), SLOW_GIT);

        rig
    }

    fn at(&self, what: &str) -> PathBuf {
        self.root.path().join(what)
    }

    /// A repository with an `origin` whose `HEAD` resolves, which is the least
    /// the app will start a spawn against.
    fn repository(&self, name: &str) -> PathBuf {
        let origin = self.at("repositories").join(format!("{name}.git"));
        let clone = self.at("repositories").join(name);
        git(&["init", "--bare", "-b", "main", text(&origin)]);
        git(&["init", "-b", "main", text(&clone)]);
        git(&["-C", text(&clone), "commit", "--allow-empty", "-m", "first"]);
        git(&["-C", text(&clone), "remote", "add", "origin", text(&origin)]);
        git(&["-C", text(&clone), "push", "-u", "origin", "main"]);
        git(&["-C", text(&clone), "remote", "set-head", "origin", "--auto"]);

        clone
    }

    /// The environment every process this rig starts is given.
    fn environment(&self) -> Vec<(String, String)> {
        let path = format!(
            "{}:{}",
            text(&self.at("bin")),
            std::env::var("PATH").unwrap_or_default()
        );

        vec![
            ("PATH".to_string(), path),
            ("HOME".to_string(), text(&self.at("home")).to_string()),
            (
                "XDG_DATA_HOME".to_string(),
                text(&self.at("data")).to_string(),
            ),
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                text(&self.at("config")).to_string(),
            ),
            (
                "TMUX_TMPDIR".to_string(),
                text(&self.at("sockets")).to_string(),
            ),
            ("TERM".to_string(), "xterm-256color".to_string()),
        ]
    }

    /// Ask the rig's own tmux server something.
    ///
    /// The real `tmux`, addressed with the rig's socket directory, so it is the
    /// app's server that answers and never the user's.
    fn tmux(&self, arguments: &[&str]) -> String {
        let mut command = Command::new("tmux");
        command.args(["-L", "harness-launcher"]).args(arguments);
        for (name, value) in self.environment() {
            command.env(name, value);
        }
        let outcome = command.output().unwrap();

        String::from_utf8_lossy(&outcome.stdout)
            .trim_end()
            .to_string()
    }
}

impl Drop for Rig {
    /// Quitting the app kills nothing, which is the design — so the rig is what
    /// stops twenty stand-ins that would otherwise outlive the test run.
    fn drop(&mut self) {
        self.tmux(&["kill-server"]);
    }
}

/// A stand-in for the harness.
///
/// It is not Claude Code and does not pretend to be. What it has in common with
/// it is the two things the app touches: it draws a whole screen on the
/// alternate buffer several times a second, and it writes a session record where
/// the harness writes one, keyed by the process id tmux reports for its pane.
///
/// Which of the three statuses it reports is carried by the model it was started
/// with: `haiku` reports itself stopped, `sonnet` writes no record at all — so
/// the app cannot account for it — and everything else reports itself busy.
///
/// The record it writes is `__RECORD__`, filled in by [`stand_in`] from the
/// capture rather than written out here.
const STAND_IN: &str = r#"#!/bin/bash
model=""; name=""; work=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --model) model="$2"; shift 2 ;;
    --effort) shift 2 ;;
    -n) name="$2"; shift 2 ;;
    *) work="$1"; shift ;;
  esac
done

case "$model" in
  haiku) status=idle ;;
  sonnet) status="" ;;
  *) status=busy ;;
esac

record() {
  [ -z "$status" ] && return 0
  mkdir -p "$CLAUDE_CONFIG_DIR/sessions"
  printf '__RECORD__\n' "$status" "$$" > "$CLAUDE_CONFIG_DIR/sessions/$$.json"
}
record

printf '\033[?1049h\033[?25l'
trap 'printf "\033[?1049l"; exit 0' TERM INT HUP

filler="the quick brown fox jumps over the lazy dog and keeps on running for a while"
turn=0
typed=""
while :; do
  turn=$((turn + 1))
  printf '\033[H\033[2J'
  printf 'at=%s turn=%d\r\n' "$EPOCHREALTIME" "$turn"
  printf '> %s\r\n\r\n' "$work"
  line=0
  while [ "$line" -lt 40 ]; do
    line=$((line + 1))
    printf '  <<%s>> %02d %s\r\n' "$name" "$line" "$filler"
  done
  printf '\r\ntyped: %s\r\n' "$typed"
  [ $((turn % 10)) -eq 0 ] && record
  if [ "$status" = idle ]; then
    IFS= read -r -s -N1 key && typed="$typed$key"
  else
    IFS= read -r -s -N1 -t 0.2 key && typed="$typed$key"
  fi
done
"#;

/// The captured session record — see `captured/README.md`.
const RECORD: &str = include_str!("../captured/session-record.json");

/// The stand-in, with the record it writes filled in from the capture.
///
/// **The capture plus one field, not a record written from memory.**
/// `captured/README.md` is explicit about why: a format remembered wrongly makes
/// a parser that passes its tests and fails in front of a user, and this rig's
/// whole output is a claim about the app reading real records. The app's own
/// tests depart from this capture in exactly one place — the `status` field the
/// captured machine does not write — and so does this, taking the other twelve
/// fields from the recording rather than inventing a two-field object.
///
/// It cannot call `harness::recorded`, which is where that one departure lives
/// for the unit tests: this is an integration test against a binary crate, and
/// there is no library to reach into. So it makes the same two substitutions
/// here, and they are the same two: a `status` at the front, and the pid, which
/// has to be the stand-in's own because that is what the app keys a record by.
fn stand_in() -> String {
    let rest = RECORD
        .trim()
        .strip_prefix('{')
        .expect("the captured record is an object");
    let record = format!("{{\"status\":\"%s\",{rest}").replace("\"pid\":531", "\"pid\":%d");

    STAND_IN.replace("__RECORD__", &record)
}

/// `git`, with one worktree made slowly on purpose.
///
/// The app puts worktree creation on a thread of its own precisely because it
/// takes seconds; a `git` that always answers at once would leave that claim
/// untested. Only the one worktree whose name says so is slowed — everything
/// else is the real `git`, at its own speed.
///
/// It finds the real one by dropping the first entry of `PATH`, which is this
/// rig's own directory and is always put there first — so nothing here has to
/// know where `git` is installed.
const SLOW_GIT: &str = r#"#!/bin/bash
for argument in "$@"; do
  case "$argument" in
    *takes-its-time*) sleep 6; break ;;
  esac
done
PATH="${PATH#*:}" exec git "$@"
"#;

fn write_program(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Run `git` for the rig's own setting-up, which is not the app's `git` and
/// never goes through the slow one: this process's `PATH` has no rig on it.
fn git(arguments: &[&str]) {
    let outcome = Command::new("git")
        .args(arguments)
        .env("GIT_AUTHOR_NAME", "rig")
        .env("GIT_AUTHOR_EMAIL", "rig@example.com")
        .env("GIT_COMMITTER_NAME", "rig")
        .env("GIT_COMMITTER_EMAIL", "rig@example.com")
        .output()
        .unwrap();
    assert!(
        outcome.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&outcome.stderr)
    );
}

fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

// ---------------------------------------------------------------------------
// The app, on a terminal of the rig's own
// ---------------------------------------------------------------------------

/// The app's terminal, kept up to date as it draws.
///
/// **A live emulator rather than a pile of bytes**, and the reason is the one
/// thing that would otherwise make every measurement here wrong: the app only
/// writes the cells that *changed*, so the stream carries fragments of words and
/// nothing that could be searched for. What a person is looking at is the screen
/// those fragments add up to, which is what this holds — put together by the same
/// emulator the app uses on its own children.
struct Terminal {
    /// What is on the screen now.
    parser: vt100::Parser,
    /// How much has arrived, which is how a wait tells new output from old.
    written: usize,
}

impl Default for Terminal {
    fn default() -> Self {
        Self {
            parser: vt100::Parser::new(ROWS, COLUMNS, 0),
            written: 0,
        }
    }
}

impl Terminal {
    fn screen(&self) -> String {
        self.parser.screen().contents()
    }
}

/// The app, running on a terminal this rig opened.
struct App {
    terminal: Arc<Mutex<Terminal>>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pid: u32,
}

impl Drop for App {
    /// **Stopped when the rig is done with it**, which is the one thing the app
    /// itself will not do: quitting kills nothing, and a measurement that
    /// stopped half way would otherwise leave a screenful of spawns running for
    /// the next one to share the machine with.
    ///
    /// The app as well as whatever was started to run it, because under the
    /// tracer they are two processes and killing the tracer leaves the app.
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-9", &self.pid.to_string()])
            .output();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl App {
    /// Start it, with everything it is to spawn on the command line.
    ///
    /// `under` is a file for `strace` to write to, for the run that counts what
    /// a tick costs in system calls. The other run has none, because tracing
    /// every file a program touches is not free and the timings would be about
    /// the tracer.
    fn start(rig: &Rig, arguments: &[String], under: Option<&Path>) -> Self {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLUMNS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();

        let mut command = match under {
            Some(log) => {
                let mut strace = CommandBuilder::new("strace");
                strace.args([
                    "-f",
                    "--seccomp-bpf",
                    "-qq",
                    "-ttt",
                    "-e",
                    "trace=execve,statx,newfstatat,openat",
                    "-o",
                    text(log),
                    env!("CARGO_BIN_EXE_harness-launcher"),
                ]);
                strace
            }
            None => CommandBuilder::new(env!("CARGO_BIN_EXE_harness-launcher")),
        };
        command.args(arguments);
        command.env_remove("TMUX");
        command.env_remove("TMUX_PANE");
        for (name, value) in rig.environment() {
            command.env(name, value);
        }

        let child = pty.slave.spawn_command(command).unwrap();
        drop(pty.slave);
        // Under the tracer the app is the tracer's child, and it is the app's
        // own process every measurement here is about.
        let pid = match under {
            None => child.process_id().unwrap(),
            Some(_) => traced_by(child.process_id().unwrap()),
        };

        let terminal = Arc::new(Mutex::new(Terminal::default()));
        let mut reader = pty.master.try_clone_reader().unwrap();
        let drawing = Arc::clone(&terminal);
        thread::spawn(move || {
            let mut buffer = vec![0u8; 65536];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                let mut terminal = drawing.lock().unwrap();
                terminal.parser.process(&buffer[..read]);
                terminal.written += read;
            }
        });

        Self {
            terminal,
            writer: pty.master.take_writer().unwrap(),
            child,
            pid,
        }
    }

    /// The screen as it stands.
    fn screen(&self) -> String {
        self.terminal.lock().unwrap().screen()
    }

    /// How many bytes it has written, which is how much drawing it did.
    fn written(&self) -> usize {
        self.terminal.lock().unwrap().written
    }

    /// Wait for the screen to say something, and say when it did.
    ///
    /// Looked at every millisecond, so what comes back is when the app had
    /// finished drawing it to within about that.
    fn wait_for(&mut self, wanted: &str) -> Instant {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if self.screen().contains(wanted) {
                return Instant::now();
            }
            assert!(
                Instant::now() < deadline,
                "the app never drew {wanted:?}{}\n{}",
                self.still_running(),
                self.screen()
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Whether it is still there, for a complaint to say so.
    ///
    /// **A screen that has stopped changing is two different faults**: an app
    /// that is stuck, and an app that is not there any more. Every complaint
    /// here says which, because the first is a bug and the second is an app that
    /// refused and said why.
    fn still_running(&mut self) -> String {
        match self.child.try_wait() {
            Ok(Some(status)) => format!(" — and the app has stopped: {status:?}"),
            Ok(None) => " — the app is still running".to_string(),
            Err(trouble) => format!(" — and the app cannot be asked how it is: {trouble}"),
        }
    }

    /// Type at it.
    fn types(&mut self, bytes: &[u8]) -> Instant {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();

        Instant::now()
    }

    /// Which spawn the slot is showing, read off the screen.
    ///
    /// Each stand-in writes its own name between guillemets at the top of its
    /// screen, and nothing else on the screen does — so this is the one thing
    /// that says which session is in front of the user.
    fn showing(&self) -> Option<String> {
        let screen = self.screen();
        let name = screen.split("<<").nth(1)?.split(">>").next()?;

        Some(name.to_string())
    }

    /// Move the selection, and time how long until the slot is showing another
    /// spawn.
    ///
    /// **Which spawn it lands on is deliberately not predicted.** The list
    /// re-sorts as statuses arrive, so a rig that worked out where the selection
    /// ought to end up would sooner or later be measuring how long it takes to
    /// draw a spawn that was already on the screen — which reads as a switch
    /// that took no time at all, and is the one wrong answer this measurement
    /// could give.
    fn switch(&mut self, key: &[u8]) -> Duration {
        let before = self.showing();
        let sent = self.types(key);
        let deadline = sent + PATIENCE;
        loop {
            let now = self.showing();
            if now.is_some() && now != before {
                return sent.elapsed();
            }
            assert!(
                Instant::now() < deadline,
                "the selection did not move off {before:?}{}\n{}",
                self.still_running(),
                self.screen()
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// How far behind what a spawn drew the screen is running.
    ///
    /// The stand-in stamps every screen it draws with the time it drew it, so
    /// this is the whole path measured at once: the child's write, tmux, the
    /// control-mode stream, the one reader thread, the emulator, the app's frame
    /// — and the rig's own emulator on the end of it.
    ///
    /// **One pane, not twenty.** The first `at=` on the app's screen is the one
    /// in the slot, so this is the lag of the spawn being *looked at* while
    /// nineteen others draw off screen. Those nineteen are load on the single
    /// reader rather than samples of it; whether any of them ran further behind
    /// is not something this can see, and the evidence document says so.
    fn lag(&self) -> Option<Duration> {
        let screen = self.screen();
        let stamped = screen.split("at=").nth(1)?;
        let drawn: f64 = stamped.split_whitespace().next()?.parse().ok()?;

        Duration::try_from_secs_f64(epoch_now() - drawn).ok()
    }

    /// How much memory it is holding.
    fn resident(&self) -> u64 {
        resident(self.pid)
    }
}

/// The resident memory of a process, in kilobytes.
fn resident(pid: u32) -> u64 {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();

    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|line| line.split_whitespace().next()?.parse().ok())
        .unwrap_or_default()
}

/// The keys the app answers to.
const UP: &[u8] = b"\x1b[17~";
const DOWN: &[u8] = b"\x1b[18~";
const COMPOSE: &[u8] = b"\x1bOQ";
const START: &[u8] = b"\x1b[15~";
const QUIT: &[u8] = b"\x1b[21~";
const TAB: &[u8] = b"\t";

// ---------------------------------------------------------------------------
// What was measured
// ---------------------------------------------------------------------------

/// A set of timings, said the way a report wants them.
fn spread(name: &str, timings: &[Duration]) -> String {
    let mut sorted: Vec<f64> = timings
        .iter()
        .map(|timing| timing.as_secs_f64() * 1000.0)
        .collect();
    sorted.sort_by(f64::total_cmp);
    // **A run that sampled nothing is a result, not a crash.** Every collector
    // here keeps only the ticks it could read something off, so an empty set is
    // an outcome the rig has to be able to print — and a report that took the
    // run down rather than saying so would lose the measurements beside it.
    let Some(worst) = sorted.last().copied() else {
        return format!("{name}: nothing was sampled");
    };
    let at = |fraction: usize| sorted[(sorted.len() - 1) * fraction / 100];

    format!(
        "{name}: {} samples, median {:.1} ms, mean {:.1} ms, p95 {:.1} ms, worst {:.1} ms",
        sorted.len(),
        at(50),
        sorted.iter().sum::<f64>() / how_many(sorted.len()),
        at(95),
        worst,
    )
}

/// A set of timings with nothing in it is said, rather than taking the run down.
#[test]
fn a_measurement_that_collected_no_samples_is_reported_rather_than_fatal() {
    assert_eq!(
        spread("how far behind what a spawn drew the screen ran", &[]),
        "how far behind what a spawn drew the screen ran: nothing was sampled"
    );
}

/// And one sample is still a spread, rather than an off-by-one into the range.
#[test]
fn a_single_sample_is_its_own_median_and_its_own_worst() {
    let said = spread("a lag", &[Duration::from_millis(7)]);

    assert!(said.contains("1 samples"), "{said}");
    assert!(said.contains("median 7.0 ms"), "{said}");
    assert!(said.contains("worst 7.0 ms"), "{said}");
}

/// A count, as something to divide by.
///
/// Written out rather than cast in place: a count is small and a division by it
/// is exact, and saying so once is cheaper than saying it at every use.
fn how_many(counted: usize) -> f64 {
    f64::from(u32::try_from(counted).unwrap_or(u32::MAX))
}

/// Now, as the stand-ins write it.
fn epoch_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// **Twenty spawns at once**, and what each part of it cost.
///
/// One test rather than six, because each measurement is about a machine
/// carrying the other nineteen spawns while it is taken — and six tests would
/// either share the machine or measure it empty. Each step says what it found
/// the moment it finds it, so a step that fails does not take the ones before it
/// down with it.
#[test]
#[ignore = "a measurement rig: it takes a minute and wants the machine to itself"]
fn twenty_spawns_at_once() {
    let rig = Rig::new();
    let repositories: Vec<PathBuf> = REPOSITORIES
        .iter()
        .map(|name| rig.repository(name))
        .collect();

    let started = Instant::now();
    let mut app = App::start(&rig, &asked_for(&repositories), None);

    let names = all_of_them_live(&rig, &mut app, started);
    what_is_held(&rig, &mut app);
    how_far_behind(&mut app);
    switching_between_them(&mut app, names.len());
    a_creation_that_takes_its_time(&mut app, &repositories);
    what_it_leaves_behind(&mut app);
}

/// Twenty spawns, over four repositories, all of them live — and what the list
/// says about them once every status has settled.
fn all_of_them_live(rig: &Rig, app: &mut App, started: Instant) -> Vec<String> {
    for work in WORK {
        app.wait_for(&work.replace(' ', "-"));
    }
    said(&format!(
        "twenty spawns over {} repositories were created, started and drawn in {:.1} s",
        REPOSITORIES.len(),
        started.elapsed().as_secs_f64()
    ));

    let windows = rig.tmux(&[
        "list-panes",
        "-s",
        "-t",
        "spawns",
        "-F",
        "#{window_name} #{pane_dead}",
    ]);
    let live = windows
        .lines()
        .filter(|line| !line.starts_with("holding ") && line.ends_with(" 0"))
        .count();
    assert_eq!(live, WORK.len(), "not twenty live panes:\n{windows}");
    said(&format!(
        "tmux holds {} windows in one session: the holding window and {live} live spawns",
        windows.lines().count()
    ));

    // Past the grace period, so the spawn nothing can be read about has settled
    // into `unknown` and all three statuses are on the list at once.
    thread::sleep(GRACE);
    let settled = app.screen();
    let names = names_on(&settled);
    assert_eq!(
        names.len(),
        WORK.len(),
        "the list does not name twenty spawns:\n{settled}"
    );
    said(&format!("the list, at twenty:\n\n{}", listed(&settled)));

    names
}

/// What the two processes are holding, and whether thirty seconds of twenty
/// spawns redrawing moves it.
fn what_is_held(rig: &Rig, app: &mut App) {
    let server: u32 = rig
        .tmux(&["display-message", "-p", "#{pid}"])
        .parse()
        .unwrap();
    let (first_server, first_app) = (resident(server), app.resident());
    let drawn = app.written();

    thread::sleep(Duration::from_secs(30));

    said(&format!(
        "the tmux server was holding {} MB at twenty panes, and {} MB thirty seconds later\n\
         the app was holding {} MB, and {} MB thirty seconds later\n\
         the app drew {} KB in those thirty seconds",
        first_server / 1024,
        resident(server) / 1024,
        first_app / 1024,
        app.resident() / 1024,
        (app.written() - drawn) / 1024,
    ));
}

/// How far behind what a spawn drew the screen runs, while twenty of them draw.
fn how_far_behind(app: &mut App) {
    let mut lags = Vec::new();
    for _ in 0..50 {
        if let Some(lag) = app.lag() {
            lags.push(lag);
        }
        thread::sleep(Duration::from_millis(100));
    }

    said(&spread(
        "how far behind what a spawn drew the screen ran",
        &lags,
    ));
}

/// Switching, mid-turn: all the way down the list and all the way back.
///
/// The selection stops at both ends rather than wrapping, so this is nineteen
/// moves each way rather than twenty — and it is walked to the top first,
/// because the app opens on the spawn that was started *first* and the
/// attention-first order has since moved that into the middle of its group.
fn switching_between_them(app: &mut App, spawns: usize) {
    for _ in 0..spawns {
        app.types(UP);
    }
    thread::sleep(Duration::from_secs(1));

    let mut switches = Vec::new();
    for _ in 1..spawns {
        switches.push(app.switch(DOWN));
    }
    for _ in 1..spawns {
        switches.push(app.switch(UP));
    }

    said(&spread(
        "switching between spawns, mid-turn, right down the list and back",
        &switches,
    ));
}

/// A creation made to take six seconds, and whether anything else waits for it.
fn a_creation_that_takes_its_time(app: &mut App, repositories: &[PathBuf]) {
    let slow = "creation that takes its time";
    app.types(COMPOSE);
    app.types(text(&repositories[0]).as_bytes());
    app.types(TAB);
    app.types(slow.as_bytes());
    let asking = Instant::now();
    app.types(START);

    // Everything typed above is still queued: the app reads one keystroke a
    // frame, so the form is waited for rather than assumed. Once it is up, the
    // selection is walked between two spawns further down the list — never back
    // onto the draft, whose slot holds a form rather than a session and so is
    // not a spawn to have switched to.
    app.wait_for("NEW SPAWN");
    let mut during = Vec::new();
    let mut drawing = 0;
    app.switch(DOWN);
    app.switch(DOWN);
    let mut up = true;
    while asking.elapsed() < Duration::from_secs(5) {
        let before = app.written();
        during.push(app.switch(if up { UP } else { DOWN }));
        drawing += app.written() - before;
        up = !up;
        thread::sleep(Duration::from_millis(100));
    }
    said(&format!(
        "{}\nand it drew {} KB while the creation ran",
        spread("switching while a six-second creation runs", &during),
        drawing / 1024
    ));

    app.wait_for(&slow.replace(' ', "-"));
    said(&format!(
        "the twenty-first spawn was on the list {:.1} s after it was asked for",
        asking.elapsed().as_secs_f64()
    ));
}

/// What it says it is leaving behind, on the way out.
fn what_it_leaves_behind(app: &mut App) {
    app.types(QUIT);
    app.wait_for("harness-launcher: ");

    let leaving = app
        .screen()
        .lines()
        .find(|line| line.contains("harness-launcher: "))
        .unwrap_or_default()
        .trim()
        .to_string();
    said(&format!("on the way out it said: {leaving}"));
}

/// The command line that asks for twenty spawns.
fn asked_for(repositories: &[PathBuf]) -> Vec<String> {
    let mut arguments: Vec<String> = Vec::new();
    for (at, work) in WORK.iter().enumerate() {
        if at > 0 {
            arguments.push("--and".to_string());
        }
        arguments.push(text(&repositories[at % REPOSITORIES.len()]).to_string());
        arguments.push((*work).to_string());
        if STOPPED.contains(&at) {
            arguments.extend(["--model".to_string(), "haiku".to_string()]);
        } else if at == SILENT {
            arguments.extend(["--model".to_string(), "sonnet".to_string()]);
        }
    }

    arguments
}

/// Say something the moment it is known, rather than at the end: a measurement
/// that fails later must not take the ones already taken with it.
fn said(what: &str) {
    println!("\n--- {what}");
}

/// The lines of a screen that are the list, which is its left third.
fn listed(screen: &str) -> String {
    screen
        .lines()
        .map(|line| {
            line.chars()
                .take(66)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// Every spawn the list names, in the order it draws them.
fn names_on(screen: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in screen.lines() {
        let list: String = line.chars().take(66).collect();
        for work in WORK {
            let slug = work.replace(' ', "-");
            let Some(at) = list.find(&slug) else {
                continue;
            };
            let name: String = list[at..]
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '-')
                .collect();
            if !found.contains(&name) {
                found.push(name);
            }
        }
    }

    found
}

/// **What one tick actually costs**, counted rather than reasoned about.
///
/// The design says a tick is one `list-panes` covering every spawn at once, plus
/// a stat per live spawn read only when it has moved, and a `ps` probe that is a
/// tie-breaker rather than a per-tick cost. Every clause of that is a count, so
/// the app is run under `strace` and the counts are taken.
///
/// **The numbers here are shapes, not speeds.** Tracing every file a program
/// touches makes everything slower, so what is worth reading is the ratio: how
/// many subprocesses per tick, how many stats per tick, how many of those stats
/// turned into a read.
///
/// Run it on its own — `--test-threads=1` — or it shares four cores with forty
/// panes and measures the contention instead.
#[test]
#[ignore = "a measurement rig: it takes a minute and wants the machine to itself"]
fn what_a_tick_costs_at_twenty_spawns() {
    let rig = Rig::new();
    let repositories: Vec<PathBuf> = REPOSITORIES
        .iter()
        .map(|name| rig.repository(name))
        .collect();
    let log = rig.at("strace.log");
    let mut app = App::start(&rig, &asked_for(&repositories), Some(&log));

    for work in WORK {
        app.wait_for(&work.replace(' ', "-"));
    }
    // Past the grace period, so the spawn the app cannot account for is running
    // its tie-breaker — which is the per-spawn cost the design says a tick must
    // not have twenty of.
    thread::sleep(GRACE);

    let from = epoch_now();
    thread::sleep(Duration::from_secs(10));
    let to = epoch_now();

    let traced = fs::read_to_string(&log).unwrap();
    if let Some(keep) = std::env::var_os("KEEP_TRACE") {
        fs::copy(&log, PathBuf::from(keep)).unwrap();
    }

    // Everything under the app is traced, stand-ins included, and they run
    // programs of their own — so what each line is *about* has to be read off
    // the line. It can be: the app reaches for `tmux`, `ps` and `git` and the
    // stand-ins reach for nothing but `mkdir`, and only the app reads a session
    // record. Every program the trace saw is reported below rather than only the
    // ones expected, so nothing is quietly left out of the count.
    //
    // Following `clone` instead — which would attribute every process to
    // whatever forked it — was tried and abandoned: tracing it stops the whole
    // tree on every thread and every fork, and the app slowed from five ticks a
    // second to one in ten seconds. A measurement that changes what it measures
    // by that much is not one.
    let mut ticks = 0;
    let mut ticked: Vec<f64> = Vec::new();
    let mut ran: Vec<String> = Vec::new();
    let mut stats = 0;
    let mut reads = 0;
    for line in traced.lines() {
        let Some((_, at, call)) = traced_call(line) else {
            continue;
        };
        if at < from || at > to {
            continue;
        }

        if call.starts_with("execve(") {
            // Only the ones that worked. A program named without a path is
            // looked for on each `PATH` directory in turn, and every miss is an
            // `execve` of its own — eleven attempts here for one process, which
            // is the C library searching rather than the app doing anything.
            if !line.ends_with("= 0") {
                continue;
            }
            let program = quoted(line).unwrap_or_default();
            let program = program.rsplit('/').next().unwrap_or_default().to_string();
            if line.contains("\"list-panes\"") {
                ticks += 1;
                ticked.push(at);
            }
            ran.push(program);
        } else if line.contains("/sessions/") && line.contains(".json") {
            if call.starts_with("statx(") || call.starts_with("newfstatat(") {
                stats += 1;
            } else if call.starts_with("openat(") && line.contains("O_RDONLY") {
                reads += 1;
            }
        }
    }

    assert!(ticks > 0, "nothing ticked in ten seconds");
    let mut counted: Vec<(String, usize)> = Vec::new();
    for program in &ran {
        match counted.iter_mut().find(|(name, _)| name == program) {
            Some((_, seen)) => *seen += 1,
            None => counted.push((program.clone(), 1)),
        }
    }
    counted.sort_by_key(|(_, seen)| std::cmp::Reverse(*seen));
    let mine = ran
        .iter()
        .filter(|program| THE_APPS_OWN.contains(&program.as_str()))
        .count();
    // One rate per program as well as the total. The total answers "how many
    // processes a tick", but the design's claim is specifically about the
    // *listing* — one `tmux` however many spawns there are — and a combined
    // figure with a stray `ps` inside it cannot be read as saying that.
    let each: Vec<String> = THE_APPS_OWN
        .iter()
        .map(|program| {
            let seen = ran.iter().filter(|run| run == program).count();

            format!("{program} {:.2}", how_many(seen) / f64::from(ticks))
        })
        .collect();

    let gaps: Vec<Duration> = ticked
        .windows(2)
        .filter_map(|pair| Duration::try_from_secs_f64(pair[1] - pair[0]).ok())
        .collect();
    said(&spread("the gap between one tick and the next", &gaps));
    said(&format!(
        "over {:.0} s of twenty spawns running, traced:\n\
         {ticks} ticks — {:.1} a second, against the {:?} the supervisor sleeps for\n\
         every program the trace saw run, and how many: {counted:?}\n\
         of which the app's own tools: {:.2} a tick, one at a time: {each:?}\n\
         {:.1} stats of a session record a tick, {:.2} of them read",
        to - from,
        f64::from(ticks) / (to - from),
        TICK,
        how_many(mine) / f64::from(ticks),
        f64::from(stats) / f64::from(ticks),
        f64::from(reads) / f64::from(ticks),
    ));
}

/// What the app runs, as against what the stand-ins in its panes run.
///
/// Named here rather than inferred: these three are the whole of what the app
/// shells out to, and a program outside this list turning up in a tick's count
/// would be worth knowing about rather than worth hiding.
const THE_APPS_OWN: [&str; 3] = ["tmux", "ps", "git"];

/// One line of a trace: which thread, when, and what it called.
fn traced_call(line: &str) -> Option<(&str, f64, &str)> {
    let mut fields = line.split_whitespace();
    let (thread, at, call) = (fields.next()?, fields.next()?, fields.next()?);

    Some((thread, at.parse().ok()?, call))
}

/// The one process a tracer is running, waited for until it is there.
fn traced_by(tracer: u32) -> u32 {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let children = fs::read_to_string(format!("/proc/{tracer}/task/{tracer}/children"))
            .unwrap_or_default();
        if let Some(traced) = children.split_whitespace().next() {
            return traced.parse().unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "the tracer never started anything"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// The first thing in double quotes on a line, which for an `execve` is the
/// program it ran.
fn quoted(line: &str) -> Option<String> {
    Some(line.split('"').nth(1)?.to_string())
}
